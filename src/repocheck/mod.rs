use std::time::{Duration, Instant};

use oci_distribution::client::{Client, ClientConfig};
use oci_distribution::errors::{OciDistributionError, OciErrorCode};
use oci_distribution::secrets::RegistryAuth;
use oci_distribution::Reference;

/// Hard timeout for a single registry request. oci-distribution builds its
/// HTTP client without any timeout, so a stalled response (observed on
/// registry-1.docker.io) would otherwise block the whole scan forever.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
/// Maximum number of retries per page after the initial attempt.
const MAX_RETRIES: usize = 3;
/// Initial backoff between retries; doubles per attempt (2s, 4s, 8s).
const FIRST_RETRY_DELAY: Duration = Duration::from_secs(2);
/// Total wall-clock budget for retrying a single image. Docker Hub's
/// anonymous rate-limit window is 6 hours, so retrying a persistent 429
/// for over a minute per page just makes scans appear hung; give up
/// quickly and move on to the next workload.
const RETRY_BUDGET: Duration = Duration::from_secs(30);

/// Translate opaque registry-client errors into actionable messages.
///
/// Some registries answer `tags/list` with `{"name":"…","tags":null}`
/// (e.g. Docker Hub for private or empty repositories). The strict
/// `TagResponse` deserialiser in oci-distribution rejects that with an
/// opaque serde error, so map it to something a human can act on.
fn describe_fetch_error(image: &str, err: &OciDistributionError) -> String {
    match err {
        OciDistributionError::JsonError(je)
            if je.to_string().contains("invalid type: null, expected a sequence") =>
        {
            format!(
                "registry returned a null tag list for {image} (repository is empty, private, or not anonymously accessible)"
            )
        }
        OciDistributionError::UnauthorizedError { .. }
        | OciDistributionError::AuthenticationFailure(_) => {
            format!(
                "registry requires authentication for {image} (anonymous access denied, e.g. Docker Hub now restricts anonymous pulls of popular repositories)"
            )
        }
        other => other.to_string(),
    }
}

fn is_rate_limited(err: &OciDistributionError) -> bool {
    match err {
        OciDistributionError::RegistryError { envelope, .. } => envelope
            .errors
            .iter()
            .any(|e| e.code == OciErrorCode::Toomanyrequests),
        OciDistributionError::ServerError { code, .. } => *code == 429,
        _ => false,
    }
}

/// Fetch one page of tags, retrying rate-limited (429) and timed-out
/// requests with exponential backoff. Retries stop after `MAX_RETRIES`
/// attempts or once `retry_deadline` has passed, whichever comes first.
async fn fetch_tags_page(
    client: &Client,
    reference: &Reference,
    auth: &RegistryAuth,
    max_tags: Option<usize>,
    last_tag: Option<&str>,
    retry_deadline: Instant,
) -> Result<Vec<String>, OciDistributionError> {
    let mut delay = FIRST_RETRY_DELAY;

    for attempt in 0..=MAX_RETRIES {
        let (err, reason) = match tokio::time::timeout(
            REQUEST_TIMEOUT,
            client.list_tags(reference, auth, max_tags, last_tag),
        )
        .await
        {
            Ok(Ok(tags)) => return Ok(tags.tags),
            Ok(Err(e)) if is_rate_limited(&e) => (e, "Rate limited"),
            Ok(Err(e)) => return Err(e),
            Err(_) => (
                OciDistributionError::GenericError(Some(format!(
                    "timed out after {:?} listing tags for {}",
                    REQUEST_TIMEOUT,
                    reference.repository()
                ))),
                "Timed out",
            ),
        };

        if attempt == MAX_RETRIES {
            log::warn!(
                "Giving up on tags for {} after {} attempts: {}",
                reference.repository(),
                attempt + 1,
                err
            );
            return Err(err);
        }
        if Instant::now() >= retry_deadline {
            log::warn!(
                "Giving up on tags for {}: retry budget {:?} exhausted ({})",
                reference.repository(),
                RETRY_BUDGET,
                err
            );
            return Err(err);
        }

        log::warn!(
            "{reason} fetching tags for {}: {}. Retrying in {:?} (attempt {}/{})",
            reference.repository(),
            err,
            delay,
            attempt + 1,
            MAX_RETRIES,
        );
        tokio::time::sleep(delay).await;
        delay *= 2;
    }

    unreachable!()
}

pub async fn get_tags_for_image(image: &str) -> Result<(Vec<String>, bool), Box<dyn std::error::Error>> {
     let reference = Reference::try_from(image)?;
     let auth = RegistryAuth::Anonymous;
     let config = ClientConfig::default();
     let mut client = Client::new(config);
     let max_tags = Some(1500);
     log::info!("Fetching tags for image: {:?}", reference.tag());

     // Shared across all pages: once spent, a persistent 429/timeout fails
     // the whole image quickly instead of stalling the scan.
     let retry_deadline = Instant::now() + RETRY_BUDGET;

     let mut all_tags = Vec::new();
     let mut last_tag = reference.tag().map(|s| s.to_string());
     let mut attempt_count = 0;
     // GHCR caps each page at 1000 tags regardless of n, so the page budget
     // must be sized for that (30 pages ~= 30k tags on both registries).
     const MAX_ATTEMPTS: usize = 30;
     let mut exhausted = false;

     loop {
         attempt_count += 1;
         if attempt_count > MAX_ATTEMPTS {
             log::warn!("Reached maximum number of attempts ({}) for fetching tags. Some tags might be missing.", MAX_ATTEMPTS);
             exhausted = true;
             break;
         }
         log::info!("Fetching tags with last tag: {:?}", last_tag);
         let tags = fetch_tags_page(
             &client,
             &reference,
             &auth,
             max_tags,
             last_tag.as_deref(),
             retry_deadline,
         )
         .await
         .map_err(|e| describe_fetch_error(image, &e))?;

         log::info!("Available tags for {}: {:?}", reference, tags);
         log::info!("Number of tags: {}", tags.len());

         all_tags.extend(tags.clone());

         if tags.len() >= 100 {
             if let Some(latest) = tags.iter().max() {
                 last_tag = Some(latest.clone());
                 log::info!("Got a full page of results, continuing with last tag: {}", latest);
             } else {
                 break;
             }
         } else {
             break;
         }
     }

     log::info!("Total number of tags collected: {}", all_tags.len());

     Ok((all_tags, exhausted))
 }

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn null_tag_list_error_is_actionable() {
        // Reproduce the exact serde error oci-distribution hits on
        // {"name":"…","tags":null}.
        #[derive(Debug, serde::Deserialize)]
        struct Strict {
            tags: Vec<String>,
        }
        let je: serde_json::error::Error =
            serde_json::from_str::<Strict>(r#"{"tags":null}"#).unwrap_err();
        let err = OciDistributionError::JsonError(je);
        let msg = describe_fetch_error("myreg.io/foo/bar", &err);
        assert!(
            msg.contains("null tag list") && msg.contains("myreg.io/foo/bar"),
            "unexpected message: {msg}"
        );
    }

    #[test]
    fn unauthorized_error_is_actionable() {
        let err = OciDistributionError::UnauthorizedError {
            url: "https://registry-1.docker.io/v2/foo/bar/tags/list".to_string(),
        };
        let msg = describe_fetch_error("foo/bar", &err);
        assert!(msg.contains("requires authentication") && msg.contains("foo/bar"));
    }

    #[test]
    fn other_errors_pass_through() {
        let err = OciDistributionError::ServerError {
            code: 500,
            url: "https://reg/v2/foo/tags/list".to_string(),
            message: "boom".to_string(),
        };
        assert_eq!(describe_fetch_error("foo", &err), err.to_string());
    }
}
