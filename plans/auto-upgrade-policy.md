# Feature: Policy-based auto-upgrade for watched workloads

## Context

Today SlackWatch only *notifies* about updates; a human must click **Upgrade**
(UI or ntfy action) to commit the new tag to the GitOps repo. Users want
SlackWatch to apply safe updates on its own, while choosing how far it may
go — e.g. "take every minor/patch release, but never a new major".

## Design

### Per-workload opt-in annotation

`slackwatch.autoupgrade: <policy>` on the pod (next to the existing
`slackwatch.repo` / `slackwatch.directory` annotations, which are required for
the commit to happen).

| Policy                      | Auto-applies                          | Example from `1.2.3`        |
|-----------------------------|---------------------------------------|-----------------------------|
| `patch`                     | same `major.minor`, newer patch       | `1.2.9` ✅  `1.3.0` ❌      |
| `minor`                     | same major, newer minor/patch         | `1.9.0` ✅  `2.0.0` ❌      |
| `major` (alias `all`, `true`) | anything newer                      | `2.0.0` ✅                  |
| `none` / `false` / `off` / absent | nothing (current behaviour)      | —                           |

Rules:

- The policy picks the **highest tag allowed by the policy**, not just "is the
  overall latest within the policy". From `1.2.3` with `1.3.0` and `2.0.0`
  published, `minor` auto-upgrades to `1.3.0`; the notification / UI still
  advertise `2.0.0` as the latest available for a manual upgrade.
- Candidate tags are the same ones the scan already uses (after
  `slackwatch.include` / `slackwatch.exclude` filtering), so those annotations
  further constrain auto-upgrades.
- Pre-release tags (`1.3.0-beta`, `1.2.4-alpine`) are never auto-applied unless
  the workload is already running a pre-release.
- Versions are compared literally (no special-casing of `0.x`), documented.
- An unknown policy value is reported in the workload's error field and
  auto-upgrade is skipped; the normal scan result is unaffected.

### Global switch

`system.auto_upgrade_enabled` (default `true`, env
`SLACKWATCH_SYSTEM.AUTO_UPGRADE_ENABLED`) — a kill switch to pause all
auto-upgrades without editing annotations.

### Avoiding repeated commits

After a commit, the pod keeps reporting the old tag until the GitOps tool
syncs, so the next scan would compute the same target again. A new SQLite
table `auto_upgrades` records each attempt
(`name, namespace, from_version, to_version, status, error, created_at`).
A target that already has a `success` row for that workload is skipped. This
also respects deliberate rollbacks: if a user rolls back, SlackWatch won't
re-apply the same version, only a newer one. Failed attempts are retried on
the next scan. A process-wide async mutex serialises auto-upgrades (the git
code reuses `/tmp/repos`).

Before committing, the workload's `slackwatch.repo` must match a `[[gitops]]`
entry; otherwise the attempt is recorded as failed with a clear message
(`run_git_operations` would otherwise silently no-op).

### Flow

`fetch_and_update_all_watched` and `update_single_workload`:

1. Scan as today (`scan_workload` = existing `parse_tags` logic plus the
   auto-upgrade target computed from the same filtered tags).
2. Store the scan in the DB as today.
3. Run eligible auto-upgrades via the existing `run_git_operations` (which
   already sends the ntfy "has been updated" commit notification).
4. Send the batch "Update Available" notification as today, minus workloads
   whose auto-upgrade succeeded *to the overall latest version* (nothing left
   for the user to do). Workloads auto-upgraded to a lower version than the
   latest stay in the notification so the user can still take the bigger jump.

### Backwards compatibility

- No annotation → `Disabled` → identical behaviour to today.
- `Workload` gains `auto_upgrade: Option<String>` with `#[serde(default)]`, so
  older frontends / API clients still deserialize.
- DB: `workloads` gets an `auto_upgrade` column added via `ALTER TABLE` when
  missing (existing DBs keep working); read as an optional column.
- `parse_tags` keeps its signature (thin wrapper over `scan_workload`).
- Manual upgrade (UI / ntfy) untouched.

## Files

- `src/services/auto_upgrade.rs` (new) — `AutoUpgradePolicy` parsing, target
  selection, dedupe + apply logic, unit tests.
- `src/services/workloads.rs` — `scan_workload`, call auto-upgrade, filter the
  batch notification.
- `src/services/mod.rs` — register module.
- `src/models/mod.rs` — `Workload.auto_upgrade`.
- `src/kubernetes/client.rs` — read `slackwatch.autoupgrade`.
- `src/database/client.rs` — column migration, `auto_upgrades` table and
  helpers (connection-parameterised for tests).
- `src/config.rs` — `system.auto_upgrade_enabled`.
- Struct literals in `gitops`/`ntfy` tests.
- Frontend: `types.ts` + `WorkloadCard.tsx` show the policy badge; CSS.
- Docs: `docs/workload_annotations.md`, `docs/configuration.md`, `CLAUDE.md`.

## Verification

- `cargo build`, `cargo test` (new unit tests for policy parsing, target
  selection incl. prerelease/`v`-prefix/include-exclude interplay, dedupe DB
  helpers, column migration on an old-schema DB), `cargo clippy --all-features`
  (no new warnings).
- `cd frontend && npm test && npm run build`.
- `docker build` path unchanged (no new system deps).
