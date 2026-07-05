# Fix: Upgrade uses stale `latest_version` from frontend

## Context

When clicking "Upgrade" in the frontend UI, the displayed version (e.g., "0.1.0") gets replaced with "1.0.0" during git operations. The ntfy callback upgrade works correctly because it fetches fresh data from the database.

**Root cause**: `handle_upgrade_workload` in `src/api.rs` uses the `Workload` object sent directly from the frontend React state, which may contain stale `latest_version` data. The frontend caches workload data on page load and sends it unchanged when upgrading.

Additionally, `kubernetes/client.rs:48` has a hardcoded `latest_version: "1.0.0"` placeholder that may propagate to the database during workload discovery.

## Changes

### 1. Fix `handle_upgrade_workload` to fetch from database (`src/api.rs`)

Change the `/api/workloads/upgrade` endpoint to fetch the workload from the database before running git operations, matching the ntfy callback pattern:

```rust
async fn handle_upgrade_workload(workload: Workload) -> Result<impl Reply, Rejection> {
    // Fetch fresh data from database instead of trusting frontend
    let fresh_workload = match return_workload(workload.name, workload.namespace) {
        Ok(wl) => wl,
        Err(e) => {
            log::error!("Failed to fetch workload for upgrade: {}", e);
            let error = json!({ "error": "Workload not found in database" });
            return Ok(warp::reply::with_status(
                warp::reply::json(&error),
                warp::http::StatusCode::NOT_FOUND,
            ));
        }
    };

    match run_git_operations(fresh_workload).await {
        // ... rest unchanged
    }
}
```

This ensures git operations always use the database's `latest_version`, not the frontend's potentially stale value.

### 2. Remove hardcoded "1.0.0" placeholder (`src/kubernetes/client.rs`)

Replace the hardcoded `latest_version: "1.0.0"` with an empty string, since the actual latest version is computed by `parse_tags()` during scan:

```rust
latest_version: String::new(), // Computed during scan via parse_tags
```

## Verification

- Build: `cargo build`
- Tests: `cargo test`
- Manual: Run slackwatch, trigger a scan, verify the UI shows correct version, then click Upgrade and confirm git operations use the correct version (check logs for "New image:" line)
