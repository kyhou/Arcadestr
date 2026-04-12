use arcadestr_core::auth::AuthState;
use arcadestr_core::nip46::ProfileMetadata;
use arcadestr_core::nostr::{GameListing, UserProfile};
use nostr::Keys;
use nostr::ToBech32;
use std::collections::HashMap;

#[path = "../src/command_contracts.rs"]
mod command_contracts;

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
