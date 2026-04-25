use futures::SinkExt;
use url::Url;
use crate::config::{Ntfy, Settings};
use crate::models::models::Workload;
use ntfy::payload::{Action, ActionType};
use ntfy::{dispatcher, Auth, Dispatcher, Payload, Priority};
use ntfy::error::Error as NtfyError;

pub async fn notify_commit(workload: &Workload) -> Result<(), NtfyError> {
    //get settings
    match load_settings() {
        Ok(settings) => {
            let url = settings.url;
            let topic = settings.topic;
            let token = settings.token;


            let dispatcher = dispatcher::builder(&url)
                .credentials(Auth::credentials("", &token)) // Add optional credentials
                .build_blocking()?; // Build dispatcher

            //let action = Action::new(
            //    ActionType::Http,
            //    "Acknowledge",
            //    Url::parse(&url)?,
            //);

            //make message for payload about new container update
            let message = format!(
                "Deployment {} has been updated to version {}",
                workload.name, workload.latest_version
            );

            let payload = Payload::new(&topic)
                .message(message) // Add optional message
                .title(&workload.name) // Add optiona title
                .tags(["Update"]) // Add optional tags
                .priority(Priority::Default) // Edit priority
                //.actions([action]) // Add optional actions
                //.click(Url::parse("https://example.com")?) // Add optional clickable url
                //.attach(Url::parse("https://example.com/file.jpg")?) // Add optional url attachment
                //.delay(Local::now() + Duration::minutes(1)) // Add optional delay
                .markdown(true); // Use markdown

            match dispatcher.send(&payload) {
                Ok(_) => log::info!("Payload sent successfully."),
                Err(e) => log::error!("Failed to send payload: {}", e),
            }
            log::info!("Notification sent");
            Ok(())// Proceed with using settings
        },
        Err(e) => {
            log::info!("Failed to load settings: {}", e);
            Ok(())
            // Handle the error, e.g., by returning or panicking
        }
    }


}

pub async fn send_batch_notification(workloads: &[Workload]) -> Result<(), NtfyError> {
    if workloads.is_empty() {
        log::info!("No updates to report");
        return Ok(());
    }

    match load_settings() {
        Ok(settings) => {
            let url = settings.url.clone();
            let topic = settings.topic.clone();
            let token = settings.token.clone();
            let callback_url = settings.callback_url.clone();

            let dispatcher = dispatcher::builder(&url)
                .credentials(Auth::credentials("", &token))
                .build_blocking()?;

            // Build message
            let mut message = "**Update Available**\n\n".to_string();
            for w in workloads {
                message.push_str(&format!(
                    "- **{}**: {} → {}\n",
                    w.name, w.current_version, w.latest_version
                ));
            }

            // Build actions if callback_url is configured
            let actions: Vec<Action> = if let Some(ref callback_base) = callback_url {
                workloads
                    .iter()
                    .filter_map(|w| {
                        let action_url = format!(
                            "{}/api/ntfy/callback?action={}&namespace={}",
                            callback_base, w.name, w.namespace
                        );
                        Url::parse(&action_url).ok().map(|url| {
                            Action::new(ActionType::Http, "Upgrade", url)
                        })
                    })
                    .collect()
            } else {
                Vec::new()
            };

            let mut payload = Payload::new(&topic)
                .message(message)
                .title("SlackWatch Updates")
                .tags(["Update"])
                .priority(Priority::High)
                .markdown(true);

            if !actions.is_empty() {
                payload = payload.actions(actions);
            }

            match dispatcher.send(&payload) {
                Ok(_) => log::info!("Batch notification sent successfully."),
                Err(e) => log::error!("Failed to send batch notification: {}", e),
            }

            Ok(())
        }
        Err(e) => {
            log::info!("Failed to load settings: {}", e);
            Ok(())
        }
    }
}

fn load_settings() -> Result<Ntfy, String> {
    //get settings
    let settings = Settings::new().unwrap_or_else(|err| {
        log::error!("Failed to load settings: {}", err);
        panic!("Failed to load settings: {}", err);
    });
    if let Some(notifications) = settings.notifications {
        if let Some(ntfy_config) = notifications.ntfy {
            return Ok(ntfy_config.clone());
        } else {
            return Err("No Ntfy Config Found".to_string());
        }
    } else {
        return Err("No Notifications Config Found".to_string());
    }

}

pub async fn send_notification(workload: &Workload) -> Result<(), NtfyError> {
    //get settings
    match load_settings() {
        Ok(settings) => {
            let url = settings.url;
            let topic = settings.topic;
            let token = settings.token;

            let mut dispatcher = dispatcher::builder(&url)
                .credentials(Auth::credentials("", &token)) // Add optional credentials
                .build_blocking();

            //let action = Action::new(
            //    ActionType::Http,
            //    "Acknowledge",
            //    Url::parse(&url)?,
            //);

            //make message for payload about new container update
            let message = format!(
                "Update Available: {} From {} to {}",
                workload.name, workload.current_version, workload.latest_version
            );

            let payload = Payload::new(&topic)
                .message(message) // Add optional message
                .title(&workload.name) // Add optiona title
                .tags(["Update"]) // Add optional tags
                .priority(Priority::High) // Edit priority
                //.actions([action]) // Add optional actions
                //.click(Url::parse("https://example.com")?) // Add optional clickable url
                //.attach(Url::parse("https://example.com/file.jpg")?) // Add optional url attachment
                //.delay(Local::now() + Duration::minutes(1)) // Add optional delay
                .markdown(true); // Use markdown

            match dispatcher?.send(&payload) {
                Ok(_) => log::info!("Payload sent successfully."),
                Err(e) => log::error!("Failed to send payload: {}", e),
            }
            log::info!("Notification sent");
            Ok(()) // Proceed with using settings
        },
        Err(e) => {
            log::info!("Failed to load settings: {}", e);
            Ok(()) // Handle the error, e.g., by returning or panicking
            // Handle the error, e.g., by returning or panicking
        }
    }


}
