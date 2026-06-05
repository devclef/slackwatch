use oci_distribution::client::{Client, ClientConfig};
use oci_distribution::errors::{OciDistributionError, OciErrorCode};
use oci_distribution::secrets::RegistryAuth;
use oci_distribution::Reference;

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

async fn fetch_tags_page(
    client: &Client,
    reference: &Reference,
    auth: &RegistryAuth,
    max_tags: Option<usize>,
    last_tag: Option<&str>,
) -> Result<Vec<String>, OciDistributionError> {
    const MAX_RETRIES: usize = 5;
    let mut base_delay: std::time::Duration = std::time::Duration::from_secs(2);

    for attempt in 0..MAX_RETRIES {
        match client
            .list_tags(reference, auth, max_tags, last_tag)
            .await
        {
            Ok(tags) => return Ok(tags.tags),
            Err(e) => {
                if !is_rate_limited(&e) {
                    return Err(e);
                }
                if attempt < MAX_RETRIES - 1 {
                    log::warn!(
                        "Rate limited fetching tags for {}: {}. Retrying in {:?} (attempt {}/{})",
                        reference.repository(),
                        e,
                        base_delay,
                        attempt + 1,
                        MAX_RETRIES,
                    );
                    tokio::time::sleep(base_delay).await;
                    base_delay *= 2;
                } else {
                    return Err(e);
                }
            }
        }
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

     let mut all_tags = Vec::new();
     let mut last_tag = reference.tag().map(|s| s.to_string());
     let mut attempt_count = 0;
     const MAX_ATTEMPTS: usize = 20;
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
         )
         .await?;

         log::info!("Available tags for {}: {:?}", reference, tags);
         log::info!("Number of tags: {}", tags.len());

         all_tags.extend(tags.clone());

         if tags.len() >= 100 {
             if let Some(latest) = tags.iter().max() {
                 last_tag = Some(latest.clone());
                 log::info!("Got 1000 results, continuing with last tag: {}", latest);
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
