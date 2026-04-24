# Ntfy Upgrade Callbacks - Design

## Problem

SlackWatch currently sends ntfy notifications when container image updates are available, but users must manually trigger upgrades through the web UI. There's no way to upgrade directly from the notification.

## Solution

Add interactive action buttons to ntfy notifications that allow users to trigger an upgrade directly from the notification. Each workload with an available update gets its own "Upgrade" button.

## Design Decisions

### 1. Notification Format

A single ntfy notification lists all workloads with available updates. Each workload gets an individual "Upgrade" action button:

```
**Update Available**

- **web-app**: 1.2.0 → 1.3.0 [Upgrade](/api/ntfy/callback?action=web-app)
- **api-server**: 2.1.0 → 2.2.0 [Upgrade](/api/ntfy/callback?action=api-server)
```

### 2. Callback Endpoint

A new POST endpoint at `/api/ntfy/callback` that:
1. Reads the `action` query parameter (workload name)
2. Looks up the workload from the database
3. Runs GitOps operations (clone → edit → commit → push)
4. Sends a confirmation notification
5. If auto-rescan is enabled, schedules a re-scan after a configurable delay

### 3. Auto-Rescan

After a successful GitOps upgrade, SlackWatch can automatically re-scan the workload to verify the new version is detected. A delay is added to give Kubernetes time to pull the new image and roll out the deployment.

**Config:** `notifications.ntfy.auto_rescan_delay`
- `"5m"` (default) — re-scan after 5 minutes
- `"1m"`, `"10m"`, etc. — configurable delay
- `"off"` — disable auto-rescan entirely

### 4. Security

No authentication on the callback endpoint. The ntfy server is in the same Kubernetes cluster and the callback URL is internal-only.

### 5. Error Handling

- Workload not found in DB → error message returned to ntfy
- GitOps fails → error logged, failure message returned to ntfy
- Re-scan fails → GitOps result is still reported (partial success)

## Files Changed

| File | Change |
|------|--------|
| `src/config.rs` | Add `auto_rescan_delay` field to `Ntfy` struct |
| `src/notifications/ntfy.rs` | Add action buttons to notifications; add callback handler |
| `src/api.rs` | Add `/api/ntfy/callback` route |
| `src/models/models.rs` | No changes |
| `src/services/workloads.rs` | Add delayed re-scan function |
| `docs/configuration.md` | Document new setting |

## Bug Fix

Fix syntax error in `src/gitops/gitops.rs:267` — stray `}` that prematurely closes `run_git_operations`, leaving `run_git_operations_internal` unreachable.
