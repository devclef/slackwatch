use oci_distribution::client::{Client, ClientConfig};
use oci_distribution::secrets::RegistryAuth;
use oci_distribution::Reference;

//pub async fn test_call() -> Result<Vec<String>, Box<dyn std::error::Error>> {
//    let reference = Reference::try_from("binwiederhier/ntfy")?;
//    let auth = RegistryAuth::Anonymous;
//    let config = ClientConfig::default();
//    let mut client = Client::new(config);
//
//    let tags = client.list_tags(&reference, &auth, None, None).await?;
//    //sort tags
//    let mut tags = tags.tags;
//    tags.sort();
//    let first_tag = tags.first().unwrap();
//    let last_tag = tags.last().unwrap();
//    println!("Available tags for {}: {:?}", reference, tags);
//    println!("First tag: {}", first_tag);
//    println!("Last tag: {}", last_tag);
//
//    Ok(tags)
//}

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
         // Prevent infinite loop by limiting the number of attempts
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

         // Add the tags to our collection
         all_tags.extend(tags.tags.clone());

         // If we got exactly 1000 tags, it might indicate we hit a limit
         // Take the latest tag and rerun the query
         if tags.tags.len() >= 100 {
             // Find the latest tag (alphabetically last)
             if let Some(latest) = tags.tags.iter().max() {
                 last_tag = Some(latest.clone());
                 log::info!("Got 1000 results, continuing with last tag: {}", latest);
             } else {
                 // This shouldn't happen as we just checked there are 1000 tags
                 break;
             }
         } else {
             // We got fewer than 1000 tags, we're done
             break;
         }
     }

     log::info!("Total number of tags collected: {}", all_tags.len());

     Ok((all_tags, exhausted))
 }
