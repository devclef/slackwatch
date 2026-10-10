use crate::database;
use crate::database::client::get_latest_scan_id;
use crate::kubernetes::client::{find_enabled_workloads, find_specific_workload};
use crate::models::{UpdateStatus, Workload};
use crate::notifications::ntfy::send_batch_notification;
use crate::repocheck::get_tags_for_image;
use regex::Regex;
use semver::Version;

pub async fn update_single_workload(current_workload: Workload) -> Result<(), String> {
    let workload = find_specific_workload(
        &current_workload.name.clone(),
        &current_workload.namespace.clone(),
    )
    .await
    .map_err(|e| e.to_string())?;
    log::info!("Found workload: {:?}", workload);
    let scan_id = get_latest_scan_id().unwrap_or(0) + 1;
    // Fetch tags once: parse_tags does the full fetch and comparison.
    // (Previously the tags were fetched a second time here via
    // find_latest_tag_for_image, which doubled registry rate-limit usage.)
    let workload = match parse_tags(&workload).await {
        Ok(w) => w,
        Err(e) => {
            log::error!("Scan error for workload {}: {}", workload.name, e);
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
                error: Some(e.to_string()),
            }
        }
    };

    if workload.update_available.to_string() == "Available" {
        let workloads: Vec<Workload> = vec![workload.clone()];
        send_batch_notification(&workloads)
            .await
            .unwrap_or_else(|e| log::error!("Error sending notification: {}", e));
    }
    std::thread::spawn(move || database::client::insert_workload(&workload, scan_id))
        .join()
        .map_err(|_| "Thread error".to_string())?
        .expect("TODO: panic message");
    Ok(())
}

pub async fn fetch_and_update_all_watched() -> Result<(), String> {
    let workloads = find_enabled_workloads().await.map_err(|e| e.to_string())?;
    log::info!("Found {} workloads", workloads.len());

    let scan_id = get_latest_scan_id().unwrap_or(0) + 1;
    let mut updates_available: Vec<Workload> = Vec::new();

    for workload in workloads {
        // Fetch tags once: parse_tags does the full fetch and comparison.
        let workload = match parse_tags(&workload).await {
            Ok(w) => w,
            Err(e) => {
                log::error!("Scan error for workload {}: {}", workload.name, e);
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
                    error: Some(e.to_string()),
                }
            }
        };

        if workload.update_available.to_string() == "Available" {
            updates_available.push(workload.clone());
        }

        std::thread::spawn(move || database::client::insert_workload(&workload, scan_id))
            .join()
            .map_err(|_| "Thread error".to_string())?
            .expect("TODO: panic message");
    }

    // Send batch notification if there are updates
    if !updates_available.is_empty() {
        send_batch_notification(&updates_available)
            .await
            .unwrap_or_else(|e| log::error!("Error sending batch notification: {}", e));
    }

    Ok(())
}


pub async fn test_call() {
    let workloads = find_enabled_workloads().await.unwrap();
    for workload in workloads.iter().take(1) {
        //let workload = workload.clone();
        let workload = parse_tags(workload).await.unwrap();
        log::info!("Workload: {:?}", workload)
    }
}

/// Parse a Docker tag as a version.
///
/// Docker tags are not required to be strict SemVer and some registries
/// publish short `major.minor` tags such as `12.1` (Jellyfin), which
/// `semver::Version::parse` rejects. An optional leading `v` is ignored
/// and the leading `major[.minor][.patch]` is normalized to a full
/// `major.minor.patch` (missing components default to zero), so `12.1`
/// compares as `12.1.0`. To keep arbitrary build tags out of the
/// comparison, tags must start with a digit and have at least two
/// components (bare numbers such as `9443289` are build/PR numbers, not
/// versions), and short `major.minor` tags must be purely numeric;
/// pre-release/build metadata is only accepted on full
/// `major.minor.patch` tags (`12.1.0-beta`). Anything else (`unstable`,
/// `sha-e959062`, `42-bound-74a3ef0`, `9443289`) returns `None`.
fn parse_version_loose(tag: &str) -> Option<Version> {
    let trimmed = tag
        .strip_prefix('v')
        .or_else(|| tag.strip_prefix('V'))
        .unwrap_or(tag);
    if !trimmed.chars().next().is_some_and(|c| c.is_ascii_digit()) {
        return None;
    }

    // Split the leading numeric core (digits and dots) from any suffix.
    let core_end = trimmed
        .char_indices()
        .find(|(_, c)| !c.is_ascii_digit() && *c != '.')
        .map(|(i, _)| i)
        .unwrap_or(trimmed.len());
    let (core, suffix) = trimmed.split_at(core_end);

    let components: Vec<&str> = core.split('.').filter(|c| !c.is_empty()).collect();
    if components.len() < 2
        || components.len() > 3
        || components
            .iter()
            .any(|c| !c.bytes().all(|b| b.is_ascii_digit()))
    {
        return None;
    }

    // Short tags must be purely numeric; suffixes (pre-release/build)
    // are only meaningful on full major.minor.patch versions.
    if components.len() < 3 && !suffix.is_empty() {
        return None;
    }

    let mut normalized = String::new();
    for (i, component) in components.iter().enumerate() {
        if i > 0 {
            normalized.push('.');
        }
        normalized.push_str(component);
    }
    while normalized.split('.').count() < 3 {
        normalized.push_str(".0");
    }

    Version::parse(&format!("{normalized}{suffix}")).ok()
}

/// Returns the highest-version tag that is strictly newer than
/// `current_version`. Tags that cannot be parsed as a version are
/// skipped.
fn find_latest_update(current_version: &str, tags: &[String]) -> Option<String> {
    let current = parse_version_loose(current_version).unwrap_or(Version::new(0, 0, 0));
    let mut latest: Option<Version> = None;
    let mut latest_tag: Option<String> = None;

    for tag in tags {
        let Some(tag_version) = parse_version_loose(tag) else {
            log::debug!("Skipping tag {}: not a valid SemVer", tag);
            continue;
        };
        if tag_version > current && latest.as_ref().is_none_or(|l| tag_version > *l) {
            match &latest_tag {
                Some(prev) => {
                    log::info!("Tag {} is newer than current latest_version {}", tag, prev)
                }
                None => log::info!("latest_version is empty - setting to tag {}", tag),
            }
            latest = Some(tag_version);
            latest_tag = Some(tag.clone());
        }
    }

    latest_tag
}

pub async fn parse_tags(workload: &Workload) -> Result<Workload, String> {
     let (mut tags, exhausted) = get_tags_for_image(&workload.image).await.map_err(|e| e.to_string())?;
     tags.sort();

    // Validate and compile include patterns, capturing errors
    let include_patterns: Result<Vec<Regex>, String> = workload
        .include_pattern
        .as_ref()
        .map(|pattern_str| {
            pattern_str
                .split(",")
                .map(|pattern| {
                    Regex::new(pattern).map_err(|e| format!("Invalid include pattern '{}': {}", pattern, e))
                })
                .collect()
        })
        .unwrap_or(Ok(Vec::new()));

    let include_patterns = include_patterns?;
    if !include_patterns.is_empty() {
        log::info!("Include pattern defined, using only include");
        log::info!("Include pattern: {:?}", workload.include_pattern);

        tags.retain(|tag| include_patterns.iter().any(|regex| regex.is_match(tag)));

        log::info!("Filtered tags: {:?}", tags);
    }

    // Validate and compile exclude patterns, capturing errors
    let exclude_patterns: Result<Vec<Regex>, String> = workload
        .exclude_pattern
        .as_ref()
        .map(|pattern_str| {
            pattern_str
                .split(",")
                .map(|pattern| {
                    Regex::new(pattern).map_err(|e| format!("Invalid exclude pattern '{}': {}", pattern, e))
                })
                .collect()
        })
        .unwrap_or(Ok(Vec::new()));

    let exclude_patterns = exclude_patterns?;
    if !exclude_patterns.is_empty() {
        log::info!("Exclude pattern defined, using only exclude");
        log::info!("Exclude pattern: {:?}", workload.exclude_pattern);

        tags.retain(|tag| exclude_patterns.iter().all(|regex| !regex.is_match(tag)));

        log::info!("Filtered tags: {:?}", tags);
    }
    // Perform SemVer comparison with each tag:
    let latest_version = find_latest_update(&workload.current_version, &tags).unwrap_or_default();
    let update_available = if latest_version.is_empty() {
        UpdateStatus::NotAvailable
    } else {
        log::info!("Latest version for {}: {}", workload.image, latest_version);
        UpdateStatus::Available
    };
Ok(Workload {
         name: workload.name.clone(),
         exclude_pattern: workload.exclude_pattern.clone(),
         git_ops_repo: workload.git_ops_repo.clone(),
         include_pattern: workload.include_pattern.clone(),
         namespace: workload.namespace.clone(),
         current_version: workload.current_version.clone(),
         image: workload.image.clone(),
         update_available,
         last_scanned: workload.last_scanned.clone(),
         latest_version: latest_version.clone(),
         git_directory: workload.git_directory.clone(),
         scan_exhausted: exhausted.to_string(),
         error: None,
     })
 }

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(tag: &str) -> Option<Version> {
        parse_version_loose(tag)
    }

    #[test]
    fn parses_full_semver() {
        assert_eq!(parse("12.1.0").unwrap(), Version::new(12, 1, 0));
    }

    #[test]
    fn parses_two_component_versions() {
        // Jellyfin publishes major.minor tags such as 12.1 / 12.2.
        assert_eq!(parse("12.1").unwrap(), Version::new(12, 1, 0));
        assert_eq!(parse("12.2").unwrap(), Version::new(12, 2, 0));
    }

    #[test]
    fn rejects_single_component_numbers() {
        // Bare numbers are build/PR numbers, not versions
        // (frigate tags 9443289, grafana tags 9799770991).
        assert!(parse("12").is_none());
        assert!(parse("9443289").is_none());
        assert!(parse("9799770991").is_none());
        assert!(parse("2024").is_none());
    }

    #[test]
    fn strips_leading_v_prefix() {
        assert_eq!(parse("v12.1").unwrap(), Version::new(12, 1, 0));
        assert_eq!(parse("v3.5.0").unwrap(), Version::new(3, 5, 0));
    }

    #[test]
    fn preserves_prerelease_and_build_metadata_on_full_versions() {
        // Pre-release suffix is kept and sorts before the release.
        let beta = parse("12.1.0-beta").unwrap();
        assert_eq!(beta.pre, semver::Prerelease::new("beta").unwrap());
        assert!(beta < Version::new(12, 1, 0));

        // Build metadata is kept but ignored in version ordering.
        let v = parse("12.1.2+build5").unwrap();
        assert!(v > Version::new(12, 1, 1));
        assert!(v < Version::new(12, 1, 3));
    }

    #[test]
    fn rejects_short_tags_with_suffixes() {
        // `42-bound-74a3ef0` (Loki) must not compare as 42.0.0.
        assert!(parse("42-bound-74a3ef0").is_none());
        assert!(parse("12.1-beta").is_none());
    }

    #[test]
    fn rejects_non_version_tags() {
        // `sha-e959062` (Seerr) must not compare as 959062.0.0.
        assert!(parse("sha-e959062").is_none());
        assert!(parse("unstable").is_none());
        assert!(parse("latest").is_none());
        assert!(parse("").is_none());
        assert!(parse("v").is_none());
        assert!(parse("release-1.2.3").is_none());
    }

    #[test]
    fn rejects_extra_components() {
        assert!(parse("12.1.2.3").is_none());
    }

    #[test]
    fn finds_latest_two_component_update() {
        // The reported bug: Jellyfin running 12.0 with 12.1/12.2
        // released, mixed with non-SemVer build tags.
        let tags = vec![
            "12.0".to_string(),
            "12.1".to_string(),
            "12.2".to_string(),
            "sha-e959062".to_string(),
            "42-bound-74a3ef0".to_string(),
            "unstable".to_string(),
        ];
        assert_eq!(find_latest_update("12.0", &tags).as_deref(), Some("12.2"));
    }

    #[test]
    fn ignores_numeric_build_tags() {
        // Reported bug: numeric build/PR tags must not beat real versions.
        let frigate = vec![
            "0.18.0".to_string(),
            "0.18.1".to_string(),
            "9443289".to_string(),
        ];
        assert_eq!(
            find_latest_update("0.18.0", &frigate).as_deref(),
            Some("0.18.1")
        );

        let grafana = vec![
            "13.2.3".to_string(),
            "13.2.4".to_string(),
            "9799770991".to_string(),
        ];
        assert_eq!(
            find_latest_update("13.2.3", &grafana).as_deref(),
            Some("13.2.4")
        );
    }

    #[test]
    fn no_update_when_current_is_latest() {
        let tags = vec!["12.1".to_string(), "12.2".to_string()];
        assert_eq!(find_latest_update("12.2", &tags), None);
    }

    #[test]
    fn unparseable_current_version_treated_as_zero() {
        let tags = vec!["12.1".to_string()];
        assert_eq!(
            find_latest_update("unstable", &tags).as_deref(),
            Some("12.1")
        );
    }

    #[test]
    fn unparseable_tags_are_skipped() {
        let tags = vec!["unstable".to_string(), "latest".to_string()];
        assert_eq!(find_latest_update("12.0", &tags), None);
    }

    #[test]
    fn compares_versions_numerically_not_lexicographically() {
        let tags = vec!["9.0".to_string(), "10.0".to_string()];
        assert_eq!(find_latest_update("1.0.0", &tags).as_deref(), Some("10.0"));
    }
}
