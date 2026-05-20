use oci_distribution::client::{Client, ClientConfig};
use oci_distribution::secrets::RegistryAuth;
use oci_distribution::Reference;

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
         let tags = client
             .list_tags(&reference, &auth, max_tags, last_tag.as_deref())
             .await?;

         log::info!("Available tags for {}: {:?}", reference, tags.tags);
         log::info!("Number of tags: {}", tags.tags.len());

         all_tags.extend(tags.tags.clone());

         if tags.tags.len() >= 100 {
             if let Some(latest) = tags.tags.iter().max() {
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
