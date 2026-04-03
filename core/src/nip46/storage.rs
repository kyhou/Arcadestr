// NIP-46 Secure Storage Layer
//
// This module is the ONLY place in the codebase that reads/writes the OS keychain.
// Uses tauri-plugin-keyring for cross-platform OS-native secure storage.
//
// ## Profile Metadata Cache
//
// Profile metadata (names, pubkeys, IDs) is cached locally in addition to the keyring.
// This ensures profiles remain visible in the UI even when the OS keyring is temporarily
// unavailable (locked, user canceled password prompt, DBus disconnected, etc.).
// Secrets (nsec, bunker URIs) remain exclusively in the keyring for security.

use keyring::Entry;
use nostr::{nips::nip46::NostrConnectURI, Keys, ToBech32};
use serde_json;
use std::path::PathBuf;
use std::sync::Mutex;
use tracing::{debug, info, warn};

use crate::nip46::types::{Nip46KeyringError, Nip46UriType, ProfileMetadata, SavedProfile};

/// Service name for keyring entries - MUST be "arcadestr-auth"
const SERVICE_NAME: &str = "arcadestr-auth";

/// Key for storing the profile index
const PROFILE_INDEX_KEY: &str = "profile_index";

/// Key for storing the last active profile ID
const LAST_ACTIVE_KEY: &str = "last_active_profile";

/// Filename for local profile metadata cache (non-sensitive data only)
const PROFILE_CACHE_FILENAME: &str = "profile_metadata_cache.json";

/// Lazy-static storage for the cache directory path
static CACHE_DIR: Mutex<Option<PathBuf>> = Mutex::new(None);

/// Set the directory for local profile metadata cache.
/// This should be called once during app initialization.
pub fn set_profile_cache_dir(dir: PathBuf) {
    let display_path = dir.display().to_string();
    let mut cache_dir = CACHE_DIR.lock().unwrap();
    *cache_dir = Some(dir);
    info!("Profile metadata cache directory set to: {}", display_path);
}

/// Get the path to the local profile metadata cache file.
fn get_profile_cache_path() -> Option<PathBuf> {
    let cache_dir = CACHE_DIR.lock().unwrap();
    cache_dir
        .as_ref()
        .map(|dir| dir.join(PROFILE_CACHE_FILENAME))
}

/// Extracts the keyring storage key for a profile.
///
/// For bunker:// URIs, uses the remote signer public key from the URI.
/// For nostrconnect:// URIs, uses the user's public key (obtained during handshake).
///
/// # Errors
/// Returns `Nip46KeyringError::UriParse` if the bunker URI lacks a remote signer pubkey.
fn extract_profile_key(profile: &SavedProfile) -> Result<String, Nip46KeyringError> {
    let uri_type = Nip46UriType::from_uri(&profile.bunker_uri.to_string());

    match uri_type {
        Nip46UriType::Bunker => {
            // For bunker://, extract remote signer pubkey from URI
            profile
                .bunker_uri
                .remote_signer_public_key()
                .ok_or_else(|| {
                    Nip46KeyringError::UriParse("No remote signer public key in URI".to_string())
                })
                .map(|pk| pk.to_hex())
        }
        Nip46UriType::NostrConnect => {
            // For nostrconnect://, use the user's pubkey (obtained during handshake)
            Ok(profile.user_pubkey.to_hex())
        }
    }
}

/// Persist a SavedProfile's secrets to the OS keychain.
/// Call this immediately after a successful NIP-46 connection.
///
/// Account identifier is the bunker's pubkey (from bunker_uri) as a string.
/// Ephemeral secret key is stored as bech32 (nsec1...).
pub fn save_profile_to_keyring(profile: &SavedProfile) -> Result<(), Nip46KeyringError> {
    info!("Saving profile {} to keyring", profile.id);

    // Get the appropriate key based on URI type (bunker:// or nostrconnect://)
    let profile_key = extract_profile_key(profile)?;

    // Store app_keys secret key as bech32 (nsec1...)
    let secret_bech32 =
        profile.app_keys.secret_key().to_bech32().map_err(|e| {
            Nip46KeyringError::Serialization(format!("Failed to encode secret: {}", e))
        })?;
    let app_key_entry = Entry::new(SERVICE_NAME, &profile_key)?;
    app_key_entry.set_password(&secret_bech32)?;
    debug!("Stored app_key (bech32) for profile key {}", profile_key);

    // Store bunker URI (serialize to string)
    let uri_entry = Entry::new(SERVICE_NAME, &format!("{}_uri", profile_key))?;
    uri_entry.set_password(&profile.bunker_uri.to_string())?;
    debug!("Stored bunker_uri for profile key {}", profile_key);

    // Update profile index
    add_to_profile_index(profile)?;

    // Also save to local cache as fallback (non-sensitive metadata only)
    if let Err(e) = save_profile_metadata_to_local_cache(profile) {
        warn!("Failed to save profile metadata to local cache: {}", e);
        // Don't fail the whole operation if cache fails
    }

    info!("Profile {} saved successfully to keyring", profile.id);

    Ok(())
}

/// Reconstruct a SavedProfile from the OS keychain by bunker pubkey.
/// Returns None if the profile does not exist in the keychain.
///
/// Attempts to load existing ephemeral nsec from keychain.
/// If found, parses it with Keys::parse.
pub fn load_profile_from_keyring(bunker_pubkey: &str) -> Option<SavedProfile> {
    debug!(
        "Loading profile for bunker pubkey {} from keyring",
        bunker_pubkey
    );

    // Load app_keys secret key (bech32 format)
    let app_key_entry = Entry::new(SERVICE_NAME, bunker_pubkey).ok()?;
    let secret_bech32 = app_key_entry.get_password().ok()?;

    // Parse the bech32 secret key
    let app_keys = Keys::parse(&secret_bech32).ok()?;

    // Load bunker URI string and parse it
    let uri_entry = Entry::new(SERVICE_NAME, &format!("{}_uri", bunker_pubkey)).ok()?;
    let bunker_uri_str = uri_entry.get_password().ok()?;
    let bunker_uri = NostrConnectURI::parse(&bunker_uri_str).ok()?;

    // Load metadata from index by bunker pubkey
    let metadata = get_profile_metadata_by_pubkey(bunker_pubkey)?;

    Some(SavedProfile {
        id: metadata.id,
        name: metadata.name,
        user_pubkey: nostr::PublicKey::from_hex(&metadata.pubkey_hex).ok()?,
        bunker_uri,
        app_keys,
    })
}

/// Delete a profile's secrets from the keychain for the given key.
/// The key can be either a bunker pubkey (for new profiles) or a profile ID (for old profiles).
/// Call entry.delete_password() for the given key.
pub fn delete_profile_from_keyring(key: &str) -> Result<(), Nip46KeyringError> {
    info!("Deleting profile for key {} from keyring", key);

    // Delete app_key (ephemeral nsec)
    match Entry::new(SERVICE_NAME, key) {
        Ok(entry) => {
            if let Err(e) = entry.delete_credential() {
                warn!("Failed to delete app_key for key {}: {}", key, e);
            } else {
                debug!("Deleted app_key for key {}", key);
            }
        }
        Err(e) => warn!("Failed to access app_key entry for key {}: {}", key, e),
    }

    // Delete bunker_uri
    match Entry::new(SERVICE_NAME, &format!("{}_uri", key)) {
        Ok(entry) => {
            if let Err(e) = entry.delete_credential() {
                warn!("Failed to delete bunker_uri for key {}: {}", key, e);
            } else {
                debug!("Deleted bunker_uri for key {}", key);
            }
        }
        Err(e) => warn!("Failed to access bunker_uri entry for key {}: {}", key, e),
    }

    // Remove from index by profile ID
    // Try to find the profile by checking if this key matches any profile's ID
    remove_from_profile_index(key)?;

    // Also remove from local cache
    if let Err(e) = remove_profile_from_local_cache(key) {
        warn!("Failed to remove profile from local cache: {}", e);
        // Don't fail the whole operation if cache removal fails
    }

    info!("Profile for key {} deleted from keyring", key);

    Ok(())
}

/// Load all saved profile metadata (no secrets) for display in the UI.
/// Returns a Vec of ProfileMetadata — NO secrets are included.
///
/// This function first tries to load from the OS keyring. If the keyring
/// is unavailable (locked, user canceled password prompt, etc.), it falls
/// back to the local file-based cache. This ensures profiles remain visible
/// in the UI even when the keyring is temporarily inaccessible.
pub fn list_profile_index() -> Vec<ProfileMetadata> {
    // First, try to load from keyring (authoritative source)
    match load_profile_index() {
        Ok(index) => {
            // Sync to local cache for future fallback
            if let Err(e) = save_profile_index_to_local_cache(&index) {
                warn!("Failed to sync profile index to local cache: {}", e);
            }
            return index;
        }
        Err(e) => {
            warn!(
                "Failed to load profile index from keyring: {}. Falling back to local cache.",
                e
            );
        }
    }

    // Fallback: try to load from local cache
    match load_profile_index_from_local_cache() {
        Ok(index) => {
            info!("Loaded {} profiles from local cache fallback", index.len());
            index
        }
        Err(e) => {
            warn!("Failed to load profile index from local cache: {}", e);
            vec![]
        }
    }
}

/// Check if a profile exists in the keyring by bunker pubkey.
pub fn profile_exists(bunker_pubkey: &str) -> bool {
    load_profile_from_keyring(bunker_pubkey).is_some()
}

/// Update the profile index with new metadata.
fn add_to_profile_index(profile: &SavedProfile) -> Result<(), Nip46KeyringError> {
    let mut index = load_profile_index()?;

    // Remove existing entry if present (by profile key)
    let profile_key = extract_profile_key(profile)?;
    index.retain(|p: &ProfileMetadata| p.pubkey_hex != profile_key);

    // Add new entry
    let metadata = ProfileMetadata {
        id: profile.id.clone(),
        name: profile.name.clone(),
        pubkey_bech32: profile.user_pubkey.to_bech32().map_err(|e| {
            Nip46KeyringError::Serialization(format!("Failed to encode pubkey: {}", e))
        })?,
        pubkey_hex: profile.user_pubkey.to_hex(),
        bunker_pubkey_hex: profile_key,
        picture: None,
        display_name: None,
        username: None,
        nip05: None,
        about: None,
    };
    index.push(metadata);

    save_profile_index(&index)?;
    Ok(())
}

/// Remove a profile from the index by profile ID or bunker pubkey.
/// Checks both the `id` field and `bunker_pubkey_hex` field for a match.
fn remove_from_profile_index(key: &str) -> Result<(), Nip46KeyringError> {
    let mut index = load_profile_index()?;
    // Remove the profile with matching ID or bunker_pubkey_hex
    let original_len = index.len();
    index.retain(|p: &ProfileMetadata| p.id != key && p.bunker_pubkey_hex != key);

    if index.len() < original_len {
        save_profile_index(&index)?;
        info!("Removed profile with key {} from index", key);
    } else {
        warn!("Profile with key {} not found in index for removal", key);
    }
    Ok(())
}

/// Load the profile index from keyring.
fn load_profile_index() -> Result<Vec<ProfileMetadata>, Nip46KeyringError> {
    let entry = Entry::new(SERVICE_NAME, PROFILE_INDEX_KEY)?;

    match entry.get_password() {
        Ok(json_str) => {
            let index: Vec<ProfileMetadata> = serde_json::from_str(&json_str).map_err(|e| {
                Nip46KeyringError::Serialization(format!("Failed to parse profile index: {}", e))
            })?;
            Ok(index)
        }
        Err(keyring::Error::NoEntry) => {
            // No index yet, return empty
            Ok(vec![])
        }
        Err(e) => Err(Nip46KeyringError::Keyring(e.to_string())),
    }
}

/// Save the profile index to keyring.
fn save_profile_index(index: &[ProfileMetadata]) -> Result<(), Nip46KeyringError> {
    let entry = Entry::new(SERVICE_NAME, PROFILE_INDEX_KEY)?;
    let json_str = serde_json::to_string(index).map_err(|e| {
        Nip46KeyringError::Serialization(format!("Failed to serialize profile index: {}", e))
    })?;
    entry.set_password(&json_str)?;
    Ok(())
}

// ============================================================================
// Local File-Based Cache for Profile Metadata (Fallback)
// ============================================================================
// These functions provide a fallback cache for profile metadata when the OS
// keyring is temporarily unavailable. Only non-sensitive metadata is cached
// locally - secrets (nsec, bunker URIs) remain exclusively in the keyring.

/// Save profile metadata to local file cache (non-sensitive data only).
/// This is used as a fallback when the keyring is unavailable.
fn save_profile_metadata_to_local_cache(profile: &SavedProfile) -> Result<(), String> {
    let cache_path = get_profile_cache_path().ok_or("Cache directory not set")?;

    // Load existing cache
    let mut cache = load_profile_index_from_local_cache().unwrap_or_default();

    // Get the appropriate key based on URI type
    let profile_key = extract_profile_key(profile)
        .map_err(|e| format!("Failed to extract profile key: {}", e))?;

    // Remove existing entry if present
    cache.retain(|p: &ProfileMetadata| p.pubkey_hex != profile.user_pubkey.to_hex());

    // Add new entry
    let metadata = ProfileMetadata {
        id: profile.id.clone(),
        name: profile.name.clone(),
        pubkey_bech32: profile
            .user_pubkey
            .to_bech32()
            .map_err(|e| format!("Failed to encode pubkey: {}", e))?,
        pubkey_hex: profile.user_pubkey.to_hex(),
        bunker_pubkey_hex: profile_key,
        picture: None,
        display_name: None,
        username: None,
        nip05: None,
        about: None,
    };
    cache.push(metadata);

    // Save to file
    save_profile_index_to_local_cache(&cache)
}

/// Save profile index to local file cache.
fn save_profile_index_to_local_cache(index: &[ProfileMetadata]) -> Result<(), String> {
    let cache_path = get_profile_cache_path().ok_or("Cache directory not set")?;

    // Ensure parent directory exists
    if let Some(parent) = cache_path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create cache directory: {}", e))?;
    }

    let json_str = serde_json::to_string_pretty(index)
        .map_err(|e| format!("Failed to serialize profile index: {}", e))?;

    std::fs::write(&cache_path, json_str)
        .map_err(|e| format!("Failed to write profile cache: {}", e))?;

    debug!(
        "Saved {} profiles to local cache at: {}",
        index.len(),
        cache_path.display()
    );
    Ok(())
}

/// Load profile index from local file cache.
fn load_profile_index_from_local_cache() -> Result<Vec<ProfileMetadata>, String> {
    let cache_path = get_profile_cache_path().ok_or("Cache directory not set")?;

    match std::fs::read_to_string(&cache_path) {
        Ok(json_str) => {
            let index: Vec<ProfileMetadata> = serde_json::from_str(&json_str)
                .map_err(|e| format!("Failed to parse profile cache: {}", e))?;
            debug!("Loaded {} profiles from local cache", index.len());
            Ok(index)
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            // No cache file yet, return empty
            Ok(vec![])
        }
        Err(e) => Err(format!("Failed to read profile cache: {}", e)),
    }
}

/// Remove a profile from the local cache.
fn remove_profile_from_local_cache(key: &str) -> Result<(), String> {
    let cache_path = get_profile_cache_path().ok_or("Cache directory not set")?;

    // Load existing cache
    let mut cache = load_profile_index_from_local_cache().unwrap_or_default();

    let original_len = cache.len();
    cache.retain(|p: &ProfileMetadata| p.id != key && p.bunker_pubkey_hex != key);

    if cache.len() < original_len {
        save_profile_index_to_local_cache(&cache)?;
        info!("Removed profile with key {} from local cache", key);
    }

    Ok(())
}

/// Get metadata for a specific profile from the index by bunker pubkey.
pub fn get_profile_metadata_by_pubkey(bunker_pubkey: &str) -> Option<ProfileMetadata> {
    let index = list_profile_index();
    index
        .into_iter()
        .find(|p| p.bunker_pubkey_hex == bunker_pubkey)
}

/// Get metadata for a specific profile from the index by profile ID.
pub fn get_profile_metadata_by_id(profile_id: &str) -> Option<ProfileMetadata> {
    let index = list_profile_index();
    index.into_iter().find(|p| p.id == profile_id)
}

/// Find the bunker pubkey for a profile by its ID.
/// This is used for migrating old profiles that don't have bunker_pubkey_hex stored.
/// Returns None if the profile is not found or if the bunker pubkey cannot be determined.
pub fn find_bunker_pubkey_by_profile_id(profile_id: &str) -> Option<String> {
    // First, try to get the metadata
    let metadata = get_profile_metadata_by_id(profile_id)?;

    // If the metadata already has the bunker pubkey, return it
    if !metadata.bunker_pubkey_hex.is_empty() {
        return Some(metadata.bunker_pubkey_hex);
    }

    // For old profiles without bunker_pubkey_hex, we need to find it
    // The profile ID is stored in the keyring entry names, so we can search for it
    // The app_key entry is named: "arcadestr-auth/{bunker_pubkey}"
    // The uri entry is named: "arcadestr-auth/{bunker_pubkey}_uri"

    // Since we can't easily iterate all keyring entries, we'll try a different approach:
    // Load the profile using the user_pubkey as a hint
    // This is a best-effort migration - if it fails, the user will need to re-add the profile

    // For now, return None and let the caller handle the error
    // The user will need to delete and re-add the profile
    None
}

/// Migrate old profile metadata to include bunker_pubkey_hex.
/// This should be called when loading profiles to ensure compatibility.
pub fn migrate_profile_metadata(
    profile_id: &str,
    bunker_pubkey: &str,
) -> Result<(), Nip46KeyringError> {
    let mut index = load_profile_index()?;

    // Find and update the profile
    let mut found = false;
    for profile in &mut index {
        if profile.id == profile_id {
            profile.bunker_pubkey_hex = bunker_pubkey.to_string();
            found = true;
            break;
        }
    }

    if found {
        save_profile_index(&index)?;
    }

    Ok(())
}

/// Migrate a profile from file-based storage to keyring storage.
/// This is used for one-time migration of existing profiles.
pub fn migrate_profile_to_keyring(
    id: &str,
    name: &str,
    user_pubkey: nostr::PublicKey,
    bunker_uri: &NostrConnectURI,
    app_keys: &Keys,
) -> Result<(), Nip46KeyringError> {
    let profile = SavedProfile {
        id: id.to_string(),
        name: name.to_string(),
        user_pubkey,
        bunker_uri: bunker_uri.clone(),
        app_keys: app_keys.clone(),
    };

    save_profile_to_keyring(&profile)
}

/// Set the last active profile ID in the keyring.
/// This is used to restore the session on app startup.
pub fn set_last_active_profile_id(profile_id: &str) -> Result<(), Nip46KeyringError> {
    let entry = Entry::new(SERVICE_NAME, LAST_ACTIVE_KEY)?;
    entry.set_password(profile_id)?;
    info!("Set last active profile ID: {}", profile_id);
    Ok(())
}

/// Get the last active profile ID from the keyring.
/// Returns None if no last active profile is set.
pub fn get_last_active_profile_id() -> Option<String> {
    let entry = match Entry::new(SERVICE_NAME, LAST_ACTIVE_KEY) {
        Ok(e) => e,
        Err(e) => {
            warn!("Failed to access keyring for last active profile: {}", e);
            return None;
        }
    };

    match entry.get_password() {
        Ok(id) => {
            if id.is_empty() {
                None
            } else {
                Some(id)
            }
        }
        Err(keyring::Error::NoEntry) => None,
        Err(e) => {
            warn!("Failed to get last active profile ID: {}", e);
            None
        }
    }
}

/// Clear the last active profile ID from the keyring.
/// Called on logout.
pub fn clear_last_active_profile_id() {
    if let Ok(entry) = Entry::new(SERVICE_NAME, LAST_ACTIVE_KEY) {
        if let Err(e) = entry.delete_credential() {
            warn!("Failed to clear last active profile ID: {}", e);
        } else {
            info!("Cleared last active profile ID");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::{nips::nip46::NostrConnectURI, Keys};

    #[test]
    fn test_service_name() {
        assert_eq!(SERVICE_NAME, "arcadestr-auth");
        assert_eq!(PROFILE_INDEX_KEY, "profile_index");
    }

    #[test]
    fn test_profile_metadata_serialization() {
        let metadata = ProfileMetadata {
            id: "test-id".to_string(),
            name: "Test Account".to_string(),
            pubkey_bech32: "npub1...".to_string(),
            pubkey_hex: "abcdef".to_string(),
            bunker_pubkey_hex: "123456".to_string(),
            picture: None,
            display_name: None,
            username: None,
            nip05: None,
            about: None,
        };

        let json = serde_json::to_string(&metadata).unwrap();
        let deserialized: ProfileMetadata = serde_json::from_str(&json).unwrap();

        assert_eq!(metadata.id, deserialized.id);
        assert_eq!(metadata.name, deserialized.name);
        assert_eq!(metadata.pubkey_bech32, deserialized.pubkey_bech32);
        assert_eq!(metadata.bunker_pubkey_hex, deserialized.bunker_pubkey_hex);
    }

    #[test]
    fn test_nip46_uri_type_detection() {
        assert_eq!(
            Nip46UriType::from_uri("bunker://pubkey123"),
            Nip46UriType::Bunker
        );
        assert_eq!(
            Nip46UriType::from_uri("nostrconnect://pubkey456"),
            Nip46UriType::NostrConnect
        );
    }

    #[test]
    #[should_panic(expected = "Invalid NIP-46 URI")]
    fn test_nip46_uri_type_invalid() {
        Nip46UriType::from_uri("https://example.com");
    }

    #[test]
    fn test_extract_profile_key_nostrconnect() {
        // Create a test profile with nostrconnect:// URI
        let app_keys = Keys::generate();
        let user_keys = Keys::generate();

        // Create a nostrconnect:// URI
        let uri = NostrConnectURI::client(
            app_keys.public_key(),
            vec!["wss://relay.example.com".parse().unwrap()],
            "Test App",
        );

        let profile = SavedProfile {
            id: "test-id".to_string(),
            name: "Test Account".to_string(),
            user_pubkey: user_keys.public_key(),
            bunker_uri: uri,
            app_keys,
        };

        // For nostrconnect://, should return user_pubkey
        let key = extract_profile_key(&profile).unwrap();
        assert_eq!(key, user_keys.public_key().to_hex());
    }
}
