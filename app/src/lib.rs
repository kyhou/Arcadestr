// Leptos application: shared UI components and pages for both desktop and web targets.

use leptos::prelude::*;
use serde::{Serialize, Deserialize};
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::spawn_local;

// Module declarations
pub mod components;
pub mod models;

#[cfg(all(target_arch = "wasm32", feature = "web"))]
pub mod web_auth;

#[cfg(any(target_arch = "wasm32", not(feature = "web")))]
mod tauri_invoke;
pub use components::{BrowseView, DetailView, ProfileView, PublishView};
pub use models::{GameListing, MarketplaceView, UserProfile, ZapInvoice, ZapRequest};

// =============================================================================
// Tauri Invoke Bridge
// =============================================================================

/// Arguments for connect_nip46 command
#[derive(Serialize)]
#[allow(dead_code)]
struct ConnectNip46Args {
    uri: String,
    relay: String,
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

/// Invoke connect_nip46 Tauri command
#[cfg(any(target_arch = "wasm32", not(feature = "web")))]
async fn invoke_connect_nip46(uri: String, relay: String) -> Result<String, String> {
    use crate::tauri_invoke::invoke;

    let connect_args = serde_json::json!({
        "uri": uri,
        "relay": relay
    });

    invoke("connect_nip46", connect_args).await
}

#[cfg(not(any(target_arch = "wasm32", not(feature = "web"))))]
async fn invoke_connect_nip46(_uri: String, _relay: String) -> Result<String, String> {
    Err("Tauri not available in web mode".to_string())
}

/// Arguments for generate_nostrconnect_uri command
#[derive(Serialize)]
#[allow(dead_code)]
struct GenerateNostrconnectUriArgs {
    relay: String,
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

/// Invoke disconnect Tauri command
#[cfg(any(target_arch = "wasm32", not(feature = "web")))]
async fn invoke_disconnect() -> Result<(), String> {
    use crate::tauri_invoke::invoke_void;

    invoke_void("disconnect", serde_json::json!(null)).await
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
    
    invoke("remove_saved_user", serde_json::json!({ "userId": user_id })).await
}

#[cfg(not(any(target_arch = "wasm32", not(feature = "web"))))]
async fn invoke_remove_saved_user(_user_id: String) -> Result<SavedUsers, String> {
    Err("Tauri not available".to_string())
}

/// Invoke rename_saved_user Tauri command
#[cfg(any(target_arch = "wasm32", not(feature = "web")))]
async fn invoke_rename_saved_user(user_id: String, new_name: String) -> Result<SavedUsers, String> {
    use crate::tauri_invoke::invoke;
    
    invoke("rename_saved_user", serde_json::json!({ "userId": user_id, "newName": new_name })).await
}

#[cfg(not(any(target_arch = "wasm32", not(feature = "web"))))]
async fn invoke_rename_saved_user(_user_id: String, _new_name: String) -> Result<SavedUsers, String> {
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
    
    invoke("connect_saved_user", serde_json::json!({ "userId": user_id })).await
}

#[cfg(not(any(target_arch = "wasm32", not(feature = "web"))))]
async fn invoke_connect_saved_user(_user_id: String) -> Result<ConnectResponse, String> {
    Err("Tauri not available".to_string())
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
}

/// Invoke fetch_profile Tauri command
#[cfg(any(target_arch = "wasm32", not(feature = "web")))]
pub async fn invoke_fetch_profile(npub: String) -> Result<UserProfile, String> {
    use crate::tauri_invoke::invoke;

    let fetch_args = serde_json::json!({
        "npub": npub
    });

    invoke("fetch_profile", fetch_args).await
}

#[cfg(not(any(target_arch = "wasm32", not(feature = "web"))))]
pub async fn invoke_fetch_profile(_npub: String) -> Result<UserProfile, String> {
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

/// Authentication context shared across components
#[derive(Clone)]
pub struct AuthContext {
    pub npub: RwSignal<Option<String>>,
    pub profile: RwSignal<Option<UserProfile>>,
    pub is_loading: RwSignal<bool>,
    pub error: RwSignal<Option<String>>,
}

impl AuthContext {
    pub fn new() -> Self {
        Self {
            npub: RwSignal::new(None),
            profile: RwSignal::new(None),
            is_loading: RwSignal::new(false),
            error: RwSignal::new(None),
        }
    }
}

impl Default for AuthContext {
    fn default() -> Self {
        Self::new()
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
.login-container {
    flex: 1;
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 2rem;
}

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

/// Login view component - displays NIP-46 connection options (both nostrconnect:// and bunker://)
#[component]
fn LoginView() -> impl IntoView {
    let auth = use_context::<AuthContext>().expect("AuthContext not provided");

    // Screen state: true = show add new user, false = show saved users
    // Will be initialized after loading saved users
    let show_add_new = RwSignal::new(false);
    let initialized = RwSignal::new(false);

    // Form input signals for bunker:// flow
    let bunker_uri = RwSignal::new(String::new());
    let relay = RwSignal::new("wss://relay.damus.io".to_string());

    // Signals for nostrconnect:// flow
    let generated_uri = RwSignal::new(None::<String>);
    let show_generated = RwSignal::new(false);

    // Signal for direct key input (testing only)
    let direct_key = RwSignal::new(String::new());

    // Handle bunker:// connect button click
    let on_connect_bunker = move |_| {
        let uri_val = bunker_uri.get();
        let relay_val = relay.get();

        if uri_val.is_empty() {
            auth.error
                .set(Some("Please enter a bunker:// URI".to_string()));
            return;
        }

        auth.is_loading.set(true);
        auth.error.set(None);

        spawn_local(async move {
            match invoke_connect_nip46(uri_val.clone(), relay_val.clone()).await {
                Ok(npub) => {
                    // Auto-save user after successful login
                    web_sys::console::log_1(&"Auto-saving user after login...".into());
                    let save_result = invoke_add_saved_user(
                        "nip46".to_string(),
                        Some(relay_val.clone()),
                        Some(uri_val.clone()),
                        None,
                        npub.clone(),
                    ).await;
                    match save_result {
                        Ok(_) => web_sys::console::log_1(&"User saved successfully!".into()),
                        Err(e) => web_sys::console::log_1(&format!("Failed to save user: {}", e).into()),
                    }
                    
                    auth.npub.set(Some(npub));
                    auth.is_loading.set(false);
                }
                Err(e) => {
                    auth.error.set(Some(e));
                    auth.is_loading.set(false);
                }
            }
        });
    };

    // Handle generate nostrconnect:// URI button click
    let on_generate_nostrconnect = move |_| {
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

    // Handle direct key connect button click (for testing)
    let on_connect_direct_key = move |_| {
        let key_val = direct_key.get();

        if key_val.is_empty() {
            auth.error.set(Some(
                "Please enter your private key (nsec1... or hex)".to_string(),
            ));
            return;
        }
        auth.is_loading.set(true);
        auth.error.set(None);

        spawn_local(async move {
            match invoke_connect_with_key(key_val.clone()).await {
                Ok(npub) => {
                    println!("Connected with direct key: {}", npub);
                    
                    // Auto-save user after successful login (without storing the private key)
                    let save_result = invoke_add_saved_user(
                        "private_key".to_string(),
                        None,
                        None,
                        None,  // Don't store private key for security
                        npub.clone(),
                    ).await;
                    if let Err(e) = save_result {
                        web_sys::console::log_1(&format!("Failed to save user: {}", e).into());
                    }
                    
                    auth.npub.set(Some(npub));
                    auth.is_loading.set(false);
                }
                Err(e) => {
                    println!("Failed to connect with direct key: {}", e);
                    auth.error.set(Some(e));
                    auth.is_loading.set(false);
                }
            }
        });
    };

    // Handle NIP-07 connect button click (web only)
    let _on_connect_nip07 = move |_ev: leptos::ev::MouseEvent| {
        auth.is_loading.set(true);
        auth.error.set(None);

        spawn_local(async move {
            match invoke_connect_nip07().await {
                Ok(npub) => {
                    // Auto-save user after successful login
                    let save_result = invoke_add_saved_user(
                        "nip07".to_string(),
                        None,
                        None,
                        None,
                        npub.clone(),
                    ).await;
                    if let Err(e) = save_result {
                        web_sys::console::log_1(&format!("Failed to save user: {}", e).into());
                    }
                    
                    auth.npub.set(Some(npub));
                    auth.is_loading.set(false);
                }
                Err(e) => {
                    auth.error.set(Some(e));
                    auth.is_loading.set(false);
                }
            }
        });
    };

    // ─────────────────────────────────────────────────────────────────────────
    // Saved Users Section
    // ─────────────────────────────────────────────────────────────────────────
    
    // Load saved users on component mount
    let saved_users = SavedUsers { users: Vec::new() };
    let saved_users_signal = RwSignal::new(saved_users);
    
    // Track visibility for reloading saved users
    let is_visible = RwSignal::new(true);
    
    // Load users on mount and when becoming visible again (after disconnect)
    Effect::new(move |_| {
        // This effect runs when is_visible changes to true
        if is_visible.get() {
            spawn_local(async move {
                #[cfg(debug_assertions)]
                {
                    web_sys::console::log_1(&"Reloading saved users...".into());
                }
                match invoke_get_saved_users().await {
                    Ok(users) => {
                        let has_users = !users.users.is_empty();
                        saved_users_signal.set(users);
                        // Start with add new screen if no saved users
                        show_add_new.set(!has_users);
                        initialized.set(true);
                        #[cfg(debug_assertions)]
                        {
                            web_sys::console::log_1(&"Saved users reloaded successfully".into());
                        }
                    }
                    Err(e) => {
                        web_sys::console::log_1(&format!("Failed to load saved users: {}", e).into());
                    }
                }
            });
        }
    });
    
    // Handle connect with saved user
    let on_connect_saved_user = move |user_id: String| {
        auth.is_loading.set(true);
        auth.error.set(None);
        
        spawn_local(async move {
            match invoke_connect_saved_user(user_id).await {
                Ok(response) => {
                    auth.npub.set(Some(response.npub));
                    auth.profile.set(Some(response.profile));
                    auth.is_loading.set(false);
                }
                Err(e) => {
                    auth.error.set(Some(e));
                    auth.is_loading.set(false);
                }
            }
        });
    };
    
    // Handle delete saved user
    let on_delete_saved_user = move |user_id: String| {
        spawn_local(async move {
            match invoke_remove_saved_user(user_id).await {
                Ok(users) => {
                    saved_users_signal.set(users);
                }
                Err(e) => {
                    web_sys::console::log_1(&format!("Failed to delete user: {}", e).into());
                }
            }
        });
    };

    // Convert saved users to a vec for rendering
    let saved_users_vec = move || {
        saved_users_signal.get().users
    };

    // Helper to check if we should show saved users
    let _show_saved_users = move || !show_add_new.get();
    
    // Handle navigation to add new screen
    let go_to_add_new = move |_| {
        show_add_new.set(true);
    };
    
    // Handle navigation back to saved users
    let go_to_saved_users = move |_| {
        show_add_new.set(false);
    };

    view! {
        <div class="login-container">
            <div class="login-card">
                <h1 class="login-title">"Arcadestr"</h1>
                <p class="login-tagline">"The decentralized game marketplace on NOSTR"</p>

                // ─────────────────────────────────────────────────────────────────
                // Loading state while initializing
                // ─────────────────────────────────────────────────────────────────
                {move || {
                    if !initialized.get() {
                        Some(view! {
                            <div class="loading-indicator">"Loading..."</div>
                        })
                    } else {
                        None
                    }
                }}

                // ─────────────────────────────────────────────────────────────────
                // Screen 1: Saved Users (shown when show_add_new is false)
                // ─────────────────────────────────────────────────────────────────
                {move || {
                    if initialized.get() && !show_add_new.get() {
                        Some(view! {
                            <div class="saved-users-screen">
                                <h2 class="screen-title">"Welcome Back"</h2>
                                
                                {move || {
                                    let users = saved_users_vec();
                                    if users.is_empty() {
                                        Some(view! {
                                            <div class="saved-users-empty">
                                                <p>"No saved users yet"</p>
                                            </div>
                                        })
                                    } else {
                                        None
                                    }
                                }}

                                {move || {
                                    let users = saved_users_vec();
                                    if !users.is_empty() {
                                        Some(view! {
                                            <div class="saved-users-section">
                                                {users.into_iter().enumerate().map(|(idx, user)| {
                                                    let npub_short = if user.npub.len() > 20 { 
                                                        format!("{}...", &user.npub[..20]) 
                                                    } else { 
                                                        user.npub.clone() 
                                                    };
                                                    // Use display_name or username or fallback to generic name
                                                    let display_name = user.display_name.clone()
                                                        .or_else(|| user.username.clone())
                                                        .unwrap_or_else(|| user.name.clone());
                                                    let user_method = user.method.clone();
                                                    let has_picture = user.picture.is_some();
                                                    let picture_url = user.picture.clone().unwrap_or_default();
                                                    let avatar_letter = display_name.chars().next().unwrap_or('?').to_uppercase().to_string();
                                                    
                                                    view! {
                                                        <div class="saved-user-card">
                                                            <div class="saved-user-info">
                                                                <div class="saved-user-avatar-row">
                                                                    {if has_picture {
                                                                        view! {
                                                                            <img src={picture_url} class="saved-user-avatar" alt="avatar" />
                                                                        }.into_any()
                                                                    } else {
                                                                        view! {
                                                                            <div class="saved-user-avatar-placeholder">{avatar_letter}</div>
                                                                        }.into_any()
                                                                    }}
                                                                    <div class="saved-user-details">
                                                                        <span class="saved-user-name">{display_name}</span>
                                                                        <span class="saved-user-npub">{npub_short}</span>
                                                                        <span class="saved-user-method">{"["}{user_method}{"]"}</span>
                                                                    </div>
                                                                </div>
                                                            </div>
                                                            <div class="saved-user-actions">
                                                                <button 
                                                                    class="saved-user-connect"
                                                                    on:click={move |_| {
                                                                        let users = saved_users_signal.get().users;
                                                                        if let Some(u) = users.get(idx) {
                                                                            on_connect_saved_user(u.id.clone());
                                                                        }
                                                                    }}
                                                                    disabled={move || auth.is_loading.get()}
                                                                >
                                                                    "Connect"
                                                                </button>
                                                                <button 
                                                                    class="saved-user-delete"
                                                                    on:click={move |_| {
                                                                        let users = saved_users_signal.get().users;
                                                                        if let Some(u) = users.get(idx) {
                                                                            on_delete_saved_user(u.id.clone());
                                                                        }
                                                                    }}
                                                                >
                                                                    "×"
                                                                </button>
                                                            </div>
                                                        </div>
                                                    }
                                                }).collect::<Vec<_>>()}
                                            </div>
                                        })
                                    } else {
                                        None
                                    }
                                }}

                                <button 
                                    class="add-new-button"
                                    on:click=go_to_add_new
                                >
                                    "Add New User"
                                </button>
                            </div>
                        })
                    } else {
                        None
                    }
                }}

                // ─────────────────────────────────────────────────────────────────
                // Screen 2: Add New User (shown when show_add_new is true)
                // ─────────────────────────────────────────────────────────────────
                {move || {
                    if initialized.get() && show_add_new.get() {
                        Some(view! {
                            <div class="add-new-screen">
                                <button 
                                    class="back-button"
                                    on:click=go_to_saved_users
                                >
                                    "← Back to Saved Users"
                                </button>

                                <h2 class="screen-title">"Add New User"</h2>

                                <div class="input-group">
                                    <label class="input-label">"Relay URL"</label>
                                    <input
                                        class="input-field"
                                        type="text"
                                        placeholder="wss://relay.example.com"
                                        prop:value={move || relay.get()}
                                        on:input:target=move |ev| {
                                            relay.set(ev.target().value());
                                        }
                                        disabled={move || auth.is_loading.get()}
                                    />
                                </div>

                // Option 1: Generate nostrconnect:// URI
                <div class="nostrconnect-section">
                    <h3 class="section-title">"Option 1: Generate Connection URI"</h3>
                    <p class="section-description">"Generate a nostrconnect:// URI and paste it into your signer app (Nsec.app, Amber, etc.)"</p>

                    <button
                        class="generate-button"
                        on:click=on_generate_nostrconnect
                        disabled={move || auth.is_loading.get()}
                    >
                        {move || {
                            if auth.is_loading.get() {
                                "Generating...".to_string()
                            } else {
                                "Generate nostrconnect:// URI".to_string()
                            }
                        }}
                    </button>

                    {move || {
                        if show_generated.get() {
                            generated_uri.get().map(|uri| view! {
                                    <div class="generated-uri-box">
                                        <label class="input-label">"Copy this URI and paste into your signer app:"</label>
                                        <textarea
                                            class="uri-textarea"
                                            readonly=true
                                            prop:value={uri.clone()}
                                        />
                                        <button
                                            class="copy-button"
                                            on:click={move |_| {
                                                #[cfg(target_arch = "wasm32")]
                                                {
                                                    if let Some(window) = leptos::web_sys::window() {
                                                        let _ = window.navigator().clipboard().write_text(&uri);
                                                    }
                                                }
                                            }}
                                        >
                                            "Copy to Clipboard"
                                        </button>
                                        <button
                                            class="connect-button"
                                            on:click={move |_| {
                                                let relay_val = relay.get();
                                                let uri_val = generated_uri.get();
                                                
                                                spawn_local(async move {
                                                    // Wait for signer to connect (60 second timeout)
                                                    match invoke_wait_for_nostrconnect_signer(60).await {
                                                        Ok(npub) => {
                                                            // Auto-save user after successful login
                                                            if let Some(uri_str) = uri_val {
                                                                let save_result = invoke_add_saved_user(
                                                                    "nostrconnect".to_string(),
                                                                    Some(relay_val),
                                                                    Some(uri_str),
                                                                    None,
                                                                    npub.clone(),
                                                                ).await;
                                                                if let Err(e) = save_result {
                                                                    web_sys::console::log_1(&format!("Failed to save user: {}", e).into());
                                                                }
                                                            }
                                                            
                                                            auth.npub.set(Some(npub));
                                                            auth.is_loading.set(false);
                                                        }
                                                        Err(e) => {
                                                            auth.error.set(Some(e));
                                                            auth.is_loading.set(false);
                                                        }
                                                    }
                                                });
                                            }}
                                        >
                                            "Connect (wait for signer)"
                                        </button>
                                        <p class="instruction-text">"1. Copy the URI above and paste it into your signer app (Amber, Nsec.app, etc.)\n2. Click 'Connect' to wait for the signer to respond"</p>
                                    </div>
                                })
                        } else {
                            None
                        }
                    }}
                </div>

                <div class="divider">"── or ──"</div>

                // Option 2: Paste bunker:// URI
                <div class="bunker-section">
                    <h3 class="section-title">"Option 2: Paste Signer URI"</h3>
                    <p class="section-description">"Paste a bunker:// URI from your signer app"</p>

                    <div class="input-group">
                        <label class="input-label">"bunker:// URI"</label>
                        <input
                            class="input-field"
                            type="text"
                            placeholder="bunker://..."
                            prop:value={move || bunker_uri.get()}
                            on:input:target=move |ev| {
                                bunker_uri.set(ev.target().value());
                            }
                            disabled={move || auth.is_loading.get()}
                        />
                    </div>

                    <button
                        class="connect-button"
                        on:click=on_connect_bunker
                        disabled={move || auth.is_loading.get()}
                    >
                        {move || {
                            if auth.is_loading.get() {
                                "Connecting...".to_string()
                            } else {
                                "Connect with bunker://".to_string()
                            }
                        }}
                    </button>
                </div>

                <div class="divider">"── or ──"</div>

                // Option 3: Direct Key Input (for testing only)
                <div class="direct-key-section">
                    <h3 class="section-title">"Option 3: Test with Private Key ⚠️"</h3>
                    <p class="section-description">"Enter your nsec or hex private key directly (TESTING ONLY - NOT SECURE)"</p>

                    <div class="input-group">
                        <label class="input-label">"Private Key (nsec1... or hex)"</label>
                        <input
                            class="input-field"
                            type="password"
                            placeholder="nsec1..."
                            prop:value={move || direct_key.get()}
                            on:input:target=move |ev| {
                                direct_key.set(ev.target().value());
                            }
                            disabled={move || auth.is_loading.get()}
                        />
                    </div>

                    <button
                        class="connect-button"
                        on:click=on_connect_direct_key
                        disabled={move || auth.is_loading.get()}
                    >
                        {move || {
                            if auth.is_loading.get() {
                                "Connecting...".to_string()
                            } else {
                                "Connect with Private Key".to_string()
                            }
                        }}
                    </button>

                    <p class="warning-text">
                        "⚠️ For testing only! Your key is exposed to the application. \
                        Use NIP-46 or NIP-07 in production."
                    </p>
                </div>

                // NIP-07 section (Web only, gated by feature)
                {#[cfg(all(target_arch = "wasm32", feature = "web"))]
                view! {
                    <div class="nip07-section">
                        <div class="divider">"── or ──"</div>
                        <button
                            class="nip07-button"
                            on:click=_on_connect_nip07
                            disabled={move || auth.is_loading.get()}
                        >
                            {move || {
                                if auth.is_loading.get() {
                                    "Connecting...".to_string()
                                } else {
                                    "Connect with Browser Extension".to_string()
                                }
                            }}
                        </button>
                        <p class="nip07-hint">"NIP-07 — Alby, nos2x"</p>
                    </div>
                }}
                            </div>
                        })
                    } else {
                        None
                    }
                }}

                {move || {
                    if auth.is_loading.get() {
                        Some(view! {
                            <div class="loading-indicator">"Connecting to signer..."</div>
                        })
                    } else {
                        None
                    }
                }}

                {move || {
                    auth.error.get().map(|err| {
                        view! {
                            <div class="error-message">{err}</div>
                        }
                    })
                }}

                <p class="login-footer">
                    "Your private key never leaves your signer app."
                </p>
            </div>
        </div>
    }
}

/// Main view component - displayed when authenticated
#[component]
fn MainView(relay_count: RwSignal<usize>) -> impl IntoView {
    let auth = use_context::<AuthContext>().expect("AuthContext not provided");

    // Marketplace view state
    let current_view = RwSignal::new(MarketplaceView::Browse);

    // Handle disconnect button click
    let on_disconnect = move |_| {
        spawn_local(async move {
            match invoke_disconnect().await {
                Ok(_) => {
                    auth.npub.set(None);
                    auth.error.set(None);
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
                            Err(e) => web_sys::console::log_1(&format!("Failed to get relay count: {}", e).into()),
                        }
                    });
                }) as Box<dyn FnMut()>).into_js_value().as_ref().unchecked_ref(),
                5000
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
                    Err(e) => web_sys::console::log_1(&format!("Failed to get relay list: {}", e).into()),
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
        let p = auth.profile.get();
        #[cfg(debug_assertions)]
        {
            web_sys::console::log_1(&format!("get_profile called, profile: {:?}", p.is_some()).into());
        }
        p
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
    let get_nip05_verified = move || {
        get_profile()
            .map(|p| p.nip05_verified)
            .unwrap_or(false)
    };

    // Get picture URL
    let get_picture_url = move || {
        let url = get_profile()
            .and_then(|p| p.picture.clone());
        #[cfg(debug_assertions)]
        {
            web_sys::console::log_1(&format!("get_picture_url: {:?}", url.is_some()).into());
        }
        url
    };

    // Get first letter for avatar placeholder
    let get_avatar_letter = move || {
        let name = get_profile_display();
        let letter = name.chars().next().map(|c| c.to_uppercase().to_string()).unwrap_or_else(|| "?".to_string());
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
                            {get_profile_display()}
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

    // Check authentication status on mount (with small delay for Tauri to initialize)
    Effect::new(move |_| {
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

            match invoke_is_authenticated().await {
                Ok(true) => {
                    // User is authenticated, fetch the public key
                    match invoke_get_public_key().await {
                        Ok(npub) => {
                            auth.npub.set(Some(npub));
                        }
                        Err(_) => {
                            // Failed to get public key, treat as not authenticated
                            auth.npub.set(None);
                        }
                    }
                }
                Ok(false) => {
                    // Not authenticated
                    auth.npub.set(None);
                }
                Err(_) => {
                    // Error checking auth status
                    auth.npub.set(None);
                }
            }
        });
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
                match invoke_fetch_profile(npub.clone()).await {
                    Ok(profile) => {
                        #[cfg(debug_assertions)]
                        {
                            web_sys::console::log_1(&format!("Profile fetched: {:?}", profile.name).into());
                        }
                        auth.profile.set(Some(profile.clone()));
                        
                        // Also save profile to saved users for display on login screen
                        spawn_local(async move {
                            match invoke_fetch_and_save_user_profile().await {
                                Ok(_) => {
                                    #[cfg(debug_assertions)]
                                    {
                                        web_sys::console::log_1(&"Profile saved to saved users".into());
                                    }
                                }
                                Err(e) => {
                                    #[cfg(debug_assertions)]
                                    {
                                        web_sys::console::log_1(&format!("Failed to save profile: {}", e).into());
                                    }
                                }
                            }
                        });
                    }
                    Err(e) => {
                        #[cfg(debug_assertions)]
                        {
                            web_sys::console::log_1(&format!("Failed to fetch profile: {}", e).into());
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
                web_sys::console::log_1(&format!("DebugOverlay: Got version info: {:?}", info).into());
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
