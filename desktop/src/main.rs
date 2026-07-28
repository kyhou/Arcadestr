// Desktop entry point: Tauri v2 application shell with NOSTR auth and listing commands.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tauri::Manager;
use tokio::sync::{Mutex, RwLock};
use tracing::{error, info, warn};

#[allow(unused_imports)]
use arcadestr_core::signers::NostrSigner;

use arcadestr_core::adp_client::{AdpClient, AdpClientError, DownloadAuth};
use arcadestr_core::adp_storage::{
    DownloadTokensRepository, InstalledGamesRepository, LibraryGame as CoreLibraryGame,
    LibraryGamesRepository,
};
use arcadestr_core::auth::{AccountManager, AuthState};
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

mod adp_commands;
mod command_contracts;
mod install;
mod nip46_commands;
mod store_page_commands;

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

impl command_contracts::PurchaseRecordsCommandState for AppState {
    fn purchase_records_command_handles(
        &self,
    ) -> (
        Arc<Mutex<AuthState>>,
        Arc<arcadestr_core::storage::Database>,
    ) {
        (self.auth.clone(), self.database.clone())
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
    let (relays, source, block_discovery) = if !cli.relays.is_empty() {
        (
            cli.relays,
            Some(DebugRelayConfigSource::Cli),
            cli.block_discovery.unwrap_or(true),
        )
    } else if !env.relays.is_empty() {
        (
            env.relays,
            Some(DebugRelayConfigSource::Environment),
            cli.block_discovery.or(env.block_discovery).unwrap_or(true),
        )
    } else if let Some(relays) = settings
        .debug_relays
        .clone()
        .filter(|relays| !relays.is_empty())
    {
        (
            relays,
            Some(DebugRelayConfigSource::Settings),
            cli.block_discovery
                .or(env.block_discovery)
                .or(settings.block_discovery)
                .unwrap_or(true),
        )
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
        block_discovery,
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
    let specs_json = serde_json::to_string(&listing.specs).unwrap_or_else(|_| "[]".to_string());
    let acquisition_json =
        serde_json::to_string(&listing.acquisition).unwrap_or_else(|_| "\"Gated\"".to_string());
    format!(
        "{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}",
        listing.publisher_npub,
        listing.id,
        listing.title,
        listing.description,
        listing.price_sats,
        listing.download_url,
        listing.created_at,
        tags_json,
        platforms_json,
        listing.nip94_event_id.as_deref().unwrap_or_default(),
        specs_json,
        acquisition_json,
        listing.event_id.as_deref().unwrap_or_default()
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
            Arc::clone(&state.database),
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
        price: listing
            .price_amount
            .as_deref()
            .and_then(|amount| amount.parse::<f64>().ok())
            .unwrap_or(listing.price_sats as f64),
        currency: listing.price_currency.unwrap_or_else(|| "SATS".to_string()),
        price_sats: listing.price_sats,
        quantity: None,
        tags: listing.tags,
        specs: listing.specs,
        publisher_npub: listing.publisher_npub,
        stall_id: String::new(),
        stall_name: None,
        lud16: listing.lud16,
        event_id: listing.event_id,
        created_at: listing.created_at,
        platforms: listing.platforms,
        nip94_event_id: listing.nip94_event_id,
        acquisition: match listing.acquisition {
            arcadestr_core::marketplace::AcquisitionPolicy::Gated => {
                arcadestr_app::models::AcquisitionPolicy::Gated
            }
            arcadestr_core::marketplace::AcquisitionPolicy::Public => {
                arcadestr_app::models::AcquisitionPolicy::Public
            }
            arcadestr_core::marketplace::AcquisitionPolicy::TimedAccess { starts_at, ends_at } => {
                arcadestr_app::models::AcquisitionPolicy::TimedAccess { starts_at, ends_at }
            }
        },
        campaigns: listing
            .campaigns
            .into_iter()
            .map(|pointer| arcadestr_app::models::CampaignPointer {
                root_event_id: pointer.root_event_id.to_hex(),
                relay_hint: pointer.relay_hint,
            })
            .collect(),
        is_owned: false,
        #[cfg(debug_assertions)]
        nip99_raw_event_json: listing.nip99_raw_event_json,
    }
}

async fn enrich_listing_ownership(
    mut listing: AppGameListing,
    database: Arc<arcadestr_core::storage::Database>,
    buyer_pubkey_hex: Option<String>,
    log_context: &'static str,
) -> AppGameListing {
    let Some(buyer_pubkey_hex) = buyer_pubkey_hex else {
        return listing;
    };

    match listing_coordinate_from_app_listing(&listing) {
        Ok(coordinate) => {
            let ownership = arcadestr_core::ownership::OwnershipService::new(
                arcadestr_core::purchases::PurchasesRepository::new(database.pool().clone()),
                arcadestr_core::entitlements_repository::EntitlementsRepository::new(
                    database.pool().clone(),
                ),
            );
            match ownership.is_owned(&buyer_pubkey_hex, &coordinate).await {
                Ok(is_owned) => listing.is_owned = is_owned,
                Err(error) => tracing::warn!(
                    "{}: ownership lookup failed for {}: {}",
                    log_context,
                    coordinate,
                    error
                ),
            }
        }
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
async fn get_listing_ownership(
    buyer_npub: String,
    publisher_npub: String,
    listing_id: String,
    state: tauri::State<'_, AppState>,
) -> Result<bool, String> {
    let buyer_pubkey_hex = nostr::PublicKey::from_bech32(&buyer_npub)
        .map_err(|error| format!("invalid buyer npub: {error}"))?
        .to_hex();
    let coordinate = listing_coordinate_from_npub(&publisher_npub, &listing_id)?;
    let ownership = arcadestr_core::ownership::OwnershipService::new(
        arcadestr_core::purchases::PurchasesRepository::new(state.database.pool().clone()),
        arcadestr_core::entitlements_repository::EntitlementsRepository::new(
            state.database.pool().clone(),
        ),
    );
    ownership
        .is_owned(&buyer_pubkey_hex, &coordinate)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn get_platform_info() -> arcadestr_app::models::PlatformInfo {
    arcadestr_app::models::PlatformInfo {
        os: std::env::consts::OS.to_string(),
        arch: std::env::consts::ARCH.to_string(),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct InstalledGame {
    game_coordinate: String,
    file_path: String,
    file_hash: String,
    version: Option<String>,
    server_url: String,
    installed_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct LibraryGame {
    game_coordinate: String,
    added_at: i64,
}

impl From<CoreLibraryGame> for LibraryGame {
    fn from(value: CoreLibraryGame) -> Self {
        Self {
            game_coordinate: value.game_coordinate,
            added_at: value.added_at,
        }
    }
}

fn validate_library_coordinate(coordinate: &str) -> Result<(), String> {
    let mut parts = coordinate.splitn(3, ':');
    if parts.next() != Some("30402") {
        return Err("library coordinate must be a kind:30402 listing".to_string());
    }
    let publisher = parts
        .next()
        .ok_or_else(|| "library coordinate is missing its publisher".to_string())?;
    nostr::PublicKey::from_hex(publisher)
        .map_err(|error| format!("library coordinate has an invalid publisher: {error}"))?;
    if parts
        .next()
        .is_none_or(|listing_id| listing_id.trim().is_empty())
    {
        return Err("library coordinate is missing its listing id".to_string());
    }
    Ok(())
}

async fn active_library_pubkey(state: &AppState) -> Result<String, String> {
    state
        .auth
        .lock()
        .await
        .public_key()
        .map(|pubkey| pubkey.to_hex())
        .ok_or_else(|| "Not authenticated".to_string())
}

#[tauri::command]
async fn add_game_to_library(
    game_coordinate: String,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    validate_library_coordinate(&game_coordinate)?;
    let buyer_pubkey = active_library_pubkey(&state).await?;
    let added_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| format!("system clock is before Unix epoch: {error}"))?
        .as_secs()
        .try_into()
        .map_err(|_| "current timestamp exceeds SQLite range".to_string())?;
    LibraryGamesRepository::new(state.database.pool().clone())
        .add(&CoreLibraryGame {
            buyer_pubkey,
            game_coordinate,
            added_at,
        })
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
async fn is_game_in_library(
    game_coordinate: String,
    state: tauri::State<'_, AppState>,
) -> Result<bool, String> {
    validate_library_coordinate(&game_coordinate)?;
    let buyer_pubkey = active_library_pubkey(&state).await?;
    LibraryGamesRepository::new(state.database.pool().clone())
        .contains(&buyer_pubkey, &game_coordinate)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
async fn get_library_games(state: tauri::State<'_, AppState>) -> Result<Vec<LibraryGame>, String> {
    let buyer_pubkey = active_library_pubkey(&state).await?;
    LibraryGamesRepository::new(state.database.pool().clone())
        .list(&buyer_pubkey)
        .await
        .map(|games| games.into_iter().map(LibraryGame::from).collect())
        .map_err(|error| error.to_string())
}

impl From<arcadestr_core::adp_storage::InstalledGame> for InstalledGame {
    fn from(value: arcadestr_core::adp_storage::InstalledGame) -> Self {
        Self {
            game_coordinate: value.game_coordinate,
            file_path: value.file_path.to_string_lossy().into_owned(),
            file_hash: value.file_hash,
            version: value.version,
            server_url: value.server_url,
            installed_at: value.installed_at,
        }
    }
}

#[tauri::command]
async fn get_installed_games(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<InstalledGame>, String> {
    let repo = InstalledGamesRepository::new(state.database.pool().clone());
    let installed_games = repo.list().await.map_err(|error| error.to_string())?;
    Ok(installed_games
        .into_iter()
        .map(InstalledGame::from)
        .collect())
}

type InstallFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

trait FreshListingFetcher: Send + Sync {
    fn fetch<'a>(&'a self, coordinate: &'a str) -> InstallFuture<'a, Result<nostr::Event, String>>;
}

struct RelayFreshListingFetcher<'a> {
    state: tauri::State<'a, AppState>,
}

impl FreshListingFetcher for RelayFreshListingFetcher<'_> {
    fn fetch<'a>(&'a self, coordinate: &'a str) -> InstallFuture<'a, Result<nostr::Event, String>> {
        Box::pin(async move {
            adp_commands::fetch_listing_event_by_coordinate(&self.state, coordinate).await
        })
    }
}

#[derive(Debug, Clone, Serialize)]
struct DownloadProgressPayload {
    game_coordinate: String,
    bytes: u64,
    total: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
struct DownloadCompletePayload {
    game_coordinate: String,
    listing_id: String,
    file_path: String,
}

#[derive(Debug, Clone)]
struct FreshListingMetadata {
    server_urls: Vec<String>,
    file_hash: String,
    version: Option<String>,
    acquisition: arcadestr_core::marketplace::AcquisitionPolicy,
}

fn extract_fresh_listing_metadata(event: &nostr::Event) -> Result<FreshListingMetadata, String> {
    event
        .verify()
        .map_err(|_| "fresh listing has an invalid signature".to_string())?;
    if event.kind.as_u16() != 30402 {
        return Err("fresh listing is not kind 30402".to_string());
    }
    let server_urls = event
        .tags
        .iter()
        .filter_map(|tag| {
            let values = tag.clone().to_vec();
            (values.first().map(String::as_str) == Some("server"))
                .then(|| values.get(1).cloned())
                .flatten()
        })
        .collect::<Vec<_>>();
    if server_urls.is_empty() {
        return Err("fresh listing is missing server tag".to_string());
    }
    let file_hash = listing_tag_value(event, "file_hash")
        .ok_or_else(|| "fresh listing is missing file_hash tag".to_string())?;
    let version = listing_tag_value(event, "version");
    let acquisition = event
        .tags
        .iter()
        .find_map(|tag| {
            let values = tag.clone().to_vec();
            (values.first().map(String::as_str) == Some("acquisition")).then_some(values)
        })
        .map(|values| match values.as_slice() {
            [_, mode] if mode == "public" => arcadestr_core::marketplace::AcquisitionPolicy::Public,
            [_, mode, starts, ends] if mode == "timed-access" => {
                match (starts.parse::<u64>(), ends.parse::<u64>()) {
                    (Ok(starts_at), Ok(ends_at)) if starts_at < ends_at => {
                        arcadestr_core::marketplace::AcquisitionPolicy::TimedAccess {
                            starts_at,
                            ends_at,
                        }
                    }
                    _ => arcadestr_core::marketplace::AcquisitionPolicy::Gated,
                }
            }
            _ => arcadestr_core::marketplace::AcquisitionPolicy::Gated,
        })
        .unwrap_or_default();

    Ok(FreshListingMetadata {
        server_urls,
        file_hash,
        version,
        acquisition,
    })
}

fn listing_tag_value(event: &nostr::Event, tag_name: &str) -> Option<String> {
    event.tags.iter().find_map(|tag| {
        let values = tag.clone().to_vec();
        match (values.first(), values.get(1)) {
            (Some(name), Some(value)) if name == tag_name => Some(value.clone()),
            _ => None,
        }
    })
}

fn coordinate_artifact_dir_name(coordinate: &str) -> String {
    let digest = Sha256::digest(coordinate.as_bytes());
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn deterministic_artifact_path(app_data_dir: &Path, coordinate: &str) -> PathBuf {
    app_data_dir
        .join("games")
        .join(coordinate_artifact_dir_name(coordinate))
        .join("artifact.bin")
}

fn desktop_download_error(error: AdpClientError) -> String {
    match error {
        AdpClientError::DownloadOwnership(message) => {
            format!(
                "download rejected: you do not own this game or the proof is invalid: {message}"
            )
        }
        AdpClientError::DownloadDistribution(message) => {
            format!("download rejected: this server no longer distributes this listing: {message}")
        }
        AdpClientError::DownloadProtocol(message) => {
            format!(
                "download failed because the ADP server returned an invalid response: {message}"
            )
        }
        AdpClientError::DownloadUnavailable(message) => {
            format!("download authorization could not be verified because relay evidence is incomplete: {message}")
        }
        AdpClientError::Http(message) => format!("download HTTP request failed: {message}"),
        AdpClientError::Auth(message) => format!("download authentication failed: {message}"),
        AdpClientError::Io(message) => format!("download file I/O failed: {message}"),
        other => format!("download failed: {other}"),
    }
}

fn now_unix_i64() -> Result<i64, String> {
    Ok(SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| error.to_string())?
        .as_secs() as i64)
}

async fn install_game_with_fetcher_and_signer<R: tauri::Runtime>(
    listing: AppGameListing,
    state: &AppState,
    active_signer: Option<Arc<dyn NostrSigner>>,
    app: Option<&tauri::AppHandle<R>>,
    app_data_dir: PathBuf,
    fresh_listing_fetcher: &dyn FreshListingFetcher,
) -> Result<(), String> {
    let signer = if let Some(signer) = active_signer {
        signer
    } else {
        let auth = state.auth.lock().await;
        auth.signer()
            .cloned()
            .map(|signer| Arc::new(signer) as Arc<dyn NostrSigner>)
            .ok_or_else(|| "not authenticated".to_string())?
    };
    let buyer_pubkey_hex = signer
        .get_public_key()
        .await
        .map_err(|error| format!("failed to get active signer pubkey: {error}"))?
        .to_hex();

    let listing_id = listing.id.clone();
    let coordinate = listing_coordinate_from_app_listing(&listing)?;
    let fresh_listing = fresh_listing_fetcher.fetch(&coordinate).await?;
    let expected_coordinate = listing_coordinate_from_npub(
        &fresh_listing
            .pubkey
            .to_bech32()
            .map_err(|error| error.to_string())?,
        &listing_tag_value(&fresh_listing, "d")
            .ok_or_else(|| "fresh listing is missing d tag".to_string())?,
    )?;
    if expected_coordinate != coordinate {
        return Err("fresh listing coordinate mismatch".into());
    }
    let metadata = extract_fresh_listing_metadata(&fresh_listing)?;
    let tokens = DownloadTokensRepository::new(state.database.pool().clone());
    let mut cached_token = None;
    for server_url in &metadata.server_urls {
        if let Some(token) = tokens
            .valid_token(&buyer_pubkey_hex, &coordinate, server_url, now_unix_i64()?)
            .await
            .map_err(|error| error.to_string())?
        {
            cached_token = Some((server_url.clone(), token.token));
            break;
        }
    }

    let has_durable_ownership = if cached_token.is_some() {
        true
    } else {
        let ownership = arcadestr_core::ownership::OwnershipService::new(
            arcadestr_core::purchases::PurchasesRepository::new(state.database.pool().clone()),
            arcadestr_core::entitlements_repository::EntitlementsRepository::new(
                state.database.pool().clone(),
            ),
        );
        ownership
            .is_owned(&buyer_pubkey_hex, &coordinate)
            .await
            .map_err(|error| error.to_string())?
    };
    if !has_durable_ownership
        && !metadata
            .acquisition
            .allows_access_at(now_unix_i64()? as u64)
    {
        return Err("ownership or explicit current access not found".into());
    }

    let (server_url, auth) = match cached_token {
        Some((server_url, token)) => (server_url, DownloadAuth::Token(token)),
        None => (
            metadata
                .server_urls
                .first()
                .cloned()
                .ok_or_else(|| "fresh listing has no authorized server".to_string())?,
            DownloadAuth::Nip98 {
                signer: signer.as_ref(),
            },
        ),
    };
    let dest_path = deterministic_artifact_path(&app_data_dir, &coordinate);
    if let Some(parent) = dest_path.parent() {
        tokio::fs::create_dir_all(parent).await.map_err(|error| {
            format!(
                "failed to create install directory {}: {error}",
                parent.display()
            )
        })?;
    }

    let adp_client = AdpClient::new(server_url.clone(), Arc::clone(&state.http_client));
    adp_client
        .download(&coordinate, auth, &dest_path, |bytes, total| {
            if let Some(app) = app {
                let _ = app.emit(
                    "download-progress",
                    DownloadProgressPayload {
                        game_coordinate: coordinate.clone(),
                        bytes,
                        total,
                    },
                );
            }
        })
        .await
        .map_err(desktop_download_error)?;

    let installed_games = InstalledGamesRepository::new(state.database.pool().clone());
    install::verify_and_record_downloaded_game(
        &installed_games,
        &coordinate,
        &dest_path,
        &metadata.file_hash,
        metadata.version,
        &server_url,
    )
    .await?;

    if let Some(app) = app {
        app.emit(
            "download-complete",
            DownloadCompletePayload {
                game_coordinate: coordinate,
                listing_id,
                file_path: dest_path.to_string_lossy().to_string(),
            },
        )
        .map_err(|error| error.to_string())?;
    }

    Ok(())
}

async fn install_game_with_fetcher<R: tauri::Runtime>(
    listing: AppGameListing,
    state: &AppState,
    app: Option<&tauri::AppHandle<R>>,
    app_data_dir: PathBuf,
    fresh_listing_fetcher: &dyn FreshListingFetcher,
) -> Result<(), String> {
    install_game_with_fetcher_and_signer(
        listing,
        state,
        None,
        app,
        app_data_dir,
        fresh_listing_fetcher,
    )
    .await
}

#[tauri::command]
async fn install_game(
    listing: AppGameListing,
    app_handle: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    signer_state: tauri::State<'_, Arc<Mutex<AppSignerState>>>,
) -> Result<(), String> {
    tracing::info!(
        "install_game called for listing '{}' ({})",
        listing.title,
        listing.id
    );

    let app_data_dir = app_handle
        .path()
        .app_data_dir()
        .map_err(|error| format!("failed to resolve app data directory: {error}"))?;
    let fetcher = RelayFreshListingFetcher {
        state: state.clone(),
    };
    let auth_snapshot = { state.auth.lock().await.clone() };
    let signer = adp_commands::resolve_active_signer(signer_state.inner(), &auth_snapshot).await?;
    install_game_with_fetcher_and_signer(
        listing,
        &state,
        Some(signer),
        Some(&app_handle),
        app_data_dir,
        &fetcher,
    )
    .await
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

#[tauri::command]
async fn get_purchase_records(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<arcadestr_core::ownership::DurableAcquisitionRecord>, String> {
    command_contracts::get_purchase_records(state.inner())
        .await
        .map_err(|error| error.to_string())
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
    request_id: String,
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
    let product_event = format!("marketplace-product-{request_id}");
    let complete_event = format!("marketplace-complete-{request_id}");

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
                    Arc::clone(&state.database),
                    buyer_pubkey_hex.clone(),
                    "fetch_marketplace_stream cache",
                )
                .await;

                if window.emit(&product_event, &listing).is_ok() {
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
    let database_for_closure = Arc::clone(&state.database);
    let buyer_pubkey_hex_for_closure = buyer_pubkey_hex.clone();
    let product_event_for_closure = product_event.clone();

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
            let database = Arc::clone(&database_for_closure);
            let buyer_pubkey_hex = buyer_pubkey_hex_for_closure.clone();
            let product_event = product_event_for_closure.clone();
            let window = window_for_closure.clone();
            let task = tauri::async_runtime::spawn(async move {
                let listing = enrich_listing_ownership(
                    listing,
                    database,
                    buyer_pubkey_hex,
                    "fetch_marketplace_stream",
                )
                .await;

                if let Err(e) = window.emit(&product_event, &listing) {
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
        match state
            .marketplace_cache
            .upsert_listing(&listing, listing.event_id.as_deref())
            .await
        {
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
    let _ = window.emit(&complete_event, ());

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
mod install_game_tests {
    use super::*;

    use std::collections::{HashMap, VecDeque};
    use std::path::Path;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Mutex;

    use arcadestr_core::adp_storage::{
        DownloadToken, DownloadTokensRepository, InstalledGame as CoreInstalledGame,
        InstalledGamesRepository as CoreInstalledGamesRepository,
    };
    use arcadestr_core::file_hash::sha256_file;
    use arcadestr_core::http_client::{HttpClientError, HttpDownloadOutcome};
    use arcadestr_core::purchases::StoredReceipt;
    use arcadestr_core::storage::Database;
    use nostr::nips::nip19::ToBech32;
    use nostr::{Event, EventBuilder, Keys, Kind, Tag, TagKind, Timestamp};
    use tokio::sync::{Mutex as AsyncMutex, RwLock};

    #[derive(Clone, Default)]
    struct MockHttpClient {
        state: Arc<Mutex<MockHttpState>>,
    }

    #[derive(Default)]
    struct MockHttpState {
        download_responses: HashMap<String, VecDeque<Vec<u8>>>,
        download_headers: HashMap<String, Vec<(String, String)>>,
        requested_urls: Vec<String>,
        call_counts: HashMap<String, usize>,
    }

    impl MockHttpClient {
        fn new() -> Self {
            Self::default()
        }

        fn with_download_response(self, url: &str, body: impl Into<Vec<u8>>) -> Self {
            self.state
                .lock()
                .expect("mock http state mutex poisoned")
                .download_responses
                .entry(url.to_string())
                .or_default()
                .push_back(body.into());
            self
        }

        fn call_count(&self, url: &str) -> usize {
            self.state
                .lock()
                .expect("mock http state mutex poisoned")
                .call_counts
                .get(url)
                .copied()
                .unwrap_or(0)
        }

        fn last_download_headers(&self, url: &str) -> Option<Vec<(String, String)>> {
            self.state
                .lock()
                .expect("mock http state mutex poisoned")
                .download_headers
                .get(url)
                .cloned()
        }

        fn last_requested_url(&self) -> Option<String> {
            self.state
                .lock()
                .expect("mock http state mutex poisoned")
                .requested_urls
                .last()
                .cloned()
        }
    }

    #[async_trait::async_trait]
    impl HttpClient for MockHttpClient {
        async fn get_json(&self, _url: &str) -> Result<serde_json::Value, HttpClientError> {
            Err(HttpClientError::Request(
                "get_json path should not be used in install_game tests".to_string(),
            ))
        }

        async fn get_json_no_redirects(
            &self,
            _url: &str,
        ) -> Result<serde_json::Value, HttpClientError> {
            Err(HttpClientError::Request(
                "get_json_no_redirects path should not be used in install_game tests".to_string(),
            ))
        }

        async fn post_json(
            &self,
            _url: &str,
            _body: serde_json::Value,
            _headers: Vec<(String, String)>,
        ) -> Result<serde_json::Value, HttpClientError> {
            Err(HttpClientError::Request(
                "post_json path should not be used in install_game tests".to_string(),
            ))
        }

        async fn download_to_path(
            &self,
            url: &str,
            headers: Vec<(String, String)>,
            dest: &Path,
            on_progress: &mut (dyn FnMut(u64, Option<u64>) + Send),
        ) -> Result<HttpDownloadOutcome, HttpClientError> {
            let bytes = {
                let mut state = self.state.lock().expect("mock http state mutex poisoned");
                state.requested_urls.push(url.to_string());
                *state.call_counts.entry(url.to_string()).or_insert(0) += 1;
                state.download_headers.insert(url.to_string(), headers);
                state
                    .download_responses
                    .get_mut(url)
                    .and_then(VecDeque::pop_front)
                    .ok_or_else(|| {
                        HttpClientError::Request(format!(
                            "No mock download response configured for URL: {url}"
                        ))
                    })?
            };

            std::fs::write(dest, &bytes)
                .map_err(|error| HttpClientError::Request(error.to_string()))?;
            let bytes_written = bytes.len() as u64;
            on_progress(bytes_written, Some(bytes_written));
            Ok(HttpDownloadOutcome { bytes_written })
        }
    }

    struct StaticFreshListingFetcher {
        event: Event,
        calls: Arc<AtomicUsize>,
    }

    impl FreshListingFetcher for StaticFreshListingFetcher {
        fn fetch<'a>(&'a self, _coordinate: &'a str) -> InstallFuture<'a, Result<Event, String>> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Box::pin(async move { Ok(self.event.clone()) })
        }
    }

    async fn test_db(prefix: &str) -> Database {
        let path = unique_test_path(prefix, "db");
        Database::new(&path)
            .await
            .expect("test database should open")
    }

    fn unique_test_path(prefix: &str, extension: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "{prefix}-{}.{extension}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system clock should be after epoch")
                .as_nanos()
        ))
    }

    async fn app_state_with_http(
        buyer: &Keys,
        database: Database,
        http_client: Arc<dyn HttpClient>,
    ) -> AppState {
        let user_cache = Arc::new(UserCache::new(database.pool().clone()));
        let nostr = NostrClient::new_with_cache(
            "install-game-test".to_string(),
            vec![],
            user_cache.clone(),
            None,
        )
        .await
        .expect("test nostr client should initialize");
        let validator_client = NostrClient::new_with_cache(
            "install-game-validator".to_string(),
            vec![],
            user_cache.clone(),
            None,
        )
        .await
        .expect("test validator client should initialize");
        let nip05_validator = Arc::new(std::sync::Mutex::new(Nip05Validator::spawn(
            Arc::new(validator_client),
            user_cache.clone(),
        )));
        let mut auth = AuthState::new();
        auth.connect_with_key(&buyer.secret_key().to_secret_hex())
            .expect("test buyer should authenticate");
        let relay_cache = Arc::new(
            RelayCache::new(unique_test_path("install-game-relay-cache", "db"))
                .expect("relay cache should open"),
        );
        let relay_hints = Arc::new(
            RelayHints::new(unique_test_path("install-game-relay-hints", "db"))
                .expect("relay hints should open"),
        );
        let marketplace_cache = Arc::new(MarketplaceCache::new(database.pool().clone()));
        let purchases = Arc::new(arcadestr_core::purchases::PurchasesRepository::new(
            database.pool().clone(),
        ));
        let profile_fetcher = Arc::new({
            let mut fetcher = ProfileFetcher::with_persistent_cache(user_cache.clone());
            fetcher.with_nip05_validator(nip05_validator.clone());
            fetcher
        });

        AppState {
            auth: Arc::new(AsyncMutex::new(auth)),
            nostr: Arc::new(AsyncMutex::new(nostr)),
            database: Arc::new(database),
            relay_cache,
            deduplicator: Arc::new(AsyncMutex::new(EventDeduplicator::new(10_000))),
            subscription_registry: Arc::new(SubscriptionRegistry::new()),
            profile_fetcher,
            user_cache,
            marketplace_cache,
            purchases,
            extended_network: Arc::new(RwLock::new(None)),
            extended_network_follows: Arc::new(RwLock::new(Vec::new())),
            relay_hints: Some(relay_hints),
            nip05_validator,
            http_client,
        }
    }

    #[tokio::test]
    async fn get_installed_games_returns_recorded_installs() {
        let database = test_db("get-installed-games").await;
        let repo = CoreInstalledGamesRepository::new(database.pool().clone());
        let older = CoreInstalledGame {
            game_coordinate: "30402:developer:older-game".to_string(),
            file_path: unique_test_path("older-game", "zip"),
            file_hash: "older-hash".to_string(),
            version: Some("1.0.0".to_string()),
            server_url: "https://dist.example.com".to_string(),
            installed_at: 100,
        };
        let newer = CoreInstalledGame {
            game_coordinate: "30402:developer:newer-game".to_string(),
            file_path: unique_test_path("newer-game", "zip"),
            file_hash: "newer-hash".to_string(),
            version: Some("2.0.0".to_string()),
            server_url: "https://dist.example.com".to_string(),
            installed_at: 200,
        };
        repo.record(&older).await.expect("older game should record");
        repo.record(&newer).await.expect("newer game should record");

        let buyer = Keys::generate();
        let app_state =
            app_state_with_http(&buyer, database, Arc::new(MockHttpClient::new())).await;
        let app = tauri::test::mock_builder()
            .manage(app_state)
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .expect("mock Tauri app should build");

        let installed_games = get_installed_games(app.state::<AppState>())
            .await
            .expect("installed games should list");

        assert_eq!(installed_games.len(), 2);
        assert_eq!(installed_games[0].game_coordinate, newer.game_coordinate);
        assert_eq!(installed_games[0].installed_at, newer.installed_at);
        assert_eq!(installed_games[1].game_coordinate, older.game_coordinate);
        assert_eq!(installed_games[1].installed_at, older.installed_at);
    }

    fn app_listing(merchant: &Keys, listing_id: &str) -> AppGameListing {
        AppGameListing {
            id: listing_id.to_string(),
            source: arcadestr_app::models::ListingSource::Nip99Listing,
            title: "Fresh Install Test".to_string(),
            description: "listing".to_string(),
            images: Vec::new(),
            download_url: String::new(),
            price: 1.0,
            currency: "SATS".to_string(),
            price_sats: 1,
            quantity: None,
            tags: Vec::new(),
            specs: Vec::new(),
            publisher_npub: merchant
                .public_key()
                .to_bech32()
                .expect("merchant npub should encode"),
            stall_id: String::new(),
            stall_name: None,
            lud16: String::new(),
            event_id: None,
            created_at: 1_700_000_000,
            platforms: Vec::new(),
            nip94_event_id: None,
            acquisition: arcadestr_app::models::AcquisitionPolicy::Gated,
            campaigns: Vec::new(),
            is_owned: false,
            #[cfg(debug_assertions)]
            nip99_raw_event_json: None,
        }
    }

    fn coordinate(merchant: &Keys, listing_id: &str) -> String {
        format!("30402:{}:{listing_id}", merchant.public_key().to_hex())
    }

    fn listing_event(
        merchant: &Keys,
        listing_id: &str,
        server_url: &str,
        file_hash: &str,
        version: &str,
    ) -> Event {
        EventBuilder::new(Kind::Custom(30402), "")
            .tags([
                Tag::custom(TagKind::d(), [listing_id]),
                Tag::custom(TagKind::custom("server"), [server_url]),
                Tag::custom(TagKind::custom("file_hash"), [file_hash]),
                Tag::custom(TagKind::custom("version"), [version]),
            ])
            .custom_created_at(Timestamp::from(1_700_000_100))
            .sign_with_keys(merchant)
            .expect("listing event should sign")
    }

    fn listing_event_with_acquisition(
        merchant: &Keys,
        listing_id: &str,
        server_url: &str,
        file_hash: &str,
        acquisition: &[&str],
    ) -> Event {
        let mut tags = vec![
            Tag::custom(TagKind::d(), [listing_id]),
            Tag::custom(TagKind::custom("server"), [server_url]),
            Tag::custom(TagKind::custom("file_hash"), [file_hash]),
            Tag::custom(TagKind::custom("version"), ["1.0.0"]),
        ];
        tags.push(Tag::custom(
            TagKind::custom("acquisition"),
            acquisition.iter().copied(),
        ));
        EventBuilder::new(Kind::Custom(30402), "")
            .tags(tags)
            .custom_created_at(Timestamp::from(1_700_000_100))
            .sign_with_keys(merchant)
            .expect("listing event should sign")
    }

    async fn grant_ownership(state: &AppState, buyer: &Keys, merchant: &Keys, listing_id: &str) {
        let event = EventBuilder::new(Kind::Custom(1020), "encrypted")
            .tags([
                Tag::custom(TagKind::custom("order"), [format!("order-{listing_id}")]),
                Tag::custom(TagKind::p(), [buyer.public_key().to_hex()]),
                Tag::custom(TagKind::a(), [coordinate(merchant, listing_id)]),
                Tag::custom(TagKind::custom("payment_hash"), ["11".repeat(32)]),
                Tag::custom(TagKind::custom("amount_msat"), ["1000"]),
                Tag::custom(TagKind::custom("settled_at"), ["1700000200"]),
                Tag::custom(TagKind::custom("proof"), ["bolt11-preimage"]),
                Tag::custom(TagKind::custom("status"), ["paid"]),
            ])
            .custom_created_at(Timestamp::from(1_700_000_200))
            .sign_with_keys(merchant)
            .expect("receipt signs");
        let receipt = arcadestr_core::purchases::parse_and_validate_receipt(
            &event,
            &buyer.public_key().to_hex(),
        )
        .expect("receipt validates");
        state
            .purchases
            .upsert_receipt(&receipt)
            .await
            .expect("test receipt should persist");
    }

    async fn grant_entitlement_ownership(
        state: &AppState,
        buyer: &Keys,
        merchant: &Keys,
        listing_id: &str,
    ) {
        let coordinate = coordinate(merchant, listing_id);
        let campaign_event = EventBuilder::new(
            Kind::Custom(arcadestr_core::adp_protocol::ADP_CAMPAIGN_KIND),
            "",
        )
        .tags([
            Tag::custom(TagKind::d(), ["campaign"]),
            Tag::custom(TagKind::custom("a"), [&coordinate]),
            Tag::custom(TagKind::custom("mode"), ["claim"]),
            Tag::custom(TagKind::custom("starts"), ["120"]),
            Tag::custom(TagKind::custom("ends"), ["300"]),
            Tag::custom(TagKind::custom("status"), ["active"]),
        ])
        .custom_created_at(Timestamp::from(100))
        .sign_with_keys(merchant)
        .expect("campaign signs");
        let campaign = arcadestr_core::campaign::resolve_campaign(
            &[
                arcadestr_core::campaign::parse_campaign_event(&campaign_event)
                    .expect("campaign parses"),
            ],
            merchant.public_key(),
            &coordinate,
        )
        .expect("campaign resolves");
        let grant = EventBuilder::new(
            Kind::Custom(arcadestr_core::adp_protocol::ENTITLEMENT_GRANT_KIND),
            "",
        )
        .tags([
            Tag::custom(TagKind::d(), [format!("grant-{listing_id}")]),
            Tag::custom(TagKind::p(), [buyer.public_key().to_hex()]),
            Tag::custom(TagKind::custom("a"), [&coordinate]),
            Tag::custom(
                TagKind::custom("source_event"),
                [campaign.root_event_id.to_hex()],
            ),
            Tag::custom(TagKind::custom("status"), ["granted"]),
        ])
        .custom_created_at(Timestamp::from(150))
        .sign_with_keys(merchant)
        .expect("grant signs");
        arcadestr_core::entitlements_repository::EntitlementsRepository::new(
            state.database.pool().clone(),
        )
        .ingest_event(&grant, &campaign, None)
        .await
        .expect("entitlement persists");
    }

    async fn sha256_hex(bytes: &[u8]) -> String {
        let path = unique_test_path("install-game-hash", "bin");
        tokio::fs::write(&path, bytes)
            .await
            .expect("hash fixture should write");
        sha256_file(&path).await.expect("hash fixture should hash")
    }

    #[tokio::test]
    async fn install_game_unpurchased_listing_fails_before_download_request() {
        let buyer = Keys::generate();
        let merchant = Keys::generate();
        let listing_id = "not-owned";
        let artifact_bytes = b"owned artifact";
        let file_hash = sha256_hex(artifact_bytes).await;
        let server_url = "https://dist.example.com";
        let coordinate = coordinate(&merchant, listing_id);
        let encoded_coordinate = urlencoding::encode(&coordinate);
        let download_url = format!("{server_url}/game/{encoded_coordinate}");
        let http = MockHttpClient::new().with_download_response(&download_url, artifact_bytes);
        let state = app_state_with_http(
            &buyer,
            test_db("install-game-unpurchased").await,
            Arc::new(http.clone()),
        )
        .await;
        let fetch_calls = Arc::new(AtomicUsize::new(0));
        let fetcher = StaticFreshListingFetcher {
            event: listing_event(&merchant, listing_id, server_url, &file_hash, "1.0.0"),
            calls: fetch_calls,
        };

        let err = install_game_with_fetcher(
            app_listing(&merchant, listing_id),
            &state,
            None::<&tauri::AppHandle<tauri::test::MockRuntime>>,
            unique_test_path("install-game-unpurchased-data", "dir"),
            &fetcher,
        )
        .await
        .expect_err("unpurchased listing should fail locally");

        assert!(err.contains("purchase") || err.contains("own"));
        assert_eq!(http.call_count(&download_url), 0);
    }

    #[tokio::test]
    async fn fake_submitted_zero_price_cannot_bypass_ownership() {
        let buyer = Keys::generate();
        let merchant = Keys::generate();
        let listing_id = "zero-price-gated";
        let artifact_bytes = b"gated artifact";
        let file_hash = sha256_hex(artifact_bytes).await;
        let server_url = "https://dist.example.com";
        let coordinate = coordinate(&merchant, listing_id);
        let encoded_coordinate = urlencoding::encode(&coordinate);
        let download_url = format!("{server_url}/game/{encoded_coordinate}");
        let http = MockHttpClient::new().with_download_response(&download_url, artifact_bytes);
        let state = app_state_with_http(
            &buyer,
            test_db("install-game-zero-gated").await,
            Arc::new(http.clone()),
        )
        .await;
        let fetcher = StaticFreshListingFetcher {
            event: listing_event(&merchant, listing_id, server_url, &file_hash, "1.0.0"),
            calls: Arc::new(AtomicUsize::new(0)),
        };
        let mut listing = app_listing(&merchant, listing_id);
        listing.price = 0.0;
        listing.price_sats = 0;

        let error = install_game_with_fetcher(
            listing,
            &state,
            None::<&tauri::AppHandle<tauri::test::MockRuntime>>,
            unique_test_path("install-game-zero-gated-data", "dir"),
            &fetcher,
        )
        .await
        .expect_err("caller zero price must not bypass ownership");

        assert!(error.contains("own") || error.contains("access"));
        assert_eq!(http.call_count(&download_url), 0);
    }

    #[tokio::test]
    async fn public_listing_uses_active_signer_when_legacy_auth_is_empty() {
        let buyer = Keys::generate();
        let merchant = Keys::generate();
        let listing_id = "public-game";
        let artifact_bytes = b"public artifact";
        let file_hash = sha256_hex(artifact_bytes).await;
        let server_url = "https://dist.example.com";
        let coordinate = coordinate(&merchant, listing_id);
        let download_url = format!("{server_url}/game/{}", urlencoding::encode(&coordinate));
        let http = MockHttpClient::new().with_download_response(&download_url, artifact_bytes);
        let state = app_state_with_http(
            &buyer,
            test_db("install-game-public").await,
            Arc::new(http.clone()),
        )
        .await;
        let active_signer = {
            let auth = state.auth.lock().await;
            Arc::new(
                auth.signer()
                    .cloned()
                    .expect("test buyer should have an active signer"),
            ) as Arc<dyn NostrSigner>
        };
        state.auth.lock().await.disconnect();
        let fetcher = StaticFreshListingFetcher {
            event: listing_event_with_acquisition(
                &merchant,
                listing_id,
                server_url,
                &file_hash,
                &["public"],
            ),
            calls: Arc::new(AtomicUsize::new(0)),
        };

        install_game_with_fetcher_and_signer(
            app_listing(&merchant, listing_id),
            &state,
            Some(active_signer),
            None::<&tauri::AppHandle<tauri::test::MockRuntime>>,
            unique_test_path("install-game-public-data", "dir"),
            &fetcher,
        )
        .await
        .expect("signed public policy allows install without ownership");

        assert_eq!(http.call_count(&download_url), 1);
        assert!(http
            .last_download_headers(&download_url)
            .expect("download headers should be captured")
            .iter()
            .any(|(name, value)| name == "Authorization" && value.starts_with("Nostr ")));
    }

    #[tokio::test]
    async fn timed_access_allows_only_inside_signed_window() {
        let buyer = Keys::generate();
        let merchant = Keys::generate();
        let server_url = "https://dist.example.com";
        let now = now_unix_i64().expect("clock") as u64;

        for (listing_id, starts, ends, allowed) in [
            ("timed-active", now - 10, now + 1000, true),
            ("timed-expired", now - 1000, now, false),
        ] {
            let artifact_bytes = listing_id.as_bytes();
            let file_hash = sha256_hex(artifact_bytes).await;
            let coordinate = coordinate(&merchant, listing_id);
            let download_url = format!("{server_url}/game/{}", urlencoding::encode(&coordinate));
            let http = MockHttpClient::new().with_download_response(&download_url, artifact_bytes);
            let state =
                app_state_with_http(&buyer, test_db(listing_id).await, Arc::new(http.clone()))
                    .await;
            let starts = starts.to_string();
            let ends = ends.to_string();
            let fetcher = StaticFreshListingFetcher {
                event: listing_event_with_acquisition(
                    &merchant,
                    listing_id,
                    server_url,
                    &file_hash,
                    &["timed-access", &starts, &ends],
                ),
                calls: Arc::new(AtomicUsize::new(0)),
            };
            let result = install_game_with_fetcher(
                app_listing(&merchant, listing_id),
                &state,
                None::<&tauri::AppHandle<tauri::test::MockRuntime>>,
                unique_test_path(&format!("{listing_id}-data"), "dir"),
                &fetcher,
            )
            .await;

            assert_eq!(result.is_ok(), allowed, "{listing_id}");
            assert_eq!(http.call_count(&download_url), usize::from(allowed));
        }
    }

    #[tokio::test]
    async fn durable_grant_allows_current_gated_build_install() {
        let buyer = Keys::generate();
        let merchant = Keys::generate();
        let listing_id = "grant-owned";
        let artifact_bytes = b"grant owned artifact";
        let file_hash = sha256_hex(artifact_bytes).await;
        let server_url = "https://dist.example.com";
        let coordinate = coordinate(&merchant, listing_id);
        let download_url = format!("{server_url}/game/{}", urlencoding::encode(&coordinate));
        let http = MockHttpClient::new().with_download_response(&download_url, artifact_bytes);
        let state = app_state_with_http(
            &buyer,
            test_db("install-game-grant-owned").await,
            Arc::new(http.clone()),
        )
        .await;
        grant_entitlement_ownership(&state, &buyer, &merchant, listing_id).await;
        let fetcher = StaticFreshListingFetcher {
            event: listing_event(&merchant, listing_id, server_url, &file_hash, "2.0.0"),
            calls: Arc::new(AtomicUsize::new(0)),
        };

        install_game_with_fetcher(
            app_listing(&merchant, listing_id),
            &state,
            None::<&tauri::AppHandle<tauri::test::MockRuntime>>,
            unique_test_path("install-game-grant-owned-data", "dir"),
            &fetcher,
        )
        .await
        .expect("durable grant permits current gated build");

        assert_eq!(http.call_count(&download_url), 1);
    }

    #[tokio::test]
    async fn install_game_uses_cached_token_path_a() {
        let buyer = Keys::generate();
        let merchant = Keys::generate();
        let listing_id = "token-path-a";
        let artifact_bytes = b"token artifact";
        let file_hash = sha256_hex(artifact_bytes).await;
        let server_url = "https://dist.example.com";
        let coordinate = coordinate(&merchant, listing_id);
        let encoded_coordinate = urlencoding::encode(&coordinate);
        let download_url = format!("{server_url}/game/{encoded_coordinate}?token=token-path-a");
        let http = MockHttpClient::new().with_download_response(&download_url, artifact_bytes);
        let state = app_state_with_http(
            &buyer,
            test_db("install-game-token").await,
            Arc::new(http.clone()),
        )
        .await;
        grant_ownership(&state, &buyer, &merchant, listing_id).await;
        DownloadTokensRepository::new(state.database.pool().clone())
            .upsert(&DownloadToken {
                buyer_pubkey: buyer.public_key().to_hex(),
                game_coordinate: coordinate.clone(),
                server_url: server_url.to_string(),
                token: "token-path-a".to_string(),
                expires_at: 4_000_000_000,
            })
            .await
            .expect("token should persist");
        let fetcher = StaticFreshListingFetcher {
            event: listing_event(&merchant, listing_id, server_url, &file_hash, "1.0.0"),
            calls: Arc::new(AtomicUsize::new(0)),
        };

        install_game_with_fetcher(
            app_listing(&merchant, listing_id),
            &state,
            None::<&tauri::AppHandle<tauri::test::MockRuntime>>,
            unique_test_path("install-game-token-data", "dir"),
            &fetcher,
        )
        .await
        .expect("token path install should succeed");

        assert_eq!(http.call_count(&download_url), 1);
    }

    #[tokio::test]
    async fn install_game_rejects_another_accounts_cached_token() {
        let buyer = Keys::generate();
        let other_buyer = Keys::generate();
        let merchant = Keys::generate();
        let listing_id = "other-account-token";
        let server_url = "https://dist.example.com";
        let coordinate = coordinate(&merchant, listing_id);
        let file_hash = sha256_hex(b"artifact").await;
        let http = MockHttpClient::new();
        let state = app_state_with_http(
            &buyer,
            test_db("install-game-other-account-token").await,
            Arc::new(http),
        )
        .await;
        DownloadTokensRepository::new(state.database.pool().clone())
            .upsert(&DownloadToken {
                buyer_pubkey: other_buyer.public_key().to_hex(),
                game_coordinate: coordinate,
                server_url: server_url.to_string(),
                token: "other-account-token".to_string(),
                expires_at: 4_000_000_000,
            })
            .await
            .expect("other account token should persist");
        let fetcher = StaticFreshListingFetcher {
            event: listing_event(&merchant, listing_id, server_url, &file_hash, "1.0.0"),
            calls: Arc::new(AtomicUsize::new(0)),
        };

        let error = install_game_with_fetcher(
            app_listing(&merchant, listing_id),
            &state,
            None::<&tauri::AppHandle<tauri::test::MockRuntime>>,
            unique_test_path("install-game-other-account-token-data", "dir"),
            &fetcher,
        )
        .await
        .expect_err("another account's token must not authorize installation");

        assert_eq!(error, "ownership or explicit current access not found");
    }

    #[tokio::test]
    async fn install_game_without_token_uses_nip98_path_b() {
        let buyer = Keys::generate();
        let merchant = Keys::generate();
        let listing_id = "nip98-path-b";
        let artifact_bytes = b"nip98 artifact";
        let file_hash = sha256_hex(artifact_bytes).await;
        let server_url = "https://dist.example.com";
        let coordinate = coordinate(&merchant, listing_id);
        let encoded_coordinate = urlencoding::encode(&coordinate);
        let download_url = format!("{server_url}/game/{encoded_coordinate}");
        let http = MockHttpClient::new().with_download_response(&download_url, artifact_bytes);
        let state = app_state_with_http(
            &buyer,
            test_db("install-game-nip98").await,
            Arc::new(http.clone()),
        )
        .await;
        grant_ownership(&state, &buyer, &merchant, listing_id).await;
        let fetcher = StaticFreshListingFetcher {
            event: listing_event(&merchant, listing_id, server_url, &file_hash, "1.0.0"),
            calls: Arc::new(AtomicUsize::new(0)),
        };

        install_game_with_fetcher(
            app_listing(&merchant, listing_id),
            &state,
            None::<&tauri::AppHandle<tauri::test::MockRuntime>>,
            unique_test_path("install-game-nip98-data", "dir"),
            &fetcher,
        )
        .await
        .expect("nip98 path install should succeed");

        let headers = http
            .last_download_headers(&download_url)
            .expect("download headers should be recorded");
        assert!(headers
            .iter()
            .any(|(name, value)| name == "Authorization" && value.starts_with("Nostr ")));
        assert!(!http
            .last_requested_url()
            .expect("download URL should be recorded")
            .contains("token="));
    }

    #[tokio::test]
    async fn install_game_refetches_listing_event_before_download() {
        let buyer = Keys::generate();
        let merchant = Keys::generate();
        let listing_id = "fresh-listing";
        let fresh_bytes = b"fresh artifact bytes";
        let fresh_hash = sha256_hex(fresh_bytes).await;
        let stale_server_url = "https://stale.example.com";
        let fresh_server_url = "https://fresh.example.com";
        let coordinate = coordinate(&merchant, listing_id);
        let encoded_coordinate = urlencoding::encode(&coordinate);
        let stale_download_url = format!("{stale_server_url}/game/{encoded_coordinate}");
        let fresh_download_url = format!("{fresh_server_url}/game/{encoded_coordinate}");
        let http = MockHttpClient::new().with_download_response(&fresh_download_url, fresh_bytes);
        let state = app_state_with_http(
            &buyer,
            test_db("install-game-fresh").await,
            Arc::new(http.clone()),
        )
        .await;
        grant_ownership(&state, &buyer, &merchant, listing_id).await;
        let fetch_calls = Arc::new(AtomicUsize::new(0));
        let fetcher = StaticFreshListingFetcher {
            event: listing_event(
                &merchant,
                listing_id,
                fresh_server_url,
                &fresh_hash,
                "2.0.0",
            ),
            calls: Arc::clone(&fetch_calls),
        };
        let mut stale_listing = app_listing(&merchant, listing_id);
        stale_listing.download_url = stale_download_url.clone();
        stale_listing.specs = vec![
            ("server".to_string(), stale_server_url.to_string()),
            ("file_hash".to_string(), "stale-hash".to_string()),
        ];

        install_game_with_fetcher(
            stale_listing,
            &state,
            None::<&tauri::AppHandle<tauri::test::MockRuntime>>,
            unique_test_path("install-game-fresh-data", "dir"),
            &fetcher,
        )
        .await
        .expect("fresh listing install should succeed");

        assert_eq!(fetch_calls.load(Ordering::SeqCst), 1);
        assert_eq!(http.call_count(&fresh_download_url), 1);
        assert_eq!(http.call_count(&stale_download_url), 0);
    }

    #[test]
    fn deterministic_artifact_path_distinguishes_punctuation_collisions() {
        let base = std::path::PathBuf::from("/tmp/arcadestr-install-path-test");
        let colon_coordinate = "30402:abcdef:game:v1";
        let question_coordinate = "30402:abcdef:game?v1";

        assert_eq!(
            colon_coordinate
                .chars()
                .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '_' })
                .collect::<String>(),
            question_coordinate
                .chars()
                .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '_' })
                .collect::<String>(),
            "fixture should represent the old underscore-collision behavior"
        );

        assert_ne!(
            deterministic_artifact_path(&base, colon_coordinate),
            deterministic_artifact_path(&base, question_coordinate)
        );
    }
}

#[cfg(test)]
mod task4_tests {
    use super::*;
    use nostr::prelude::Keys;

    fn listing_with_merchant_npub(
        merchant_npub: String,
    ) -> arcadestr_core::marketplace::Nip99Listing {
        arcadestr_core::marketplace::Nip99Listing {
            event_id: String::new(),
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
            servers: Vec::new(),
            file_hash: None,
            version: None,
            fulfillment_authorizations: Vec::new(),
            malformed_fulfillment_authorization_tags: Vec::new(),
            acquisition: arcadestr_core::marketplace::AcquisitionPolicy::Gated,
            campaigns: Vec::new(),
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
            event_id: None,
            source: ListingSource::Nip99Listing,
            title: "Game".to_string(),
            description: "Description".to_string(),
            price_sats: 100,
            price_amount: Some("100".to_string()),
            price_currency: Some("SATS".to_string()),
            download_url: "https://example.com/game.zip".to_string(),
            publisher_npub: "npub1publisher".to_string(),
            created_at: 1,
            tags: Vec::new(),
            specs: Vec::new(),
            lud16: String::new(),
            images: Vec::new(),
            platforms: vec!["linux-x86_64".to_string()],
            nip94_event_id: None,
            acquisition: arcadestr_core::marketplace::AcquisitionPolicy::Gated,
            campaigns: Vec::new(),
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

        let mut public_listing = linux_listing.clone();
        public_listing.acquisition = arcadestr_core::marketplace::AcquisitionPolicy::Public;
        assert_ne!(
            listing_signature(&linux_listing),
            listing_signature(&public_listing)
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
            acquisition: arcadestr_app::models::AcquisitionPolicy::Gated,
            campaigns: Vec::new(),
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
    fn cached_core_listing_preserves_exact_non_sats_price() {
        let cached = CoreGameListing {
            id: "game-v1".to_string(),
            event_id: None,
            source: ListingSource::Nip99Listing,
            title: "Game".to_string(),
            description: "Description".to_string(),
            price_sats: 0,
            price_amount: Some("19.99".to_string()),
            price_currency: Some("USD".to_string()),
            download_url: "https://example.com/game.zip".to_string(),
            publisher_npub: "npub1publisher".to_string(),
            created_at: 7,
            tags: vec!["arcade".to_string()],
            specs: Vec::new(),
            lud16: "merchant@example.com".to_string(),
            images: vec!["https://example.com/cover.png".to_string()],
            platforms: vec!["linux-x86_64".to_string()],
            nip94_event_id: Some("nip94-event-1".to_string()),
            acquisition: arcadestr_core::marketplace::AcquisitionPolicy::Gated,
            campaigns: Vec::new(),
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
        assert_eq!(listing.price, 19.99);
        assert_eq!(listing.price_sats, 0);
        assert_eq!(listing.currency, "USD");
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
    fn test_cli_relay_defaults_to_block_discovery_ignoring_settings_allow() {
        let settings = NetworkDiscoverySettings {
            allow_insecure_public_ws: false,
            debug_relays: Some(vec!["wss://settings.example.com".to_string()]),
            block_discovery: Some(false),
        };
        let cli = parse_debug_relay_cli_args(vec![
            "--relay".to_string(),
            "wss://debug.example".to_string(),
        ])
        .expect("cli should parse");

        let resolved = resolve_debug_relay_options(cli, DebugRelayEnvOptions::default(), &settings)
            .expect("options should resolve");

        assert_eq!(
            resolved.relays,
            Some(vec!["wss://debug.example/".to_string()])
        );
        assert!(resolved.block_discovery);
        assert_eq!(resolved.source, Some(DebugRelayConfigSource::Cli));
    }

    #[test]
    fn test_env_relay_defaults_to_block_discovery_ignoring_settings_allow() {
        let settings = NetworkDiscoverySettings {
            allow_insecure_public_ws: false,
            debug_relays: Some(vec!["wss://settings.example.com".to_string()]),
            block_discovery: Some(false),
        };
        let env = parse_debug_relay_env(Some("wss://env.example".to_string()), None)
            .expect("env should parse");

        let resolved = resolve_debug_relay_options(DebugRelayCliOptions::default(), env, &settings)
            .expect("options should resolve");

        assert_eq!(
            resolved.relays,
            Some(vec!["wss://env.example/".to_string()])
        );
        assert!(resolved.block_discovery);
        assert_eq!(resolved.source, Some(DebugRelayConfigSource::Environment));
    }

    #[test]
    fn test_settings_relay_uses_settings_allow_when_no_higher_priority_inputs() {
        let settings = NetworkDiscoverySettings {
            allow_insecure_public_ws: false,
            debug_relays: Some(vec!["wss://settings.example.com".to_string()]),
            block_discovery: Some(false),
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
        assert!(!resolved.block_discovery);
        assert_eq!(resolved.source, Some(DebugRelayConfigSource::Settings));
    }

    #[test]
    fn test_cli_allow_discovery_overrides_env_block_for_env_relay_source() {
        let cli = parse_debug_relay_cli_args(vec!["--allow-discovery".to_string()])
            .expect("cli should parse");
        let env = parse_debug_relay_env(
            Some("wss://env.example".to_string()),
            Some("true".to_string()),
        )
        .expect("env should parse");

        let resolved = resolve_debug_relay_options(cli, env, &NetworkDiscoverySettings::default())
            .expect("options should resolve");

        assert_eq!(
            resolved.relays,
            Some(vec!["wss://env.example/".to_string()])
        );
        assert!(!resolved.block_discovery);
        assert_eq!(resolved.source, Some(DebugRelayConfigSource::Environment));
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

    let account_manager = runtime
        .block_on(AccountManager::new(&keys_dir))
        .expect("Failed to initialize local account manager");
    info!("Local account manager initialized");

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

    let mut builder = tauri::Builder::default();

    #[cfg(debug_assertions)]
    {
        builder = builder.plugin(tauri_plugin_mcp_bridge::init());
    }

    builder
        .plugin(tauri_plugin_dialog::init())
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
        .manage(Arc::new(account_manager))
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
            adp_commands::check_adp_server,
            adp_commands::discover_adp_servers,
            adp_commands::discover_campaigns,
            adp_commands::discover_campaign_summaries,
            adp_commands::publish_campaign,
            adp_commands::update_campaign_pointer,
            adp_commands::hash_build_file,
            adp_commands::select_build_file,
            adp_commands::connect_nwc_wallet,
            adp_commands::request_lnurl_invoice,
            adp_commands::pay_nwc_invoice,
            adp_commands::confirm_purchase,
            adp_commands::claim_entitlement,
            adp_commands::resolve_adp_operator,
            adp_commands::publish_adp_listing,
            fetch_listings,
            fetch_listing_by_id,
            fetch_marketplace,
            fetch_marketplace_stream,
            store_page_commands::enrich_store_pages,
            store_page_commands::enrich_store_page_detail,
            store_page_commands::load_publisher_store_page_editor,
            store_page_commands::validate_store_page_draft_command,
            store_page_commands::publish_store_page,
            store_page_commands::clone_store_page,
            store_page_commands::retry_store_page_pointer_sync,
            get_listing_ownership,
            get_platform_info,
            get_installed_games,
            add_game_to_library,
            is_game_in_library,
            get_library_games,
            install_game,
            ingest_receipt,
            get_purchase_records,
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
            nip46_commands::login_with_nsec,
            nip46_commands::attempt_reconnect,
            #[cfg(debug_assertions)]
            test_extended_network_discovery,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Arcadestr");
}
