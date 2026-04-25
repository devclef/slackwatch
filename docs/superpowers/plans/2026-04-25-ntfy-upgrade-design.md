# Ntfy Upgrade Callbacks Implementation Plan

> **Goal:** Add interactive upgrade buttons to ntfy notifications that allow users to trigger GitOps upgrades directly from notifications, with optional delayed auto-rescan.

> **Architecture:** A single ntfy notification lists all workloads with available updates. Each workload gets an "Upgrade" action button that calls a new `/api/ntfy/callback` endpoint. The endpoint runs GitOps operations, sends a confirmation notification, and optionally schedules a delayed re-scan.

> **Tech Stack:** Rust, warp (HTTP), ntfy (notifications), tokio (async), rusqlite (database)

---

## File Structure

| File | Responsibility |
|------|---------------|
| `Cargo.toml` | Add `url` crate dependency for constructing action URLs |
| `src/config.rs` | Add `callback_url` and `auto_rescan_delay` fields to `Ntfy` struct |
| `src/notifications/ntfy.rs` | Add `send_batch_notification`, `parse_duration`, `schedule_rescan` functions |
| `src/services/workloads.rs` | Modify `fetch_and_update_all_watched` to use batch notification |
| `src/api.rs` | Add `/api/ntfy/callback` route and handler |
| `src/gitops/gitops.rs` | Fix syntax error in `run_git_operations` |
| `docs/configuration.md` | Document new config fields |

---

### Task 0: Add url crate dependency

**Files:**
- Modify: `Cargo.toml`

- [ ] **Step 1: Add url dependency**

Add the following line to the `[dependencies]` section of `Cargo.toml` (alphabetically ordered):

```toml
url = "2"
```

- [ ] **Step 2: Verify build**

Run: `cargo check`
Expected: No errors

- [ ] **Step 3: Commit**

```bash
git add Cargo.toml
git commit -m "deps: add url crate for ntfy action callback URLs"
```

---

### Task 1: Add config fields and tests

**Files:**
- Modify: `src/config.rs`

- [ ] **Step 1: Add config fields to Ntfy struct**

In `src/config.rs`, modify the `Ntfy` struct and add a default function:

```rust
// Change the existing Ntfy struct from:
#[derive(Debug, Deserialize, Serialize, Clone)]
#[allow(unused)]
pub struct Ntfy {
    pub url: String,
    pub topic: String,
    pub reminder: String,
    pub token: String,
}

// To:
#[derive(Debug, Deserialize, Serialize, Clone)]
#[allow(unused)]
pub struct Ntfy {
    pub url: String,
    pub topic: String,
    pub reminder: String,
    pub token: String,
    #[serde(default)]
    pub callback_url: Option<String>,
    #[serde(default = "default_auto_rescan_delay")]
    pub auto_rescan_delay: String,
}

fn default_auto_rescan_delay() -> String {
    "5m".to_string()
}
```

- [ ] **Step 2: Add tests for config parsing**

Add these tests to the existing `#[cfg(test)]` module in `src/config.rs`:

```rust
#[test]
fn test_ntfy_config_with_new_fields() {
    let dir = tempdir().unwrap();
    let config_path = dir.path().join("config.toml");
    let mut file = File::create(&config_path).unwrap();
    writeln!(
        file,
        r#"
        [system]
        schedule = "0 0 * * * *"
        data_dir = "/tmp/data"

        [notifications.ntfy]
        url = "http://ntfy.example.com"
        topic = "updates"
        reminder = "24h"
        token = "secrettoken"
        callback_url = "http://slackwatch:8080"
        auto_rescan_delay = "10m"
        "#
    )
    .unwrap();

    std::env::set_var("SLACKWATCH_CONFIG", config_path.to_str().unwrap());
    let settings = Settings::new().expect("Settings should load successfully");
    let ntfy = settings.notifications.unwrap().ntfy.unwrap();
    assert_eq!(ntfy.callback_url, Some("http://slackwatch:8080".to_string()));
    assert_eq!(ntfy.auto_rescan_delay, "10m");
}

#[test]
fn test_ntfy_config_default_auto_rescan_delay() {
    let dir = tempdir().unwrap();
    let config_path = dir.path().join("config.toml");
    let mut file = File::create(&config_path).unwrap();
    writeln!(
        file,
        r#"
        [system]
        schedule = "0 0 * * * *"
        data_dir = "/tmp/data"

        [notifications.ntfy]
        url = "http://ntfy.example.com"
        topic = "updates"
        reminder = "24h"
        token = "secrettoken"
        "#
    )
    .unwrap();

    std::env::set_var("SLACKWATCH_CONFIG", config_path.to_str().unwrap());
    let settings = Settings::new().expect("Settings should load successfully");
    let ntfy = settings.notifications.unwrap().ntfy.unwrap();
    assert_eq!(ntfy.auto_rescan_delay, "5m");
}
```

- [ ] **Step 3: Run tests**

Run: `cargo test test_ntfy_config`
Expected: Both tests pass

- [ ] **Step 4: Commit**

```bash
git add src/config.rs
git commit -m "feat: add callback_url and auto_rescan_delay config fields"
```

---

### Task 2: Fix gitops.rs syntax error

**Files:**
- Modify: `src/gitops/gitops.rs`

- [ ] **Step 1: Fix syntax error**

In `src/gitops/gitops.rs`, the `run_git_operations` function is missing its closing brace. The match block closes on line 224, but the function body is not closed. Additionally, there's an extra closing brace on line 267.

Apply this edit to fix the issue:

```rust
// Before (lines 212-227):
pub async fn run_git_operations(workload: Workload) -> Result<(), Box<dyn Error>> {
    match load_settings() {
        Ok(settings) => {
            log::info!("Settings: {:?}", settings);
            return Ok(run_git_operations_internal(settings, workload).await?);
        }
        Err(e) => {
            log::info!("Failed to load settings: {}", e);
            return Ok(());
        }
    }

async fn run_git_operations_internal(

// After:
pub async fn run_git_operations(workload: Workload) -> Result<(), Box<dyn Error>> {
    match load_settings() {
        Ok(settings) => {
            log::info!("Settings: {:?}", settings);
            return Ok(run_git_operations_internal(settings, workload).await?);
        }
        Err(e) => {
            log::info!("Failed to load settings: {}", e);
            return Ok(());
        }
    }
}

async fn run_git_operations_internal(
```

- [ ] **Step 2: Verify build**

Run: `cargo check`
Expected: No errors

- [ ] **Step 3: Commit**

```bash
git add src/gitops/gitops.rs
git commit -m "fix: fix syntax error in run_git_operations function"
```

---

### Task 3: Create send_batch_notification function

**Files:**
- Modify: `src/notifications/ntfy.rs`

- [ ] **Step 1: Add send_batch_notification function**

Add the following function to `src/notifications/ntfy.rs` (after the existing functions, before the `load_settings` function):

```rust
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
```

- [ ] **Step 2: Add url import**

Add `use url::Url;` to the imports at the top of `src/notifications/ntfy.rs`:

```rust
use futures::SinkExt;
use url::Url;
use crate::config::{Ntfy, Settings};
```

- [ ] **Step 3: Verify build**

Run: `cargo check`
Expected: No errors

- [ ] **Step 4: Commit**

```bash
git add src/notifications/ntfy.rs
git commit -m "feat: add send_batch_notification function with action buttons"
```

---

### Task 4: Update workloads.rs to use batch notification

**Files:**
- Modify: `src/services/workloads.rs`

- [ ] **Step 1: Update imports**

Change the existing import:

```rust
// Before:
use crate::notifications::ntfy::send_notification;

// After:
use crate::notifications::ntfy::{send_notification, send_batch_notification};
```

- [ ] **Step 2: Modify fetch_and_update_all_watched**

Replace the entire `fetch_and_update_all_watched` function. Find the existing function (lines 62-111) and replace it with:

```rust
pub async fn fetch_and_update_all_watched() -> Result<(), String> {
    let workloads = find_enabled_workloads().await.map_err(|e| e.to_string())?;
    log::info!("Found {} workloads", workloads.len());

    let scan_id = get_latest_scan_id().unwrap_or(0) + 1;
    let mut updates_available = Vec::new();

    for workload in workloads {
        if let Some(_) = find_latest_tag_for_image(&workload).await {
            let result = parse_tags(&workload).await;
            let workload = match result {
                Ok(w) => w,
                Err(e) => {
                    log::error!("Regex error for workload {}: {}", workload.name, e);
                    Workload {
                        name: workload.name.clone(),
                        exclude_pattern: workload.exclude_pattern.clone(),
                        git_ops_repo: workload.git_ops_repo.clone(),
                        include_pattern: workload.include_pattern.clone(),
                        namespace: workload.namespace.clone(),
                        current_version: workload.current_version.clone(),
                        image: workload.image.clone(),
                        update_available: UpdateStatus::NotAvailable,
                        last_scanned: workload.last_scanned.clone(),
                        latest_version: String::new(),
                        git_directory: workload.git_directory.clone(),
                        scan_exhausted: "False".to_string(),
                        regex_error: Some(e.to_string()),
                    }
                }
            };

            if workload.update_available.to_string() == "Available" {
                updates_available.push(workload);
            }

            std::thread::spawn(move || database::client::insert_workload(&workload, scan_id))
                .join()
                .map_err(|_| "Thread error".to_string())?
                .expect("TODO: panic message");
        } else {
            log::info!("No tags found for image: {}", workload.image);
            std::thread::spawn(move || database::client::insert_workload(&workload, scan_id))
                .join()
                .map_err(|_| "Thread error".to_string())?
                .expect("TODO: panic message");
        }
    }

    // Send batch notification if there are updates
    if !updates_available.is_empty() {
        send_batch_notification(&updates_available)
            .await
            .unwrap_or_else(|e| log::error!("Error sending batch notification: {}", e));
    }

    Ok(())
}
```

- [ ] **Step 3: Verify build**

Run: `cargo check`
Expected: No errors

- [ ] **Step 4: Commit**

```bash
git add src/services/workloads.rs
git commit -m "feat: batch notifications with action buttons for updates"
```

---

### Task 5: Add callback endpoint

**Files:**
- Modify: `src/api.rs`

- [ ] **Step 1: Add imports**

Add these imports to the top of `src/api.rs`:

```rust
use std::collections::HashMap;
use crate::notifications::ntfy::{notify_commit, schedule_rescan};
```

- [ ] **Step 2: Add route in start_api_server**

Add the new route variable after `get_next_schedule`:

```rust
// Add after the get_next_schedule block:
    let ntfy_callback = api
        .and(warp::path("ntfy"))
        .and(warp::path("callback"))
        .and(warp::post())
        .and(warp::query::<HashMap<String, String>>())
        .and_then(handle_ntfy_callback);
```

Update the routes combination to include the new route:

```rust
// Before:
    let routes = get_workloads
        .or(update_workload)
        .or(upgrade_workload)
        .or(refresh_all)
        .or(get_settings)
        .or(get_next_schedule)
        .or(static_files)
        .or(spa_fallback)
        .with(cors);

// After:
    let routes = get_workloads
        .or(update_workload)
        .or(upgrade_workload)
        .or(refresh_all)
        .or(get_settings)
        .or(get_next_schedule)
        .or(ntfy_callback)
        .or(static_files)
        .or(spa_fallback)
        .with(cors);
```

- [ ] **Step 3: Add handler function**

Add this handler function at the end of `src/api.rs` (after `handle_get_next_schedule`):

```rust
async fn handle_ntfy_callback(
    query: HashMap<String, String>,
) -> impl Reply {
    let action = query.get("action").cloned();
    let namespace = query.get("namespace").cloned().unwrap_or_else(|| "default".to_string());

    match action {
        Some(name) => {
            match database::client::return_workload(name.clone(), namespace.clone()) {
                Ok(workload) => {
                    let wl = workload.clone();
                    match run_git_operations(wl.clone()).await {
                        Ok(_) => {
                            let _ = notify_commit(&wl).await;

                            if let Ok(settings) = Settings::new() {
                                if let Some(ref notifications) = settings.notifications {
                                    if let Some(ref ntfy) = notifications.ntfy {
                                        schedule_rescan(wl.clone(), &ntfy.auto_rescan_delay).await;
                                    }
                                }
                            }

                            warp::reply::with_status(
                                format!("Upgrade initiated for {}", name),
                                warp::http::StatusCode::OK,
                            )
                        }
                        Err(e) => {
                            log::error!("GitOps failed for {}: {}", name, e);
                            warp::reply::with_status(
                                format!("Upgrade failed for {}: {}", name, e),
                                warp::http::StatusCode::OK,
                            )
                        }
                    }
                }
                Err(_) => {
                    log::error!("Workload {} not found in DB", name);
                    warp::reply::with_status(
                        format!("Workload {} not found", name),
                        warp::http::StatusCode::OK,
                    )
                }
            }
        }
        None => {
            warp::reply::with_status(
                "No action parameter provided",
                warp::http::StatusCode::OK,
            )
        }
    }
}
```

- [ ] **Step 4: Verify build**

Run: `cargo check`
Expected: No errors

- [ ] **Step 5: Commit**

```bash
git add src/api.rs
git commit -m "feat: add ntfy callback endpoint for upgrade actions"
```

---

### Task 6: Add duration parsing and delayed re-scan with tests

**Files:**
- Modify: `src/notifications/ntfy.rs`

- [ ] **Step 1: Add duration parsing and schedule_rescan**

Add these functions to `src/notifications/ntfy.rs` (after `send_batch_notification`):

```rust
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
            log::info!("Re-scanning workload {}", wl.name);
            if let Err(e) = crate::services::workloads::update_single_workload(wl).await {
                log::error!("Re-scan failed for workload {}: {}", wl.name, e);
            }
        });
    } else {
        log::info!("Auto-rescan disabled or invalid delay: {}", delay);
    }
}
```

- [ ] **Step 2: Add tests for parse_duration**

Add a new test module at the bottom of `src/notifications/ntfy.rs`:

```rust
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
```

- [ ] **Step 3: Run tests**

Run: `cargo test parse_duration`
Expected: All 4 tests pass

- [ ] **Step 4: Verify build**

Run: `cargo check`
Expected: No errors

- [ ] **Step 5: Commit**

```bash
git add src/notifications/ntfy.rs
git commit -m "feat: add delayed auto-rescan with configurable delay"
```

---

### Task 7: Update documentation

**Files:**
- Modify: `docs/configuration.md`

- [ ] **Step 1: Document new config fields**

Add these sections to `docs/configuration.md` after the existing `token` section (around line 93), before the GitOps section:

```markdown
---

#### callback_url
value: string (None)

default: `null`

description: The base URL of the SlackWatch API server. Used to construct callback URLs for ntfy action buttons. This URL must be reachable from the ntfy server. For in-cluster access use the Kubernetes service URL (e.g., `http://slackwatch.slackwatch.svc.cluster.local:8080`). For external access use the public URL (e.g., `https://slackwatch.example.com`).

---

#### auto_rescan_delay
value: string

default: `5m`

description: Duration to wait before automatically re-scanning a workload after an upgrade via ntfy callback. This gives Kubernetes time to pull the new image and roll out the deployment. Use `"off"` to disable auto-rescan. Examples: `"1m"`, `"5m"`, `"10m"`, `"1h"`, `"off"`.

---
```

- [ ] **Step 2: Commit**

```bash
git add docs/configuration.md
git commit -m "docs: document new ntfy config fields"
```

---

## Self-Review

**Spec coverage:**
- Notification format with action buttons → Task 3 (send_batch_notification)
- Callback endpoint at /api/ntfy/callback → Task 5 (handle_ntfy_callback)
- Auto-rescan with configurable delay → Task 6 (parse_duration, schedule_rescan)
- No auth on callback → No changes needed (existing pattern)
- Error handling → Task 5 (callback handler covers all error cases)
- Config fields → Task 1 (callback_url, auto_rescan_delay)
- Bug fix in gitops.rs → Task 2

**Placeholder scan:** No TBD, TODO, or vague requirements. All code is complete.

**Type consistency:** `Workload` is used consistently across all tasks. `Ntfy` struct fields match between config and usage. `Result` types are consistent.

**Ambiguity check:** Callback URL format is explicit (`{callback_url}/api/ntfy/callback?action={name}&namespace={namespace}`). Delay default is 5 minutes. `"off"` disables auto-rescan. All clear.
