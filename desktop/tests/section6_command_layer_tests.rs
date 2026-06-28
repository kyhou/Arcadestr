use arcadestr_core::auth::AuthState;
use arcadestr_core::http_client::{HttpClient, HttpClientError, ReqwestHttpClient};
use arcadestr_core::nip46::ProfileMetadata;
use arcadestr_core::nostr::{GameListing, UserProfile};
use async_trait::async_trait;
use nostr::Keys;
use nostr::ToBech32;
use serde_json::json;
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;

pub struct AppState {
    pub auth: Arc<Mutex<AuthState>>,
    pub http_client: Arc<dyn HttpClient>,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            auth: Arc::new(Mutex::new(AuthState::new())),
            http_client: Arc::new(
                ReqwestHttpClient::new(Duration::from_secs(1))
                    .expect("test reqwest http client should build"),
            ),
        }
    }
}

#[path = "../src/command_contracts.rs"]
mod command_contracts;

#[derive(Default)]
struct MockNoRedirectHttpClient {
    no_redirect_calls: AtomicUsize,
}

#[async_trait]
impl HttpClient for MockNoRedirectHttpClient {
    async fn get_json(&self, _url: &str) -> Result<serde_json::Value, HttpClientError> {
        Err(HttpClientError::Request(
            "get_json path should not be used in NIP-05 verification".to_string(),
        ))
    }

    async fn get_json_no_redirects(
        &self,
        _url: &str,
    ) -> Result<serde_json::Value, HttpClientError> {
        self.no_redirect_calls.fetch_add(1, Ordering::SeqCst);
        Ok(json!({
            "names": {
                "alice": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
            },
            "relays": {
                "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa": ["wss://relay.example.com"]
            }
        }))
    }
}

#[test]
fn is_authenticated_before_login_returns_false() {
    let auth = AuthState::new();
    let result = command_contracts::auth_is_authenticated(&auth);
    assert!(!result);
}

#[test]
fn get_public_key_before_login_returns_error_string() {
    let auth = AuthState::new();
    let result = command_contracts::auth_get_public_key(&auth);

    assert!(result.is_err());
    assert_eq!(result.expect_err("must be error"), "Not authenticated");
}

#[test]
fn connect_with_key_valid_nsec_sets_authenticated_and_reports_event() {
    let keys = Keys::generate();
    let nsec = keys
        .secret_key()
        .to_bech32()
        .expect("nsec encode must work");

    let mut auth = AuthState::new();
    let response = command_contracts::auth_connect_with_key(&mut auth, &nsec)
        .expect("connect_with_key should succeed for valid nsec");

    assert!(auth.is_authenticated());
    assert!(response.npub.starts_with("npub1"));
    assert_eq!(response.event_name, "auth_success");
}

#[test]
fn get_version_info_returns_non_empty_version() {
    let info = command_contracts::version_info();

    assert!(!info.version.is_empty());
    assert!(!info.full.is_empty());
}

#[test]
fn empty_vec_serializes_as_array_not_null() {
    let payload: Vec<String> = Vec::new();
    let value = serde_json::to_value(&payload).expect("vec serialization should work");

    assert!(value.is_array());
    assert_eq!(value, serde_json::json!([]));
}

fn sample_listing() -> GameListing {
    GameListing {
        id: "test-game-v1".to_string(),
        title: "Test Game".to_string(),
        description: "Section 6 test listing".to_string(),
        price_sats: 1000,
        download_url: "https://example.com/game.zip".to_string(),
        publisher_npub: "npub1testpublisher".to_string(),
        created_at: 1_710_000_000,
        tags: vec!["indie".to_string()],
        lud16: "seller@example.com".to_string(),
        platforms: Vec::new(),
        images: vec!["https://example.com/cover.png".to_string()],
        summary: Some("summary".to_string()),
        published_at: Some(1_710_000_000),
        location: Some("online".to_string()),
        geohash: None,
        status: Some("active".to_string()),
    }
}

fn sample_profile(npub: &str) -> UserProfile {
    UserProfile {
        npub: npub.to_string(),
        name: Some("alice".to_string()),
        display_name: Some("Alice".to_string()),
        about: Some("builder".to_string()),
        picture: Some("https://example.com/alice.png".to_string()),
        website: None,
        nip05: Some("alice@example.com".to_string()),
        lud16: None,
        nip05_verified: true,
    }
}

fn sample_profile_metadata(
    id: &str,
    npub: &str,
    pubkey_hex: &str,
    bunker_pubkey_hex: &str,
) -> ProfileMetadata {
    ProfileMetadata {
        id: id.to_string(),
        name: "Account".to_string(),
        pubkey_bech32: npub.to_string(),
        pubkey_hex: pubkey_hex.to_string(),
        bunker_pubkey_hex: bunker_pubkey_hex.to_string(),
        picture: None,
        display_name: None,
        username: None,
        nip05: None,
        about: None,
    }
}

#[test]
fn publish_listing_contract_returns_event_id_string() {
    let event_id = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    let value = command_contracts::serialize_publish_listing_result(event_id);

    assert_eq!(value, serde_json::json!(event_id));
}

#[test]
fn fetch_listings_contract_serializes_vec_of_game_listing() {
    let listings = vec![sample_listing()];

    let value = command_contracts::serialize_fetch_listings_result(&listings)
        .expect("fetch_listings payload should serialize");

    assert!(value.is_array());
    assert_eq!(value.as_array().expect("array").len(), 1);
    assert_eq!(value[0]["id"], serde_json::json!("test-game-v1"));
}

#[test]
fn fetch_profile_contract_serializes_user_profile_with_matching_npub() {
    let profile = sample_profile("npub1alice");

    let value = command_contracts::serialize_fetch_profile_result(&profile)
        .expect("fetch_profile payload should serialize");
    let roundtrip: UserProfile =
        serde_json::from_value(value).expect("profile payload should deserialize");

    assert_eq!(roundtrip.npub, "npub1alice");
}

#[test]
fn list_saved_profiles_contract_contains_added_profile() {
    let profiles = vec![sample_profile_metadata(
        "profile-1",
        "npub1alice",
        "abc123",
        "bunker123",
    )];

    let mut cached_profiles = HashMap::new();
    cached_profiles.insert("npub1alice".to_string(), sample_profile("npub1alice"));

    let response = command_contracts::build_list_saved_profiles_response(
        profiles,
        Some("profile-1"),
        &cached_profiles,
    );

    let accounts = response["accounts"]
        .as_array()
        .expect("accounts should be array");
    assert_eq!(accounts.len(), 1);
    assert_eq!(accounts[0]["id"], serde_json::json!("profile-1"));
    assert_eq!(accounts[0]["npub"], serde_json::json!("npub1alice"));
    assert_eq!(accounts[0]["is_current"], serde_json::json!(false));
}

#[test]
fn delete_profile_contract_removes_profile_from_followup_list() {
    let profiles = vec![
        sample_profile_metadata("profile-1", "npub1alice", "abc123", "bunker123"),
        sample_profile_metadata("profile-2", "npub1bob", "def456", "bunker456"),
    ];

    let remaining = command_contracts::apply_delete_profile_index(profiles, "profile-1");

    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0].id, "profile-2");

    let response =
        command_contracts::build_list_saved_profiles_response(remaining, None, &HashMap::new());
    let accounts = response["accounts"]
        .as_array()
        .expect("accounts should be array");
    assert_eq!(accounts.len(), 1);
    assert_eq!(accounts[0]["id"], serde_json::json!("profile-2"));
}

#[tokio::test]
async fn verify_nip05_identity_uses_expected_pubkey_from_request() {
    let http_client = Arc::new(MockNoRedirectHttpClient::default());
    let state = AppState {
        auth: Arc::new(Mutex::new(AuthState::new())),
        http_client: http_client.clone(),
    };

    let result = command_contracts::verify_nip05_identity(
        &state,
        command_contracts::VerifyNip05Request {
            nip05: "alice@example.com".to_string(),
            expected_pubkey: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                .to_string(),
        },
    )
    .await
    .expect("verification should use request.expected_pubkey");

    assert!(result.verified);
    assert_eq!(result.nip05, "alice@example.com");
    assert_eq!(result.relays, vec!["wss://relay.example.com".to_string()]);
    assert_eq!(result.error, None);
    assert_eq!(http_client.no_redirect_calls.load(Ordering::SeqCst), 1);
}

#[test]
fn export_import_verify_contract_structs_match_batch4_schema() {
    let export_request = command_contracts::ExportKeyRequest {
        password: "strong-password".to_string(),
        scrypt_n: Some(131072),
    };
    let export_request_json = serde_json::to_value(export_request).expect("serialize request");
    assert_eq!(export_request_json.get("entry_id"), None);
    assert!(export_request_json.get("password").is_some());
    assert!(export_request_json.get("scrypt_n").is_some());

    let export_result = command_contracts::ExportKeyResult {
        ncryptsec: "ncryptsec1xxx".to_string(),
        keychain_entry: "arcadestr_nip49_deadbeef_123".to_string(),
    };
    let export_result_json = serde_json::to_value(export_result).expect("serialize result");
    assert!(export_result_json.get("ncryptsec").is_some());
    assert!(export_result_json.get("keychain_entry").is_some());
    assert_eq!(export_result_json.get("entry_id"), None);
    assert_eq!(export_result_json.get("npub"), None);

    let import_result = command_contracts::ImportKeyResult {
        pubkey: "abc".to_string(),
        success: true,
    };
    let import_result_json = serde_json::to_value(import_result).expect("serialize import");
    assert_eq!(import_result_json["pubkey"], json!("abc"));
    assert_eq!(import_result_json["success"], json!(true));

    let verify_request = command_contracts::VerifyNip05Request {
        nip05: "alice@example.com".to_string(),
        expected_pubkey: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
            .to_string(),
    };
    let verify_request_json = serde_json::to_value(verify_request).expect("serialize verify req");
    assert!(verify_request_json.get("nip05").is_some());
    assert!(verify_request_json.get("expected_pubkey").is_some());
    assert_eq!(verify_request_json.get("identifier"), None);

    let verify_result = command_contracts::VerifyNip05Result {
        nip05: "alice@example.com".to_string(),
        verified: true,
        relays: vec![],
        error: None,
    };
    let verify_result_json = serde_json::to_value(verify_result).expect("serialize verify result");
    assert!(verify_result_json.get("nip05").is_some());
    assert!(verify_result_json.get("verified").is_some());
    assert!(verify_result_json.get("relays").is_some());
    assert!(verify_result_json.get("error").is_some());
}
