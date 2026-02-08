use serde::Serialize;
use serde_derive::Deserialize;

//Data model for workload
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
