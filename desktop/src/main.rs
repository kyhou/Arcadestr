// Desktop entry point: Tauri v2 application shell with NOSTR auth and listing commands.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tauri::Manager;
use tokio::sync::{Mutex, RwLock};
use tracing::{error, info, warn};

#[allow(unused_imports)]
use arcadestr_core::signers::NostrSigner;

use arcadestr_core::auth::AuthState;
use arcadestr_core::extended_network::ExtendedNetworkRepository;
use arcadestr_core::http_client::{HttpClient, ReqwestHttpClient};
use arcadestr_core::lightning::{request_zap_invoice, ZapInvoice, ZapRequest};
use arcadestr_core::marketplace::{apply_filter, apply_filter_nip99, MarketplaceFilter};
use arcadestr_core::marketplace_cache::{MarketplaceCache, UpsertOutcome};
use arcadestr_core::nip05_validator::Nip05Validator;
use arcadestr_core::nip46::AppSignerState;
use arcadestr_core::nip46::{
    restore_session_on_startup,
    storage::{get_profile_metadata_by_id, load_profile_from_keyring},
    SessionRestoreResult,
};
use arcadestr_core::nostr::{
    parse_nip19_identifier, EventDeduplicator, GameListing as CoreGameListing, ListingSource,
    NostrClient, UserProfile, DEFAULT_RELAYS,
};
use arcadestr_core::profile_fetcher::ProfileFetcher;
use arcadestr_core::relay_cache::RelayCache;
use arcadestr_core::relay_events::{RelayConnectionEvent, RelayStatus};
use arcadestr_core::relay_hints::RelayHints;
use arcadestr_core::relay_manager::normalize_relay_urls;
use arcadestr_core::social_graph::SocialGraphDb;
use arcadestr_core::subscriptions::{
    dispatch_ephemeral_reads_batch_with_policy, dispatch_permanent_subscriptions,
    run_notification_loop, SubscriptionRegistry,
};
use arcadestr_core::user_cache::UserCache;
use nostr::nips::nip19::FromBech32;
use nostr::nips::nip46::NostrConnectURI;
use nostr::prelude::ToBech32;
use tauri::Emitter;

mod command_contracts;
mod nip46_commands;

use arcadestr_app::models::GameListing as AppGameListing;
use command_contracts::{
    ExportKeyRequest, ExportKeyResult, ImportKeyRequest, ImportKeyResult, Nip05Status,
    Nip49ExportResult, Nip49ImportRequest, VerifyNip05Request, VerifyNip05Result,
};

const MARKETPLACE_REFRESH_OVERLAP_SECS: u64 = 86_400;

/// Application state shared across Tauri commands.
pub struct AppState {
    /// Authentication state wrapped in Arc<Mutex<>> for thread-safe access.
    pub auth: Arc<Mutex<AuthState>>,
    /// NOSTR client for relay communication.
    pub nostr: Arc<Mutex<NostrClient>>,
    /// Shared SQLite database for command contract cache coordination.
    pub database: Arc<arcadestr_core::storage::Database>,
    /// Relay cache for NIP-65 relay list management.
    pub relay_cache: Arc<RelayCache>,
    /// Event deduplicator to prevent duplicate event processing.
    pub deduplicator: Arc<Mutex<EventDeduplicator>>,
    /// Subscription registry for managing connection types.
    pub subscription_registry: Arc<SubscriptionRegistry>,
    /// Profile fetcher for batched profile fetching.
    pub profile_fetcher: Arc<ProfileFetcher>,
    /// User cache for persistent profile storage.
    pub user_cache: Arc<UserCache>,
    /// Marketplace cache for persistent listing storage.
    pub marketplace_cache: Arc<MarketplaceCache>,
    /// Purchase receipts repository for ownership lookups.
    pub purchases: Arc<arcadestr_core::purchases::PurchasesRepository>,
    /// Extended network repository for 2nd-degree follow discovery.
    pub extended_network: Arc<RwLock<Option<Arc<Mutex<ExtendedNetworkRepository>>>>>,
    /// Follows list for extended network refresh cycles.
    pub extended_network_follows: Arc<RwLock<Vec<String>>>,
    /// Relay hints store for extracting relay URLs from p-tags.
    pub relay_hints: Option<Arc<RelayHints>>,
    /// NIP-05 validator for background verification
    pub nip05_validator: Arc<std::sync::Mutex<Nip05Validator>>,
    /// Shared HTTP client used by command contracts.
    pub http_client: Arc<dyn HttpClient>,
}

impl command_contracts::BadgeCommandState for AppState {
    fn badge_command_handles(
        &self,
    ) -> (
        Arc<Mutex<arcadestr_core::nostr::NostrClient>>,
        Arc<arcadestr_core::storage::Database>,
    ) {
        (self.nostr.clone(), self.database.clone())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct NetworkDiscoverySettings {
    #[serde(default)]
    allow_insecure_public_ws: bool,
    #[serde(default)]
    debug_relays: Option<Vec<String>>,
    #[serde(default)]
    block_discovery: Option<bool>,
}

impl Default for NetworkDiscoverySettings {
    fn default() -> Self {
        Self {
            allow_insecure_public_ws: false,
            debug_relays: None,
            block_discovery: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DebugRelayConfigSource {
    Cli,
    Environment,
    Settings,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct DebugRelayCliOptions {
    relays: Vec<String>,
    block_discovery: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct DebugRelayEnvOptions {
    relays: Vec<String>,
    block_discovery: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ResolvedDebugRelayOptions {
    relays: Option<Vec<String>>,
    block_discovery: bool,
    source: Option<DebugRelayConfigSource>,
}

fn parse_debug_relay_cli_args(args: Vec<String>) -> Result<DebugRelayCliOptions, String> {
    let mut options = DebugRelayCliOptions::default();
    let mut saw_block_discovery = false;
    let mut saw_allow_discovery = false;
    let mut args = args.into_iter();

    while let Some(arg) = args.next() {
        if arg == "--relay" {
            let relay = args
                .next()
                .filter(|relay| !relay.starts_with("--"))
                .ok_or_else(|| "--relay requires a URL".to_string())?;
            options.relays.push(relay);
        } else if let Some(relay) = arg.strip_prefix("--relay=") {
            if relay.is_empty() || relay.starts_with("--") {
                return Err("--relay requires a URL".to_string());
            }
            options.relays.push(relay.to_string());
        } else if arg == "--block-discovery" {
            saw_block_discovery = true;
            options.block_discovery = Some(true);
        } else if arg == "--allow-discovery" {
            saw_allow_discovery = true;
            options.block_discovery = Some(false);
        }
    }

    if saw_block_discovery && saw_allow_discovery {
        return Err("--block-discovery and --allow-discovery cannot be used together".to_string());
    }

    Ok(options)
}

fn parse_debug_relay_env(
    relays: Option<String>,
    block_discovery: Option<String>,
) -> Result<DebugRelayEnvOptions, String> {
    let relays = match relays {
        Some(value) => {
            let mut relays = Vec::new();

            for relay in value.split(',').map(str::trim) {
                if relay.is_empty() {
                    return Err("ARCADESTR_RELAYS contains an empty relay URL entry".to_string());
                }

                relays.push(relay.to_owned());
            }

            if relays.is_empty() {
                return Err("ARCADESTR_RELAYS requires at least one relay URL".to_string());
            }

            relays
        }
        None => Vec::new(),
    };

    let block_discovery = block_discovery.as_deref().map(parse_bool_env).transpose()?;

    Ok(DebugRelayEnvOptions {
        relays,
        block_discovery,
    })
}

fn parse_bool_env(value: &str) -> Result<bool, String> {
    match value.trim().to_ascii_lowercase().as_str() {
        "true" | "1" | "yes" | "on" => Ok(true),
        "false" | "0" | "no" | "off" => Ok(false),
        _ => Err(format!(
            "invalid boolean value `{value}`; expected true/1/yes/on or false/0/no/off"
        )),
    }
}

fn resolve_debug_relay_options(
    cli: DebugRelayCliOptions,
    env: DebugRelayEnvOptions,
    settings: &NetworkDiscoverySettings,
) -> Result<ResolvedDebugRelayOptions, String> {
    let block_discovery = cli
        .block_discovery
        .or(env.block_discovery)
        .or(settings.block_discovery);

    let (relays, source) = if !cli.relays.is_empty() {
        (cli.relays, Some(DebugRelayConfigSource::Cli))
    } else if !env.relays.is_empty() {
        (env.relays, Some(DebugRelayConfigSource::Environment))
    } else if let Some(relays) = settings
        .debug_relays
        .clone()
        .filter(|relays| !relays.is_empty())
    {
        (relays, Some(DebugRelayConfigSource::Settings))
    } else {
        return Ok(ResolvedDebugRelayOptions {
            relays: None,
            block_discovery: false,
            source: None,
        });
    };

    let relays = normalize_relay_urls(relays).map_err(|err| err.to_string())?;

    Ok(ResolvedDebugRelayOptions {
        relays: Some(relays),
        block_discovery: block_discovery.unwrap_or(true),
        source,
    })
}

fn build_startup_relay_config(
    debug_relay_options: &ResolvedDebugRelayOptions,
) -> (
    arcadestr_core::relay_manager::RelayManagerConfig,
    Vec<String>,
) {
    let relay_config = arcadestr_core::relay_manager::RelayManagerConfig {
        max_relays: 100,
        query_timeout_secs: 10,
        connection_poll_timeout_ms: 3000,
        connection_poll_interval_ms: 50,
        debug_relays: debug_relay_options.relays.clone(),
        block_discovery: debug_relay_options.block_discovery,
    };

    let startup_relays = if debug_relay_options.relays.is_some() {
        vec![]
    } else {
        DEFAULT_RELAYS.iter().map(|s| s.to_string()).collect()
    };

    (relay_config, startup_relays)
}

fn settings_file_path() -> std::path::PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("arcadestr")
        .join("settings.json")
}

fn parse_network_discovery_settings(content: &str) -> Result<NetworkDiscoverySettings, String> {
    serde_json::from_str::<NetworkDiscoverySettings>(content)
        .map_err(|e| format!("failed to parse settings.json: {e}"))
}

fn try_load_network_discovery_settings() -> Result<NetworkDiscoverySettings, String> {
    let path = settings_file_path();
    let content = match std::fs::read_to_string(&path) {
        Ok(content) => content,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Ok(NetworkDiscoverySettings::default());
        }
        Err(e) => {
            return Err(format!(
                "failed to read settings.json at {}: {e}",
                path.display()
            ));
        }
    };

    parse_network_discovery_settings(&content)
}

fn load_network_discovery_settings() -> NetworkDiscoverySettings {
    try_load_network_discovery_settings().unwrap_or_default()
}

fn save_network_discovery_settings(settings: &NetworkDiscoverySettings) -> Result<(), String> {
    let path = settings_file_path();
    let parent = path
        .parent()
        .ok_or_else(|| "Invalid settings file path".to_string())?;
    std::fs::create_dir_all(parent).map_err(|e| format!("Failed to create settings dir: {e}"))?;

    let json = serde_json::to_string_pretty(settings)
        .map_err(|e| format!("Failed to serialize settings: {e}"))?;
    std::fs::write(path, json).map_err(|e| format!("Failed to write settings file: {e}"))
}

#[tauri::command]
async fn get_network_discovery_settings() -> Result<NetworkDiscoverySettings, String> {
    Ok(load_network_discovery_settings())
}

#[tauri::command]
async fn set_allow_insecure_public_ws(
    allow: bool,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    let mut settings = load_network_discovery_settings();
    settings.allow_insecure_public_ws = allow;
    save_network_discovery_settings(&settings)?;

    let en_option = state.extended_network.read().await;
    if let Some(ref repo) = *en_option {
        let repo_guard = repo.lock().await;
        repo_guard.set_allow_insecure_public_ws(allow);
    }

    Ok(())
}

fn listing_cache_key(listing: &CoreGameListing) -> (String, String) {
    (listing.publisher_npub.clone(), listing.id.clone())
}

fn listing_signature(listing: &CoreGameListing) -> String {
    let tags_json = serde_json::to_string(&listing.tags).unwrap_or_else(|_| "[]".to_string());
    let platforms_json =
        serde_json::to_string(&listing.platforms).unwrap_or_else(|_| "[]".to_string());
    format!(
        "{}|{}|{}|{}|{}|{}|{}|{}|{}|{}",
        listing.publisher_npub,
        listing.id,
        listing.title,
        listing.description,
        listing.price_sats,
        listing.download_url,
        listing.created_at,
        tags_json,
        platforms_json,
        listing.nip94_event_id.as_deref().unwrap_or_default()
    )
}

fn since_days_cutoff(since_days: Option<u64>) -> Option<u64> {
    let days = since_days?;
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0);
    Some(now.saturating_sub(days.saturating_mul(86_400)))
}

fn marketplace_refresh_since_secs(
    latest_cached_created_at: Option<u64>,
    since_days: Option<u64>,
    until_secs: Option<u64>,
) -> Option<u64> {
    if until_secs.is_some() {
        return since_days_cutoff(since_days);
    }

    latest_cached_created_at
        .map(|latest| latest.saturating_sub(MARKETPLACE_REFRESH_OVERLAP_SECS))
        .or_else(|| since_days_cutoff(since_days))
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
    use arcadestr_core::signers::Nip46Signer;
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

    info!(
        "URI generated successfully, client pubkey: {}",
        client_keys.public_key().to_hex()
    );

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
    app_handle: tauri::AppHandle,
) -> Result<String, String> {
    use tracing::{error, info};

    info!("Connecting with direct key...");
    info!("Key length: {} chars", key.len());

    let mut auth = state.auth.lock().await;

    let result = match command_contracts::auth_connect_with_key(&mut auth, &key) {
        Ok(ok) => {
            info!("Direct key authentication successful");
            ok
        }
        Err(e) => {
            error!("Direct key authentication failed: {}", e);
            return Err(e);
        }
    };

    let _ = app_handle.emit(result.event_name, result.npub.clone());
    Ok(result.npub)
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
    use arcadestr_core::signers::Nip46Signer;
    use nostr::nips::nip19::ToBech32;

    let mut auth = state.auth.lock().await;

    // Check if we have pending nostrconnect credentials
    let pending = auth
        .take_pending_nostrconnect()
        .ok_or("No pending nostrconnect connection. Generate a URI first.")?;
    let client_keys = pending.client_keys;
    let relay = pending.relay;
    let _secret = pending.secret;

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
    command_contracts::auth_get_public_key(&auth)
}

/// Checks if the user is currently authenticated.
///
/// # Returns
/// `true` if authenticated, `false` otherwise.
#[tauri::command]
async fn is_authenticated(state: tauri::State<'_, AppState>) -> Result<bool, String> {
    let auth = state.auth.lock().await;
    Ok(command_contracts::auth_is_authenticated(&auth))
}

#[tauri::command]
async fn nip49_import(
    request: Nip49ImportRequest,
    state: tauri::State<'_, AppState>,
) -> Result<String, String> {
    command_contracts::nip49_import(request, state.inner())
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
async fn export_encrypted_key(
    request: ExportKeyRequest,
    state: tauri::State<'_, AppState>,
) -> Result<ExportKeyResult, String> {
    command_contracts::export_encrypted_key(state.inner(), request)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
async fn import_encrypted_key(
    request: ImportKeyRequest,
    state: tauri::State<'_, AppState>,
) -> Result<ImportKeyResult, String> {
    command_contracts::import_encrypted_key(state.inner(), request)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
async fn verify_nip05_identity(
    request: VerifyNip05Request,
    state: tauri::State<'_, AppState>,
) -> Result<VerifyNip05Result, String> {
    command_contracts::verify_nip05_identity(state.inner(), request)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
async fn nip49_export(
    npub: String,
    password: String,
    state: tauri::State<'_, AppState>,
) -> Result<Nip49ExportResult, String> {
    command_contracts::nip49_export(npub, password, state.inner())
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
async fn verify_nip05(
    identifier: String,
    expected_npub: String,
    state: tauri::State<'_, AppState>,
) -> Result<Nip05Status, String> {
    command_contracts::verify_nip05(identifier, expected_npub, state.inner())
        .await
        .map_err(|error| error.to_string())
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
    listing: CoreGameListing,
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
) -> Result<Vec<CoreGameListing>, String> {
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
) -> Result<CoreGameListing, String> {
    let nostr = state.nostr.lock().await;

    nostr
        .fetch_listing_by_id(&publisher_npub, &listing_id)
        .await
        .map_err(|e| e.to_string())
}

/// Fetches NIP-15 stalls and products, applies an optional filter, and
/// returns the results as `GameListing` values ready for the browse view.
///
/// * `limit`      — max events to retrieve per kind (30017 and 30018 each).
/// * `since_days` — time window; `None` means no time restriction.
/// * `filter`     — pass `None` (or `{}` from JS) to skip all filtering.
#[tauri::command]
async fn fetch_marketplace(
    state: tauri::State<'_, AppState>,
    limit: usize,
    since_days: Option<u64>,
    filter: Option<MarketplaceFilter>,
) -> Result<Vec<AppGameListing>, String> {
    let filter = filter.unwrap_or_default();

    tracing::info!(
        "fetch_marketplace called: limit={}, since_days={:?}",
        limit,
        since_days
    );

    let products = {
        let nostr = state.nostr.lock().await;
        nostr.fetch_nip99_listings(limit, since_days).await?
    };
    tracing::info!("fetch_marketplace: got {} products", products.len());

    // TODO(nip99-migration): stall enrichment pipeline removed
    // NIP-99 listings are self-contained — no stall join needed

    // Apply the filter (no-op while filter == default).
    let filtered = apply_filter_nip99(products, &filter);
    tracing::info!(
        "fetch_marketplace: {} products after filtering",
        filtered.len()
    );

    let buyer_pubkey_hex = {
        let auth = state.auth.lock().await;
        auth.public_key().map(|public_key| public_key.to_hex())
    };

    let mut listings = Vec::with_capacity(filtered.len());
    for product in filtered {
        let listing = AppGameListing::from_listing(product);
        let listing = enrich_listing_ownership(
            listing,
            Arc::clone(&state.purchases),
            buyer_pubkey_hex.clone(),
            "fetch_marketplace",
        )
        .await;

        listings.push(listing);
    }

    tracing::info!("fetch_marketplace: returning {} listings", listings.len());
    Ok(listings)
}

fn listing_coordinate(listing: &arcadestr_core::marketplace::Nip99Listing) -> Option<String> {
    listing_coordinate_from_npub(&listing.merchant_npub, &listing.id).ok()
}

fn listing_coordinate_from_app_listing(listing: &AppGameListing) -> Result<String, String> {
    listing_coordinate_from_npub(&listing.publisher_npub, &listing.id)
}

fn listing_coordinate_from_npub(publisher_npub: &str, listing_id: &str) -> Result<String, String> {
    let merchant_pubkey = nostr::PublicKey::from_bech32(publisher_npub)
        .map_err(|_| "invalid publisher pubkey".to_string())?;
    Ok(format!("30402:{}:{}", merchant_pubkey.to_hex(), listing_id))
}

fn app_listing_from_cached_listing(listing: CoreGameListing) -> AppGameListing {
    AppGameListing {
        id: listing.id,
        source: arcadestr_app::models::ListingSource::Nip99Listing,
        title: listing.title,
        description: listing.description,
        images: listing.images,
        download_url: listing.download_url,
        price: listing.price_sats as f64,
        currency: "SATS".to_string(),
        price_sats: listing.price_sats,
        quantity: None,
        tags: listing.tags,
        specs: Vec::new(),
        publisher_npub: listing.publisher_npub,
        stall_id: String::new(),
        stall_name: None,
        lud16: listing.lud16,
        event_id: None,
        created_at: listing.created_at,
        platforms: listing.platforms,
        nip94_event_id: listing.nip94_event_id,
        is_owned: false,
        #[cfg(debug_assertions)]
        nip99_raw_event_json: listing.nip99_raw_event_json,
    }
}

async fn enrich_listing_ownership(
    mut listing: AppGameListing,
    purchases: Arc<arcadestr_core::purchases::PurchasesRepository>,
    buyer_pubkey_hex: Option<String>,
    log_context: &'static str,
) -> AppGameListing {
    let Some(buyer_pubkey_hex) = buyer_pubkey_hex else {
        return listing;
    };

    match listing_coordinate_from_app_listing(&listing) {
        Ok(coordinate) => match purchases.is_owned(&buyer_pubkey_hex, &coordinate).await {
            Ok(is_owned) => listing.is_owned = is_owned,
            Err(error) => tracing::warn!(
                "{}: ownership lookup failed for {}: {}",
                log_context,
                coordinate,
                error
            ),
        },
        Err(error) => tracing::warn!(
            "{}: unable to build ownership coordinate for listing '{}': {}",
            log_context,
            listing.id,
            error
        ),
    }

    listing
}

#[tauri::command]
fn get_platform_info() -> arcadestr_app::models::PlatformInfo {
    arcadestr_app::models::PlatformInfo {
        os: std::env::consts::OS.to_string(),
        arch: std::env::consts::ARCH.to_string(),
    }
}

#[tauri::command]
async fn install_game(
    listing: AppGameListing,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    tracing::info!(
        "install_game called for listing '{}' ({})",
        listing.title,
        listing.id
    );

    let buyer_pubkey_hex = {
        let auth = state.auth.lock().await;
        auth.public_key()
            .map(|public_key| public_key.to_hex())
            .ok_or_else(|| "not authenticated".to_string())?
    };

    let coordinate = listing_coordinate_from_app_listing(&listing)?;
    let is_owned = state
        .purchases
        .is_owned(&buyer_pubkey_hex, &coordinate)
        .await
        .map_err(|error| error.to_string())?;

    if !is_owned {
        return Err("purchase not found".to_string());
    }

    // TODO(nip94-delivery): download and verify the NIP-94 payload before install.
    Ok(())
}

#[tauri::command]
async fn ingest_receipt(
    raw_event_json: String,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    let event: nostr::Event = serde_json::from_str(&raw_event_json).map_err(|e| e.to_string())?;
    let buyer_pubkey_hex = {
        let auth = state.auth.lock().await;
        auth.public_key().ok_or("not authenticated")?.to_hex()
    };
    let receipt = arcadestr_core::purchases::parse_and_validate_receipt(&event, &buyer_pubkey_hex)
        .map_err(|e| e.to_string())?;
    state
        .purchases
        .upsert_receipt(&receipt)
        .await
        .map_err(|e| e.to_string())
}

/// Streams NIP-15 products from relays with real-time updates via Tauri events.
///
/// Products are emitted as they arrive from each relay via the `marketplace-product` event.
/// When all relays finish or inactivity timeout is reached, `marketplace-complete` is emitted.
/// This command returns immediately after starting the fetch; results come via events.
///
/// * `limit`      — max events to retrieve per relay.
/// * `since_days` — time window; `None` means no time restriction.
#[tauri::command]
async fn fetch_marketplace_stream(
    window: tauri::Window,
    state: tauri::State<'_, AppState>,
    limit: usize,
    since_days: Option<u64>,
    until_secs: Option<u64>,
) -> Result<(), String> {
    use std::collections::HashMap;
    use std::sync::{Arc as StdArc, Mutex as StdMutex};

    tracing::info!(
        "fetch_marketplace_stream called: limit={}, since_days={:?}, until_secs={:?}",
        limit,
        since_days,
        until_secs
    );

    let buyer_pubkey_hex = {
        let auth = state.auth.lock().await;
        auth.public_key().map(|public_key| public_key.to_hex())
    };

    let mut cached_emitted = 0usize;
    let mut cached_signatures: HashMap<(String, String), String> = HashMap::new();

    match state
        .marketplace_cache
        .load_listings(limit, since_days, until_secs)
        .await
    {
        Ok(cached) => {
            for listing in cached {
                cached_signatures.insert(listing_cache_key(&listing), listing_signature(&listing));
                let listing = app_listing_from_cached_listing(listing);
                let listing = enrich_listing_ownership(
                    listing,
                    Arc::clone(&state.purchases),
                    buyer_pubkey_hex.clone(),
                    "fetch_marketplace_stream cache",
                )
                .await;

                if window.emit("marketplace-product", &listing).is_ok() {
                    cached_emitted += 1;
                }
            }
        }
        Err(e) => {
            tracing::warn!("fetch_marketplace_stream: failed to load cache: {}", e);
        }
    }
    tracing::info!(
        "fetch_marketplace_stream: emitted {} cached products",
        cached_emitted
    );

    let latest_cached_created_at = match state.marketplace_cache.latest_created_at().await {
        Ok(latest) => latest,
        Err(e) => {
            tracing::warn!("fetch_marketplace_stream: failed to read cache cursor: {e}");
            None
        }
    };
    let refresh_since_secs =
        marketplace_refresh_since_secs(latest_cached_created_at, since_days, until_secs);
    tracing::info!(
        "fetch_marketplace_stream: relay refresh cursor since_secs={:?} latest_cached_created_at={:?} overlap_secs={}",
        refresh_since_secs,
        latest_cached_created_at,
        MARKETPLACE_REFRESH_OVERLAP_SECS
    );

    let seen_signatures = StdArc::new(StdMutex::new(cached_signatures));
    let relay_updates: StdArc<StdMutex<Vec<CoreGameListing>>> =
        StdArc::new(StdMutex::new(Vec::new()));
    let emit_tasks: StdArc<StdMutex<Vec<tauri::async_runtime::JoinHandle<()>>>> =
        StdArc::new(StdMutex::new(Vec::new()));

    // Clone window for use in the closure
    let window_for_closure = window.clone();
    let seen_signatures_for_closure = StdArc::clone(&seen_signatures);
    let relay_updates_for_closure = StdArc::clone(&relay_updates);
    let emit_tasks_for_closure = StdArc::clone(&emit_tasks);
    let purchases_for_closure = Arc::clone(&state.purchases);
    let buyer_pubkey_hex_for_closure = buyer_pubkey_hex.clone();

    // Capture relay manager without holding the AppState nostr lock across awaits.
    let relay_manager = {
        let nostr = state.nostr.lock().await;
        nostr.relay_manager()
    };

    // Start streaming product fetch.
    // IMPORTANT: this await should not hold state.nostr lock, otherwise other commands
    // (e.g. get_connected_relays for topbar relay badge) can be blocked behind marketplace fetch.
    let products = arcadestr_core::marketplace::fetch_nip99_listings_streaming_since(
        &relay_manager,
        limit,
        refresh_since_secs,
        until_secs,
        move |product| {
            let core_listing = CoreGameListing::from_listing(product.clone());
            let key = listing_cache_key(&core_listing);
            let signature = listing_signature(&core_listing);

            let should_emit = {
                let mut known = seen_signatures_for_closure
                    .lock()
                    .expect("seen signatures mutex poisoned");
                match known.get(&key) {
                    Some(existing) if existing == &signature => false,
                    _ => {
                        known.insert(key, signature);
                        true
                    }
                }
            };

            if !should_emit {
                return;
            }

            {
                let mut updates = relay_updates_for_closure
                    .lock()
                    .expect("relay updates mutex poisoned");
                updates.push(core_listing);
            }

            let listing = AppGameListing::from_listing(product);
            let purchases = Arc::clone(&purchases_for_closure);
            let buyer_pubkey_hex = buyer_pubkey_hex_for_closure.clone();
            let window = window_for_closure.clone();
            let task = tauri::async_runtime::spawn(async move {
                let listing = enrich_listing_ownership(
                    listing,
                    purchases,
                    buyer_pubkey_hex,
                    "fetch_marketplace_stream",
                )
                .await;

                if let Err(e) = window.emit("marketplace-product", &listing) {
                    tracing::debug!("Failed to emit marketplace-product: {}", e);
                }
            });

            let mut tasks = emit_tasks_for_closure
                .lock()
                .expect("emit tasks mutex poisoned");
            tasks.push(task);
        },
    )
    .await;

    match products {
        Ok(count) => {
            tracing::info!("fetch_marketplace_stream: emitted {} products", count);
        }
        Err(e) => {
            tracing::warn!("fetch_marketplace_stream: fetch error: {}", e);
        }
    }

    let emit_handles = {
        let mut tasks = emit_tasks.lock().expect("emit tasks mutex poisoned");
        std::mem::take(&mut *tasks)
    };
    for handle in emit_handles {
        if let Err(error) = handle.await {
            tracing::warn!("fetch_marketplace_stream: emit task failed: {}", error);
        }
    }

    let updates = {
        let updates_guard = relay_updates.lock().expect("relay updates mutex poisoned");
        updates_guard.clone()
    };

    let mut upserted = 0usize;
    let mut updated = 0usize;
    let mut unchanged = 0usize;

    for listing in updates {
        match state.marketplace_cache.upsert_listing(&listing, None).await {
            Ok(UpsertOutcome::Inserted) => upserted += 1,
            Ok(UpsertOutcome::Updated) => updated += 1,
            Ok(UpsertOutcome::Unchanged) => unchanged += 1,
            Err(e) => tracing::warn!("fetch_marketplace_stream: upsert failed: {}", e),
        }
    }

    tracing::info!(
        "fetch_marketplace_stream: cache upserts inserted={}, updated={}, unchanged={}",
        upserted,
        updated,
        unchanged
    );

    // Signal completion
    let _ = window.emit("marketplace-complete", ());

    Ok(())
}

/// Fetches user profile metadata (NIP-01 kind-0) with NIP-65 relay discovery.
///
/// # Arguments
/// * `npub` - The bech32 npub of the user
///
/// # Returns
/// The user profile on success.
#[tauri::command]
async fn fetch_profile(
    npub: String,
    _additional_relays: Option<Vec<String>>,
    state: tauri::State<'_, AppState>,
) -> Result<UserProfile, String> {
    let nostr = state.nostr.lock().await;

    // Use NIP-65 relay discovery first, then fetch profile
    nostr
        .fetch_profile_with_relay_discovery(&npub)
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

#[cfg(test)]
mod task4_tests {
    use super::*;
    use nostr::prelude::Keys;

    fn listing_with_merchant_npub(
        merchant_npub: String,
    ) -> arcadestr_core::marketplace::Nip99Listing {
        arcadestr_core::marketplace::Nip99Listing {
            id: "game-v1".to_string(),
            title: "Game".to_string(),
            content: String::new(),
            summary: None,
            published_at: None,
            location: None,
            images: Vec::new(),
            price_amount: None,
            price_currency: None,
            price_frequency: None,
            geohash: None,
            merchant_npub,
            tags: Vec::new(),
            created_at: 0,
            platforms: Vec::new(),
            nip94_event_id: None,
            status: None,
            #[cfg(debug_assertions)]
            raw_event_json: None,
        }
    }

    #[test]
    fn marketplace_refresh_cursor_uses_latest_cache_with_overlap() {
        let since = marketplace_refresh_since_secs(Some(1_710_030_000), Some(30), None);

        assert_eq!(
            since,
            Some(1_710_030_000 - MARKETPLACE_REFRESH_OVERLAP_SECS)
        );
    }

    #[test]
    fn marketplace_refresh_cursor_ignores_latest_cache_for_pagination() {
        let since = marketplace_refresh_since_secs(Some(1_710_030_000), None, Some(1_710_000_000));

        assert_eq!(since, None);
    }

    #[test]
    fn listing_coordinate_uses_kind_merchant_hex_and_d_tag() {
        let merchant = Keys::generate();
        let listing = listing_with_merchant_npub(
            merchant
                .public_key()
                .to_bech32()
                .expect("public key should encode as npub"),
        );

        let coordinate = listing_coordinate(&listing).expect("valid npub should build coordinate");

        assert_eq!(
            coordinate,
            format!("30402:{}:game-v1", merchant.public_key().to_hex())
        );
    }

    #[test]
    fn listing_signature_includes_platform_tags() {
        let linux_listing = CoreGameListing {
            id: "game-v1".to_string(),
            source: ListingSource::Nip99Listing,
            title: "Game".to_string(),
            description: "Description".to_string(),
            price_sats: 100,
            download_url: "https://example.com/game.zip".to_string(),
            publisher_npub: "npub1publisher".to_string(),
            created_at: 1,
            tags: Vec::new(),
            lud16: String::new(),
            images: Vec::new(),
            platforms: vec!["linux-x86_64".to_string()],
            nip94_event_id: None,
            summary: None,
            published_at: None,
            location: None,
            geohash: None,
            status: None,
            #[cfg(debug_assertions)]
            nip99_raw_event_json: None,
        };
        let mut windows_listing = linux_listing.clone();
        windows_listing.platforms = vec!["windows-x86_64".to_string()];

        assert_ne!(
            listing_signature(&linux_listing),
            listing_signature(&windows_listing)
        );

        let mut nip94_listing = linux_listing.clone();
        nip94_listing.nip94_event_id = Some("nip94-event-1".to_string());
        assert_ne!(
            listing_signature(&linux_listing),
            listing_signature(&nip94_listing)
        );
    }

    #[test]
    fn listing_coordinate_rejects_invalid_merchant_npub() {
        let listing = listing_with_merchant_npub("not-an-npub".to_string());

        assert!(listing_coordinate(&listing).is_none());
    }

    #[test]
    fn app_listing_coordinate_uses_publisher_npub_and_listing_id() {
        let merchant = Keys::generate();
        let listing = AppGameListing {
            id: "game-v1".to_string(),
            source: arcadestr_app::models::ListingSource::Nip99Listing,
            title: "Game".to_string(),
            description: String::new(),
            images: Vec::new(),
            download_url: String::new(),
            price: 0.0,
            currency: String::new(),
            price_sats: 0,
            quantity: None,
            tags: Vec::new(),
            specs: Vec::new(),
            publisher_npub: merchant
                .public_key()
                .to_bech32()
                .expect("public key should encode as npub"),
            stall_id: String::new(),
            stall_name: None,
            lud16: String::new(),
            event_id: None,
            created_at: 0,
            platforms: Vec::new(),
            nip94_event_id: None,
            is_owned: false,
            #[cfg(debug_assertions)]
            nip99_raw_event_json: None,
        };

        let coordinate = listing_coordinate_from_app_listing(&listing)
            .expect("valid npub should build coordinate");

        assert_eq!(
            coordinate,
            format!("30402:{}:game-v1", merchant.public_key().to_hex())
        );
    }

    #[test]
    fn cached_core_listing_maps_to_app_listing_with_defaults() {
        let cached = CoreGameListing {
            id: "game-v1".to_string(),
            source: ListingSource::Nip99Listing,
            title: "Game".to_string(),
            description: "Description".to_string(),
            price_sats: 21,
            download_url: "https://example.com/game.zip".to_string(),
            publisher_npub: "npub1publisher".to_string(),
            created_at: 7,
            tags: vec!["arcade".to_string()],
            lud16: "merchant@example.com".to_string(),
            images: vec!["https://example.com/cover.png".to_string()],
            platforms: vec!["linux-x86_64".to_string()],
            nip94_event_id: Some("nip94-event-1".to_string()),
            summary: Some("Summary".to_string()),
            published_at: Some(6),
            location: None,
            geohash: None,
            status: Some("active".to_string()),
            #[cfg(debug_assertions)]
            nip99_raw_event_json: Some(r#"{"kind":30402}"#.to_string()),
        };

        let listing = app_listing_from_cached_listing(cached);

        assert_eq!(listing.platforms, vec!["linux-x86_64"]);
        assert_eq!(listing.nip94_event_id, Some("nip94-event-1".to_string()));
        assert_eq!(listing.price_sats, 21);
        assert_eq!(listing.currency, "SATS");
        assert!(!listing.is_owned);
    }
}

#[cfg(test)]
mod debug_relay_config_tests {
    use super::*;

    #[test]
    fn test_cli_debug_relays_override_env_and_settings() {
        let settings = NetworkDiscoverySettings {
            allow_insecure_public_ws: false,
            debug_relays: Some(vec!["wss://settings.example.com".to_string()]),
            block_discovery: Some(false),
        };

        let cli = parse_debug_relay_cli_args(vec![
            "--relay".to_string(),
            "wss://cli.example.com".to_string(),
            "--block-discovery".to_string(),
        ])
        .expect("cli should parse");

        let env = parse_debug_relay_env(
            Some("wss://env.example.com".to_string()),
            Some("false".to_string()),
        )
        .expect("env should parse");

        let resolved =
            resolve_debug_relay_options(cli, env, &settings).expect("options should resolve");

        assert_eq!(
            resolved.relays,
            Some(vec!["wss://cli.example.com/".to_string()])
        );
        assert!(resolved.block_discovery);
        assert_eq!(resolved.source, Some(DebugRelayConfigSource::Cli));
    }

    #[test]
    fn test_env_debug_relays_override_settings() {
        let settings = NetworkDiscoverySettings {
            allow_insecure_public_ws: false,
            debug_relays: Some(vec!["wss://settings.example.com".to_string()]),
            block_discovery: Some(false),
        };

        let cli = DebugRelayCliOptions::default();
        let env = parse_debug_relay_env(
            Some("wss://env.example.com".to_string()),
            Some("true".to_string()),
        )
        .expect("env should parse");

        let resolved =
            resolve_debug_relay_options(cli, env, &settings).expect("options should resolve");

        assert_eq!(
            resolved.relays,
            Some(vec!["wss://env.example.com/".to_string()])
        );
        assert!(resolved.block_discovery);
        assert_eq!(resolved.source, Some(DebugRelayConfigSource::Environment));
    }

    #[test]
    fn test_settings_debug_relays_default_to_block_discovery() {
        let settings = NetworkDiscoverySettings {
            allow_insecure_public_ws: false,
            debug_relays: Some(vec!["wss://settings.example.com".to_string()]),
            block_discovery: None,
        };

        let resolved = resolve_debug_relay_options(
            DebugRelayCliOptions::default(),
            DebugRelayEnvOptions::default(),
            &settings,
        )
        .expect("options should resolve");

        assert_eq!(
            resolved.relays,
            Some(vec!["wss://settings.example.com/".to_string()])
        );
        assert!(resolved.block_discovery);
        assert_eq!(resolved.source, Some(DebugRelayConfigSource::Settings));
    }

    #[test]
    fn test_allow_discovery_sets_block_discovery_false() {
        let cli = parse_debug_relay_cli_args(vec![
            "--relay".to_string(),
            "wss://cli.example.com".to_string(),
            "--allow-discovery".to_string(),
        ])
        .expect("cli should parse");

        let resolved = resolve_debug_relay_options(
            cli,
            DebugRelayEnvOptions::default(),
            &NetworkDiscoverySettings::default(),
        )
        .expect("options should resolve");

        assert!(!resolved.block_discovery);
    }

    #[test]
    fn test_cli_allow_discovery_overrides_env_block_for_env_relays() {
        let cli = parse_debug_relay_cli_args(vec!["--allow-discovery".to_string()])
            .expect("cli should parse");
        let env = parse_debug_relay_env(
            Some("wss://env.example.com".to_string()),
            Some("true".to_string()),
        )
        .expect("env should parse");

        let resolved = resolve_debug_relay_options(cli, env, &NetworkDiscoverySettings::default())
            .expect("options should resolve");

        assert_eq!(
            resolved.relays,
            Some(vec!["wss://env.example.com/".to_string()])
        );
        assert!(!resolved.block_discovery);
        assert_eq!(resolved.source, Some(DebugRelayConfigSource::Environment));
    }

    #[test]
    fn test_env_allow_discovery_overrides_settings_block_for_settings_relays() {
        let settings = NetworkDiscoverySettings {
            allow_insecure_public_ws: false,
            debug_relays: Some(vec!["wss://settings.example.com".to_string()]),
            block_discovery: Some(true),
        };
        let env = parse_debug_relay_env(None, Some("false".to_string())).expect("env should parse");

        let resolved = resolve_debug_relay_options(DebugRelayCliOptions::default(), env, &settings)
            .expect("options should resolve");

        assert_eq!(
            resolved.relays,
            Some(vec!["wss://settings.example.com/".to_string()])
        );
        assert!(!resolved.block_discovery);
        assert_eq!(resolved.source, Some(DebugRelayConfigSource::Settings));
    }

    #[test]
    fn test_cli_block_discovery_overrides_settings_allow_for_settings_relays() {
        let settings = NetworkDiscoverySettings {
            allow_insecure_public_ws: false,
            debug_relays: Some(vec!["wss://settings.example.com".to_string()]),
            block_discovery: Some(false),
        };
        let cli = parse_debug_relay_cli_args(vec!["--block-discovery".to_string()])
            .expect("cli should parse");

        let resolved = resolve_debug_relay_options(cli, DebugRelayEnvOptions::default(), &settings)
            .expect("options should resolve");

        assert_eq!(
            resolved.relays,
            Some(vec!["wss://settings.example.com/".to_string()])
        );
        assert!(resolved.block_discovery);
        assert_eq!(resolved.source, Some(DebugRelayConfigSource::Settings));
    }

    #[test]
    fn test_conflicting_discovery_flags_are_rejected() {
        let err = parse_debug_relay_cli_args(vec![
            "--relay".to_string(),
            "wss://cli.example.com".to_string(),
            "--block-discovery".to_string(),
            "--allow-discovery".to_string(),
        ])
        .expect_err("conflicting flags should fail");

        assert!(err.contains("cannot be used together"));
    }

    #[test]
    fn test_relay_flag_rejects_discovery_flag_as_missing_value() {
        let err = parse_debug_relay_cli_args(vec![
            "--relay".to_string(),
            "--block-discovery".to_string(),
        ])
        .expect_err("relay flag should reject another flag as its value");

        assert!(err.contains("--relay requires"));
    }

    #[test]
    fn test_whitespace_only_env_relay_config_is_rejected() {
        let err = parse_debug_relay_env(Some(" \t  \n ".to_string()), None)
            .expect_err("env relay config without relay URLs should fail");

        assert!(err.contains("ARCADESTR_RELAYS"));
    }

    #[test]
    fn test_env_relay_config_with_internal_empty_token_is_rejected() {
        let err = parse_debug_relay_env(
            Some("wss://a.example.com, ,wss://b.example.com".to_string()),
            None,
        )
        .expect_err("env relay config with empty relay token should fail");

        assert!(err.contains("ARCADESTR_RELAYS"));
    }

    #[test]
    fn test_legacy_settings_json_defaults_debug_relay_fields() {
        let settings: NetworkDiscoverySettings =
            serde_json::from_str(r#"{"allow_insecure_public_ws":true}"#)
                .expect("legacy settings JSON should deserialize");

        assert!(settings.allow_insecure_public_ws);
        assert_eq!(settings.debug_relays, None);
        assert_eq!(settings.block_discovery, None);
    }

    #[test]
    fn test_no_debug_relay_sources_resolves_to_discovery_defaults() {
        let resolved = resolve_debug_relay_options(
            DebugRelayCliOptions::default(),
            DebugRelayEnvOptions::default(),
            &NetworkDiscoverySettings::default(),
        )
        .expect("empty relay sources should resolve");

        assert_eq!(resolved.relays, None);
        assert!(!resolved.block_discovery);
        assert_eq!(resolved.source, None);
    }

    #[test]
    fn test_block_discovery_without_relay_source_is_ignored() {
        let cli = DebugRelayCliOptions {
            relays: Vec::new(),
            block_discovery: Some(true),
        };

        let resolved = resolve_debug_relay_options(
            cli,
            DebugRelayEnvOptions::default(),
            &NetworkDiscoverySettings::default(),
        )
        .expect("block flag without relays should resolve to normal startup");

        assert_eq!(resolved.relays, None);
        assert!(!resolved.block_discovery);
        assert_eq!(resolved.source, None);
    }

    #[test]
    fn test_parse_network_discovery_settings_reports_malformed_json() {
        let err = parse_network_discovery_settings("{not valid json")
            .expect_err("malformed settings JSON should be reported");

        assert!(err.contains("failed to parse settings.json"));
    }

    #[test]
    fn test_startup_relay_config_uses_debug_relays_without_defaults() {
        let resolved = ResolvedDebugRelayOptions {
            relays: Some(vec!["wss://debug.example.com/".to_string()]),
            block_discovery: true,
            source: Some(DebugRelayConfigSource::Cli),
        };

        let (relay_config, startup_relays) = build_startup_relay_config(&resolved);

        assert_eq!(
            relay_config.debug_relays,
            Some(vec!["wss://debug.example.com/".to_string()])
        );
        assert!(relay_config.block_discovery);
        assert!(startup_relays.is_empty());
    }

    #[test]
    fn test_startup_relay_config_uses_default_startup_relays_without_debug_mode() {
        let resolved = ResolvedDebugRelayOptions {
            relays: None,
            block_discovery: false,
            source: None,
        };

        let (relay_config, startup_relays) = build_startup_relay_config(&resolved);

        assert_eq!(relay_config.debug_relays, None);
        assert!(!relay_config.block_discovery);
        assert_eq!(
            startup_relays,
            DEFAULT_RELAYS
                .iter()
                .map(|s| s.to_string())
                .collect::<Vec<_>>()
        );
    }
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
    arcadestr_core::signers::set_keys_dir(keys_dir.clone());
    info!("NIP-46 keys directory: {}", keys_dir.display());

    // Set the users directory for saved users
    arcadestr_core::saved_users::set_users_dir(keys_dir.clone());
    info!("Saved users directory: {}", keys_dir.display());

    // Set the profile metadata cache directory for NIP-46
    arcadestr_core::nip46::set_profile_cache_dir(keys_dir.clone());
    info!("Profile metadata cache directory: {}", keys_dir.display());

    // Initialize database pool for persistent storage FIRST
    let db_path = keys_dir.join("arcadestr.db");

    // Create subscription_registry BEFORE the async block (needed for validator spawn order)
    let subscription_registry = Arc::new(SubscriptionRegistry::new());

    let network_settings = try_load_network_discovery_settings().unwrap_or_else(|e| {
        eprintln!("Invalid network discovery settings: {}", e);
        std::process::exit(2);
    });

    let debug_relay_options = {
        let cli =
            parse_debug_relay_cli_args(std::env::args().skip(1).collect()).unwrap_or_else(|e| {
                eprintln!("Invalid debug relay CLI options: {}", e);
                std::process::exit(2);
            });

        let env = parse_debug_relay_env(
            std::env::var("ARCADESTR_RELAYS").ok(),
            std::env::var("ARCADESTR_BLOCK_DISCOVERY").ok(),
        )
        .unwrap_or_else(|e| {
            eprintln!("Invalid debug relay environment options: {}", e);
            std::process::exit(2);
        });

        resolve_debug_relay_options(cli, env, &network_settings).unwrap_or_else(|e| {
            eprintln!("Invalid debug relay configuration: {}", e);
            std::process::exit(2);
        })
    };

    if let Some(relays) = &debug_relay_options.relays {
        info!(
            "Debug relay mode active from {:?}: {} relay(s), block_discovery={}",
            debug_relay_options.source,
            relays.len(),
            debug_relay_options.block_discovery
        );
    }

    // Create a single runtime for all initialization
    let runtime = tokio::runtime::Runtime::new().expect("Failed to create Tokio runtime");

    let (database, nostr_client, user_cache, marketplace_cache, purchases, nip05_validator) =
        runtime.block_on(async {
            // Initialize database
            let db = arcadestr_core::storage::Database::new(&db_path)
                .await
                .expect("Failed to initialize database");

            let cache = Arc::new(UserCache::new(db.pool().clone()));

            // Initialize NostrClient with relay manager
            let (relay_config, startup_relays) = build_startup_relay_config(&debug_relay_options);

            let client = match NostrClient::new_with_cache(
                "default".to_string(),
                startup_relays,
                cache.clone(),
                Some(relay_config.clone()),
            )
            .await
            {
                Ok(c) => c,
                Err(e) => {
                    eprintln!("Warning: Failed to initialize NostrClient: {}", e);
                    eprintln!("The app will start but relay functionality may be limited.");
                    // Create a client with no relays - user can retry later
                    NostrClient::new_with_cache(
                        "default".to_string(),
                        vec![],
                        cache.clone(),
                        Some(relay_config.clone()),
                    )
                    .await
                    .expect("Failed to create empty client")
                }
            };

            // Wrap client in Arc for sharing
            let client = Arc::new(client);

            // Spawn NIP-05 background validator
            let validator_client = match NostrClient::new_with_cache(
                "default".to_string(),
                vec![],
                cache.clone(),
                Some(relay_config.clone()),
            )
            .await
            {
                Ok(c) => Arc::new(c),
                Err(e) => {
                    warn!("Failed to create validator client: {}", e);
                    client.clone() // Fallback to shared client
                }
            };
            let nip05_validator = Arc::new(std::sync::Mutex::new(Nip05Validator::spawn(
                validator_client,
                cache.clone(),
            )));
            info!("NIP-05 background validator spawned");

            // Unwrap Arc to return the client directly (it will be re-wrapped later)
            let client = match Arc::try_unwrap(client) {
                Ok(c) => c,
                Err(_) => {
                    // If we can't unwrap (because validator is still using it), create a new empty client
                    warn!("Client is shared, creating new client for main use");
                    NostrClient::new_with_cache(
                        "default".to_string(),
                        vec![],
                        cache.clone(),
                        Some(relay_config.clone()),
                    )
                    .await
                    .expect("Failed to create fallback client")
                }
            };

            // Connect to configured relays immediately to reduce query latency
            info!("Connecting to configured relays before starting Tauri...");
            client.connect().await;
            info!("Configured relay connections initiated");

            // Start connection monitoring for real-time relay status updates
            {
                let relay_manager = client.relay_manager();
                let manager = relay_manager.lock().await;
                manager.start_connection_monitor().await;
                info!("Relay connection monitoring started");
            }

            let marketplace_cache = Arc::new(MarketplaceCache::new(db.pool().clone()));
            let purchases = Arc::new(arcadestr_core::purchases::PurchasesRepository::new(
                db.pool().clone(),
            ));

            (
                db,
                client,
                cache,
                marketplace_cache,
                purchases,
                nip05_validator,
            )
        });
    info!("Database initialized at: {}", db_path.display());
    info!("UserCache initialized");

    let http_client: Arc<dyn HttpClient> = match ReqwestHttpClient::new(Duration::from_secs(10)) {
        Ok(client) => Arc::new(client),
        Err(error) => {
            eprintln!("Failed to initialize HTTP client: {}", error);
            return;
        }
    };

    // Initialize RelayCache for NIP-65 relay list management
    let relay_cache =
        RelayCache::new(keys_dir.join("relay_cache.db")).expect("Failed to create relay cache");

    // Initialize RelayHints for extracting relay URLs from p-tags
    let relay_hints = Arc::new(
        RelayHints::new(keys_dir.join("relay_hints.db"))
            .expect("Failed to create relay hint store"),
    );
    info!("RelayHints initialized");

    let deduplicator = EventDeduplicator::new(10000);

    // Wrap in Arc for sharing across tasks
    let nostr_client = Arc::new(tokio::sync::Mutex::new(nostr_client));
    let relay_cache = Arc::new(relay_cache);

    // Initialize ProfileFetcher with persistent cache and NIP-05 validator
    let profile_fetcher = Arc::new({
        let mut fetcher = ProfileFetcher::with_persistent_cache(user_cache.clone());
        fetcher.with_nip05_validator(nip05_validator.clone());
        fetcher
    });
    info!("ProfileFetcher initialized with persistent cache and NIP-05 validator");

    // Load cached profiles on startup
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        let cached = profile_fetcher.load_cached_profiles().await;
        info!("Loaded {} cached profiles on startup", cached.len());
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
        use arcadestr_core::saved_users::{
            add_saved_user as save_user, create_saved_user, LoginMethod,
        };

        // Handle various method names from frontend
        let login_method = match method.as_str() {
            "nostrconnect" | "nip46" | "nostrconnect_uri" => LoginMethod::Nostrconnect,
            "bunker" | "bunker_uri" => LoginMethod::Bunker,
            "direct_key" | "private_key" | "key" => LoginMethod::DirectKey,
            "nip07" => LoginMethod::DirectKey, // NIP-07 uses same reconnection as direct key
            _ => {
                // Default to Nostrconnect for unknown methods
                tracing::warn!(
                    "Unknown login method '{}', defaulting to Nostrconnect",
                    method
                );
                LoginMethod::Nostrconnect
            }
        };

        let user = create_saved_user(login_method, relay, uri, private_key, &npub);

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
    /// Returns JSON with npub and user profile.
    #[tauri::command]
    async fn connect_saved_user(
        user_id: String,
        state: tauri::State<'_, AppState>,
        app_handle: tauri::AppHandle,
    ) -> Result<serde_json::Value, String> {
        use arcadestr_core::saved_users::{get_saved_user, mark_user_as_used, LoginMethod};
        use arcadestr_core::signers::SignerError;

        let user = get_saved_user(&user_id)?;

        let mut auth = state.auth.lock().await;

        match user.method {
            LoginMethod::DirectKey => {
                if let Some(key) = user.private_key {
                    auth.connect_with_key(&key)
                        .map_err(|e: SignerError| e.to_string())?;
                    let _ = mark_user_as_used(&user_id);

                    let pubkey = auth.public_key().ok_or("Public key not available")?;

                    // IMPORTANT: Initialize relay gossip BEFORE returning
                    // This ensures user's relays are connected before fetch_profile is called
                    let user_npub = pubkey.to_bech32().unwrap_or_default();
                    let state_nostr = state.nostr.clone();
                    let state_cache = state.relay_cache.clone();
                    let state_registry = state.subscription_registry.clone();
                    let state_profile_fetcher = state.profile_fetcher.clone();

                    // Drop the auth lock before awaiting
                    drop(auth);

                    // Wait for relay gossip initialization to complete and get profile
                    let (user_profile, followed) = initialize_relay_gossip(
                        state_nostr,
                        state_cache,
                        state.relay_hints.clone(),
                        state_registry,
                        state_profile_fetcher,
                        app_handle.clone(),
                        user_npub.clone(),
                        Some(user_id.clone()),
                        user.relay.clone(), // Pass the relay from saved user
                    )
                    .await;

                    // Initialize extended network discovery
                    initialize_extended_network(&state, &user_npub, followed, app_handle).await;

                    Ok(serde_json::json!({
                        "npub": user_npub,
                        "profile": user_profile
                    }))
                } else {
                    Err("No private key found for this user".to_string())
                }
            }
            LoginMethod::Nostrconnect | LoginMethod::Bunker => {
                // For nostrconnect/bunker, we reconnect using the client keys
                // The signer will remember the approval based on the client public key
                let relay = user
                    .relay
                    .clone()
                    .unwrap_or_else(|| "wss://relay.nsec.app".to_string());

                // Build URI from client keys (we already have them saved)
                let uri_str = user.uri.clone().unwrap_or_else(|| {
                    // If no URI saved, generate a new nostrconnect URI with the same keys
                    // This works because the signer remembers the client pubkey
                    format!("nostrconnect://?relay={}", relay)
                });

                match auth.connect_nip46(&uri_str, &relay).await {
                    Ok(_) => {
                        let _ = mark_user_as_used(&user_id);
                        let pubkey = auth
                            .public_key()
                            .ok_or("Public key not available after connection")?;

                        // IMPORTANT: Initialize relay gossip BEFORE returning
                        let user_npub = pubkey.to_bech32().unwrap_or_default();
                        let state_nostr = state.nostr.clone();
                        let state_cache = state.relay_cache.clone();
                        let state_registry = state.subscription_registry.clone();
                        let state_profile_fetcher = state.profile_fetcher.clone();

                        // Drop the auth lock before awaiting
                        drop(auth);

                        // Wait for relay gossip initialization to complete and get profile
                        let (user_profile, followed) = initialize_relay_gossip(
                            state_nostr,
                            state_cache,
                            state.relay_hints.clone(),
                            state_registry,
                            state_profile_fetcher,
                            app_handle.clone(),
                            user_npub.clone(),
                            Some(user_id.clone()),
                            Some(relay), // Pass the relay from NIP-46 connection
                        )
                        .await;

                        // Initialize extended network discovery
                        initialize_extended_network(&state, &user_npub, followed, app_handle).await;

                        // Return both npub and profile
                        Ok(serde_json::json!({
                            "npub": user_npub,
                            "profile": user_profile
                        }))
                    }
                    Err(e) => Err(format!("Connection failed: {}", e)),
                }
            }
        }
    }

    /// Fetches a user profile using NIP-19 hints (nprofile/nevent) for relay hints.
    ///
    /// This command parses a NIP-19 identifier (nprofile or nevent), connects to the
    /// hint relays, fetches the user's relay list (NIP-65), caches it, and then
    /// fetches the profile data.
    ///
    /// # Arguments
    /// * `identifier` - NIP-19 identifier (nprofile or nevent)
    ///
    /// # Returns
    /// The user profile on success.
    #[tauri::command]
    async fn fetch_profile_with_hints(
        identifier: String,
        state: tauri::State<'_, AppState>,
    ) -> Result<UserProfile, String> {
        // Parse NIP-19 identifier
        let parsed = parse_nip19_identifier(&identifier).map_err(|e| e.to_string())?;

        let nostr = state.nostr.lock().await;
        let cache = state.relay_cache.clone();

        // Connect to hint relays if present
        for hint in &parsed.relays {
            let _ = nostr.add_relay(hint).await;
        }

        // Fetch relay list and cache it
        let npub = format!("npub1{}", &parsed.pubkey[4.min(parsed.pubkey.len())..]);
        if let Ok(relays) = nostr.fetch_relay_list(&npub).await {
            let _ = cache.save_relay_list(&relays);
        }

        // Fetch profile
        nostr
            .fetch_profile(&npub, None)
            .await
            .map_err(|e| e.to_string())
    }

    /// Perform post-authentication relay discovery and start subscriptions
    /// with batched profile fetching and progress tracking
    /// Returns the user profile and follow list that were fetched
    async fn initialize_relay_gossip(
        nostr: Arc<Mutex<NostrClient>>,
        relay_cache: Arc<RelayCache>,
        relay_hints: Option<Arc<RelayHints>>,
        subscription_registry: Arc<SubscriptionRegistry>,
        profile_fetcher: Arc<ProfileFetcher>,
        app_handle: tauri::AppHandle,
        user_npub: String,
        user_id: Option<String>,
        bunker_relay: Option<String>, // NEW PARAMETER
    ) -> (UserProfile, Vec<String>) {
        use arcadestr_core::nostr::{build_relay_map, score_relays, select_relays};
        use arcadestr_core::CachedRelayList;
        use std::collections::HashSet;

        let nostr_client = nostr.lock().await;

        // Load persisted relays for this profile and add them to the pool
        if let Some(ref profile_id) = user_id {
            match relay_cache.load_relay_pool(profile_id) {
                Ok(persisted_relays) if !persisted_relays.is_empty() => {
                    tracing::info!(
                        "Loading {} persisted relays for profile {}",
                        persisted_relays.len(),
                        profile_id
                    );
                    for relay in &persisted_relays {
                        let _ = nostr_client.add_relay(relay).await;
                    }
                }
                Ok(_) => {
                    tracing::info!("No persisted relays found for profile {}", profile_id);
                }
                Err(e) => {
                    tracing::warn!(
                        "Failed to load persisted relays for profile {}: {}",
                        profile_id,
                        e
                    );
                }
            }
        }

        // Add bunker relay if provided (for NIP-46 connections)
        if let Some(ref relay) = bunker_relay {
            tracing::info!("Adding bunker relay from NIP-46: {}", relay);
            match nostr_client.add_relay(relay).await {
                Ok(_) => {
                    tracing::info!("Successfully added bunker relay: {}", relay);
                    // Connect to the relay immediately
                    nostr_client.connect().await;
                    tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
                }
                Err(e) => {
                    tracing::warn!("Failed to add bunker relay {}: {}", relay, e);
                }
            }
        }

        // FAST PATH: Fetch logged-in user's profile immediately from indexers
        tracing::info!("Fast path: fetching user profile for {}", user_npub);
        let mut user_profile = UserProfile {
            npub: user_npub.clone(),
            ..Default::default()
        };

        if let Some(profile) = profile_fetcher
            .fetch_single(&nostr_client, &user_npub)
            .await
        {
            tracing::info!(
                "User profile loaded: name={:?}, display_name={:?}, picture={:?}",
                profile.name,
                profile.display_name,
                profile.picture
            );
            user_profile = profile.clone();
            // Emit event to update UI immediately
            let _ = app_handle.emit("user_profile_loaded", profile.clone());

            // Save profile to saved user if we have a user_id
            if let Some(ref uid) = user_id {
                tracing::info!(
                    "Saving profile to saved user {}: display_name={:?}, name={:?}, picture={:?}",
                    uid,
                    profile.display_name,
                    profile.name,
                    profile.picture
                );
                let result = arcadestr_core::saved_users::update_user_profile(
                    uid,
                    profile.display_name.clone(),
                    profile.name.clone(),
                    profile.picture.clone(),
                    profile.nip05.clone(),
                    profile.about.clone(),
                );
                match result {
                    Ok(_) => tracing::info!("Profile saved successfully"),
                    Err(e) => tracing::error!("Failed to save profile: {}", e),
                }
            }
        }

        // Step 1: Fetch user's metadata (profile + relay list) from indexers
        tracing::info!("Fetching user metadata from indexers for {}", user_npub);
        let user_relays = match nostr_client.fetch_user_metadata(&user_npub).await {
            Ok((profile, relays)) => {
                if let Some(ref r) = &relays {
                    tracing::info!(
                        "Found {} write relays, {} read relays for user",
                        r.write_relays.len(),
                        r.read_relays.len()
                    );
                }
                // Update user_profile with the fetched profile
                user_profile = profile.clone();

                // Save profile to saved user if we have a user_id
                if let Some(ref uid) = user_id {
                    tracing::info!("Saving metadata profile to saved user {}: display_name={:?}, name={:?}, picture={:?}",
                        uid, profile.display_name, profile.name, profile.picture);
                    let result = arcadestr_core::saved_users::update_user_profile(
                        uid,
                        profile.display_name.clone(),
                        profile.name.clone(),
                        profile.picture.clone(),
                        profile.nip05.clone(),
                        profile.about.clone(),
                    );
                    match result {
                        Ok(_) => tracing::info!("Metadata profile saved successfully"),
                        Err(e) => tracing::error!("Failed to save metadata profile: {}", e),
                    }
                }

                relays
            }
            Err(e) => {
                tracing::warn!(
                    "Failed to fetch user metadata from indexers: {}. Using default relays.",
                    e
                );
                None
            }
        };

        // Save user's relay list to cache
        if let Some(ref relays) = user_relays {
            let _ = relay_cache.save_relay_list(relays);

            // Connect to user's write relays first
            for relay in &relays.write_relays {
                tracing::info!("Adding user's relay: {}", relay);
                let _ = nostr_client.add_relay(relay).await;
            }

            // Also connect to read relays
            for relay in &relays.read_relays {
                if !relays.write_relays.contains(relay) {
                    tracing::info!("Adding user's read relay: {}", relay);
                    let _ = nostr_client.add_relay(relay).await;
                }
            }

            // Connect to all added relays
            nostr_client.connect().await;

            tracing::info!(
                "Skipping blocking relay readiness wait before follow list fetch (startup fast path)"
            );
        }

        // Step 2: Now fetch the user's follow list (should be on their relays)
        tracing::info!("Fetching follow list for {}", user_npub);
        let followed = match nostr_client.fetch_follow_list(&user_npub).await {
            Ok(list) => {
                tracing::info!("Found {} followed pubkeys", list.len());
                list
            }
            Err(e) => {
                tracing::warn!("Failed to fetch follow list: {}", e);
                vec![] // Continue with empty follow list
            }
        };

        // Step 3: BATCHED PROFILE FETCHING for followed users
        if !followed.is_empty() {
            let total = followed.len();
            tracing::info!("Queueing {} profiles for batched fetching", total);

            // Emit initial progress
            let _ = app_handle.emit(
                "profile_fetch_progress",
                ProfileFetchProgress {
                    completed: 0,
                    total,
                },
            );

            // Queue all followed profiles
            profile_fetcher.enqueue_many(followed.clone());

            // Also fast-path fetch any profiles that appear in feed immediately
            // (first 10 followed users get priority)
            let priority_users: Vec<String> = followed.iter().take(10).cloned().collect();
            for pubkey in &priority_users {
                if let Some(profile) = profile_fetcher.fetch_single(&nostr_client, pubkey).await {
                    let _ = app_handle.emit("profile_fetched", profile);
                }
            }

            // Process remaining in batches with progress updates
            let mut completed = priority_users.len();
            loop {
                let (batch, remaining) = profile_fetcher.fetch_batch(&nostr_client).await;
                if batch.is_empty() {
                    break;
                }
                completed += batch.len();

                // Emit progress update
                let _ = app_handle.emit(
                    "profile_fetch_progress",
                    ProfileFetchProgress { completed, total },
                );

                // Emit individual profiles for UI updates
                for profile in batch {
                    let _ = app_handle.emit("profile_fetched", profile);
                }

                // Small delay between batches to prevent overwhelming
                tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
            }

            tracing::info!("Completed fetching {} profiles", completed);
        }

        // Step 4: Fetch relay lists for followed pubkeys
        for pubkey in &followed {
            match nostr_client.fetch_relay_list(pubkey).await {
                Ok(relays) => {
                    let _ = relay_cache.save_relay_list(&relays);
                }
                Err(_) => {
                    // Fallback to seen_on if no relay list
                    let seen = relay_cache.get_seen_on(pubkey);
                    if !seen.is_empty() {
                        let fallback = CachedRelayList {
                            pubkey: pubkey.clone(),
                            write_relays: seen.clone(),
                            read_relays: seen,
                            updated_at: 0,
                        };
                        let _ = relay_cache.save_relay_list(&fallback);
                    }
                }
            }
        }

        // Step 5: Build relay map and select optimal relays
        let all_pubkeys: HashSet<_> = followed.iter().cloned().collect();
        let relay_map = build_relay_map(&followed, &relay_cache);
        let scored = score_relays(&relay_map, &relay_cache, Some(&user_npub));
        let selection = select_relays(scored, 10, &all_pubkeys);

        tracing::info!("Selected {} permanent relays", selection.permanent.len());

        // Add selected relays for followed users
        for relay in &selection.permanent {
            let _ = nostr_client.add_relay(relay).await;
        }

        nostr_client.connect().await;

        // Get the inner client for subscription dispatch
        drop(nostr_client);

        // Activate ephemeral subscriptions for uncovered pubkeys
        if !selection.uncovered_pubkeys.is_empty() {
            let client = nostr.lock().await;
            let manager = client.relay_manager();
            drop(client);

            let (inner_client, blocks_discovery, debug_relays) = {
                let manager_guard = manager.lock().await;
                (
                    manager_guard.get_client_arc(),
                    manager_guard.blocks_discovery(),
                    manager_guard.debug_relays(),
                )
            };

            dispatch_ephemeral_reads_batch_with_policy(
                inner_client.as_ref(),
                &selection.uncovered_pubkeys,
                &relay_cache,
                &subscription_registry,
                blocks_discovery,
                debug_relays.as_deref(),
            )
            .await;

            info!(
                "Activated ephemeral subscriptions for {} uncovered pubkeys",
                selection.uncovered_pubkeys.len()
            );
        }

        // Schedule background refresh with recurring timer
        let cache_for_refresh = relay_cache.clone();
        let nostr_for_refresh = nostr.clone();
        let profile_id_for_refresh = user_id.clone().unwrap_or_else(|| "default".to_string());
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(tokio::time::Duration::from_secs(3600)).await; // check every hour
                refresh_stale_relays(
                    nostr_for_refresh.clone(),
                    cache_for_refresh.clone(),
                    profile_id_for_refresh.clone(),
                )
                .await;
            }
        });

        // Return the user profile and follow list
        (user_profile, followed)
    }

    /// Progress structure for profile fetching
    #[derive(serde::Serialize, Clone)]
    struct ProfileFetchProgress {
        completed: usize,
        total: usize,
    }

    /// Get fallback relay for a pubkey using the 4-tier waterfall
    async fn get_fallback_relay(
        pubkey: &str,
        nostr: &Arc<Mutex<NostrClient>>,
        relay_cache: &Arc<RelayCache>,
        relay_hints: &Option<Arc<RelayHints>>,
        user_npub: &str,
    ) -> String {
        let nostr_client = nostr.lock().await;

        // Use the 4-tier waterfall discovery
        let result = match relay_hints {
            Some(hints) => {
                nostr_client
                    .get_relays_for_pubkey_with_hints(pubkey, relay_cache, Some(hints.as_ref()))
                    .await
            }
            None => {
                nostr_client
                    .get_relays_for_pubkey_with_hints(pubkey, relay_cache, None)
                    .await
            }
        };

        // Return first relay from the discovered list, or fallback to default
        result
            .write_relays
            .first()
            .cloned()
            .unwrap_or_else(|| DEFAULT_RELAYS[0].to_string())
    }

    /// Refreshes stale relay lists for followed users.
    async fn refresh_stale_relays(
        nostr: Arc<Mutex<NostrClient>>,
        relay_cache: Arc<RelayCache>,
        profile_id: String,
    ) {
        let stale_pubkeys = relay_cache.get_stale_pubkeys();

        if stale_pubkeys.is_empty() {
            return;
        }

        tracing::info!("Refreshing {} stale relay lists", stale_pubkeys.len());

        let mut nostr_client = nostr.lock().await;

        for pubkey in stale_pubkeys {
            let npub = if pubkey.starts_with("npub1") {
                pubkey.clone()
            } else {
                format!("npub1{}", &pubkey[4..])
            };

            match nostr_client.fetch_relay_list(&npub).await {
                Ok(relays) => {
                    let _ = relay_cache.save_relay_list(&relays);

                    // Add discovered relays to unified pool
                    let manager = nostr_client.relay_manager();
                    let manager_guard = manager.lock().await;
                    for relay in &relays.write_relays {
                        let _ = manager_guard.add_discovered_relay(relay.clone()).await;
                    }
                    for relay in &relays.read_relays {
                        let _ = manager_guard.add_discovered_relay(relay.clone()).await;
                    }

                    // Persist the updated pool
                    let pool = manager_guard.get_relay_pool().await;
                    let all_relays: Vec<String> = pool.get_relays().await;
                    let _ = relay_cache.save_relay_pool(&profile_id, &all_relays);
                }
                Err(e) => {
                    tracing::debug!("Failed to refresh {}: {}", pubkey, e);
                }
            }
        }
    }

    /// Initialize extended network discovery after authentication.
    /// This sets up the social graph DB and starts background discovery.
    async fn initialize_extended_network(
        state: &tauri::State<'_, AppState>,
        user_npub: &str,
        followed: Vec<String>,
        app_handle: tauri::AppHandle,
    ) {
        use tracing::{info, warn};

        {
            let relay_manager = {
                let nostr = state.nostr.lock().await;
                nostr.relay_manager()
            };
            let manager = relay_manager.lock().await;

            if manager.blocks_discovery() {
                info!(
                    "Skipping extended network discovery because debug relay discovery is blocked"
                );
                return;
            }
        }

        // Get config directory for database paths
        let config_dir = dirs::data_local_dir()
            .unwrap_or_else(|| std::path::PathBuf::from("."))
            .join("arcadestr");

        // Initialize social graph database
        let social_graph = match SocialGraphDb::new(config_dir.join("social_graph.db")) {
            Ok(db) => Arc::new(db),
            Err(e) => {
                warn!("Failed to create social graph DB: {}", e);
                return;
            }
        };

        // Create extended network repository
        let extended_network = Arc::new(Mutex::new(ExtendedNetworkRepository::new(social_graph)));

        // Set the user's pubkey
        {
            let mut repo = extended_network.lock().await;
            repo.set_pubkey(user_npub.to_string());
            let settings = load_network_discovery_settings();
            repo.set_allow_insecure_public_ws(settings.allow_insecure_public_ws);
        }

        // Store in app state using interior mutability
        {
            let mut en_slot = state.extended_network.write().await;
            *en_slot = Some(extended_network.clone());
            info!("ExtendedNetworkRepository stored in AppState");
        }

        // Store follows list for refresh cycles
        {
            let mut follows_slot = state.extended_network_follows.write().await;
            *follows_slot = followed.clone();
            info!(
                "Extended network follows list stored ({} follows)",
                followed.len()
            );
        }

        // Spawn discovery task
        let nostr = state.nostr.clone();
        let relay_cache = state.relay_cache.clone();
        let en_repo = extended_network.clone();

        tokio::spawn(async move {
            let mut repo = en_repo.lock().await;
            let nostr_client = nostr.lock().await;
            match repo
                .discover_network(&*nostr_client, &relay_cache, followed)
                .await
            {
                Ok(stats) => {
                    info!(
                        "Extended network discovery complete: {} qualified, {} relays",
                        stats.qualified_count, stats.relays_covered
                    );

                    // Get discovered relays and connect them
                    let relay_configs = repo.get_relay_configs();
                    drop(repo);
                    drop(nostr_client);

                    // Connect to extended network relays
                    let nostr_client = nostr.lock().await;
                    let mut connected_count = 0;
                    for relay_url in relay_configs {
                        // Skip if already connected (check against default relays)
                        if DEFAULT_RELAYS.contains(&relay_url.as_str()) {
                            continue;
                        }

                        match nostr_client.add_relay(&relay_url).await {
                            Ok(_) => {
                                connected_count += 1;
                                tracing::info!("Connected extended network relay: {}", relay_url);
                            }
                            Err(e) => {
                                tracing::debug!(
                                    "Failed to add extended network relay {}: {}",
                                    relay_url,
                                    e
                                );
                            }
                        }
                    }

                    if connected_count > 0 {
                        nostr_client.connect().await;
                        info!(
                            "Connected {} extended network relays to gossip",
                            connected_count
                        );
                    }

                    // Emit event to notify UI
                    let _ = app_handle.emit("extended_network_discovered", stats);
                }
                Err(e) => {
                    warn!("Extended network discovery failed: {}", e);
                }
            }
        });

        // Spawn periodic extended network refresh task (every 24 hours)
        let en_for_refresh = state.extended_network.clone();
        let follows_for_refresh = state.extended_network_follows.clone();
        let nostr_for_refresh = state.nostr.clone();
        let relay_cache_for_refresh = state.relay_cache.clone();

        tokio::spawn(async move {
            let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(86400)); // 24 hours
            loop {
                interval.tick().await;

                // Read repository from AppState
                let en_option = en_for_refresh.read().await;
                if let Some(ref repo) = *en_option {
                    let repo_lock = repo.lock().await;

                    // Only refresh if stale
                    if repo_lock.is_cache_stale() {
                        info!("Extended network cache is stale, starting refresh...");

                        // Clone the Arc before dropping the read lock
                        let repo_clone = repo.clone();
                        drop(repo_lock);
                        drop(en_option);

                        // Get follows list (try stored first, we'll add network re-fetch in future enhancement)
                        let follows = {
                            let follows_guard = follows_for_refresh.read().await;
                            follows_guard.clone()
                        };

                        if !follows.is_empty() {
                            // Perform refresh
                            let mut repo_lock = repo_clone.lock().await;
                            let nostr_client = nostr_for_refresh.lock().await;

                            match repo_lock
                                .discover_network(&*nostr_client, &relay_cache_for_refresh, follows)
                                .await
                            {
                                Ok(stats) => {
                                    info!("Extended network refresh complete: {} qualified, {} relays",
                                        stats.qualified_count, stats.relays_covered);

                                    // Connect to newly discovered relays
                                    let relay_configs = repo_lock.get_relay_configs();
                                    drop(repo_lock);
                                    drop(nostr_client);

                                    // Add and connect new relays
                                    let nostr_client = nostr_for_refresh.lock().await;
                                    let mut connected_count = 0;
                                    for relay_url in relay_configs {
                                        if let Err(e) = nostr_client.add_relay(&relay_url).await {
                                            tracing::debug!(
                                                "Failed to add extended network relay {}: {}",
                                                relay_url,
                                                e
                                            );
                                        } else {
                                            connected_count += 1;
                                        }
                                    }
                                    if connected_count > 0 {
                                        nostr_client.connect().await;
                                        info!(
                                            "Connected {} extended network relays",
                                            connected_count
                                        );
                                    }
                                }
                                Err(e) => {
                                    warn!("Extended network refresh failed: {}", e);
                                }
                            }
                        } else {
                            info!("No follows list available for extended network refresh");
                        }
                    }
                }
            }
        });
    }

    /// Get the number of currently connected relays.
    #[tauri::command]
    async fn get_connected_relay_count(state: tauri::State<'_, AppState>) -> Result<usize, String> {
        let manager_arc = {
            let nostr = state.nostr.lock().await;
            nostr.relay_manager()
        };
        let manager = manager_arc.lock().await;
        Ok(manager.get_connected_count().await)
    }

    /// Get the list of currently connected relay URLs.
    #[tauri::command]
    async fn get_connected_relays(
        state: tauri::State<'_, AppState>,
    ) -> Result<Vec<String>, String> {
        let manager_arc = {
            let nostr = state.nostr.lock().await;
            nostr.relay_manager()
        };
        let manager = manager_arc.lock().await;
        Ok(manager
            .get_connected_relays()
            .await
            .into_iter()
            .filter_map(|status| status.connected.then_some(status.url))
            .collect())
    }

    /// Get extended network discovery statistics.
    /// Returns None if extended network discovery hasn't been initialized yet.
    #[tauri::command]
    async fn get_extended_network_stats(
        state: tauri::State<'_, AppState>,
    ) -> Result<Option<arcadestr_core::extended_network::NetworkStats>, String> {
        let en_option = state.extended_network.read().await;
        if let Some(ref repo) = *en_option {
            let repo_lock = repo.lock().await;
            Ok(repo_lock.get_cached_network().map(|cache| cache.stats))
        } else {
            Ok(None)
        }
    }

    /// Get relay hints for a specific pubkey.
    /// Returns empty vector if no hints available or if relay hints not initialized.
    #[tauri::command]
    async fn get_relay_hints_for_pubkey(
        state: tauri::State<'_, AppState>,
        pubkey: String,
    ) -> Result<Vec<String>, String> {
        if let Some(ref hints) = state.relay_hints {
            match hints.get_hints(&pubkey) {
                Ok(hints) => Ok(hints),
                Err(e) => Err(format!("Failed to get relay hints: {}", e)),
            }
        } else {
            Ok(Vec::new())
        }
    }

    /// Test command for extended network discovery (debug builds only).
    /// Forces a fresh discovery run and returns detailed statistics.
    #[cfg(debug_assertions)]
    #[tauri::command]
    async fn test_extended_network_discovery(
        state: tauri::State<'_, AppState>,
    ) -> Result<serde_json::Value, String> {
        use tracing::{debug, info, warn};

        // Get the extended network repository
        let en_option = state.extended_network.read().await;
        let repo = match *en_option {
            Some(ref repo) => repo.clone(),
            None => return Err("Extended network not initialized".to_string()),
        };
        drop(en_option);

        // Get follows list
        let follows = {
            let follows_guard = state.extended_network_follows.read().await;
            follows_guard.clone()
        };

        if follows.is_empty() {
            return Err("No follows list available".to_string());
        }

        info!("Test discovery: Starting with {} follows", follows.len());

        // Perform discovery
        let mut repo_lock = repo.lock().await;
        let nostr_client = state.nostr.lock().await;

        match repo_lock
            .discover_network(&*nostr_client, &state.relay_cache, follows)
            .await
        {
            Ok(stats) => {
                info!(
                    "Test discovery complete: {} qualified, {} relays",
                    stats.qualified_count, stats.relays_covered
                );

                // Get additional details
                let relay_configs = repo_lock.get_relay_configs();
                drop(repo_lock);
                drop(nostr_client);

                // Build detailed response
                let result = serde_json::json!({
                    "first_degree_count": stats.first_degree_count,
                    "total_second_degree": stats.total_second_degree,
                    "qualified_count": stats.qualified_count,
                    "relays_covered": stats.relays_covered,
                    "computed_at": stats.computed_at,
                    "computed_relays": relay_configs,
                    "success": true,
                });

                Ok(result)
            }
            Err(e) => {
                warn!("Test discovery failed: {}", e);
                Err(format!("Discovery failed: {}", e))
            }
        }
    }

    /// Fetch and save profile for the current authenticated user.
    /// This is called when the app initializes to update saved user metadata.
    #[tauri::command]
    async fn fetch_and_save_user_profile(
        app: tauri::AppHandle,
        state: tauri::State<'_, AppState>,
    ) -> Result<UserProfile, String> {
        use arcadestr_core::saved_users::{load_saved_users, update_user_profile};

        let auth = state.auth.lock().await;
        let npub = auth
            .public_key()
            .ok_or("Not authenticated")?
            .to_bech32()
            .map_err(|e| e.to_string())?;
        drop(auth);

        tracing::info!("fetch_and_save_user_profile called for npub: {}", npub);

        // Find the saved user with this npub
        let users = load_saved_users()?;
        tracing::info!("Loaded {} saved users", users.users.len());

        let user = users
            .users
            .iter()
            .find(|u| u.npub == npub)
            .cloned()
            .ok_or_else(|| {
                tracing::error!("User with npub {} not found in saved users", npub);
                "User not found in saved users".to_string()
            })?;

        tracing::info!("Found saved user: id={}, name={}", user.id, user.name);

        // Get the bunker relay from NIP-46 session
        let signer_state = app.state::<Arc<Mutex<AppSignerState>>>();
        let signer_state_guard = signer_state.lock().await;

        let bunker_relays: Vec<String> =
            if let Some(ref profile_id) = signer_state_guard.active_profile_id {
                // Get the bunker pubkey from metadata
                if let Some(metadata) = get_profile_metadata_by_id(profile_id) {
                    if let Some(profile) = load_profile_from_keyring(&metadata.bunker_pubkey_hex) {
                        // Extract relay URLs from bunker_uri
                        profile
                            .bunker_uri
                            .relays()
                            .iter()
                            .map(|url| url.to_string())
                            .collect()
                    } else {
                        vec![]
                    }
                } else {
                    vec![]
                }
            } else {
                vec![]
            };

        drop(signer_state_guard);

        tracing::info!(
            "Using {} bunker relays from NIP-46: {:?}",
            bunker_relays.len(),
            bunker_relays
        );

        // Fetch profile with bunker relays
        let nostr = state.nostr.lock().await;
        let profile = match nostr.fetch_profile(&npub, Some(bunker_relays)).await {
            Ok(p) => {
                tracing::info!(
                    "Profile fetched: name={:?}, display_name={:?}, picture={:?}",
                    p.name,
                    p.display_name,
                    p.picture
                );
                p
            }
            Err(e) => {
                tracing::error!("Failed to fetch profile: {}", e);
                return Err(e.to_string());
            }
        };
        drop(nostr);

        // Save profile to saved user
        tracing::info!(
            "Saving profile to saved user {}: display_name={:?}, name={:?}, picture={:?}",
            user.id,
            profile.display_name,
            profile.name,
            profile.picture
        );

        let result = update_user_profile(
            &user.id,
            profile.display_name.clone(),
            profile.name.clone(),
            profile.picture.clone(),
            profile.nip05.clone(),
            profile.about.clone(),
        );

        match result {
            Ok(_) => {
                tracing::info!("Profile saved successfully for user {}", npub);
                Ok(profile)
            }
            Err(e) => {
                tracing::error!("Failed to save profile: {}", e);
                Err(e)
            }
        }
    }

    /// Get all cached profiles from SQLite
    #[tauri::command]
    async fn get_cached_profiles(
        state: tauri::State<'_, AppState>,
    ) -> Result<Vec<UserProfile>, String> {
        let cache = state.user_cache.clone();

        cache.get_all().await.map_err(|e| e.to_string())
    }

    /// Get a single cached profile by npub
    #[tauri::command]
    async fn get_cached_profile(
        npub: String,
        state: tauri::State<'_, AppState>,
    ) -> Result<Option<UserProfile>, String> {
        let cache = state.user_cache.clone();

        Ok(cache.get(&npub).await)
    }

    /// Get application version and revision info
    #[tauri::command]
    fn get_version_info() -> Result<VersionInfo, String> {
        Ok(command_contracts::version_info())
    }

    #[tauri::command]
    async fn get_cached_earned_badges(
        state: tauri::State<'_, AppState>,
        profile_pubkey: String,
    ) -> Result<Vec<arcadestr_core::achievements::EarnedBadgeSummary>, String> {
        command_contracts::get_cached_earned_badges(state.inner(), profile_pubkey)
            .await
            .map_err(|error| error.to_string())
    }

    #[tauri::command]
    async fn get_cached_profile_badges(
        state: tauri::State<'_, AppState>,
        profile_pubkey: String,
    ) -> Result<Vec<arcadestr_core::achievements::ProfileBadgeEntry>, String> {
        command_contracts::get_cached_profile_badges(state.inner(), profile_pubkey)
            .await
            .map_err(|error| error.to_string())
    }

    #[tauri::command]
    async fn fetch_earned_badges(
        state: tauri::State<'_, AppState>,
        profile_pubkey: String,
    ) -> Result<Vec<arcadestr_core::achievements::EarnedBadgeSummary>, String> {
        command_contracts::fetch_earned_badges(state.inner(), profile_pubkey)
            .await
            .map_err(|error| error.to_string())
    }

    #[tauri::command]
    async fn fetch_profile_badges(
        state: tauri::State<'_, AppState>,
        profile_pubkey: String,
    ) -> Result<Vec<arcadestr_core::achievements::ProfileBadgeEntry>, String> {
        command_contracts::fetch_profile_badges(state.inner(), profile_pubkey)
            .await
            .map_err(|error| error.to_string())
    }

    /// Version info structure for frontend
    type VersionInfo = command_contracts::VersionInfo;

    tauri::Builder::default()
        .manage(AppState {
            auth: Arc::new(Mutex::new(AuthState::new())),
            nostr: nostr_client.clone(),
            database: Arc::new(database),
            relay_cache: relay_cache.clone(),
            deduplicator: Arc::new(Mutex::new(deduplicator)),
            subscription_registry: subscription_registry.clone(),
            profile_fetcher,
            user_cache,
            marketplace_cache,
            purchases,
            nip05_validator,
            http_client,
            extended_network: Arc::new(RwLock::new(None)),
            extended_network_follows: Arc::new(RwLock::new(Vec::new())),
            relay_hints: Some(relay_hints.clone()),
        })
        .manage(Arc::new(Mutex::new(AppSignerState::new())))
        .setup(move |app| {
            // Ensure window is visible and focused
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.show();
                let _ = window.set_focus();
            }

            // Attempt to restore session on startup
            let app_handle = app.handle().clone();
            let signer_state: Arc<Mutex<AppSignerState>> = (*app.state::<Arc<Mutex<AppSignerState>>>()).clone();

            // Clone necessary state for extended network discovery after restore
            // We need to clone the Arc pointers directly, not through State reference
            let nostr_for_restore: Arc<Mutex<NostrClient>> = (*app.state::<AppState>()).nostr.clone();
            let relay_cache_for_restore: Arc<RelayCache> = (*app.state::<AppState>()).relay_cache.clone();
            let extended_network_for_restore = (*app.state::<AppState>()).extended_network.clone();
            let extended_network_follows_for_restore = (*app.state::<AppState>()).extended_network_follows.clone();

            // Use tauri's async runtime instead of tokio::spawn
            tauri::async_runtime::spawn(async move {
                info!("Attempting to restore session on startup...");

                // Emit restoring event
                let _ = app_handle.emit("session_restoring", ());

                // Attempt restore
                match restore_session_on_startup(&signer_state).await {
                    SessionRestoreResult::Success => {
                        info!("Session restored successfully on startup");
                        let _ = app_handle.emit("session_restored", ());

                        // Trigger extended network discovery after successful restore
                        // Check if discovery hasn't already been initialized
                        let en_option = extended_network_for_restore.read().await;
                        if en_option.is_none() {
                            drop(en_option); // Release read lock

                            let discovery_blocked = {
                                let relay_manager = {
                                    let nostr = nostr_for_restore.lock().await;
                                    nostr.relay_manager()
                                };
                                let manager = relay_manager.lock().await;
                                manager.blocks_discovery()
                            };

                            if discovery_blocked {
                                info!(
                                    "Skipping extended network discovery because debug relay discovery is blocked"
                                );
                            } else {
                                info!("Triggering extended network discovery after session restore");

                            // Get user npub from the restored session
                            let user_npub = {
                                let signer_guard = signer_state.lock().await;
                                if let Some(ref client) = signer_guard.active_client {
                                    // Get public key from the client
                                    match client.signer().await {
                                        Ok(signer) => {
                                            match signer.get_public_key().await {
                                                Ok(pubkey) => pubkey.to_bech32().unwrap_or_default(),
                                                Err(_) => {
                                                    warn!("Failed to get public key from signer");
                                                    String::new()
                                                }
                                            }
                                        }
                                        Err(_) => {
                                            warn!("Failed to get signer from client");
                                            String::new()
                                        }
                                    }
                                } else {
                                    warn!("No active client in signer state");
                                    String::new()
                                }
                            };

                            if !user_npub.is_empty() {
                                // First, connect to user's relays from cache to ensure we can fetch their follow list
                                info!("Connecting to user's relays from cache before fetching follow list...");
                                {
                                    let nostr_client = nostr_for_restore.lock().await;
                                    let relay_cache = relay_cache_for_restore.clone();

                                    // Convert npub to hex for cache lookup
                                    let user_pubkey_hex = if let Ok(pubkey) = nostr::PublicKey::parse(&user_npub) {
                                        pubkey.to_hex()
                                    } else {
                                        user_npub.clone() // fallback to npub if conversion fails
                                    };

                                    // Get user's relay list from cache using hex pubkey
                                    if let Some(relay_list) = relay_cache.get_relay_list(&user_pubkey_hex) {
                                        info!("Found cached relay list for user, connecting to {} write relays...", relay_list.write_relays.len());
                                        for relay_url in &relay_list.write_relays {
                                            if let Err(e) = nostr_client.add_relay(relay_url).await {
                                                tracing::debug!("Failed to add user relay {}: {}", relay_url, e);
                                            }
                                        }
                                        // Also connect to read relays
                                        for relay_url in &relay_list.read_relays {
                                            if let Err(e) = nostr_client.add_relay(relay_url).await {
                                                tracing::debug!("Failed to add user inbox relay {}: {}", relay_url, e);
                                            }
                                        }
                                        // Connect all relays
                                        nostr_client.connect().await;
                                    } else {
                                        info!("No cached relay list found for user (tried hex: {}), using default relays", user_pubkey_hex);
                                    }
                                }

                                info!(
                                    "Skipping blocking relay readiness wait before follow list fetch (restore fast path)"
                                );

                                // Fetch follows list
                                let nostr_client = nostr_for_restore.lock().await;
                                info!("Fetching follow list for npub: {}...", user_npub);
                                let followed = match nostr_client.fetch_follow_list(&user_npub).await {
                                    Ok(list) => {
                                        info!("Fetched {} follows for restored session", list.len());
                                        list
                                    }
                                    Err(e) => {
                                        warn!("Failed to fetch follow list after restore: {}", e);
                                        vec![]
                                    }
                                };
                                drop(nostr_client);

                                if !followed.is_empty() {
                                    // Store follows list
                                    {
                                        let mut follows_slot = extended_network_follows_for_restore.write().await;
                                        *follows_slot = followed.clone();
                                    }

                                    // Create a minimal AppState reference for initialize_extended_network
                                    // We need to pass the state, but we only have the cloned fields
                                    // Let's create a wrapper or modify the function signature
                                    // For now, we'll call it directly with the cloned fields
                                    info!("Starting extended network discovery with {} follows", followed.len());

                                    // Initialize social graph database
                                    let config_dir = dirs::data_local_dir()
                                        .unwrap_or_else(|| std::path::PathBuf::from("."))
                                        .join("arcadestr");

                                    let social_graph = match SocialGraphDb::new(config_dir.join("social_graph.db")) {
                                        Ok(db) => Arc::new(db),
                                        Err(e) => {
                                            warn!("Failed to create social graph DB: {}", e);
                                            return;
                                        }
                                    };

                                    // Create extended network repository
                                    let extended_network = Arc::new(Mutex::new(
                                        ExtendedNetworkRepository::new(social_graph)
                                    ));

                                    // Set the user's pubkey
                                    {
                                        let mut repo = extended_network.lock().await;
                                        repo.set_pubkey(user_npub.to_string());
                                        let settings = load_network_discovery_settings();
                                        repo.set_allow_insecure_public_ws(
                                            settings.allow_insecure_public_ws,
                                        );
                                    }

                                    // Store in app state using interior mutability
                                    {
                                        let mut en_slot = extended_network_for_restore.write().await;
                                        *en_slot = Some(extended_network.clone());
                                        info!("ExtendedNetworkRepository stored in AppState");
                                    }

                                    // Spawn discovery task
                                    let en_repo = extended_network.clone();
                                    let app_handle_clone = app_handle.clone();
                                    let nostr = nostr_for_restore.clone();
                                    let relay_cache = relay_cache_for_restore.clone();

                                    tokio::spawn(async move {
                                        let repo = en_repo.lock().await;
                                        let nostr_client = nostr.lock().await;
                                        match repo.discover_network(&*nostr_client, &relay_cache, followed).await {
                                            Ok(stats) => {
                                                info!("Extended network discovery complete: {} qualified, {} relays",
                                                    stats.qualified_count, stats.relays_covered);

                                                // Get discovered relays and connect them
                                                let relay_configs = repo.get_relay_configs();
                                                drop(repo);
                                                drop(nostr_client);

                                                // Connect to extended network relays
                                                let nostr_client = nostr.lock().await;
                                                let mut connected_count = 0;
                                                for relay_url in relay_configs {
                                                    // Skip if already connected (check against default relays)
                                                    if DEFAULT_RELAYS.contains(&relay_url.as_str()) {
                                                        continue;
                                                    }

                                                    match nostr_client.add_relay(&relay_url).await {
                                                        Ok(_) => {
                                                            connected_count += 1;
                                                            tracing::info!("Connected extended network relay: {}", relay_url);
                                                        }
                                                        Err(e) => {
                                                            tracing::debug!("Failed to add extended network relay {}: {}", relay_url, e);
                                                        }
                                                    }
                                                }

                                                if connected_count > 0 {
                                                    nostr_client.connect().await;
                                                    info!("Connected {} extended network relays to gossip", connected_count);
                                                }

                                                // Emit event to notify UI
                                                let _ = app_handle_clone.emit("extended_network_discovered", stats);
                                            }
                                            Err(e) => {
                                                warn!("Extended network discovery failed: {}", e);
                                            }
                                        }
                                    });
                                } else {
                                    info!("No follows found for restored session, skipping extended network discovery");
                                }
                            } else {
                                warn!("Could not determine user npub after restore, skipping extended network discovery");
                            }
                            }
                        } else {
                            info!("Extended network already initialized, skipping discovery after restore");
                        }
                    }
                    SessionRestoreResult::OfflineMode => {
                        info!("Session restored in offline mode (bunker unreachable)");
                        let _ = app_handle.emit("session_offline_mode", ());
                    }
                    SessionRestoreResult::NoSession => {
                        info!("No saved session to restore");
                        let _ = app_handle.emit("show_login", ());
                    }
                    SessionRestoreResult::Failed(e) => {
                        error!("Failed to restore session: {}", e);
                        let _ = app_handle.emit("session_restore_failed", e);
                    }
                }
            });

            // Spawn periodic hint flush task
            let hints = relay_hints.clone();
            tauri::async_runtime::spawn(async move {
                let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(60));
                loop {
                    interval.tick().await;
                    if let Err(e) = hints.flush() {
                        tracing::warn!("Failed to flush relay hints: {}", e);
                    } else {
                        tracing::debug!("Flushed relay hints to database");
                    }
                }
            });

            // Spawn the notification loop for processing relay events
            // Clone the Arc pointers for use in the async task
            let nostr_client_clone = nostr_client.clone();
            let relay_cache_clone = relay_cache.clone();
            let registry_clone = subscription_registry.clone();
            let hints_for_loop = relay_hints.clone();
            let app_handle = app.handle().clone();

            tauri::async_runtime::spawn(async move {
                // Get the client directly from RelayManager
                let client = nostr_client_clone.lock().await;
                let relay_manager_arc = client.relay_manager();
                let manager = relay_manager_arc.lock().await;
                let inner_client = manager.get_client_arc();
                drop(manager);
                drop(client);

                run_notification_loop(
                    inner_client,
                    relay_cache_clone,
                    registry_clone,
                    Some(hints_for_loop),
                    Box::new(move |event| {
                        // Emit event to frontend
                        let _ = app_handle.emit("nostr_event", event);
                    }),
                ).await;
            });

            // Spawn relay event listener for connection status updates
            let nostr_for_events = nostr_client.clone();
            let app_handle_for_events = app.handle().clone();

            tauri::async_runtime::spawn(async move {
                let nostr = nostr_for_events.lock().await;
                let mut rx = nostr.subscribe_relay_events();
                drop(nostr); // Release lock

                while let Ok(event) = rx.recv().await {
                    tracing::info!("Emitting relay event: {:?}", event);

                    let payload = match &event {
                        RelayConnectionEvent::Connected { url } => {
                            serde_json::json!({
                                "type": "connected",
                                "url": url
                            })
                        }
                        RelayConnectionEvent::Disconnected { url, reason } => {
                            serde_json::json!({
                                "type": "disconnected",
                                "url": url,
                                "reason": reason
                            })
                        }
                    };

                    let _ = app_handle_for_events.emit("relay-connection", payload);
                }
            });

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            wait_for_nostrconnect_signer,
            generate_nostrconnect_uri,
            connect_nip46,
            connect_with_key,
            nip49_import,
            nip49_export,
            export_encrypted_key,
            import_encrypted_key,
            verify_nip05,
            verify_nip05_identity,
            reconnect_relays,
            get_public_key,
            is_authenticated,
            disconnect,
            publish_listing,
            fetch_listings,
            fetch_listing_by_id,
            fetch_marketplace,
            fetch_marketplace_stream,
            get_platform_info,
            install_game,
            ingest_receipt,
            fetch_profile,
            get_cached_earned_badges,
            get_cached_profile_badges,
            fetch_earned_badges,
            fetch_profile_badges,
            fetch_profile_with_hints,
            request_invoice,
            // Saved users management
            get_saved_users,
            add_saved_user,
            remove_saved_user,
            get_saved_user,
            rename_saved_user,
            connect_saved_user,
            get_connected_relay_count,
            get_connected_relays,
            get_network_discovery_settings,
            set_allow_insecure_public_ws,
            get_extended_network_stats,
            get_relay_hints_for_pubkey,
            fetch_and_save_user_profile,
            get_cached_profiles,
            get_cached_profile,
            get_version_info,
            // New NIP-46 commands from nip46_commands module
            nip46_commands::connect_bunker,
            nip46_commands::get_connection_status,
            nip46_commands::start_qr_login,
            nip46_commands::check_qr_connection,
            nip46_commands::list_saved_profiles,
            nip46_commands::switch_profile,
            nip46_commands::delete_profile,
            nip46_commands::publish_game_score,
            nip46_commands::ping_bunker,
            nip46_commands::logout_nip46,
            nip46_commands::has_accounts,
            nip46_commands::load_active_account,
            nip46_commands::attempt_reconnect,
            #[cfg(debug_assertions)]
            test_extended_network_discovery,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Arcadestr");
}
