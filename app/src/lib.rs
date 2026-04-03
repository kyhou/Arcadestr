// Leptos application: shared UI components and pages for both desktop and web targets.

use leptos::prelude::*;
use serde::{Deserialize, Serialize};
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::spawn_local;

// web_sys for browser APIs (window.open, etc.)
#[cfg(target_arch = "wasm32")]
use web_sys;

// Module declarations
pub mod components;
pub mod models;
pub mod store;

// Import ProfileStore and related functions for store initialization and event handlers
use crate::store::{provide_profile_store, try_use_profile_store, use_profile_store, ProfileStore};

#[cfg(all(target_arch = "wasm32", feature = "web"))]
pub mod web_auth;

#[cfg(any(target_arch = "wasm32", not(feature = "web")))]
mod tauri_invoke;
pub use components::{
    AccountSelector, BackupManager, BrowseView, DetailView, ProfileView, PublishView,
};
pub use models::{GameListing, MarketplaceView, UserProfile, ZapInvoice, ZapRequest};

// =============================================================================
// Profile Event Types
// =============================================================================

/// Profile fetch progress event from desktop
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ProfileFetchProgress {
    pub completed: usize,
    pub total: usize,
}

// =============================================================================
// Profile Event Handlers
// =============================================================================

/// Setup Tauri event listeners for profile updates
#[cfg(any(target_arch = "wasm32", not(feature = "web")))]
pub fn setup_profile_event_handlers(profile_store: ProfileStore) {
    use wasm_bindgen_futures::spawn_local;

    spawn_local(async move {
        // Listen for individual profile fetched events
        let listen_result = crate::tauri_invoke::listen("profile_fetched", move |data| {
            // Parse the UserProfile from the event data
            if let Ok(profile) = serde_json::from_value::<UserProfile>(data.clone()) {
                profile_store.put(profile);
            } else if let Some(payload) = data.get("payload") {
                // Try parsing from payload field if wrapped
                if let Ok(profile) = serde_json::from_value::<UserProfile>(payload.clone()) {
                    profile_store.put(profile);
                }
            }
        })
        .await;

        if let Err(e) = listen_result {
            web_sys::console::error_1(
                &format!("Failed to listen for profile_fetched: {}", e).into(),
            );
        }

        // Optionally listen for progress events (can be used for UI progress bars)
        let _ = crate::tauri_invoke::listen("profile_fetch_progress", |_data| {
            // Progress events can be handled here if needed
            // For now, we just listen to prevent events from accumulating
        })
        .await;
    });
}

/// Fallback for non-Tauri targets
#[cfg(not(any(target_arch = "wasm32", not(feature = "web"))))]
pub fn setup_profile_event_handlers(_profile_store: ProfileStore) {
    // No-op on web target
}

// =============================================================================
// Profile Fetch Helpers
// =============================================================================

/// Fetch a profile and store it in the global cache
pub async fn fetch_and_store_profile(npub: String) -> Result<UserProfile, String> {
    let profile = invoke_fetch_profile(npub.clone(), None).await?;

    // Store in global cache
    if let Some(store) = use_context::<ProfileStore>() {
        store.put(profile.clone());
    }

    Ok(profile)
}

/// Batch fetch profiles that aren't in the cache
pub async fn fetch_missing_profiles(npubs: Vec<String>) -> Result<Vec<UserProfile>, String> {
    let store = use_context::<ProfileStore>();

    // Filter out already cached profiles
    let missing: Vec<String> = npubs
        .into_iter()
        .filter(|npub| {
            if let Some(ref s) = store {
                !s.has(npub)
            } else {
                true
            }
        })
        .collect();

    if missing.is_empty() {
        return Ok(vec![]);
    }

    // Fetch missing profiles (we'll add the batch command later)
    // For now, fetch individually
    let mut profiles = Vec::new();
    for npub in missing {
        if let Ok(profile) = fetch_and_store_profile(npub).await {
            profiles.push(profile);
        }
    }

    Ok(profiles)
}

/// Invoke get_cached_profiles Tauri command
#[cfg(any(target_arch = "wasm32", not(feature = "web")))]
pub async fn invoke_get_cached_profiles() -> Result<Vec<UserProfile>, String> {
    use crate::tauri_invoke::invoke;

    invoke("get_cached_profiles", serde_json::json!({})).await
}

#[cfg(not(any(target_arch = "wasm32", not(feature = "web"))))]
pub async fn invoke_get_cached_profiles() -> Result<Vec<UserProfile>, String> {
    Err("Tauri not available".to_string())
}

/// Invoke get_cached_profile Tauri command (single profile)
#[cfg(any(target_arch = "wasm32", not(feature = "web")))]
pub async fn invoke_get_cached_profile(npub: String) -> Result<Option<UserProfile>, String> {
    use crate::tauri_invoke::invoke;

    invoke("get_cached_profile", serde_json::json!({ "npub": npub })).await
}

#[cfg(not(any(target_arch = "wasm32", not(feature = "web"))))]
pub async fn invoke_get_cached_profile(_npub: String) -> Result<Option<UserProfile>, String> {
    Err("Tauri not available".to_string())
}

// =============================================================================
// Tauri Invoke Bridge
// =============================================================================

/// Arguments for connect_bunker command (new NIP-46 API)
#[derive(Serialize)]
#[allow(dead_code)]
struct ConnectBunkerArgs {
    identifier: String,
    display_name: String,
}

/// Arguments for publish_listing command
#[derive(Serialize)]
#[allow(dead_code)]
struct PublishListingArgs {
    listing: GameListing,
}

/// Arguments for fetch_listings command
#[derive(Serialize)]
#[allow(dead_code)]
struct FetchListingsArgs {
    limit: usize,
}

/// Arguments for fetch_listing_by_id command
#[derive(Serialize)]
#[allow(dead_code)]
struct FetchListingByIdArgs {
    publisher_npub: String,
    listing_id: String,
}

/// Arguments for request_invoice command
#[derive(Serialize)]
#[allow(dead_code)]
struct RequestInvoiceArgs {
    zap_request: ZapRequest,
}

/// Invoke connect_bunker Tauri command (new NIP-46 API)
/// Connects with a bunker URI or NIP-05 identifier
#[cfg(any(target_arch = "wasm32", not(feature = "web")))]
async fn invoke_connect_bunker(
    identifier: String,
    display_name: String,
) -> Result<serde_json::Value, String> {
    use crate::tauri_invoke::invoke;

    let connect_args = serde_json::json!({
        "identifier": identifier,
        "displayName": display_name
    });

    invoke("connect_bunker", connect_args).await
}

#[cfg(not(any(target_arch = "wasm32", not(feature = "web"))))]
async fn invoke_connect_bunker(
    _identifier: String,
    _display_name: String,
) -> Result<serde_json::Value, String> {
    Err("Tauri not available in web mode".to_string())
}

/// Invoke generate_nostrconnect_uri Tauri command
/// Available for both desktop (Tauri) and WASM builds
#[cfg(any(target_arch = "wasm32", not(feature = "web")))]
async fn invoke_generate_nostrconnect_uri(relay: String) -> Result<serde_json::Value, String> {
    use crate::tauri_invoke::invoke;

    let gen_args = serde_json::json!({
        "relay": relay
    });

    invoke("generate_nostrconnect_uri", gen_args).await
}

/// Fallback for environments where Tauri is not available
#[cfg(not(any(target_arch = "wasm32", not(feature = "web"))))]
async fn invoke_generate_nostrconnect_uri(_relay: String) -> Result<serde_json::Value, String> {
    Err("Tauri not available".to_string())
}

/// Invoke wait_for_nostrconnect_signer Tauri command
#[cfg(any(target_arch = "wasm32", not(feature = "web")))]
async fn invoke_wait_for_nostrconnect_signer(timeout_secs: u64) -> Result<String, String> {
    use crate::tauri_invoke::invoke;

    let args = serde_json::json!({
        "timeoutSecs": timeout_secs
    });

    invoke::<String>("wait_for_nostrconnect_signer", args).await
}

#[cfg(not(any(target_arch = "wasm32", not(feature = "web"))))]
async fn invoke_wait_for_nostrconnect_signer(_timeout_secs: u64) -> Result<String, String> {
    Err("Tauri not available".to_string())
}

/// Invoke connect_with_key Tauri command (for testing with raw private key)
#[cfg(any(target_arch = "wasm32", not(feature = "web")))]
async fn invoke_connect_with_key(key: String) -> Result<String, String> {
    use crate::tauri_invoke::invoke;

    let args = serde_json::json!({
        "key": key
    });

    invoke::<String>("connect_with_key", args).await
}

#[cfg(not(any(target_arch = "wasm32", not(feature = "web"))))]
async fn invoke_connect_with_key(_key: String) -> Result<String, String> {
    Err("Tauri not available in web mode".to_string())
}

/// Invoke connect_nip07 for web target (browser extension)
/// On web target: calls core directly via web_auth module.
/// On Tauri WASM target: NIP-07 is not supported (desktop uses NIP-46).
/// On native: returns error (compile-time eliminated).
pub async fn invoke_connect_nip07() -> Result<String, String> {
    #[cfg(all(target_arch = "wasm32", feature = "web"))]
    {
        crate::web_auth::web_connect_nip07().await
    }
    #[cfg(not(all(target_arch = "wasm32", feature = "web")))]
    {
        Err("NIP-07 only available on web target".to_string())
    }
}

/// Invoke get_public_key Tauri command
#[cfg(any(target_arch = "wasm32", not(feature = "web")))]
async fn invoke_get_public_key() -> Result<String, String> {
    use crate::tauri_invoke::invoke;

    invoke("get_public_key", serde_json::json!(null)).await
}

#[cfg(not(any(target_arch = "wasm32", not(feature = "web"))))]
async fn invoke_get_public_key() -> Result<String, String> {
    Err("Tauri not available in web mode".to_string())
}

/// Invoke is_authenticated Tauri command
#[cfg(any(target_arch = "wasm32", not(feature = "web")))]
async fn invoke_is_authenticated() -> Result<bool, String> {
    use crate::tauri_invoke::invoke;

    invoke("is_authenticated", serde_json::json!(null)).await
}

#[cfg(not(any(target_arch = "wasm32", not(feature = "web"))))]
async fn invoke_is_authenticated() -> Result<bool, String> {
    Err("Tauri not available in web mode".to_string())
}

/// Invoke disconnect Tauri command (legacy - use logout_nip46 for NIP-46)
#[cfg(any(target_arch = "wasm32", not(feature = "web")))]
async fn invoke_disconnect() -> Result<(), String> {
    use crate::tauri_invoke::invoke_void;
    invoke_void("disconnect", serde_json::json!({})).await
}

#[cfg(not(any(target_arch = "wasm32", not(feature = "web"))))]
async fn invoke_disconnect() -> Result<(), String> {
    Err("Tauri not available in web mode".to_string())
}

/// Invoke get_connected_relay_count Tauri command
#[cfg(any(target_arch = "wasm32", not(feature = "web")))]
async fn invoke_get_relay_count() -> Result<usize, String> {
    use crate::tauri_invoke::invoke;

    invoke("get_connected_relay_count", serde_json::json!(null)).await
}

#[cfg(not(any(target_arch = "wasm32", not(feature = "web"))))]
async fn invoke_get_relay_count() -> Result<usize, String> {
    Err("Tauri not available in web mode".to_string())
}

/// Invoke get_connected_relays Tauri command to get list of relay URLs
#[cfg(any(target_arch = "wasm32", not(feature = "web")))]
async fn invoke_get_connected_relays() -> Result<Vec<String>, String> {
    use crate::tauri_invoke::invoke;

    invoke("get_connected_relays", serde_json::json!(null)).await
}

#[cfg(not(any(target_arch = "wasm32", not(feature = "web"))))]
async fn invoke_get_connected_relays() -> Result<Vec<String>, String> {
    Err("Tauri not available in web mode".to_string())
}

/// Invoke fetch_and_save_user_profile Tauri command
/// Fetches and saves profile for the current authenticated user
#[cfg(any(target_arch = "wasm32", not(feature = "web")))]
async fn invoke_fetch_and_save_user_profile() -> Result<UserProfile, String> {
    use crate::tauri_invoke::invoke;

    invoke("fetch_and_save_user_profile", serde_json::json!(null)).await
}

#[cfg(not(any(target_arch = "wasm32", not(feature = "web"))))]
async fn invoke_fetch_and_save_user_profile() -> Result<UserProfile, String> {
    Err("Tauri not available in web mode".to_string())
}

// ─────────────────────────────────────────────────────────────────────────────
// Saved Users Functions
// ─────────────────────────────────────────────────────────────────────────────

/// Saved user data structure
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SavedUser {
    pub id: String,
    pub name: String,
    pub method: String,
    pub relay: Option<String>,
    pub uri: Option<String>,
    pub npub: String,
    pub created_at: i64,
    pub last_used_at: Option<i64>,
    pub display_name: Option<String>,
    pub username: Option<String>,
    pub picture: Option<String>,
    pub nip05: Option<String>,
    pub about: Option<String>,
    pub profile_updated_at: Option<i64>,
}

/// Saved users container
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SavedUsers {
    pub users: Vec<SavedUser>,
}

/// Invoke get_saved_users Tauri command
#[cfg(any(target_arch = "wasm32", not(feature = "web")))]
async fn invoke_get_saved_users() -> Result<SavedUsers, String> {
    use crate::tauri_invoke::invoke;

    invoke("get_saved_users", serde_json::json!(null)).await
}

#[cfg(not(any(target_arch = "wasm32", not(feature = "web"))))]
async fn invoke_get_saved_users() -> Result<SavedUsers, String> {
    Err("Tauri not available".to_string())
}

/// Invoke add_saved_user Tauri command
#[cfg(any(target_arch = "wasm32", not(feature = "web")))]
pub async fn invoke_add_saved_user(
    method: String,
    relay: Option<String>,
    uri: Option<String>,
    private_key: Option<String>,
    npub: String,
) -> Result<SavedUsers, String> {
    use crate::tauri_invoke::invoke;

    let args = serde_json::json!({
        "method": method,
        "relay": relay,
        "uri": uri,
        "privateKey": private_key,
        "npub": npub,
    });

    invoke("add_saved_user", args).await
}

#[cfg(not(any(target_arch = "wasm32", not(feature = "web"))))]
async fn invoke_add_saved_user(
    _method: String,
    _relay: Option<String>,
    _uri: Option<String>,
    _private_key: Option<String>,
    _npub: String,
) -> Result<SavedUsers, String> {
    Err("Tauri not available".to_string())
}

/// Invoke remove_saved_user Tauri command
#[cfg(any(target_arch = "wasm32", not(feature = "web")))]
async fn invoke_remove_saved_user(user_id: String) -> Result<SavedUsers, String> {
    use crate::tauri_invoke::invoke;

    invoke(
        "remove_saved_user",
        serde_json::json!({ "userId": user_id }),
    )
    .await
}

#[cfg(not(any(target_arch = "wasm32", not(feature = "web"))))]
async fn invoke_remove_saved_user(_user_id: String) -> Result<SavedUsers, String> {
    Err("Tauri not available".to_string())
}

/// Invoke rename_saved_user Tauri command
#[cfg(any(target_arch = "wasm32", not(feature = "web")))]
async fn invoke_rename_saved_user(user_id: String, new_name: String) -> Result<SavedUsers, String> {
    use crate::tauri_invoke::invoke;

    invoke(
        "rename_saved_user",
        serde_json::json!({ "userId": user_id, "newName": new_name }),
    )
    .await
}

#[cfg(not(any(target_arch = "wasm32", not(feature = "web"))))]
async fn invoke_rename_saved_user(
    _user_id: String,
    _new_name: String,
) -> Result<SavedUsers, String> {
    Err("Tauri not available".to_string())
}

/// Response from connect_saved_user
#[derive(Debug, Clone, Serialize, Deserialize)]
struct ConnectResponse {
    npub: String,
    profile: UserProfile,
}

/// Invoke connect_saved_user Tauri command
#[cfg(any(target_arch = "wasm32", not(feature = "web")))]
async fn invoke_connect_saved_user(user_id: String) -> Result<ConnectResponse, String> {
    use crate::tauri_invoke::invoke;

    invoke(
        "connect_saved_user",
        serde_json::json!({ "userId": user_id }),
    )
    .await
}

#[cfg(not(any(target_arch = "wasm32", not(feature = "web"))))]
async fn invoke_connect_saved_user(_user_id: String) -> Result<ConnectResponse, String> {
    Err("Tauri not available".to_string())
}

// =============================================================================
// NEW SECURE STORAGE API WRAPPERS
// =============================================================================

/// Check if any accounts exist in encrypted storage
#[cfg(any(target_arch = "wasm32", not(feature = "web")))]
async fn invoke_has_accounts() -> Result<bool, String> {
    use crate::tauri_invoke::invoke;
    invoke("has_accounts", serde_json::json!({})).await
}

#[cfg(not(any(target_arch = "wasm32", not(feature = "web"))))]
async fn invoke_has_accounts() -> Result<bool, String> {
    Err("Tauri not available in web mode".to_string())
}

/// Load active account for fast login (~4 seconds)
#[cfg(any(target_arch = "wasm32", not(feature = "web")))]
async fn invoke_load_active_account() -> Result<serde_json::Value, String> {
    use crate::tauri_invoke::invoke;
    invoke("load_active_account", serde_json::json!({})).await
}

#[cfg(not(any(target_arch = "wasm32", not(feature = "web"))))]
async fn invoke_load_active_account() -> Result<serde_json::Value, String> {
    Err("Tauri not available in web mode".to_string())
}

/// List all saved profiles (new NIP-46 API)
#[cfg(any(target_arch = "wasm32", not(feature = "web")))]
async fn invoke_list_saved_profiles() -> Result<serde_json::Value, String> {
    use crate::tauri_invoke::invoke;
    invoke("list_saved_profiles", serde_json::json!({})).await
}

#[cfg(not(any(target_arch = "wasm32", not(feature = "web"))))]
async fn invoke_list_saved_profiles() -> Result<serde_json::Value, String> {
    Err("Tauri not available in web mode".to_string())
}

/// Switch to a different profile (new NIP-46 API)
#[cfg(any(target_arch = "wasm32", not(feature = "web")))]
async fn invoke_switch_profile(profile_id: String) -> Result<serde_json::Value, String> {
    use crate::tauri_invoke::invoke;
    invoke(
        "switch_profile",
        serde_json::json!({ "profileId": profile_id }),
    )
    .await
}

#[cfg(not(any(target_arch = "wasm32", not(feature = "web"))))]
async fn invoke_switch_profile(_profile_id: String) -> Result<serde_json::Value, String> {
    Err("Tauri not available in web mode".to_string())
}

/// Login with nsec - creates encrypted local account
#[cfg(any(target_arch = "wasm32", not(feature = "web")))]
async fn invoke_login_with_nsec(
    nsec: String,
    name: Option<String>,
) -> Result<serde_json::Value, String> {
    use crate::tauri_invoke::invoke;
    invoke(
        "login_with_nsec",
        serde_json::json!({ "nsec": nsec, "name": name }),
    )
    .await
}

#[cfg(not(any(target_arch = "wasm32", not(feature = "web"))))]
async fn invoke_login_with_nsec(
    _nsec: String,
    _name: Option<String>,
) -> Result<serde_json::Value, String> {
    Err("Tauri not available in web mode".to_string())
}

/// Save NIP-46 remote account after successful connection
#[cfg(any(target_arch = "wasm32", not(feature = "web")))]
async fn invoke_save_nip46_account(
    uri: String,
    relay: String,
    name: Option<String>,
) -> Result<serde_json::Value, String> {
    use crate::tauri_invoke::invoke;
    invoke(
        "save_nip46_account",
        serde_json::json!({
            "uri": uri,
            "relay": relay,
            "name": name
        }),
    )
    .await
}

#[cfg(not(any(target_arch = "wasm32", not(feature = "web"))))]
async fn invoke_save_nip46_account(
    _uri: String,
    _relay: String,
    _name: Option<String>,
) -> Result<serde_json::Value, String> {
    Err("Tauri not available in web mode".to_string())
}

/// Delete a profile (new NIP-46 API)
#[cfg(any(target_arch = "wasm32", not(feature = "web")))]
async fn invoke_delete_profile(profile_id: String) -> Result<(), String> {
    use crate::tauri_invoke::invoke_void;
    invoke_void(
        "delete_profile",
        serde_json::json!({ "profileId": profile_id }),
    )
    .await
}

#[cfg(not(any(target_arch = "wasm32", not(feature = "web"))))]
async fn invoke_delete_profile(_profile_id: String) -> Result<(), String> {
    Err("Tauri not available in web mode".to_string())
}

/// Logout from NIP-46 session (new NIP-46 API)
#[cfg(any(target_arch = "wasm32", not(feature = "web")))]
async fn invoke_logout_nip46() -> Result<(), String> {
    use crate::tauri_invoke::invoke_void;
    invoke_void("logout_nip46", serde_json::json!({})).await
}

#[cfg(not(any(target_arch = "wasm32", not(feature = "web"))))]
async fn invoke_logout_nip46() -> Result<(), String> {
    Err("Tauri not available in web mode".to_string())
}

/// Start QR login session (new NIP-46 Flow B API)
#[cfg(any(target_arch = "wasm32", not(feature = "web")))]
async fn invoke_start_qr_login() -> Result<String, String> {
    use crate::tauri_invoke::invoke;
    invoke("start_qr_login", serde_json::json!({})).await
}

#[cfg(not(any(target_arch = "wasm32", not(feature = "web"))))]
async fn invoke_start_qr_login() -> Result<String, String> {
    Err("Tauri not available in web mode".to_string())
}

/// Check QR connection status (new NIP-46 Flow B API)
/// Returns profile info if connected, None if still waiting
#[cfg(any(target_arch = "wasm32", not(feature = "web")))]
async fn invoke_check_qr_connection() -> Result<Option<serde_json::Value>, String> {
    use crate::tauri_invoke::invoke;
    invoke("check_qr_connection", serde_json::json!({})).await
}

#[cfg(not(any(target_arch = "wasm32", not(feature = "web"))))]
async fn invoke_check_qr_connection() -> Result<Option<serde_json::Value>, String> {
    Err("Tauri not available in web mode".to_string())
}

/// Ping the bunker to check connection health (new NIP-46 API)
#[cfg(any(target_arch = "wasm32", not(feature = "web")))]
async fn invoke_ping_bunker() -> Result<serde_json::Value, String> {
    use crate::tauri_invoke::invoke;
    invoke("ping_bunker", serde_json::json!({})).await
}

#[cfg(not(any(target_arch = "wasm32", not(feature = "web"))))]
async fn invoke_ping_bunker() -> Result<serde_json::Value, String> {
    Err("Tauri not available in web mode".to_string())
}

/// Get NIP-46 connection status (new async auth API)
#[cfg(any(target_arch = "wasm32", not(feature = "web")))]
async fn invoke_get_connection_status() -> Result<serde_json::Value, String> {
    use crate::tauri_invoke::invoke;
    invoke("get_connection_status", serde_json::json!({})).await
}

#[cfg(not(any(target_arch = "wasm32", not(feature = "web"))))]
async fn invoke_get_connection_status() -> Result<serde_json::Value, String> {
    Err("Tauri not available in web mode".to_string())
}

/// Create encrypted backup of all accounts
#[cfg(any(target_arch = "wasm32", not(feature = "web")))]
async fn invoke_create_backup() -> Result<serde_json::Value, String> {
    use crate::tauri_invoke::invoke;
    invoke("create_backup", serde_json::json!({})).await
}

#[cfg(not(any(target_arch = "wasm32", not(feature = "web"))))]
async fn invoke_create_backup() -> Result<serde_json::Value, String> {
    Err("Tauri not available in web mode".to_string())
}

/// Restore accounts from backup
#[cfg(any(target_arch = "wasm32", not(feature = "web")))]
async fn invoke_restore_backup(backup_data: String) -> Result<serde_json::Value, String> {
    use crate::tauri_invoke::invoke;
    invoke(
        "restore_backup",
        serde_json::json!({ "backup_data": backup_data }),
    )
    .await
}

#[cfg(not(any(target_arch = "wasm32", not(feature = "web"))))]
async fn invoke_restore_backup(_backup_data: String) -> Result<serde_json::Value, String> {
    Err("Tauri not available in web mode".to_string())
}

/// Invoke publish_listing Tauri command
#[cfg(any(target_arch = "wasm32", not(feature = "web")))]
pub async fn invoke_publish_listing(listing: GameListing) -> Result<String, String> {
    use crate::tauri_invoke::invoke;

    let publish_args = serde_json::json!({
        "listing": listing
    });

    invoke("publish_listing", publish_args).await
}

#[cfg(not(any(target_arch = "wasm32", not(feature = "web"))))]
pub async fn invoke_publish_listing(_listing: GameListing) -> Result<String, String> {
    Err("Tauri not available in web mode".to_string())
}

/// Invoke fetch_listings Tauri command
#[cfg(any(target_arch = "wasm32", not(feature = "web")))]
pub async fn invoke_fetch_listings(limit: usize) -> Result<Vec<GameListing>, String> {
    use crate::tauri_invoke::invoke;

    let fetch_args = serde_json::json!({
        "limit": limit
    });

    invoke("fetch_listings", fetch_args).await
}

#[cfg(not(any(target_arch = "wasm32", not(feature = "web"))))]
pub async fn invoke_fetch_listings(_limit: usize) -> Result<Vec<GameListing>, String> {
    Err("Tauri not available in web mode".to_string())
}

/// Invoke fetch_listing_by_id Tauri command
#[cfg(any(target_arch = "wasm32", not(feature = "web")))]
pub async fn invoke_fetch_listing_by_id(
    publisher_npub: String,
    listing_id: String,
) -> Result<GameListing, String> {
    use crate::tauri_invoke::invoke;

    let fetch_args = serde_json::json!({
        "publisher_npub": publisher_npub,
        "listing_id": listing_id
    });

    invoke("fetch_listing_by_id", fetch_args).await
}

#[cfg(not(any(target_arch = "wasm32", not(feature = "web"))))]
pub async fn invoke_fetch_listing_by_id(
    _publisher_npub: String,
    _listing_id: String,
) -> Result<GameListing, String> {
    Err("Tauri not available in web mode".to_string())
}

/// Arguments for fetch_profile command
#[derive(Serialize)]
#[allow(dead_code)]
struct FetchProfileArgs {
    npub: String,
    additional_relays: Option<Vec<String>>,
}

/// Invoke fetch_profile Tauri command
#[cfg(any(target_arch = "wasm32", not(feature = "web")))]
pub async fn invoke_fetch_profile(
    npub: String,
    additional_relays: Option<Vec<String>>,
) -> Result<UserProfile, String> {
    use crate::tauri_invoke::invoke;

    let fetch_args = serde_json::json!({
        "npub": npub,
        "additional_relays": additional_relays
    });

    invoke("fetch_profile", fetch_args).await
}

#[cfg(not(any(target_arch = "wasm32", not(feature = "web"))))]
pub async fn invoke_fetch_profile(
    _npub: String,
    _additional_relays: Option<Vec<String>>,
) -> Result<UserProfile, String> {
    Err("Tauri not available in web mode".to_string())
}

/// Invoke request_invoice Tauri command
#[cfg(any(target_arch = "wasm32", not(feature = "web")))]
pub async fn invoke_request_invoice(zap_req: ZapRequest) -> Result<ZapInvoice, String> {
    use crate::tauri_invoke::invoke;

    let request_args = serde_json::json!({
        "zap_request": zap_req
    });

    invoke("request_invoice", request_args).await
}

#[cfg(not(any(target_arch = "wasm32", not(feature = "web"))))]
pub async fn invoke_request_invoice(_zap_req: ZapRequest) -> Result<ZapInvoice, String> {
    Err("Tauri not available in web mode".to_string())
}

// =============================================================================
// Reactive State
// =============================================================================

/// Account information from secure storage
#[derive(Debug, Clone)]
pub struct StoredAccount {
    pub id: String,
    pub npub: String,
    pub name: Option<String>,
    pub signing_mode: String,
    pub last_used: i64,
    pub is_current: bool, // ← NEW: indicates if this is the currently active account
    // Profile metadata fields
    pub picture: Option<String>,
    pub display_name: Option<String>,
    pub username: Option<String>,
    pub nip05: Option<String>,
    pub about: Option<String>,
}

/// Authentication context shared across components
#[derive(Clone)]
pub struct AuthContext {
    // Existing fields
    pub npub: RwSignal<Option<String>>,
    pub profile: RwSignal<Option<UserProfile>>,
    pub is_loading: RwSignal<bool>,
    pub error: RwSignal<Option<String>>,

    // NEW: Secure storage fields
    /// All stored accounts (for account switching UI)
    pub accounts: RwSignal<Vec<StoredAccount>>,
    /// Currently active account
    pub active_account: RwSignal<Option<StoredAccount>>,
    /// Whether accounts exist in new storage
    pub has_secure_accounts: RwSignal<bool>,

    // NEW: NIP-46 connection status
    /// Current connection state: "disconnected", "connecting", "connected", "failed"
    pub connection_status: RwSignal<String>,
    /// Connection error message if status is "failed"
    pub connection_error: RwSignal<Option<String>>,
}

impl AuthContext {
    pub fn new() -> Self {
        Self {
            npub: RwSignal::new(None),
            profile: RwSignal::new(None),
            is_loading: RwSignal::new(false),
            error: RwSignal::new(None),
            accounts: RwSignal::new(Vec::new()),
            active_account: RwSignal::new(None),
            has_secure_accounts: RwSignal::new(false),
            connection_status: RwSignal::new("disconnected".to_string()),
            connection_error: RwSignal::new(None),
        }
    }
}

impl Default for AuthContext {
    fn default() -> Self {
        Self::new()
    }
}

/// Helper function to parse account from JSON result
fn parse_account_from_result(result: &serde_json::Value) -> Result<StoredAccount, String> {
    let account = result.get("account").ok_or("Missing account field")?;

    Ok(StoredAccount {
        id: account
            .get("id")
            .and_then(|v| v.as_str())
            .ok_or("Missing id")?
            .to_string(),
        npub: account
            .get("npub")
            .and_then(|v| v.as_str())
            .ok_or("Missing npub")?
            .to_string(),
        name: account
            .get("name")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        signing_mode: account
            .get("signing_mode")
            .and_then(|v| v.as_str())
            .unwrap_or("Local")
            .to_string(),
        last_used: account
            .get("last_used")
            .and_then(|v| v.as_i64())
            .unwrap_or(0),
        is_current: account
            .get("is_current")
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
        // Profile metadata fields
        picture: account
            .get("picture")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        display_name: account
            .get("display_name")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        username: account
            .get("username")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        nip05: account
            .get("nip05")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        about: account
            .get("about")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
    })
}

/// Helper function to parse accounts list from JSON result
fn parse_accounts_list(result: &serde_json::Value) -> Result<Vec<StoredAccount>, String> {
    let accounts = result
        .get("accounts")
        .and_then(|a| a.as_array())
        .ok_or("Missing accounts field")?;

    let stored_accounts: Vec<StoredAccount> = accounts
        .iter()
        .filter_map(|acc| {
            Some(StoredAccount {
                id: acc.get("id")?.as_str()?.to_string(),
                npub: acc.get("npub")?.as_str()?.to_string(),
                name: acc
                    .get("name")
                    .and_then(|n| n.as_str())
                    .map(|s| s.to_string()),
                signing_mode: acc.get("signing_mode")?.as_str()?.to_string(),
                last_used: acc.get("last_used")?.as_i64()?,
                is_current: acc
                    .get("is_current")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false),
                // Parse profile metadata fields
                picture: acc
                    .get("picture")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string()),
                display_name: acc
                    .get("display_name")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string()),
                username: acc
                    .get("username")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string()),
                nip05: acc
                    .get("nip05")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string()),
                about: acc
                    .get("about")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string()),
            })
        })
        .collect();

    Ok(stored_accounts)
}

/// Generate a QR code SVG from a string
fn generate_qr_svg(data: &str) -> String {
    use qrcode::render::svg;
    use qrcode::{EcLevel, QrCode, Version};

    // Create QR code with medium error correction
    let code = QrCode::with_error_correction_level(data, EcLevel::M).unwrap_or_else(|_| {
        // Fallback: create a minimal QR code
        QrCode::new("ERROR").expect("Failed to create QR code")
    });

    // Render as SVG with styling
    let svg = code
        .render()
        .min_dimensions(200, 200)
        .max_dimensions(300, 300)
        .quiet_zone(true)
        .dark_color(svg::Color("#000000"))
        .light_color(svg::Color("#ffffff"))
        .build();

    svg
}

/// AuthContext methods for secure storage operations
impl AuthContext {
    /// Load list of all profiles from secure storage (new NIP-46 API)
    pub async fn load_profiles_list(&self) -> Result<(), String> {
        web_sys::console::log_1(&"[load_profiles_list] Starting...".into());
        match invoke_list_saved_profiles().await {
            Ok(result) => {
                web_sys::console::log_1(
                    &format!("[load_profiles_list] Got result: {:?}", result).into(),
                );
                if let Ok(accounts) = parse_accounts_list(&result) {
                    web_sys::console::log_1(
                        &format!("[load_profiles_list] Parsed {} accounts", accounts.len()).into(),
                    );
                    self.accounts.set(accounts.clone());

                    // Fetch profiles for all accounts to show pictures/names in the list
                    let store = try_use_profile_store();
                    web_sys::console::log_1(
                        &format!(
                            "[load_profiles_list] ProfileStore available: {}",
                            store.is_some()
                        )
                        .into(),
                    );
                    for account in accounts {
                        if let Some(store) = store.clone() {
                            let npub = account.npub.clone();
                            let name = account.name.clone();
                            web_sys::console::log_1(&format!("[load_profiles_list] Processing account: {} (npub: {}, name: {:?})", account.id, npub, name).into());
                            spawn_local(async move {
                                // First try to get from backend cache
                                match invoke_get_cached_profile(npub.clone()).await {
                                    Ok(Some(profile)) => {
                                        web_sys::console::log_1(
                                            &format!(
                                                "[LOAD_PROFILES] Cached profile for {}: {:?}",
                                                npub, profile.name
                                            )
                                            .into(),
                                        );
                                        store.put(profile);
                                    }
                                    Ok(None) => {
                                        web_sys::console::log_1(&format!("[LOAD_PROFILES] No cached profile for {}, fetching from relays", npub).into());
                                        // Not in cache, fetch from relays
                                        match invoke_fetch_profile(npub.clone(), None).await {
                                            Ok(profile) => {
                                                web_sys::console::log_1(&format!("[LOAD_PROFILES] Fetched profile for {}: {:?}", npub, profile.name).into());
                                                store.put(profile);
                                            }
                                            Err(e) => {
                                                web_sys::console::log_1(&format!("[LOAD_PROFILES] Failed to fetch profile for {}: {}", npub, e).into());
                                            }
                                        }
                                    }
                                    Err(e) => {
                                        web_sys::console::log_1(&format!("[LOAD_PROFILES] Error getting cached profile for {}: {}", npub, e).into());
                                    }
                                }
                            });
                        }
                    }

                    Ok(())
                } else {
                    web_sys::console::log_1(
                        &"[load_profiles_list] Failed to parse accounts".into(),
                    );
                    Err("Failed to parse profiles".to_string())
                }
            }
            Err(e) => {
                web_sys::console::log_1(&format!("[load_profiles_list] Error: {}", e).into());
                Err(e)
            }
        }
    }

    /// Load list of all accounts (alias for load_profiles_list)
    pub async fn load_accounts_list(&self) -> Result<(), String> {
        self.load_profiles_list().await
    }

    /// Switch to a different profile (new NIP-46 API)
    pub async fn switch_profile(&self, profile_id: String) -> Result<(), String> {
        self.is_loading.set(true);

        match invoke_switch_profile(profile_id).await {
            Ok(result) => {
                if let Ok(account) = parse_account_from_result(&result) {
                    self.active_account.set(Some(account.clone()));
                    self.npub.set(Some(account.npub.clone()));
                    self.is_loading.set(false);

                    // Start connection status polling for NIP-46 accounts
                    if account.signing_mode == "nip46" {
                        self.start_connection_status_polling().await;
                    }

                    // Explicitly fetch profile immediately to avoid delay
                    let npub_for_fetch = account.npub.clone();
                    let auth_for_fetch = self.clone();
                    spawn_local(async move {
                        web_sys::console::log_1(
                            &format!("[SWITCH] Immediate profile fetch for: {}", npub_for_fetch)
                                .into(),
                        );

                        // First try to get from backend cache
                        match invoke_get_cached_profile(npub_for_fetch.clone()).await {
                            Ok(Some(profile)) => {
                                web_sys::console::log_1(
                                    &format!(
                                        "[SWITCH] Got cached profile immediately: {:?}",
                                        profile.name
                                    )
                                    .into(),
                                );
                                auth_for_fetch.profile.set(Some(profile));
                            }
                            _ => {
                                web_sys::console::log_1(
                                    &"[SWITCH] No cached profile, fetching from relays...".into(),
                                );
                                // Fetch from relays
                                match invoke_fetch_profile(npub_for_fetch.clone(), None).await {
                                    Ok(profile) => {
                                        web_sys::console::log_1(
                                            &format!(
                                                "[SWITCH] Profile fetched from relays: {:?}",
                                                profile.name
                                            )
                                            .into(),
                                        );
                                        auth_for_fetch.profile.set(Some(profile));
                                    }
                                    Err(e) => {
                                        web_sys::console::log_1(
                                            &format!("[SWITCH] Profile fetch failed: {}", e).into(),
                                        );
                                    }
                                }
                            }
                        }
                    });

                    Ok(())
                } else {
                    self.is_loading.set(false);
                    Err("Failed to parse profile".to_string())
                }
            }
            Err(e) => {
                self.error.set(Some(e.clone()));
                self.is_loading.set(false);
                Err(e)
            }
        }
    }

    /// Switch to a different account (alias for switch_profile)
    pub async fn switch_account(&self, account_id: String) -> Result<(), String> {
        self.switch_profile(account_id).await
    }

    /// Login with nsec - creates encrypted local account
    pub async fn login_with_nsec(&self, nsec: String, name: Option<String>) -> Result<(), String> {
        self.is_loading.set(true);

        match invoke_login_with_nsec(nsec, name).await {
            Ok(result) => {
                if let Ok(account) = parse_account_from_result(&result) {
                    self.active_account.set(Some(account.clone()));
                    self.npub.set(Some(account.npub.clone()));
                    self.has_secure_accounts.set(true);
                    self.is_loading.set(false);
                    Ok(())
                } else {
                    self.is_loading.set(false);
                    Err("Failed to parse account".to_string())
                }
            }
            Err(e) => {
                self.error.set(Some(e.clone()));
                self.is_loading.set(false);
                Err(e)
            }
        }
    }

    /// Delete a profile (new NIP-46 API)
    pub async fn delete_profile(&self, profile_id: String) -> Result<(), String> {
        match invoke_delete_profile(profile_id).await {
            Ok(_) => {
                // Refresh profiles list
                self.load_profiles_list().await
            }
            Err(e) => Err(e),
        }
    }

    /// Delete an account (alias for delete_profile)
    pub async fn delete_account(&self, account_id: String) -> Result<(), String> {
        self.delete_profile(account_id).await
    }

    /// Logout from NIP-46 session (new NIP-46 API)
    pub async fn logout_nip46(&self) -> Result<(), String> {
        match invoke_logout_nip46().await {
            Ok(_) => {
                self.active_account.set(None);
                self.npub.set(None);
                self.connection_status.set("disconnected".to_string());
                self.connection_error.set(None);
                Ok(())
            }
            Err(e) => Err(e),
        }
    }

    /// Start polling for NIP-46 connection status
    /// This runs every 2 seconds until connection is established or fails
    pub async fn start_connection_status_polling(&self) {
        let auth = self.clone();
        spawn_local(async move {
            loop {
                match invoke_get_connection_status().await {
                    Ok(status) => {
                        let status_str = status
                            .get("status")
                            .and_then(|s| s.as_str())
                            .unwrap_or("unknown");

                        let error = status
                            .get("error")
                            .and_then(|e| e.as_str())
                            .map(|s| s.to_string());

                        auth.connection_status.set(status_str.to_string());
                        auth.connection_error.set(error);

                        // Stop polling if we reach a final state
                        if status_str == "connected" || status_str == "failed" {
                            break;
                        }
                    }
                    Err(_) => {
                        // Error getting status, stop polling
                        break;
                    }
                }

                // Wait 2 seconds before next poll
                #[cfg(target_arch = "wasm32")]
                {
                    use js_sys::Promise;
                    use wasm_bindgen_futures::JsFuture;
                    let _ = JsFuture::from(Promise::new(&mut |resolve, _| {
                        web_sys::window()
                            .unwrap()
                            .set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, 2000)
                            .unwrap();
                    }))
                    .await;
                }
            }
        });
    }
}

// =============================================================================
// Styles
// =============================================================================

const STYLES: &str = r#"
:root {
    --bg-primary: #0d0d0d;
    --bg-secondary: #1a1a1a;
    --bg-card: #141414;
    --accent: #f5821f;
    --accent-hover: #ff9a3d;
    --text-primary: #ffffff;
    --text-secondary: #a0a0a0;
    --text-muted: #666666;
    --error: #ff4444;
    --success: #44ff44;
    --border: #2a2a2a;
}

* {
    margin: 0;
    padding: 0;
    box-sizing: border-box;
}

body {
    font-family: 'SF Mono', 'Monaco', 'Inconsolata', 'Fira Code', monospace;
    background-color: var(--bg-primary);
    color: var(--text-primary);
    min-height: 100vh;
    line-height: 1.6;
}

.arcadestr-app {
    min-height: 100vh;
    display: flex;
    flex-direction: column;
}

/* Login styles */
.login-view {
    min-height: 100vh;
    display: flex;
    flex-direction: column;
    background-color: var(--bg-primary);
}

.login-container {
    flex: 1;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: flex-start;
    padding: 2rem;
    max-width: 600px;
    margin: 0 auto;
    width: 100%;
}

.login-header {
    text-align: center;
    margin-bottom: 2rem;
    margin-top: 2rem;
}

.login-header h1 {
    font-size: 2.5rem;
    font-weight: 700;
    color: var(--accent);
    margin-bottom: 0.5rem;
    letter-spacing: -0.02em;
}

.login-header .tagline {
    color: var(--text-secondary);
    font-size: 0.95rem;
}

.login-content {
    width: 100%;
    background-color: var(--bg-card);
    border: 1px solid var(--border);
    border-radius: 12px;
    padding: 2rem;
    box-shadow: 0 4px 24px rgba(0, 0, 0, 0.5);
}

/* Account selector styles */
.account-selector-view h2,
.add-account-view h2,
.nsec-login-view h2 {
    font-size: 1.5rem;
    color: var(--text-primary);
    margin-bottom: 0.5rem;
}

.account-selector-view .subtitle,
.add-account-view .subtitle {
    color: var(--text-secondary);
    margin-bottom: 1.5rem;
    font-size: 0.9rem;
}

.back-btn {
    background: none;
    border: none;
    color: var(--accent);
    cursor: pointer;
    font-size: 0.9rem;
    margin-bottom: 1rem;
    padding: 0;
    font-family: inherit;
}

.back-btn:hover {
    color: var(--accent-hover);
}

/* Login options grid */
.login-options {
    display: flex;
    flex-direction: column;
    gap: 1.5rem;
}

.login-option {
    background-color: var(--bg-secondary);
    border: 1px solid var(--border);
    border-radius: 8px;
    padding: 1.5rem;
}

.login-option.primary {
    border-color: var(--accent);
    border-width: 2px;
}

.login-option h3 {
    font-size: 1.1rem;
    color: var(--text-primary);
    margin-bottom: 0.5rem;
}

.login-option .description {
    color: var(--text-secondary);
    font-size: 0.85rem;
    margin-bottom: 1rem;
}

/* Input group styling for NIP-46 section */
.login-option .input-group {
    margin-bottom: 1rem;
}

.login-option .input-group label {
    display: block;
    color: var(--text-secondary);
    font-size: 0.8rem;
    margin-bottom: 0.5rem;
    text-transform: uppercase;
    letter-spacing: 0.05em;
}

.login-option .input-group input {
    width: 100%;
    padding: 0.75rem;
    background-color: var(--bg-primary);
    border: 1px solid var(--border);
    border-radius: 6px;
    color: var(--text-primary);
    font-family: inherit;
    font-size: 0.9rem;
    margin-bottom: 0.5rem;
}

.login-option .input-group input:focus {
    outline: none;
    border-color: var(--accent);
}

.login-option button {
    width: 100%;
    padding: 0.75rem 1rem;
    background-color: var(--accent);
    border: none;
    border-radius: 6px;
    color: var(--bg-primary);
    font-family: inherit;
    font-size: 0.9rem;
    font-weight: 600;
    cursor: pointer;
    transition: background-color 0.2s;
    margin-bottom: 0.5rem;
}

.login-option button:hover:not(:disabled) {
    background-color: var(--accent-hover);
}

.login-option button:disabled {
    opacity: 0.6;
    cursor: not-allowed;
}

/* Nsec login form */
.nsec-login-view .form-group {
    margin-bottom: 1rem;
}

.nsec-login-view label {
    display: block;
    color: var(--text-secondary);
    font-size: 0.8rem;
    margin-bottom: 0.25rem;
    text-transform: uppercase;
    letter-spacing: 0.05em;
}

.nsec-login-view input {
    width: 100%;
    padding: 0.75rem;
    background-color: var(--bg-secondary);
    border: 1px solid var(--border);
    border-radius: 6px;
    color: var(--text-primary);
    font-family: inherit;
    font-size: 0.9rem;
}

.nsec-login-view input:focus {
    outline: none;
    border-color: var(--accent);
}

.security-notice {
    background-color: rgba(68, 255, 68, 0.1);
    border: 1px solid rgba(68, 255, 68, 0.3);
    border-radius: 8px;
    padding: 1rem;
    margin: 1rem 0;
}

.security-notice p {
    color: var(--success);
    font-weight: 600;
    margin-bottom: 0.5rem;
}

.security-notice ul {
    list-style: none;
    padding: 0;
    margin: 0;
}

.security-notice li {
    color: var(--text-secondary);
    font-size: 0.8rem;
    padding: 0.25rem 0;
}

/* Restore backup view */
.restore-backup-view {
    padding: 1rem 0;
}

.restore-backup-view h2 {
    font-size: 1.5rem;
    color: var(--text-primary);
    margin-bottom: 0.5rem;
}

.restore-backup-view .subtitle {
    color: var(--text-secondary);
    margin-bottom: 1.5rem;
    font-size: 0.9rem;
}

/* Generated URI display */
.generated-uri {
    margin-top: 1rem;
    padding: 1rem;
    background-color: var(--bg-primary);
    border: 1px solid var(--border);
    border-radius: 6px;
}

.generated-uri textarea {
    width: 100%;
    min-height: 80px;
    padding: 0.75rem;
    background-color: var(--bg-secondary);
    border: 1px solid var(--border);
    border-radius: 6px;
    color: var(--text-primary);
    font-family: monospace;
    font-size: 0.8rem;
    resize: vertical;
    margin-bottom: 0.75rem;
}

.generated-uri button {
    width: 100%;
    padding: 0.75rem;
    background-color: var(--accent);
    border: none;
    border-radius: 6px;
    color: var(--bg-primary);
    font-family: inherit;
    font-size: 0.9rem;
    font-weight: 600;
    cursor: pointer;
}

.generated-uri button:hover {
    background-color: var(--accent-hover);
}

/* QR Login View Styles */
.qr-login-view {
    padding: 1rem 0;
    max-width: 500px;
    margin: 0 auto;
}

.qr-login-view h2 {
    font-size: 1.5rem;
    color: var(--text-primary);
    margin-bottom: 0.5rem;
    text-align: center;
}

.qr-login-view .subtitle {
    color: var(--text-secondary);
    margin-bottom: 1.5rem;
    font-size: 0.9rem;
    text-align: center;
}

.qr-container {
    background-color: var(--bg-secondary);
    border: 2px solid var(--border);
    border-radius: 12px;
    padding: 1.5rem;
    margin-bottom: 1.5rem;
}

.qr-code-display {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 1rem;
}

.qr-placeholder {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 1rem;
    width: 100%;
}

.qr-visual {
    font-size: 4rem;
    margin-bottom: 0.5rem;
}

.qr-uri-text {
    width: 100%;
    min-height: 80px;
    padding: 0.75rem;
    background-color: var(--bg-primary);
    border: 1px solid var(--border);
    border-radius: 8px;
    color: var(--text-primary);
    font-family: monospace;
    font-size: 0.75rem;
    line-height: 1.4;
    resize: none;
    word-break: break-all;
}

.qr-image {
    width: 100%;
    max-width: 280px;
    margin: 0 auto;
    background-color: white;
    padding: 1rem;
    border-radius: 8px;
    box-shadow: 0 2px 8px rgba(0, 0, 0, 0.3);
}

.qr-image svg {
    width: 100%;
    height: auto;
    display: block;
}

.qr-uri-details {
    width: 100%;
    margin-top: 1rem;
}

.qr-uri-details summary {
    color: var(--text-secondary);
    font-size: 0.85rem;
    cursor: pointer;
    padding: 0.5rem;
    text-align: center;
}

.qr-uri-details summary:hover {
    color: var(--accent);
}

.qr-uri-details[open] summary {
    margin-bottom: 0.5rem;
}

.copy-btn {
    padding: 0.75rem 1.5rem;
    background-color: var(--accent);
    border: none;
    border-radius: 8px;
    color: var(--bg-primary);
    font-family: inherit;
    font-size: 0.9rem;
    font-weight: 600;
    cursor: pointer;
    transition: background-color 0.2s;
    margin-top: 0.5rem;
}

.copy-btn:hover {
    background-color: var(--accent-hover);
}

.qr-status {
    text-align: center;
    padding: 1rem;
    margin-top: 1rem;
}

.qr-status.polling {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 0.5rem;
}

.qr-status .spinner {
    width: 40px;
    height: 40px;
    border: 3px solid var(--border);
    border-top-color: var(--accent);
    border-radius: 50%;
    animation: spin 1s linear infinite;
}

@keyframes spin {
    to { transform: rotate(360deg); }
}

.qr-status p {
    color: var(--text-primary);
    font-weight: 500;
    margin: 0;
}

.qr-status .hint {
    color: var(--text-secondary);
    font-size: 0.8rem;
}

.qr-status.error {
    background-color: rgba(255, 68, 68, 0.1);
    border: 1px solid rgba(255, 68, 68, 0.3);
    border-radius: 8px;
}

.qr-status .error-text {
    color: var(--error);
    font-size: 0.9rem;
}

.qr-instructions {
    background-color: var(--bg-secondary);
    border: 1px solid var(--border);
    border-radius: 8px;
    padding: 1.25rem;
}

.qr-instructions h4 {
    color: var(--text-primary);
    margin-bottom: 0.75rem;
    font-size: 1rem;
}

.qr-instructions ol {
    margin: 0;
    padding-left: 1.25rem;
    color: var(--text-secondary);
    font-size: 0.85rem;
    line-height: 1.6;
}

.qr-instructions li {
    margin-bottom: 0.5rem;
}

/* QR option in login options */
.login-option.qr-option {
    border: 2px dashed var(--accent);
    background-color: rgba(255, 136, 0, 0.05);
}

.login-option.qr-option:hover {
    background-color: rgba(255, 136, 0, 0.1);
}

.login-btn.primary {
    width: 100%;
    padding: 1rem;
    background-color: var(--accent);
    border: none;
    border-radius: 8px;
    color: var(--bg-primary);
    font-family: inherit;
    font-size: 1rem;
    font-weight: 600;
    cursor: pointer;
    margin-top: 1rem;
}

/* Account selector component styles */
.account-selector h2 {
    font-size: 1.5rem;
    color: var(--text-primary);
    margin-bottom: 0.5rem;
}

.account-selector .subtitle {
    color: var(--text-secondary);
    margin-bottom: 1.5rem;
    font-size: 0.9rem;
}

.accounts-list {
    display: flex;
    flex-direction: column;
    gap: 0.75rem;
    margin-bottom: 1.5rem;
}

.account-card {
    display: flex;
    align-items: center;
    gap: 1rem;
    background-color: var(--bg-secondary);
    border: 1px solid var(--border);
    border-radius: 8px;
    padding: 1rem;
    transition: border-color 0.2s;
}

.account-card.active {
    border-color: var(--accent);
    border-width: 2px;
}

.account-avatar {
    flex-shrink: 0;
    width: 40px;
    height: 40px;
}

.account-avatar-img {
    width: 40px;
    height: 40px;
    border-radius: 50%;
    object-fit: cover;
}

.avatar-placeholder {
    width: 40px;
    height: 40px;
    background-color: var(--accent);
    border-radius: 50%;
    display: flex;
    align-items: center;
    justify-content: center;
    font-weight: 600;
    color: var(--bg-primary);
    font-size: 1.2rem;
}

.account-info {
    flex: 1;
    display: flex;
    flex-direction: column;
    gap: 0.25rem;
}

.account-name {
    font-weight: 600;
    color: var(--text-primary);
}

.account-npub {
    font-size: 0.8rem;
    color: var(--text-muted);
    font-family: monospace;
}

.account-mode {
    font-size: 0.75rem;
    color: var(--text-secondary);
    text-transform: uppercase;
}

.account-actions {
    display: flex;
    align-items: center;
    gap: 0.5rem;
}

.switch-btn {
    padding: 0.5rem 1rem;
    background-color: var(--accent);
    border: none;
    border-radius: 6px;
    color: var(--bg-primary);
    font-family: inherit;
    font-size: 0.8rem;
    font-weight: 600;
    cursor: pointer;
}

.delete-btn {
    padding: 0.5rem 0.75rem;
    background-color: transparent;
    border: 1px solid var(--error);
    border-radius: 6px;
    color: var(--error);
    font-family: inherit;
    font-size: 1rem;
    cursor: pointer;
}

.current-badge {
    font-size: 0.8rem;
    color: var(--success);
    font-weight: 600;
}

.add-account-btn {
    width: 100%;
    padding: 0.75rem;
    background-color: transparent;
    border: 2px dashed var(--border);
    border-radius: 8px;
    color: var(--text-secondary);
    font-family: inherit;
    font-size: 0.9rem;
    cursor: pointer;
    transition: all 0.2s;
}

.add-account-btn:hover {
    border-color: var(--accent);
    color: var(--accent);
}

/* Backup manager styles */
.backup-manager {
    margin-top: 1rem;
}

.backup-manager h3 {
    font-size: 1.1rem;
    color: var(--text-primary);
    margin-bottom: 1rem;
}

.backup-section,
.restore-section {
    margin-bottom: 1.5rem;
}

.backup-section h4,
.restore-section h4 {
    font-size: 0.9rem;
    color: var(--text-secondary);
    margin-bottom: 0.5rem;
}

.backup-section .info,
.restore-section .info {
    font-size: 0.8rem;
    color: var(--text-muted);
    margin-bottom: 1rem;
}

.backup-output,
.restore-input {
    margin-top: 1rem;
}

.backup-output label {
    display: block;
    font-size: 0.8rem;
    color: var(--text-secondary);
    margin-bottom: 0.5rem;
}

.backup-string,
.restore-input textarea {
    width: 100%;
    padding: 0.75rem;
    background-color: var(--bg-secondary);
    border: 1px solid var(--border);
    border-radius: 6px;
    color: var(--text-primary);
    font-family: monospace;
    font-size: 0.8rem;
    resize: vertical;
}

.backup-output .warning {
    font-size: 0.75rem;
    color: var(--error);
    margin-top: 0.5rem;
}

.status-message {
    margin-top: 1rem;
    padding: 0.75rem;
    background-color: rgba(245, 130, 31, 0.1);
    border: 1px solid rgba(245, 130, 31, 0.3);
    border-radius: 6px;
    color: var(--accent);
    font-size: 0.85rem;
}

/* Legacy login styles (keep for compatibility) */
.login-card {
    background-color: var(--bg-card);
    border: 1px solid var(--border);
    border-radius: 12px;
    padding: 3rem;
    width: 100%;
    max-width: 480px;
    box-shadow: 0 4px 24px rgba(0, 0, 0, 0.5);
}

.login-title {
    font-size: 2.5rem;
    font-weight: 700;
    color: var(--accent);
    text-align: center;
    margin-bottom: 0.5rem;
    letter-spacing: -0.02em;
}

.login-tagline {
    color: var(--text-secondary);
    text-align: center;
    margin-bottom: 2.5rem;
    font-size: 0.95rem;
}

.input-group {
    margin-bottom: 1.25rem;
}

.input-label {
    display: block;
    color: var(--text-secondary);
    font-size: 0.875rem;
    margin-bottom: 0.5rem;
    text-transform: uppercase;
    letter-spacing: 0.05em;
}

.input-field {
    width: 100%;
    padding: 0.875rem 1rem;
    background-color: var(--bg-secondary);
    border: 1px solid var(--border);
    border-radius: 8px;
    color: var(--text-primary);
    font-family: inherit;
    font-size: 0.95rem;
    transition: border-color 0.2s, box-shadow 0.2s;
}

.input-field:focus {
    outline: none;
    border-color: var(--accent);
    box-shadow: 0 0 0 2px rgba(245, 130, 31, 0.2);
}

.input-field::placeholder {
    color: var(--text-muted);
}

.connect-button {
    width: 100%;
    padding: 1rem 1.5rem;
    background-color: var(--accent);
    border: none;
    border-radius: 8px;
    color: var(--bg-primary);
    font-family: inherit;
    font-size: 1rem;
    font-weight: 600;
    cursor: pointer;
    transition: background-color 0.2s, transform 0.1s;
    margin-top: 0.5rem;
}

.connect-button:hover:not(:disabled) {
    background-color: var(--accent-hover);
}

.connect-button:active:not(:disabled) {
    transform: translateY(1px);
}

.connect-button:disabled {
    opacity: 0.6;
    cursor: not-allowed;
}

.loading-indicator {
    text-align: center;
    color: var(--accent);
    margin-top: 1rem;
    font-size: 0.9rem;
}

.error-message {
    background-color: rgba(255, 68, 68, 0.1);
    border: 1px solid rgba(255, 68, 68, 0.3);
    border-radius: 8px;
    padding: 0.875rem 1rem;
    margin-top: 1rem;
    color: var(--error);
    font-size: 0.9rem;
}

.success-message {
    background-color: rgba(68, 255, 68, 0.1);
    border: 1px solid rgba(68, 255, 68, 0.3);
    border-radius: 8px;
    padding: 0.875rem 1rem;
    margin-top: 1rem;
    color: var(--success);
    font-size: 0.9rem;
}

.login-footer {
    text-align: center;
    margin-top: 1.5rem;
    color: var(--text-muted);
    font-size: 0.8rem;
}

/* NIP-07 section styles */
.nip46-section {
    margin-bottom: 1.5rem;
}

.nip07-section {
    margin-top: 1.5rem;
    padding-top: 1.5rem;
    border-top: 1px solid var(--border);
}

.divider {
    text-align: center;
    color: var(--text-muted);
    margin-bottom: 1rem;
    font-size: 0.9rem;
}

.saved-users-section {
    margin-bottom: 1rem;
}

.saved-users-empty {
    padding: 1rem;
    text-align: center;
    color: var(--text-muted);
    background: var(--bg-secondary);
    border-radius: 8px;
    margin-bottom: 1rem;
}

.saved-user-card {
    display: flex;
    justify-content: space-between;
    align-items: center;
    padding: 0.75rem;
    background: var(--bg-secondary);
    border: 1px solid var(--border);
    border-radius: 8px;
    margin-bottom: 0.5rem;
}

.saved-user-info {
    display: flex;
    flex-direction: column;
    gap: 0.25rem;
    flex: 1;
    min-width: 0;
}

.saved-user-avatar-row {
    display: flex;
    align-items: center;
    gap: 0.75rem;
}

.saved-user-avatar {
    width: 40px;
    height: 40px;
    border-radius: 50%;
    object-fit: cover;
    border: 2px solid var(--accent);
    flex-shrink: 0;
}

.saved-user-avatar-placeholder {
    width: 40px;
    height: 40px;
    border-radius: 50%;
    background: var(--accent);
    color: var(--bg-primary);
    font-weight: bold;
    font-size: 1.2rem;
    display: flex;
    align-items: center;
    justify-content: center;
    flex-shrink: 0;
}

.saved-user-details {
    display: flex;
    flex-direction: column;
    gap: 0.2rem;
    min-width: 0;
    overflow: hidden;
}

.saved-user-name {
    font-weight: 600;
    color: var(--text-primary);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
}

.saved-user-npub {
    font-size: 0.8rem;
    color: var(--text-muted);
    font-family: monospace;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
}

.saved-user-method {
    font-size: 0.7rem;
    color: var(--accent);
    text-transform: uppercase;
}

.saved-user-actions {
    display: flex;
    gap: 0.5rem;
    align-items: center;
}

.saved-user-connect {
    padding: 0.4rem 0.8rem;
    background: var(--accent);
    color: var(--bg-primary);
    border: none;
    border-radius: 4px;
    font-size: 0.8rem;
    font-weight: 600;
    cursor: pointer;
}

.saved-user-connect:hover {
    background: var(--accent-hover);
}

.saved-user-connect:disabled {
    opacity: 0.5;
    cursor: not-allowed;
}

.saved-user-delete {
    padding: 0.4rem 0.6rem;
    background: transparent;
    color: var(--error);
    border: 1px solid var(--error);
    border-radius: 4px;
    font-size: 1rem;
    cursor: pointer;
}

.saved-user-delete:hover {
    background: var(--error);
    color: white;
}

/* Screen navigation buttons */
.screen-title {
    font-size: 1.5rem;
    font-weight: 700;
    color: var(--text-primary);
    margin-bottom: 1.5rem;
    text-align: center;
}

.back-button {
    background: transparent;
    border: 1px solid var(--border);
    border-radius: 6px;
    color: var(--text-secondary);
    padding: 0.5rem 1rem;
    font-size: 0.9rem;
    cursor: pointer;
    margin-bottom: 1rem;
    transition: all 0.2s;
}

.back-button:hover {
    background: var(--bg-secondary);
    color: var(--text-primary);
}

.add-new-button {
    width: 100%;
    padding: 1rem;
    background: var(--accent);
    color: white;
    border: none;
    border-radius: 8px;
    font-size: 1rem;
    font-weight: 600;
    cursor: pointer;
    margin-top: 1.5rem;
    transition: background 0.2s;
}

.add-new-button:hover {
    background: var(--accent-hover);
}

.nip07-button {
    width: 100%;
    padding: 1rem 1.5rem;
    background-color: var(--bg-secondary);
    border: 1px solid var(--accent);
    border-radius: 8px;
    color: var(--accent);
    font-family: inherit;
    font-size: 1rem;
    font-weight: 600;
    cursor: pointer;
    transition: background-color 0.2s, color 0.2s;
}

.nip07-button:hover:not(:disabled) {
    background-color: var(--accent);
    color: var(--bg-primary);
}

.nip07-button:disabled {
    opacity: 0.6;
    cursor: not-allowed;
}

.nip07-hint {
    text-align: center;
    margin-top: 0.5rem;
    color: var(--text-muted);
    font-size: 0.8rem;
}

/* Main view styles */
.main-view {
    flex: 1;
    display: flex;
    flex-direction: column;
}

.header {
    display: flex;
    justify-content: space-between;
    align-items: center;
    padding: 1rem 1.5rem;
    border-bottom: 1px solid var(--border);
    background-color: var(--bg-secondary);
}

.header-title {
    font-size: 1.5rem;
    color: var(--accent);
    font-weight: 600;
}

.user-info {
    display: flex;
    align-items: center;
    gap: 1rem;
}

.npub-display {
    color: var(--text-secondary);
    font-size: 0.875rem;
    font-family: monospace;
}

.avatar {
    width: 28px;
    height: 28px;
    border-radius: 50%;
    object-fit: cover;
    border: 1px solid var(--accent);
    vertical-align: middle;
    margin-right: 6px;
}

.avatar-placeholder {
    width: 28px;
    height: 28px;
    border-radius: 50%;
    background: var(--accent);
    color: #000;
    font-weight: bold;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    font-size: 13px;
    margin-right: 6px;
}

.verified-badge {
    color: var(--accent);
    font-size: 12px;
    margin-left: 3px;
}

.user-display-name {
    color: var(--text-primary);
    font-size: 0.875rem;
    font-weight: 500;
}

.user-profile-btn {
    display: flex;
    align-items: center;
    gap: 6px;
    background: transparent;
    border: none;
    cursor: pointer;
    padding: 4px 8px;
    border-radius: 6px;
    transition: background-color 0.2s;
}

.user-profile-btn:hover {
    background: rgba(255, 255, 255, 0.05);
}

.disconnect-button {
    padding: 0.5rem 1rem;
    background-color: transparent;
    border: 1px solid var(--border);
    border-radius: 6px;
    color: var(--text-secondary);
    font-family: inherit;
    font-size: 0.875rem;
    cursor: pointer;
    transition: border-color 0.2s, color 0.2s;
}

.disconnect-button:hover {
    border-color: var(--error);
    color: var(--error);
}

/* Relay count badge styles */
.relay-count-badge {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 6px 12px;
    background: var(--bg-card);
    border: 1px solid var(--border);
    border-radius: 20px;
    color: var(--text-secondary);
    font-size: 0.875rem;
    font-weight: 500;
    transition: all 0.2s ease;
}

.relay-count-badge:hover {
    border-color: var(--accent);
    color: var(--accent);
}

.relay-count-badge.connected {
    border-color: var(--success);
    color: var(--success);
}

.relay-count-badge.connecting {
    border-color: var(--accent);
    color: var(--accent);
    animation: pulse 2s infinite;
}

.relay-count-badge.disconnected {
    border-color: var(--error);
    color: var(--error);
}

/* Connection status indicator styles */
.connection-status {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 4px 10px;
    border-radius: 12px;
    font-size: 12px;
    font-weight: 500;
    background: var(--bg-card);
    border: 1px solid var(--border);
    margin-right: 8px;
    transition: all 0.2s ease;
}

.connection-status.connecting {
    border-color: #f5a623;
    color: #f5a623;
    animation: pulse 2s infinite;
}

.connection-status.connected {
    border-color: var(--success);
    color: var(--success);
}

.connection-status.failed {
    border-color: var(--error);
    color: var(--error);
}

.connection-status.unknown {
    border-color: var(--text-muted);
    color: var(--text-muted);
}

.connection-icon {
    font-size: 10px;
}

.connection-text {
    white-space: nowrap;
}

@keyframes pulse {
    0%, 100% { opacity: 1; }
    50% { opacity: 0.6; }
}

.relay-icon {
    width: 16px;
    height: 16px;
    flex-shrink: 0;
}

.relay-count-number {
    font-weight: 600;
    min-width: 20px;
    text-align: center;
}

/* Relay dropdown styles */
.relay-badge-container {
    position: relative;
    display: inline-block;
}

.relay-dropdown {
    position: absolute;
    top: 100%;
    right: 0;
    margin-top: 8px;
    background: var(--bg-card);
    border: 1px solid var(--border);
    border-radius: 8px;
    min-width: 280px;
    max-width: 350px;
    box-shadow: 0 4px 20px rgba(0, 0, 0, 0.5);
    z-index: 1000;
    opacity: 0;
    visibility: hidden;
    transform: translateY(-10px);
    transition: all 0.2s ease;
}

.relay-dropdown.open {
    opacity: 1;
    visibility: visible;
    transform: translateY(0);
}

.relay-dropdown-header {
    display: flex;
    justify-content: space-between;
    align-items: center;
    padding: 12px 16px;
    border-bottom: 1px solid var(--border);
    background: var(--bg-secondary);
    border-radius: 8px 8px 0 0;
}

.relay-dropdown-title {
    font-weight: 600;
    color: var(--text-primary);
    font-size: 0.9rem;
}

.relay-dropdown-close {
    background: none;
    border: none;
    color: var(--text-secondary);
    font-size: 1rem;
    cursor: pointer;
    padding: 4px 8px;
    border-radius: 4px;
    transition: all 0.2s;
}

.relay-dropdown-close:hover {
    background: rgba(255, 255, 255, 0.1);
    color: var(--text-primary);
}

.relay-dropdown-list {
    max-height: 300px;
    overflow-y: auto;
    padding: 8px 0;
}

.relay-dropdown-list ul {
    list-style: none;
    margin: 0;
    padding: 0;
}

.relay-dropdown-item {
    padding: 8px 16px;
    border-bottom: 1px solid var(--border);
    transition: background 0.2s;
}

.relay-dropdown-item:last-child {
    border-bottom: none;
}

.relay-dropdown-item:hover {
    background: rgba(255, 255, 255, 0.05);
}

.relay-url {
    font-family: 'SF Mono', 'Monaco', 'Inconsolata', monospace;
    font-size: 0.8rem;
    color: var(--text-secondary);
    word-break: break-all;
}

.relay-dropdown-empty {
    padding: 20px 16px;
    text-align: center;
    color: var(--text-muted);
    font-size: 0.9rem;
}

/* Make the badge button styled properly */
button.relay-count-badge {
    cursor: pointer;
    background: var(--bg-card);
    font-family: inherit;
}

button.relay-count-badge:hover {
    background: rgba(255, 255, 255, 0.05);
}

/* Marketplace layout */
.marketplace-container {
    flex: 1;
    display: flex;
    overflow: hidden;
}

.sidebar {
    width: 200px;
    background-color: var(--bg-secondary);
    border-right: 1px solid var(--border);
    padding: 1.5rem;
    display: flex;
    flex-direction: column;
    gap: 0.75rem;
}

.sidebar-button {
    padding: 0.75rem 1rem;
    background-color: transparent;
    border: 1px solid var(--border);
    border-radius: 6px;
    color: var(--text-secondary);
    font-family: inherit;
    font-size: 0.9rem;
    cursor: pointer;
    transition: all 0.2s;
    text-align: left;
}

.sidebar-button:hover {
    border-color: var(--accent);
    color: var(--accent);
}

.sidebar-button.active {
    background-color: var(--accent);
    border-color: var(--accent);
    color: var(--bg-primary);
}

.content-area {
    flex: 1;
    padding: 1.5rem;
    overflow-y: auto;
}

/* Browse view styles */
.browse-container {
    max-width: 1200px;
}

.browse-title {
    font-size: 1.75rem;
    color: var(--text-primary);
    margin-bottom: 1.5rem;
}

.loading-state, .error-state, .empty-state {
    text-align: center;
    padding: 3rem;
    color: var(--text-secondary);
}

.listings-grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(300px, 1fr));
    gap: 1.5rem;
}

.listing-card {
    background-color: var(--bg-card);
    border: 1px solid var(--border);
    border-radius: 12px;
    padding: 1.5rem;
    display: flex;
    flex-direction: column;
    gap: 0.75rem;
    transition: border-color 0.2s, transform 0.2s;
}

.listing-card:hover {
    border-color: var(--accent);
    transform: translateY(-2px);
}

.listing-header {
    margin-bottom: 0.5rem;
}

.listing-title {
    font-size: 1.25rem;
    font-weight: 600;
    color: var(--text-primary);
    margin-bottom: 0.25rem;
}

.listing-publisher {
    font-size: 0.8rem;
    color: var(--text-muted);
}

.listing-price {
    margin: 0.5rem 0;
}

.price-free {
    color: #44ff44;
    font-weight: 600;
}

.price-paid {
    color: var(--accent);
    font-weight: 600;
}

.listing-tags {
    display: flex;
    flex-wrap: wrap;
    gap: 0.5rem;
    margin-bottom: 0.5rem;
}

.tag-badge {
    background-color: var(--bg-secondary);
    color: var(--text-secondary);
    padding: 0.25rem 0.5rem;
    border-radius: 4px;
    font-size: 0.75rem;
}

.view-button {
    margin-top: auto;
    padding: 0.75rem;
    background-color: var(--accent);
    border: none;
    border-radius: 6px;
    color: var(--bg-primary);
    font-family: inherit;
    font-size: 0.9rem;
    font-weight: 600;
    cursor: pointer;
    transition: background-color 0.2s;
}

.view-button:hover {
    background-color: var(--accent-hover);
}

/* Detail view styles */
.detail-container {
    max-width: 800px;
}

.back-button {
    padding: 0.5rem 1rem;
    background-color: transparent;
    border: 1px solid var(--border);
    border-radius: 6px;
    color: var(--text-secondary);
    font-family: inherit;
    font-size: 0.875rem;
    cursor: pointer;
    margin-bottom: 1.5rem;
    transition: all 0.2s;
}

.back-button:hover {
    border-color: var(--accent);
    color: var(--accent);
}

.detail-content {
    background-color: var(--bg-card);
    border: 1px solid var(--border);
    border-radius: 12px;
    padding: 2rem;
}

.detail-title {
    font-size: 2rem;
    color: var(--text-primary);
    margin-bottom: 1rem;
}

.detail-meta {
    margin-bottom: 1.5rem;
    padding-bottom: 1rem;
    border-bottom: 1px solid var(--border);
}

.detail-meta p {
    margin-bottom: 0.5rem;
}

.meta-label {
    color: var(--text-muted);
}

.detail-description {
    margin-bottom: 1.5rem;
}

.detail-description h3 {
    font-size: 1.1rem;
    color: var(--text-secondary);
    margin-bottom: 0.75rem;
}

.detail-description p {
    color: var(--text-primary);
    line-height: 1.7;
}

.detail-tags {
    margin-bottom: 1.5rem;
}

.detail-tags h3 {
    font-size: 1.1rem;
    color: var(--text-secondary);
    margin-bottom: 0.75rem;
}

.tags-container {
    display: flex;
    flex-wrap: wrap;
    gap: 0.5rem;
}

.no-tags {
    color: var(--text-muted);
    font-style: italic;
}

/* Seller profile card styles */
.seller-card-loading {
    padding: 1rem;
    color: var(--text-muted);
    font-size: 0.9rem;
    margin-bottom: 20px;
}

.seller-card {
    background: #1a1a1a;
    border: 1px solid #2a2a2a;
    border-radius: 8px;
    padding: 14px 16px;
    margin-bottom: 20px;
    display: flex;
    align-items: flex-start;
    gap: 12px;
}

.seller-card-fallback {
    padding: 1rem;
    color: var(--text-secondary);
    font-size: 0.9rem;
    margin-bottom: 20px;
    background: #1a1a1a;
    border: 1px solid #2a2a2a;
    border-radius: 8px;
}

.seller-avatar {
    width: 48px;
    height: 48px;
    border-radius: 50%;
    object-fit: cover;
    border: 1px solid var(--accent);
    flex-shrink: 0;
}

.seller-avatar-placeholder {
    width: 48px;
    height: 48px;
    border-radius: 50%;
    background: var(--accent);
    color: #000;
    font-weight: bold;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    font-size: 18px;
    flex-shrink: 0;
}

.seller-info {
    flex: 1;
    min-width: 0;
}

.seller-name {
    font-weight: bold;
    color: #fff;
    font-size: 15px;
}

.seller-verified {
    color: var(--accent);
    font-size: 12px;
}

.seller-nip05 {
    color: #888;
    font-size: 12px;
    margin-top: 2px;
}

.seller-about {
    color: #aaa;
    font-size: 13px;
    margin-top: 6px;
    line-height: 1.4;
}

.seller-website {
    color: var(--accent);
    font-size: 12px;
    margin-top: 4px;
    text-decoration: none;
}

.seller-website:hover {
    text-decoration: underline;
}

.detail-actions {
    margin-top: 1.5rem;
    padding-top: 1.5rem;
    border-top: 1px solid var(--border);
}

.download-button {
    padding: 1rem 2rem;
    background-color: var(--accent);
    border: none;
    border-radius: 8px;
    color: var(--bg-primary);
    font-family: inherit;
    font-size: 1rem;
    font-weight: 600;
    cursor: pointer;
    transition: background-color 0.2s;
}

.download-button:hover:not(:disabled) {
    background-color: var(--accent-hover);
}

.download-button:disabled {
    opacity: 0.5;
    cursor: not-allowed;
}

/* Buy section styles */
.buy-section {
    margin-top: 1.5rem;
    padding-top: 1.5rem;
    border-top: 1px solid var(--border);
}

.buy-section h3 {
    font-size: 1.1rem;
    color: var(--text-secondary);
    margin-bottom: 1rem;
}

.buy-button {
    width: 100%;
    padding: 1rem;
    background-color: var(--accent);
    border: none;
    border-radius: 8px;
    color: var(--bg-primary);
    font-family: inherit;
    font-size: 1rem;
    font-weight: 600;
    cursor: pointer;
    transition: background-color 0.2s;
}

.buy-button:hover:not(:disabled) {
    background-color: var(--accent-hover);
}

.buy-button:disabled {
    opacity: 0.6;
    cursor: not-allowed;
}

.invoice-card {
    background-color: var(--bg-secondary);
    border: 1px solid var(--border);
    border-radius: 8px;
    padding: 1.5rem;
    margin-top: 1rem;
}

.invoice-card h4 {
    color: var(--accent);
    margin-bottom: 1rem;
}

.invoice-text {
    font-family: monospace;
    font-size: 0.875rem;
    color: var(--text-secondary);
    background-color: var(--bg-primary);
    padding: 0.75rem;
    border-radius: 4px;
    margin-bottom: 1rem;
    word-break: break-all;
}

.invoice-actions {
    display: flex;
    gap: 0.75rem;
    margin-bottom: 1rem;
}

.copy-button, .wallet-button {
    flex: 1;
    padding: 0.75rem;
    border: none;
    border-radius: 6px;
    font-family: inherit;
    font-size: 0.9rem;
    font-weight: 600;
    cursor: pointer;
    transition: background-color 0.2s;
}

.copy-button {
    background-color: var(--bg-card);
    color: var(--text-primary);
    border: 1px solid var(--border);
}

.copy-button:hover {
    border-color: var(--accent);
}

.wallet-button {
    background-color: var(--accent);
    color: var(--bg-primary);
}

.wallet-button:hover {
    background-color: var(--accent-hover);
}

.invoice-hint {
    font-size: 0.875rem;
    color: var(--text-muted);
    text-align: center;
}

.free-note {
    margin-top: 1.5rem;
    padding: 1rem;
    background-color: rgba(68, 255, 68, 0.1);
    border: 1px solid rgba(68, 255, 68, 0.3);
    border-radius: 8px;
    text-align: center;
    color: #44ff44;
}

/* Publish view styles */
.publish-container {
    max-width: 600px;
}

.publish-title {
    font-size: 1.75rem;
    color: var(--text-primary);
    margin-bottom: 1.5rem;
}

.publish-form {
    background-color: var(--bg-card);
    border: 1px solid var(--border);
    border-radius: 12px;
    padding: 2rem;
}

.form-group {
    margin-bottom: 1.25rem;
}

.form-label {
    display: block;
    color: var(--text-secondary);
    font-size: 0.875rem;
    margin-bottom: 0.5rem;
}

.form-input, .form-textarea {
    width: 100%;
    padding: 0.75rem 1rem;
    background-color: var(--bg-secondary);
    border: 1px solid var(--border);
    border-radius: 6px;
    color: var(--text-primary);
    font-family: inherit;
    font-size: 0.9rem;
    transition: border-color 0.2s;
}

.form-input:focus, .form-textarea:focus {
    outline: none;
    border-color: var(--accent);
}

.form-textarea {
    resize: vertical;
    min-height: 100px;
}

.publish-button {
    width: 100%;
    padding: 1rem;
    background-color: var(--accent);
    border: none;
    border-radius: 8px;
    color: var(--bg-primary);
    font-family: inherit;
    font-size: 1rem;
    font-weight: 600;
    cursor: pointer;
    transition: background-color 0.2s;
    margin-top: 1rem;
}

.publish-button:hover:not(:disabled) {
    background-color: var(--accent-hover);
}

.publish-button:disabled {
    opacity: 0.6;
    cursor: not-allowed;
}

/* NostrConnect section styles */
.nostrconnect-section,
.bunker-section {
    margin-bottom: 1.5rem;
}

.section-title {
    font-size: 1rem;
    font-weight: 600;
    color: var(--text-primary);
    margin-bottom: 0.5rem;
}

.section-description {
    font-size: 0.875rem;
    color: var(--text-secondary);
    margin-bottom: 1rem;
}

.generate-button {
    width: 100%;
    padding: 1rem 1.5rem;
    background-color: var(--bg-secondary);
    border: 2px solid var(--accent);
    border-radius: 8px;
    color: var(--accent);
    font-family: inherit;
    font-size: 1rem;
    font-weight: 600;
    cursor: pointer;
    transition: background-color 0.2s, color 0.2s;
    margin-bottom: 1rem;
}

.generate-button:hover:not(:disabled) {
    background-color: var(--accent);
    color: var(--bg-primary);
}

.generate-button:disabled {
    opacity: 0.6;
    cursor: not-allowed;
}

.generated-uri-box {
    background-color: var(--bg-secondary);
    border: 1px solid var(--border);
    border-radius: 8px;
    padding: 1rem;
    margin-top: 1rem;
}

.uri-textarea {
    width: 100%;
    min-height: 80px;
    padding: 0.75rem;
    background-color: var(--bg-primary);
    border: 1px solid var(--border);
    border-radius: 6px;
    color: var(--text-primary);
    font-family: monospace;
    font-size: 0.8rem;
    resize: vertical;
    margin-bottom: 0.75rem;
}

.copy-button {
    width: 100%;
    padding: 0.75rem;
    background-color: var(--accent);
    border: none;
    border-radius: 6px;
    color: var(--bg-primary);
    font-family: inherit;
    font-size: 0.9rem;
    font-weight: 600;
    cursor: pointer;
    transition: background-color 0.2s;
    margin-bottom: 0.75rem;
}

.copy-button:hover {
    background-color: var(--accent-hover);
}

.instruction-text {
    font-size: 0.8rem;
    color: var(--text-muted);
    text-align: center;
}

/* Profile page styles */
.profile-page {
    padding: 32px;
    max-width: 700px;
}
.profile-header {
    display: flex;
    gap: 24px;
    align-items: flex-start;
    margin-bottom: 32px;
}
.profile-avatar-lg {
    width: 80px;
    height: 80px;
    border-radius: 50%;
    object-fit: cover;
    border: 2px solid var(--accent);
    flex-shrink: 0;
}
.profile-avatar-placeholder-lg {
    width: 80px;
    height: 80px;
    border-radius: 50%;
    background: var(--accent);
    color: #000;
    font-weight: bold;
    display: flex;
    align-items: center;
    justify-content: center;
    font-size: 32px;
    flex-shrink: 0;
}
.profile-display-name {
    font-size: 22px;
    font-weight: bold;
    color: #fff;
    margin: 0 0 4px 0;
}
.profile-username {
    color: #888;
    font-size: 14px;
    margin: 0 0 8px 0;
}
.profile-nip05 {
    font-size: 13px;
    color: #888;
    margin-bottom: 10px;
}
.profile-nip05 .verified { color: var(--accent); }
.profile-about {
    color: #aaa;
    font-size: 14px;
    line-height: 1.6;
    margin-bottom: 12px;
    white-space: pre-wrap;
}
.profile-link {
    color: var(--accent);
    text-decoration: none;
    font-size: 14px;
    display: block;
    margin-bottom: 6px;
}
.profile-npub-row {
    display: flex;
    align-items: center;
    gap: 10px;
    margin-top: 14px;
}
.profile-npub {
    font-family: monospace;
    font-size: 12px;
    color: #666;
    word-break: break-all;
}
.copy-btn {
    background: #1e1e1e;
    border: 1px solid #333;
    color: #ccc;
    padding: 3px 10px;
    border-radius: 4px;
    cursor: pointer;
    font-size: 12px;
    white-space: nowrap;
}
.copy-btn:hover { border-color: var(--accent); color: var(--accent); }
.profile-divider {
    border: none;
    border-top: 1px solid #1e1e1e;
    margin: 28px 0;
}
.my-listings-header {
    display: flex;
    align-items: baseline;
    gap: 8px;
    margin-bottom: 20px;
}
.my-listings-title {
    font-size: 18px;
    font-weight: bold;
    color: #fff;
}
.my-listings-count { color: #666; font-size: 14px; }
.empty-listings {
    color: #666;
    text-align: center;
    padding: 40px 0;
}
.empty-listings-btn {
    margin-top: 14px;
    background: transparent;
    border: 1px solid var(--accent);
    color: var(--accent);
    padding: 8px 18px;
    border-radius: 4px;
    cursor: pointer;
    font-family: monospace;
}
"#;

// =============================================================================
// Components
// =============================================================================

/// Login view modes
#[derive(Debug, Clone, PartialEq)]
enum LoginViewMode {
    AccountSelector, // Primary: List of stored accounts
    AddAccount,      // Secondary: Login methods (nsec, NIP-46, QR, etc.)
    NsecLogin,       // Tertiary: Direct nsec input
    QrLogin,         // Tertiary: QR code for mobile signer
    RestoreBackup,   // Tertiary: Restore from backup
}

/// Redesigned LoginView with secure storage as primary
#[component]
fn LoginView() -> impl IntoView {
    let auth = use_context::<AuthContext>().expect("AuthContext not provided");
    let auth_stored = StoredValue::new(auth.clone());

    // View mode state
    let view_mode = RwSignal::new(LoginViewMode::AccountSelector);

    // Check if we have accounts on mount
    Effect::new(move |_| {
        let auth = auth_stored.get_value();
        spawn_local(async move {
            web_sys::console::log_1(&"[LoginView] Checking for accounts...".into());
            let has_accounts = invoke_has_accounts().await.unwrap_or(false);
            web_sys::console::log_1(&format!("[LoginView] has_accounts: {}", has_accounts).into());
            if !has_accounts {
                // No accounts, go directly to add account view
                view_mode.set(LoginViewMode::AddAccount);
            } else {
                // Load accounts list
                web_sys::console::log_1(&"[LoginView] Loading accounts list...".into());
                let result = auth.load_accounts_list().await;
                web_sys::console::log_1(
                    &format!(
                        "[LoginView] load_accounts_list result: {:?}",
                        result.is_ok()
                    )
                    .into(),
                );
            }
        });
    });

    // Set up event listener for bunker-auth-challenge (opens browser for approval)
    Effect::new(move |_| {
        spawn_local(async move {
            #[cfg(any(target_arch = "wasm32", not(feature = "web")))]
            {
                use crate::tauri_invoke::listen_bunker_auth_challenge;
                let _ = listen_bunker_auth_challenge(|auth_url| {
                    web_sys::console::log_1(
                        &format!("Received bunker-auth-challenge: {}", auth_url).into(),
                    );
                    // Open the auth URL in a new browser tab
                    if let Some(window) = web_sys::window() {
                        let _ = window.open_with_url_and_target(&auth_url, "_blank");
                    }
                })
                .await;
            }
        });
    });

    // Input signals for nsec login
    let nsec_input = RwSignal::new(String::new());
    let name_input = RwSignal::new(String::new());

    // Form input signals for bunker:// flow (updated for new NIP-46 API)
    let bunker_uri = RwSignal::new(String::new());
    let bunker_display_name = RwSignal::new(String::new()); // NEW: display name for the profile
    let relay = RwSignal::new("wss://relay.damus.io".to_string());

    // Signals for nostrconnect:// flow (keep existing)
    let generated_uri = RwSignal::new(None::<String>);
    let show_generated = RwSignal::new(false);

    // Signals for QR login (Flow B)
    let qr_uri = RwSignal::new(None::<String>);
    let qr_loading = RwSignal::new(false);
    let qr_error = RwSignal::new(None::<String>);
    let qr_polling = RwSignal::new(false);

    // Set up event listener for qr-login-complete (Flow B)
    Effect::new(move |_| {
        spawn_local(async move {
            #[cfg(any(target_arch = "wasm32", not(feature = "web")))]
            {
                use crate::tauri_invoke::listen_qr_login_complete;
                let auth = auth_stored.get_value();
                let view_mode_event = view_mode.clone();
                let _ = listen_qr_login_complete(move |npub| {
                    web_sys::console::log_1(&format!("QR login complete: {}", npub).into());
                    // Update auth state
                    let auth = auth.clone();
                    let view_mode_event = view_mode_event.clone();
                    spawn_local(async move {
                        let _ = auth.load_profiles_list().await;
                        auth.npub.set(Some(npub));
                        auth.has_secure_accounts.set(true);
                        qr_polling.set(false);
                        qr_loading.set(false);
                        // Navigate back to account selector
                        view_mode_event.set(LoginViewMode::AccountSelector);
                    });
                })
                .await;
            }
        });
    });

    // Handle QR login button click
    let on_start_qr_login = move |_| {
        let auth = auth_stored.get_value();
        qr_loading.set(true);
        qr_error.set(None);

        spawn_local(async move {
            match invoke_start_qr_login().await {
                Ok(uri) => {
                    qr_uri.set(Some(uri));
                    qr_loading.set(false);
                    view_mode.set(LoginViewMode::QrLogin);

                    // Start polling for connection
                    qr_polling.set(true);
                    let view_mode_poll = view_mode.clone();
                    let qr_polling_clone = qr_polling.clone();
                    spawn_local(async move {
                        let auth = auth.clone();
                        while qr_polling_clone.get() {
                            match invoke_check_qr_connection().await {
                                Ok(Some(result)) => {
                                    // Connection established
                                    if let Some(npub) =
                                        result.get("pubkey").and_then(|v| v.as_str())
                                    {
                                        let _ = auth.load_profiles_list().await;
                                        auth.npub.set(Some(npub.to_string()));
                                        auth.has_secure_accounts.set(true);
                                    }
                                    qr_polling_clone.set(false);
                                    // Navigate back to account selector (which will show logged-in state)
                                    view_mode_poll.set(LoginViewMode::AccountSelector);
                                    break;
                                }
                                Ok(None) => {
                                    // Still waiting, continue polling
                                    // Wait 5 seconds before next poll (backend has 30s timeout)
                                }
                                Err(e) => {
                                    qr_error.set(Some(e));
                                    qr_polling_clone.set(false);
                                    break;
                                }
                            }
                            // Poll every 5 seconds (backend has 30s timeout)
                            #[cfg(target_arch = "wasm32")]
                            {
                                use js_sys::Promise;
                                use wasm_bindgen_futures::JsFuture;
                                let _ = JsFuture::from(Promise::new(&mut |resolve, _| {
                                    web_sys::window()
                                        .unwrap()
                                        .set_timeout_with_callback_and_timeout_and_arguments_0(
                                            &resolve, 5000,
                                        )
                                        .unwrap();
                                }))
                                .await;
                            }
                        }
                    });
                }
                Err(e) => {
                    qr_error.set(Some(e));
                    qr_loading.set(false);
                }
            }
        });
    };

    // Handle cancel QR login
    let on_cancel_qr = move |_| {
        qr_polling.set(false);
        qr_uri.set(None);
        view_mode.set(LoginViewMode::AddAccount);
    };

    // Handle bunker:// connect button click (updated for new NIP-46 API)
    // Now uses connect_bunker which handles both connection and saving
    let on_connect_bunker = move |_| {
        let auth = auth_stored.get_value();
        let uri_val = bunker_uri.get();
        let display_name_val = bunker_display_name.get();

        if uri_val.is_empty() {
            auth.error.set(Some(
                "Please enter a bunker:// URI or NIP-05 identifier".to_string(),
            ));
            return;
        }

        auth.is_loading.set(true);
        auth.error.set(None);

        spawn_local(async move {
            // Use the new connect_bunker API which handles both connection and saving
            match invoke_connect_bunker(uri_val, display_name_val).await {
                Ok(result) => {
                    // Extract npub from result
                    if let Some(npub) = result.get("pubkey").and_then(|v| v.as_str()) {
                        // Reload profiles list to show the new profile
                        let _ = auth.load_profiles_list().await;
                        auth.npub.set(Some(npub.to_string()));
                        auth.has_secure_accounts.set(true);
                        auth.is_loading.set(false);

                        // Start connection status polling for NIP-46 accounts
                        auth.start_connection_status_polling().await;
                    } else {
                        auth.error.set(Some(
                            "Connected but failed to get pubkey from response".to_string(),
                        ));
                        auth.is_loading.set(false);
                    }
                }
                Err(e) => {
                    auth.error.set(Some(format!("Failed to connect: {}", e)));
                    auth.is_loading.set(false);
                }
            }
        });
    };

    // Handle generate nostrconnect:// URI button click (keep existing)
    let on_generate_nostrconnect = move |_| {
        let auth = auth_stored.get_value();
        let relay_val = relay.get();
        auth.is_loading.set(true);
        auth.error.set(None);

        spawn_local(async move {
            match invoke_generate_nostrconnect_uri(relay_val).await {
                Ok(result) => {
                    if let Some(uri) = result.get("uri").and_then(|v| v.as_str()) {
                        generated_uri.set(Some(uri.to_string()));
                        show_generated.set(true);
                    }
                    auth.is_loading.set(false);
                }
                Err(e) => {
                    auth.error
                        .set(Some(format!("Failed to generate URI: {}", e)));
                    auth.is_loading.set(false);
                }
            }
        });
    };

    view! {
        <div class="login-view">
            <div class="login-container">
                // Header
                <div class="login-header">
                    <h1>"Arcadestr"</h1>
                    <p class="tagline">"Nostr-powered indie game marketplace"</p>
                </div>

                // Main content based on view mode
                <div class="login-content">
                    <Show when=move || view_mode.get() == LoginViewMode::AccountSelector>
                        {move || {
                            let auth = auth_stored.get_value();
                            let auth_for_switch = auth.clone();
                            let auth_for_delete = auth.clone();
                            view! {
                                <div class="account-selector-view">
                                    <h2>"Welcome Back"</h2>
                                    <p class="subtitle">"Select an account to continue"</p>

                                    <AccountSelector
                                        auth=auth.clone()
                                        on_switch=move |id: String| {
                                            let auth = auth_for_switch.clone();
                                            spawn_local(async move {
                                                if let Err(e) = auth.switch_account(id).await {
                                                    auth.error.set(Some(e));
                                                }
                                            });
                                        }
                                        on_delete=move |id: String| {
                                            let auth = auth_for_delete.clone();
                                            let view_mode = view_mode.clone();
                                            spawn_local(async move {
                                                if let Err(e) = auth.delete_account(id).await {
                                                    auth.error.set(Some(e));
                                                } else {
                                                    // Check if we still have accounts
                                                    let has_accounts = invoke_has_accounts().await.unwrap_or(false);
                                                    if !has_accounts {
                                                        view_mode.set(LoginViewMode::AddAccount);
                                                    }
                                                }
                                            });
                                        }
                                        on_add_account=move || view_mode.set(LoginViewMode::AddAccount)
                                    />
                                </div>
                            }
                        }}
                    </Show>

                    <Show when=move || view_mode.get() == LoginViewMode::AddAccount>
                        <div class="add-account-view">
                            <button class="back-btn" on:click=move |_| {
                                // Only go back if we have accounts
                                spawn_local(async move {
                                    if invoke_has_accounts().await.unwrap_or(false) {
                                        view_mode.set(LoginViewMode::AccountSelector);
                                    }
                                });
                            }>
                                "← Back"
                            </button>

                            <h2>"Add Account"</h2>
                            <p class="subtitle">"Choose how you want to connect"</p>

                            <div class="login-options">
                                // Option 1: Nsec with encryption (NEW PRIMARY)
                                <div class="login-option primary">
                                    <h3>"🔐 Login with Private Key"</h3>
                                    <p class="description">
                                        "Fast login (~4 seconds) with encrypted local storage"
                                    </p>
                                    <button on:click=move |_| view_mode.set(LoginViewMode::NsecLogin)>
                                        "Enter Private Key"
                                    </button>
                                </div>

                                // Option 2: NIP-46 (existing, keep but less prominent)
                                <div class="login-option">
                                    <h3>"📱 Remote Signer (NIP-46)"</h3>
                                    <p class="description">
                                        "Connect with Amber, Nsec.app, or other signer"
                                    </p>

                                    <div class="input-group">
                                        <label>"Relay URL"</label>
                                        <input
                                            type="text"
                                            placeholder="wss://relay.example.com"
                                            prop:value={move || relay.get()}
                                            on:input:target=move |ev| relay.set(ev.target().value())
                                        />
                                    </div>

                                    <div class="input-group">
                                        <label>"Bunker URI or NIP-05"</label>
                                        <input
                                            type="text"
                                            placeholder="bunker://... or user@nsec.app"
                                            prop:value={move || bunker_uri.get()}
                                            on:input:target=move |ev| bunker_uri.set(ev.target().value())
                                        />
                                    </div>

                                    <div class="input-group">
                                        <label>"Profile Name (optional)"</label>
                                        <input
                                            type="text"
                                            placeholder="My Gaming Account"
                                            prop:value={move || bunker_display_name.get()}
                                            on:input:target=move |ev| bunker_display_name.set(ev.target().value())
                                        />
                                    </div>

                                    <button on:click=on_connect_bunker disabled=move || auth_stored.get_value().is_loading.get()>
                                        {move || if auth_stored.get_value().is_loading.get() { "Connecting..." } else { "Connect with Bunker" }}
                                    </button>

                                    <button on:click=on_generate_nostrconnect disabled=move || auth_stored.get_value().is_loading.get()>
                                        "Generate nostrconnect:// URI"
                                    </button>

                                    <Show when=move || show_generated.get()>
                                        {move || generated_uri.get().map(|uri| {
                                            let _uri_for_clipboard = uri.clone();
                                            view! {
                                                <div class="generated-uri">
                                                    <textarea readonly prop:value=uri rows="3"/>
                                                    <button on:click=move |_| {
                                                        #[cfg(target_arch = "wasm32")]
                                                        {
                                                            if let Some(window) = web_sys::window() {
                                                                let _ = window.navigator().clipboard().write_text(&_uri_for_clipboard);
                                                            }
                                                        }
                                                    }>
                                                        "Copy URI"
                                                    </button>
                                                </div>
                                            }
                                        })}
                                    </Show>
                                </div>

                                // Option 3: QR Code Login (Flow B)
                                <div class="login-option qr-option">
                                    <h3>"📲 Scan QR Code"</h3>
                                    <p class="description">
                                        "Scan a QR code with your mobile signer (Amber, Amethyst)"
                                    </p>
                                    <button
                                        on:click=on_start_qr_login
                                        disabled=move || qr_loading.get()
                                    >
                                        {move || if qr_loading.get() { "Generating..." } else { "Show QR Code" }}
                                    </button>
                                    <Show when=move || qr_error.get().is_some()>
                                        <div class="error-text">
                                            {move || qr_error.get().unwrap_or_default()}
                                        </div>
                                    </Show>
                                </div>

                                // Option 4: Restore from backup
                                <div class="login-option">
                                    <h3>"☁️ Restore from Backup"</h3>
                                    <p class="description">
                                        "Restore your accounts from an encrypted backup string"
                                    </p>
                                    <button on:click=move |_| view_mode.set(LoginViewMode::RestoreBackup)>
                                        "Restore from Backup"
                                    </button>
                                </div>
                            </div>
                        </div>
                    </Show>

                    <Show when=move || view_mode.get() == LoginViewMode::NsecLogin>
                        {move || {
                            let auth = auth_stored.get_value();
                            let auth_for_click = auth.clone();
                            view! {
                                <div class="nsec-login-view">
                                    <button class="back-btn" on:click=move |_| view_mode.set(LoginViewMode::AddAccount)>
                                        "← Back"
                                    </button>

                                    <h2>"Login with Private Key"</h2>
                                    <p class="info">
                                        "Your key will be encrypted with AES-256-GCM and stored locally. "
                                        "Master key never leaves your device."
                                    </p>

                                    <div class="form-group">
                                        <label>"Private Key (nsec1...)"</label>
                                        <input
                                            type="password"
                                            placeholder="nsec1..."
                                            bind:value=nsec_input
                                        />
                                    </div>

                                    <div class="form-group">
                                        <label>"Account Name (optional)"</label>
                                        <input
                                            type="text"
                                            placeholder="My Gaming Account"
                                            bind:value=name_input
                                        />
                                    </div>

                                    <div class="security-notice">
                                        <p>"🔒 Security Features:"</p>
                                        <ul>
                                            <li>"Encrypted with AES-256-GCM"</li>
                                            <li>"Master key stored securely (0600 permissions)"</li>
                                            <li>"~4 second login time"</li>
                                            <li>"Automatic encrypted backups available"</li>
                                        </ul>
                                    </div>

                                    <button
                                        class="login-btn primary"
                                        on:click=move |_| {
                                            let nsec = nsec_input.get();
                                            let name = name_input.get();
                                            if !nsec.is_empty() {
                                                let auth = auth_for_click.clone();
                                                spawn_local(async move {
                                                    let name_opt = if name.is_empty() { None } else { Some(name) };
                                                    if let Err(e) = auth.login_with_nsec(nsec, name_opt).await {
                                                        auth.error.set(Some(e));
                                                    }
                                                });
                                            }
                                        }
                                        disabled=move || nsec_input.get().is_empty() || auth.is_loading.get()
                                    >
                                        {move || if auth.is_loading.get() { "Logging in..." } else { "Login & Encrypt" }}
                                    </button>
                                </div>
                            }
                        }}
                    </Show>

                    <Show when=move || view_mode.get() == LoginViewMode::QrLogin>
                        <div class="qr-login-view">
                            <button class="back-btn" on:click=on_cancel_qr>
                                "← Back"
                            </button>

                            <h2>"Scan with Mobile Signer"</h2>
                            <p class="subtitle">"Scan this QR code with Amber, Amethyst, or another NIP-46 signer"</p>

                            <div class="qr-container">
                                <Show when=move || qr_uri.get().is_some()>
                                    {move || qr_uri.get().map(|uri| {
                                        let uri_for_button = uri.clone();
                                        // Generate QR code SVG
                                        let qr_svg = generate_qr_svg(&uri);
                                        view! {
                                            <div class="qr-code-display">
                                                <div class="qr-placeholder">
                                                    // Display actual QR code
                                                    <div class="qr-image" inner_html=qr_svg></div>

                                                    // Also show the URI for manual copy/paste
                                                    <details class="qr-uri-details">
                                                        <summary>"Show URI (for manual copy)"</summary>
                                                        <textarea
                                                            readonly
                                                            prop:value=uri
                                                            rows="3"
                                                            class="qr-uri-text"
                                                        />
                                                    </details>

                                                    <button
                                                        class="copy-btn"
                                                        on:click=move |_| {
                                                            #[cfg(target_arch = "wasm32")]
                                                            {
                                                                if let Some(window) = web_sys::window() {
                                                                    let _ = window.navigator().clipboard().write_text(&uri_for_button);
                                                                }
                                                            }
                                                        }
                                                    >
                                                        "Copy URI"
                                                    </button>
                                                </div>
                                            </div>
                                        }
                                    })}
                                </Show>

                                <Show when=move || qr_polling.get()>
                                    <div class="qr-status polling">
                                        <div class="spinner"></div>
                                        <p>"Waiting for mobile signer to connect..."</p>
                                        <p class="hint">"Make sure your mobile wallet is online"</p>
                                    </div>
                                </Show>

                                <Show when=move || qr_error.get().is_some()>
                                    <div class="qr-status error">
                                        <p class="error-text">{move || qr_error.get().unwrap_or_default()}</p>
                                    </div>
                                </Show>
                            </div>

                            <div class="qr-instructions">
                                <h4>"How to connect:"</h4>
                                <ol>
                                    <li>"Open your mobile Nostr wallet (Amber, Amethyst)"</li>
                                    <li>"Go to Settings → Nostr Connect"</li>
                                    <li>"Tap 'Scan QR' and point at this code"</li>
                                    <li>"Approve the connection on your phone"</li>
                                </ol>
                            </div>
                        </div>
                    </Show>

                    <Show when=move || view_mode.get() == LoginViewMode::RestoreBackup>
                        <div class="restore-backup-view">
                            <button class="back-btn" on:click=move |_| view_mode.set(LoginViewMode::AddAccount)>
                                "← Back"
                            </button>

                            <h2>"Restore from Backup"</h2>
                            <p class="subtitle">"Paste your encrypted backup string to restore accounts"</p>

                            <BackupManager />
                        </div>
                    </Show>
                </div>

                // Error display
                <Show when=move || auth_stored.get_value().error.get().is_some()>
                    <div class="error-message">
                        {auth_stored.get_value().error.get().unwrap()}
                    </div>
                </Show>
            </div>
        </div>
    }
}

/// Main view component - displayed when authenticated
#[component]
fn MainView(relay_count: RwSignal<usize>) -> impl IntoView {
    let auth = use_context::<AuthContext>().expect("AuthContext not provided");
    let auth_stored = StoredValue::new(auth.clone());

    // Marketplace view state
    let current_view = RwSignal::new(MarketplaceView::Browse);

    // Handle disconnect button click (updated for new NIP-46 API)
    let on_disconnect = move |_| {
        let auth = auth_stored.get_value();
        spawn_local(async move {
            match invoke_logout_nip46().await {
                Ok(_) => {
                    auth.npub.set(None);
                    auth.error.set(None);
                    auth.active_account.set(None);
                    auth.connection_status.set("disconnected".to_string());
                    auth.connection_error.set(None);
                    // Reload accounts list to refresh "Current" status
                    let _ = auth.load_accounts_list().await;
                }
                Err(e) => {
                    auth.error.set(Some(e));
                }
            }
        });
    };

    // Poll relay count every 5 seconds
    Effect::new(move |_| {
        if let Some(window) = web_sys::window() {
            let relay_count_clone = relay_count.clone();
            let _ = window.set_interval_with_callback_and_timeout_and_arguments_0(
                &Closure::wrap(Box::new(move || {
                    let relay_count_local = relay_count_clone.clone();
                    spawn_local(async move {
                        match invoke_get_relay_count().await {
                            Ok(count) => relay_count_local.set(count),
                            Err(e) => web_sys::console::log_1(
                                &format!("Failed to get relay count: {}", e).into(),
                            ),
                        }
                    });
                }) as Box<dyn FnMut()>)
                .into_js_value()
                .as_ref()
                .unchecked_ref(),
                5000,
            );
        }
    });

    // Relay dropdown state
    let show_relay_dropdown = RwSignal::new(false);
    let relay_list = RwSignal::new(Vec::<String>::new());

    // Toggle relay dropdown and fetch relay list
    let on_relay_badge_click = move |_| {
        let current = show_relay_dropdown.get();
        show_relay_dropdown.set(!current);

        // Fetch relay list when opening
        if !current {
            spawn_local(async move {
                match invoke_get_connected_relays().await {
                    Ok(relays) => relay_list.set(relays),
                    Err(e) => {
                        web_sys::console::log_1(&format!("Failed to get relay list: {}", e).into())
                    }
                }
            });
        }
    };

    // Close dropdown when clicking outside
    let on_close_relay_dropdown = move |_| {
        show_relay_dropdown.set(false);
    };

    // Get profile for display - returns a closure that will be called in reactive context
    let get_profile = move || {
        // First check AuthContext profile
        if let Some(p) = auth.profile.get() {
            #[cfg(debug_assertions)]
            {
                web_sys::console::log_1(&format!("get_profile: Found in auth.profile").into());
            }
            return Some(p);
        }

        // Then check ProfileStore (use_context returns Option<T>)
        if let Some(store) = use_context::<ProfileStore>() {
            if let Some(npub) = auth.npub.get() {
                #[cfg(debug_assertions)]
                {
                    web_sys::console::log_1(
                        &format!("get_profile: Checking ProfileStore for {}", npub).into(),
                    );
                    let count = store.signal().get_untracked().len();
                    web_sys::console::log_1(
                        &format!("get_profile: Store has {} profiles", count).into(),
                    );
                }
                if let Some(p) = store.get(&npub) {
                    #[cfg(debug_assertions)]
                    {
                        web_sys::console::log_1(
                            &format!("get_profile: Found in ProfileStore!").into(),
                        );
                    }
                    return Some(p);
                }
            }
        }

        #[cfg(debug_assertions)]
        {
            web_sys::console::log_1(&format!("get_profile: Not found anywhere").into());
        }
        None
    };
    let get_npub = move || {
        let n = auth.npub.get();
        #[cfg(debug_assertions)]
        {
            web_sys::console::log_1(&format!("get_npub called, npub: {:?}", n.is_some()).into());
        }
        n
    };

    // Get display name from profile - returns a closure
    let get_profile_display = move || {
        if let Some(p) = get_profile() {
            let name = p.display();
            #[cfg(debug_assertions)]
            {
                web_sys::console::log_1(&format!("Display name from profile: {}", name).into());
            }
            name
        } else if let Some(n) = get_npub() {
            let name = if n.len() > 16 {
                format!("{}...", &n[..16])
            } else {
                n
            };
            #[cfg(debug_assertions)]
            {
                web_sys::console::log_1(&format!("Display name from npub: {}", name).into());
            }
            name
        } else {
            "?".to_string()
        }
    };

    // Check if NIP-05 is verified
    let get_nip05_verified = move || get_profile().map(|p| p.nip05_verified).unwrap_or(false);

    // Get picture URL
    let get_picture_url = move || {
        let url = get_profile().and_then(|p| p.picture.clone());
        #[cfg(debug_assertions)]
        {
            web_sys::console::log_1(&format!("get_picture_url: {:?}", url.is_some()).into());
        }
        url
    };

    // Get first letter for avatar placeholder
    let get_avatar_letter = move || {
        let name = get_profile_display();
        let letter = name
            .chars()
            .next()
            .map(|c| c.to_uppercase().to_string())
            .unwrap_or_else(|| "?".to_string());
        #[cfg(debug_assertions)]
        {
            web_sys::console::log_1(&format!("avatar_letter: {}", letter).into());
        }
        letter
    };

    // Get NIP-05 identifier for tooltip
    let get_nip05 = move || {
        get_profile()
            .and_then(|p| p.nip05.clone())
            .unwrap_or_default()
    };

    // Navigation handlers
    let on_browse = move |_| {
        current_view.set(MarketplaceView::Browse);
    };

    let on_publish = move |_| {
        current_view.set(MarketplaceView::Publish);
    };

    let on_profile = move |_| {
        current_view.set(MarketplaceView::Profile);
    };

    let on_select_listing = Callback::new(move |listing: GameListing| {
        current_view.set(MarketplaceView::Detail(listing));
    });

    let on_back = Callback::new(move |_| {
        current_view.set(MarketplaceView::Browse);
    });

    view! {
        <div class="main-view">
            <header class="header">
                <h2 class="header-title">"Arcadestr"</h2>
                <div class="user-info">
                    // Relay count badge with mesh network icon - now clickable
                    {move || {
                        let count = relay_count.get();
                        let badge_class = if count == 0 {
                            "relay-count-badge disconnected"
                        } else if count < 3 {
                            "relay-count-badge connecting"
                        } else {
                            "relay-count-badge connected"
                        };
                        let is_open = show_relay_dropdown.get();
                        let dropdown_class = if is_open { "relay-dropdown open" } else { "relay-dropdown" };
                        let relays = relay_list.get();

                        view! {
                            <div class="relay-badge-container">
                                <button
                                    class={badge_class}
                                    on:click={on_relay_badge_click}
                                    title={format!("{} relays connected - Click to view", count)}
                                >
                                    // Mesh network icon (SVG)
                                    <svg class="relay-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                                        <circle cx="12" cy="5" r="2"/>
                                        <circle cx="5" cy="12" r="2"/>
                                        <circle cx="19" cy="12" r="2"/>
                                        <circle cx="8" cy="19" r="2"/>
                                        <circle cx="16" cy="19" r="2"/>
                                        <line x1="12" y1="7" x2="12" y2="11"/>
                                        <line x1="6.5" y1="10.5" x2="10.5" y2="12.5"/>
                                        <line x1="17.5" y1="10.5" x2="13.5" y2="12.5"/>
                                        <line x1="9" y1="17" x2="11" y2="15"/>
                                        <line x1="15" y1="17" x2="13" y2="15"/>
                                    </svg>
                                    <span class="relay-count-number">{count}</span>
                                </button>

                                // Dropdown menu
                                <div class={dropdown_class}>
                                    <div class="relay-dropdown-header">
                                        <span class="relay-dropdown-title">"Connected Relays"</span>
                                        <button class="relay-dropdown-close" on:click={on_close_relay_dropdown}>"✕"</button>
                                    </div>
                                    <div class="relay-dropdown-list">
                                        {if relays.is_empty() {
                                            view! {
                                                <div class="relay-dropdown-empty">"No relays connected"</div>
                                            }.into_any()
                                        } else {
                                            view! {
                                                <ul>
                                                    {relays.into_iter().map(|relay| {
                                                        view! {
                                                            <li class="relay-dropdown-item">
                                                                <span class="relay-url">{relay}</span>
                                                            </li>
                                                        }
                                                    }).collect::<Vec<_>>()}
                                                </ul>
                                            }.into_any()
                                        }}
                                    </div>
                                </div>
                            </div>
                        }.into_any()
                    }}
                    // Connection status indicator for NIP-46 accounts
                    {move || {
                        let status = auth.connection_status.get();
                        let error = auth.connection_error.get();

                        // Only show for NIP-46 accounts (when we have a connection status to show)
                        if status != "disconnected" {
                            let (icon, text, class) = match status.as_str() {
                                "connecting" => ("🟡", "Connecting...", "connection-status connecting"),
                                "connected" => ("🟢", "Connected", "connection-status connected"),
                                "failed" => ("🔴", "Connection Failed", "connection-status failed"),
                                _ => ("⚪", "Unknown", "connection-status unknown"),
                            };

                            let title = if let Some(err) = error {
                                format!("{}: {}", text, err)
                            } else {
                                text.to_string()
                            };

                            Some(view! {
                                <div class={class} title={title}>
                                    <span class="connection-icon">{icon}</span>
                                    <span class="connection-text">{text}</span>
                                </div>
                            }.into_any())
                        } else {
                            None
                        }
                    }}

                    <button class="user-profile-btn" on:click={on_profile}>
                        {move || {
                            if let Some(url) = get_picture_url() {
                                Some(view! {
                                    <img src={url} class="avatar" alt="avatar" />
                                }.into_any())
                            } else {
                                Some(view! {
                                    <div class="avatar-placeholder">{get_avatar_letter()}</div>
                                }.into_any())
                            }
                        }}
                        <span class="user-display-name">
                            {move || get_profile_display()}
                            {move || {
                                if get_nip05_verified() {
                                    let nip05 = get_nip05();
                                    Some(view! {
                                        <span class="verified-badge" title={format!("NIP-05 verified: {}", nip05)}>{"✓"}</span>
                                    }.into_any())
                                } else {
                                    None
                                }
                            }}
                        </span>
                    </button>
                    <button
                        class="disconnect-button"
                        on:click=on_disconnect
                    >
                        "Disconnect"
                    </button>
                </div>
            </header>

            <div class="marketplace-container">
                <aside class="sidebar">
                    <button
                        class={move || {
                            if matches!(current_view.get(), MarketplaceView::Browse) {
                                "sidebar-button active"
                            } else {
                                "sidebar-button"
                            }
                        }}
                        on:click={on_browse}
                    >
                        "Browse"
                    </button>
                    <button
                        class={move || {
                            if matches!(current_view.get(), MarketplaceView::Publish) {
                                "sidebar-button active"
                            } else {
                                "sidebar-button"
                            }
                        }}
                        on:click={on_publish}
                    >
                        "Publish"
                    </button>
                </aside>

                <main class="content-area">
                    {move || {
                        match current_view.get() {
                            MarketplaceView::Browse => {
                                view! {
                                    <BrowseView on_select={on_select_listing} />
                                }.into_any()
                            }
                            MarketplaceView::Publish => {
                                view! {
                                    <PublishView />
                                }.into_any()
                            }
                            MarketplaceView::Detail(listing) => {
                                view! {
                                    <DetailView
                                        listing={listing.clone()}
                                        on_back={on_back}
                                    />
                                }.into_any()
                            }
                            MarketplaceView::Profile => {
                                let set_view = current_view.write_only();
                                view! {
                                    <ProfileView set_view={set_view} />
                                }.into_any()
                            }
                        }
                    }}
                </main>
            </div>

            {move || {
                auth.error.get().map(|err| {
                    view! {
                        <div class="error-message" style="margin: 1rem;">{err}</div>
                    }
                })
            }}
        </div>
    }
}

/// Root application component
#[component]
pub fn App() -> impl IntoView {
    // Create and provide auth context
    let auth = AuthContext::new();
    provide_context(auth.clone());
    let relay_count = RwSignal::new(0);

    // Store auth for use in effects
    let auth_stored = StoredValue::new(auth.clone());

    // Initialize profile store
    provide_profile_store();
    let profile_store = use_profile_store();

    // Setup event listeners for profile updates
    #[cfg(any(target_arch = "wasm32", not(feature = "web")))]
    setup_profile_event_handlers(profile_store.clone());

    // Check authentication status on mount (with small delay for Tauri to initialize)
    Effect::new(move |_| {
        let auth = auth_stored.get_value();
        spawn_local(async move {
            // Small delay to ensure Tauri API is ready
            #[cfg(target_arch = "wasm32")]
            {
                use js_sys::Promise;
                use wasm_bindgen_futures::JsFuture;
                let _ = JsFuture::from(Promise::new(&mut |resolve, _| {
                    web_sys::window()
                        .unwrap()
                        .set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, 100)
                        .unwrap();
                }))
                .await;
            }

            // Try new secure storage initialization first
            match invoke_has_accounts().await {
                Ok(true) => {
                    // We have accounts in secure storage, try fast login
                    match invoke_load_active_account().await {
                        Ok(result) => {
                            if let Ok(account) = parse_account_from_result(&result) {
                                auth.active_account.set(Some(account.clone()));
                                auth.npub.set(Some(account.npub.clone()));
                                auth.has_secure_accounts.set(true);

                                // Start connection status polling for NIP-46 accounts
                                if account.signing_mode == "nip46" {
                                    auth.start_connection_status_polling().await;
                                }

                                // IMMEDIATE: Try to get profile from backend cache first
                                let npub_for_fetch = account.npub.clone();
                                let auth_for_fetch = auth.clone();
                                spawn_local(async move {
                                    web_sys::console::log_1(
                                        &format!(
                                            "[INIT] Immediate profile fetch for: {}",
                                            npub_for_fetch
                                        )
                                        .into(),
                                    );

                                    // First try to get from backend cache
                                    match invoke_get_cached_profile(npub_for_fetch.clone()).await {
                                        Ok(Some(profile)) => {
                                            web_sys::console::log_1(
                                                &format!(
                                                    "[INIT] Got cached profile immediately: {:?}",
                                                    profile.name
                                                )
                                                .into(),
                                            );
                                            auth_for_fetch.profile.set(Some(profile));
                                        }
                                        _ => {
                                            web_sys::console::log_1(&"[INIT] No cached profile, fetching from relays...".into());
                                            // Fetch from relays
                                            match invoke_fetch_profile(npub_for_fetch.clone(), None)
                                                .await
                                            {
                                                Ok(profile) => {
                                                    web_sys::console::log_1(&format!("[INIT] Profile fetched from relays: {:?}", profile.name).into());
                                                    auth_for_fetch.profile.set(Some(profile));
                                                }
                                                Err(e) => {
                                                    web_sys::console::log_1(
                                                        &format!(
                                                            "[INIT] Profile fetch failed: {}",
                                                            e
                                                        )
                                                        .into(),
                                                    );
                                                }
                                            }
                                        }
                                    }
                                });
                            }
                        }
                        Err(_) => {
                            // No active account, but we have accounts
                            // Load list for user to select
                            auth.has_secure_accounts.set(true);
                            if let Ok(result) = invoke_list_saved_profiles().await {
                                if let Ok(accounts) = parse_accounts_list(&result) {
                                    auth.accounts.set(accounts.clone());

                                    // Fetch profiles for all saved accounts to show pictures/names
                                    let store = try_use_profile_store();
                                    for account in accounts {
                                        if let Some(store) = store.clone() {
                                            let npub = account.npub.clone();
                                            spawn_local(async move {
                                                // First try to get from backend cache
                                                match invoke_get_cached_profile(npub.clone()).await
                                                {
                                                    Ok(Some(profile)) => {
                                                        web_sys::console::log_1(&format!("[ACCOUNTS] Cached profile for {}: {:?}", npub, profile.name).into());
                                                        store.put(profile);
                                                    }
                                                    _ => {
                                                        // Not in cache, fetch from relays
                                                        web_sys::console::log_1(&format!("[ACCOUNTS] Fetching profile for {} from relays", npub).into());
                                                        if let Ok(profile) =
                                                            invoke_fetch_profile(npub.clone(), None)
                                                                .await
                                                        {
                                                            web_sys::console::log_1(&format!("[ACCOUNTS] Fetched profile for {}: {:?}", npub, profile.name).into());
                                                            store.put(profile);
                                                        }
                                                    }
                                                }
                                            });
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                Ok(false) => {
                    // No accounts in new system, will show login screen
                    auth.has_secure_accounts.set(false);
                    auth.npub.set(None);
                }
                Err(_) => {
                    // Error checking secure storage, fallback to legacy check
                    match invoke_is_authenticated().await {
                        Ok(true) => {
                            if let Ok(npub) = invoke_get_public_key().await {
                                auth.npub.set(Some(npub));
                            }
                        }
                        _ => {
                            auth.npub.set(None);
                        }
                    }
                }
            }
        });
    });

    // Load cached profiles from backend on startup
    spawn_local(async move {
        match invoke_get_cached_profiles().await {
            Ok(profiles) => {
                // Try to get the store - it might not be ready yet
                let store: Option<ProfileStore> = try_use_profile_store();
                if let Some(store) = store {
                    #[cfg(debug_assertions)]
                    {
                        web_sys::console::log_1(
                            &format!("Loaded {} profiles from backend cache", profiles.len())
                                .into(),
                        );
                        for profile in &profiles {
                            web_sys::console::log_1(
                                &format!(
                                    "  - {}: name={:?}, display_name={:?}",
                                    profile.npub, profile.name, profile.display_name
                                )
                                .into(),
                            );
                        }
                    }
                    store.put_many(profiles);
                    #[cfg(debug_assertions)]
                    {
                        web_sys::console::log_1(
                            &format!("ProfileStore now has {} profiles", store.get_all().len())
                                .into(),
                        );
                    }
                } else {
                    #[cfg(debug_assertions)]
                    {
                        web_sys::console::log_1(
                            &"ProfileStore not ready yet, skipping cache load".into(),
                        );
                    }
                }
            }
            Err(e) => {
                #[cfg(debug_assertions)]
                {
                    web_sys::console::log_1(
                        &format!("Failed to load cached profiles: {}", e).into(),
                    );
                }
            }
        }
    });

    // Auto-fetch profile when npub becomes Some
    // Track npub signal and fetch profile when it changes
    let auth_for_effect = auth.clone();
    Effect::new(move |_| {
        // This read makes the effect track auth.npub
        let npub = auth_for_effect.npub.get();

        #[cfg(debug_assertions)]
        {
            web_sys::console::log_1(&format!("Profile effect triggered, npub: {:?}", npub).into());
        }

        if let Some(npub) = npub {
            let auth = auth_for_effect.clone();
            spawn_local(async move {
                #[cfg(debug_assertions)]
                {
                    web_sys::console::log_1(&format!("Fetching profile for: {}", npub).into());
                }
                match invoke_fetch_profile(npub.clone(), None).await {
                    Ok(profile) => {
                        #[cfg(debug_assertions)]
                        {
                            web_sys::console::log_1(
                                &format!("Profile fetched: {:?}", profile.name).into(),
                            );
                        }
                        auth.profile.set(Some(profile.clone()));

                        // Also save profile to saved users for display on login screen
                        spawn_local(async move {
                            match invoke_fetch_and_save_user_profile().await {
                                Ok(_) => {
                                    #[cfg(debug_assertions)]
                                    {
                                        web_sys::console::log_1(
                                            &"Profile saved to saved users".into(),
                                        );
                                    }
                                }
                                Err(e) => {
                                    #[cfg(debug_assertions)]
                                    {
                                        web_sys::console::log_1(
                                            &format!("Failed to save profile: {}", e).into(),
                                        );
                                    }
                                }
                            }
                        });
                    }
                    Err(e) => {
                        #[cfg(debug_assertions)]
                        {
                            web_sys::console::log_1(
                                &format!("Failed to fetch profile: {}", e).into(),
                            );
                        }
                    }
                }

                // Retry logic: if profile is empty (no metadata), retry after 30 seconds
                let auth = auth.clone();
                if auth
                    .profile
                    .get()
                    .map(|p| p.name.is_none() && p.display_name.is_none())
                    .unwrap_or(true)
                {
                    #[cfg(debug_assertions)]
                    {
                        web_sys::console::log_1(
                            &"Profile empty or not found, scheduling retry in 30s...".into(),
                        );
                    }

                    #[cfg(target_arch = "wasm32")]
                    {
                        use js_sys::Promise;
                        use wasm_bindgen_futures::JsFuture;
                        let _ = JsFuture::from(Promise::new(&mut |resolve, _| {
                            web_sys::window()
                                .unwrap()
                                .set_timeout_with_callback_and_timeout_and_arguments_0(
                                    &resolve, 30000,
                                )
                                .unwrap();
                        }))
                        .await;
                    }

                    // Retry fetch
                    #[cfg(debug_assertions)]
                    {
                        web_sys::console::log_1(&"Retrying profile fetch...".into());
                    }
                    if let Ok(profile) = invoke_fetch_profile(npub.clone(), None).await {
                        if profile.name.is_some() || profile.display_name.is_some() {
                            #[cfg(debug_assertions)]
                            {
                                web_sys::console::log_1(
                                    &format!("Profile fetched on retry: {:?}", profile.name).into(),
                                );
                            }
                            auth.profile.set(Some(profile));
                        }
                    }
                }
            });
        } else {
            #[cfg(debug_assertions)]
            {
                web_sys::console::log_1(&"No npub, clearing profile".into());
            }
            auth_for_effect.profile.set(None);
        }
    });

    view! {
        <div class="arcadestr-app">
            <style>{STYLES}</style>

            // Debug overlay - only in debug builds
            {#[cfg(debug_assertions)]
            view! {
                <DebugOverlay />
            }}

            <Show
                when={move || auth.npub.get().is_some()}
                fallback={|| view! { <LoginView /> }}
            >
                <MainView relay_count=relay_count.clone() />
            </Show>
        </div>
    }
}

/// Debug overlay component - only shown in debug builds
#[cfg(debug_assertions)]
#[component]
fn DebugOverlay() -> impl IntoView {
    let show_debug = RwSignal::new(false);
    let version_info = RwSignal::new(None::<VersionInfo>);
    let error_msg = RwSignal::new(None::<String>);

    // Fetch version info on mount
    spawn_local(async move {
        use crate::tauri_invoke::invoke;
        web_sys::console::log_1(&"DebugOverlay: Fetching version info...".into());

        match invoke::<VersionInfo>("get_version_info", serde_json::json!(null)).await {
            Ok(info) => {
                web_sys::console::log_1(
                    &format!("DebugOverlay: Got version info: {:?}", info).into(),
                );
                version_info.set(Some(info));
            }
            Err(e) => {
                let msg = format!("Failed to get version: {:?}", e);
                web_sys::console::error_1(&msg.clone().into());
                error_msg.set(Some(msg));
            }
        }
    });

    view! {
        <div class="debug-overlay" style="position: fixed; bottom: 10px; right: 10px; z-index: 99999;">
            <button
                on:click={move |_| show_debug.set(!show_debug.get())}
                style="background: #333; color: #f5821f; border: 1px solid #f5821f; padding: 5px 10px; border-radius: 4px; cursor: pointer; font-size: 12px;"
            >
                {move || if show_debug.get() { "Hide Debug" } else { "Debug" }}
            </button>

            {move || if show_debug.get() {
                Some(view! {
                    <div style="background: #1a1a1a; border: 1px solid #f5821f; padding: 10px; border-radius: 4px; margin-top: 5px; font-size: 12px; color: #fff; max-width: 300px;">
                        <h4 style="color: #f5821f; margin-bottom: 5px;">Debug Info</h4>
                        <p>Build: Debug</p>
                        {move || version_info.get().map(|info| {
                            let rev_display = format!("(rev {})", info.revision);
                            view! {
                                <div>
                                    <p>Target: {info.os.clone()}</p>
                                    <p>Arch: {info.arch.clone()}</p>
                                    <hr style="border-color: #333; margin: 5px 0;" />
                                    <p style="color: #f5821f; font-weight: bold;">Version: {info.full.clone()}</p>
                                    <p style="color: #888; font-size: 10px;">{rev_display}</p>
                                </div>
                            }
                        })}
                        {move || error_msg.get().map(|err| view! {
                            <div>
                                <hr style="border-color: #333; margin: 5px 0;" />
                                <p style="color: #ff4444;">Error: {err}</p>
                            </div>
                        })}
                        <hr style="border-color: #333; margin: 5px 0;" />
                        <p style="color: #888;">Press Ctrl+Shift+I for DevTools</p>
                    </div>
                })
            } else {
                None
            }}
        </div>
    }
}

/// Version info structure matching the backend
#[cfg(debug_assertions)]
#[derive(Debug, Clone, Serialize, Deserialize)]
struct VersionInfo {
    version: String,
    revision: u32,
    full: String,
    os: String,
    arch: String,
}
