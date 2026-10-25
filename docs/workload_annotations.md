# Workload Annotations

Only workloads with the annotation `slackwatch.enabled` set to `true` will be watched by slackwatch.

Additional annotations can be set to customize the behavior of slackwatch for a given workload.

## Main annotations

### `slackwatch.enabled`
description: Set to `true` to enable slackwatch for this workload. If this annotation is not present or set to `false`, slackwatch will not watch this workload.


### `slackwatc.include`
description: A comma-seperated list of regex patterns to apply to tags. Only tags which match will be evaluated by slackwatch.

### `slackwatch.exclude`
description: A comma-seperated list of regex patterns to apply to tags. Tags which match will be ignored by slackwatch during evaluation.


## If using automated gitops commits

### `slackwatch.repo`
description: The name of the gitops configuration to use for syncing changes. This should match the `name` field in the gitops configuration.

### `slackwatch.directory`
description: The directory which your application deployment files are located, within your repo. By default it expects the name of the workload to match the directory name. Slackwatch will walk subdirectories below this directory to find deployment files containing the expected tag. This is only used when `slackwatch.repo` is defined.

### `slackwatch.autoupgrade`
description: Opt this workload into automatic upgrades. On every scan, SlackWatch picks the highest tag the policy allows and commits it to the GitOps repo (the same commit the **Upgrade** button makes). Requires `slackwatch.repo` to match a `[[gitops]]` entry. Tags filtered out by `slackwatch.include` / `slackwatch.exclude` are never auto-applied.

| Value | Auto-applies | From `1.2.3` with `1.2.9`, `1.4.0`, `2.0.0` published |
|-------|--------------|------------------------------------------------------|
| `patch` | newer patch releases within the same `major.minor` | `1.2.9` |
| `minor` | newer minor/patch releases within the same major | `1.4.0` |
| `major` (aliases `all`, `true`) | any newer release | `2.0.0` |
| `none` (aliases `false`, `off`, or annotation absent) | nothing — notify only (default) | — |

Notes:
- The policy selects the best version *within* its range, so a `minor` workload still moves to `1.4.0` even though `2.0.0` exists. The larger update is still shown in the UI and the ntfy notification for a manual upgrade.
- Pre-release tags (e.g. `1.3.0-rc.1`, `1.2.4-alpine`) are only auto-applied if the workload already runs a pre-release.
- Workloads whose current tag isn't a version (e.g. `latest`) are never auto-upgraded.
- Versions are compared literally: for `0.x` images, `minor` allows `0.3.0 → 0.9.0`. Use `patch` if minor bumps of a `0.x` image may be breaking.
- Each auto-upgraded tag is committed only once per workload, even while your GitOps tool is still rolling it out. If you roll a workload back, SlackWatch won't re-apply that same tag — only a newer one.
- An invalid value is shown as an error on the workload card and auto-upgrade is skipped; scanning works as normal.
- All auto-upgrades can be paused globally with [`system.auto_upgrade_enabled = false`](configuration.md#auto_upgrade_enabled).

Example:
```yaml
metadata:
  annotations:
    slackwatch.enable: "true"
    slackwatch.repo: "my-gitops-repo"
    slackwatch.directory: "apps/jellyfin"
    slackwatch.autoupgrade: "minor"
```
