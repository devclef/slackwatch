use crate::config::Settings;
use crate::database::client::{
    auto_upgrade_already_applied, record_auto_upgrade, AUTO_UPGRADE_FAILED, AUTO_UPGRADE_SUCCESS,
};
use crate::gitops::run_git_operations;
use crate::models::Workload;
use crate::services::workloads::parse_version_loose;
use semver::Version;
use std::str::FromStr;
use tokio::sync::Mutex;

/// How far SlackWatch may move a workload on its own, set per workload via
/// the `slackwatch.autoupgrade` annotation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AutoUpgradePolicy {
    /// Never auto-upgrade (default when the annotation is absent).
    Disabled,
    /// Only newer patch releases within the current `major.minor`.
    Patch,
    /// Newer minor and patch releases within the current major.
    Minor,
    /// Any newer release, including new majors.
    Major,
}

impl FromStr for AutoUpgradePolicy {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "" | "none" | "false" | "off" | "disabled" => Ok(AutoUpgradePolicy::Disabled),
            "patch" => Ok(AutoUpgradePolicy::Patch),
            "minor" => Ok(AutoUpgradePolicy::Minor),
            "major" | "all" | "true" => Ok(AutoUpgradePolicy::Major),
            _ => Err(format!(
                "Invalid slackwatch.autoupgrade policy '{}': expected one of none, patch, minor, major",
                s
            )),
        }
    }
}

impl AutoUpgradePolicy {
    pub fn from_annotation(value: Option<&str>) -> Result<Self, String> {
        value.map_or(Ok(AutoUpgradePolicy::Disabled), str::parse)
    }

    fn allows(self, current: &Version, candidate: &Version) -> bool {
        match self {
            AutoUpgradePolicy::Disabled => false,
            AutoUpgradePolicy::Patch => {
                candidate.major == current.major && candidate.minor == current.minor
            }
            AutoUpgradePolicy::Minor => candidate.major == current.major,
            AutoUpgradePolicy::Major => true,
        }
    }
}

/// Everything before the first digit, e.g. `v` for `v1.2.3`.
fn tag_prefix(tag: &str) -> &str {
    tag.find(|c: char| c.is_ascii_digit())
        .map_or(tag, |i| &tag[..i])
}

/// Returns the highest tag newer than `current_version` that `policy`
/// permits. Pre-release tags are only considered when the workload already
/// runs a pre-release, and an unparseable current version never
/// auto-upgrades (there is nothing to compare the policy against). When the
/// same version is published under several spellings (`1.3.0`, `v1.3.0`),
/// the one matching the current tag's prefix wins.
pub fn select_auto_upgrade_target(
    current_version: &str,
    tags: &[String],
    policy: AutoUpgradePolicy,
) -> Option<String> {
    if policy == AutoUpgradePolicy::Disabled {
        return None;
    }
    let current = parse_version_loose(current_version)?;
    let allow_prerelease = !current.pre.is_empty();
    let current_prefix = tag_prefix(current_version);

    tags.iter()
        .filter_map(|tag| parse_version_loose(tag).map(|v| (v, tag)))
        .filter(|(v, _)| {
            *v > current && (allow_prerelease || v.pre.is_empty()) && policy.allows(&current, v)
        })
        .max_by(|(a, a_tag), (b, b_tag)| {
            a.cmp(b).then_with(|| {
                (tag_prefix(a_tag) == current_prefix).cmp(&(tag_prefix(b_tag) == current_prefix))
            })
        })
        .map(|(_, tag)| tag.clone())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AutoUpgradeOutcome {
    /// The new tag was committed to the GitOps repo.
    Applied,
    /// This tag was already committed by an earlier scan.
    AlreadyApplied,
    /// Auto-upgrades are switched off globally.
    Disabled,
    Failed(String),
}

// The GitOps code clones into a shared /tmp/repos directory, so commits
// must not run concurrently (scheduler and API refresh can overlap).
static AUTO_UPGRADE_LOCK: Mutex<()> = Mutex::const_new(());

/// Commit `target` for `workload` to its GitOps repo, unless auto-upgrades
/// are globally disabled or this exact target was already applied. Every
/// attempt is recorded in the `auto_upgrades` table.
pub async fn apply_auto_upgrade(workload: &Workload, target: &str) -> AutoUpgradeOutcome {
    let _guard = AUTO_UPGRADE_LOCK.lock().await;

    let settings = match Settings::new() {
        Ok(s) => s,
        Err(e) => return fail(workload, target, format!("Failed to load settings: {}", e)),
    };
    if !settings.system.auto_upgrade_enabled {
        log::info!(
            "Auto-upgrade for {} skipped: system.auto_upgrade_enabled is false",
            workload.name
        );
        return AutoUpgradeOutcome::Disabled;
    }

    match auto_upgrade_already_applied(&workload.name, &workload.namespace, target) {
        Ok(true) => {
            log::info!(
                "Auto-upgrade of {} to {} already applied; waiting for rollout",
                workload.name,
                target
            );
            return AutoUpgradeOutcome::AlreadyApplied;
        }
        Ok(false) => {}
        Err(e) => {
            return fail(
                workload,
                target,
                format!("Failed to read auto-upgrade history: {}", e),
            )
        }
    }

    let repo = workload.git_ops_repo.as_deref().unwrap_or_default();
    if repo.is_empty() {
        return fail(
            workload,
            target,
            "auto-upgrade requires the slackwatch.repo annotation".to_string(),
        );
    }
    let has_repo_config = settings
        .gitops
        .as_ref()
        .is_some_and(|configs| configs.iter().any(|c| c.name == repo));
    if !has_repo_config {
        return fail(
            workload,
            target,
            format!("no [[gitops]] configuration named '{}'", repo),
        );
    }

    log::info!(
        "Auto-upgrading {} ({}) from {} to {}",
        workload.name,
        workload.namespace,
        workload.current_version,
        target
    );
    let mut upgrade = workload.clone();
    upgrade.latest_version = target.to_string();
    match run_git_operations(upgrade).await {
        Ok(()) => {
            if let Err(e) = record_auto_upgrade(workload, target, AUTO_UPGRADE_SUCCESS, None) {
                log::error!("Failed to record auto-upgrade of {}: {}", workload.name, e);
            }
            AutoUpgradeOutcome::Applied
        }
        Err(e) => fail(workload, target, format!("GitOps operation failed: {}", e)),
    }
}

fn fail(workload: &Workload, target: &str, message: String) -> AutoUpgradeOutcome {
    log::error!(
        "Auto-upgrade of {} to {} failed: {}",
        workload.name,
        target,
        message
    );
    if let Err(e) = record_auto_upgrade(workload, target, AUTO_UPGRADE_FAILED, Some(&message)) {
        log::error!("Failed to record auto-upgrade of {}: {}", workload.name, e);
    }
    AutoUpgradeOutcome::Failed(message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use AutoUpgradePolicy::*;

    fn tags(list: &[&str]) -> Vec<String> {
        list.iter().map(|t| t.to_string()).collect()
    }

    fn target(current: &str, list: &[&str], policy: AutoUpgradePolicy) -> Option<String> {
        select_auto_upgrade_target(current, &tags(list), policy)
    }

    #[test]
    fn parses_policies() {
        assert_eq!(AutoUpgradePolicy::from_annotation(None), Ok(Disabled));
        for (value, expected) in [
            ("", Disabled),
            ("none", Disabled),
            ("false", Disabled),
            ("off", Disabled),
            ("patch", Patch),
            ("minor", Minor),
            (" Minor ", Minor),
            ("major", Major),
            ("all", Major),
            ("true", Major),
        ] {
            assert_eq!(
                AutoUpgradePolicy::from_annotation(Some(value)),
                Ok(expected),
                "{value}"
            );
        }
        assert!(AutoUpgradePolicy::from_annotation(Some("minors")).is_err());
    }

    const TAGS: &[&str] = &[
        "1.2.3", "1.2.4", "1.2.9", "1.3.0", "1.9.1", "2.0.0", "3.1.0",
    ];

    #[test]
    fn disabled_never_upgrades() {
        assert_eq!(target("1.2.3", TAGS, Disabled), None);
    }

    #[test]
    fn patch_stays_within_minor() {
        assert_eq!(target("1.2.3", TAGS, Patch).as_deref(), Some("1.2.9"));
        assert_eq!(target("1.9.1", TAGS, Patch), None);
    }

    #[test]
    fn minor_stays_within_major() {
        assert_eq!(target("1.2.3", TAGS, Minor).as_deref(), Some("1.9.1"));
        assert_eq!(target("2.0.0", TAGS, Minor), None);
    }

    #[test]
    fn major_takes_latest() {
        assert_eq!(target("1.2.3", TAGS, Major).as_deref(), Some("3.1.0"));
        assert_eq!(target("3.1.0", TAGS, Major), None);
    }

    #[test]
    fn short_tags_are_supported() {
        // Jellyfin-style major.minor tags.
        assert_eq!(
            target("12.0", &["12.1", "12.2", "13.0"], Minor).as_deref(),
            Some("12.2")
        );
        assert_eq!(target("12.0", &["12.1", "13.0"], Patch), None);
    }

    #[test]
    fn skips_prereleases_for_stable_workloads() {
        let list = &["1.2.4-beta", "1.3.0-rc.1", "1.2.4-alpine", "1.2.3"];
        assert_eq!(target("1.2.3", list, Major), None);
        assert_eq!(
            target("1.2.3", &["1.2.4-beta", "1.2.4"], Patch).as_deref(),
            Some("1.2.4")
        );
    }

    #[test]
    fn allows_prereleases_when_already_on_one() {
        assert_eq!(
            target("1.3.0-rc.1", &["1.3.0-rc.2", "1.3.0-rc.1"], Patch).as_deref(),
            Some("1.3.0-rc.2")
        );
        // The final release still beats its release candidates.
        assert_eq!(
            target("1.3.0-rc.1", &["1.3.0-rc.2", "1.3.0"], Patch).as_deref(),
            Some("1.3.0")
        );
    }

    #[test]
    fn unparseable_current_version_never_upgrades() {
        assert_eq!(target("latest", TAGS, Major), None);
        assert_eq!(target("sha-e959062", TAGS, Major), None);
    }

    #[test]
    fn ignores_non_version_tags() {
        let list = &["1.2.4", "latest", "sha-e959062", "9443289", "unstable"];
        assert_eq!(target("1.2.3", list, Major).as_deref(), Some("1.2.4"));
    }

    #[test]
    fn prefers_tag_spelling_matching_current() {
        let list = &["1.3.0", "v1.3.0"];
        assert_eq!(target("v1.2.0", list, Minor).as_deref(), Some("v1.3.0"));
        assert_eq!(target("1.2.0", list, Minor).as_deref(), Some("1.3.0"));
        assert_eq!(
            target("version-6.7.7", &["6.7.8", "version-6.7.8"], Patch).as_deref(),
            Some("version-6.7.8")
        );
    }
}
