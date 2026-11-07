//! Safe receipt synchronization IPC: no raw evidence, plaintext or credentials.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReceiptVerification {
    Verified,
    Indeterminate,
    Invalid,
}
impl ReceiptVerification {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Verified => "verified",
            Self::Indeterminate => "indeterminate",
            Self::Invalid => "invalid",
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReceiptChainResult {
    pub order_id: String,
    pub listing_coordinate: String,
    pub verification: ReceiptVerification,
    pub reason: String,
    pub tip_event_id: Option<String>,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ReceiptSyncReport {
    pub chains: Vec<ReceiptChainResult>,
    pub rejected_events: usize,
    pub relay_failures: usize,
    pub incomplete: bool,
    pub message: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReceiptSyncRequest {
    pub expected_account_pubkey: String,
    #[serde(default)]
    pub full_rescan: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReceiptImportRequest {
    pub expected_account_pubkey: String,
    pub events_json: String,
}
