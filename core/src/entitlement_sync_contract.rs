//! Known-grant reconciliation only; not grant discovery/import or paid receipt sync.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct EntitlementSyncReport {
    pub checked: usize,
    pub revoked: usize,
    pub rejected_events: usize,
    pub indeterminate: usize,
    /// Relay retention/availability cannot establish global absence of revocations.
    pub incomplete: bool,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct EntitlementSyncRequest {
    pub expected_account_pubkey: String,
}
