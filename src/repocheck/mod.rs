use std::future::Future;
use std::pin::Pin;
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
/// Tags per page requested from the registry. GHCR caps each page at
/// 1000 tags regardless of `n`, so the effective page size there is 1000.
const PAGE_SIZE: Option<usize> = Some(1500);
/// A page with fewer tags than this is treated as the end of the list.
/// Must stay well below the 1000-tag page cap GHCR applies.
const SHORT_PAGE_THRESHOLD: usize = 100;

/// Translate opaque registry-client errors into actionable messages.
fn describe_fetch_error(image: &str, err: &OciDistributionError) -> String {
    match err {
        OciDistributionError::UnauthorizedError { .. }
        | OciDistributionError::AuthenticationFailure(_) => {
            format!(
                "registry requires authentication for {image} (anonymous access denied, e.g. Docker Hub now restricts anonymous pulls of popular repositories)"
            )
        }
        other => other.to_string(),
    }
}

/// Some registries (GHCR for sure) answer `tags/list` with
/// `{"name":"…","tags":null}` — most notably when the `last` cursor points
/// at or beyond the end of their internal tag ordering, and for empty or
/// not-anonymously-visible repositories. The strict `TagResponse`
/// deserialiser in oci-distribution rejects that with an opaque serde
/// error, so detect it explicitly instead of surfacing it.
fn is_null_tags_error(err: &OciDistributionError) -> bool {
    matches!(
        err,
        OciDistributionError::JsonError(je)
            if je.to_string().contains("invalid type: null, expected a sequence")
    )
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

/// A single `tags/list` page, owned by the caller (no borrows of the
/// request context), so pagination can be driven by plain `FnMut` closures
/// and unit-tested without a real registry.
type PageFuture<'a> =
    Pin<Box<dyn Future<Output = Result<Vec<String>, OciDistributionError>> + Send + 'a>>;

/// Walk the `tags/list` pagination of one image and collect all its tags.
///
/// Pagination stops when the registry returns a short or empty page, when
/// it signals the end of the list (GHCR answers a final `last` cursor with
/// `tags: null`), or when a registry that ignores the `last` cursor
/// returns the same page twice. Returns the collected tags plus a flag that
/// is set when `max_pages` was used up and the tag list may therefore be
/// truncated.
async fn collect_tag_pages<F>(
    max_pages: usize,
    image: &str,
    mut fetch_page: F,
) -> Result<(Vec<String>, bool), OciDistributionError>
where
    F: FnMut(Option<&str>) -> PageFuture<'static>,
{
    let max_pages = max_pages.max(1);
    let mut all_tags: Vec<String> = Vec::new();
    // Start from the beginning of the registry's own tag ordering, not from
    // the workload's current tag: GHCR's ordering is not lexicographic and
    // it answers a `last` cursor at the end of the list with `tags: null`,
    // so starting at the current tag failed every scan of a workload
    // running the newest tag.
    let mut last_tag: Option<String> = None;
    let mut prev_first: Option<String> = None;
    let mut page = 0usize;
    let mut exhausted = false;

    loop {
        page += 1;
        if page > max_pages {
            log::warn!(
                "Reached the page budget of {} pages for {}. Collected {} tags; tags beyond this point were not scanned. Raise system.registry_max_pages to scan more.",
                max_pages,
                image,
                all_tags.len()
            );
            exhausted = true;
            break;
        }

        log::info!(
            "Fetching tags (page {}) with last tag: {:?}",
            page,
            last_tag
        );
        let tags = match fetch_page(last_tag.as_deref()).await {
            Ok(tags) => tags,
            Err(e) if is_null_tags_error(&e) => {
                if all_tags.is_empty() {
                    log::warn!(
                        "Registry returned a null tag list for {} (repository is empty or not anonymously accessible)",
                        image
                    );
                } else {
                    log::info!(
                        "Registry returned a null tag list for {} — end of tag list",
                        image
                    );
                }
                break;
            }
            Err(e) => return Err(e),
        };

        if tags.is_empty() {
            break;
        }
        // A registry that ignores the `last` cursor returns the same page
        // again; stop instead of burning the whole page budget re-fetching
        // tags we already have.
        if let (Some(first), Some(prev)) = (tags.first(), prev_first.as_ref()) {
            if first == prev {
                log::warn!(
                    "Registry for {} ignored the pagination cursor (returned the same page twice); treating the {} tags collected so far as complete",
                    image,
                    all_tags.len()
                );
                break;
            }
        }

        prev_first = tags.first().cloned();
        all_tags.extend(tags.clone());

        if tags.len() >= SHORT_PAGE_THRESHOLD {
            if let Some(last) = tags.last() {
                last_tag = Some(last.clone());
                log::info!(
                    "Got a full page of results, continuing with last tag: {}",
                    last
                );
            } else {
                break;
            }
        } else {
            break;
        }
    }

    Ok((all_tags, exhausted))
}

/// Fetch all tags for `image` from its registry.
///
/// `max_pages` bounds how many `tags/list` pages are followed (each page
/// holds up to 1000 tags on GHCR, where pages are capped regardless of the
/// requested size). If the budget runs out before the list ends, the tags
/// collected so far are returned with `exhausted = true` so callers can
/// flag that the list may be incomplete.
pub async fn get_tags_for_image(
    image: &str,
    max_pages: usize,
) -> Result<(Vec<String>, bool), Box<dyn std::error::Error>> {
    let reference = Reference::try_from(image)?;
    let auth = RegistryAuth::Anonymous;
    let mut client = Client::new(ClientConfig::default());
    log::info!("Fetching tags for image: {:?}", reference.tag());

    // Shared across all pages: once spent, a persistent 429/timeout fails
    // the whole image quickly instead of stalling the scan.
    let retry_deadline = Instant::now() + RETRY_BUDGET;

    // The closure clones the (cheap, reference-counted) client per page so
    // the boxed future owns everything it needs.
    let fetch_page = |last_tag: Option<&str>| -> PageFuture<'static> {
        let client = client.clone();
        let reference = reference.clone();
        let auth = auth.clone();
        // Own the cursor so the boxed future holds no borrows of this scope.
        let last = last_tag.map(str::to_owned);
        Box::pin(async move {
            fetch_tags_page(
                &client,
                &reference,
                &auth,
                PAGE_SIZE,
                last.as_deref(),
                retry_deadline,
            )
            .await
        })
    };

    let (mut all_tags, exhausted) = collect_tag_pages(max_pages, image, fetch_page)
        .await
        .map_err(|e| describe_fetch_error(image, &e))?;

    // Registries can return overlapping pages when the cursor behaviour is
    // off; deduplicate before handing the list to the version comparator.
    all_tags.sort();
    all_tags.dedup();

    log::info!("Total number of tags collected: {}", all_tags.len());

    Ok((all_tags, exhausted))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    fn null_tag_list_error() -> OciDistributionError {
        // Reproduce the exact serde error oci-distribution hits on
        // {"name":"…","tags":null} (GHCR's end-of-cursor / empty response).
        #[derive(Debug, serde::Deserialize)]
        struct Strict {
            tags: Vec<String>,
        }
        let je: serde_json::error::Error =
            serde_json::from_str::<Strict>(r#"{"tags":null}"#).unwrap_err();
        OciDistributionError::JsonError(je)
    }

    fn tag_page(start: usize, count: usize) -> Vec<String> {
        (start..start + count)
            .map(|i| format!("tag-{i:05}"))
            .collect()
    }

    /// Fake registry that serves `pages` in order and then repeats the last
    /// one forever (a well-behaved registry beyond its list ends with a
    /// short page; this fake models an endless one).
    fn endless_pages(pages: Vec<Vec<String>>) -> impl FnMut(Option<&str>) -> PageFuture<'static> {
        let calls = AtomicUsize::new(0);
        move |last: Option<&str>| -> PageFuture<'static> {
            let idx = calls.fetch_add(1, Ordering::SeqCst).min(pages.len() - 1);
            let page = pages[idx].clone();
            let _ = last;
            Box::pin(async move { Ok(page) })
        }
    }

    #[test]
    fn null_tag_list_error_is_detected() {
        let err = null_tag_list_error();
        assert!(is_null_tags_error(&err));
        // Other JSON-ish errors are not misdetected.
        let other = OciDistributionError::ServerError {
            code: 500,
            url: "https://reg/v2/foo/tags/list".to_string(),
            message: "boom".to_string(),
        };
        assert!(!is_null_tags_error(&other));
    }

    #[tokio::test]
    async fn pagination_collects_until_short_page() {
        let pages = vec![tag_page(0, 150), tag_page(150, 150), tag_page(300, 20)];
        let (tags, exhausted) = collect_tag_pages(10, "reg/img", endless_pages(pages))
            .await
            .unwrap();
        assert_eq!(tags.len(), 320);
        assert!(!exhausted);
        // No duplicates were introduced.
        let mut deduped = tags.clone();
        deduped.sort();
        deduped.dedup();
        assert_eq!(deduped.len(), tags.len());
    }

    #[tokio::test]
    async fn pagination_flags_budget_exhaustion() {
        // Every page is full, so the list keeps growing until the budget
        // runs out (immich-server has ~32k tags: 32 full pages).
        let pages = (0..5).map(|p| tag_page(p * 150, 150)).collect();
        let (tags, exhausted) = collect_tag_pages(3, "reg/img", endless_pages(pages))
            .await
            .unwrap();
        assert_eq!(tags.len(), 450);
        assert!(exhausted);
    }

    #[tokio::test]
    async fn pagination_stops_on_repeated_page() {
        // A registry that ignores `last` returns the same page again; we
        // must stop after two fetches and not report the list as exhausted.
        let pages = vec![tag_page(0, 150)];
        let calls = Arc::new(AtomicUsize::new(0));
        let calls_in = Arc::clone(&calls);
        let fetch = move |_last: Option<&str>| -> PageFuture<'static> {
            calls_in.fetch_add(1, Ordering::SeqCst);
            let page = pages[0].clone();
            Box::pin(async move { Ok(page) })
        };
        let (tags, exhausted) = collect_tag_pages(30, "reg/img", fetch).await.unwrap();
        assert_eq!(tags.len(), 150);
        assert!(!exhausted);
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn pagination_stops_on_null_tag_list() {
        // GHCR end-of-cursor behaviour: first page fine, second is null.
        let calls = Arc::new(AtomicUsize::new(0));
        let calls_in = Arc::clone(&calls);
        let first = tag_page(0, 150);
        let fetch = move |_last: Option<&str>| -> PageFuture<'static> {
            let idx = calls_in.fetch_add(1, Ordering::SeqCst);
            let page = if idx == 0 {
                Ok(first.clone())
            } else {
                Err(null_tag_list_error())
            };
            Box::pin(async move { page })
        };
        let (tags, exhausted) = collect_tag_pages(10, "reg/img", fetch).await.unwrap();
        assert_eq!(tags.len(), 150);
        assert!(!exhausted);
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn pagination_stops_on_empty_page() {
        let calls = Arc::new(AtomicUsize::new(0));
        let calls_in = Arc::clone(&calls);
        let fetch = move |_last: Option<&str>| -> PageFuture<'static> {
            calls_in.fetch_add(1, Ordering::SeqCst);
            Box::pin(async move { Ok(Vec::<String>::new()) })
        };
        let (tags, exhausted) = collect_tag_pages(10, "reg/img", fetch).await.unwrap();
        assert!(tags.is_empty());
        assert!(!exhausted);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn pagination_propagates_fetch_errors() {
        let fetch = |_last: Option<&str>| -> PageFuture<'static> {
            Box::pin(async move {
                Err(OciDistributionError::ServerError {
                    code: 500,
                    url: "https://reg/v2/foo/tags/list".to_string(),
                    message: "boom".to_string(),
                })
            })
        };
        assert!(collect_tag_pages(10, "reg/img", fetch).await.is_err());
    }

    /// Live-registry smoke test (opt-in): ghcr.io/immich-app/immich-server
    /// has ~32k tags — just over the old hard-coded 30-page budget that
    /// used to flag this image as scan-exhausted every run.
    #[tokio::test]
    #[ignore = "live registry test: needs network access to ghcr.io"]
    async fn live_ghcr_immich_server_full_tag_list() {
        let (tags, exhausted) = get_tags_for_image("ghcr.io/immich-app/immich-server:v3.3.1", 100)
            .await
            .expect("tag fetch should succeed");
        assert!(
            !exhausted,
            "immich-server tag list should fit in the default page budget"
        );
        assert!(
            tags.contains(&"v3.3.1".to_string()),
            "must include the running tag"
        );
        assert!(
            tags.len() > 30_000,
            "expected the full ~31k tag list, got {}",
            tags.len()
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
