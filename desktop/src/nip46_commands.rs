// NIP-46 Tauri Commands (IPC Layer)
//
// These are the functions the Leptos frontend calls via `invoke()`.
// Each command is the single crossing point between the untrusted WebView and the trusted Rust backend.

use std::sync::Arc;
use tokio::sync::Mutex;
use tauri::{State, AppHandle, Emitter};
use tracing::{error, info, warn};
use nostr::signer::NostrSigner;
use nostr::prelude::ToBech32;

use arcadestr_core::nip46::{
    AppSignerState,
    init_signer_session, 
    init_signer_session_fast,
    generate_login_qr,
    wait_for_qr_connection,
    save_profile_to_keyring,
    load_profile_from_keyring,
    list_profile_index,
    delete_profile_from_keyring,
    activate_profile,
    logout,
    ping_active_signer,
    get_profile_metadata_by_id,
    set_last_active_profile_id,
    cancel_bunker_retry,
    attempt_manual_reconnect,
    ProfileMetadata,
    PendingQrState,
    ConnectionState,
};

/// Called by Leptos when the user submits a bunker URI or NIP-05 address.
/// Flow A entry point - FAST VERSION (async connection).
/// On success: saves profile to keyring and activates it immediately.
/// Returns the new profile's ID and display name to the frontend.
#[tauri::command]
pub async fn connect_bunker(
    identifier: String,
    display_name: String,
    state: State<'_, Arc<Mutex<AppSignerState>>>,
    app_handle: AppHandle,
) -> Result<serde_json::Value, String> {
    info!("connect_bunker called (fast mode): identifier={}, display_name={}", identifier, display_name);

    // Parse the bunker URI to extract user_pubkey and create NostrConnectURI
    let (bunker_uri, user_pubkey) = if identifier.contains('@') {
        // NIP-05 identifier - resolve to get pubkey and relays
        match resolve_nip05_to_uri_and_pubkey(&identifier).await {
            Ok((uri, pubkey)) => (uri, pubkey),
            Err(e) => {
                error!("Failed to resolve NIP-05: {}", e);
                return Err(format!("Failed to resolve NIP-05: {}", e));
            }
        }
    } else {
        // Parse bunker URI directly
        match parse_bunker_uri(&identifier) {
            Ok((uri, pubkey)) => (uri, pubkey),
            Err(e) => {
                error!("Failed to parse bunker URI: {}", e);
                return Err(format!("Invalid bunker URI: {}", e));
            }
        }
    };

    // Initialize signer session with fast async flow (no blocking handshake)
    let (mut profile, client) = init_signer_session_fast(bunker_uri, user_pubkey)
        .await
        .map_err(|e| {
            error!("init_signer_session_fast failed: {}", e);
            e.to_string()
        })?;

    // Allow the user to override the auto-generated name
    profile.name = display_name.clone();

    // Save to keyring
    save_profile_to_keyring(&profile)
        .map_err(|e| {
            error!("save_profile_to_keyring failed: {}", e);
            e.to_string()
        })?;

    info!("Profile saved to keyring: id={}", profile.id);

    // Get bunker pubkey for state management
    let bunker_pubkey = profile.bunker_uri.remote_signer_public_key()
        .ok_or("No remote signer public key in URI")?
        .to_hex();

    // Update state with the client (connection happens in background)
    {
        let mut state_guard = state.lock().await;
        state_guard.active_client = Some(client);
        state_guard.active_profile_id = Some(bunker_pubkey.clone());
        state_guard.connection_state = ConnectionState::Connecting; // Will transition on first sign
    }

    // Set as last active profile for auto-restore on next startup
    if let Err(e) = set_last_active_profile_id(&profile.id) {
        warn!("Failed to set last active profile ID: {}", e);
    }

    // Emit auth success event immediately (fast!)
    let user_npub = profile.user_pubkey.to_bech32()
        .map_err(|e| e.to_string())?;
    let _ = app_handle.emit("auth_success", user_npub.clone());

    info!("Fast authentication complete! user_npub={}", user_npub);

    // Return profile info immediately with connection state
    Ok(serde_json::json!({
        "id": profile.id,
        "name": profile.name,
        "pubkey": user_npub,
        "pubkey_hex": profile.user_pubkey.to_hex(),
        "connection_state": "connecting",
    }))
}

/// Parse a bunker:// URI and extract the NostrConnectURI and user public key
fn parse_bunker_uri(uri_str: &str) -> Result<(nostr::nips::nip46::NostrConnectURI, nostr::PublicKey), String> {
    use nostr::nips::nip46::NostrConnectURI;
    
    let uri = NostrConnectURI::parse(uri_str)
        .map_err(|e| format!("Failed to parse bunker URI: {}", e))?;
    
    // For bunker URIs, we need to get the remote signer pubkey
    // The user_pubkey will be obtained during the actual handshake
    // For now, we use a placeholder that will be updated on first connection
    let remote_pubkey = uri.remote_signer_public_key()
        .ok_or_else(|| "No remote signer public key in bunker URI".to_string())?;
    
    // Note: The actual user pubkey will be obtained during the handshake
    // We use the remote signer pubkey as a placeholder for now
    Ok((uri, remote_pubkey))
}

/// Resolve NIP-05 identifier to NostrConnectURI and public key
async fn resolve_nip05_to_uri_and_pubkey(identifier: &str) -> Result<(nostr::nips::nip46::NostrConnectURI, nostr::PublicKey), String> {
    use nostr::nips::nip05;
    
    let address = nostr::nips::nip05::Nip05Address::parse(identifier)
        .map_err(|e| format!("Invalid NIP-05 format: {}", e))?;

    let client = reqwest::Client::new();
    let url = format!("https://{}/.well-known/nostr.json?name={}", 
        address.domain(), 
        address.name()
    );
    
    info!("Fetching NIP-05 from: {}", url);
    
    let response = client.get(&url)
        .send()
        .await
        .map_err(|e| format!("Failed to fetch NIP-05: {}", e))?;
    
    let json: serde_json::Value = response.json()
        .await
        .map_err(|e| format!("Failed to parse NIP-05 JSON: {}", e))?;

    // Extract the public key from the response
    let names = json.get("names")
        .and_then(|n| n.as_object())
        .ok_or_else(|| "No 'names' field in NIP-05 response".to_string())?;
    
    let pubkey_hex = names.get(address.name())
        .and_then(|v| v.as_str())
        .ok_or_else(|| format!("No pubkey found for '{}' in NIP-05 response", address.name()))?;
    
    let pubkey = nostr::PublicKey::from_hex(pubkey_hex)
        .map_err(|e| format!("Invalid pubkey in NIP-05 response: {}", e))?;

    // Get NIP-46 relays from the response
    let nip46_relays = json.get("nip46")
        .and_then(|n| n.as_object())
        .and_then(|n| n.get(address.name()))
        .and_then(|v| v.as_array())
        .ok_or_else(|| "No NIP-46 relays found in NIP-05 response".to_string())?;

    if nip46_relays.is_empty() {
        return Err("NIP-46 relay list is empty".to_string());
    }

    // Parse relay URLs
    let mut relays = Vec::new();
    for relay_url in nip46_relays {
        if let Some(url_str) = relay_url.as_str() {
            match url_str.parse::<nostr::types::url::RelayUrl>() {
                Ok(url) => relays.push(url),
                Err(e) => warn!("Invalid relay URL in NIP-05 response: {}", e),
            }
        }
    }

    if relays.is_empty() {
        return Err("No valid NIP-46 relays found".to_string());
    }

    // Create NostrConnectURI
    let uri = nostr::nips::nip46::NostrConnectURI::client(
        pubkey,
        relays,
        "Arcadestr",
    );

    info!("Resolved NIP-05 {} to pubkey {} with {} relays", identifier, pubkey_hex, relays.len());
    
    Ok((uri, pubkey))
}

/// Get the current NIP-46 connection status
/// Called by frontend to poll for connection state changes
#[tauri::command]
pub async fn get_connection_status(
    state: State<'_, Arc<Mutex<AppSignerState>>>,
) -> Result<serde_json::Value, String> {
    let state_guard = state.lock().await;
    
    let status = match &state_guard.connection_state {
        ConnectionState::Disconnected => "disconnected",
        ConnectionState::Connecting => "connecting",
        ConnectionState::Connected => "connected",
        ConnectionState::Failed(_) => "failed",
    };
    
    let error = match &state_guard.connection_state {
        ConnectionState::Failed(e) => Some(e.clone()),
        _ => None,
    };
    
    Ok(serde_json::json!({
        "status": status,
        "error": error,
        "has_active_client": state_guard.active_client.is_some(),
        "is_offline_mode": state_guard.is_offline_mode,
    }))
}

/// Called by Leptos to start a QR login session.
/// Flow B entry point. Returns the URI string for QR rendering.
/// The frontend renders the string as a QR code and polls for connection.
#[tauri::command]
pub async fn start_qr_login(
    state: State<'_, Arc<Mutex<AppSignerState>>>,
    _app_handle: AppHandle,
) -> Result<String, String> {
    info!("start_qr_login called");

    // Generate QR with no specific permissions requested
    let (uri_string, app_keys, secret) = generate_login_qr(None)
        .await
        .map_err(|e| {
            error!("generate_login_qr failed: {}", e);
            e.to_string()
        })?;

    // Store pending QR state in AppSignerState
    {
        let mut state_guard = state.lock().await;
        state_guard.pending_qr = Some(PendingQrState {
            uri: uri_string.clone(),
            app_keys,
            secret,
            created_at: std::time::Instant::now(),
        });
    }

    info!("QR login started, URI generated and stored in pending state");
    Ok(uri_string)
}

/// Check if a QR connection has been established.
/// Called by frontend to poll for connection status after showing QR.
/// Returns the profile info if connected, or None if still waiting.
#[tauri::command]
pub async fn check_qr_connection(
    state: State<'_, Arc<Mutex<AppSignerState>>>,
    app_handle: AppHandle,
) -> Result<Option<serde_json::Value>, String> {
    info!("check_qr_connection called");

    // Check if already processing to prevent concurrent calls
    {
        let state_guard = state.lock().await;
        if state_guard.pending_qr.is_none() {
            // No pending QR - either already connected or never started
            // Check if we have an active profile (connection succeeded)
            if state_guard.active_profile_id.is_some() {
                // Connection already established, return success
            if let Some(ref client) = state_guard.active_client {
                // Get signer from client, then get public key
                match client.signer().await {
                    Ok(signer) => {
                        match signer.get_public_key().await {
                            Ok(pubkey) => {
                                let user_npub = pubkey.to_bech32().unwrap_or_default();
                                return Ok(Some(serde_json::json!({
                                    "id": state_guard.active_profile_id.as_ref().unwrap(),
                                    "name": "QR Connected Account",
                                    "pubkey": user_npub,
                                    "pubkey_hex": pubkey.to_hex(),
                                })));
                            }
                            Err(_) => {}
                        }
                    }
                    Err(_) => {}
                }
            }
            }
            return Err("No QR login in progress. Call start_qr_login first.".to_string());
        }
    }

    // Get pending QR state
    let pending = {
        let state_guard = state.lock().await;
        state_guard.pending_qr.clone().unwrap()
    };

    // Check if QR has expired (5 minute timeout)
    if pending.created_at.elapsed().as_secs() > 300 {
        // Clear expired QR state
        let mut state_guard = state.lock().await;
        state_guard.pending_qr = None;
        return Err("QR code expired. Please generate a new one.".to_string());
    }

    // Try to complete the connection with 30 second timeout per poll
    // The frontend polls every 3 seconds, so this gives plenty of time
    match wait_for_qr_connection(&pending.uri, pending.app_keys.clone(), pending.secret.clone(), 30).await {
        Ok(profile) => {
            info!("QR connection established for profile: {}", profile.id);
            
            // Save profile to keyring
            if let Err(e) = save_profile_to_keyring(&profile) {
                error!("Failed to save QR profile to keyring: {}", e);
                return Err(format!("Connected but failed to save profile: {}", e));
            }

            // For QR profiles, we don't need to activate via bunker - connection is already established
            // Just set the active profile directly
            let mut state_guard = state.lock().await;
            state_guard.pending_qr = None; // Clear pending state
            state_guard.active_profile_id = Some(profile.id.clone());
            // Note: We don't set active_client here because QR connections don't use the NostrConnect client
            // The signer will send events directly to our relays
            drop(state_guard);

            info!("QR profile {} activated (without bunker connection)", profile.id);

            // Emit success event
            let user_npub = profile.user_pubkey.to_bech32()
                .map_err(|e| e.to_string())?;
            let _ = app_handle.emit("qr-login-complete", user_npub.clone());

            // Return profile info
            Ok(Some(serde_json::json!({
                "id": profile.id,
                "name": profile.name,
                "pubkey": user_npub,
                "pubkey_hex": profile.user_pubkey.to_hex(),
            })))
        }
        Err(e) => {
            // Connection not yet established, return None (frontend will poll again)
            info!("QR connection not yet established: {}", e);
            Ok(None)
        }
    }
}

/// Called by Leptos to list all saved profiles for the profile switcher UI.
/// Returns an array of { id, name, npub, is_current } — NO secrets.
/// Deduplicates by npub (only shows first profile for each npub).
#[tauri::command]
pub async fn list_saved_profiles(
    state: State<'_, Arc<Mutex<AppSignerState>>>,
) -> Result<serde_json::Value, String> {
    info!("list_saved_profiles called");

    // Get the currently active profile ID from state
    let active_profile_id = {
        let state_guard = state.lock().await;
        state_guard.active_profile_id.clone()
    };

    let profiles = list_profile_index();
    
    // Deduplicate by npub - use a HashMap to keep only first profile for each npub
    use std::collections::HashMap;
    let mut seen_npubs: HashMap<String, ProfileMetadata> = HashMap::new();
    
    for profile in profiles {
        // Only keep the first profile for each npub
        seen_npubs.entry(profile.pubkey_bech32.clone()).or_insert(profile);
    }
    
    // Convert back to vec
    let unique_profiles: Vec<ProfileMetadata> = seen_npubs.into_values().collect();
    
    // Return in the format the frontend expects (matching old API)
    let accounts_list: Vec<serde_json::Value> = unique_profiles
        .into_iter()
        .map(|p| {
            // Check if this profile is currently active
            let is_current = active_profile_id.as_ref() == Some(&p.id);
            serde_json::json!({
                "id": p.id,
                "name": p.name,
                "npub": p.pubkey_bech32,
                "pubkey_hex": p.pubkey_hex,
                "signing_mode": "nip46", // Default to nip46 for NIP-46 profiles
                "last_used": 0, // We don't track this currently
                "is_current": is_current,
            })
        })
        .collect();

    info!("Returning {} unique profiles", accounts_list.len());
    Ok(serde_json::json!({
        "accounts": accounts_list,
    }))
}

/// Switch the active session to a different saved profile.
#[tauri::command]
pub async fn switch_profile(
    profile_id: String,
    state: State<'_, Arc<Mutex<AppSignerState>>>,
    app_handle: AppHandle,
) -> Result<serde_json::Value, String> {
    info!("switch_profile called: profile_id={}", profile_id);

    // STEP 1: Cancel any existing bunker retry task
    cancel_bunker_retry(state.inner()).await;

    // Get the profile metadata to find the bunker pubkey
    let metadata = match get_profile_metadata_by_id(&profile_id) {
        Some(m) => m,
        None => {
            error!("Profile metadata {} not found in index", profile_id);
            return Err(format!("Profile not found: {}", profile_id));
        }
    };

    // Determine which key to use for loading the profile
    let key_to_use = if metadata.bunker_pubkey_hex.is_empty() {
        // Old profile - use profile_id
        profile_id.clone()
    } else {
        // New profile - use bunker_pubkey_hex
        metadata.bunker_pubkey_hex.clone()
    };

    // Load the full profile using the determined key
    let profile = match load_profile_from_keyring(&key_to_use) {
        Some(p) => p,
        None => {
            error!("Profile {} not found in keyring (key: {})", profile_id, key_to_use);
            return Err(format!("Profile not found: {}", profile_id));
        }
    };

    // Check if this is a QR profile (name contains "QR Connected")
    let is_qr_profile = profile.name.contains("QR Connected");
    
    if is_qr_profile {
        // For QR profiles, just set as active without bunker connection
        info!("QR profile detected, activating without bunker connection");
        let mut state_guard = state.lock().await;
        state_guard.active_profile_id = Some(profile_id.clone());
        state_guard.is_offline_mode = false;
        // Note: active_client remains None for QR profiles
        drop(state_guard);
        
        // Set as last active
        let _ = set_last_active_profile_id(&profile_id);
        
        let user_npub = match profile.user_pubkey.to_bech32() {
            Ok(npub) => npub,
            Err(e) => return Err(format!("Failed to encode pubkey: {}", e)),
        };
        let _ = app_handle.emit("auth_success", user_npub.clone());
        
        return Ok(serde_json::json!({
            "account": {
                "id": profile_id,
                "npub": user_npub,
                "name": profile.name,
                "signing_mode": "nip46",
                "last_used": 0,
            }
        }));
    }

    // For bunker profiles, use the normal activation flow
    // Use the key_to_use (bunker_pubkey_hex for new profiles, profile_id for old)
    activate_profile(state.inner(), &key_to_use)
        .await
        .map_err(|e| {
            error!("activate_profile failed: {}", e);
            e.to_string()
        })?;

    // Set as last active profile
    if let Err(e) = set_last_active_profile_id(&profile_id) {
        warn!("Failed to set last active profile ID: {}", e);
    }

    // Get the activated profile info
    let state_guard = state.lock().await;
    if let Some(ref client) = state_guard.active_client {
        // Get signer from client, then get public key
        match client.signer().await {
            Ok(signer) => {
                match signer.get_public_key().await {
                    Ok(pubkey) => {
                        let user_npub = pubkey.to_bech32().unwrap_or_default();
                        let _ = app_handle.emit("auth_success", user_npub.clone());
                        // Return in format frontend expects (wrapped in "account" field)
                        Ok(serde_json::json!({
                            "account": {
                                "id": profile_id,
                                "npub": user_npub,
                                "name": null,
                                "signing_mode": "nip46",
                                "last_used": 0,
                            }
                        }))
                    }
                    Err(e) => Err(format!("Failed to get public key: {}", e)),
                }
            }
            Err(e) => Err(format!("Failed to get signer: {}", e)),
        }
    } else {
        Err("Failed to activate profile - no active client".to_string())
    }
}

/// Attempt to manually reconnect to the bunker.
/// This is called when the user clicks the "reconnect" button in offline mode.
#[tauri::command]
pub async fn attempt_reconnect(
    state: State<'_, Arc<Mutex<AppSignerState>>>,
    app_handle: AppHandle,
) -> Result<serde_json::Value, String> {
    info!("attempt_reconnect called");

    match attempt_manual_reconnect(state.inner()).await {
        Ok(_) => {
            info!("Manual reconnect successful");
            
            // Get the user pubkey
            let state_guard = state.lock().await;
            if let Some(ref client) = state_guard.active_client {
                match client.signer().await {
                    Ok(signer) => {
                        match signer.get_public_key().await {
                            Ok(pubkey) => {
                                let user_npub = pubkey.to_bech32().unwrap_or_default();
                                let _ = app_handle.emit("bunker_reconnected", user_npub.clone());
                                Ok(serde_json::json!({
                                    "success": true,
                                    "npub": user_npub,
                                }))
                            }
                            Err(e) => Err(format!("Connected but failed to get pubkey: {}", e)),
                        }
                    }
                    Err(e) => Err(format!("Connected but failed to get signer: {}", e)),
                }
            } else {
                Ok(serde_json::json!({ "success": true }))
            }
        }
        Err(e) => {
            error!("Manual reconnect failed: {}", e);
            Err(format!("Reconnect failed: {}", e))
        }
    }
}

/// Delete a saved profile permanently (removes from keyring).
/// If it is the active profile, logout first.
#[tauri::command]
pub async fn delete_profile(
    profile_id: String,
    state: State<'_, Arc<Mutex<AppSignerState>>>,
) -> Result<(), String> {
    info!("delete_profile called: profile_id={}", profile_id);

    // Get the profile metadata to find the bunker pubkey
    let metadata = match get_profile_metadata_by_id(&profile_id) {
        Some(m) => m,
        None => {
            error!("Profile metadata {} not found in index", profile_id);
            return Err(format!("Profile not found: {}", profile_id));
        }
    };

    // Check if this is the active profile (compare by profile_id)
    // For old profiles without bunker_pubkey_hex, we use the profile_id
    let key_to_delete = if metadata.bunker_pubkey_hex.is_empty() {
        // Old profile - use profile_id as the key
        profile_id.clone()
    } else {
        // New profile - use bunker_pubkey_hex as the key
        metadata.bunker_pubkey_hex.clone()
    };
    
    {
        let state_guard = state.lock().await;
        // Check if active_profile_id matches either the bunker pubkey or the profile id
        let is_active = state_guard.active_profile_id.as_ref() == Some(&key_to_delete) ||
                       state_guard.active_profile_id.as_ref() == Some(&profile_id);
        if is_active {
            info!("Profile {} is active, logging out first", profile_id);
            drop(state_guard);
            logout(state.inner()).await;
        }
    }

    delete_profile_from_keyring(&key_to_delete)
        .map_err(|e| {
            error!("delete_profile_from_keyring failed: {}", e);
            e.to_string()
        })
}

/// Sign and publish a game event (example usage from game logic).
/// This demonstrates how all signing is transparently routed to the bunker.
#[tauri::command]
pub async fn publish_game_score(
    score: u64,
    state: State<'_, Arc<Mutex<AppSignerState>>>,
) -> Result<String, String> {
    let state_guard = state.lock().await;
    
    let _client = state_guard
        .active_client
        .as_ref()
        .ok_or("No active session. Please log in first.")?;

    // The client knows it holds a NostrConnect internally.
    // To sign an event, we would:
    //   1. Create an EventBuilder for the game score
    //   2. Use the client to sign it (which sends NIP-46 request to bunker)
    //   3. Return the event ID
    
    // For now, return a placeholder since we need the full event builder integration
    info!("publish_game_score called with score: {}", score);
    
    // TODO: Implement full event signing via NIP-46
    // let builder = nostr::EventBuilder::text_note(format!("I scored {} on Arcadestr!", score));
    // let event = client.sign_event_builder(builder).await?;
    
    Ok(format!("score_{}_published", score))
}

/// Ping the active bunker to check connection health.
/// Emits "bunker-heartbeat" event.
#[tauri::command]
pub async fn ping_bunker(
    state: State<'_, Arc<Mutex<AppSignerState>>>,
    app_handle: AppHandle,
) -> Result<serde_json::Value, String> {
    let is_alive = ping_active_signer(state.inner()).await;
    
    let payload = serde_json::json!({
        "alive": is_alive,
    });
    
    let _ = app_handle.emit("bunker-heartbeat", payload.clone());
    
    Ok(payload)
}

/// Logout from current session.
#[tauri::command]
pub async fn logout_nip46(
    state: State<'_, Arc<Mutex<AppSignerState>>>,
    app_handle: AppHandle,
) -> Result<(), String> {
    info!("logout_nip46 called");
    logout(state.inner()).await;
    
    // Emit logout event to notify frontend
    let _ = app_handle.emit("auth_logout", ());
    info!("Emitted auth_logout event");
    
    Ok(())
}

/// Check if any saved profiles exist (for startup check)
#[tauri::command]
pub async fn has_accounts() -> Result<bool, String> {
    info!("has_accounts called");
    let profiles = list_profile_index();
    Ok(!profiles.is_empty())
}

/// Load the currently active account if one exists
/// Returns the active profile info or error if none is active
#[tauri::command]
pub async fn load_active_account(
    state: State<'_, Arc<Mutex<AppSignerState>>>,
) -> Result<serde_json::Value, String> {
    info!("load_active_account called");
    
    let state_guard = state.lock().await;
    
    // Check if we have an active profile
    if let Some(ref profile_id) = state_guard.active_profile_id {
        // Try to get the client and fetch pubkey
        if let Some(ref client) = state_guard.active_client {
            // Get signer from client, then get public key
            match client.signer().await {
                Ok(signer) => {
                    match signer.get_public_key().await {
                        Ok(pubkey) => {
                            let user_npub = pubkey.to_bech32().unwrap_or_default();
                            // Return in format frontend expects (wrapped in "account" field)
                            return Ok(serde_json::json!({
                                "account": {
                                    "id": profile_id,
                                    "npub": user_npub,
                                    "name": null,
                                    "signing_mode": "nip46",
                                    "last_used": 0,
                                }
                            }));
                        }
                        Err(e) => {
                            warn!("Failed to get public key from active client: {}", e);
                            // Return profile without verifying connection
                            return Ok(serde_json::json!({
                                "account": {
                                    "id": profile_id,
                                    "npub": null,
                                    "name": null,
                                    "signing_mode": "nip46",
                                    "last_used": 0,
                                }
                            }));
                        }
                    }
                }
                Err(e) => {
                    warn!("Failed to get signer from active client: {}", e);
                    // Return profile without verifying connection
                    return Ok(serde_json::json!({
                        "account": {
                            "id": profile_id,
                            "npub": null,
                            "name": null,
                            "signing_mode": "nip46",
                            "last_used": 0,
                        }
                    }));
                }
            }
        }
    }
    
    // No active account
    Err("No active account".to_string())
}
