use futures::SinkExt;
use url::Url;
use crate::config::{Ntfy, Settings};
use crate::models::models::Workload;
use ntfy::payload::{Action, ActionType};
use ntfy::{dispatcher, Auth, Dispatcher, Payload, Priority};
use ntfy::error::Error as NtfyError;

pub async fn notify_commit(workload: &Workload) -> Result<(), NtfyError> {
    match load_settings() {
        Ok(settings) => {
            let url = settings.url;
            let topic = settings.topic;
            let token = settings.token;

            let dispatcher = dispatcher::builder(&url)
                .credentials(Auth::credentials("", &token))
                .build_blocking()?;

            let message = format!(
                "Deployment {} has been updated to version {}",
                workload.name, workload.latest_version
            );

            let payload = Payload::new(&topic)
                .message(message)
                .title(&workload.name)
                .tags(["Update"])
                .priority(Priority::Default)
                .markdown(true);

            match dispatcher.send(&payload) {
                Ok(_) => log::info!("Payload sent successfully."),
                Err(e) => log::error!("Failed to send payload: {}", e),
            }
            log::info!("Notification sent");
            Ok(())
        },
        Err(e) => {
            log::info!("Failed to load settings: {}", e);
            Ok(())
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
            log::info!("Ntfy callback_url configured: {:?}", settings.callback_url);

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
                        let mut action_url = format!(
                            "{}/api/ntfy/callback?action={}&namespace={}",
                            callback_base, w.name, w.namespace
                        );
                        if let Some(ref token) = settings.callback_token {
                            action_url = format!("{}&token={}", action_url, token);
                        }
                        Url::parse(&action_url).ok().map(|url| {
                            Action::new(ActionType::Http, "Upgrade", url)
                        })
                    })
                    .collect()
            } else {
                Vec::new()
            };

            log::info!("Built {} action buttons for batch notification", actions.len());

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

use std::time::Duration;
use tokio::time::sleep;

pub fn parse_duration(s: &str) -> Option<Duration> {
    if s == "off" {
        return None;
    }

    let s = s.trim();
    if let Some(num) = s.strip_suffix('m') {
        if let Ok(minutes) = num.parse::<u64>() {
            return Some(Duration::from_secs(minutes * 60));
        }
    }
    if let Some(num) = s.strip_suffix('h') {
        if let Ok(hours) = num.parse::<u64>() {
            return Some(Duration::from_secs(hours * 3600));
        }
    }
    None
}

pub async fn schedule_rescan(workload: Workload, delay: &str) {
    if let Some(duration) = parse_duration(delay) {
        let wl = workload.clone();
        log::info!("Scheduling re-scan for {} in {:?}", wl.name, duration);
        tokio::spawn(async move {
            sleep(duration).await;
            let name = wl.name.clone();
            log::info!("Re-scanning workload {}", name);
            if let Err(e) = crate::services::workloads::update_single_workload(wl).await {
                log::error!("Re-scan failed for workload {}: {}", name, e);
            }
        });
    } else {
        log::info!("Auto-rescan disabled or invalid delay: {}", delay);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_duration_minutes() {
        assert_eq!(parse_duration("5m"), Some(Duration::from_secs(300)));
        assert_eq!(parse_duration("10m"), Some(Duration::from_secs(600)));
    }

    #[test]
    fn test_parse_duration_hours() {
        assert_eq!(parse_duration("1h"), Some(Duration::from_secs(3600)));
    }

    #[test]
    fn test_parse_duration_off() {
        assert_eq!(parse_duration("off"), None);
    }

    #[test]
    fn test_parse_duration_invalid() {
        assert_eq!(parse_duration("invalid"), None);
    }
}
