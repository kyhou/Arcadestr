//! Tauri commands for ADP publish flow.

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use arcadestr_core::adp_client::{
    AdpClient, AdpServerInfo, PurchaseConfirmRequest as CorePurchaseConfirmRequest,
    PurchaseConfirmResponse as CorePurchaseConfirmResponse, UploadResponse,
};
use arcadestr_core::adp_publish::{
    build_adp_listing_event_builder, build_provisioning_acceptance_event_builder, AdpListingInput,
};
use arcadestr_core::adp_storage::{
    AdpProvisioning, AdpProvisioningRepository, DownloadToken, DownloadTokensRepository,
};
use arcadestr_core::file_hash::sha256_file;
use arcadestr_core::http_client::HttpClient;
use arcadestr_core::lnurlp::{request_invoice, resolve_lud16};
use arcadestr_core::marketplace::confirm_nip99_listing_propagated;
use arcadestr_core::nwc_client::{
    load_default_nwc_connection, save_default_nwc_connection, NwcClient,
};
use arcadestr_core::signers::NostrSigner;
use nostr::nips::nip19::FromBech32;
use serde::{Deserialize, Serialize};
use tauri::{Emitter, State};

use crate::AppState;

#[derive(Debug, Clone, Deserialize)]
pub struct PublishAdpListingRequest {
    pub d_tag: String,
    pub title: String,
    pub description: String,
    pub price_sats: u64,
    pub lud16: Option<String>,
    pub server_url: String,
    pub file_path: String,
    pub version: String,
    pub platforms: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PublishAdpListingResult {
    pub event_id: String,
    pub acceptance_event_id: String,
    pub fulfillment_pubkey: String,
    pub file_hash: String,
    pub upload: UploadResponse,
}

#[derive(Debug, Clone, Serialize)]
pub struct PublishProgressPayload {
    pub step: String,
    pub status: String,
    pub message: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RequestLnurlInvoiceRequest {
    pub lud16: String,
    pub amount_sats: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct RequestLnurlInvoiceResponse {
    pub bolt11: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ConnectNwcWalletRequest {
    pub connection_string: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ConnectNwcWalletResponse {
    pub wallet_pubkey: String,
    pub relays: Vec<String>,
    pub lud16: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PayNwcInvoiceRequest {
    pub bolt11: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct PayNwcInvoiceResponse {
    pub preimage: String,
    pub fees_paid_msat: Option<u64>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ConfirmPurchaseRequest {
    pub publisher_npub: String,
    pub listing_id: String,
    pub server_url: String,
    pub bolt11: String,
    pub preimage: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ConfirmPurchaseResponse {
    pub receipt: nostr::Event,
    pub download_token: String,
    pub token_expires_at: i64,
}

#[tauri::command]
pub async fn check_adp_server(
    server_url: String,
    state: State<'_, AppState>,
) -> Result<AdpServerInfo, String> {
    let client = AdpClient::new(server_url, Arc::clone(&state.http_client));
    client.well_known().await.map_err(|err| err.to_string())
}

#[tauri::command]
pub async fn request_lnurl_invoice(
    request: RequestLnurlInvoiceRequest,
    state: State<'_, AppState>,
) -> Result<RequestLnurlInvoiceResponse, String> {
    request_lnurl_invoice_with_http(state.http_client.as_ref(), request).await
}

#[tauri::command]
pub async fn connect_nwc_wallet(
    request: ConnectNwcWalletRequest,
) -> Result<ConnectNwcWalletResponse, String> {
    let connection = save_default_nwc_connection(&request.connection_string)
        .map_err(|err| err.to_string())?;
    Ok(nwc_connection_response(&connection))
}

fn nwc_connection_response(
    connection: &arcadestr_core::nwc_client::NwcConnection,
) -> ConnectNwcWalletResponse {
    ConnectNwcWalletResponse {
        wallet_pubkey: connection.wallet_pubkey_hex().to_string(),
        relays: connection.relay_urls().to_vec(),
        lud16: connection.lud16().map(ToOwned::to_owned),
    }
}

#[tauri::command]
pub async fn pay_nwc_invoice(
    request: PayNwcInvoiceRequest,
) -> Result<PayNwcInvoiceResponse, String> {
    let connection = load_default_nwc_connection().map_err(|err| err.to_string())?;
    let result = NwcClient::new(connection)
        .pay_invoice(&request.bolt11)
        .await
        .map_err(|err| err.to_string())?;
    Ok(PayNwcInvoiceResponse {
        preimage: result.preimage,
        fees_paid_msat: result.fees_paid_msat,
    })
}

#[tauri::command]
pub async fn confirm_purchase(
    request: ConfirmPurchaseRequest,
    state: State<'_, AppState>,
) -> Result<ConfirmPurchaseResponse, String> {
    let auth_snapshot = { state.auth.lock().await.clone() };
    let signer = auth_snapshot
        .signer()
        .ok_or_else(|| "not authenticated".to_string())?;
    let buyer_pubkey = signer
        .get_public_key()
        .await
        .map_err(|err| err.to_string())?
        .to_hex();

    let game_coordinate = listing_coordinate_from_npub(&request.publisher_npub, &request.listing_id)?;
    let listing_event = fetch_listing_event_by_coordinate(&state, &game_coordinate).await?;
    let listing_for_validation = listing_event.clone();
    let client = AdpClient::new(request.server_url.clone(), Arc::clone(&state.http_client));
    let response = client
        .purchase_confirm(
            signer,
            CorePurchaseConfirmRequest {
                game_coordinate: game_coordinate.clone(),
                listing_event,
                bolt11: request.bolt11,
                preimage: request.preimage,
            },
        )
        .await
        .map_err(|err| err.to_string())?;
    persist_purchase_confirmation(
        &state,
        &buyer_pubkey,
        &game_coordinate,
        &request.server_url,
        response,
        &listing_for_validation,
    )
    .await
}

fn listing_coordinate_from_npub(publisher_npub: &str, listing_id: &str) -> Result<String, String> {
    let merchant_pubkey = nostr::PublicKey::from_bech32(publisher_npub)
        .map_err(|_| "invalid publisher pubkey".to_string())?;
    Ok(format!("30402:{}:{}", merchant_pubkey.to_hex(), listing_id))
}

async fn request_lnurl_invoice_with_http(
    http: &dyn HttpClient,
    request: RequestLnurlInvoiceRequest,
) -> Result<RequestLnurlInvoiceResponse, String> {
    let endpoint = resolve_lud16(http, &request.lud16)
        .await
        .map_err(|err| err.to_string())?;
    let amount_msat = request
        .amount_sats
        .checked_mul(1_000)
        .ok_or_else(|| "amount_sats overflows millisats".to_string())?;
    let bolt11 = request_invoice(http, &endpoint, amount_msat)
        .await
        .map_err(|err| err.to_string())?;
    Ok(RequestLnurlInvoiceResponse { bolt11 })
}

async fn fetch_listing_event_by_coordinate(
    state: &State<'_, AppState>,
    coordinate: &str,
) -> Result<nostr::Event, String> {
    let mut parts = coordinate.splitn(3, ':');
    let kind = parts.next().ok_or_else(|| "missing coordinate kind".to_string())?;
    let developer = parts
        .next()
        .ok_or_else(|| "missing coordinate developer".to_string())?;
    let d_tag = parts
        .next()
        .ok_or_else(|| "missing coordinate d tag".to_string())?;
    if kind != "30402" {
        return Err("purchase coordinate must be a kind:30402 listing".to_string());
    }

    let developer_pubkey = nostr::PublicKey::from_hex(developer)
        .map_err(|err| format!("invalid coordinate developer pubkey: {err}"))?;
    let relay_manager = { state.nostr.lock().await.get_relay_manager().clone() };
    let events = relay_manager
        .lock()
        .await
        .fetch_events_with_timeout(
            nostr::Filter::new()
                .kind(nostr::Kind::Custom(30402))
                .author(developer_pubkey)
                .identifier(d_tag),
            10,
        )
        .await
        .map_err(|err| err.to_string())?;
    events
        .into_iter()
        .max_by_key(|event| event.created_at.as_secs())
        .ok_or_else(|| "listing event not found on relays".to_string())
}

async fn persist_purchase_confirmation(
    state: &State<'_, AppState>,
    buyer_pubkey: &str,
    game_coordinate: &str,
    server_url: &str,
    response: CorePurchaseConfirmResponse,
    listing_event: &nostr::Event,
) -> Result<ConfirmPurchaseResponse, String> {
    let receipt = arcadestr_core::purchases::parse_and_validate_receipt_with_listing(
        &response.receipt,
        buyer_pubkey,
        listing_event,
    )
    .map_err(|err| err.to_string())?;
    state
        .purchases
        .upsert_receipt(&receipt)
        .await
        .map_err(|err| err.to_string())?;
    let tokens = DownloadTokensRepository::new(state.database.pool().clone());
    tokens
        .upsert(&DownloadToken {
            game_coordinate: game_coordinate.to_string(),
            server_url: server_url.to_string(),
            token: response.download_token.clone(),
            expires_at: response.token_expires_at,
        })
        .await
        .map_err(|err| err.to_string())?;

    Ok(ConfirmPurchaseResponse {
        receipt: response.receipt,
        download_token: response.download_token,
        token_expires_at: response.token_expires_at,
    })
}

#[tauri::command]
pub async fn publish_adp_listing<R: tauri::Runtime>(
    request: PublishAdpListingRequest,
    app: tauri::AppHandle<R>,
    state: State<'_, AppState>,
) -> Result<PublishAdpListingResult, String> {
    emit_progress(&app, "check-server", "pending", None)?;
    let adp_client = AdpClient::new(request.server_url.clone(), Arc::clone(&state.http_client));
    let server_info = adp_client
        .well_known()
        .await
        .map_err(|err| progress_error(&app, "check-server", err))?;
    emit_progress(&app, "check-server", "ok", Some(server_info.pubkey.clone()))?;

    emit_progress(&app, "hash-file", "pending", None)?;
    let file_hash = sha256_file(std::path::Path::new(&request.file_path))
        .await
        .map_err(|err| progress_error(&app, "hash-file", err))?;
    emit_progress(&app, "hash-file", "ok", Some(file_hash.clone()))?;

    let auth_snapshot = { state.auth.lock().await.clone() };
    let signer = auth_snapshot
        .signer()
        .ok_or_else(|| "not authenticated".to_string())?;
    let developer_pubkey = signer
        .get_public_key()
        .await
        .map_err(|err| err.to_string())?;
    let developer_npub = developer_pubkey.to_hex();

    let provisioning_repo = AdpProvisioningRepository::new(state.database.pool().clone());
    let scope = request.d_tag.as_str();

    emit_progress(&app, "provision", "pending", None)?;
    let provisioning = resolve_provisioning(ResolveProvisioningInput {
        provisioning_repo: &provisioning_repo,
        adp_client: &adp_client,
        signer,
        developer_pubkey,
        developer_npub: &developer_npub,
        server_url: &request.server_url,
        scope,
        server_info: &server_info,
    })
    .await
    .map_err(|err| progress_error(&app, "provision", err))?;
    let (fulfillment_pubkey, acceptance_event_id, fulfillment_valid_from) = match provisioning {
        ProvisioningDecision::Reused {
            fulfillment_pubkey,
            acceptance_event_id,
            valid_from,
        } => {
            emit_progress(
                &app,
                "provision",
                "ok",
                Some("reused existing provisioning".into()),
            )?;
            (fulfillment_pubkey, acceptance_event_id, valid_from)
        }
        ProvisioningDecision::Created {
            fulfillment_pubkey,
            acceptance_event_id,
            valid_from,
            acceptance_event,
            row,
        } => {
            publish_event(&state, &acceptance_event)
                .await
                .map_err(|err| progress_error(&app, "provision", err))?;
            provisioning_repo
                .upsert(&row)
                .await
                .map_err(|err| progress_error(&app, "provision", err))?;
            emit_progress(&app, "provision", "ok", Some("created provisioning".into()))?;
            (fulfillment_pubkey, acceptance_event_id, valid_from)
        }
    };

    emit_progress(&app, "publish-listing", "pending", None)?;
    let listing_input = AdpListingInput {
        d_tag: request.d_tag.clone(),
        title: request.title.clone(),
        description: request.description.clone(),
        price_sats: request.price_sats,
        lud16: request.lud16.clone(),
        server_url: request.server_url.clone(),
        file_hash: file_hash.clone(),
        version: request.version.clone(),
        fulfillment_pubkey: fulfillment_pubkey.clone(),
        fulfillment_valid_from: fulfillment_valid_from
            .try_into()
            .map_err(|_| "provisioning valid_from is negative".to_string())?,
        platforms: request.platforms.clone(),
    };
    let listing_builder =
        build_adp_listing_event_builder(&listing_input).map_err(|err| err.to_string())?;
    let listing_event = signer
        .sign_event(listing_builder.build(developer_pubkey))
        .await
        .map_err(|err| progress_error(&app, "publish-listing", err))?;
    publish_event(&state, &listing_event)
        .await
        .map_err(|err| progress_error(&app, "publish-listing", err))?;
    emit_progress(
        &app,
        "publish-listing",
        "ok",
        Some(listing_event.id.to_hex()),
    )?;

    emit_progress(&app, "confirm-propagation", "pending", None)?;
    let relay_manager = { state.nostr.lock().await.get_relay_manager().clone() };
    let coordinate = format!("30402:{}:{}", listing_event.pubkey.to_hex(), request.d_tag);
    let propagated = confirm_nip99_listing_propagated(&relay_manager, &coordinate, 2)
        .await
        .map_err(|err| progress_error(&app, "confirm-propagation", err))?;
    if !propagated {
        emit_progress(
            &app,
            "confirm-propagation",
            "error",
            Some("listing not visible on two relays".into()),
        )?;
        return Err("listing not visible on two relays".into());
    }
    emit_progress(&app, "confirm-propagation", "ok", Some(coordinate.clone()))?;

    emit_progress(&app, "upload", "pending", None)?;
    let upload = adp_client
        .upload(
            signer,
            &listing_event,
            std::path::Path::new(&request.file_path),
        )
        .await
        .map_err(|err| progress_error(&app, "upload", err))?;
    emit_progress(&app, "upload", "ok", Some(upload.download_url.clone()))?;

    Ok(PublishAdpListingResult {
        event_id: listing_event.id.to_hex(),
        acceptance_event_id,
        fulfillment_pubkey,
        file_hash,
        upload,
    })
}

enum ProvisioningDecision {
    Reused {
        fulfillment_pubkey: String,
        acceptance_event_id: String,
        valid_from: i64,
    },
    Created {
        fulfillment_pubkey: String,
        acceptance_event_id: String,
        valid_from: i64,
        acceptance_event: Box<nostr::Event>,
        row: Box<AdpProvisioning>,
    },
}

struct ResolveProvisioningInput<'a> {
    provisioning_repo: &'a AdpProvisioningRepository,
    adp_client: &'a AdpClient,
    signer: &'a dyn NostrSigner,
    developer_pubkey: nostr::PublicKey,
    developer_npub: &'a str,
    server_url: &'a str,
    scope: &'a str,
    server_info: &'a AdpServerInfo,
}

async fn resolve_provisioning(input: ResolveProvisioningInput<'_>) -> Result<ProvisioningDecision, String> {
    if let Some(existing) = input.provisioning_repo
        .active_for_scope(input.developer_npub, input.server_url, Some(input.scope))
        .await
        .map_err(|err| err.to_string())?
    {
        return Ok(ProvisioningDecision::Reused {
            fulfillment_pubkey: existing.fulfillment_pubkey,
            acceptance_event_id: existing.acceptance_event_id,
            valid_from: existing.valid_from,
        });
    }

    let provision = input.adp_client
        .provision(input.signer, Some(input.scope))
        .await
        .map_err(|err| err.to_string())?;
    let acceptance_builder = build_provisioning_acceptance_event_builder(
        &input.server_info.pubkey,
        &provision.fulfillment_pubkey,
    );
    let acceptance_event = input.signer
        .sign_event(acceptance_builder.build(input.developer_pubkey))
        .await
        .map_err(|err| err.to_string())?;
    let acceptance_event_id = acceptance_event.id.to_hex();
    let now = now_unix_i64()?;
    let row = AdpProvisioning {
        id: format!("{}:{}:{}", input.developer_npub, input.server_url, input.scope),
        developer_npub: input.developer_npub.to_string(),
        server_url: input.server_url.to_string(),
        operator_pubkey: input.server_info.pubkey.clone(),
        scope: Some(input.scope.to_string()),
        fulfillment_pubkey: provision.fulfillment_pubkey.clone(),
        attestation_event_id: provision.attestation_event_id,
        acceptance_event_id: acceptance_event_id.clone(),
        valid_from: now,
        revoked_at: None,
        created_at: now,
    };

    Ok(ProvisioningDecision::Created {
        fulfillment_pubkey: provision.fulfillment_pubkey,
        acceptance_event_id,
        valid_from: row.valid_from,
        acceptance_event: Box::new(acceptance_event),
        row: Box::new(row),
    })
}

async fn publish_event(state: &State<'_, AppState>, event: &nostr::Event) -> Result<(), String> {
    let relay_manager = { state.nostr.lock().await.get_relay_manager().clone() };
    let manager = relay_manager.lock().await;
    let result = manager
        .send_event(event)
        .await
        .map_err(|err| err.to_string())?;
    if result.success_count == 0 {
        return Err("event publish failed on all relays".to_string());
    }
    Ok(())
}

fn emit_progress<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    step: &str,
    status: &str,
    message: Option<String>,
) -> Result<(), String> {
    app.emit(
        "publish-progress",
        PublishProgressPayload {
            step: step.to_string(),
            status: status.to_string(),
            message,
        },
    )
    .map_err(|err| err.to_string())
}

fn progress_error<R: tauri::Runtime, E: std::fmt::Display>(
    app: &tauri::AppHandle<R>,
    step: &str,
    err: E,
) -> String {
    let message = err.to_string();
    let _ = emit_progress(app, step, "error", Some(message.clone()));
    message
}

fn now_unix_i64() -> Result<i64, String> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .map_err(|err| err.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    use std::path::PathBuf;
    use std::time::Duration;

    use arcadestr_core::adp_client::{AdpClient, AdpServerInfo};
    use arcadestr_core::adp_storage::AdpProvisioningRepository;
    use arcadestr_core::auth::AuthState;
    use arcadestr_core::http_client::{HttpClient, ReqwestHttpClient};
    use arcadestr_core::marketplace_cache::MarketplaceCache;
    use arcadestr_core::nip05_validator::Nip05Validator;
    use arcadestr_core::nostr::{EventDeduplicator, NostrClient};
    use arcadestr_core::profile_fetcher::ProfileFetcher;
    use arcadestr_core::relay_cache::RelayCache;
    use arcadestr_core::relay_hints::RelayHints;
    use arcadestr_core::relay_manager::RelayManagerConfig;
    use arcadestr_core::signers::{LocalSigner, NostrSigner};
    use arcadestr_core::storage::Database;
    use arcadestr_core::subscriptions::SubscriptionRegistry;
    use arcadestr_core::user_cache::UserCache;
    use nostr::nips::nip19::ToBech32;
    use serde_json::json;
    use tauri::Manager;
    use tokio::sync::{Mutex as AsyncMutex, RwLock};

    async fn test_db() -> Database {
        let path = std::env::temp_dir().join(format!(
            "arcadestr-adp-command-{}.db",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system clock should be after epoch")
                .as_nanos()
        ));
        Database::new(&path)
            .await
            .expect("test database should open")
    }

    #[tokio::test]
    async fn request_lnurl_invoice_resolves_lud16_and_requests_fixed_amount_invoice() {
        let http = Arc::new(
            LocalMockHttpClient::default()
                .with_json_response(
                    "https://example.com/.well-known/lnurlp/buyer",
                    json!({
                        "callback": "https://example.com/lnurl/callback",
                        "minSendable": 1_000,
                        "maxSendable": 10_000,
                        "nostrPubkey": "lsp-pubkey"
                    }),
                )
                .with_json_response(
                    "https://example.com/lnurl/callback?amount=5000",
                    json!({ "pr": "lnbc5u1fixed" }),
                ),
        );

        let response = request_lnurl_invoice_with_http(
            http.as_ref(),
            RequestLnurlInvoiceRequest {
                lud16: "buyer@example.com".to_string(),
                amount_sats: 5,
            },
        )
        .await
        .expect("invoice request succeeds");

        assert_eq!(response.bolt11, "lnbc5u1fixed");
    }

    #[test]
    fn nwc_connection_response_exposes_safe_metadata_without_secret() {
        let uri = concat!(
            "nostr+walletconnect://",
            "0000000000000000000000000000000000000000000000000000000000000001",
            "?relay=wss%3A%2F%2Frelay.example.com",
            "&secret=0000000000000000000000000000000000000000000000000000000000000002",
            "&lud16=buyer%40example.com"
        );
        let connection = arcadestr_core::nwc_client::NwcConnection::parse(uri)
            .expect("valid NWC connection should parse");

        let response = nwc_connection_response(&connection);

        assert_eq!(
            response.wallet_pubkey,
            "0000000000000000000000000000000000000000000000000000000000000001"
        );
        assert_eq!(response.relays, vec!["wss://relay.example.com".to_string()]);
        assert_eq!(response.lud16.as_deref(), Some("buyer@example.com"));
        assert!(!format!("{response:?}").contains("0000000000000000000000000000000000000000000000000000000000000002"));
    }

    fn unique_test_path(prefix: &str, extension: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "{prefix}-{}.{extension}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system clock should be after epoch")
                .as_nanos()
        ))
    }

    fn live_relays_from_env() -> Vec<String> {
        std::env::var("ARCADESTR_RELAYS")
            .expect("ARCADESTR_RELAYS must be set for live ADP publish test")
            .split(',')
            .map(str::trim)
            .filter(|relay| !relay.is_empty())
            .map(ToOwned::to_owned)
            .collect()
    }

    async fn live_test_app(relays: Vec<String>) -> tauri::App<tauri::test::MockRuntime> {
        let db_path = unique_test_path("arcadestr-adp-live-command", "db");
        let database = Database::new(&db_path)
            .await
            .expect("live test database should open");
        let user_cache = Arc::new(UserCache::new(database.pool().clone()));
        let relay_config = RelayManagerConfig {
            debug_relays: Some(relays),
            block_discovery: true,
            ..RelayManagerConfig::default()
        };
        let nostr = NostrClient::new_with_cache(
            "adp-live-command".to_string(),
            vec![],
            user_cache.clone(),
            Some(relay_config.clone()),
        )
        .await
        .expect("live test nostr client should initialize");
        nostr.connect().await;
        tokio::time::sleep(Duration::from_millis(500)).await;

        let validator_client = NostrClient::new_with_cache(
            "adp-live-validator".to_string(),
            vec![],
            user_cache.clone(),
            Some(relay_config),
        )
        .await
        .expect("live test validator client should initialize");
        let nip05_validator = Arc::new(std::sync::Mutex::new(Nip05Validator::spawn(
            Arc::new(validator_client),
            user_cache.clone(),
        )));
        let mut auth = AuthState::new();
        auth.connect_with_key("0000000000000000000000000000000000000000000000000000000000000001")
            .expect("live test signer should initialize");
        let relay_cache = Arc::new(
            RelayCache::new(unique_test_path("arcadestr-adp-live-relay-cache", "db"))
                .expect("relay cache should open"),
        );
        let relay_hints = Arc::new(
            RelayHints::new(unique_test_path("arcadestr-adp-live-relay-hints", "db"))
                .expect("relay hints should open"),
        );
        let http_client: Arc<dyn HttpClient> = Arc::new(
            ReqwestHttpClient::new(Duration::from_secs(10))
                .expect("reqwest http client should initialize"),
        );
        let profile_fetcher = Arc::new({
            let mut fetcher = ProfileFetcher::with_persistent_cache(user_cache.clone());
            fetcher.with_nip05_validator(nip05_validator.clone());
            fetcher
        });
        let marketplace_cache = Arc::new(MarketplaceCache::new(database.pool().clone()));
        let purchases = Arc::new(arcadestr_core::purchases::PurchasesRepository::new(
            database.pool().clone(),
        ));

        tauri::test::mock_builder()
            .manage(AppState {
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
            })
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .expect("mock Tauri app should build")
    }

    async fn write_live_test_file() -> PathBuf {
        let file_path = unique_test_path("arcadestr-adp-live-build", "bin");
        let bytes: Vec<u8> = (0..4096).map(|index| (index % 251) as u8).collect();
        tokio::fs::write(&file_path, bytes)
            .await
            .expect("live test binary should be written");
        file_path
    }

    fn tag_values(event: &nostr::Event, tag_name: &str) -> Vec<Vec<String>> {
        event
            .tags
            .iter()
            .map(|tag| tag.clone().to_vec())
            .filter(|values| values.first().map(String::as_str) == Some(tag_name))
            .collect()
    }

    async fn fetch_live_listing_event(
        app: &tauri::App<tauri::test::MockRuntime>,
        event_id: &str,
    ) -> nostr::Event {
        let event_id = nostr::EventId::from_hex(event_id).expect("listing event id should parse");
        let relay_manager = {
            let state = app.state::<AppState>();
            let relay_manager = state.nostr.lock().await.get_relay_manager().clone();
            relay_manager
        };
        let manager = relay_manager.lock().await;
        let events = manager
            .fetch_events_with_timeout(nostr::Filter::new().id(event_id), 5)
            .await
            .expect("listing event should fetch from live relays");
        events
            .into_iter()
            .find(|event| event.id == event_id)
            .expect("published listing event should be returned by a relay")
    }

    fn assert_live_listing_tags(event: &nostr::Event, result: &PublishAdpListingResult, lud16: &str) {
        assert!(
            !tag_values(event, "server").is_empty(),
            "missing server tag"
        );
        assert_eq!(
            tag_values(event, "file_hash")[0],
            vec!["file_hash", result.file_hash.as_str()]
        );
        assert_eq!(
            tag_values(event, "version")[0],
            vec!["version", "0.0.1-live"]
        );
        assert_eq!(
            tag_values(event, "lud16")[0],
            vec!["lud16", lud16]
        );
        assert_eq!(
            tag_values(event, "platform")[0],
            vec!["platform", "linux-x86_64"]
        );

        let fulfillment_tags = tag_values(event, "fulfillment_pubkey");
        assert_eq!(fulfillment_tags.len(), 1);
        let fulfillment_tag = &fulfillment_tags[0];
        assert_eq!(fulfillment_tag.len(), 4);
        assert_eq!(fulfillment_tag[0], "fulfillment_pubkey");
        assert_eq!(fulfillment_tag[1], result.fulfillment_pubkey);
        assert!(
            fulfillment_tag[2].parse::<u64>().is_ok(),
            "fulfillment_pubkey valid_from should be a unix timestamp"
        );
        assert_eq!(fulfillment_tag[3], "");
    }

    #[derive(Default)]
    struct LocalMockHttpClient {
        get_responses: Mutex<HashMap<String, serde_json::Value>>,
        post_responses: Mutex<HashMap<String, serde_json::Value>>,
        post_counts: Mutex<HashMap<String, usize>>,
    }

    impl LocalMockHttpClient {
        fn with_json_response(self, url: &str, body: serde_json::Value) -> Self {
            self.get_responses
                .lock()
                .expect("get responses mutex poisoned")
                .insert(url.to_string(), body);
            self
        }

        fn with_json_post_response(self, url: &str, body: serde_json::Value) -> Self {
            self.post_responses
                .lock()
                .expect("post responses mutex poisoned")
                .insert(url.to_string(), body);
            self
        }

        fn post_call_count(&self, url: &str) -> usize {
            self.post_counts
                .lock()
                .expect("post counts mutex poisoned")
                .get(url)
                .copied()
                .unwrap_or(0)
        }
    }

    #[async_trait::async_trait]
    impl HttpClient for LocalMockHttpClient {
        async fn get_json(
            &self,
            url: &str,
        ) -> Result<serde_json::Value, arcadestr_core::http_client::HttpClientError> {
            self.get_responses
                .lock()
                .expect("get responses mutex poisoned")
                .get(url)
                .cloned()
                .ok_or_else(|| {
                    arcadestr_core::http_client::HttpClientError::Request(format!(
                        "No local desktop mock GET response for {url}"
                    ))
                })
        }

        async fn get_json_no_redirects(
            &self,
            url: &str,
        ) -> Result<serde_json::Value, arcadestr_core::http_client::HttpClientError> {
            self.get_json(url).await
        }

        async fn post_json(
            &self,
            url: &str,
            _body: serde_json::Value,
            _headers: Vec<(String, String)>,
        ) -> Result<serde_json::Value, arcadestr_core::http_client::HttpClientError> {
            *self
                .post_counts
                .lock()
                .expect("post counts mutex poisoned")
                .entry(url.to_string())
                .or_insert(0) += 1;
            self.post_responses
                .lock()
                .expect("post responses mutex poisoned")
                .get(url)
                .cloned()
                .ok_or_else(|| {
                    arcadestr_core::http_client::HttpClientError::Request(format!(
                        "No local desktop mock POST response for {url}"
                    ))
                })
        }
    }

    #[tokio::test]
    async fn publish_same_scope_reuses_provisioning_without_second_provision_call() {
        let db = test_db().await;
        let repo = AdpProvisioningRepository::new(db.pool().clone());
        let provision_url = "https://dist.example.com/provision";
        let mock = Arc::new(LocalMockHttpClient::default().with_json_post_response(
            provision_url,
            json!({
                "fulfillment_pubkey": "fulfillment-key",
                "attestation_event_id": "attestation-id",
                "scope": "game"
            }),
        ));
        let http: Arc<dyn HttpClient> = mock.clone();
        let client = AdpClient::new("https://dist.example.com", http);
        let signer = LocalSigner::from_hex(
            "0000000000000000000000000000000000000000000000000000000000000001",
        )
        .expect("test key should be valid");
        let developer_pubkey = signer
            .get_public_key()
            .await
            .expect("public key should load");
        let developer_npub = developer_pubkey.to_hex();
        let server_info = AdpServerInfo {
            adp_version: "0.2.0".to_string(),
            pubkey: "operator-key".to_string(),
            name: Some("Test ADP".to_string()),
            url: Some("https://dist.example.com".to_string()),
        };

        let first = resolve_provisioning(ResolveProvisioningInput {
            provisioning_repo: &repo,
            adp_client: &client,
            signer: &signer,
            developer_pubkey,
            developer_npub: &developer_npub,
            server_url: "https://dist.example.com",
            scope: "game",
            server_info: &server_info,
        })
        .await
        .expect("first provisioning should succeed");
        let ProvisioningDecision::Created { row, .. } = first else {
            panic!("first call should provision");
        };
        repo.upsert(&row).await.expect("row should persist");

        let second = resolve_provisioning(ResolveProvisioningInput {
            provisioning_repo: &repo,
            adp_client: &client,
            signer: &signer,
            developer_pubkey,
            developer_npub: &developer_npub,
            server_url: "https://dist.example.com",
            scope: "game",
            server_info: &server_info,
        })
        .await
        .expect("second provisioning should succeed");

        assert!(matches!(second, ProvisioningDecision::Reused { .. }));
        assert_eq!(mock.post_call_count(provision_url), 1);
    }

    #[tokio::test]
    #[ignore = "requires ADP_TEST_SERVER_URL and live local relays"]
    async fn live_publish_adp_listing_uploads_and_propagates_to_two_relays() {
        let server_url = std::env::var("ADP_TEST_SERVER_URL")
            .expect("ADP_TEST_SERVER_URL must be set for live ADP publish test");
        let lud16 = std::env::var("ADP_TEST_LUD16")
            .unwrap_or_else(|_| "seller@example.com".to_string());
        let price_sats = std::env::var("ADP_TEST_PRICE_SATS")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(0);
        let relays = live_relays_from_env();
        assert_eq!(
            relays,
            vec![
                "ws://localhost:10547".to_string(),
                "ws://localhost:10548".to_string()
            ],
            "live ADP publish test must use the two local Gate 3 relays"
        );
        assert_eq!(
            std::env::var("ARCADESTR_BLOCK_DISCOVERY").as_deref(),
            Ok("1"),
            "live ADP publish test must block public relay discovery"
        );

        let app = live_test_app(relays).await;
        let file_path = write_live_test_file().await;
        let d_tag = format!(
            "gate3-live-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system clock should be after epoch")
                .as_nanos()
        );
        let request = PublishAdpListingRequest {
            d_tag: d_tag.clone(),
            title: "Gate 3 Live Test Binary".to_string(),
            description: "Small live ADP command-level publish test fixture".to_string(),
            price_sats,
            lud16: Some(lud16.clone()),
            server_url,
            file_path: file_path.display().to_string(),
            version: "0.0.1-live".to_string(),
            platforms: vec!["linux-x86_64".to_string()],
        };

        let result = publish_adp_listing(request, app.handle().clone(), app.state::<AppState>())
            .await
            .expect("live ADP publish command should complete");

        assert_eq!(result.file_hash.len(), 64);
        assert!(!result.event_id.is_empty());
        assert!(!result.acceptance_event_id.is_empty());
        assert!(!result.fulfillment_pubkey.is_empty());
        assert!(!result.upload.download_url.is_empty());
        assert_eq!(result.upload.file_hash, result.file_hash);
        assert!(
            result.upload.game_coordinate.contains(&d_tag),
            "upload response should reference the published listing coordinate"
        );
        println!("ADP_LIVE_EVENT_ID={}", result.event_id);
        println!("ADP_LIVE_GAME_COORDINATE={}", result.upload.game_coordinate);
        println!("ADP_LIVE_DOWNLOAD_URL={}", result.upload.download_url);

        let listing_event = fetch_live_listing_event(&app, &result.event_id).await;
        assert_live_listing_tags(&listing_event, &result, &lud16);
    }

    fn listing_parts_from_coordinate(coordinate: &str) -> (String, String) {
        let mut parts = coordinate.splitn(3, ':');
        assert_eq!(parts.next(), Some("30402"));
        let publisher_hex = parts
            .next()
            .expect("coordinate publisher should exist")
            .to_string();
        let listing_id = parts
            .next()
            .expect("coordinate d tag should exist")
            .to_string();
        let publisher_npub = nostr::PublicKey::from_hex(&publisher_hex)
            .expect("coordinate publisher should parse")
            .to_bech32()
            .expect("coordinate publisher should encode as npub");
        (publisher_npub, listing_id)
    }

    struct LiveConfirmPurchaseInput<'a> {
        app: &'a tauri::App<tauri::test::MockRuntime>,
        server_url: &'a str,
        coordinate: &'a str,
        lud16: &'a str,
        nwc_connection: &'a arcadestr_core::nwc_client::NwcConnection,
        use_nwc_payment: bool,
    }

    async fn live_confirm_purchase_pass(input: LiveConfirmPurchaseInput<'_>) -> ConfirmPurchaseResponse {
        eprintln!("ADP_GATE4_STAGE=confirm_pass_start nwc={}", input.use_nwc_payment);
        let (publisher_npub, listing_id) = listing_parts_from_coordinate(input.coordinate);
        eprintln!("ADP_GATE4_STAGE=request_invoice_start nwc={}", input.use_nwc_payment);
        let invoice = request_lnurl_invoice(
            RequestLnurlInvoiceRequest {
                lud16: input.lud16.to_string(),
                amount_sats: 1,
            },
            input.app.state::<AppState>(),
        )
        .await
        .expect("LNURL invoice should resolve and request");
        eprintln!("ADP_GATE4_STAGE=request_invoice_ok nwc={}", input.use_nwc_payment);
        let preimage = if input.use_nwc_payment {
            eprintln!("ADP_GATE4_STAGE=pay_nwc_start primary");
            arcadestr_core::nwc_client::NwcClient::new(input.nwc_connection.clone())
                .pay_invoice(&invoice.bolt11)
                .await
                .expect("NWC pay_invoice should return preimage")
                .preimage
        } else {
            eprintln!("ADP_GATE4_STAGE=pay_nwc_start manual_preimage_fixture");
            arcadestr_core::nwc_client::NwcClient::new(input.nwc_connection.clone())
                .pay_invoice(&invoice.bolt11)
                .await
                .expect("test backend should pay second invoice for manual-path preimage")
                .preimage
        };
        eprintln!("ADP_GATE4_STAGE=pay_preimage_ok nwc={}", input.use_nwc_payment);

        eprintln!("ADP_GATE4_STAGE=confirm_purchase_start nwc={}", input.use_nwc_payment);
        confirm_purchase(
            ConfirmPurchaseRequest {
                publisher_npub,
                listing_id,
                server_url: input.server_url.to_string(),
                bolt11: invoice.bolt11,
                preimage,
            },
            input.app.state::<AppState>(),
        )
        .await
        .expect("purchase confirmation should persist receipt and token")
    }

    #[tokio::test]
    #[ignore = "requires ADP_TEST_SERVER_URL, ARCADESTR_RELAYS, ADP_TEST_LUD16, ADP_TEST_GAME_COORDINATE, and ADP_TEST_BUYER_NWC"]
    async fn live_nwc_purchase_confirm_and_manual_preimage_paths() {
        eprintln!("ADP_GATE4_STAGE=test_start");
        let server_url = std::env::var("ADP_TEST_SERVER_URL")
            .expect("ADP_TEST_SERVER_URL must be set for live Gate 4 test");
        let coordinate = std::env::var("ADP_TEST_GAME_COORDINATE")
            .expect("ADP_TEST_GAME_COORDINATE must be set for live Gate 4 test");
        let lud16 = std::env::var("ADP_TEST_LUD16")
            .expect("ADP_TEST_LUD16 must be set for live Gate 4 test");
        let nwc_connection_string = std::env::var("ADP_TEST_BUYER_NWC")
            .expect("ADP_TEST_BUYER_NWC must be set for live Gate 4 test");
        let relays = live_relays_from_env();

        eprintln!("ADP_GATE4_STAGE=app_init_start");
        let app = live_test_app(relays).await;
        eprintln!("ADP_GATE4_STAGE=app_init_ok");
        eprintln!("ADP_GATE4_STAGE=parse_nwc_start");
        let nwc_connection = arcadestr_core::nwc_client::NwcConnection::parse(&nwc_connection_string)
            .expect("buyer NWC connection string should parse");
        let connect_result = nwc_connection_response(&nwc_connection);
        eprintln!("ADP_GATE4_STAGE=parse_nwc_ok");
        assert!(!connect_result.wallet_pubkey.is_empty());
        assert!(!connect_result.relays.is_empty());

        let primary = live_confirm_purchase_pass(LiveConfirmPurchaseInput {
            app: &app,
            server_url: &server_url,
            coordinate: &coordinate,
            lud16: &lud16,
            nwc_connection: &nwc_connection,
            use_nwc_payment: true,
        })
        .await;
        eprintln!("ADP_GATE4_STAGE=primary_confirm_ok");
        assert!(!primary.download_token.is_empty());
        assert!(primary.token_expires_at > 0);

        let buyer_pubkey = {
            let state = app.state::<AppState>();
            let auth = state.auth.lock().await;
            auth.public_key()
                .expect("live buyer should be authenticated")
                .to_hex()
        };
        let owned = app
            .state::<AppState>()
            .purchases
            .is_owned(&buyer_pubkey, &coordinate)
            .await
            .expect("ownership lookup should succeed");
        assert!(owned, "receipt persistence should flip is_owned");

        let token_repo = DownloadTokensRepository::new(app.state::<AppState>().database.pool().clone());
        let token = token_repo
            .valid_token(&coordinate, &server_url, now_unix_i64().expect("clock should work"))
            .await
            .expect("download token lookup should succeed")
            .expect("download token should be stored");
        assert_eq!(token.token, primary.download_token);
        println!("ADP_GATE4_DOWNLOAD_TOKEN={}", primary.download_token);

        let manual = live_confirm_purchase_pass(LiveConfirmPurchaseInput {
            app: &app,
            server_url: &server_url,
            coordinate: &coordinate,
            lud16: &lud16,
            nwc_connection: &nwc_connection,
            use_nwc_payment: false,
        })
        .await;
        eprintln!("ADP_GATE4_STAGE=manual_confirm_ok");
        assert!(!manual.download_token.is_empty());
    }

    #[tokio::test]
    #[ignore = "requires ADP_TEST_SERVER_URL, ARCADESTR_RELAYS, ADP_TEST_GAME_COORDINATE, ADP_TEST_MANUAL_BOLT11, and ADP_TEST_MANUAL_PREIMAGE"]
    async fn live_manual_preimage_purchase_confirm_persists_receipt_and_token() {
        eprintln!("ADP_GATE4_STAGE=manual_test_start");
        let server_url = std::env::var("ADP_TEST_SERVER_URL")
            .expect("ADP_TEST_SERVER_URL must be set for manual Gate 4 test");
        let coordinate = std::env::var("ADP_TEST_GAME_COORDINATE")
            .expect("ADP_TEST_GAME_COORDINATE must be set for manual Gate 4 test");
        let bolt11 = std::env::var("ADP_TEST_MANUAL_BOLT11")
            .expect("ADP_TEST_MANUAL_BOLT11 must be set for manual Gate 4 test");
        let preimage = std::env::var("ADP_TEST_MANUAL_PREIMAGE")
            .expect("ADP_TEST_MANUAL_PREIMAGE must be set for manual Gate 4 test");
        let relays = live_relays_from_env();

        eprintln!("ADP_GATE4_STAGE=manual_app_init_start");
        let app = live_test_app(relays).await;
        eprintln!("ADP_GATE4_STAGE=manual_app_init_ok");
        let (publisher_npub, listing_id) = listing_parts_from_coordinate(&coordinate);
        eprintln!("ADP_GATE4_STAGE=manual_confirm_purchase_start");
        let response = confirm_purchase(
            ConfirmPurchaseRequest {
                publisher_npub,
                listing_id,
                server_url: server_url.clone(),
                bolt11,
                preimage,
            },
            app.state::<AppState>(),
        )
        .await
        .expect("manual preimage purchase confirmation should persist receipt and token");
        eprintln!("ADP_GATE4_STAGE=manual_confirm_purchase_ok");
        assert!(!response.download_token.is_empty());
        assert!(response.token_expires_at > 0);

        let buyer_pubkey = {
            let state = app.state::<AppState>();
            let auth = state.auth.lock().await;
            auth.public_key()
                .expect("live buyer should be authenticated")
                .to_hex()
        };
        let owned = app
            .state::<AppState>()
            .purchases
            .is_owned(&buyer_pubkey, &coordinate)
            .await
            .expect("ownership lookup should succeed");
        assert!(owned, "manual receipt persistence should flip is_owned");

        let token_repo = DownloadTokensRepository::new(app.state::<AppState>().database.pool().clone());
        let token = token_repo
            .valid_token(&coordinate, &server_url, now_unix_i64().expect("clock should work"))
            .await
            .expect("download token lookup should succeed")
            .expect("download token should be stored");
        assert_eq!(token.token, response.download_token);
        println!("ADP_GATE4_MANUAL_DOWNLOAD_TOKEN={}", response.download_token);
    }
}
