use arcadestr_core::auth::AuthState;
use arcadestr_core::nip46::ProfileMetadata;
use arcadestr_core::nostr::{GameListing, UserProfile};
use nostr::prelude::ToBech32;
use serde::Serialize;
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize)]
pub struct VersionInfo {
    pub version: String,
    pub revision: u32,
    pub full: String,
    pub os: String,
    pub arch: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectWithKeyResult {
    pub npub: String,
    pub event_name: &'static str,
}

pub fn auth_is_authenticated(auth: &AuthState) -> bool {
    auth.is_authenticated()
}

pub fn auth_get_public_key(auth: &AuthState) -> Result<String, String> {
    let pubkey = auth
        .public_key()
        .ok_or_else(|| "Not authenticated".to_string())?;

    pubkey.to_bech32().map_err(|e| e.to_string())
}

pub fn auth_connect_with_key(
    auth: &mut AuthState,
    key: &str,
) -> Result<ConnectWithKeyResult, String> {
    auth.connect_with_key(key).map_err(|e| {
        format!(
            "Failed to authenticate with provided key. Make sure you're entering a valid nsec1... key or hex private key. Error: {}",
            e
        )
    })?;

    let npub = auth_get_public_key(auth)?;

    Ok(ConnectWithKeyResult {
        npub,
        event_name: "auth_success",
    })
}

pub fn version_info() -> VersionInfo {
    VersionInfo {
        version: arcadestr_core::version::VERSION.to_string(),
        revision: arcadestr_core::version::REVISION,
        full: arcadestr_core::version::full_version(),
        os: std::env::consts::OS.to_string(),
        arch: std::env::consts::ARCH.to_string(),
    }
}

pub fn serialize_publish_listing_result(event_id: &str) -> serde_json::Value {
    serde_json::json!(event_id)
}

pub fn serialize_fetch_listings_result(
    listings: &[GameListing],
) -> Result<serde_json::Value, String> {
    serde_json::to_value(listings).map_err(|e| e.to_string())
}

pub fn serialize_fetch_profile_result(profile: &UserProfile) -> Result<serde_json::Value, String> {
    serde_json::to_value(profile).map_err(|e| e.to_string())
}

pub fn build_list_saved_profiles_response(
    profiles: Vec<ProfileMetadata>,
    active_profile_id: Option<&str>,
    cached_profiles: &HashMap<String, UserProfile>,
) -> serde_json::Value {
    let mut seen_npubs: HashMap<String, ProfileMetadata> = HashMap::new();

    for profile in profiles {
        seen_npubs
            .entry(profile.pubkey_bech32.clone())
            .or_insert(profile);
    }

    let unique_profiles: Vec<ProfileMetadata> = seen_npubs.into_values().collect();
    let single_profile_mode = unique_profiles.len() == 1;

    let accounts: Vec<serde_json::Value> = unique_profiles
        .into_iter()
        .map(|profile| {
            let is_current = if single_profile_mode {
                false
            } else {
                active_profile_id == Some(profile.id.as_str())
            };

            let cached = cached_profiles.get(&profile.pubkey_bech32);

            serde_json::json!({
                "id": profile.id,
                "name": profile.name,
                "npub": profile.pubkey_bech32,
                "pubkey_hex": profile.pubkey_hex,
                "signing_mode": "nip46",
                "last_used": 0,
                "is_current": is_current,
                "picture": cached.and_then(|entry| entry.picture.clone()),
                "display_name": cached.and_then(|entry| entry.display_name.clone()),
                "username": cached.and_then(|entry| entry.name.clone()),
                "nip05": cached.and_then(|entry| entry.nip05.clone()),
                "about": cached.and_then(|entry| entry.about.clone()),
            })
        })
        .collect();

    serde_json::json!({ "accounts": accounts })
}

pub fn apply_delete_profile_index(
    profiles: Vec<ProfileMetadata>,
    key: &str,
) -> Vec<ProfileMetadata> {
    profiles
        .into_iter()
        .filter(|profile| profile.id != key && profile.bunker_pubkey_hex != key)
        .collect()
}
