pub mod gitops_models;

use serde::Serialize;
use serde_derive::Deserialize;

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct Workload {
    pub name: String,
    pub exclude_pattern: Option<String>,
    pub git_ops_repo: Option<String>,
    pub include_pattern: Option<String>,
    pub update_available: UpdateStatus,
    pub git_directory: Option<String>,
    pub image: String,
    pub last_scanned: String,
    pub namespace: String,
    pub current_version: String,
    pub latest_version: String,
    pub scan_exhausted: String,
    pub error: Option<String>,
    /// Auto-upgrade policy from the `slackwatch.autoupgrade` annotation
    /// (`patch`, `minor`, `major`, ...). `None` means auto-upgrade is off.
    #[serde(default)]
    pub auto_upgrade: Option<String>,
}

#[derive(
    strum_macros::Display, strum_macros::EnumString, Debug, Serialize, Deserialize, Clone, PartialEq,
)]
pub enum UpdateStatus {
    Available,
    NotAvailable,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct ApiResponse {
    pub(crate) status: String,
    pub(crate) message: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workload_without_auto_upgrade_field_still_deserializes() {
        // Payload shape sent by API clients predating auto-upgrade.
        let json = r#"{
            "name": "app", "exclude_pattern": null, "git_ops_repo": null,
            "include_pattern": null, "update_available": "Available",
            "git_directory": null, "image": "example/app:1.0.0",
            "last_scanned": "now", "namespace": "default",
            "current_version": "1.0.0", "latest_version": "1.1.0",
            "scan_exhausted": "False", "error": null
        }"#;
        let workload: Workload = serde_json::from_str(json).unwrap();
        assert_eq!(workload.auto_upgrade, None);
    }
}
