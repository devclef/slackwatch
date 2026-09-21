use futures::SinkExt;
use url::Url;
use crate::config::{Ntfy, Settings};
use crate::models::Workload;
use ntfy::payload::{Action, ActionType};
use ntfy::{dispatcher, Auth, Dispatcher, Payload, Priority};
use ntfy::error::Error as NtfyError;

/// Maximum number of action buttons the ntfy server accepts per message.
///
/// Publishing more than this makes the server reject the *entire* message
/// with HTTP 400 ("invalid request: actions invalid"), so the whole batch
/// notification is lost. See https://docs.ntfy.sh/publish/#action-buttons
const NTFY_MAX_ACTIONS: usize = 3;

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
            let callback_token = settings.callback_token.clone();

            let dispatcher = dispatcher::builder(&url)
                .credentials(Auth::credentials("", &token))
                .build_blocking()?;

            // ntfy rejects messages with more than NTFY_MAX_ACTIONS action
            // buttons, so split the updates into chunks and send one
            // notification per chunk.
            let payloads = build_batch_payloads(workloads, &topic, &callback_url, &callback_token);

            for (i, payload) in payloads.iter().enumerate() {
                let part = i + 1;
                match dispatcher.send(payload) {
                    Ok(_) => log::info!("Batch notification {}/{} sent successfully.", part, payloads.len()),
                    Err(e) => log::error!("Failed to send batch notification {}/{}: {}", part, payloads.len(), e),
                }
            }

            Ok(())
        }
        Err(e) => {
            log::info!("Failed to load settings: {}", e);
            Ok(())
        }
    }
}

/// Split a list of workloads with available updates into one or more ntfy
/// payloads, each carrying at most [`NTFY_MAX_ACTIONS`] action buttons, since
/// the ntfy server rejects messages with more than that (HTTP 400). When
/// `callback_url` is configured every workload gets an "Upgrade" action
/// button pointing at the callback API.
fn build_batch_payloads(
    workloads: &[Workload],
    topic: &str,
    callback_url: &Option<String>,
    callback_token: &Option<String>,
) -> Vec<Payload> {
    let chunks: Vec<&[Workload]> = workloads.chunks(NTFY_MAX_ACTIONS).collect();
    let total = chunks.len();

    chunks
        .iter()
        .enumerate()
        .map(|(i, chunk)| {
            let part = i + 1;
            let title = if total > 1 {
                format!("SlackWatch Updates ({}/{})", part, total)
            } else {
                "SlackWatch Updates".to_string()
            };

            // Build message
            let mut message = "**Update Available**\n\n".to_string();
            for w in *chunk {
                message.push_str(&format!(
                    "- **{}**: {} → {}\n",
                    w.name, w.current_version, w.latest_version
                ));
            }

            // Build actions if callback_url is configured
            let actions: Vec<Action> = if let Some(ref callback_base) = callback_url {
                chunk
                    .iter()
                    .filter_map(|w| {
                        let mut action_url = format!(
                            "{}/api/ntfy/callback?action={}&namespace={}&latest_version={}",
                            callback_base, w.name, w.namespace, w.latest_version
                        );
                        if let Some(ref token) = callback_token {
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

            log::info!(
                "Built {} action buttons for batch notification {}/{}",
                actions.len(),
                part,
                total
            );

            let mut payload = Payload::new(topic)
                .message(message)
                .title(&title)
                .tags(["Update"])
                .priority(Priority::High)
                .markdown(true);

            if !actions.is_empty() {
                payload = payload.actions(actions);
            }
            payload
        })
        .collect()
}

fn load_settings() -> Result<Ntfy, String> {
    let settings = Settings::new().unwrap_or_else(|err| {
        log::error!("Failed to load settings: {}", err);
        panic!("Failed to load settings: {}", err);
    });
    if let Some(notifications) = settings.notifications {
        if let Some(ntfy_config) = notifications.ntfy {
            Ok(ntfy_config.clone())
        } else {
            Err("No Ntfy Config Found".to_string())
        }
    } else {
        Err("No Notifications Config Found".to_string())
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
    use crate::models::UpdateStatus;

    fn test_workload(name: &str) -> Workload {
        Workload {
            name: name.to_string(),
            exclude_pattern: None,
            git_ops_repo: None,
            include_pattern: None,
            update_available: UpdateStatus::Available,
            git_directory: None,
            image: format!("docker.io/example/{}", name),
            last_scanned: "2026-09-21T00:00:00Z".to_string(),
            namespace: "default".to_string(),
            current_version: "1.0.0".to_string(),
            latest_version: "1.1.0".to_string(),
            scan_exhausted: "False".to_string(),
            error: None,
        }
    }

    fn action_count(payload: &Payload) -> usize {
        payload.actions.as_ref().map(|a| a.len()).unwrap_or(0)
    }

    #[test]
    fn test_build_batch_payloads_single_chunk() {
        let workloads: Vec<Workload> = (0..3).map(|i| test_workload(&format!("wl{}", i))).collect();
        let payloads = build_batch_payloads(&workloads, "topic", &None, &None);

        assert_eq!(payloads.len(), 1);
        assert_eq!(payloads[0].title.as_deref(), Some("SlackWatch Updates"));
        assert!(action_count(&payloads[0]) == 0);
        assert!(payloads[0].message.contains("**wl0**"));
        assert!(payloads[0].message.contains("**wl2**"));
    }

    #[test]
    fn test_build_batch_payloads_never_exceeds_ntfy_action_limit() {
        // ntfy rejects messages with more than NTFY_MAX_ACTIONS actions
        let workloads: Vec<Workload> = (0..8).map(|i| test_workload(&format!("wl{}", i))).collect();
        let callback_url = Some("https://slackwatch.example.com".to_string());
        let payloads = build_batch_payloads(&workloads, "topic", &callback_url, &None);

        assert_eq!(payloads.len(), 3);
        for payload in &payloads {
            assert!(
                action_count(payload) <= NTFY_MAX_ACTIONS,
                "payload has more than {} actions",
                NTFY_MAX_ACTIONS
            );
            assert!(!payload.message.is_empty());
        }
        // 8 workloads -> 3 + 3 + 2 actions, none dropped
        let total_actions: usize = payloads.iter().map(action_count).sum();
        assert_eq!(total_actions, 8);
        assert_eq!(payloads[0].title.as_deref(), Some("SlackWatch Updates (1/3)"));
        assert_eq!(payloads[1].title.as_deref(), Some("SlackWatch Updates (2/3)"));
        assert_eq!(payloads[2].title.as_deref(), Some("SlackWatch Updates (3/3)"));
    }

    #[test]
    fn test_build_batch_payloads_action_urls_include_token() {
        let workloads: Vec<Workload> = vec![test_workload("wl0")];
        let callback_url = Some("https://slackwatch.example.com".to_string());
        let callback_token = Some("secret-token".to_string());
        let payloads = build_batch_payloads(&workloads, "topic", &callback_url, &callback_token);

        assert_eq!(payloads.len(), 1);
        let actions = payloads[0].actions.as_ref().unwrap();
        assert_eq!(actions.len(), 1);
        let url = actions[0].url.as_str();
        assert!(url.starts_with("https://slackwatch.example.com/api/ntfy/callback?"));
        assert!(url.contains("action=wl0"));
        assert!(url.contains("namespace=default"));
        assert!(url.contains("latest_version=1.1.0"));
        assert!(url.contains("token=secret-token"));
        assert_eq!(actions[0].label, "Upgrade");
    }

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
