use arcadestr_core::auth::AuthState;
use arcadestr_core::http_client::HttpClient;
use arcadestr_core::nostr::NostrClient;
use arcadestr_core::storage::Database;
use serde_json::json;
use std::sync::Arc;
use tokio::sync::Mutex;

pub struct AppState {
    pub auth: Arc<Mutex<AuthState>>,
    pub nostr: Arc<Mutex<NostrClient>>,
    pub database: Arc<Database>,
    pub http_client: Arc<dyn HttpClient>,
}

#[path = "../src/command_contracts.rs"]
mod command_contracts;

#[test]
fn fetch_earned_badges_command_name_serializes_empty_vec() {
    let payload: Vec<arcadestr_core::achievements::EarnedBadgeSummary> = Vec::new();
    let value = command_contracts::serialize_fetch_earned_badges_result(&payload)
        .expect("earned badge payload serializes");

    assert_eq!(value, json!([]));
}

#[test]
fn fetch_profile_badges_command_name_serializes_empty_vec() {
    let payload: Vec<arcadestr_core::achievements::ProfileBadgeEntry> = Vec::new();
    let value = command_contracts::serialize_fetch_profile_badges_result(&payload)
        .expect("profile badge payload serializes");

    assert_eq!(value, json!([]));
}
