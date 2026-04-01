// NIP-46 Session Management
//
// This module manages the lifecycle of the active session: switching profiles, logout, and heartbeat monitoring.

use nostr::signer::NostrSigner;
use nostr_sdk::Client;
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::time::{interval, Duration};
use tracing::{error, info, warn};

use crate::nip46::types::AppSignerState;
use crate::nip46::types::session_config::{BUNKER_RETRY_INTERVAL_SECS, BUNKER_CONNECT_TIMEOUT_SECS};
use crate::nip46::types::ConnectionState;
use crate::nip46::storage::{load_profile_from_keyring, set_last_active_profile_id, clear_last_active_profile_id, get_last_active_profile_id};
use crate::signers::LazyNip46Signer;

/// Activate a previously saved profile by bunker pubkey.
/// This is called when the user selects a profile from the saved list.
/// FAST VERSION: Uses deferred connection (handshake on first sign).
///
/// Steps:
///   1. Load the SavedProfile from the keyring by bunker pubkey
///   2. Drop + disconnect the current active_client (if any)
///   3. Create LazyNip46Signer with deferred connection
///   4. Build nostr-sdk Client with lazy signer
///   5. Update AppSignerState immediately (connection happens in background)
///
/// # Arguments
/// * `state` - The application state containing active session
/// * `bunker_pubkey` - The bunker pubkey (hex) of the profile to activate
pub async fn activate_profile(
    state: &Arc<Mutex<AppSignerState>>,
    bunker_pubkey: &str,
) -> anyhow::Result<()> {
    info!("Activating profile (fast mode) for bunker pubkey: {}", bunker_pubkey);

    // STEP 1: Load profile from keyring using bunker pubkey
    let profile = load_profile_from_keyring(bunker_pubkey)
        .ok_or_else(|| anyhow::anyhow!("Profile with bunker pubkey {} not found in keyring", bunker_pubkey))?;

    // STEP 2: Drop old active_client by setting it to None
    {
        let mut state_guard = state.lock().await;
        if state_guard.active_client.is_some() {
            info!("Dropping previous active client");
            state_guard.active_client = None;
            state_guard.active_profile_id = None;
        }
    }

    // STEP 3: Create LazyNip46Signer (deferred connection - no blocking handshake)
    info!("Creating LazyNip46Signer for bunker pubkey {}...", bunker_pubkey);
    let lazy_signer = LazyNip46Signer::new(
        profile.bunker_uri.clone(),
        profile.app_keys.clone(),
        profile.user_pubkey,
    );

    // STEP 4: Build nostr-sdk Client with lazy signer
    info!("Building nostr-sdk Client with LazyNip46Signer (deferred connection)...");
    let client = Client::new(Arc::new(lazy_signer) as Arc<dyn nostr::NostrSigner>);
    info!("Client built successfully (deferred connection)");

    // STEP 5: Update AppSignerState immediately
    {
        let mut state_guard = state.lock().await;
        state_guard.active_client = Some(client.clone());
        state_guard.active_profile_id = Some(bunker_pubkey.to_string());
        state_guard.connection_state = ConnectionState::Connecting;
    }

    // STEP 6: Trigger connection in background
    // This ensures the NIP-46 handshake happens without blocking the UI
    let state_clone = state.clone();
    tokio::spawn(async move {
        info!("Triggering deferred NIP-46 connection in background...");
        // Get the signer from client and trigger connection by calling get_public_key
        match client.signer().await {
            Ok(signer) => {
                match signer.get_public_key().await {
                    Ok(_) => {
                        info!("NIP-46 connection established successfully in background");
                        // Update state to connected
                        let mut state_guard = state_clone.lock().await;
                        state_guard.connection_state = ConnectionState::Connected;
                    }
                    Err(e) => {
                        error!("Failed to establish NIP-46 connection: {}", e);
                        let mut state_guard = state_clone.lock().await;
                        state_guard.connection_state = ConnectionState::Failed(e.to_string());
                    }
                }
            }
            Err(e) => {
                error!("Failed to get signer from client: {}", e);
                let mut state_guard = state_clone.lock().await;
                state_guard.connection_state = ConnectionState::Failed(e.to_string());
            }
        }
    });

    info!("Profile {} activated successfully (fast mode): user_pubkey={}", 
        bunker_pubkey, profile.user_pubkey.to_hex());

    Ok(())
}

/// Result type for session restoration attempts
#[derive(Debug, Clone)]
pub enum SessionRestoreResult {
    /// Successfully restored and connected to bunker
    Success,
    /// Bunker unreachable, entered offline mode
    OfflineMode,
    /// No saved session to restore
    NoSession,
    /// Other error occurred
    Failed(String),
}

/// Restore the last active session on app startup.
/// 
/// This function:
/// 1. Checks for a last active profile ID in the keyring
/// 2. Loads the profile from keyring
/// 3. Creates LazyNip46Signer with deferred connection (no blocking handshake)
/// 4. Updates state immediately with connection_state = Connecting
///
/// # Arguments
/// * `state` - The application state
///
/// # Returns
/// SessionRestoreResult indicating the outcome
pub async fn restore_session_on_startup(
    state: &Arc<Mutex<AppSignerState>>,
) -> SessionRestoreResult {
    info!("Attempting to restore session on startup (fast mode)...");

    // STEP 1: Get last active profile ID
    let profile_id = match get_last_active_profile_id() {
        Some(id) => id,
        None => {
            info!("No last active profile found");
            return SessionRestoreResult::NoSession;
        }
    };

    info!("Found last active profile ID: {}", profile_id);

    // STEP 2: Load profile metadata to get bunker pubkey
    let metadata = match crate::nip46::storage::get_profile_metadata_by_id(&profile_id) {
        Some(m) => m,
        None => {
            warn!("Profile metadata not found for ID: {}", profile_id);
            return SessionRestoreResult::NoSession;
        }
    };

    // Determine which key to use for loading the full profile
    let key_to_use = if metadata.bunker_pubkey_hex.is_empty() {
        // Old profile - use profile_id
        profile_id.clone()
    } else {
        // New profile - use bunker_pubkey_hex
        metadata.bunker_pubkey_hex.clone()
    };

    // STEP 3: Load full profile from keyring
    let profile = match load_profile_from_keyring(&key_to_use) {
        Some(p) => p,
        None => {
            warn!("Profile not found in keyring for key: {}", key_to_use);
            return SessionRestoreResult::NoSession;
        }
    };

    // STEP 4: Create LazyNip46Signer with deferred connection (fast!)
    info!("Creating LazyNip46Signer for auto-restore (deferred connection)...");
    let lazy_signer = LazyNip46Signer::new(
        profile.bunker_uri.clone(),
        profile.app_keys.clone(),
        profile.user_pubkey,
    );

    // STEP 5: Build Client immediately without waiting for handshake
    info!("Building nostr-sdk Client with LazyNip46Signer...");
    let client = Client::new(Arc::new(lazy_signer) as Arc<dyn nostr::NostrSigner>);
    info!("Client built successfully for bunker pubkey: {}", key_to_use);
    
    // Update state
    {
        let mut state_guard = state.lock().await;
        state_guard.active_client = Some(client.clone());
        state_guard.active_profile_id = Some(profile_id);
        state_guard.is_offline_mode = false;
        state_guard.connection_state = ConnectionState::Connecting;
    }
    
    // STEP 6: Trigger connection in background
    let state_clone = state.clone();
    tokio::spawn(async move {
        info!("Triggering deferred NIP-46 connection in background for restored session...");
        match client.signer().await {
            Ok(signer) => {
                match signer.get_public_key().await {
                    Ok(_) => {
                        info!("NIP-46 connection established successfully for restored session");
                        let mut state_guard = state_clone.lock().await;
                        state_guard.connection_state = ConnectionState::Connected;
                    }
                    Err(e) => {
                        error!("Failed to establish NIP-46 connection for restored session: {}", e);
                        let mut state_guard = state_clone.lock().await;
                        state_guard.connection_state = ConnectionState::Failed(e.to_string());
                    }
                }
            }
            Err(e) => {
                error!("Failed to get signer from client for restored session: {}", e);
                let mut state_guard = state_clone.lock().await;
                state_guard.connection_state = ConnectionState::Failed(e.to_string());
            }
        }
    });
    
    SessionRestoreResult::Success
}

/// Start a periodic task to retry bunker connection.
/// This runs every 30 seconds (configurable) until connection succeeds.
fn start_bunker_retry_task(
    state: Arc<Mutex<AppSignerState>>,
    profile: crate::nip46::types::SavedProfile,
) -> tokio::task::AbortHandle
{
    let mut interval = interval(Duration::from_secs(BUNKER_RETRY_INTERVAL_SECS));
    
    let task = tokio::spawn(async move {
        loop {
            interval.tick().await;
            
            info!("Periodic retry: Testing bunker connection...");
            
            // Create LazyNip46Signer and test connection immediately
            let lazy_signer = LazyNip46Signer::new(
                profile.bunker_uri.clone(),
                profile.app_keys.clone(),
                profile.user_pubkey,
            );
            
            // Test connection by calling get_public_key (this triggers handshake)
            match lazy_signer.get_public_key().await {
                Ok(user_pubkey) => {
                    info!("Retry successful! Reconnected to bunker: {}", user_pubkey.to_hex());
                    
                    // Build Client with the now-connected signer
                    let client = Client::new(Arc::new(lazy_signer) as Arc<dyn nostr::NostrSigner>);
                    
                    // Update state to online
                    {
                        let mut state_guard = state.lock().await;
                        state_guard.active_client = Some(client);
                        state_guard.is_offline_mode = false;
                        state_guard.connection_state = ConnectionState::Connected;
                        state_guard.bunker_retry_handle = None; // Clear the handle
                    }
                    
                    // Task completes successfully - stop retrying
                    break;
                }
                Err(e) => {
                    warn!("Retry failed: Bunker still unreachable: {}", e);
                    // Continue loop - will retry after interval
                }
            }
        }
    });
    
    task.abort_handle()
}

/// Cancel any ongoing bunker retry task.
/// Call this before switching to a different profile.
pub async fn cancel_bunker_retry(state: &Arc<Mutex<AppSignerState>>) {
    let mut state_guard = state.lock().await;
    if let Some(handle) = state_guard.bunker_retry_handle.take() {
        info!("Canceling bunker retry task");
        handle.abort();
    }
}

/// Attempt to manually reconnect to the bunker.
/// This is called when the user clicks the "reconnect" button in offline mode.
pub async fn attempt_manual_reconnect(
    state: &Arc<Mutex<AppSignerState>>,
) -> Result<(), String>
{
    info!("Manual reconnect attempt...");
    
    // Get the current profile ID
    let profile_id = {
        let state_guard = state.lock().await;
        match &state_guard.active_profile_id {
            Some(id) => id.clone(),
            None => return Err("No active profile to reconnect".to_string()),
        }
    };
    
    // Get metadata to find bunker pubkey
    let metadata = match crate::nip46::storage::get_profile_metadata_by_id(&profile_id) {
        Some(m) => m,
        None => return Err("Profile metadata not found".to_string()),
    };
    
    let key_to_use = if metadata.bunker_pubkey_hex.is_empty() {
        profile_id.clone()
    } else {
        metadata.bunker_pubkey_hex.clone()
    };
    
    // Load profile
    let profile = match load_profile_from_keyring(&key_to_use) {
        Some(p) => p,
        None => return Err("Profile not found in keyring".to_string()),
    };
    
    // Create LazyNip46Signer and test connection immediately
    let lazy_signer = LazyNip46Signer::new(
        profile.bunker_uri.clone(),
        profile.app_keys.clone(),
        profile.user_pubkey,
    );
    
    // Test connection by calling get_public_key (this triggers handshake)
    match lazy_signer.get_public_key().await {
        Ok(user_pubkey) => {
            info!("Manual reconnect successful: {}", user_pubkey.to_hex());
            
            let client = Client::new(Arc::new(lazy_signer) as Arc<dyn nostr::NostrSigner>);
            
            {
                let mut state_guard = state.lock().await;
                state_guard.active_client = Some(client);
                state_guard.is_offline_mode = false;
                state_guard.connection_state = ConnectionState::Connected;
            }
            
            Ok(())
        }
        Err(e) => Err(format!("Failed to connect: {}", e)),
    }
}

/// Send a NIP-46 "ping" to the active bunker and await "pong".
/// Returns true if the bunker is alive, false otherwise.
///
/// Reference: Amethyst implements this as a "bunker heartbeat indicator" in LoginViewModel.
pub async fn ping_active_signer(
    state: &Arc<Mutex<AppSignerState>>,
) -> bool {
    let state_guard = state.lock().await;
    
    match &state_guard.active_client {
        Some(client) => {
            // Clone the client to avoid holding the lock across await
            let client_clone = client.clone();
            drop(state_guard);
            
            // Try to get signer and call get_public_key as a ping test
            match client_clone.signer().await {
                Ok(signer) => {
                    match signer.get_public_key().await {
                        Ok(pubkey) => {
                            info!("Bunker ping successful: {}", pubkey.to_hex());
                            true
                        }
                        Err(e) => {
                            warn!("Bunker ping failed: {}", e);
                            false
                        }
                    }
                }
                Err(e) => {
                    warn!("No signer available for ping: {}", e);
                    false
                }
            }
        }
        None => {
            // No active client
            false
        }
    }
}

/// Log out of the current active session.
/// Drops the WebSocket client but does NOT delete the saved profile from keyring.
pub async fn logout(state: &Arc<Mutex<AppSignerState>>) {
    info!("Logging out of current session...");
    
    // Cancel any ongoing retry task
    cancel_bunker_retry(state).await;
    
    let mut state_guard = state.lock().await;
    
    if state_guard.active_client.is_some() {
        info!("Dropping active client (WebSocket connections will close)");
        state_guard.active_client = None;
        state_guard.active_profile_id = None;
        state_guard.is_offline_mode = false;
    } else {
        info!("No active client to drop");
    }
    
    // Clear the last active profile ID
    clear_last_active_profile_id();
}
