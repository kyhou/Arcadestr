use arcadestr_core::achievements::{EarnedBadgeSummary, ProfileBadgeEntry};
use arcadestr_core::auth::AuthState;
use arcadestr_core::nip46::store_ncryptsec_in_keychain;
use arcadestr_core::nip46::ProfileMetadata;
use arcadestr_core::nostr::{
    parse_nip05_identifier, parse_nip19_identifier,
    verify_nip05_identity as core_verify_nip05_identity, GameListing, UserProfile,
};
use arcadestr_core::signers::ActiveSigner;
use arcadestr_core::storage::{
    decrypt_private_key_nip49, encrypt_private_key_nip49, extract_nip49_version, parse_ncryptsec,
    serialize_ncryptsec, validate_nip49_format, validate_nip49_password, ScryptParams,
};
use nostr::prelude::ToBech32;
use nostr::Keys;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use thiserror::Error;
use tokio::sync::Mutex;
use tracing::error;

fn normalize_profile_pubkey_identifier(profile_pubkey: &str) -> Result<String, CommandError> {
    let trimmed = profile_pubkey.trim();
    if trimmed.is_empty() {
        return Err(CommandError::InvalidInput(
            "Profile pubkey cannot be empty".to_string(),
        ));
    }

    if trimmed.starts_with("npub1") || trimmed.starts_with("nprofile1") {
        return parse_nip19_identifier(trimmed)
            .map(|parsed| parsed.pubkey)
            .map_err(|error| CommandError::InvalidInput(error.to_string()));
    }

    nostr::key::PublicKey::parse(trimmed)
        .map(|pubkey| pubkey.to_hex())
        .map_err(|error| CommandError::InvalidInput(format!("Invalid profile pubkey: {error}")))
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExportKeyRequest {
    pub password: String,
    pub scrypt_n: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExportKeyResult {
    pub ncryptsec: String,
    pub keychain_entry: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ImportKeyRequest {
    pub ncryptsec: String,
    pub password: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ImportKeyResult {
    pub success: bool,
    pub pubkey: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct VerifyNip05Request {
    pub nip05: String,
    pub expected_pubkey: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct VerifyNip05Result {
    pub nip05: String,
    pub verified: bool,
    pub relays: Vec<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum CommandError {
    #[error("Encryption failed")]
    Encryption(String),
    #[error("Wrong password or corrupted backup")]
    Decryption(String),
    #[error("Keychain operation failed")]
    Keychain(String),
    #[error("Network error during verification")]
    Http(String),
    #[error("Identity verification failed")]
    Nip05(String),
    #[error("No active local private key available for this operation")]
    NoActiveKey,
    #[error("{0}")]
    InvalidInput(String),
    #[error("Achievement operation failed: {0}")]
    Achievements(String),
}

pub trait BadgeCommandState {
    fn badge_command_handles(
        &self,
    ) -> (
        Arc<Mutex<arcadestr_core::nostr::NostrClient>>,
        Arc<arcadestr_core::storage::Database>,
    );
}

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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Nip49ImportRequest {
    pub ncryptsec: String,
    pub password: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Nip49ExportResult {
    pub npub: String,
    pub ncryptsec: String,
    pub deferred: bool,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Nip05Status {
    pub identifier: String,
    pub normalized_identifier: String,
    pub local_part: String,
    pub domain: String,
    pub verified: bool,
    pub status: String,
    pub message: String,
}

pub async fn export_encrypted_key(
    state: &crate::AppState,
    request: ExportKeyRequest,
) -> Result<ExportKeyResult, CommandError> {
    validate_nip49_password(&request.password)
        .map_err(|error| CommandError::InvalidInput(error.to_string()))?;

    let default_params = ScryptParams::default_nip49();
    let scrypt_n = request.scrypt_n.unwrap_or(default_params.n);
    if !scrypt_n.is_power_of_two() {
        return Err(CommandError::InvalidInput(
            "scrypt_n must be a power-of-two value".to_string(),
        ));
    }

    let (private_key_hex, pubkey_hex) = {
        let auth = state.auth.lock().await;
        let signer = auth.signer().ok_or(CommandError::NoActiveKey)?;
        let public_key = auth.public_key().ok_or(CommandError::NoActiveKey)?;

        let active_private_key = if let ActiveSigner::DirectKey(direct_key_signer) = signer {
            direct_key_signer.keys().secret_key().to_secret_hex()
        } else {
            return Err(CommandError::NoActiveKey);
        };

        (active_private_key, public_key.to_hex())
    };

    let params = ScryptParams {
        n: scrypt_n,
        r: default_params.r,
        p: default_params.p,
    };
    let typed_ncryptsec =
        encrypt_private_key_nip49(&private_key_hex, &request.password, Some(params))
            .map_err(|error| CommandError::Encryption(error.to_string()))?;
    let serialized_ncryptsec = serialize_ncryptsec(&typed_ncryptsec)
        .map_err(|error| CommandError::Encryption(error.to_string()))?;

    let entry_prefix: String = pubkey_hex.chars().take(8).collect();
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0);
    let keychain_entry = format!("arcadestr_nip49_{entry_prefix}_{timestamp}");

    store_ncryptsec_in_keychain(&keychain_entry, &serialized_ncryptsec)
        .await
        .map_err(|error| {
            error!("Failed to persist ncryptsec to keychain: {error}");
            CommandError::Keychain(error.to_string())
        })?;

    Ok(ExportKeyResult {
        ncryptsec: serialized_ncryptsec,
        keychain_entry,
    })
}

pub async fn import_encrypted_key(
    _state: &crate::AppState,
    request: ImportKeyRequest,
) -> Result<ImportKeyResult, CommandError> {
    validate_nip49_password(&request.password)
        .map_err(|error| CommandError::InvalidInput(error.to_string()))?;

    let typed_ncryptsec = parse_ncryptsec(&request.ncryptsec)
        .map_err(|error| CommandError::InvalidInput(error.to_string()))?;
    let private_key_hex =
        decrypt_private_key_nip49(&typed_ncryptsec, &request.password).map_err(|error| {
            error!("Failed to decrypt NIP-49 payload during import: {error}");
            CommandError::Decryption(error.to_string())
        })?;
    let keys = Keys::parse(&private_key_hex)
        .map_err(|error| CommandError::InvalidInput(format!("Invalid private key: {error}")))?;

    Ok(ImportKeyResult {
        success: true,
        pubkey: keys.public_key().to_hex(),
    })
}

pub async fn verify_nip05_identity(
    _state: &crate::AppState,
    request: VerifyNip05Request,
) -> Result<VerifyNip05Result, CommandError> {
    let parsed = parse_nip05_identifier(&request.nip05)
        .map_err(|error| CommandError::InvalidInput(error.to_string()))?;

    let normalized_identifier = format!("{}@{}", parsed.local_part, parsed.domain);

    let verification = core_verify_nip05_identity(
        _state.http_client.as_ref(),
        &normalized_identifier,
        &request.expected_pubkey,
    )
    .await
    .map_err(|error| {
        let details = error.to_string();
        error!(
            "NIP-05 verification failed for '{}': {details}",
            request.nip05
        );
        match error {
            arcadestr_core::nostr::NostrError::RelayError(_) => CommandError::Http(details),
            _ => CommandError::Nip05(details),
        }
    })?;

    Ok(VerifyNip05Result {
        nip05: verification.nip05,
        verified: verification.verified,
        relays: verification.relays,
        error: None,
    })
}

pub async fn nip49_import(
    request: Nip49ImportRequest,
    state: &crate::AppState,
) -> Result<String, CommandError> {
    validate_nip49_format(&request.ncryptsec)
        .map_err(|error| CommandError::InvalidInput(error.to_string()))?;
    validate_nip49_password(&request.password)
        .map_err(|error| CommandError::InvalidInput(error.to_string()))?;
    let _version = extract_nip49_version(&request.ncryptsec)
        .map_err(|error| CommandError::InvalidInput(error.to_string()))?;

    let result = import_encrypted_key(
        state,
        ImportKeyRequest {
            ncryptsec: request.ncryptsec,
            password: request.password,
        },
    )
    .await?;

    Ok(format!("Import successful for pubkey {}", result.pubkey))
}

pub async fn nip49_export(
    npub: String,
    password: String,
    state: &crate::AppState,
) -> Result<Nip49ExportResult, CommandError> {
    parse_nip19_identifier(&npub).map_err(|error| CommandError::InvalidInput(error.to_string()))?;
    validate_nip49_password(&password)
        .map_err(|error| CommandError::InvalidInput(error.to_string()))?;

    let result = export_encrypted_key(
        state,
        ExportKeyRequest {
            password,
            scrypt_n: None,
        },
    )
    .await?;

    let active_npub = {
        let auth = state.auth.lock().await;
        let public_key = auth.public_key().ok_or(CommandError::NoActiveKey)?;
        public_key
            .to_bech32()
            .map_err(|error| CommandError::InvalidInput(error.to_string()))?
    };

    if active_npub != npub {
        return Err(CommandError::InvalidInput(
            "Requested npub does not match active authenticated key".to_string(),
        ));
    }

    Ok(Nip49ExportResult {
        npub,
        ncryptsec: result.ncryptsec,
        deferred: false,
        message: format!(
            "NIP-49 encrypted backup exported and stored with entry id {}",
            result.keychain_entry
        ),
    })
}

pub async fn verify_nip05(
    identifier: String,
    expected_npub: String,
    state: &crate::AppState,
) -> Result<Nip05Status, CommandError> {
    let expected_pubkey =
        if expected_npub.starts_with("npub1") || expected_npub.starts_with("nprofile1") {
            parse_nip19_identifier(&expected_npub)
                .map_err(|error| CommandError::InvalidInput(error.to_string()))?
                .pubkey
        } else {
            expected_npub
        };

    let result = verify_nip05_identity(
        state,
        VerifyNip05Request {
            nip05: identifier.clone(),
            expected_pubkey,
        },
    )
    .await?;

    let parsed = parse_nip05_identifier(&result.nip05)
        .map_err(|error| CommandError::InvalidInput(error.to_string()))?;

    Ok(Nip05Status {
        identifier: result.nip05.clone(),
        normalized_identifier: result.nip05,
        local_part: parsed.local_part,
        domain: parsed.domain,
        verified: result.verified,
        status: if result.verified {
            "verified".to_string()
        } else {
            "failed".to_string()
        },
        message: if result.verified {
            "NIP-05 identity verified".to_string()
        } else {
            "NIP-05 identity verification failed".to_string()
        },
    })
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

pub fn serialize_fetch_earned_badges_result(
    badges: &[EarnedBadgeSummary],
) -> Result<serde_json::Value, CommandError> {
    serde_json::to_value(badges).map_err(|error| CommandError::InvalidInput(error.to_string()))
}

pub fn serialize_fetch_profile_badges_result(
    badges: &[ProfileBadgeEntry],
) -> Result<serde_json::Value, CommandError> {
    serde_json::to_value(badges).map_err(|error| CommandError::InvalidInput(error.to_string()))
}

pub async fn fetch_earned_badges<S>(
    state: &S,
    profile_pubkey: String,
) -> Result<Vec<EarnedBadgeSummary>, CommandError>
where
    S: BadgeCommandState + ?Sized,
{
    let normalized_pubkey = normalize_profile_pubkey_identifier(&profile_pubkey)?;

    let (nostr, database) = state.badge_command_handles();
    let relay_manager = {
        let nostr = nostr.lock().await;
        nostr.relay_manager()
    };
    let client = {
        let manager = relay_manager.lock().await;
        manager.get_client_arc()
    };

    arcadestr_core::achievements::fetch_user_badges(client, database.as_ref(), &normalized_pubkey)
        .await
        .map_err(|error| CommandError::Achievements(error.to_string()))
}

pub async fn fetch_profile_badges<S>(
    state: &S,
    profile_pubkey: String,
) -> Result<Vec<ProfileBadgeEntry>, CommandError>
where
    S: BadgeCommandState + ?Sized,
{
    let normalized_pubkey = normalize_profile_pubkey_identifier(&profile_pubkey)?;

    let (nostr, database) = state.badge_command_handles();
    let relay_manager = {
        let nostr = nostr.lock().await;
        nostr.relay_manager()
    };
    let client = {
        let manager = relay_manager.lock().await;
        manager.get_client_arc()
    };

    arcadestr_core::achievements::fetch_profile_badges(
        client,
        database.as_ref(),
        &normalized_pubkey,
    )
    .await
    .map_err(|error| CommandError::Achievements(error.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_profile_pubkey_identifier_accepts_hex_and_npub() {
        let hex = "7c4cf6fb7248c4580e7215244f2f3f0ec3de6f7d9862092f155cb7dc39034f3c".to_string();
        let npub = nostr::key::PublicKey::parse(&hex)
            .expect("hex test vector should parse")
            .to_bech32()
            .expect("npub conversion should succeed");

        let normalized_hex =
            normalize_profile_pubkey_identifier(&hex).expect("hex pubkey should normalize");
        let normalized_npub = normalize_profile_pubkey_identifier(&npub)
            .expect("npub identifier should normalize");

        assert_eq!(normalized_hex, hex);
        assert_eq!(normalized_npub, hex);
    }

    #[test]
    fn normalize_profile_pubkey_identifier_rejects_invalid_identifier() {
        let result = normalize_profile_pubkey_identifier("not-a-pubkey");
        assert!(result.is_err());
    }
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
