// Desktop entry point: Tauri v2 application shell with NOSTR auth and listing commands.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::{info, error};

#[allow(unused_imports)]
use arcadestr_core::signer::NostrSigner;

use arcadestr_core::auth::AuthState;
use arcadestr_core::lightning::{request_zap_invoice, ZapInvoice, ZapRequest};
use arcadestr_core::nostr::{GameListing, NostrClient, DEFAULT_RELAYS};
use nostr::nips::nip46::NostrConnectURI;
use nostr::prelude::ToBech32;

/// Application state shared across Tauri commands.
pub struct AppState {
    /// Authentication state wrapped in Arc<Mutex<>> for thread-safe access.
    pub auth: Arc<Mutex<AuthState>>,
    /// NOSTR client for relay communication.
    pub nostr: Arc<Mutex<NostrClient>>,
}

/// Generates a nostrconnect:// URI for client-initiated NIP-46 connections.
///
/// This creates a URI that users can paste into their signer app (Nsec.app, Amber, etc.)
/// to establish a connection. The client keys are stored in state for later use.
///
/// # Arguments
/// * `relay` - The relay URL where the client will listen for responses
/// * `state` - The application state to store pending connection
///
/// # Returns
/// A JSON object containing the nostrconnect URI and the client pubkey.
#[tauri::command]
async fn generate_nostrconnect_uri(
    relay: String,
    state: tauri::State<'_, AppState>,
) -> Result<String, String> {
    use arcadestr_core::signer::Nip46Signer;
    use nostr::nips::nip19::ToBech32;
    use tracing::info;

    info!("generate_nostrconnect_uri called with relay: {}", relay);

    // Generate nostrconnect URI using the library method (matching working implementation)
    // Note: We pass empty secret/perms as the library generates these automatically
    let result = Nip46Signer::generate_nostrconnect_uri(&relay, "", None, Some("Arcadestr"));
    
    let (uri, client_keys) = match result {
        Ok(ok) => ok,
        Err(e) => {
            error!("generate_nostrconnect_uri failed: {:?}", e);
            return Err(format!("Failed to generate URI: {}", e));
        }
    };

    info!("URI generated successfully, client pubkey: {}", client_keys.public_key().to_hex());

    // Store the client keys in state for later connection
    // IMPORTANT: Must preserve these keys - signers associate approvals with specific client pubkeys
    {
        let mut auth = state.auth.lock().await;
        // Note: The library generates its own secret, we extract it from the URI if needed
        // For now, store without explicit secret as the library handles this internally
        auth.set_pending_nostrconnect(client_keys.clone(), relay.clone(), "".to_string());
    }

    let response = serde_json::json!({
        "uri": uri,
        "client_pubkey": client_keys.public_key().to_bech32().map_err(|e| e.to_string())?,
        "relay": relay,
    });

    Ok(response.to_string())
}

/// Connects to a NIP-46 signer using the provided URI and relay.
///
/// # Arguments
/// * `uri` - The NIP-46 connection URI (nostrconnect:// or bunker://)
/// * `relay` - The relay URL to use for communication
///
/// # Returns
/// The public key as a bech32 npub string on success.
#[tauri::command]
async fn connect_nip46(
    uri: String,
    relay: String,
    state: tauri::State<'_, AppState>,
) -> Result<String, String> {
    use tracing::{error, info};

    info!("Connecting via NIP-46...");
    info!("URI: {}", uri);
    info!("Relay: {}", relay);

    let mut auth = state.auth.lock().await;

    match auth.connect_nip46(&uri, &relay).await {
        Ok(_) => {
            info!("NIP-46 connection successful");
        }
        Err(e) => {
            error!("NIP-46 connection failed: {}", e);
            return Err(format!("Connection failed: {}", e));
        }
    }

    // Get the public key and convert to bech32 npub
    let pubkey = auth
        .public_key()
        .ok_or_else(|| "Public key not available after connection".to_string())?;

    let npub = pubkey.to_bech32().map_err(|e| e.to_string())?;
    
    info!("NIP-46 connection successful, npub: {}", npub);
    Ok(npub)
}

/// Connects with a raw private key for testing purposes.
///
/// ⚠️ WARNING: This is for testing only! Use NIP-46 or NIP-07 in production
/// to keep your private key secure.
///
/// # Arguments
/// * `key` - The private key as nsec1... string or hex string
///
/// # Returns
/// The public key as a bech32 npub string on success.
#[tauri::command]
async fn connect_with_key(
    key: String,
    state: tauri::State<'_, AppState>,
) -> Result<String, String> {
    use tracing::{error, info};

    info!("Connecting with direct key...");
    info!("Key length: {} chars", key.len());

    let mut auth = state.auth.lock().await;

    match auth.connect_with_key(&key) {
        Ok(_) => {
            info!("Direct key authentication successful");
        }
        Err(e) => {
            error!("Direct key authentication failed: {}", e);
            return Err(format!(
                "Failed to authenticate with provided key. \
                Make sure you're entering a valid nsec1... key or hex private key. \
                Error: {}",
                e
            ));
        }
    }

    // Get the public key and convert to bech32 npub
    let pubkey = auth
        .public_key()
        .ok_or_else(|| "Public key not available after authentication".to_string())?;

    pubkey.to_bech32().map_err(|e| e.to_string())
}

/// Waits for a nostrconnect:// signer to connect.
///
/// This should be called after the user has pasted the nostrconnect:// URI into their signer app.
/// It waits for the signer to connect via the relay and completes the handshake.
///
/// # Arguments
/// * `timeout_secs` - How long to wait for the signer to connect (default: 60)
/// * `state` - The application state containing pending connection
///
/// # Returns
/// The public key as a bech32 npub string on success.
#[tauri::command]
async fn wait_for_nostrconnect_signer(
    timeout_secs: u64,
    state: tauri::State<'_, AppState>,
) -> Result<String, String> {
    use arcadestr_core::signer::Nip46Signer;
    use nostr::nips::nip19::ToBech32;

    let mut auth = state.auth.lock().await;

    // Check if we have pending nostrconnect credentials
    let (client_keys, relay, _secret) = auth
        .take_pending_nostrconnect()
        .ok_or("No pending nostrconnect connection. Generate a URI first.")?;

    // Build the URI from stored credentials (matching working implementation)
    let uri = NostrConnectURI::client(
        client_keys.public_key(),
        vec![relay.parse().map_err(|e| format!("Invalid relay: {}", e))?],
        "Arcadestr",
    );

    // Wait for the signer to connect (returns both signer and public key)
    let (signer, public_key) =
        Nip46Signer::wait_for_nostrconnect_signer(uri, client_keys, timeout_secs)
            .await
            .map_err(|e| e.to_string())?;

    // Store the signer and public key
    auth.set_nip46_signer(signer);
    auth.set_public_key(public_key);

    // Return the npub
    auth.public_key()
        .ok_or_else(|| "Public key not available after connection".to_string())?
        .to_bech32()
        .map_err(|e| e.to_string())
}

/// Reconnects to default relays.
/// Useful if relays were down during app startup.
#[tauri::command]
async fn reconnect_relays(state: tauri::State<'_, AppState>) -> Result<String, String> {
    use arcadestr_core::nostr::DEFAULT_RELAYS;
    use tracing::{error, info};

    info!("Reconnecting to relays...");
    let nostr = state.nostr.lock().await;

    for relay in DEFAULT_RELAYS {
        match nostr.add_relay(relay).await {
            Ok(added) => {
                if added {
                    info!("Connected to relay: {}", relay);
                } else {
                    info!("Relay already connected: {}", relay);
                }
            }
            Err(e) => error!("Failed to connect to relay {}: {}", relay, e),
        }
    }

    nostr.connect().await;
    Ok("Relays reconnected".to_string())
}

/// Returns the authenticated user's public key as a bech32 npub string.
///
/// # Returns
/// The npub string if authenticated, or an error if not authenticated.
#[tauri::command]
async fn get_public_key(state: tauri::State<'_, AppState>) -> Result<String, String> {
    let auth = state.auth.lock().await;

    let pubkey = auth
        .public_key()
        .ok_or_else(|| "Not authenticated".to_string())?;

    pubkey.to_bech32().map_err(|e| e.to_string())
}

/// Checks if the user is currently authenticated.
///
/// # Returns
/// `true` if authenticated, `false` otherwise.
#[tauri::command]
async fn is_authenticated(state: tauri::State<'_, AppState>) -> Result<bool, String> {
    let auth = state.auth.lock().await;
    Ok(auth.is_authenticated())
}

/// Disconnects the current signer and clears the authentication state.
#[tauri::command]
async fn disconnect(state: tauri::State<'_, AppState>) -> Result<(), String> {
    let mut auth = state.auth.lock().await;
    auth.disconnect();
    Ok(())
}

/// Publishes a game listing as a signed NOSTR event.
///
/// # Arguments
/// * `listing` - The game listing to publish
///
/// # Returns
/// The event ID as a hex string on success.
#[tauri::command]
async fn publish_listing(
    listing: GameListing,
    state: tauri::State<'_, AppState>,
) -> Result<String, String> {
    // Clone auth state before dropping the lock to avoid holding across await
    let auth_snapshot = {
        let auth = state.auth.lock().await;
        auth.clone()
    };

    let nostr = state.nostr.lock().await;

    nostr
        .publish_listing(&listing, &auth_snapshot)
        .await
        .map(|id| id.to_hex())
        .map_err(|e| e.to_string())
}

/// Fetches recent game listings from relays.
///
/// # Arguments
/// * `limit` - Maximum number of listings to fetch
///
/// # Returns
/// A vector of game listings on success.
#[tauri::command]
async fn fetch_listings(
    limit: usize,
    state: tauri::State<'_, AppState>,
) -> Result<Vec<GameListing>, String> {
    let nostr = state.nostr.lock().await;

    nostr.fetch_listings(limit).await.map_err(|e| e.to_string())
}

/// Fetches a specific game listing by its ID and publisher.
///
/// # Arguments
/// * `publisher_npub` - The bech32 npub of the publisher
/// * `listing_id` - The unique ID of the listing (d-tag value)
///
/// # Returns
/// The game listing on success.
#[tauri::command]
async fn fetch_listing_by_id(
    publisher_npub: String,
    listing_id: String,
    state: tauri::State<'_, AppState>,
) -> Result<GameListing, String> {
    let nostr = state.nostr.lock().await;

    nostr
        .fetch_listing_by_id(&publisher_npub, &listing_id)
        .await
        .map_err(|e| e.to_string())
}

/// Requests a Lightning invoice for a zap payment.
///
/// # Arguments
/// * `zap_request` - The zap request parameters
///
/// # Returns
/// The zap invoice containing the bolt11 invoice string.
#[tauri::command]
async fn request_invoice(
    zap_request: ZapRequest,
    state: tauri::State<'_, AppState>,
) -> Result<ZapInvoice, String> {
    // Clone auth state before dropping the lock to avoid holding across await
    let auth_snapshot = {
        let auth = state.auth.lock().await;
        auth.clone()
    };

    request_zap_invoice(&zap_request, &auth_snapshot)
        .await
        .map_err(|e| e.to_string())
}

fn main() {
    // Initialize tracing subscriber to see logs
    tracing_subscriber::fmt::init();

    // Initialize NIP-46 client keys directory
    // This must be done before any NIP-46 operations
    let keys_dir = dirs::data_local_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("arcadestr");
    
    // Create directory if it doesn't exist
    if let Err(e) = std::fs::create_dir_all(&keys_dir) {
        eprintln!("Warning: Could not create keys directory: {}", e);
    }
    
    // Set the keys directory for the signer module
    arcadestr_core::signer::set_keys_dir(keys_dir.clone());
    info!("NIP-46 keys directory: {}", keys_dir.display());

    // Set the users directory for saved users
    arcadestr_core::saved_users::set_users_dir(keys_dir.clone());
    info!("Saved users directory: {}", keys_dir.display());

    // Initialize NostrClient in a temporary runtime BEFORE Tauri starts
    let nostr_client = tokio::runtime::Runtime::new().unwrap().block_on(async {
        match NostrClient::new(DEFAULT_RELAYS.iter().map(|s| s.to_string()).collect()).await {
            Ok(client) => client,
            Err(e) => {
                eprintln!("Warning: Failed to initialize NostrClient: {}", e);
                eprintln!("The app will start but relay functionality may be limited.");
                // Create a client with no relays - user can retry later
                NostrClient::new(vec![])
                    .await
                    .expect("Failed to create empty client")
            }
        }
    });

    // ─────────────────────────────────────────────────────────────────────────────
    // Saved Users Management Commands
    // ─────────────────────────────────────────────────────────────────────────────

    /// Get all saved users.
    #[tauri::command]
    fn get_saved_users() -> Result<String, String> {
        use arcadestr_core::saved_users::load_saved_users;
        
        let users = load_saved_users()?;
        serde_json::to_string(&users).map_err(|e| e.to_string())
    }

    /// Add a new saved user.
    #[tauri::command]
    fn add_saved_user(
        method: String,
        relay: Option<String>,
        uri: Option<String>,
        private_key: Option<String>,
        npub: String,
    ) -> Result<String, String> {
        use arcadestr_core::saved_users::{create_saved_user, add_saved_user as save_user, LoginMethod};
        
        // Handle various method names from frontend
        let login_method = match method.as_str() {
            "nostrconnect" | "nip46" | "nostrconnect_uri" => LoginMethod::Nostrconnect,
            "bunker" | "bunker_uri" => LoginMethod::Bunker,
            "direct_key" | "private_key" | "key" => LoginMethod::DirectKey,
            "nip07" => LoginMethod::DirectKey, // NIP-07 uses same reconnection as direct key
            _ => {
                // Default to Nostrconnect for unknown methods
                tracing::warn!("Unknown login method '{}', defaulting to Nostrconnect", method);
                LoginMethod::Nostrconnect
            }
        };
        
        let user = create_saved_user(
            login_method,
            relay,
            uri,
            private_key,
            &npub,
        );
        
        let users = save_user(user)?;
        serde_json::to_string(&users).map_err(|e| e.to_string())
    }

    /// Remove a saved user by ID.
    #[tauri::command]
    fn remove_saved_user(user_id: String) -> Result<String, String> {
        use arcadestr_core::saved_users::remove_saved_user;
        
        let users = remove_saved_user(&user_id)?;
        serde_json::to_string(&users).map_err(|e| e.to_string())
    }

    /// Get a specific saved user.
    #[tauri::command]
    fn get_saved_user(user_id: String) -> Result<String, String> {
        use arcadestr_core::saved_users::get_saved_user;
        
        let user = get_saved_user(&user_id)?;
        serde_json::to_string(&user).map_err(|e| e.to_string())
    }

    /// Update user name/alias.
    #[tauri::command]
    fn rename_saved_user(user_id: String, new_name: String) -> Result<String, String> {
        use arcadestr_core::saved_users::{get_saved_user, update_saved_user};
        
        let mut user = get_saved_user(&user_id)?;
        user.name = new_name;
        let users = update_saved_user(user)?;
        serde_json::to_string(&users).map_err(|e| e.to_string())
    }

    /// Connect using a saved user (reconnect without re-entering credentials).
    #[tauri::command]
    async fn connect_saved_user(
        user_id: String,
        state: tauri::State<'_, AppState>,
    ) -> Result<String, String> {
        use arcadestr_core::saved_users::{get_saved_user, mark_user_as_used, LoginMethod};
        use arcadestr_core::signer::SignerError;
        
        let user = get_saved_user(&user_id)?;
        
        let mut auth = state.auth.lock().await;
        
        match user.method {
            LoginMethod::DirectKey => {
                if let Some(key) = user.private_key {
                    auth.connect_with_key(&key).map_err(|e: SignerError| e.to_string())?;
                    let _ = mark_user_as_used(&user_id);
                    
                    let pubkey = auth.public_key()
                        .ok_or("Public key not available")?;
                    return pubkey.to_bech32().map_err(|e| e.to_string());
                } else {
                    return Err("No private key found for this user".to_string());
                }
            }
            LoginMethod::Nostrconnect | LoginMethod::Bunker => {
                // For nostrconnect/bunker, we reconnect using the client keys
                // The signer will remember the approval based on the client public key
                let relay = user.relay.clone().unwrap_or_else(|| "wss://relay.nsec.app".to_string());
                
                // Build URI from client keys (we already have them saved)
                let uri_str = user.uri.clone().unwrap_or_else(|| {
                    // If no URI saved, generate a new nostrconnect URI with the same keys
                    // This works because the signer remembers the client pubkey
                    format!("nostrconnect://?relay={}", relay)
                });
                
                match auth.connect_nip46(&uri_str, &relay).await {
                    Ok(_) => {
                        let _ = mark_user_as_used(&user_id);
                        let pubkey = auth.public_key()
                            .ok_or("Public key not available after connection")?;
                        return pubkey.to_bech32().map_err(|e| e.to_string());
                    }
                    Err(e) => return Err(format!("Connection failed: {}", e)),
                }
            }
        }
    }

    tauri::Builder::default()
        .manage(AppState {
            auth: Arc::new(Mutex::new(AuthState::new())),
            nostr: Arc::new(Mutex::new(nostr_client)),
        })
        .invoke_handler(tauri::generate_handler![
            wait_for_nostrconnect_signer,
            generate_nostrconnect_uri,
            connect_nip46,
            connect_with_key,
            reconnect_relays,
            get_public_key,
            is_authenticated,
            disconnect,
            publish_listing,
            fetch_listings,
            fetch_listing_by_id,
            request_invoice,
            // Saved users management
            get_saved_users,
            add_saved_user,
            remove_saved_user,
            get_saved_user,
            rename_saved_user,
            connect_saved_user,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Arcadestr");
}
