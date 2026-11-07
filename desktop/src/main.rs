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
    DownloadTokensRepository, InstallationId, InstallationProfile, InstallationRecord,
    InstallationRecoveryDiscriminator, InstallationSnapshot, InstallationState,
    InstallationsRepository, LibraryGame as CoreLibraryGame, LibraryGamesRepository,
    ProfileAvailability, ProfileUnavailableReason, RetainedPackage, RetainedPackageId,
    RetainedPackageReinstallSnapshot,
};
use arcadestr_core::auth::{AccountManager, AuthState};
use arcadestr_core::distribution::{
    host_platform, resolve_install_plan, ArtifactSource, LaunchType,
};
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
use sysinfo::Disks;
use tauri::Emitter;

mod adp_commands;
mod archive_commands;
mod archive_selection;
pub mod blossom_commands;
pub mod blossom_settings;
pub mod blossom_upload;
mod command_contracts;
mod install;
mod launcher;
mod marketplace_stream;
mod nip46_commands;
mod receipt_commands;
mod settlement_ui;
mod startup_runtime;
mod store_page_commands;

use arcadestr_app::models::GameListing as AppGameListing;
use command_contracts::{
    ExportKeyRequest, ExportKeyResult, ImportKeyRequest, ImportKeyResult, Nip05Status,
    Nip49ExportResult, Nip49ImportRequest, VerifyNip05Request, VerifyNip05Result,
};

const MARKETPLACE_REFRESH_OVERLAP_SECS: u64 = 86_400;

/// Application state shared across Tauri commands.
pub struct AppState {
    pub settlement_recovery: arcadestr_core::settlement_recovery::RecoveryCoordinator,
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

    state.settlement_recovery.activate(None);
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
    state.settlement_recovery.observe(Some(pubkey.to_hex()));

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
    state
        .settlement_recovery
        .activate(auth.public_key().map(|key| key.to_hex()));
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
    state.settlement_recovery.activate(None);
    let (signer, public_key) =
        Nip46Signer::wait_for_nostrconnect_signer(uri, client_keys, timeout_secs)
            .await
            .map_err(|e| e.to_string())?;

    // Store the signer and public key
    auth.set_nip46_signer(signer);
    auth.set_public_key(public_key);
    state.settlement_recovery.observe(Some(public_key.to_hex()));

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
    state.settlement_recovery.activate(None);
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
    installation_id: String,
    game_coordinate: String,
    installation_root: String,
    package_digest: String,
    package_format: String,
    state: InstallationState,
    recovery_discriminator: Option<InstallationRecoveryDiscriminator>,
    retained_package: bool,
    launchable: bool,
    installed_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct InstallationLifecycleIssue {
    installation_id: String,
    game_coordinate: String,
    installation_root: String,
    state: InstallationState,
    recovery_discriminator: Option<InstallationRecoveryDiscriminator>,
    updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct RetainedPackageDto {
    game_coordinate: String,
    package_id: String,
    package_path: String,
    listing_event_id: Option<String>,
    size_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct UninstallSizesDto {
    installation_size_bytes: u64,
    retained_package_size_bytes: Option<u64>,
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
    expected_buyer_npub: String,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    validate_library_coordinate(&game_coordinate)?;
    let expected_buyer = nostr::PublicKey::from_bech32(&expected_buyer_npub)
        .map_err(|error| format!("invalid initiating buyer npub: {error}"))?
        .to_hex();
    let active_buyer = active_library_pubkey(&state).await?;
    if active_buyer != expected_buyer {
        return Err("active account changed before the library update".to_string());
    }
    let added_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| format!("system clock is before Unix epoch: {error}"))?
        .as_secs()
        .try_into()
        .map_err(|_| "current timestamp exceeds SQLite range".to_string())?;
    LibraryGamesRepository::new(state.database.pool().clone())
        .add(&CoreLibraryGame {
            buyer_pubkey: expected_buyer,
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
    let saved = LibraryGamesRepository::new(state.database.pool().clone())
        .list(&buyer_pubkey)
        .await
        .map_err(|error| error.to_string())?;
    arcadestr_core::ownership::OwnershipService::new(
        arcadestr_core::purchases::PurchasesRepository::new(state.database.pool().clone()),
        arcadestr_core::entitlements_repository::EntitlementsRepository::new(
            state.database.pool().clone(),
        ),
    )
    .library_entries_for(&buyer_pubkey, saved)
    .await
    .map(|games| games.into_iter().map(LibraryGame::from).collect())
    .map_err(|error| error.to_string())
}

impl From<InstallationSnapshot> for InstalledGame {
    fn from(value: InstallationSnapshot) -> Self {
        let launchable = value.profiles.iter().any(|profile| {
            profile.profile.launch_type == LaunchType::Game
                && profile.availability == ProfileAvailability::Available
        });
        let installation = value.installation;
        Self {
            installation_id: installation.id.as_str().to_string(),
            game_coordinate: installation.game_coordinate,
            installation_root: installation
                .installation_root
                .to_string_lossy()
                .into_owned(),
            package_digest: installation.package_digest.as_str().to_string(),
            package_format: package_format_dto(&installation.package_format).to_string(),
            state: installation.state,
            recovery_discriminator: installation.recovery_discriminator,
            retained_package: installation.retained_package_coordinate.is_some(),
            launchable,
            installed_at: installation.created_at,
        }
    }
}

fn package_format_dto(format: &arcadestr_core::distribution::PackageFormat) -> &'static str {
    match format {
        arcadestr_core::distribution::PackageFormat::Zip => "zip",
        arcadestr_core::distribution::PackageFormat::TarZstd => "tar_zstd",
        arcadestr_core::distribution::PackageFormat::TarGz => "tar_gz",
    }
}

#[tauri::command]
async fn get_installed_games(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<InstalledGame>, String> {
    let repo = InstallationsRepository::new(state.database.pool().clone());
    let installed_games = repo.list().await.map_err(|error| error.to_string())?;
    Ok(installed_games
        .into_iter()
        .filter(|snapshot| snapshot.installation.state == InstallationState::Ready)
        .map(InstalledGame::from)
        .collect())
}

#[tauri::command]
async fn list_installation_lifecycle_issues(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<InstallationLifecycleIssue>, String> {
    InstallationsRepository::new(state.database.pool().clone())
        .list()
        .await
        .map_err(|error| error.to_string())
        .map(|snapshots| {
            snapshots
                .into_iter()
                .filter(|snapshot| snapshot.installation.state != InstallationState::Ready)
                .map(|snapshot| InstallationLifecycleIssue {
                    installation_id: snapshot.installation.id.as_str().to_string(),
                    game_coordinate: snapshot.installation.game_coordinate,
                    installation_root: snapshot
                        .installation
                        .installation_root
                        .to_string_lossy()
                        .into_owned(),
                    state: snapshot.installation.state,
                    recovery_discriminator: snapshot.installation.recovery_discriminator,
                    updated_at: snapshot.installation.updated_at,
                })
                .collect()
        })
}

#[tauri::command]
async fn retry_installation_recovery(
    app_handle: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<install::RecoveryReport, String> {
    let app_data = app_handle
        .path()
        .app_local_data_dir()
        .map_err(|error| format!("failed to resolve app data directory: {error}"))?;
    install::recover_interrupted_installations(
        &InstallationsRepository::new(state.database.pool().clone()),
        &app_data.join("installations"),
        now_unix_i64()?,
    )
    .await
    .map_err(|error| error.to_string())
}

#[tauri::command]
async fn list_retained_packages(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<RetainedPackageDto>, String> {
    let packages = InstallationsRepository::new(state.database.pool().clone())
        .list_retained_packages()
        .await
        .map_err(|error| error.to_string())?;
    Ok(packages
        .into_iter()
        .map(|package| {
            let listing_event_id =
                match serde_json::from_str::<nostr::Event>(&package.reinstall.raw_event_json) {
                    Ok(event) => Some(event.id.to_hex()),
                    Err(error) => {
                        tracing::warn!(
                            game_coordinate = %package.game_coordinate,
                            %error,
                            "Retained package has an invalid listing snapshot"
                        );
                        None
                    }
                };
            RetainedPackageDto {
                game_coordinate: package.game_coordinate,
                package_id: package.package_id.as_str().to_string(),
                package_path: package.package_path.to_string_lossy().into_owned(),
                listing_event_id,
                size_bytes: package.size_bytes,
            }
        })
        .collect())
}

fn filesystem_object_size_no_follow(path: &Path) -> Result<u64, std::io::Error> {
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(error) => return Err(error),
    };
    if metadata.file_type().is_dir() {
        let mut total = 0_u64;
        for entry in std::fs::read_dir(path)? {
            let entry = entry?;
            total = total
                .checked_add(filesystem_object_size_no_follow(&entry.path())?)
                .ok_or_else(|| std::io::Error::other("installation size overflowed u64"))?;
        }
        Ok(total)
    } else {
        Ok(metadata.len())
    }
}

async fn get_uninstall_sizes_owned(
    repository: &InstallationsRepository,
    managed_installations: &Path,
    installation_id: &InstallationId,
) -> Result<UninstallSizesDto, String> {
    let snapshot = repository
        .get_by_id(installation_id)
        .await
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "installation not found".to_string())?;
    install::require_managed_direct_child(
        managed_installations,
        &snapshot.installation.installation_root,
    )?;
    let root = snapshot.installation.installation_root.clone();
    let installation_size_bytes =
        tokio::task::spawn_blocking(move || filesystem_object_size_no_follow(&root))
            .await
            .map_err(|error| format!("installation size task failed: {error}"))?
            .map_err(|error| format!("failed to compute installation size: {error}"))?;
    let retained_package_size_bytes = if snapshot.installation.retained_package_coordinate.is_some()
    {
        repository
            .get_retained_package(&snapshot.installation.game_coordinate)
            .await
            .map_err(|error| error.to_string())?
            .map(|package| package.size_bytes)
    } else {
        None
    };
    Ok(UninstallSizesDto {
        installation_size_bytes,
        retained_package_size_bytes,
    })
}

#[tauri::command]
async fn get_uninstall_sizes(
    installation_id: String,
    app_handle: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<UninstallSizesDto, String> {
    let id = InstallationId::parse(&installation_id).map_err(|error| error.to_string())?;
    let app_data = app_handle
        .path()
        .app_local_data_dir()
        .map_err(|error| format!("failed to resolve app data directory: {error}"))?;
    get_uninstall_sizes_owned(
        &InstallationsRepository::new(state.database.pool().clone()),
        &app_data.join("installations"),
        &id,
    )
    .await
}

async fn delete_retained_package_owned(
    repository: &InstallationsRepository,
    retained_directory: &Path,
    package: RetainedPackage,
) -> Result<(), String> {
    install::require_managed_direct_child(retained_directory, &package.package_path)?;
    let package_path = package.package_path.clone();
    let managed_directory = retained_directory.to_owned();
    tokio::task::spawn_blocking(move || {
        install::remove_managed_direct_child_no_follow(&managed_directory, &package_path)
    })
    .await
    .map_err(|error| format!("retained package deletion task failed: {error}"))?
    .map_err(|error| format!("failed to delete retained package: {error}"))?;
    repository
        .delete_retained_package(&package.game_coordinate)
        .await
        .map_err(|error| error.to_string())?;
    Ok(())
}

async fn uninstall_game_owned(
    repository: &InstallationsRepository,
    managed_installations: &Path,
    retained_directory: &Path,
    id: &InstallationId,
    retain_package: bool,
    updated_at: i64,
) -> Result<(), String> {
    let initial = repository
        .get_by_id(id)
        .await
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "installation not found".to_string())?;
    if initial.installation.state != InstallationState::Ready {
        return Err("installation is not ready for uninstall".to_string());
    }
    install::require_managed_direct_child(
        managed_installations,
        &initial.installation.installation_root,
    )?;
    let root_for_lock = initial.installation.installation_root.clone();
    let operation_lock = tokio::task::spawn_blocking(move || {
        install::OperationLock::acquire_existing_for_recovery(&root_for_lock)
    })
    .await
    .map_err(|error| format!("uninstall lock task failed: {error}"))?
    .map_err(|error| error.to_string())?;
    let current = repository
        .get_by_id(id)
        .await
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "installation changed before uninstall".to_string())?;
    if current.installation.state != InstallationState::Ready
        || current.installation.installation_root != initial.installation.installation_root
    {
        return Err("installation changed before uninstall".to_string());
    }
    install::require_managed_direct_child(
        managed_installations,
        &current.installation.installation_root,
    )?;
    let package = if retain_package {
        None
    } else {
        repository
            .get_retained_package(&current.installation.game_coordinate)
            .await
            .map_err(|error| error.to_string())?
    };
    repository
        .transition(id, InstallationState::Uninstalling, updated_at)
        .await
        .map_err(|error| error.to_string())?;
    install::require_managed_direct_child(
        managed_installations,
        &current.installation.installation_root,
    )?;
    let removal_lock = operation_lock.clone();
    tokio::task::spawn_blocking(move || removal_lock.remove_root_no_follow())
        .await
        .map_err(|error| format!("uninstall removal task failed: {error}"))?
        .map_err(|error| format!("failed to remove installation root: {error}"))?;
    repository
        .delete(id)
        .await
        .map_err(|error| error.to_string())?;
    if let Some(package) = package {
        delete_retained_package_owned(repository, retained_directory, package).await?;
    }
    Ok(())
}

async fn remove_failed_installation_owned(
    repository: &InstallationsRepository,
    managed_installations: &Path,
    id: &InstallationId,
) -> Result<(), String> {
    let initial = repository
        .get_by_id(id)
        .await
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "installation not found".to_string())?;
    if initial.installation.state != InstallationState::Failed {
        return Err("installation is not failed".to_string());
    }
    install::require_managed_direct_child(
        managed_installations,
        &initial.installation.installation_root,
    )?;
    let root_for_lock = initial.installation.installation_root.clone();
    let operation_lock = tokio::task::spawn_blocking(move || {
        install::OperationLock::acquire_existing_for_recovery(&root_for_lock)
    })
    .await
    .map_err(|error| format!("failed-installation lock task failed: {error}"))?
    .map_err(|error| error.to_string())?;
    let current = repository
        .get_by_id(id)
        .await
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "failed installation changed before removal".to_string())?;
    if current.installation.state != InstallationState::Failed
        || current.installation.installation_root != initial.installation.installation_root
    {
        return Err("failed installation changed before removal".to_string());
    }
    install::require_managed_direct_child(
        managed_installations,
        &current.installation.installation_root,
    )?;
    match current.installation.recovery_discriminator {
        Some(InstallationRecoveryDiscriminator::FinalRootPresent) => {
            let removal_lock = operation_lock.clone();
            tokio::task::spawn_blocking(move || removal_lock.remove_root_no_follow())
                .await
                .map_err(|error| format!("failed-installation removal task failed: {error}"))?
                .map_err(|error| format!("failed to remove failed installation root: {error}"))?;
        }
        Some(InstallationRecoveryDiscriminator::FinalRootAbsent) | None => {
            match operation_lock.root_exists_no_follow() {
                Ok(false) => {}
                Ok(true) => {
                    return Err(
                        "failed installation root appeared after recovery; refusing deletion"
                            .to_string(),
                    )
                }
                Err(error) => {
                    return Err(format!(
                        "failed to verify absent installation root before cleanup: {error}"
                    ))
                }
            }
        }
    }
    repository
        .delete_failed(id)
        .await
        .map_err(|error| error.to_string())
}

async fn resume_uninstall_owned(
    repository: &InstallationsRepository,
    managed_installations: &Path,
    id: &InstallationId,
) -> Result<(), String> {
    let initial = repository
        .get_by_id(id)
        .await
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "installation not found".to_string())?;
    if initial.installation.state != InstallationState::Uninstalling {
        return Err("installation is not awaiting uninstall resumption".to_string());
    }
    install::require_managed_direct_child(
        managed_installations,
        &initial.installation.installation_root,
    )?;
    let root_for_lock = initial.installation.installation_root.clone();
    let operation_lock = tokio::task::spawn_blocking(move || {
        install::OperationLock::acquire_existing_for_recovery(&root_for_lock)
    })
    .await
    .map_err(|error| format!("resume-uninstall lock task failed: {error}"))?
    .map_err(|error| error.to_string())?;
    let current = repository
        .get_by_id(id)
        .await
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "installation changed before uninstall resumption".to_string())?;
    if current.installation.state != InstallationState::Uninstalling
        || current.installation.installation_root != initial.installation.installation_root
    {
        return Err("installation changed before uninstall resumption".to_string());
    }
    install::require_managed_direct_child(
        managed_installations,
        &current.installation.installation_root,
    )?;
    let removal_lock = operation_lock.clone();
    tokio::task::spawn_blocking(move || removal_lock.remove_root_no_follow())
        .await
        .map_err(|error| format!("resume-uninstall removal task failed: {error}"))?
        .map_err(|error| format!("failed to resume installation removal: {error}"))?;
    repository
        .delete(id)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
async fn delete_retained_package(
    game_coordinate: String,
    expected_package_id: String,
    app_handle: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    let repository = InstallationsRepository::new(state.database.pool().clone());
    let app_data = app_handle
        .path()
        .app_local_data_dir()
        .map_err(|error| format!("failed to resolve app data directory: {error}"))?;
    let _coordinate_lock = acquire_coordinate_lifecycle_lock(&app_data, &game_coordinate).await?;
    install::recover_updates(
        &repository,
        &app_data,
        now_unix_i64()?,
        Some(&game_coordinate),
    )
    .await?;
    let package = repository
        .get_retained_package(&game_coordinate)
        .await
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "retained package not found".to_string())?;
    if package.package_id.as_str() != expected_package_id {
        return Err("retained package changed; refresh Library before deleting it".to_string());
    }
    delete_retained_package_owned(&repository, &app_data.join("retained-packages"), package).await
}

async fn reinstall_retained_package_with_id<R: tauri::Runtime>(
    game_coordinate: String,
    expected_package_id: String,
    app_handle: tauri::AppHandle<R>,
    installation_id: InstallationId,
) -> Result<(), String> {
    let state = app_handle.state::<AppState>();
    let repository = InstallationsRepository::new(state.database.pool().clone());
    let app_data = app_handle
        .path()
        .app_local_data_dir()
        .map_err(|error| format!("failed to resolve app data directory: {error}"))?;
    let _coordinate_lock = acquire_coordinate_lifecycle_lock(&app_data, &game_coordinate).await?;
    install::recover_updates(
        &repository,
        &app_data,
        now_unix_i64()?,
        Some(&game_coordinate),
    )
    .await?;
    let package = repository
        .get_retained_package(&game_coordinate)
        .await
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "retained package not found".to_string())?;
    if package.package_id.as_str() != expected_package_id {
        return Err("retained package changed; refresh Library before reinstalling it".to_string());
    }
    let retained_directory = app_data.join("retained-packages");
    let installations_directory = app_data.join("installations");
    install::require_managed_direct_child(&retained_directory, &package.package_path)
        .map_err(|error| format!("retained package path is untrusted: {error}"))?;
    let final_root = installations_directory.join(installation_id.as_str());
    install::require_managed_direct_child(&installations_directory, &final_root)?;
    emit_install_progress(Some(&app_handle), &game_coordinate, "preparing", 0, None);
    let root_for_lock = final_root.clone();
    let operation_lock =
        tokio::task::spawn_blocking(move || install::OperationLock::acquire(&root_for_lock))
            .await
            .map_err(|error| format!("local reinstall lock task failed: {error}"))?
            .map_err(|error| error.to_string())?;
    let outer_lock = operation_lock.clone();
    let created_at = now_unix_i64()?;
    let initial_profiles = package
        .reinstall
        .launch_profiles
        .iter()
        .cloned()
        .map(|profile| InstallationProfile {
            profile,
            availability: ProfileAvailability::Unavailable(
                ProfileUnavailableReason::ExecutableMissing,
            ),
        })
        .collect::<Vec<_>>();
    let installation = InstallationRecord {
        id: installation_id.clone(),
        game_coordinate: package.game_coordinate.clone(),
        installation_root: final_root.clone(),
        package_digest: package.package_digest.clone(),
        package_format: package.package_format.clone(),
        resolved_target: package.reinstall.resolved_target.clone(),
        raw_event_json: package.reinstall.raw_event_json.clone(),
        publisher_default_profile_id: package.reinstall.publisher_default_profile_id.clone(),
        state: InstallationState::Installing,
        recovery_discriminator: None,
        retained_package_coordinate: Some(package.game_coordinate.clone()),
        created_at,
        updated_at: created_at,
    };
    repository
        .begin(&installation, &initial_profiles)
        .await
        .map_err(|error| error.to_string())?;

    let reinstall_result = async {
        emit_install_progress(
            Some(&app_handle),
            &game_coordinate,
            "verifying",
            package.size_bytes,
            Some(package.size_bytes),
        );
        let package_path = package.package_path.clone();
        let capability_root = app_data.clone();
        let expected_digest = package.package_digest.clone();
        let expected_format = package.package_format.clone();
        let verified = tokio::task::spawn_blocking(move || {
            let file = open_existing_retained_regular_file(&capability_root, &package_path)?;
            arcadestr_core::package_extraction::VerifiedPackage::verify(
                file,
                expected_digest,
                expected_format,
            )
            .map_err(|error| format!("retained package is corrupt or invalid: {error}"))
        })
        .await
        .map_err(|error| format!("local reinstall verification task failed: {error}"))??;
        extraction_storage_preflight(
            installation_disk(&final_root).map(|(available, _)| available),
            verified.declared_regular_file_size_bytes(),
        )
        .map_err(|error| desktop_download_error(AdpClientError::Http(error)))?;
        emit_install_progress(
            Some(&app_handle),
            &game_coordinate,
            "installing",
            package.size_bytes,
            Some(package.size_bytes),
        );
        let root = final_root.clone();
        let profiles = package.reinstall.launch_profiles.clone();
        let placement_lock = operation_lock;
        let placed = tokio::task::spawn_blocking(move || {
            install::extract_and_place_verified_package(&verified, &root, &profiles, placement_lock)
        })
        .await
        .map_err(|error| format!("local reinstall extraction task failed: {error}"))?
        .map_err(|error| {
            if error.is_insufficient_storage() {
                "insufficient_storage: not enough storage for local reinstall".to_string()
            } else if error.is_archive_policy() {
                format!("archive_policy: {error}")
            } else {
                error.to_string()
            }
        })?;
        install::commit_placed_installation(&repository, &installation_id, placed, now_unix_i64()?)
            .await
            .map_err(|error| error.to_string())?;
        emit_install_progress(
            Some(&app_handle),
            &game_coordinate,
            "complete",
            package.size_bytes,
            Some(package.size_bytes),
        );
        Ok::<(), String>(())
    }
    .await;

    if let Err(error) = reinstall_result {
        if let Ok(Some(current)) = repository.get_by_id(&installation_id).await {
            if current.installation.state == InstallationState::Installing {
                let discriminator = match std::fs::symlink_metadata(&final_root) {
                    Err(io) if io.kind() == std::io::ErrorKind::NotFound => {
                        InstallationRecoveryDiscriminator::FinalRootAbsent
                    }
                    Ok(_) | Err(_) => InstallationRecoveryDiscriminator::FinalRootPresent,
                };
                let failed_at = match now_unix_i64() {
                    Ok(timestamp) => timestamp,
                    Err(_) => current.installation.updated_at,
                };
                let _ = repository
                    .mark_failed_for_recovery_if_unchanged(
                        &installation_id,
                        current.installation.updated_at,
                        discriminator,
                        failed_at,
                    )
                    .await;
            }
        }
        drop(outer_lock);
        return Err(error);
    }
    drop(outer_lock);
    Ok(())
}

#[tauri::command]
async fn reinstall_retained_package(
    game_coordinate: String,
    expected_package_id: String,
    app_handle: tauri::AppHandle,
) -> Result<(), String> {
    reinstall_retained_package_with_id(
        game_coordinate,
        expected_package_id,
        app_handle,
        InstallationId::generate(),
    )
    .await
}

#[tauri::command]
async fn uninstall_game(
    installation_id: String,
    retain_package: bool,
    app_handle: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    let id = InstallationId::parse(&installation_id).map_err(|error| error.to_string())?;
    let repository = InstallationsRepository::new(state.database.pool().clone());
    let app_data = app_handle
        .path()
        .app_local_data_dir()
        .map_err(|error| format!("failed to resolve app data directory: {error}"))?;
    let initial = repository
        .get_by_id(&id)
        .await
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "installation not found".to_string())?;
    let _coordinate_lock =
        acquire_coordinate_lifecycle_lock(&app_data, &initial.installation.game_coordinate).await?;
    install::recover_updates(
        &repository,
        &app_data,
        now_unix_i64()?,
        Some(&initial.installation.game_coordinate),
    )
    .await?;
    let managed = app_data.join("installations");
    uninstall_game_owned(
        &repository,
        &managed,
        &app_data.join("retained-packages"),
        &id,
        retain_package,
        now_unix_i64()?,
    )
    .await
}

#[tauri::command]
async fn remove_failed_installation(
    installation_id: String,
    app_handle: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    let id = InstallationId::parse(&installation_id).map_err(|error| error.to_string())?;
    let app_data = app_handle
        .path()
        .app_local_data_dir()
        .map_err(|error| format!("failed to resolve app data directory: {error}"))?;
    let repository = InstallationsRepository::new(state.database.pool().clone());
    let initial = repository
        .get_by_id(&id)
        .await
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "installation not found".to_string())?;
    let _coordinate_lock =
        acquire_coordinate_lifecycle_lock(&app_data, &initial.installation.game_coordinate).await?;
    install::recover_updates(
        &repository,
        &app_data,
        now_unix_i64()?,
        Some(&initial.installation.game_coordinate),
    )
    .await?;
    remove_failed_installation_owned(&repository, &app_data.join("installations"), &id).await
}

#[tauri::command]
async fn resume_uninstall(
    installation_id: String,
    app_handle: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    let id = InstallationId::parse(&installation_id).map_err(|error| error.to_string())?;
    let app_data = app_handle
        .path()
        .app_local_data_dir()
        .map_err(|error| format!("failed to resolve app data directory: {error}"))?;
    let repository = InstallationsRepository::new(state.database.pool().clone());
    let initial = repository
        .get_by_id(&id)
        .await
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "installation not found".to_string())?;
    let _coordinate_lock =
        acquire_coordinate_lifecycle_lock(&app_data, &initial.installation.game_coordinate).await?;
    install::recover_updates(
        &repository,
        &app_data,
        now_unix_i64()?,
        Some(&initial.installation.game_coordinate),
    )
    .await?;
    resume_uninstall_owned(&repository, &app_data.join("installations"), &id).await
}

type InstallFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

trait FreshListingFetcher: Send + Sync {
    fn fetch<'a>(&'a self, coordinate: &'a str) -> InstallFuture<'a, Result<nostr::Event, String>>;
}

struct RelayFreshListingFetcher<'a> {
    state: tauri::State<'a, AppState>,
}

struct InstallAccountRecheckingSigner {
    inner: Arc<dyn NostrSigner>,
    expected: nostr::PublicKey,
    auth: Arc<Mutex<AuthState>>,
    signer_state: Arc<Mutex<AppSignerState>>,
}

impl InstallAccountRecheckingSigner {
    async fn recheck_current_public_key(
        &self,
    ) -> Result<nostr::PublicKey, arcadestr_core::signers::SignerError> {
        let auth = { self.auth.lock().await.clone() };
        let current = adp_commands::resolve_active_signer(&self.signer_state, &auth)
            .await
            .map_err(arcadestr_core::signers::SignerError::SigningFailed)?
            .get_public_key()
            .await?;
        if current != self.expected {
            return Err(arcadestr_core::signers::SignerError::SigningFailed(
                "active account changed while installation was being prepared".to_string(),
            ));
        }
        Ok(current)
    }
}

#[async_trait::async_trait]
impl NostrSigner for InstallAccountRecheckingSigner {
    async fn get_public_key(
        &self,
    ) -> Result<nostr::PublicKey, arcadestr_core::signers::SignerError> {
        self.recheck_current_public_key().await
    }

    async fn sign_event(
        &self,
        unsigned: nostr::UnsignedEvent,
    ) -> Result<nostr::Event, arcadestr_core::signers::SignerError> {
        let signed = self.inner.sign_event(unsigned).await?;
        self.recheck_current_public_key().await?;
        Ok(signed)
    }
}

impl FreshListingFetcher for RelayFreshListingFetcher<'_> {
    fn fetch<'a>(&'a self, coordinate: &'a str) -> InstallFuture<'a, Result<nostr::Event, String>> {
        Box::pin(async move {
            adp_commands::fetch_listing_event_by_coordinate(&self.state, coordinate).await
        })
    }
}

#[derive(Debug, Clone, Serialize)]
struct InstallProgressPayload {
    game_coordinate: String,
    phase: &'static str,
    bytes: u64,
    total: Option<u64>,
}

const INSTALL_STORAGE_MARGIN: u64 = 512 * 1024 * 1024;

fn emit_install_progress<R: tauri::Runtime>(
    app: Option<&tauri::AppHandle<R>>,
    game_coordinate: &str,
    phase: &'static str,
    bytes: u64,
    total: Option<u64>,
) {
    if let Some(app) = app {
        let _ = app.emit(
            "install-progress",
            InstallProgressPayload {
                game_coordinate: game_coordinate.to_string(),
                phase,
                bytes,
                total,
            },
        );
    }
}

fn installation_disk(path: &Path) -> Option<(u64, String)> {
    Disks::new_with_refreshed_list()
        .list()
        .iter()
        .filter(|disk| path.starts_with(disk.mount_point()))
        .max_by_key(|disk| disk.mount_point().components().count())
        .map(|disk| {
            (
                disk.available_space(),
                disk.file_system().to_string_lossy().to_ascii_lowercase(),
            )
        })
}

fn download_storage_preflight(
    path: &Path,
    length: Option<u64>,
) -> Result<(), arcadestr_core::http_client::HttpClientError> {
    let Some(length) = length else {
        tracing::warn!(
            "install package has no Content-Length; proceeding with streaming safeguards"
        );
        return Ok(());
    };
    if let Some((available, filesystem)) = installation_disk(path) {
        let required = length.saturating_add(INSTALL_STORAGE_MARGIN);
        if available < required {
            return Err(
                arcadestr_core::http_client::HttpClientError::InsufficientStorage(format!(
                    "{required} bytes required but {available} bytes available"
                )),
            );
        }
        if matches!(filesystem.as_str(), "fat32" | "vfat" | "msdos" | "msdosfs")
            && length > u64::from(u32::MAX)
        {
            return Err(
                arcadestr_core::http_client::HttpClientError::FilesystemFileSizeLimit(format!(
                    "{filesystem} cannot retain a {length}-byte package"
                )),
            );
        }
    } else {
        tracing::warn!(
            "install root mount was not reported; proceeding without capacity preflight"
        );
    }
    Ok(())
}

fn extraction_storage_preflight(
    available: Option<u64>,
    declared_regular_file_size: u64,
) -> Result<(), arcadestr_core::http_client::HttpClientError> {
    let Some(available) = available else {
        tracing::warn!(
            "install root mount was not reported after verification; extraction capacity cannot be preflighted"
        );
        return Ok(());
    };
    let required = declared_regular_file_size.saturating_add(INSTALL_STORAGE_MARGIN);
    if available < required {
        return Err(
            arcadestr_core::http_client::HttpClientError::InsufficientStorage(format!(
                "verified package declares {declared_regular_file_size} regular-file bytes; \
                 {required} bytes required with margin but {available} bytes available"
            )),
        );
    }
    Ok(())
}

fn open_existing_retained_regular_file(
    capability_root: &Path,
    path: &Path,
) -> Result<Arc<std::fs::File>, String> {
    let relative = path.strip_prefix(capability_root).map_err(|_| {
        format!(
            "retained package path is outside the application data directory: {}",
            path.display()
        )
    })?;
    let directory =
        cap_std::fs::Dir::open_ambient_dir(capability_root, cap_std::ambient_authority()).map_err(
            |error| {
                format!(
                    "application data directory could not be opened securely at {}: {error}",
                    capability_root.display()
                )
            },
        )?;
    let path_metadata = directory.symlink_metadata(relative).map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            format!(
                "retained package file is absent at {}; remove the retained-package record before retrying",
                path.display()
            )
        } else {
            format!(
                "retained package file could not be inspected at {}: {error}",
                path.display()
            )
        }
    })?;
    if path_metadata.file_type().is_symlink() {
        return Err(format!(
            "retained package path is a symbolic link: {}",
            path.display()
        ));
    }
    let mut options = cap_std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use cap_std::fs::OpenOptionsExt;

        #[cfg(any(target_os = "linux", target_os = "android"))]
        options.custom_flags(0o400000);
        #[cfg(any(
            target_os = "macos",
            target_os = "ios",
            target_os = "freebsd",
            target_os = "dragonfly",
            target_os = "netbsd",
            target_os = "openbsd"
        ))]
        options.custom_flags(0x100);
        #[cfg(not(any(
            target_os = "linux",
            target_os = "android",
            target_os = "macos",
            target_os = "ios",
            target_os = "freebsd",
            target_os = "dragonfly",
            target_os = "netbsd",
            target_os = "openbsd"
        )))]
        return Err(
            "retained-package no-follow open is unsupported on this Unix target".to_string(),
        );
    }
    let file = directory.open_with(relative, &options).map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            format!(
                "retained package file is absent at {}; remove the retained-package record before retrying",
                path.display()
            )
        } else {
            format!(
                "retained package file could not be inspected at {}: {error}",
                path.display()
            )
        }
    })?.into_std();
    let opened_metadata = file.metadata().map_err(|error| {
        format!(
            "opened retained package could not be inspected at {}: {error}",
            path.display()
        )
    })?;
    if !opened_metadata.file_type().is_file() {
        return Err(format!(
            "opened retained package is not a regular file: {}",
            path.display()
        ));
    }
    Ok(Arc::new(file))
}

#[derive(Debug, Clone)]
struct FreshListingMetadata {
    server_urls: Vec<String>,
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

async fn acquire_coordinate_lifecycle_lock(
    app_data: &Path,
    game_coordinate: &str,
) -> Result<install::OperationLock, String> {
    let lock_root = app_data
        .join("lifecycle-locks")
        .join(coordinate_artifact_dir_name(game_coordinate));
    tokio::task::spawn_blocking(move || install::OperationLock::acquire(&lock_root))
        .await
        .map_err(|error| format!("lifecycle lock task failed: {error}"))?
        .map_err(|error| error.to_string())
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
        AdpClientError::Http(
            arcadestr_core::http_client::HttpClientError::InsufficientStorage(message),
        ) => format!("insufficient_storage: not enough storage to install the package: {message}"),
        AdpClientError::Http(
            arcadestr_core::http_client::HttpClientError::FilesystemFileSizeLimit(message),
        ) => format!("archive_policy: package exceeds the filesystem file-size limit: {message}"),
        AdpClientError::Http(
            arcadestr_core::http_client::HttpClientError::Status(413)
            | arcadestr_core::http_client::HttpClientError::StatusWithBody { status: 413, .. },
        ) => "archive_policy: package request rejected as too large (HTTP 413)".to_string(),
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

use arcadestr_core::install_contract::InstallCommandError;

async fn install_game_with_fetcher_and_signer<R: tauri::Runtime>(
    listing: AppGameListing,
    state: &AppState,
    active_signer: Option<Arc<dyn NostrSigner>>,
    app: Option<&tauri::AppHandle<R>>,
    app_data_dir: PathBuf,
    fresh_listing_fetcher: &dyn FreshListingFetcher,
) -> Result<(), InstallCommandError> {
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
    let host = host_platform();
    let resolved = resolve_install_plan(&fresh_listing, &host)
        .map_err(|error| format!("fresh listing has no installable host plan: {error}"))?;
    let plan = resolved.plan;
    let resolved_coordinate = format!(
        "{}:{}:{}",
        plan.game_coordinate.kind,
        plan.game_coordinate.publisher_pubkey_hex,
        plan.game_coordinate.d_tag
    );
    if resolved_coordinate != coordinate {
        return Err("resolved fresh listing coordinate mismatch".into());
    }
    let tokens = DownloadTokensRepository::new(state.database.pool().clone());
    let mut cached_tokens = Vec::new();
    for server_url in &metadata.server_urls {
        if let Some(token) = tokens
            .valid_token(&buyer_pubkey_hex, &coordinate, server_url, now_unix_i64()?)
            .await
            .map_err(|error| error.to_string())?
        {
            cached_tokens.push((server_url.clone(), token.token));
        }
    }

    let has_durable_ownership = {
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
    if cached_tokens.is_empty()
        && !has_durable_ownership
        && !metadata
            .acquisition
            .allows_access_at(now_unix_i64()? as u64)
    {
        return Err("ownership or explicit current access not found".into());
    }

    let authorized_servers = metadata.server_urls.iter().filter(|server_url| plan.artifact.sources.iter().any(|source| {
        matches!(source, ArtifactSource::Adp { base_url } if base_url.trim_end_matches('/') == server_url.trim_end_matches('/'))
    })).cloned().collect::<Vec<_>>();
    let candidates = arcadestr_core::adp_download::download_candidates(
        &authorized_servers,
        &cached_tokens,
        has_durable_ownership
            || metadata
                .acquisition
                .allows_access_at(now_unix_i64()? as u64),
    );
    if candidates.is_empty() {
        return Err("fresh listing has no eligible authorized server".into());
    }

    let _coordinate_lock = acquire_coordinate_lifecycle_lock(&app_data_dir, &coordinate).await?;
    emit_install_progress(app, &coordinate, "preparing", 0, None);
    let repository = InstallationsRepository::new(state.database.pool().clone());
    let installations_directory = app_data_dir.join("installations");
    let retained_directory = app_data_dir.join("retained-packages");
    install::recover_updates(
        &repository,
        &app_data_dir,
        now_unix_i64()?,
        Some(&coordinate),
    )
    .await?;
    let existing_retained = repository
        .get_retained_package(&coordinate)
        .await
        .map_err(|error| error.to_string())?;
    let retained_matches = !existing_retained.as_ref().is_some_and(|package| {
        package.package_digest != plan.artifact.digest
            || package.package_format != plan.artifact.package_format
            || package.reinstall.raw_event_json != plan.source_event_json
            || package.reinstall.resolved_target != host
            || package.reinstall.launch_profiles != plan.launch_profiles
    });
    if repository
        .pending_updates()
        .await
        .map_err(|error| error.to_string())?
        .iter()
        .any(|update| update.candidate.game_coordinate == coordinate)
    {
        return Err("an interrupted update requires recovery before retrying".into());
    }
    let mut update_from = None;
    let failed_installation_id = if let Some(existing) = repository
        .get_by_coordinate(&coordinate)
        .await
        .map_err(|error| error.to_string())?
    {
        match existing.installation.state {
            InstallationState::Failed => Some(existing.installation.id),
            InstallationState::Ready => {
                let existing_profiles = existing
                    .profiles
                    .iter()
                    .map(|profile| &profile.profile)
                    .collect::<Vec<_>>();
                let requested_profiles = plan.launch_profiles.iter().collect::<Vec<_>>();
                if existing.installation.package_digest == plan.artifact.digest
                    && existing.installation.package_format == plan.artifact.package_format
                    && existing.installation.raw_event_json == plan.source_event_json
                    && existing.installation.resolved_target == host
                    && existing_profiles == requested_profiles
                {
                    return Ok(());
                }
                update_from = Some(existing);
                None
            }
            InstallationState::Installing => {
                return Err("an installation for this game is already in progress".into());
            }
            InstallationState::Uninstalling => {
                return Err("this game is still being uninstalled; retry when it finishes".into());
            }
        }
    } else {
        None
    };
    if !retained_matches && update_from.is_none() {
        return Err("remove the retained older build before installing a different revision without a ready installation".into());
    }
    // Never reuse a retained archive from a different signed revision.
    let verified_retained = if let Some(package) =
        existing_retained.filter(|_| retained_matches && update_from.is_none())
    {
        install::require_managed_direct_child(&retained_directory, &package.package_path)
            .map_err(|error| format!("retained package path is untrusted: {error}"))?;
        emit_install_progress(
            app,
            &coordinate,
            "verifying",
            package.size_bytes,
            Some(package.size_bytes),
        );
        let retained_path = package.package_path.clone();
        let capability_root = app_data_dir.to_owned();
        let expected_digest = package.package_digest.clone();
        let expected_format = package.package_format.clone();
        let verified = tokio::task::spawn_blocking(move || {
            let file = open_existing_retained_regular_file(&capability_root, &retained_path)?;
            arcadestr_core::package_extraction::VerifiedPackage::verify(
                file,
                expected_digest,
                expected_format,
            )
            .map_err(|error| match InstallCommandError::from_extraction(&error) {
                InstallCommandError::CompressionRatioExceeded => {
                    InstallCommandError::CompressionRatioExceeded
                }
                _ => format!("retained package is corrupt or invalid: {error}").into(),
            })
        })
        .await
        .map_err(|error| format!("retained package verification task failed: {error}"))??;
        Some((package, verified))
    } else {
        None
    };
    if let Some(failed_id) = failed_installation_id {
        remove_failed_installation_owned(&repository, &installations_directory, &failed_id).await?;
    }
    let installation_id = InstallationId::generate();
    let package_id = RetainedPackageId::generate();
    let final_root = installations_directory.join(installation_id.as_str());
    let package_path = retained_directory.join(format!("{}.package", package_id.as_str()));
    install::require_managed_direct_child(&installations_directory, &final_root)?;
    install::require_managed_direct_child(&retained_directory, &package_path)?;
    let _retained_directory_guard = if verified_retained.is_none() {
        let retained_directory_for_create = retained_directory.clone();
        Some(
            tokio::task::spawn_blocking(move || {
                install::open_or_create_managed_directory(&retained_directory_for_create)
            })
            .await
            .map_err(|error| format!("retained directory task failed: {error}"))?
            .map_err(|error| error.to_string())?,
        )
    } else {
        None
    };
    let root_for_lock = final_root.clone();
    let operation_lock =
        tokio::task::spawn_blocking(move || install::OperationLock::acquire(&root_for_lock))
            .await
            .map_err(|error| format!("install lock task failed: {error}"))?
            .map_err(|error| error.to_string())?;
    let outer_lock = operation_lock.clone();
    let created_at = now_unix_i64()?;
    let initial_profiles = plan
        .launch_profiles
        .iter()
        .cloned()
        .map(|profile| InstallationProfile {
            profile,
            availability: ProfileAvailability::Unavailable(
                ProfileUnavailableReason::ExecutableMissing,
            ),
        })
        .collect::<Vec<_>>();
    let installation = InstallationRecord {
        id: update_from
            .as_ref()
            .map(|old| old.installation.id.clone())
            .unwrap_or_else(|| installation_id.clone()),
        game_coordinate: coordinate.clone(),
        installation_root: final_root.clone(),
        package_digest: plan.artifact.digest.clone(),
        package_format: plan.artifact.package_format.clone(),
        resolved_target: host,
        raw_event_json: plan.source_event_json,
        publisher_default_profile_id: plan.publisher_default_profile_id.clone(),
        state: InstallationState::Installing,
        recovery_discriminator: None,
        retained_package_coordinate: None,
        created_at,
        updated_at: created_at,
    };
    let installation_id = installation.id.clone();
    let update_id = if let Some(old) = &update_from {
        Some(
            repository
                .begin_update(old, &installation, &initial_profiles, &package_path)
                .await
                .map_err(|error| error.to_string())?,
        )
    } else {
        repository
            .begin(&installation, &initial_profiles)
            .await
            .map_err(|error| error.to_string())?;
        None
    };

    if let Some((retained_package, verified)) = verified_retained {
        let retained_result = async {
            extraction_storage_preflight(
                installation_disk(&final_root).map(|(available, _)| available),
                verified.declared_regular_file_size_bytes(),
            )
            .map_err(|error| desktop_download_error(AdpClientError::Http(error)))?;
            repository
                .attach_existing_retained_installing(
                    &installation_id,
                    &retained_package,
                    now_unix_i64()?,
                )
                .await
                .map_err(|error| error.to_string())?;
            emit_install_progress(
                app,
                &coordinate,
                "installing",
                retained_package.size_bytes,
                Some(retained_package.size_bytes),
            );
            let root = final_root.clone();
            let profiles = plan.launch_profiles;
            let placement_lock = operation_lock;
            let placed = tokio::task::spawn_blocking(move || {
                install::extract_and_place_verified_package(
                    &verified,
                    &root,
                    &profiles,
                    placement_lock,
                )
            })
            .await
            .map_err(|error| format!("retained package extraction task failed: {error}"))?
            .map_err(|error| {
                if let install::InstallationPlacementError::Extraction(extraction) = &error {
                    if InstallCommandError::from_extraction(extraction)
                        == InstallCommandError::CompressionRatioExceeded
                    {
                        return InstallCommandError::CompressionRatioExceeded;
                    }
                }
                InstallCommandError::from(if error.is_insufficient_storage() {
                    "insufficient_storage: not enough storage to install the retained package"
                        .to_string()
                } else if error.is_archive_policy() {
                    format!("archive_policy: {error}")
                } else {
                    error.to_string()
                })
            })?;
            install::commit_placed_installation(
                &repository,
                &installation_id,
                placed,
                now_unix_i64()?,
            )
            .await
            .map_err(|error| error.to_string())?;
            emit_install_progress(
                app,
                &coordinate,
                "complete",
                retained_package.size_bytes,
                Some(retained_package.size_bytes),
            );
            Ok::<(), InstallCommandError>(())
        }
        .await;
        if let Err(error) = retained_result {
            if let Ok(Some(current)) = repository.get_by_id(&installation_id).await {
                if current.installation.state == InstallationState::Installing {
                    let discriminator = match std::fs::symlink_metadata(&final_root) {
                        Err(io) if io.kind() == std::io::ErrorKind::NotFound => {
                            InstallationRecoveryDiscriminator::FinalRootAbsent
                        }
                        Ok(_) | Err(_) => InstallationRecoveryDiscriminator::FinalRootPresent,
                    };
                    let _ = repository
                        .mark_failed_for_recovery_if_unchanged(
                            &installation_id,
                            current.installation.updated_at,
                            discriminator,
                            now_unix_i64().unwrap_or(current.installation.updated_at),
                        )
                        .await;
                }
            }
            drop(outer_lock);
            return Err(error);
        }
        drop(outer_lock);
        return Ok(());
    }

    let mut package_persisted = false;
    let mut candidate_lease = None;
    let install_result = async {
        let current_pubkey = signer
            .get_public_key()
            .await
            .map_err(|error| format!("active signer could not be rechecked: {error}"))?;
        if current_pubkey.to_hex() != buyer_pubkey_hex {
            return Err(InstallCommandError::from(
                "active account changed while installation was being prepared",
            ));
        }
        emit_install_progress(app, &coordinate, "downloading", 0, None);
        let preflight_root = final_root.clone();
        let progress_coordinate = coordinate.clone();
        let retained_capability = _retained_directory_guard
            .as_ref()
            .ok_or_else(|| "retained package directory capability is unavailable".to_string())?;
        candidate_lease = Some(
            install::create_managed_direct_child_file(retained_capability, &package_path)
                .map_err(|error| error.to_string())?,
        );
        let download_file = candidate_lease
            .as_ref()
            .ok_or_else(|| "retained package candidate lease is unavailable".to_string())?
            .download_file()
            .map_err(|error| error.to_string())?;
        let verified_download = arcadestr_core::adp_download::download_verified(
            Arc::clone(&state.http_client),
            &coordinate,
            &buyer_pubkey_hex,
            signer.as_ref(),
            candidates,
            download_file,
            plan.artifact.digest.clone(),
            plan.artifact.package_format.clone(),
            move |length| download_storage_preflight(&preflight_root, length),
            |bytes, total| {
                emit_install_progress(app, &progress_coordinate, "downloading", bytes, total);
            },
            |bytes| emit_install_progress(app, &coordinate, "verifying", bytes, Some(bytes)),
        )
        .await
        .map_err(|error| match error {
            arcadestr_core::adp_download::DownloadFailure::Adp(error) => {
                InstallCommandError::from(desktop_download_error(error))
            }
            arcadestr_core::adp_download::DownloadFailure::Package(error) => {
                match InstallCommandError::from_extraction(&error) {
                    InstallCommandError::CompressionRatioExceeded => {
                        InstallCommandError::CompressionRatioExceeded
                    }
                    _ => format!("package verification failed: {error}").into(),
                }
            }
        })?;
        let download = verified_download.download;
        let verified = verified_download.package;
        extraction_storage_preflight(
            installation_disk(&final_root).map(|(available, _)| available),
            verified.declared_regular_file_size_bytes(),
        )
        .map_err(|error| desktop_download_error(AdpClientError::Http(error)))?;
        let retained_package = RetainedPackage {
            game_coordinate: coordinate.clone(),
            package_id,
            package_path: package_path.clone(),
            package_digest: verified.digest().clone(),
            package_format: verified.format().clone(),
            reinstall: RetainedPackageReinstallSnapshot {
                raw_event_json: installation.raw_event_json.clone(),
                resolved_target: installation.resolved_target.clone(),
                launch_profiles: plan.launch_profiles.clone(),
                publisher_default_profile_id: installation.publisher_default_profile_id.clone(),
            },
            size_bytes: download.bytes_written,
            created_at,
            updated_at: now_unix_i64()?,
        };
        if update_id.is_none() {
            repository
                .retain_and_attach_installing(&installation_id, &retained_package)
                .await
                .map_err(|error| error.to_string())?;
            package_persisted = true;
        }
        emit_install_progress(
            app,
            &coordinate,
            "installing",
            download.bytes_written,
            Some(download.bytes_written),
        );
        let root = final_root.clone();
        let profiles = plan.launch_profiles;
        let placement_lock = operation_lock;
        let placed = tokio::task::spawn_blocking(move || {
            install::extract_and_place_verified_package(&verified, &root, &profiles, placement_lock)
        })
        .await
        .map_err(|error| format!("package extraction task failed: {error}"))?
        .map_err(|error| {
            if let install::InstallationPlacementError::Extraction(extraction) = &error {
                if InstallCommandError::from_extraction(extraction)
                    == InstallCommandError::CompressionRatioExceeded
                {
                    return InstallCommandError::CompressionRatioExceeded;
                }
            }
            InstallCommandError::from(if error.is_insufficient_storage() {
                "insufficient_storage: not enough storage to install the package".to_string()
            } else if error.is_archive_policy() {
                format!("archive_policy: {error}")
            } else {
                error.to_string()
            })
        })?;
        if let Some(id) = &update_id {
            install::commit_placed_update(
                &repository,
                id,
                &retained_package,
                placed,
                now_unix_i64()?,
            )
            .await?;
            package_persisted = true;
            // Cleanup is recoverable and never makes a committed replacement fail.
            if let Err(error) = install::recover_updates(
                &repository,
                &app_data_dir,
                now_unix_i64()?,
                Some(&coordinate),
            )
            .await
            {
                tracing::warn!(%error,"update cleanup deferred to recovery");
            }
        } else {
            install::commit_placed_installation(
                &repository,
                &installation_id,
                placed,
                now_unix_i64()?,
            )
            .await
            .map_err(|error| error.to_string())?;
        }
        emit_install_progress(
            app,
            &coordinate,
            "complete",
            download.bytes_written,
            Some(download.bytes_written),
        );
        Ok::<(), InstallCommandError>(())
    }
    .await;

    if let Err(mut error) = install_result {
        if update_id.is_some() {
            drop(outer_lock);
            // The ready version was never modified. Journal-driven cleanup is
            // restart-safe; retain intent if any capability cleanup fails.
            let _ = install::recover_updates(
                &repository,
                &app_data_dir,
                now_unix_i64()?,
                Some(&coordinate),
            )
            .await;
            return Err(error);
        }
        if !package_persisted {
            if let Some(lease) = candidate_lease.take() {
                let cleanup_result = tokio::task::spawn_blocking(move || lease.remove_if_owned())
                    .await
                    .map_err(|join_error| {
                        format!("retained package cleanup task failed: {join_error}")
                    })
                    .and_then(|result| result.map_err(|cleanup_error| cleanup_error.to_string()));
                if let Err(cleanup_error) = cleanup_result {
                    tracing::warn!(%cleanup_error, "retained package cleanup failed");
                    if let InstallCommandError::Other { message } = &mut error {
                        *message = format!("{message}; {cleanup_error}");
                    }
                }
            }
        }
        if let Ok(Some(current)) = repository.get_by_id(&installation_id).await {
            if current.installation.state == InstallationState::Installing {
                let discriminator = match std::fs::symlink_metadata(&final_root) {
                    Err(io) if io.kind() == std::io::ErrorKind::NotFound => {
                        InstallationRecoveryDiscriminator::FinalRootAbsent
                    }
                    Ok(_) | Err(_) => InstallationRecoveryDiscriminator::FinalRootPresent,
                };
                let _ = repository
                    .mark_failed_for_recovery_if_unchanged(
                        &installation_id,
                        current.installation.updated_at,
                        discriminator,
                        now_unix_i64().unwrap_or(current.installation.updated_at),
                    )
                    .await;
            }
        }
        drop(outer_lock);
        return Err(error);
    }
    drop(outer_lock);
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
    .map_err(|error| error.to_string())
}

#[tauri::command]
async fn install_game(
    listing: AppGameListing,
    app_handle: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    signer_state: tauri::State<'_, Arc<Mutex<AppSignerState>>>,
) -> Result<(), InstallCommandError> {
    let listing_id = listing.id.clone();
    let listing_title = listing.title.clone();
    tracing::info!(
        "install_game called for listing '{}' ({})",
        listing_title,
        listing_id
    );

    let app_data_dir = app_handle
        .path()
        .app_local_data_dir()
        .map_err(|error| format!("failed to resolve app data directory: {error}"))?;
    let fetcher = RelayFreshListingFetcher {
        state: state.clone(),
    };
    let auth_snapshot = { state.auth.lock().await.clone() };
    let signer = adp_commands::resolve_active_signer(signer_state.inner(), &auth_snapshot).await?;
    let expected = signer
        .get_public_key()
        .await
        .map_err(|error| format!("failed to verify active install account: {error}"))?;
    let signer: Arc<dyn NostrSigner> = Arc::new(InstallAccountRecheckingSigner {
        inner: signer,
        expected,
        auth: state.auth.clone(),
        signer_state: signer_state.inner().clone(),
    });
    let result = install_game_with_fetcher_and_signer(
        listing,
        &state,
        Some(signer),
        Some(&app_handle),
        app_data_dir,
        &fetcher,
    )
    .await;
    if let Err(error) = &result {
        tracing::error!(%listing_id, %listing_title, %error, "Game installation failed");
    }
    result
}

#[tauri::command]
async fn ingest_receipt(
    app: tauri::AppHandle,
    raw_event_json: String,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    let buyer = {
        let auth = state.auth.lock().await;
        auth.public_key().ok_or("not authenticated")?
    };
    let report = receipt_commands::import_for_account(buyer, &raw_event_json, &state).await?;
    let _ = app.emit(
        "receipt-ownership-updated",
        serde_json::json!({"account":buyer.to_hex()}),
    );
    if report.rejected_events > 0
        || report.chains.iter().any(|c| {
            c.verification != arcadestr_core::receipt_sync_contract::ReceiptVerification::Verified
        })
    {
        return Err("Receipt import contains invalid or indeterminate evidence; inspect Purchase History verification results".into());
    }
    Ok(())
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

    match &products {
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

    // Persist and deliver partial results before propagating a refresh failure.
    // Only successful refreshes may mark frontend caches fresh.
    marketplace_stream::finish(products, || {
        let _ = window.emit(&complete_event, ());
    })
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
    #[test]
    fn archive_policy_download_mapping_uses_typed_size_errors() {
        use arcadestr_core::http_client::HttpClientError;

        for error in [
            HttpClientError::Status(413),
            HttpClientError::StatusWithBody {
                status: 413,
                body: "private server details".into(),
            },
        ] {
            assert_eq!(
                super::desktop_download_error(AdpClientError::Http(error)),
                "archive_policy: package request rejected as too large (HTTP 413)"
            );
        }
        assert_eq!(
            super::desktop_download_error(AdpClientError::Http(
                HttpClientError::FilesystemFileSizeLimit("limit".into())
            )),
            "archive_policy: package exceeds the filesystem file-size limit: limit"
        );
        assert_eq!(
            super::desktop_download_error(AdpClientError::Http(HttpClientError::Request(
                "HTTP 413".into()
            ))),
            "download HTTP request failed: HTTP request failed: HTTP 413"
        );
        assert_eq!(
            super::desktop_download_error(AdpClientError::Http(
                HttpClientError::InsufficientStorage("disk full".into())
            )),
            "insufficient_storage: not enough storage to install the package: disk full"
        );
    }

    use super::*;

    use std::collections::{HashMap, VecDeque};
    use std::path::Path;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Mutex;

    use arcadestr_core::adp_storage::{
        DownloadToken, DownloadTokensRepository, InstallationId, InstallationProfile,
        InstallationRecord, InstallationsRepository as CoreInstallationsRepository,
        ProfileAvailability,
    };
    use arcadestr_core::distribution::{
        LaunchProfile, LaunchType, PackageFormat, PlatformTarget, ProfileId, RelativePath,
        Sha256Digest, WorkingDir,
    };
    use arcadestr_core::file_hash::sha256_file;
    use arcadestr_core::http_client::{HttpClientError, HttpDownloadOutcome, HttpRetainedDownload};
    use arcadestr_core::purchases::StoredReceipt;
    use arcadestr_core::storage::Database;
    use nostr::nips::nip19::ToBech32;
    use nostr::{Event, EventBuilder, Keys, Kind, Tag, TagKind, Timestamp};
    use tokio::sync::{Mutex as AsyncMutex, RwLock};

    #[derive(Clone, Default)]
    struct MockHttpClient {
        state: Arc<Mutex<MockHttpState>>,
    }

    include!("gate5_tests.rs");

    #[derive(Default)]
    struct MockHttpState {
        download_errors: HashMap<String, HttpClientError>,
        download_responses: HashMap<String, VecDeque<Vec<u8>>>,
        download_headers: HashMap<String, Vec<(String, String)>>,
        requested_urls: Vec<String>,
        call_counts: HashMap<String, usize>,
    }

    impl MockHttpClient {
        fn with_error(self, url: &str, error: HttpClientError) -> Self {
            self.state
                .lock()
                .expect("mock state")
                .download_errors
                .insert(url.to_string(), error);
            self
        }
        fn with_exact_bytes(self, url: &str, bytes: Vec<u8>) -> Self {
            self.state
                .lock()
                .expect("mock state")
                .download_responses
                .entry(url.to_string())
                .or_default()
                .push_back(bytes);
            self
        }
        fn new() -> Self {
            Self::default()
        }

        fn with_download_response(self, url: &str, _body: impl Into<Vec<u8>>) -> Self {
            self.state
                .lock()
                .expect("mock http state mutex poisoned")
                .download_responses
                .entry(url.to_string())
                .or_default()
                .push_back(install_fixture_bytes());
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

        async fn download_to_retained_file_with_preflight(
            &self,
            url: &str,
            headers: Vec<(String, String)>,
            candidate: &Path,
            on_preflight: &mut (dyn FnMut(Option<u64>) -> Result<(), HttpClientError> + Send),
            on_progress: &mut (dyn FnMut(u64, Option<u64>) + Send),
        ) -> Result<HttpRetainedDownload, HttpClientError> {
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
            let length = bytes.len() as u64;
            on_preflight(Some(length))?;
            let mut options = std::fs::OpenOptions::new();
            options.read(true).write(true).create_new(true);
            let mut file = options
                .open(candidate)
                .map_err(|error| HttpClientError::Request(error.to_string()))?;
            use std::io::{Seek, Write};
            file.write_all(&bytes)
                .map_err(|error| HttpClientError::Request(error.to_string()))?;
            file.flush()
                .map_err(|error| HttpClientError::Request(error.to_string()))?;
            file.seek(std::io::SeekFrom::Start(0))
                .map_err(|error| HttpClientError::Request(error.to_string()))?;
            on_progress(length, Some(length));
            Ok(HttpRetainedDownload {
                file: Arc::new(file),
                bytes_written: length,
            })
        }

        async fn download_to_open_retained_file_with_preflight(
            &self,
            url: &str,
            headers: Vec<(String, String)>,
            mut candidate: std::fs::File,
            on_preflight: &mut (dyn FnMut(Option<u64>) -> Result<(), HttpClientError> + Send),
            on_progress: &mut (dyn FnMut(u64, Option<u64>) + Send),
        ) -> Result<HttpRetainedDownload, HttpClientError> {
            let bytes = {
                let mut state = self.state.lock().expect("mock http state mutex poisoned");
                state.requested_urls.push(url.to_string());
                *state.call_counts.entry(url.to_string()).or_insert(0) += 1;
                state.download_headers.insert(url.to_string(), headers);
                if let Some(error) = state.download_errors.get(url) {
                    return Err(error.clone());
                }
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
            let length = bytes.len() as u64;
            on_preflight(Some(length))?;
            use std::io::{Seek, Write};
            candidate
                .write_all(&bytes)
                .map_err(|error| HttpClientError::Request(error.to_string()))?;
            candidate
                .flush()
                .map_err(|error| HttpClientError::Request(error.to_string()))?;
            candidate
                .seek(std::io::SeekFrom::Start(0))
                .map_err(|error| HttpClientError::Request(error.to_string()))?;
            on_progress(length, Some(length));
            Ok(HttpRetainedDownload {
                file: Arc::new(candidate),
                bytes_written: length,
            })
        }
    }

    fn install_fixture_path() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../tests/fixtures/a3-launch-profiles/generated/windows-nested.zip")
    }

    fn install_fixture_bytes() -> Vec<u8> {
        std::fs::read(install_fixture_path()).expect("generated install fixture should be readable")
    }

    fn install_fixture_digest() -> String {
        let sums = std::fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../tests/fixtures/a3-launch-profiles/SHA256SUMS"),
        )
        .expect("fixture SHA256SUMS should be readable");
        sums.lines()
            .find_map(|line| {
                let mut fields = line.split_whitespace();
                let digest = fields.next()?;
                (fields.next()? == "./windows-nested.zip").then(|| digest.to_string())
            })
            .expect("windows fixture digest should be listed")
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
            settlement_recovery: Default::default(),
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
        let repo = CoreInstallationsRepository::new(database.pool().clone());
        let (older, mut older_profiles) = installation_record("older-game", 100);
        let (newer, newer_profiles) = installation_record("newer-game", 200);
        older_profiles[0].availability = ProfileAvailability::Unavailable(
            arcadestr_core::adp_storage::ProfileUnavailableReason::ExecutableMissing,
        );
        repo.begin(&older, &older_profiles)
            .await
            .expect("older game should begin");
        repo.replace_manifest(
            &older.id,
            &[arcadestr_core::adp_storage::InstallationManifestEntry {
                path: RelativePath::parse("bin/game").expect("manifest path"),
                entry_type: arcadestr_core::adp_storage::ManifestEntryType::File,
                file_digest: Some(older.package_digest.clone()),
            }],
        )
        .await
        .expect("older manifest should record");
        repo.transition(&older.id, InstallationState::Ready, 100)
            .await
            .expect("older game should become ready");
        repo.begin(&newer, &newer_profiles)
            .await
            .expect("newer game should begin");
        repo.replace_manifest(
            &newer.id,
            &[arcadestr_core::adp_storage::InstallationManifestEntry {
                path: RelativePath::parse("bin/game").expect("manifest path"),
                entry_type: arcadestr_core::adp_storage::ManifestEntryType::File,
                file_digest: Some(newer.package_digest.clone()),
            }],
        )
        .await
        .expect("newer manifest should record");
        repo.transition(&newer.id, InstallationState::Ready, 200)
            .await
            .expect("newer game should become ready");

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
        assert_eq!(installed_games[0].installed_at, newer.created_at);
        assert_eq!(installed_games[0].package_format, "zip");
        assert!(installed_games[0].launchable);
        assert_eq!(installed_games[1].game_coordinate, older.game_coordinate);
        assert_eq!(installed_games[1].installed_at, older.created_at);
        assert!(!installed_games[1].launchable);
    }

    #[tokio::test]
    async fn add_to_library_rejects_an_account_switch_before_mutation() {
        let initiating_buyer = Keys::generate();
        let replacement_buyer = Keys::generate();
        let merchant = Keys::generate();
        let database = test_db("library-account-switch").await;
        let state =
            app_state_with_http(&initiating_buyer, database, Arc::new(MockHttpClient::new())).await;
        state
            .auth
            .lock()
            .await
            .connect_with_key(&replacement_buyer.secret_key().to_secret_hex())
            .expect("replacement buyer should authenticate");
        let app = tauri::test::mock_builder()
            .manage(state)
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .expect("mock Tauri app should build");
        let coordinate = coordinate(&merchant, "account-switch");

        let error = add_game_to_library(
            coordinate.clone(),
            initiating_buyer
                .public_key()
                .to_bech32()
                .expect("initiating npub"),
            app.state::<AppState>(),
        )
        .await
        .expect_err("replacement account must not receive the initiating account's mutation");

        assert!(error.contains("active account changed"));
        let library = LibraryGamesRepository::new(app.state::<AppState>().database.pool().clone());
        assert!(library
            .list(&initiating_buyer.public_key().to_hex())
            .await
            .expect("initiating library query")
            .is_empty());
        assert!(library
            .list(&replacement_buyer.public_key().to_hex())
            .await
            .expect("replacement library query")
            .is_empty());
    }

    fn installation_record(
        d_tag: &str,
        created_at: i64,
    ) -> (InstallationRecord, Vec<InstallationProfile>) {
        let keys = Keys::generate();
        let digest = "ab".repeat(32);
        let event = EventBuilder::new(Kind::Custom(30402), "listing")
            .tags([
                Tag::parse(["d", d_tag]).expect("d tag"),
                Tag::parse(["file_hash", &digest]).expect("file hash tag"),
                Tag::parse(["server", "https://dist.example.com"]).expect("server tag"),
                Tag::parse(["package", "zip"]).expect("package tag"),
                Tag::parse(["platform", "linux-x86_64"]).expect("platform tag"),
                Tag::parse(["launch", "play", "bin/game", ".", "[]", "game", "Play"])
                    .expect("launch tag"),
            ])
            .sign_with_keys(&keys)
            .expect("listing should sign");
        let profile = InstallationProfile {
            profile: LaunchProfile {
                id: ProfileId::parse("play").expect("profile id"),
                display_name: "Play".to_string(),
                executable: RelativePath::parse("bin/game").expect("executable"),
                working_dir: WorkingDir::parse(".").expect("working directory"),
                arguments: Vec::new(),
                launch_type: LaunchType::Game,
            },
            availability: ProfileAvailability::Available,
        };
        (
            InstallationRecord {
                id: InstallationId::generate(),
                game_coordinate: format!("30402:{}:{d_tag}", keys.public_key().to_hex()),
                installation_root: std::env::temp_dir().join(d_tag),
                package_digest: Sha256Digest::parse(&digest).expect("digest"),
                package_format: PackageFormat::Zip,
                resolved_target: PlatformTarget {
                    os: "linux".to_string(),
                    arch: "x86_64".to_string(),
                },
                raw_event_json: serde_json::to_string(&event).expect("event JSON"),
                publisher_default_profile_id: None,
                state: InstallationState::Installing,
                recovery_discriminator: None,
                retained_package_coordinate: None,
                created_at,
                updated_at: created_at,
            },
            vec![profile],
        )
    }

    async fn ready_lifecycle_fixture(
        prefix: &str,
        with_retained_package: bool,
    ) -> (
        Database,
        InstallationRecord,
        PathBuf,
        PathBuf,
        Option<RetainedPackage>,
    ) {
        let database = test_db(prefix).await;
        let repository = CoreInstallationsRepository::new(database.pool().clone());
        let managed = unique_test_path(&format!("{prefix}-installations"), "dir");
        let retained_directory = unique_test_path(&format!("{prefix}-retained"), "dir");
        std::fs::create_dir_all(&managed).expect("managed directory");
        std::fs::create_dir_all(&retained_directory).expect("retained directory");
        let (mut installation, profiles) = installation_record(prefix, 100);
        installation.installation_root = managed.join(installation.id.as_str());
        let stable_lock = install::OperationLock::acquire(&installation.installation_root)
            .expect("stable operation lock");
        drop(stable_lock);
        let retained_package = if with_retained_package {
            let package = RetainedPackage {
                game_coordinate: installation.game_coordinate.clone(),
                package_id: RetainedPackageId::generate(),
                package_path: retained_directory.join("package.bin"),
                package_digest: installation.package_digest.clone(),
                package_format: installation.package_format.clone(),
                reinstall: RetainedPackageReinstallSnapshot {
                    raw_event_json: installation.raw_event_json.clone(),
                    resolved_target: installation.resolved_target.clone(),
                    launch_profiles: profiles
                        .iter()
                        .map(|profile| profile.profile.clone())
                        .collect(),
                    publisher_default_profile_id: installation.publisher_default_profile_id.clone(),
                },
                size_bytes: 7,
                created_at: 100,
                updated_at: 100,
            };
            std::fs::write(&package.package_path, b"package").expect("retained package fixture");
            repository
                .replace_retained_package(&package)
                .await
                .expect("retained package row");
            installation.retained_package_coordinate = Some(package.game_coordinate.clone());
            Some(package)
        } else {
            None
        };
        repository
            .begin(&installation, &profiles)
            .await
            .expect("begin lifecycle fixture");
        std::fs::create_dir_all(installation.installation_root.join("bin"))
            .expect("installation fixture directory");
        std::fs::write(
            installation.installation_root.join("bin/game"),
            b"installed-game",
        )
        .expect("installation fixture file");
        repository
            .replace_manifest(
                &installation.id,
                &[arcadestr_core::adp_storage::InstallationManifestEntry {
                    path: RelativePath::parse("bin/game").expect("manifest path"),
                    entry_type: arcadestr_core::adp_storage::ManifestEntryType::File,
                    file_digest: Some(installation.package_digest.clone()),
                }],
            )
            .await
            .expect("fixture manifest");
        repository
            .transition(&installation.id, InstallationState::Ready, 101)
            .await
            .expect("fixture ready");
        (
            database,
            installation,
            managed,
            retained_directory,
            retained_package,
        )
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
        _file_hash: &str,
        version: &str,
    ) -> Event {
        EventBuilder::new(Kind::Custom(30402), "")
            .tags([
                Tag::custom(TagKind::d(), [listing_id]),
                Tag::custom(TagKind::custom("server"), [server_url]),
                Tag::custom(TagKind::custom("file_hash"), [install_fixture_digest()]),
                Tag::custom(TagKind::custom("version"), [version]),
                Tag::custom(TagKind::custom("package"), ["zip"]),
                Tag::custom(
                    TagKind::custom("platform"),
                    [format!(
                        "{}-{}",
                        std::env::consts::OS,
                        std::env::consts::ARCH
                    )],
                ),
                Tag::parse([
                    "launch",
                    "play",
                    "bin/Game.exe",
                    "bin",
                    "[]",
                    "game",
                    "Play",
                ])
                .expect("launch tag"),
            ])
            .custom_created_at(Timestamp::from(1_700_000_100))
            .sign_with_keys(merchant)
            .expect("listing event should sign")
    }

    fn listing_event_with_acquisition(
        merchant: &Keys,
        listing_id: &str,
        server_url: &str,
        _file_hash: &str,
        acquisition: &[&str],
    ) -> Event {
        let mut tags = vec![
            Tag::custom(TagKind::d(), [listing_id]),
            Tag::custom(TagKind::custom("server"), [server_url]),
            Tag::custom(TagKind::custom("file_hash"), [install_fixture_digest()]),
            Tag::custom(TagKind::custom("version"), ["1.0.0"]),
            Tag::custom(TagKind::custom("package"), ["zip"]),
            Tag::custom(
                TagKind::custom("platform"),
                [format!(
                    "{}-{}",
                    std::env::consts::OS,
                    std::env::consts::ARCH
                )],
            ),
            Tag::parse([
                "launch",
                "play",
                "bin/Game.exe",
                "bin",
                "[]",
                "game",
                "Play",
            ])
            .expect("launch tag"),
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

    fn retained_listing_event(merchant: &Keys, listing_id: &str, digest: &str) -> Event {
        EventBuilder::new(Kind::Custom(30402), "")
            .tags([
                Tag::custom(TagKind::d(), [listing_id]),
                Tag::custom(TagKind::custom("server"), ["https://dist.example.com"]),
                Tag::custom(TagKind::custom("file_hash"), [digest]),
                Tag::custom(TagKind::custom("version"), ["1.0.0"]),
                Tag::custom(TagKind::custom("package"), ["zip"]),
                Tag::custom(
                    TagKind::custom("platform"),
                    [format!(
                        "{}-{}",
                        std::env::consts::OS,
                        std::env::consts::ARCH
                    )],
                ),
                Tag::parse([
                    "launch",
                    "play",
                    "bin/Game.exe",
                    "bin",
                    "[]",
                    "game",
                    "Play",
                ])
                .expect("launch tag"),
                Tag::custom(TagKind::custom("acquisition"), ["public"]),
            ])
            .sign_with_keys(merchant)
            .expect("retained listing event")
    }

    fn retained_fixture(event: &Event, app_data: &Path) -> RetainedPackage {
        let resolved = resolve_install_plan(event, &host_platform()).expect("retained plan");
        let plan = resolved.plan;
        let coordinate = format!(
            "{}:{}:{}",
            plan.game_coordinate.kind,
            plan.game_coordinate.publisher_pubkey_hex,
            plan.game_coordinate.d_tag
        );
        let package_id = RetainedPackageId::generate();
        let package = RetainedPackage {
            game_coordinate: coordinate,
            package_id: package_id.clone(),
            package_path: app_data
                .join("retained-packages")
                .join(format!("{}.package", package_id.as_str())),
            package_digest: plan.artifact.digest,
            package_format: plan.artifact.package_format,
            reinstall: RetainedPackageReinstallSnapshot {
                raw_event_json: plan.source_event_json,
                resolved_target: host_platform(),
                launch_profiles: plan.launch_profiles,
                publisher_default_profile_id: plan.publisher_default_profile_id,
            },
            size_bytes: install_fixture_bytes().len() as u64,
            created_at: 1,
            updated_at: 1,
        };
        std::fs::create_dir_all(
            package
                .package_path
                .parent()
                .expect("retained package parent"),
        )
        .expect("retained package directory");
        std::fs::write(&package.package_path, install_fixture_bytes())
            .expect("retained fixture bytes");
        package
    }

    #[tokio::test]
    async fn retained_package_delete_removes_file_before_row_and_accepts_absence() {
        for remove_first in [false, true] {
            let app_data = unique_test_path("retained-delete-order", "dir");
            let database = test_db("retained-delete-order").await;
            let repository = InstallationsRepository::new(database.pool().clone());
            let merchant = Keys::generate();
            let event = retained_listing_event(
                &merchant,
                "retained-delete-order",
                &install_fixture_digest(),
            );
            let package = retained_fixture(&event, &app_data);
            repository
                .replace_retained_package(&package)
                .await
                .expect("persist retained package");
            if remove_first {
                std::fs::remove_file(&package.package_path).expect("remove retained fixture first");
            }

            delete_retained_package_owned(
                &repository,
                &app_data.join("retained-packages"),
                package.clone(),
            )
            .await
            .expect("delete retained package");

            assert!(!package.package_path.exists());
            assert!(repository
                .get_retained_package(&package.game_coordinate)
                .await
                .expect("query retained package")
                .is_none());
            let _ = std::fs::remove_dir_all(app_data);
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn retained_package_filesystem_failure_preserves_row_for_retry() {
        use std::os::unix::fs::symlink;

        let app_data = unique_test_path("retained-delete-retry", "dir");
        let database = test_db("retained-delete-retry").await;
        let repository = InstallationsRepository::new(database.pool().clone());
        let merchant = Keys::generate();
        let event = retained_listing_event(
            &merchant,
            "retained-delete-retry",
            &install_fixture_digest(),
        );
        let package = retained_fixture(&event, &app_data);
        repository
            .replace_retained_package(&package)
            .await
            .expect("persist retained package");
        let retained_directory = app_data.join("retained-packages");
        let outside = app_data.join("outside-retained-target");
        std::fs::write(&outside, b"must survive").expect("write outside target");
        std::fs::remove_file(&package.package_path).expect("remove retained fixture");
        symlink(&outside, &package.package_path).expect("replace package with symlink");

        let result =
            delete_retained_package_owned(&repository, &retained_directory, package.clone()).await;

        assert!(result.is_err());
        assert!(package.package_path.symlink_metadata().is_ok());
        assert_eq!(
            std::fs::read(&outside).expect("read outside target"),
            b"must survive"
        );
        assert_eq!(
            repository
                .get_retained_package(&package.game_coordinate)
                .await
                .expect("query retained package after filesystem failure"),
            Some(package.clone())
        );
        std::fs::remove_file(&package.package_path).expect("remove hostile symlink");
        let _ = std::fs::remove_dir_all(app_data);
    }

    #[tokio::test]
    async fn exact_retained_build_reinstalls_without_http_download() {
        let buyer = Keys::generate();
        let merchant = Keys::generate();
        let listing_id = "retained-reinstall";
        let coordinate = coordinate(&merchant, listing_id);
        let database = test_db("retained-reinstall").await;
        let http = MockHttpClient::new();
        let state = app_state_with_http(&buyer, database, Arc::new(http.clone())).await;
        let app_data = unique_test_path("retained-reinstall-data", "dir");
        let retained_event =
            retained_listing_event(&merchant, listing_id, &install_fixture_digest());
        let package = retained_fixture(&retained_event, &app_data);
        CoreInstallationsRepository::new(state.database.pool().clone())
            .replace_retained_package(&package)
            .await
            .expect("persist retained fixture");
        let repository = CoreInstallationsRepository::new(state.database.pool().clone());
        let failed_id = InstallationId::generate();
        let failed_root = app_data.join("installations").join(failed_id.as_str());
        let failed_lock = install::OperationLock::acquire(&failed_root).expect("failed root lock");
        drop(failed_lock);
        let failed_profiles = package
            .reinstall
            .launch_profiles
            .iter()
            .cloned()
            .map(|profile| InstallationProfile {
                profile,
                availability: ProfileAvailability::Unavailable(
                    ProfileUnavailableReason::ExecutableMissing,
                ),
            })
            .collect::<Vec<_>>();
        repository
            .begin(
                &InstallationRecord {
                    id: failed_id.clone(),
                    game_coordinate: coordinate.clone(),
                    installation_root: failed_root,
                    package_digest: package.package_digest.clone(),
                    package_format: package.package_format.clone(),
                    resolved_target: package.reinstall.resolved_target.clone(),
                    raw_event_json: package.reinstall.raw_event_json.clone(),
                    publisher_default_profile_id: package
                        .reinstall
                        .publisher_default_profile_id
                        .clone(),
                    state: InstallationState::Installing,
                    recovery_discriminator: None,
                    retained_package_coordinate: None,
                    created_at: 1,
                    updated_at: 1,
                },
                &failed_profiles,
            )
            .await
            .expect("seed failed installation");
        repository
            .mark_failed_for_recovery_if_unchanged(
                &failed_id,
                1,
                InstallationRecoveryDiscriminator::FinalRootAbsent,
                2,
            )
            .await
            .expect("mark failed installation");
        let fetcher = StaticFreshListingFetcher {
            event: retained_event,
            calls: Arc::new(AtomicUsize::new(0)),
        };

        install_game_with_fetcher(
            app_listing(&merchant, listing_id),
            &state,
            None::<&tauri::AppHandle<tauri::test::MockRuntime>>,
            app_data,
            &fetcher,
        )
        .await
        .expect("exact retained build should reinstall");

        assert!(http.last_requested_url().is_none());
        let installed = CoreInstallationsRepository::new(state.database.pool().clone())
            .get_by_coordinate(&coordinate)
            .await
            .expect("installation query")
            .expect("ready retained installation");
        assert_eq!(installed.installation.state, InstallationState::Ready);
        assert_ne!(installed.installation.id, failed_id);
        assert_eq!(
            installed.installation.retained_package_coordinate,
            Some(coordinate)
        );
    }

    #[tokio::test]
    async fn mismatched_retained_build_without_ready_installation_requires_explicit_removal() {
        let buyer = Keys::generate();
        let merchant = Keys::generate();
        let listing_id = "retained-mismatch";
        let server_url = "https://dist.example.com";
        let coordinate = coordinate(&merchant, listing_id);
        let database = test_db("retained-mismatch").await;
        let http = MockHttpClient::new();
        let state = app_state_with_http(&buyer, database, Arc::new(http.clone())).await;
        let app_data = unique_test_path("retained-mismatch-data", "dir");
        let old_event = retained_listing_event(&merchant, listing_id, &"cd".repeat(32));
        let package = retained_fixture(&old_event, &app_data);
        let original_bytes = std::fs::read(&package.package_path).expect("old package bytes");
        CoreInstallationsRepository::new(state.database.pool().clone())
            .replace_retained_package(&package)
            .await
            .expect("persist old retained build");
        let repository = CoreInstallationsRepository::new(state.database.pool().clone());
        let failed_id = InstallationId::generate();
        let failed_root = app_data.join("installations").join(failed_id.as_str());
        let failed_lock = install::OperationLock::acquire(&failed_root).expect("failed root lock");
        drop(failed_lock);
        let failed_profiles = package
            .reinstall
            .launch_profiles
            .iter()
            .cloned()
            .map(|profile| InstallationProfile {
                profile,
                availability: ProfileAvailability::Unavailable(
                    ProfileUnavailableReason::ExecutableMissing,
                ),
            })
            .collect::<Vec<_>>();
        repository
            .begin(
                &InstallationRecord {
                    id: failed_id.clone(),
                    game_coordinate: coordinate.clone(),
                    installation_root: failed_root,
                    package_digest: package.package_digest.clone(),
                    package_format: package.package_format.clone(),
                    resolved_target: package.reinstall.resolved_target.clone(),
                    raw_event_json: package.reinstall.raw_event_json.clone(),
                    publisher_default_profile_id: package
                        .reinstall
                        .publisher_default_profile_id
                        .clone(),
                    state: InstallationState::Installing,
                    recovery_discriminator: None,
                    retained_package_coordinate: None,
                    created_at: 1,
                    updated_at: 1,
                },
                &failed_profiles,
            )
            .await
            .expect("seed failed installation");
        repository
            .mark_failed_for_recovery_if_unchanged(
                &failed_id,
                1,
                InstallationRecoveryDiscriminator::FinalRootAbsent,
                2,
            )
            .await
            .expect("mark failed installation");
        let fetcher = StaticFreshListingFetcher {
            event: listing_event_with_acquisition(
                &merchant,
                listing_id,
                server_url,
                &install_fixture_digest(),
                &["public"],
            ),
            calls: Arc::new(AtomicUsize::new(0)),
        };

        let error = install_game_with_fetcher(
            app_listing(&merchant, listing_id),
            &state,
            None::<&tauri::AppHandle<tauri::test::MockRuntime>>,
            app_data,
            &fetcher,
        )
        .await
        .expect_err("different retained revision must be deferred");

        assert!(error.contains("remove the retained older build"));
        assert!(http.last_requested_url().is_none());
        assert!(repository
            .get_by_id(&failed_id)
            .await
            .expect("failed installation query")
            .is_some());
        assert_eq!(
            std::fs::read(&package.package_path).expect("old package survives"),
            original_bytes
        );
        assert_eq!(
            CoreInstallationsRepository::new(state.database.pool().clone())
                .get_retained_package(&coordinate)
                .await
                .expect("retained query"),
            Some(package)
        );
    }

    #[cfg(unix)]
    #[test]
    fn retained_open_rejects_intermediate_symlink_escape() {
        use std::os::unix::fs::symlink;

        let app_data = unique_test_path("retained-capability-root", "dir");
        let outside = unique_test_path("retained-capability-outside", "dir");
        std::fs::create_dir_all(&app_data).expect("app data directory");
        std::fs::create_dir_all(&outside).expect("outside directory");
        std::fs::write(outside.join("package.zip"), b"outside").expect("outside package");
        symlink(&outside, app_data.join("retained-packages")).expect("intermediate symlink");

        let error = open_existing_retained_regular_file(
            &app_data,
            &app_data.join("retained-packages/package.zip"),
        )
        .expect_err("capability open must reject an escape through an intermediate symlink");

        assert!(error.contains("could not be inspected") || error.contains("could not be opened"));
        let _ = std::fs::remove_dir_all(&app_data);
        let _ = std::fs::remove_dir_all(&outside);
    }

    #[tokio::test]
    async fn offline_retained_reinstall_is_ready_without_listing_auth_relay_or_http() {
        let buyer = Keys::generate();
        let merchant = Keys::generate();
        let listing_id = "offline-retained-reinstall";
        let coordinate = coordinate(&merchant, listing_id);
        let database = test_db("offline-retained-reinstall").await;
        let repository = CoreInstallationsRepository::new(database.pool().clone());
        let http = MockHttpClient::new();
        let state = app_state_with_http(&buyer, database, Arc::new(http.clone())).await;
        let app = tauri::test::mock_builder()
            .manage(state)
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .expect("mock app");
        let app_data = app.path().app_local_data_dir().expect("mock app data");
        let event = retained_listing_event(&merchant, listing_id, &install_fixture_digest());
        let package = retained_fixture(&event, &app_data);
        repository
            .replace_retained_package(&package)
            .await
            .expect("persist offline retained package");

        // A newer network listing is intentionally not supplied: this API has
        // no listing fetcher, signer, authorization, relay, or HTTP argument.
        reinstall_retained_package_with_id(
            coordinate.clone(),
            package.package_id.as_str().to_string(),
            app.handle().clone(),
            InstallationId::generate(),
        )
        .await
        .expect("offline reinstall");

        assert!(http.last_requested_url().is_none());
        let snapshot = repository
            .get_by_coordinate(&coordinate)
            .await
            .expect("installation query")
            .expect("ready reinstall");
        assert_eq!(snapshot.installation.state, InstallationState::Ready);
        assert_eq!(
            snapshot
                .profiles
                .iter()
                .map(|profile| profile.profile.clone())
                .collect::<Vec<_>>(),
            package.reinstall.launch_profiles
        );
        assert!(snapshot
            .profiles
            .iter()
            .all(|profile| profile.availability == ProfileAvailability::Available));
        assert!(snapshot
            .manifest
            .iter()
            .any(|entry| entry.path.as_str() == "bin/Game.exe"));
        uninstall_game_owned(
            &repository,
            &app_data.join("installations"),
            &app_data.join("retained-packages"),
            &snapshot.installation.id,
            true,
            2,
        )
        .await
        .expect("uninstall local reinstall while retaining package");
        assert_eq!(
            repository
                .get_retained_package(&coordinate)
                .await
                .expect("retained snapshot after uninstall"),
            Some(package)
        );
    }

    #[tokio::test]
    async fn offline_reinstall_missing_package_preserves_retained_row_and_marks_failed() {
        let buyer = Keys::generate();
        let merchant = Keys::generate();
        let listing_id = "offline-retained-missing";
        let coordinate = coordinate(&merchant, listing_id);
        let database = test_db("offline-retained-missing").await;
        let repository = CoreInstallationsRepository::new(database.pool().clone());
        let state = app_state_with_http(&buyer, database, Arc::new(MockHttpClient::new())).await;
        let app = tauri::test::mock_builder()
            .manage(state)
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .expect("mock app");
        let app_data = app.path().app_local_data_dir().expect("mock app data");
        let event = retained_listing_event(&merchant, listing_id, &install_fixture_digest());
        let package = retained_fixture(&event, &app_data);
        repository
            .replace_retained_package(&package)
            .await
            .expect("persist retained package");
        std::fs::remove_file(&package.package_path).expect("remove retained file fixture");

        let error = reinstall_retained_package_with_id(
            coordinate.clone(),
            package.package_id.as_str().to_string(),
            app.handle().clone(),
            InstallationId::generate(),
        )
        .await
        .expect_err("missing retained file must fail");
        assert!(error.contains("absent"));
        assert_eq!(
            repository
                .get_by_coordinate(&coordinate)
                .await
                .expect("installation query")
                .expect("failed reinstall")
                .installation
                .state,
            InstallationState::Failed
        );
        assert_eq!(
            repository
                .get_retained_package(&coordinate)
                .await
                .expect("retained query"),
            Some(package)
        );
    }

    #[tokio::test]
    async fn offline_reinstall_live_lock_prevents_begin() {
        let buyer = Keys::generate();
        let merchant = Keys::generate();
        let listing_id = "offline-retained-live-lock";
        let coordinate = coordinate(&merchant, listing_id);
        let database = test_db("offline-retained-live-lock").await;
        let repository = CoreInstallationsRepository::new(database.pool().clone());
        let state = app_state_with_http(&buyer, database, Arc::new(MockHttpClient::new())).await;
        let app = tauri::test::mock_builder()
            .manage(state)
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .expect("mock app");
        let app_data = app.path().app_local_data_dir().expect("mock app data");
        let event = retained_listing_event(&merchant, listing_id, &install_fixture_digest());
        let package = retained_fixture(&event, &app_data);
        repository
            .replace_retained_package(&package)
            .await
            .expect("persist retained package");
        let id = InstallationId::generate();
        let root = app_data.join("installations").join(id.as_str());
        let live_lock = install::OperationLock::acquire(&root).expect("live lock");

        reinstall_retained_package_with_id(
            coordinate.clone(),
            package.package_id.as_str().to_string(),
            app.handle().clone(),
            id,
        )
        .await
        .expect_err("live lock must prevent reinstall begin");
        assert!(repository
            .get_by_coordinate(&coordinate)
            .await
            .expect("installation query")
            .is_none());
        drop(live_lock);
    }

    #[tokio::test]
    async fn offline_reinstall_rejects_stale_package_identity_before_begin() {
        let buyer = Keys::generate();
        let merchant = Keys::generate();
        let listing_id = "offline-retained-stale-package";
        let coordinate = coordinate(&merchant, listing_id);
        let database = test_db("offline-retained-stale-package").await;
        let repository = CoreInstallationsRepository::new(database.pool().clone());
        let state = app_state_with_http(&buyer, database, Arc::new(MockHttpClient::new())).await;
        let app = tauri::test::mock_builder()
            .manage(state)
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .expect("mock app");
        let app_data = app.path().app_local_data_dir().expect("mock app data");
        let event = retained_listing_event(&merchant, listing_id, &install_fixture_digest());
        let package = retained_fixture(&event, &app_data);
        repository
            .replace_retained_package(&package)
            .await
            .expect("persist retained package");

        let error = reinstall_retained_package_with_id(
            coordinate.clone(),
            RetainedPackageId::generate().as_str().to_string(),
            app.handle().clone(),
            InstallationId::generate(),
        )
        .await
        .expect_err("stale package identity must be rejected");

        assert!(error.contains("changed"));
        assert!(repository
            .get_by_coordinate(&coordinate)
            .await
            .expect("installation query")
            .is_none());
    }

    #[tokio::test]
    async fn offline_reinstall_corrupt_or_hazardous_package_preserves_evidence_and_fails_row() {
        for hazardous in [false, true] {
            let case = if hazardous { "hazardous" } else { "corrupt" };
            let buyer = Keys::generate();
            let merchant = Keys::generate();
            let listing_id = format!("offline-retained-{case}");
            let coordinate = coordinate(&merchant, &listing_id);
            let bytes = if hazardous {
                std::fs::read(
                    Path::new(env!("CARGO_MANIFEST_DIR"))
                        .join("../tests/fixtures/a3-launch-profiles/generated/traversal.zip"),
                )
                .expect("hazard fixture")
            } else {
                install_fixture_bytes()
            };
            let declared_digest = sha256_hex(&bytes).await;
            let database = test_db(&format!("offline-retained-{case}")).await;
            let repository = CoreInstallationsRepository::new(database.pool().clone());
            let state =
                app_state_with_http(&buyer, database, Arc::new(MockHttpClient::new())).await;
            let app = tauri::test::mock_builder()
                .manage(state)
                .build(tauri::test::mock_context(tauri::test::noop_assets()))
                .expect("mock app");
            let app_data = app.path().app_local_data_dir().expect("mock app data");
            let event = retained_listing_event(&merchant, &listing_id, &declared_digest);
            let package = retained_fixture(&event, &app_data);
            let stored_bytes = if hazardous {
                bytes
            } else {
                b"corrupt retained package".to_vec()
            };
            std::fs::write(&package.package_path, &stored_bytes).expect("failure fixture bytes");
            repository
                .replace_retained_package(&package)
                .await
                .expect("persist retained failure fixture");

            reinstall_retained_package_with_id(
                coordinate.clone(),
                package.package_id.as_str().to_string(),
                app.handle().clone(),
                InstallationId::generate(),
            )
            .await
            .expect_err("invalid retained package must fail");

            assert_eq!(
                std::fs::read(&package.package_path).expect("retained evidence survives"),
                stored_bytes
            );
            assert_eq!(
                repository
                    .get_by_coordinate(&coordinate)
                    .await
                    .expect("installation query")
                    .expect("failed reinstall row")
                    .installation
                    .state,
                InstallationState::Failed
            );
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn offline_reinstall_symlink_package_is_not_followed_and_marks_failed() {
        use std::os::unix::fs::symlink;

        let buyer = Keys::generate();
        let merchant = Keys::generate();
        let listing_id = "offline-retained-symlink";
        let coordinate = coordinate(&merchant, listing_id);
        let database = test_db("offline-retained-symlink").await;
        let repository = CoreInstallationsRepository::new(database.pool().clone());
        let state = app_state_with_http(&buyer, database, Arc::new(MockHttpClient::new())).await;
        let app = tauri::test::mock_builder()
            .manage(state)
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .expect("mock app");
        let app_data = app.path().app_local_data_dir().expect("mock app data");
        let event = retained_listing_event(&merchant, listing_id, &install_fixture_digest());
        let package = retained_fixture(&event, &app_data);
        repository
            .replace_retained_package(&package)
            .await
            .expect("persist retained package");
        let target = app_data.join("retained-symlink-target.zip");
        std::fs::write(&target, install_fixture_bytes()).expect("symlink target");
        std::fs::remove_file(&package.package_path).expect("remove regular package fixture");
        symlink(&target, &package.package_path).expect("retained symlink fixture");

        reinstall_retained_package_with_id(
            coordinate.clone(),
            package.package_id.as_str().to_string(),
            app.handle().clone(),
            InstallationId::generate(),
        )
        .await
        .expect_err("retained symlink must fail");

        assert!(std::fs::symlink_metadata(&package.package_path)
            .expect("symlink survives")
            .file_type()
            .is_symlink());
        assert_eq!(
            repository
                .get_by_coordinate(&coordinate)
                .await
                .expect("installation query")
                .expect("failed reinstall row")
                .installation
                .state,
            InstallationState::Failed
        );
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
    async fn install_game_ratio_fixture_returns_typed_rejection_without_installing() {
        let buyer = Keys::generate();
        let merchant = Keys::generate();
        let listing_id = "ratio-rejected";
        let server_url = "https://dist.example.com";
        let coordinate = coordinate(&merchant, listing_id);
        let download_url = format!("{server_url}/game/{}", urlencoding::encode(&coordinate));
        let bytes = std::fs::read(install_fixture_path().with_file_name("ratio-bomb.zip"))
            .expect("ratio fixture");
        let digest = sha256_hex(&bytes).await;
        let http = MockHttpClient::new();
        http.state
            .lock()
            .expect("mock state")
            .download_responses
            .entry(download_url.clone())
            .or_default()
            .push_back(bytes);
        let state = app_state_with_http(
            &buyer,
            test_db("install-ratio-rejected").await,
            Arc::new(http.clone()),
        )
        .await;
        grant_ownership(&state, &buyer, &merchant, listing_id).await;
        let template = listing_event(&merchant, listing_id, server_url, &digest, "1.0.0");
        let tags = template
            .tags
            .iter()
            .filter(|tag| tag.as_slice()[0] != "file_hash")
            .cloned()
            .chain([Tag::custom(TagKind::custom("file_hash"), [digest])]);
        let fetcher = StaticFreshListingFetcher {
            event: EventBuilder::new(Kind::Custom(30402), "")
                .tags(tags)
                .sign_with_keys(&merchant)
                .expect("ratio listing"),
            calls: Arc::new(AtomicUsize::new(0)),
        };
        let app_data = unique_test_path("install-ratio-rejected-data", "dir");
        let error = install_game_with_fetcher_and_signer(
            app_listing(&merchant, listing_id),
            &state,
            None,
            None::<&tauri::AppHandle<tauri::test::MockRuntime>>,
            app_data.clone(),
            &fetcher,
        )
        .await
        .expect_err("ratio bomb must be rejected");
        assert_eq!(error, InstallCommandError::CompressionRatioExceeded);
        assert_eq!(
            serde_json::to_value(error).expect("command error JSON"),
            serde_json::json!({"code": "compression_ratio_exceeded"})
        );
        assert_eq!(http.call_count(&download_url), 1);
        let repository = CoreInstallationsRepository::new(state.database.pool().clone());
        let snapshot = repository
            .get_by_coordinate(&coordinate)
            .await
            .expect("installation query")
            .expect("failed installation record");
        assert_eq!(snapshot.installation.state, InstallationState::Failed);
        assert!(!snapshot.installation.installation_root.exists());
        assert!(snapshot.manifest.is_empty());
        assert!(repository
            .get_retained_package(&coordinate)
            .await
            .expect("retained query")
            .is_none());
        assert_eq!(
            std::fs::read_dir(app_data.join("retained-packages"))
                .expect("retained directory")
                .count(),
            0
        );
        std::fs::remove_dir_all(app_data).expect("remove test installation data");
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

        let app_data = unique_test_path("install-game-token-data", "dir");
        let started = std::time::Instant::now();
        install_game_with_fetcher(
            app_listing(&merchant, listing_id),
            &state,
            None::<&tauri::AppHandle<tauri::test::MockRuntime>>,
            app_data.clone(),
            &fetcher,
        )
        .await
        .expect("token path install should succeed");
        let elapsed = started.elapsed();

        assert_eq!(http.call_count(&download_url), 1);
        let snapshot = CoreInstallationsRepository::new(state.database.pool().clone())
            .get_by_coordinate(&coordinate)
            .await
            .expect("completed installation should be queryable")
            .expect("completed installation should remain recorded");
        assert_eq!(snapshot.installation.state, InstallationState::Ready);
        assert_eq!(
            snapshot.installation.package_digest.as_str(),
            install_fixture_digest()
        );
        assert_eq!(snapshot.profiles.len(), 1);
        assert!(snapshot.profiles[0].availability == ProfileAvailability::Available);
        assert!(!snapshot.manifest.is_empty());
        assert_eq!(
            snapshot.installation.retained_package_coordinate.as_deref(),
            Some(coordinate.as_str())
        );
        eprintln!("A4.4 token-authenticated fixture install: {elapsed:?}");
        std::fs::remove_dir_all(app_data).expect("fixture install data should be removable");
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

    #[test]
    fn extraction_storage_preflight_rejects_below_declared_size_plus_margin() {
        let declared = 4096;
        let required = declared + INSTALL_STORAGE_MARGIN;
        let error = extraction_storage_preflight(Some(required - 1), declared)
            .expect_err("one byte below required capacity must be rejected");
        assert!(matches!(error, HttpClientError::InsufficientStorage(_)));
    }

    #[test]
    fn installed_game_package_format_dto_is_stable_snake_case() {
        assert_eq!(package_format_dto(&PackageFormat::Zip), "zip");
        assert_eq!(package_format_dto(&PackageFormat::TarZstd), "tar_zstd");
        assert_eq!(package_format_dto(&PackageFormat::TarGz), "tar_gz");
    }

    #[test]
    fn extraction_storage_preflight_accepts_exact_declared_size_plus_margin() {
        let declared = 4096;
        let required = declared + INSTALL_STORAGE_MARGIN;
        extraction_storage_preflight(Some(required), declared)
            .expect("exact required capacity should pass the point-in-time preflight");
    }

    #[tokio::test]
    async fn install_signer_get_public_key_rechecks_current_active_account() {
        let expected_keys = Keys::generate();
        let replacement_keys = Keys::generate();
        let mut auth = AuthState::new();
        auth.connect_with_key(&expected_keys.secret_key().to_secret_hex())
            .expect("expected account connects");
        let inner =
            Arc::new(auth.signer().cloned().expect("expected signer")) as Arc<dyn NostrSigner>;
        let auth = Arc::new(AsyncMutex::new(auth));
        let signer = InstallAccountRecheckingSigner {
            inner,
            expected: expected_keys.public_key(),
            auth: Arc::clone(&auth),
            signer_state: Arc::new(AsyncMutex::new(AppSignerState::new())),
        };
        assert_eq!(
            signer.get_public_key().await.expect("expected account"),
            expected_keys.public_key()
        );
        auth.lock()
            .await
            .connect_with_key(&replacement_keys.secret_key().to_secret_hex())
            .expect("replacement account connects");

        assert!(signer.get_public_key().await.is_err());
    }

    #[cfg(unix)]
    #[test]
    fn installed_size_does_not_follow_directory_symlinks() {
        use std::os::unix::fs::symlink;

        let root = unique_test_path("install-size-root", "dir");
        let outside = unique_test_path("install-size-outside", "dir");
        std::fs::create_dir_all(&root).expect("size root");
        std::fs::create_dir_all(&outside).expect("size outside");
        std::fs::write(root.join("local.bin"), vec![0_u8; 11]).expect("local size file");
        std::fs::write(outside.join("large.bin"), vec![0_u8; 1024 * 1024])
            .expect("outside size file");
        symlink(&outside, root.join("outside-link")).expect("directory symlink");

        let measured = filesystem_object_size_no_follow(&root).expect("size walk");
        assert!(measured >= 11);
        assert!(measured < 1024 * 1024, "outside tree must not be followed");
    }

    #[tokio::test]
    async fn uninstall_sizes_reports_installation_and_retained_package_on_demand() {
        let (database, installation, managed, _retained, package) =
            ready_lifecycle_fixture("uninstall-sizes", true).await;
        let repository = CoreInstallationsRepository::new(database.pool().clone());

        let sizes = get_uninstall_sizes_owned(&repository, &managed, &installation.id)
            .await
            .expect("uninstall sizes");

        assert_eq!(
            sizes.installation_size_bytes,
            b"installed-game".len() as u64
        );
        assert_eq!(
            sizes.retained_package_size_bytes,
            package.map(|package| package.size_bytes)
        );
    }

    #[tokio::test]
    async fn uninstall_retains_or_deletes_package_only_as_explicitly_requested() {
        for retain_package in [true, false] {
            let prefix = if retain_package {
                "uninstall-retain-package"
            } else {
                "uninstall-delete-package"
            };
            let (database, installation, managed, retained_directory, package) =
                ready_lifecycle_fixture(prefix, true).await;
            let repository = CoreInstallationsRepository::new(database.pool().clone());
            let package = package.expect("retained fixture");

            uninstall_game_owned(
                &repository,
                &managed,
                &retained_directory,
                &installation.id,
                retain_package,
                102,
            )
            .await
            .expect("uninstall succeeds");

            assert!(!installation.installation_root.exists());
            assert!(repository
                .get_by_id(&installation.id)
                .await
                .expect("installation query")
                .is_none());
            assert_eq!(package.package_path.exists(), retain_package);
            assert_eq!(
                repository
                    .get_retained_package(&package.game_coordinate)
                    .await
                    .expect("retained query")
                    .is_some(),
                retain_package
            );
        }
    }

    #[tokio::test]
    async fn uninstall_accepts_already_absent_root_but_rejects_untrusted_root() {
        let (database, installation, managed, retained_directory, _package) =
            ready_lifecycle_fixture("uninstall-absent", false).await;
        let repository = CoreInstallationsRepository::new(database.pool().clone());
        std::fs::remove_dir_all(&installation.installation_root).expect("remove root fixture");
        uninstall_game_owned(
            &repository,
            &managed,
            &retained_directory,
            &installation.id,
            true,
            102,
        )
        .await
        .expect("already absent ready root is a successful local removal");
        assert!(repository
            .get_by_id(&installation.id)
            .await
            .expect("installation query")
            .is_none());

        let (database, installation, managed, retained_directory, _package) =
            ready_lifecycle_fixture("uninstall-untrusted", false).await;
        let repository = CoreInstallationsRepository::new(database.pool().clone());
        let outside = unique_test_path("uninstall-untrusted-outside", "dir");
        std::fs::create_dir_all(&outside).expect("outside root");
        let marker = outside.join("preserve");
        std::fs::write(&marker, b"preserve").expect("outside marker");
        let outside_text = outside.to_string_lossy().into_owned();
        sqlx::query("UPDATE installations SET installation_root = ? WHERE id = ?")
            .bind(outside_text)
            .bind(installation.id.as_str())
            .execute(database.pool())
            .await
            .expect("alter root fixture");

        let error = uninstall_game_owned(
            &repository,
            &managed,
            &retained_directory,
            &installation.id,
            true,
            102,
        )
        .await
        .expect_err("untrusted root must be rejected");
        assert!(error.contains("direct child"));
        assert!(installation.installation_root.exists());
        assert_eq!(std::fs::read(marker).expect("marker survives"), b"preserve");
    }

    #[tokio::test]
    async fn resume_uninstall_removes_present_or_absent_root_and_retains_package() {
        for root_present in [true, false] {
            let prefix = if root_present {
                "resume-uninstall-present"
            } else {
                "resume-uninstall-absent"
            };
            let (database, installation, managed, _retained_directory, package) =
                ready_lifecycle_fixture(prefix, true).await;
            let repository = CoreInstallationsRepository::new(database.pool().clone());
            repository
                .transition(&installation.id, InstallationState::Uninstalling, 102)
                .await
                .expect("persist interrupted uninstall");
            if !root_present {
                std::fs::remove_dir_all(&installation.installation_root)
                    .expect("remove interrupted root fixture");
            }

            resume_uninstall_owned(&repository, &managed, &installation.id)
                .await
                .expect("resume uninstall");

            assert!(!installation.installation_root.exists());
            assert!(repository
                .get_by_id(&installation.id)
                .await
                .expect("installation query")
                .is_none());
            let package = package.expect("retained package fixture");
            assert!(package.package_path.exists());
            assert!(repository
                .get_retained_package(&package.game_coordinate)
                .await
                .expect("retained query")
                .is_some());
        }
    }

    #[tokio::test]
    async fn resume_uninstall_defers_live_lock_and_rejects_altered_root() {
        let (database, installation, managed, _retained_directory, _package) =
            ready_lifecycle_fixture("resume-uninstall-live", false).await;
        let repository = CoreInstallationsRepository::new(database.pool().clone());
        repository
            .transition(&installation.id, InstallationState::Uninstalling, 102)
            .await
            .expect("persist interrupted uninstall");
        let live_lock =
            install::OperationLock::acquire_existing_for_recovery(&installation.installation_root)
                .expect("live operation lock");

        let error = resume_uninstall_owned(&repository, &managed, &installation.id)
            .await
            .expect_err("live lock must defer uninstall resumption");
        assert!(error.contains("another installation operation"));
        assert_eq!(
            repository
                .get_by_id(&installation.id)
                .await
                .expect("installation query")
                .expect("uninstalling row")
                .installation
                .state,
            InstallationState::Uninstalling
        );
        assert!(installation.installation_root.exists());
        drop(live_lock);

        let (database, installation, managed, _retained_directory, _package) =
            ready_lifecycle_fixture("resume-uninstall-altered", false).await;
        let repository = CoreInstallationsRepository::new(database.pool().clone());
        repository
            .transition(&installation.id, InstallationState::Uninstalling, 102)
            .await
            .expect("persist interrupted uninstall");
        let outside = unique_test_path("resume-uninstall-altered-outside", "dir");
        std::fs::create_dir_all(&outside).expect("outside root");
        let marker = outside.join("preserve");
        std::fs::write(&marker, b"preserve").expect("outside marker");
        let outside_text = outside.to_string_lossy().into_owned();
        sqlx::query("UPDATE installations SET installation_root = ? WHERE id = ?")
            .bind(outside_text)
            .bind(installation.id.as_str())
            .execute(database.pool())
            .await
            .expect("alter uninstalling root");

        let error = resume_uninstall_owned(&repository, &managed, &installation.id)
            .await
            .expect_err("altered root must be rejected");
        assert!(error.contains("direct child"));
        assert!(installation.installation_root.exists());
        assert_eq!(
            std::fs::read(marker).expect("outside marker survives"),
            b"preserve"
        );
    }

    #[tokio::test]
    async fn remove_failed_deletes_exact_root_and_row_but_preserves_retained_package() {
        let (database, installation, managed, _retained_directory, package) =
            ready_lifecycle_fixture("remove-failed", true).await;
        let repository = CoreInstallationsRepository::new(database.pool().clone());
        sqlx::query(
            "UPDATE installations SET state = 'failed', \
             recovery_discriminator = 'final_root_present' WHERE id = ?",
        )
        .bind(installation.id.as_str())
        .execute(database.pool())
        .await
        .expect("failed fixture transition");

        remove_failed_installation_owned(&repository, &managed, &installation.id)
            .await
            .expect("failed installation removal");

        assert!(!installation.installation_root.exists());
        assert!(repository
            .get_by_id(&installation.id)
            .await
            .expect("installation query")
            .is_none());
        let package = package.expect("retained package fixture");
        assert!(package.package_path.exists());
        assert!(repository
            .get_retained_package(&package.game_coordinate)
            .await
            .expect("retained query")
            .is_some());
    }

    #[tokio::test]
    async fn remove_failed_absent_discriminator_confirms_absence_then_deletes_row() {
        let (database, installation, managed, _retained_directory, _package) =
            ready_lifecycle_fixture("remove-failed-absent", false).await;
        let repository = CoreInstallationsRepository::new(database.pool().clone());
        std::fs::remove_dir_all(&installation.installation_root)
            .expect("remove failed root fixture");
        sqlx::query(
            "UPDATE installations SET state = 'failed', \
             recovery_discriminator = 'final_root_absent' WHERE id = ?",
        )
        .bind(installation.id.as_str())
        .execute(database.pool())
        .await
        .expect("absent failed fixture transition");

        remove_failed_installation_owned(&repository, &managed, &installation.id)
            .await
            .expect("absent failed installation removal");
        assert!(repository
            .get_by_id(&installation.id)
            .await
            .expect("installation query")
            .is_none());
    }

    #[tokio::test]
    async fn remove_failed_absent_discriminator_preserves_late_root() {
        let (database, installation, managed, _retained_directory, _package) =
            ready_lifecycle_fixture("remove-failed-late-root", false).await;
        let repository = CoreInstallationsRepository::new(database.pool().clone());
        std::fs::remove_dir_all(&installation.installation_root)
            .expect("remove original failed root");
        sqlx::query(
            "UPDATE installations SET state = 'failed', \
             recovery_discriminator = 'final_root_absent' WHERE id = ?",
        )
        .bind(installation.id.as_str())
        .execute(database.pool())
        .await
        .expect("absent failed fixture transition");
        std::fs::create_dir(&installation.installation_root).expect("late root");
        let marker = installation.installation_root.join("preserve");
        std::fs::write(&marker, b"unowned").expect("late root marker");

        let error = remove_failed_installation_owned(&repository, &managed, &installation.id)
            .await
            .expect_err("late root must block failed cleanup");
        assert!(error.contains("appeared after recovery"));
        assert_eq!(
            std::fs::read(marker).expect("late root preserved"),
            b"unowned"
        );
        assert!(repository
            .get_by_id(&installation.id)
            .await
            .expect("installation query")
            .is_some());
    }

    #[tokio::test]
    async fn remove_failed_rejects_untrusted_root_without_touching_it() {
        let (database, installation, managed, _retained_directory, _package) =
            ready_lifecycle_fixture("remove-failed-untrusted", false).await;
        let repository = CoreInstallationsRepository::new(database.pool().clone());
        let outside = unique_test_path("remove-failed-untrusted-outside", "dir");
        std::fs::create_dir_all(&outside).expect("outside failed root");
        let marker = outside.join("preserve");
        std::fs::write(&marker, b"preserve").expect("outside marker");
        let outside_text = outside.to_string_lossy().into_owned();
        sqlx::query(
            "UPDATE installations SET state = 'failed', recovery_discriminator = \
             'final_root_present', installation_root = ? WHERE id = ?",
        )
        .bind(outside_text)
        .bind(installation.id.as_str())
        .execute(database.pool())
        .await
        .expect("untrusted failed fixture");

        let error = remove_failed_installation_owned(&repository, &managed, &installation.id)
            .await
            .expect_err("untrusted failed root must be rejected");
        assert!(error.contains("direct child"));
        assert_eq!(std::fs::read(marker).expect("marker survives"), b"preserve");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn remove_failed_refuses_root_symlink_without_following_target() {
        use std::os::unix::fs::symlink;

        let (database, installation, managed, _retained_directory, _package) =
            ready_lifecycle_fixture("remove-failed-root-symlink", false).await;
        let repository = CoreInstallationsRepository::new(database.pool().clone());
        std::fs::remove_dir_all(&installation.installation_root).expect("remove original root");
        let outside = unique_test_path("remove-failed-root-symlink-target", "dir");
        std::fs::create_dir_all(&outside).expect("symlink target");
        let marker = outside.join("preserve");
        std::fs::write(&marker, b"preserve").expect("target marker");
        symlink(&outside, &installation.installation_root).expect("root symlink fixture");
        sqlx::query(
            "UPDATE installations SET state = 'failed', \
             recovery_discriminator = 'final_root_present' WHERE id = ?",
        )
        .bind(installation.id.as_str())
        .execute(database.pool())
        .await
        .expect("failed symlink fixture transition");

        let error = remove_failed_installation_owned(&repository, &managed, &installation.id)
            .await
            .expect_err("root symlink must be refused");
        assert!(error.contains("refusing to remove a symbolic link"));
        assert!(std::fs::symlink_metadata(&installation.installation_root).is_ok());
        assert_eq!(
            std::fs::read(marker).expect("target marker survives"),
            b"preserve"
        );
    }

    #[tokio::test]
    async fn coordinate_lifecycle_lock_serializes_same_game() {
        let app_data = unique_test_path("coordinate-lifecycle-lock", "dir");
        std::fs::create_dir_all(&app_data).expect("app data fixture");
        let coordinate = "30402:merchant:game";
        let first = acquire_coordinate_lifecycle_lock(&app_data, coordinate)
            .await
            .expect("first coordinate lock");

        let error = acquire_coordinate_lifecycle_lock(&app_data, coordinate)
            .await
            .expect_err("second operation for the same coordinate must be busy");
        assert!(error.contains("another installation operation"));

        drop(first);
        acquire_coordinate_lifecycle_lock(&app_data, coordinate)
            .await
            .expect("coordinate lock after release");
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
            accepted_terms: Vec::new(),
            malformed_accepted_terms_tags: Vec::new(),
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

    // Startup transports must live on the same durable runtime as commands.
    // A local Runtime captured by the FnOnce setup hook is dropped after setup,
    // cancelling relay tasks even though their cached status remains Connected.
    let account_manager = startup_runtime::block_on(AccountManager::new(&keys_dir))
        .expect("Failed to initialize local account manager");
    info!("Local account manager initialized");

    let (database, nostr_client, user_cache, marketplace_cache, purchases, nip05_validator) =
        startup_runtime::block_on(async {
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
    startup_runtime::block_on(async {
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

    let database = Arc::new(database);
    let auth = Arc::new(Mutex::new(AuthState::new()));
    let signer_state = Arc::new(Mutex::new(AppSignerState::new()));
    let blossom_provider: Arc<dyn blossom_upload::BlossomAccountProvider> = Arc::new(
        blossom_commands::DesktopBlossomAccountProvider::new(auth.clone(), signer_state.clone()),
    );
    let blossom_upload = Arc::new(blossom_upload::BlossomUploadService::new(
        blossom_provider.clone(),
        blossom_upload::BlossomUploadConfig {
            allow_loopback: cfg!(debug_assertions),
            ..blossom_upload::BlossomUploadConfig::default()
        },
    ));
    #[cfg(debug_assertions)]
    let blossom_settings = blossom_settings::BlossomServerSettingsRepository::development_loopback(
        database.clone(),
        blossom_provider,
    );
    #[cfg(not(debug_assertions))]
    let blossom_settings = blossom_settings::BlossomServerSettingsRepository::production(
        database.clone(),
        blossom_provider,
    );
    let blossom_state =
        blossom_commands::BlossomManagedState::new(blossom_upload, Arc::new(blossom_settings));

    let mut builder = tauri::Builder::default();

    #[cfg(debug_assertions)]
    {
        builder = builder.plugin(tauri_plugin_mcp_bridge::init());
    }

    builder
        .plugin(tauri_plugin_dialog::init())
        .manage(
            archive_selection::ArchiveSelectionService::new()
                .expect("archive staging service should initialize"),
        )
        .manage(AppState {
            settlement_recovery: Default::default(),
            auth,
            nostr: nostr_client.clone(),
            database,
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
        .manage(Arc::new(launcher::Launcher::new().expect("game supervisor should initialize")))
        .manage(signer_state)
        .manage(blossom_state)
        .manage(Arc::new(account_manager))
        .setup(move |app| {
            settlement_ui::spawn_startup_recovery(app.handle().clone());
            // Recovery completes synchronously before the first window is shown,
            // so stale installing rows cannot be presented to the frontend.
            let recovery_app_data = app
                .path()
                .app_local_data_dir()
                .map_err(|error| std::io::Error::other(format!("failed to resolve app data directory: {error}")))?;
            let recovery_repository = InstallationsRepository::new(
                app.state::<AppState>().database.pool().clone(),
            );
            let recovery_report = startup_runtime::block_on(install::recover_interrupted_installations(
                    &recovery_repository,
                    &recovery_app_data.join("installations"),
                    now_unix_i64().map_err(|error| std::io::Error::other(format!("recovery clock failed: {error}")))?,
                ))
                .map_err(|error| std::io::Error::other(format!("startup install recovery failed: {error}")))?;
            info!(?recovery_report, "startup install recovery completed");

            // Ensure window is visible and focused
            if let Some(window) = app.get_webview_window("main") {
                #[cfg(debug_assertions)]
                warn_if_debug_uses_embedded_frontend(window.url().ok().as_ref());
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
            blossom_commands::select_blossom_media_file,
            blossom_commands::start_blossom_upload,
            blossom_commands::retry_blossom_upload,
            blossom_commands::cancel_blossom_upload,
            blossom_commands::discard_blossom_media_selection,
            blossom_commands::get_blossom_server_settings,
            blossom_commands::probe_blossom_server_health,
            blossom_commands::replace_blossom_server_settings,
            blossom_commands::add_blossom_server,
            blossom_commands::update_blossom_server,
            blossom_commands::remove_blossom_server,
            blossom_commands::reorder_blossom_servers,
            blossom_commands::set_preferred_blossom_server,
            blossom_commands::resolve_blossom_server_candidates,
            adp_commands::check_adp_server,
            adp_commands::discover_adp_servers,
            adp_commands::discover_campaigns,
            adp_commands::discover_campaign_summaries,
            adp_commands::publish_campaign,
            adp_commands::update_campaign_pointer,
            adp_commands::hash_build_file,
            adp_commands::select_build_file,
            adp_commands::connect_nwc_wallet,
            adp_commands::get_nwc_wallet,
            adp_commands::disconnect_nwc_wallet,
            adp_commands::verify_nwc_wallet,
            adp_commands::request_lnurl_invoice,
            adp_commands::pay_nwc_invoice,
            adp_commands::get_purchase_plan,
            adp_commands::approve_purchase_plan,
            adp_commands::confirm_purchase,
            adp_commands::start_split_purchase,
            adp_commands::next_split_invoice,
            adp_commands::submit_split_payment,
            adp_commands::resume_split_payment,
            settlement_ui::reconcile_pending_settlements,
            settlement_ui::recipient_refund_history,
            settlement_ui::pay_split_refund,
            settlement_ui::resume_split_refund,
            adp_commands::settlement_history,
            adp_commands::refresh_settlement,
            adp_commands::replace_split_refund_destination,
            adp_commands::prepare_split_recovery,
            adp_commands::prepare_split_refund,
            adp_commands::confirm_split_refund,
            adp_commands::claim_entitlement,
            adp_commands::resolve_adp_operator,
            adp_commands::load_authoritative_adp_edit_metadata,
            adp_commands::publish_adp_listing,
            adp_commands::publish_adp_listing_with_contract,
            archive_commands::select_archive_build,
            archive_commands::inspect_archive,
            archive_commands::hash_selected_archive,
            archive_commands::query_archive_files,
            archive_commands::query_archive_working_directories,
            archive_commands::query_archive_candidates,
            archive_commands::query_archive_hazards,
            archive_commands::query_archive_warnings,
            archive_commands::discard_archive_selection,
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
            launcher::get_launch_options,
            launcher::set_launch_profile_preference,
            launcher::launch_game,
            launcher::get_launch_status,
            list_installation_lifecycle_issues,
            retry_installation_recovery,
            get_uninstall_sizes,
            uninstall_game,
            resume_uninstall,
            remove_failed_installation,
            list_retained_packages,
            delete_retained_package,
            reinstall_retained_package,
            add_game_to_library,
            is_game_in_library,
            get_library_games,
            install_game,
            ingest_receipt,
            receipt_commands::sync_purchase_receipts,
            receipt_commands::import_purchase_receipts,
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
            nip46_commands::regtest_nip44_probe,
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
        .build(tauri::generate_context!())
        .expect("error while building Arcadestr")
        .run(|app, event| {
            if matches!(event, tauri::RunEvent::Exit) {
                app.state::<Arc<launcher::Launcher>>().detach_all();
            }
        });
}

#[cfg(debug_assertions)]
fn warn_if_debug_uses_embedded_frontend(url: Option<&url::Url>) {
    const STATUS: &str = env!("ARCADESTR_FRONTEND_BUILD_STATUS");
    const BUILT_AT: &str = env!("ARCADESTR_FRONTEND_BUILD_UNIX");
    const FINGERPRINT: &str = env!("ARCADESTR_FRONTEND_BUILD_FINGERPRINT");
    const COMMAND: &str = env!("ARCADESTR_FRONTEND_BUILD_COMMAND");

    if url.is_some_and(|url| {
        matches!(url.scheme(), "http" | "https")
            && url
                .host_str()
                .is_some_and(|host| host == "localhost" || host == "127.0.0.1")
    }) {
        return;
    }

    let warning = format!(
        "\n================ ARCADESTR FRONTEND VERIFICATION WARNING ================\n\
         This debug binary may use embedded web/dist assets.\n\
         Embedded frontend status: {STATUS}\n\
         Embedded frontend built_at_unix: {BUILT_AT}\n\
         Embedded frontend fingerprint: {FINGERPRINT}\n\
         Before live verification, run exactly: {COMMAND}\n\
         Then rebuild the desktop binary.\n\
         ========================================================================\n"
    );
    eprintln!("{warning}");
    warn!("{warning}");
}
