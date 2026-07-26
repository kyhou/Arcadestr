//! Tauri commands for ADP publish flow.

use std::collections::BTreeSet;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use arcadestr_core::adp_client::{
    AdpClient, AdpServerInfo, EntitlementClaimRequest as CoreEntitlementClaimRequest,
    ProvisionResponse, PurchaseConfirmRequest as CorePurchaseConfirmRequest,
    PurchaseConfirmResponse as CorePurchaseConfirmResponse, UploadResponse,
};
use arcadestr_core::adp_discovery::{
    discover_adp_servers as discover_adp_servers_core, AdpServerAnnouncement,
};
use arcadestr_core::adp_publish::{
    build_adp_listing_event_builder, build_fulfillment_authorization_event_builder,
    AdpListingInput, FulfillmentAuthorizationInput,
};
use arcadestr_core::adp_storage::{
    AdpProvisioning, AdpProvisioningRepository, DownloadToken, DownloadTokensRepository,
};
use arcadestr_core::auth::AuthState;
use arcadestr_core::authorization::{
    AuthorizationTerms, CAPABILITY_ISSUE_GRANT, CAPABILITY_ISSUE_RECEIPT, CAPABILITY_UPLOAD_BUILD,
};
use arcadestr_core::file_hash::{sha256_file, sha256_file_with_progress};
use arcadestr_core::http_client::HttpClient;
use arcadestr_core::lnurlp::{request_invoice, resolve_lud16};
use arcadestr_core::marketplace::confirm_nip99_listing_propagated;
use arcadestr_core::nip46::AppSignerState;
use arcadestr_core::nwc_client::{
    load_default_nwc_connection, save_default_nwc_connection, NwcClient,
};
use arcadestr_core::signers::{ActiveSigner, NostrSigner, SignerError};
use arcadestr_core::{is_replaceable_event_newer, is_sha256_hex};
use nostr::nips::nip19::FromBech32;
use serde::{Deserialize, Serialize};
use tauri::{Emitter, State};
use tauri_plugin_dialog::DialogExt;

use crate::AppState;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FulfillmentMode {
    None,
    Direct,
    Delegate,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PublishAdpListingRequest {
    pub expected_publisher_npub: String,
    pub existing_event_id: Option<String>,
    pub d_tag: String,
    pub title: String,
    pub description: String,
    pub price_sats: u64,
    pub lud16: Option<String>,
    pub tags: Vec<String>,
    pub images: Vec<String>,
    pub fulfillment_mode: FulfillmentMode,
    pub operator_url: Option<String>,
    pub servers: Vec<String>,
    pub file_path: Option<String>,
    pub existing_file_hash: Option<String>,
    pub existing_fulfillment_pubkey: Option<String>,
    pub version: Option<String>,
    pub acquisition: arcadestr_core::marketplace::AcquisitionPolicy,
    pub platforms: Vec<String>,
    pub campaigns: Vec<CampaignPointerInput>,
    pub nip94_event_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ResolveAdpOperatorRequest {
    pub publisher_npub: String,
    pub fulfillment_pubkey: String,
    pub scope: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct PublishServerUploadResult {
    pub server_url: String,
    pub status: String,
    pub error: Option<String>,
    pub upload: Option<UploadResponse>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PublishAdpListingResult {
    pub event_id: String,
    pub acceptance_event_id: Option<String>,
    pub fulfillment_pubkey: Option<String>,
    pub file_hash: Option<String>,
    pub uploads: Vec<PublishServerUploadResult>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PublishProgressPayload {
    pub step: String,
    pub status: String,
    pub server_url: Option<String>,
    pub message: Option<String>,
    pub bytes_uploaded: Option<u64>,
    pub total_bytes: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct HashProgressPayload {
    pub bytes_hashed: u64,
    pub total_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FulfillmentMetadata {
    fulfillment_pubkey: Option<String>,
    should_provision: bool,
}

fn resolve_existing_fulfillment_metadata(
    mode: &FulfillmentMode,
    developer_pubkey: &str,
    existing_pubkey: Option<&str>,
) -> Result<FulfillmentMetadata, String> {
    match mode {
        FulfillmentMode::None => {
            if existing_pubkey.is_some() {
                return Err(
                    "existing fulfillment metadata cannot be cleared by an ordinary edit"
                        .to_string(),
                );
            }
            Ok(FulfillmentMetadata {
                fulfillment_pubkey: None,
                should_provision: false,
            })
        }
        FulfillmentMode::Direct => Ok(FulfillmentMetadata {
            fulfillment_pubkey: Some(developer_pubkey.to_string()),
            should_provision: false,
        }),
        FulfillmentMode::Delegate => match existing_pubkey {
            Some(pubkey) if pubkey == developer_pubkey => Ok(FulfillmentMetadata {
                fulfillment_pubkey: None,
                should_provision: true,
            }),
            Some(pubkey) if !pubkey.is_empty() => Ok(FulfillmentMetadata {
                fulfillment_pubkey: Some(pubkey.to_string()),
                should_provision: false,
            }),
            Some(_) => Err("existing fulfillment key cannot be empty".to_string()),
            None => Ok(FulfillmentMetadata {
                fulfillment_pubkey: None,
                should_provision: true,
            }),
        },
    }
}

struct SdkSignerAdapter(Arc<dyn nostr::signer::NostrSigner>);

#[async_trait::async_trait]
impl NostrSigner for SdkSignerAdapter {
    async fn get_public_key(&self) -> Result<nostr::PublicKey, SignerError> {
        self.0
            .get_public_key()
            .await
            .map_err(|err| SignerError::SigningFailed(err.to_string()))
    }

    async fn sign_event(
        &self,
        unsigned: nostr::UnsignedEvent,
    ) -> Result<nostr::Event, SignerError> {
        self.0
            .sign_event(unsigned)
            .await
            .map_err(|err| SignerError::SigningFailed(err.to_string()))
    }
}

pub(crate) async fn resolve_active_signer(
    signer_state: &Arc<tokio::sync::Mutex<AppSignerState>>,
    auth: &AuthState,
) -> Result<Arc<dyn NostrSigner>, String> {
    let active_client = { signer_state.lock().await.active_client.clone() };
    if let Some(client) = active_client {
        let signer = client
            .signer()
            .await
            .map_err(|err| format!("failed to access active signer: {err}"))?;
        return Ok(Arc::new(SdkSignerAdapter(signer)));
    }

    auth.signer()
        .cloned()
        .map(|signer: ActiveSigner| Arc::new(signer) as Arc<dyn NostrSigner>)
        .ok_or_else(|| "not authenticated".to_string())
}

async fn ensure_publish_account_current(
    state: &State<'_, AppState>,
    signer_state: &Arc<tokio::sync::Mutex<AppSignerState>>,
    expected: nostr::PublicKey,
) -> Result<(), String> {
    let auth = { state.auth.lock().await.clone() };
    let current = resolve_active_signer(signer_state, &auth)
        .await?
        .get_public_key()
        .await
        .map_err(|error| error.to_string())?;
    if current != expected {
        return Err(
            "active account changed while fulfillment authorization was being prepared".into(),
        );
    }
    Ok(())
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

#[derive(Debug, Clone, Deserialize)]
pub struct ClaimEntitlementRequest {
    pub publisher_npub: String,
    pub listing_id: String,
    pub campaign_event_id: String,
    pub server_url: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ClaimEntitlementResponse {
    pub grant: nostr::Event,
    pub download_token: String,
    pub token_expires_at: i64,
    pub already_claimed: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DiscoverCampaignsRequest {
    pub publisher_npub: String,
    pub listing_id: String,
    pub pointers: Vec<CampaignPointerInput>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CampaignPointerInput {
    pub root_event_id: String,
    pub relay_hint: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DiscoverCampaignSummariesRequest {
    pub publisher_npub: String,
    pub listings: Vec<CampaignSummaryListingInput>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CampaignSummaryListingInput {
    pub listing_id: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct CampaignSummary {
    pub listing_id: String,
    pub active: usize,
    pub upcoming: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct DiscoveredCampaign {
    pub root_event_id: String,
    pub campaign_id: String,
    pub starts_at: u64,
    pub ends_at: u64,
    pub classification: String,
    pub event_id: Option<String>,
    pub predecessor_event_id: Option<String>,
    pub mode: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PublishCampaignRequest {
    pub publisher_npub: String,
    pub listing_id: String,
    pub campaign_id: String,
    pub starts_at: Option<u64>,
    pub ends_at: Option<u64>,
    pub predecessor_event_id: Option<String>,
    pub cancel: bool,
    pub update_listing_pointer: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct PublishCampaignResponse {
    pub event_id: String,
    pub root_event_id: String,
    pub listing_event_id: Option<String>,
    pub pointer_update_error: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct UpdateCampaignPointerRequest {
    pub publisher_npub: String,
    pub listing_id: String,
    pub campaign_root_id: String,
    pub remove: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct HashBuildFileRequest {
    pub file_path: String,
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
pub async fn discover_adp_servers(
    state: State<'_, AppState>,
) -> Result<Vec<AdpServerAnnouncement>, String> {
    let relay_manager = { state.nostr.lock().await.get_relay_manager().clone() };
    discover_adp_servers_core(&relay_manager)
        .await
        .map_err(|err| err.to_string())
}

#[tauri::command]
pub async fn discover_campaigns(
    request: DiscoverCampaignsRequest,
    state: State<'_, AppState>,
) -> Result<Vec<DiscoveredCampaign>, String> {
    let coordinate = listing_coordinate_from_npub(&request.publisher_npub, &request.listing_id)?;
    let publisher = nostr::PublicKey::from_bech32(&request.publisher_npub)
        .map_err(|_| "invalid publisher pubkey".to_string())?;
    let pointers = request
        .pointers
        .into_iter()
        .filter_map(|pointer| {
            nostr::EventId::from_hex(&pointer.root_event_id)
                .ok()
                .map(
                    |root_event_id| arcadestr_core::marketplace::CampaignPointer {
                        root_event_id,
                        relay_hint: pointer.relay_hint,
                    },
                )
        })
        .collect::<Vec<_>>();
    let relay_manager = { state.nostr.lock().await.get_relay_manager().clone() };
    let relay_manager = relay_manager.lock().await;
    let report = arcadestr_core::campaign_discovery::CampaignDiscoveryService::new(&relay_manager)
        .discover(&pointers, publisher, &coordinate, now_unix_i64()? as u64)
        .await
        .map_err(|error| error.to_string())?;

    Ok(report
        .campaigns
        .into_iter()
        .filter_map(|candidate| {
            let state = candidate.campaign.state_at(now_unix_i64().ok()? as u64)?;
            let tip = candidate.campaign.events.last();
            Some(DiscoveredCampaign {
                root_event_id: candidate.campaign.root_event_id.to_hex(),
                campaign_id: candidate.campaign.campaign_id,
                starts_at: state.terms.starts,
                ends_at: state.terms.ends,
                classification: match candidate.classification {
                    arcadestr_core::campaign_discovery::CampaignClassification::Upcoming => {
                        "upcoming"
                    }
                    arcadestr_core::campaign_discovery::CampaignClassification::Active => "active",
                    arcadestr_core::campaign_discovery::CampaignClassification::Ended => "ended",
                    arcadestr_core::campaign_discovery::CampaignClassification::Cancelled => {
                        "cancelled"
                    }
                    arcadestr_core::campaign_discovery::CampaignClassification::Invalid => {
                        "invalid"
                    }
                }
                .to_string(),
                event_id: tip.map(|node| node.event.id.to_hex()),
                predecessor_event_id: tip
                    .and_then(|node| node.predecessor)
                    .map(|event_id| event_id.to_hex()),
                mode: "claim".to_string(),
            })
        })
        .collect())
}

#[tauri::command]
pub async fn discover_campaign_summaries(
    request: DiscoverCampaignSummariesRequest,
    state: State<'_, AppState>,
) -> Result<Vec<CampaignSummary>, String> {
    let publisher = nostr::PublicKey::from_bech32(&request.publisher_npub)
        .map_err(|_| "invalid publisher pubkey".to_string())?;
    let relay_manager = { state.nostr.lock().await.get_relay_manager().clone() };
    let relay_manager = relay_manager.lock().await;
    let events = match relay_manager
        .fetch_events_best_effort(
            nostr::Filter::new()
                .kind(nostr::Kind::Custom(
                    arcadestr_core::adp_protocol::ADP_CAMPAIGN_KIND,
                ))
                .author(publisher),
        )
        .await
    {
        Ok(events) => events,
        Err(arcadestr_core::relay_manager::RelayManagerError::QueryTimeout) => Vec::new(),
        Err(error) => return Err(error.to_string()),
    };
    drop(relay_manager);
    let now = now_unix_i64()? as u64;

    request
        .listings
        .into_iter()
        .map(|listing| {
            let coordinate =
                listing_coordinate_from_npub(&request.publisher_npub, &listing.listing_id)?;
            let report = arcadestr_core::campaign_discovery::resolve_campaign_candidates_report(
                &[],
                &[],
                &events,
                &events,
                publisher,
                &coordinate,
                now,
            );
            Ok(CampaignSummary {
                listing_id: listing.listing_id,
                active: report
                    .campaigns
                    .iter()
                    .filter(|item| {
                        item.classification
                            == arcadestr_core::campaign_discovery::CampaignClassification::Active
                    })
                    .count(),
                upcoming: report
                    .campaigns
                    .iter()
                    .filter(|item| {
                        item.classification
                            == arcadestr_core::campaign_discovery::CampaignClassification::Upcoming
                    })
                    .count(),
            })
        })
        .collect()
}

#[tauri::command]
pub async fn publish_campaign(
    request: PublishCampaignRequest,
    state: State<'_, AppState>,
    signer_state: State<'_, Arc<tokio::sync::Mutex<AppSignerState>>>,
) -> Result<PublishCampaignResponse, String> {
    let auth_snapshot = { state.auth.lock().await.clone() };
    let signer = resolve_active_signer(signer_state.inner(), &auth_snapshot).await?;
    let publisher = signer
        .get_public_key()
        .await
        .map_err(|error| error.to_string())?;
    let expected_publisher = nostr::PublicKey::from_bech32(&request.publisher_npub)
        .map_err(|_| "invalid publisher pubkey".to_string())?;
    if publisher != expected_publisher {
        return Err("active signer is not the listing publisher".into());
    }
    let coordinate = listing_coordinate_from_npub(&request.publisher_npub, &request.listing_id)?;
    // Confirm the managed listing exists before publishing a campaign.
    fetch_listing_event_by_coordinate(&state, &coordinate).await?;
    let relay_manager = { state.nostr.lock().await.get_relay_manager().clone() };
    let manager = relay_manager.lock().await;
    let report = arcadestr_core::campaign_discovery::CampaignDiscoveryService::new(&manager)
        .discover(&[], publisher, &coordinate, now_unix_i64()? as u64)
        .await
        .map_err(|error| error.to_string())?;
    drop(manager);
    let existing = report
        .campaigns
        .into_iter()
        .find(|candidate| candidate.campaign.campaign_id == request.campaign_id);
    let predecessor = request
        .predecessor_event_id
        .as_deref()
        .map(nostr::EventId::from_hex)
        .transpose()
        .map_err(|_| "invalid predecessor event id".to_string())?;

    let params = if request.cancel {
        let predecessor =
            predecessor.ok_or_else(|| "cancellation requires predecessor".to_string())?;
        arcadestr_core::campaign::CampaignBuildParams::cancel(
            request.campaign_id.clone(),
            coordinate.clone(),
            predecessor,
        )
    } else {
        arcadestr_core::campaign::CampaignBuildParams::active(
            request.campaign_id.clone(),
            coordinate.clone(),
            request
                .starts_at
                .ok_or_else(|| "campaign start is required".to_string())?,
            request
                .ends_at
                .ok_or_else(|| "campaign end is required".to_string())?,
            predecessor,
        )
    };
    if predecessor.is_none() && existing.is_some() {
        return Err("campaign id already exists".into());
    }
    let builder = arcadestr_core::campaign::build_campaign_event_builder(&params)
        .map_err(|error| error.to_string())?;
    let event = signer
        .sign_event(builder.build(publisher))
        .await
        .map_err(|error| error.to_string())?;

    let (root_event_id, mut prospective) = if let Some(existing) = existing {
        let expected_tip = existing
            .campaign
            .events
            .last()
            .map(|node| node.event.id)
            .ok_or_else(|| "campaign chain is empty".to_string())?;
        if predecessor != Some(expected_tip) {
            return Err("predecessor is not the current campaign tip".into());
        }
        (existing.campaign.root_event_id, existing.campaign.events)
    } else {
        (event.id, Vec::new())
    };
    prospective.push(
        arcadestr_core::campaign::parse_campaign_event(&event)
            .map_err(|error| error.to_string())?,
    );
    arcadestr_core::campaign::resolve_campaign(&prospective, publisher, &coordinate)
        .map_err(|error| error.to_string())?;
    publish_event(&state, &event).await?;

    let (listing_event_id, pointer_update_error) = if request.update_listing_pointer {
        let pointer_result: Result<String, String> = async {
            // Campaign publication can involve several relay and signing awaits. Merge the
            // pointer into the latest listing instead of overwriting a concurrent edit.
            let listing = fetch_listing_event_by_coordinate(&state, &coordinate).await?;
            let mut tags = listing
                .tags
                .iter()
                .filter(|tag| {
                    let values = (*tag).clone().to_vec();
                    values.first().map(String::as_str) != Some("campaign")
                        || values.get(1).map(String::as_str)
                            != Some(root_event_id.to_hex().as_str())
                })
                .cloned()
                .collect::<Vec<_>>();
            if !request.cancel {
                tags.push(
                    nostr::Tag::parse(["campaign", root_event_id.to_hex().as_str()])
                        .map_err(|error| error.to_string())?,
                );
            }
            let listing_update = signer
                .sign_event(
                    nostr::EventBuilder::new(nostr::Kind::Custom(30402), listing.content.clone())
                        .tags(tags)
                        .build(publisher),
                )
                .await
                .map_err(|error| error.to_string())?;
            publish_event(&state, &listing_update).await?;
            Ok(listing_update.id.to_hex())
        }
        .await;
        match pointer_result {
            Ok(event_id) => (Some(event_id), None),
            Err(error) => (None, Some(error)),
        }
    } else {
        (None, None)
    };

    Ok(PublishCampaignResponse {
        event_id: event.id.to_hex(),
        root_event_id: root_event_id.to_hex(),
        listing_event_id,
        pointer_update_error,
    })
}

#[tauri::command]
pub async fn update_campaign_pointer(
    request: UpdateCampaignPointerRequest,
    state: State<'_, AppState>,
    signer_state: State<'_, Arc<tokio::sync::Mutex<AppSignerState>>>,
) -> Result<String, String> {
    let auth_snapshot = { state.auth.lock().await.clone() };
    let signer = resolve_active_signer(signer_state.inner(), &auth_snapshot).await?;
    let publisher = signer
        .get_public_key()
        .await
        .map_err(|error| error.to_string())?;
    let expected = nostr::PublicKey::from_bech32(&request.publisher_npub)
        .map_err(|_| "invalid publisher pubkey".to_string())?;
    if publisher != expected {
        return Err("active signer is not the listing publisher".into());
    }
    let coordinate = listing_coordinate_from_npub(&request.publisher_npub, &request.listing_id)?;
    let listing = fetch_listing_event_by_coordinate(&state, &coordinate).await?;
    let root = nostr::EventId::from_hex(&request.campaign_root_id)
        .map_err(|_| "invalid campaign root event id".to_string())?;
    let mut tags = listing
        .tags
        .iter()
        .filter(|tag| {
            let values = (*tag).clone().to_vec();
            values.first().map(String::as_str) != Some("campaign")
                || values.get(1).map(String::as_str) != Some(root.to_hex().as_str())
        })
        .cloned()
        .collect::<Vec<_>>();
    if !request.remove {
        tags.push(
            nostr::Tag::parse(["campaign", root.to_hex().as_str()])
                .map_err(|error| error.to_string())?,
        );
    }
    let updated = signer
        .sign_event(
            nostr::EventBuilder::new(nostr::Kind::Custom(30402), listing.content)
                .tags(tags)
                .build(publisher),
        )
        .await
        .map_err(|error| error.to_string())?;
    publish_event(&state, &updated).await?;
    Ok(updated.id.to_hex())
}

#[tauri::command]
pub async fn hash_build_file(
    app: tauri::AppHandle,
    request: HashBuildFileRequest,
) -> Result<String, String> {
    sha256_file_with_progress(
        std::path::Path::new(&request.file_path),
        |bytes_hashed, total_bytes| {
            let _ = app.emit(
                "hash-progress",
                HashProgressPayload {
                    bytes_hashed,
                    total_bytes,
                },
            );
        },
    )
    .await
    .map_err(|err| err.to_string())
}

#[tauri::command]
pub async fn select_build_file<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
) -> Result<Option<String>, String> {
    let path = app
        .dialog()
        .file()
        .blocking_pick_file()
        .map(|path| path.to_string());
    Ok(path)
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
    let connection =
        save_default_nwc_connection(&request.connection_string).map_err(|err| err.to_string())?;
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

    let game_coordinate =
        listing_coordinate_from_npub(&request.publisher_npub, &request.listing_id)?;
    let listing_event = fetch_listing_event_by_coordinate(&state, &game_coordinate).await?;
    let listing_for_validation = listing_event.clone();
    let bolt11 = request.bolt11.clone();
    let preimage = request.preimage.clone();
    let client = AdpClient::new(request.server_url.clone(), Arc::clone(&state.http_client));
    let response = client
        .purchase_confirm(
            signer,
            CorePurchaseConfirmRequest {
                game_coordinate: game_coordinate.clone(),
                listing_event,
                zap_receipt_event: None,
                bolt11: Some(request.bolt11),
                preimage: Some(request.preimage),
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
        &bolt11,
        &preimage,
    )
    .await
}

#[tauri::command]
pub async fn claim_entitlement(
    request: ClaimEntitlementRequest,
    state: State<'_, AppState>,
) -> Result<ClaimEntitlementResponse, String> {
    let auth_snapshot = { state.auth.lock().await.clone() };
    let signer = auth_snapshot
        .signer()
        .ok_or_else(|| "not authenticated".to_string())?;
    let buyer = signer
        .get_public_key()
        .await
        .map_err(|error| error.to_string())?;
    let game_coordinate =
        listing_coordinate_from_npub(&request.publisher_npub, &request.listing_id)?;
    let campaign_root_id = nostr::EventId::from_hex(&request.campaign_event_id)
        .map_err(|_| "invalid campaign event id".to_string())?;
    let listing = fetch_listing_event_by_coordinate(&state, &game_coordinate).await?;
    let publisher = listing.pubkey;
    let pointer = arcadestr_core::marketplace::CampaignPointer {
        root_event_id: campaign_root_id,
        relay_hint: None,
    };
    let relay_manager = { state.nostr.lock().await.get_relay_manager().clone() };
    let relay_manager_guard = relay_manager.lock().await;
    let report =
        arcadestr_core::campaign_discovery::CampaignDiscoveryService::new(&relay_manager_guard)
            .discover(
                &[pointer],
                publisher,
                &game_coordinate,
                now_unix_i64()? as u64,
            )
            .await
            .map_err(|error| error.to_string())?;
    drop(relay_manager_guard);
    let campaign = report
        .campaigns
        .into_iter()
        .find(|candidate| candidate.campaign.root_event_id == campaign_root_id)
        .ok_or_else(|| "campaign is invalid or unavailable".to_string())?
        .campaign;

    let client = AdpClient::new(request.server_url.clone(), Arc::clone(&state.http_client));
    let response = client
        .entitlement_claim(
            signer,
            CoreEntitlementClaimRequest {
                game_coordinate: game_coordinate.clone(),
                campaign_event_id: request.campaign_event_id,
            },
        )
        .await
        .map_err(|error| error.to_string())?;

    let parsed = arcadestr_core::entitlements::parse_entitlement_event(&response.grant)
        .map_err(|error| format!("invalid grant response: {error}"))?;
    if parsed.recipient != buyer {
        return Err("invalid grant response: recipient mismatch".into());
    }
    if parsed.coordinate != game_coordinate {
        return Err("invalid grant response: coordinate mismatch".into());
    }
    if parsed.source_event != campaign_root_id {
        return Err("invalid grant response: campaign source mismatch".into());
    }
    let grant = arcadestr_core::entitlements::resolve_entitlement_grant(&[parsed.clone()])
        .map_err(|error| format!("invalid grant response: {error}"))?;
    let authorization = if let Some(root_event_id) = parsed.authorization {
        let relay_manager_guard = relay_manager.lock().await;
        Some(
            arcadestr_core::authorization::discover_authorization(
                &relay_manager_guard,
                root_event_id,
                publisher,
            )
            .await
            .map_err(|error| format!("invalid grant authorization: {error}"))?,
        )
    } else if parsed.event.pubkey == publisher {
        None
    } else {
        return Err("invalid grant response: delegated grant has no authorization event".into());
    };
    arcadestr_core::entitlements::validate_adp_entitlement(
        &grant,
        &campaign,
        authorization.as_ref(),
    )
    .map_err(|error| format!("invalid grant response: {error}"))?;
    let buyer_hex = buyer.to_hex();
    if state
        .auth
        .lock()
        .await
        .public_key()
        .map(|key| key.to_hex())
        .as_deref()
        != Some(buyer_hex.as_str())
    {
        return Err("active account changed while grant authorization was being verified".into());
    }

    let entitlements = arcadestr_core::entitlements_repository::EntitlementsRepository::new(
        state.database.pool().clone(),
    );
    entitlements
        .ingest_event(&response.grant, &campaign, authorization.as_ref())
        .await
        .map_err(|error| error.to_string())?;
    DownloadTokensRepository::new(state.database.pool().clone())
        .upsert(&DownloadToken {
            buyer_pubkey: buyer_hex,
            game_coordinate: game_coordinate,
            server_url: request.server_url,
            token: response.download_token.clone(),
            expires_at: response.token_expires_at,
        })
        .await
        .map_err(|error| error.to_string())?;

    Ok(ClaimEntitlementResponse {
        grant: response.grant,
        download_token: response.download_token,
        token_expires_at: response.token_expires_at,
        already_claimed: response.already_claimed,
    })
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

pub(crate) async fn fetch_listing_event_by_coordinate(
    state: &State<'_, AppState>,
    coordinate: &str,
) -> Result<nostr::Event, String> {
    let mut parts = coordinate.splitn(3, ':');
    let kind = parts
        .next()
        .ok_or_else(|| "missing coordinate kind".to_string())?;
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
    let relay_client = {
        let relay_manager = { state.nostr.lock().await.get_relay_manager().clone() };
        let manager = relay_manager.lock().await;
        manager.get_client_arc()
    };
    let timeout_duration = std::time::Duration::from_secs(10);
    let events = tokio::time::timeout(
        timeout_duration,
        relay_client.fetch_events(
            nostr::Filter::new()
                .kind(nostr::Kind::Custom(30402))
                .author(developer_pubkey)
                .identifier(d_tag),
            timeout_duration,
        ),
    )
    .await
    .map_err(|_| "listing event fetch timed out".to_string())?
    .map_err(|err| err.to_string())?;
    select_replaceable_event(events).ok_or_else(|| "listing event not found on relays".to_string())
}

fn select_replaceable_event(
    events: impl IntoIterator<Item = nostr::Event>,
) -> Option<nostr::Event> {
    events.into_iter().reduce(|current, candidate| {
        let candidate_id = candidate.id.to_hex();
        let current_id = current.id.to_hex();
        if is_replaceable_event_newer(
            candidate.created_at.as_secs(),
            Some(&candidate_id),
            current.created_at.as_secs(),
            Some(&current_id),
        ) {
            candidate
        } else {
            current
        }
    })
}

fn reusable_file_hash(existing_file_hash: Option<&str>) -> Result<String, String> {
    let hash = existing_file_hash.ok_or_else(|| {
        "build file or existing file hash is required for fulfillment".to_string()
    })?;
    if !is_sha256_hex(hash) {
        return Err("existing file hash is invalid; select a replacement build file".to_string());
    }
    Ok(hash.to_string())
}

async fn persist_purchase_confirmation(
    state: &State<'_, AppState>,
    buyer_pubkey: &str,
    game_coordinate: &str,
    server_url: &str,
    response: CorePurchaseConfirmResponse,
    listing_event: &nostr::Event,
    bolt11: &str,
    preimage: &str,
) -> Result<ConfirmPurchaseResponse, String> {
    let parsed = arcadestr_core::purchases::parse_receipt_event(&response.receipt)
        .map_err(|error| error.to_string())?;
    let authorization = if response.receipt.pubkey == listing_event.pubkey {
        None
    } else {
        let root = parsed
            .authorization
            .ok_or_else(|| "delegated receipt is missing authorization".to_string())?;
        let relay_manager = { state.nostr.lock().await.get_relay_manager().clone() };
        let relay_manager = relay_manager.lock().await;
        Some(
            arcadestr_core::authorization::discover_authorization(
                &relay_manager,
                root,
                listing_event.pubkey,
            )
            .await
            .map_err(|error| format!("receipt authorization unavailable or invalid: {error}"))?,
        )
    };
    let receipt = arcadestr_core::purchases::parse_and_validate_receipt_with_authorization(
        &response.receipt,
        buyer_pubkey,
        authorization.as_ref(),
        arcadestr_core::purchases::ReceiptEvidence {
            bolt11: Some(bolt11),
            preimage: Some(preimage),
            zap_receipts: &[],
            lsp_pubkey: None,
        },
    )
    .map_err(|err| err.to_string())?;
    let current_buyer = state.auth.lock().await.public_key().map(|key| key.to_hex());
    if current_buyer.as_deref() != Some(buyer_pubkey) {
        return Err("active account changed while receipt authorization was being verified".into());
    }
    state
        .purchases
        .upsert_receipt(&receipt)
        .await
        .map_err(|err| err.to_string())?;
    let tokens = DownloadTokensRepository::new(state.database.pool().clone());
    tokens
        .upsert(&DownloadToken {
            buyer_pubkey: buyer_pubkey.to_string(),
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

fn publisher_hex_from_npub(publisher_npub: &str) -> Result<String, String> {
    nostr::PublicKey::from_bech32(publisher_npub)
        .map(|publisher| publisher.to_hex())
        .map_err(|err| format!("invalid publisher npub: {err}"))
}

fn verify_expected_publisher(
    expected_publisher_npub: &str,
    signer_pubkey: nostr::PublicKey,
) -> Result<(), String> {
    let expected = nostr::PublicKey::from_bech32(expected_publisher_npub)
        .map_err(|err| format!("invalid expected publisher npub: {err}"))?;
    if expected != signer_pubkey {
        return Err(
            "Active account changed before publication. Nothing was published; review the form and publish again with the intended account. (expected publisher does not match signer pubkey)"
                .to_string(),
        );
    }
    Ok(())
}

fn is_publish_form_tag(values: &[String]) -> bool {
    matches!(
        values.first().map(String::as_str),
        Some(
            "d" | "title"
                | "price"
                | "t"
                | "image"
                | "acquisition"
                | "server"
                | "file_hash"
                | "version"
                | "fulfillment_pubkey"
                | "lud16"
                | "platform"
        )
    )
}

fn unique_operator_url(matches: &[AdpProvisioning]) -> Option<String> {
    match matches {
        [entry] => Some(entry.server_url.clone()),
        _ => None,
    }
}

#[tauri::command]
pub async fn resolve_adp_operator(
    request: ResolveAdpOperatorRequest,
    state: State<'_, AppState>,
) -> Result<Option<String>, String> {
    let publisher_hex = publisher_hex_from_npub(&request.publisher_npub)?;
    let provisioning_repo = AdpProvisioningRepository::new(state.database.pool().clone());
    let matches = provisioning_repo
        .for_fulfillment_scope(&publisher_hex, &request.fulfillment_pubkey, &request.scope)
        .await
        .map_err(|err| err.to_string())?;

    Ok(unique_operator_url(&matches))
}

#[tauri::command]
pub async fn publish_adp_listing<R: tauri::Runtime>(
    request: PublishAdpListingRequest,
    app: tauri::AppHandle<R>,
    state: State<'_, AppState>,
    signer_state: State<'_, Arc<tokio::sync::Mutex<AppSignerState>>>,
) -> Result<PublishAdpListingResult, String> {
    let auth_snapshot = { state.auth.lock().await.clone() };
    let signer = resolve_active_signer(signer_state.inner(), &auth_snapshot).await?;
    let developer_pubkey = signer
        .get_public_key()
        .await
        .map_err(|err| err.to_string())?;
    verify_expected_publisher(&request.expected_publisher_npub, developer_pubkey)?;
    let developer_npub = developer_pubkey.to_hex();
    let preserving_existing_listing = request.existing_event_id.is_some();
    let preserved_listing_tags = if let Some(expected_event_id) =
        request.existing_event_id.as_deref()
    {
        let coordinate = format!("30402:{developer_npub}:{}", request.d_tag);
        let existing = fetch_listing_event_by_coordinate(&state, &coordinate).await?;
        if existing.id.to_hex() != expected_event_id {
            return Err(
                "This Game page changed since it was opened. Reload it before publishing so newer metadata is not overwritten."
                    .to_string(),
            );
        }
        existing
            .tags
            .iter()
            .map(|tag| tag.clone().to_vec())
            .filter(|values| !is_publish_form_tag(values))
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    };
    let now: u64 = now_unix_i64()?
        .try_into()
        .map_err(|_| "current time is negative".to_string())?;
    let FulfillmentMetadata {
        fulfillment_pubkey: mut fulfillment_pubkey,
        should_provision,
    } = resolve_existing_fulfillment_metadata(
        &request.fulfillment_mode,
        &developer_npub,
        request.existing_fulfillment_pubkey.as_deref(),
    )?;

    let mut file_hash = None;
    let mut acceptance_event_id = None;

    match request.fulfillment_mode {
        FulfillmentMode::None => {}
        FulfillmentMode::Direct => {
            if request.servers.is_empty() {
                return Err(
                    "at least one distribution server is required for fulfillment".to_string(),
                );
            }
            let version = request
                .version
                .as_deref()
                .ok_or_else(|| "version is required for fulfillment".to_string())?;
            if version.trim().is_empty() {
                return Err("version is required for fulfillment".to_string());
            }

            let hash = if let Some(file_path) = request.file_path.as_deref() {
                emit_progress(&app, "hash-file", "pending", None)?;
                let hash = sha256_file(std::path::Path::new(file_path))
                    .await
                    .map_err(|err| progress_error(&app, "hash-file", err))?;
                emit_progress(&app, "hash-file", "ok", Some(hash.clone()))?;
                hash
            } else {
                reusable_file_hash(request.existing_file_hash.as_deref())?
            };
            file_hash = Some(hash);
        }
        FulfillmentMode::Delegate => {
            if request.servers.is_empty() {
                return Err(
                    "at least one distribution server is required for fulfillment".to_string(),
                );
            }
            let version = request
                .version
                .as_deref()
                .ok_or_else(|| "version is required for fulfillment".to_string())?;
            if version.trim().is_empty() {
                return Err("version is required for fulfillment".to_string());
            }

            let hash = if let Some(file_path) = request.file_path.as_deref() {
                emit_progress(&app, "hash-file", "pending", None)?;
                let hash = sha256_file(std::path::Path::new(file_path))
                    .await
                    .map_err(|err| progress_error(&app, "hash-file", err))?;
                emit_progress(&app, "hash-file", "ok", Some(hash.clone()))?;
                hash
            } else {
                reusable_file_hash(request.existing_file_hash.as_deref())?
            };
            file_hash = Some(hash);

            if should_provision {
                let operator_url = request.operator_url.as_deref().ok_or_else(|| {
                    "operator URL is required for delegated fulfillment".to_string()
                })?;
                if operator_url.trim().is_empty() {
                    return Err("operator URL is required for delegated fulfillment".to_string());
                }
                emit_progress(
                    &app,
                    "check-operator",
                    "pending",
                    Some(operator_url.to_string()),
                )?;
                let adp_client =
                    AdpClient::new(operator_url.to_string(), Arc::clone(&state.http_client));
                let server_info = adp_client
                    .well_known()
                    .await
                    .map_err(|err| progress_error(&app, "check-operator", err))?;
                emit_progress(
                    &app,
                    "check-operator",
                    "ok",
                    Some(server_info.pubkey.clone()),
                )?;

                let provisioning_repo =
                    AdpProvisioningRepository::new(state.database.pool().clone());
                let scope = request.d_tag.as_str();

                emit_progress(&app, "provision", "pending", None)?;
                let provisioning = resolve_provisioning(ResolveProvisioningInput {
                    provisioning_repo: &provisioning_repo,
                    adp_client: &adp_client,
                    signer: signer.as_ref(),
                    developer_pubkey,
                    developer_npub: &developer_npub,
                    server_url: operator_url,
                    scope,
                    server_info: &server_info,
                })
                .await
                .map_err(|err| progress_error(&app, "provision", err))?;

                match provisioning {
                    ProvisioningDecision::Reused {
                        fulfillment_pubkey: reused_pubkey,
                        authorization_event_id,
                        valid_from: _,
                        authorization_event,
                        attestation,
                        row,
                    } => {
                        validate_current_attestation(&state, &row, attestation.as_deref(), now)
                            .await?;
                        ensure_publish_account_current(
                            &state,
                            signer_state.inner(),
                            developer_pubkey,
                        )
                        .await?;
                        provisioning_repo
                            .upsert(&row)
                            .await
                            .map_err(|err| progress_error(&app, "provision", err))?;
                        publish_event(&state, &authorization_event)
                            .await
                            .map_err(|err| progress_error(&app, "provision", err))?;
                        emit_progress(
                            &app,
                            "provision",
                            "ok",
                            Some("reused existing provisioning".into()),
                        )?;
                        fulfillment_pubkey = Some(reused_pubkey);
                        acceptance_event_id = Some(authorization_event_id);
                    }
                    ProvisioningDecision::Created {
                        fulfillment_pubkey: created_pubkey,
                        authorization_event_id,
                        valid_from: _,
                        authorization_event,
                        attestation,
                        row,
                    } => {
                        validate_current_attestation(&state, &row, attestation.as_deref(), now)
                            .await?;
                        ensure_publish_account_current(
                            &state,
                            signer_state.inner(),
                            developer_pubkey,
                        )
                        .await?;
                        provisioning_repo
                            .upsert(&row)
                            .await
                            .map_err(|err| progress_error(&app, "provision", err))?;
                        publish_event(&state, &authorization_event)
                            .await
                            .map_err(|err| progress_error(&app, "provision", err))?;
                        emit_progress(
                            &app,
                            "provision",
                            "ok",
                            Some("created provisioning".into()),
                        )?;
                        fulfillment_pubkey = Some(created_pubkey);
                        acceptance_event_id = Some(authorization_event_id);
                    }
                }
            } else if let Some(existing_pubkey) =
                existing_authorization_repair_key(should_provision, fulfillment_pubkey.as_deref())
            {
                let operator_url = request.operator_url.as_deref().ok_or_else(|| {
                    "operator URL is required to repair delegated fulfillment authorization"
                        .to_string()
                })?;
                if operator_url.trim().is_empty() {
                    return Err(
                        "operator URL is required to repair delegated fulfillment authorization"
                            .to_string(),
                    );
                }

                let provisioning_repo =
                    AdpProvisioningRepository::new(state.database.pool().clone());
                let adp_client =
                    AdpClient::new(operator_url.to_string(), Arc::clone(&state.http_client));
                let provision = adp_client
                    .provision(signer.as_ref(), Some(&request.d_tag))
                    .await
                    .map_err(|error| progress_error(&app, "provision", error))?;
                let repair = repair_existing_authorization(ExistingAuthorizationRepairInput {
                    provisioning_repo: &provisioning_repo,
                    signer: signer.as_ref(),
                    developer_pubkey,
                    developer_hex: &developer_npub,
                    operator_url,
                    scope: &request.d_tag,
                    fulfillment_pubkey: existing_pubkey,
                    provision: &provision,
                })
                .await?;
                validate_current_attestation(
                    &state,
                    &repair.row,
                    repair.attestation.as_deref(),
                    now,
                )
                .await?;
                ensure_publish_account_current(&state, signer_state.inner(), developer_pubkey)
                    .await?;
                provisioning_repo
                    .upsert(&repair.row)
                    .await
                    .map_err(|err| err.to_string())?;
                publish_event(&state, &repair.authorization_event).await?;
                acceptance_event_id = Some(repair.authorization_event_id);
            }
        }
    }

    if let Some(expected_event_id) = request.existing_event_id.as_deref() {
        let coordinate = format!("30402:{developer_npub}:{}", request.d_tag);
        let current = fetch_listing_event_by_coordinate(&state, &coordinate).await?;
        if current.id.to_hex() != expected_event_id {
            return Err(
                "This Game page changed while publishing authorization was prepared. Reload it before publishing so newer metadata is not overwritten."
                    .to_string(),
            );
        }
    }

    emit_progress(&app, "publish-listing", "pending", None)?;
    ensure_publish_account_current(&state, signer_state.inner(), developer_pubkey).await?;
    let listing_input = AdpListingInput {
        d_tag: request.d_tag.clone(),
        title: request.title.clone(),
        description: request.description.clone(),
        price_sats: request.price_sats,
        lud16: request.lud16.clone(),
        tags: request.tags.clone(),
        images: request.images.clone(),
        servers: request.servers.clone(),
        file_hash: file_hash.clone(),
        version: request.version.clone(),
        fulfillment_authorizations: if matches!(request.fulfillment_mode, FulfillmentMode::Delegate)
        {
            acceptance_event_id
                .as_ref()
                .zip(fulfillment_pubkey.as_ref())
                .map(|(root_event_id, fulfillment_pubkey)| {
                    vec![FulfillmentAuthorizationInput {
                        root_event_id: root_event_id.clone(),
                        fulfillment_pubkey: fulfillment_pubkey.clone(),
                        relay_hint: None,
                    }]
                })
                .unwrap_or_default()
        } else {
            Vec::new()
        },
        acquisition: request.acquisition.clone(),
        platforms: request.platforms.clone(),
        campaigns: request
            .campaigns
            .iter()
            .filter(|_| !preserving_existing_listing)
            .map(|pointer| (pointer.root_event_id.clone(), pointer.relay_hint.clone()))
            .collect(),
        nip94_event_id: (!preserving_existing_listing)
            .then(|| request.nip94_event_id.clone())
            .flatten(),
        preserved_tags: preserved_listing_tags,
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

    let mut uploads = Vec::new();
    if !matches!(request.fulfillment_mode, FulfillmentMode::None) && request.file_path.is_some() {
        let file_path = request.file_path.as_deref().ok_or_else(|| {
            "build file is required when uploading a replacement artifact".to_string()
        })?;
        let mut upload_errors = Vec::new();
        for server_url in &request.servers {
            emit_server_progress(&app, "upload", "pending", Some(server_url.clone()), None)?;
            let adp_client = AdpClient::new(server_url.clone(), Arc::clone(&state.http_client));
            let progress_app = app.clone();
            let progress_server_url = server_url.clone();
            let last_reported_percent = Arc::new(AtomicU64::new(u64::MAX));
            match adp_client
                .upload_with_progress(
                    signer.as_ref(),
                    &listing_event,
                    std::path::Path::new(file_path),
                    move |bytes_uploaded, total_bytes| {
                        let percent = if total_bytes == 0 {
                            100
                        } else {
                            bytes_uploaded.saturating_mul(100) / total_bytes
                        };
                        if last_reported_percent.swap(percent, Ordering::Relaxed) != percent {
                            let _ = emit_upload_progress(
                                &progress_app,
                                &progress_server_url,
                                bytes_uploaded,
                                total_bytes,
                            );
                        }
                    },
                )
                .await
            {
                Ok(upload) => {
                    emit_server_progress(
                        &app,
                        "upload",
                        "ok",
                        Some(server_url.clone()),
                        Some(upload.download_url.clone()),
                    )?;
                    uploads.push(PublishServerUploadResult {
                        server_url: server_url.clone(),
                        status: "ok".to_string(),
                        error: None,
                        upload: Some(upload),
                    });
                }
                Err(err) => {
                    let message = err.to_string();
                    emit_server_progress(
                        &app,
                        "upload",
                        "error",
                        Some(server_url.clone()),
                        Some(message.clone()),
                    )?;
                    upload_errors.push(format!("{server_url}: {message}"));
                    uploads.push(PublishServerUploadResult {
                        server_url: server_url.clone(),
                        status: "error".to_string(),
                        error: Some(message),
                        upload: None,
                    });
                }
            }
        }

        if !upload_errors.is_empty() {
            return Err(format!("upload failed for: {}", upload_errors.join("; ")));
        }
    }

    Ok(PublishAdpListingResult {
        event_id: listing_event.id.to_hex(),
        acceptance_event_id,
        fulfillment_pubkey,
        file_hash,
        uploads,
    })
}

async fn validate_current_attestation(
    state: &State<'_, AppState>,
    row: &AdpProvisioning,
    provided: Option<&nostr::Event>,
    now: u64,
) -> Result<(), String> {
    let operator = nostr::PublicKey::from_hex(&row.operator_pubkey)
        .map_err(|_| "operator pubkey is invalid".to_string())?;
    let d = format!("{}:{}", row.developer_npub, row.fulfillment_pubkey);
    let attestation = if let Some(attestation) = provided {
        if attestation.id.to_hex() != row.attestation_event_id {
            return Err("operator attestation evidence ID mismatch".into());
        }
        attestation.clone()
    } else {
        let relay_manager = { state.nostr.lock().await.get_relay_manager().clone() };
        let relay_manager = relay_manager.lock().await;
        let events = relay_manager
            .fetch_events_best_effort(
                nostr::Filter::new()
                    .kind(nostr::Kind::Custom(30404))
                    .author(operator),
            )
            .await
            .map_err(|error| format!("operator attestation evidence unavailable: {error}"))?;
        select_replaceable_event(events.into_iter().filter(|event| {
            event
                .tags
                .iter()
                .any(|tag| matches!(tag.as_slice(), [name, value] if name == "d" && value == &d))
        }))
        .ok_or_else(|| "operator attestation evidence unavailable".to_string())?
    };
    let parsed = arcadestr_core::authorization::parse_attestation_event(&attestation)
        .map_err(|error| format!("invalid operator attestation: {error}"))?;
    if attestation.pubkey != operator
        || parsed.developer_pubkey.to_hex() != row.developer_npub
        || parsed.fulfillment_pubkey.to_hex() != row.fulfillment_pubkey
        || parsed.scope.as_deref() != row.scope.as_deref()
        || !parsed.allows_new_operations_at(now)
    {
        return Err("operator no longer attests possession of this fulfillment key".into());
    }
    Ok(())
}

enum ProvisioningDecision {
    Reused {
        fulfillment_pubkey: String,
        authorization_event_id: String,
        valid_from: i64,
        authorization_event: Box<nostr::Event>,
        attestation: Option<Box<nostr::Event>>,
        row: Box<AdpProvisioning>,
    },
    Created {
        fulfillment_pubkey: String,
        authorization_event_id: String,
        valid_from: i64,
        authorization_event: Box<nostr::Event>,
        attestation: Option<Box<nostr::Event>>,
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

struct ExistingAuthorizationRepairInput<'a> {
    provisioning_repo: &'a AdpProvisioningRepository,
    signer: &'a dyn NostrSigner,
    developer_pubkey: nostr::PublicKey,
    developer_hex: &'a str,
    operator_url: &'a str,
    scope: &'a str,
    fulfillment_pubkey: &'a str,
    provision: &'a ProvisionResponse,
}

struct ExistingAuthorizationRepair {
    authorization_event_id: String,
    authorization_event: nostr::Event,
    attestation: Option<Box<nostr::Event>>,
    row: AdpProvisioning,
}

fn existing_authorization_repair_key(
    should_provision: bool,
    fulfillment_pubkey: Option<&str>,
) -> Option<&str> {
    (!should_provision).then_some(fulfillment_pubkey).flatten()
}

async fn repair_existing_authorization(
    input: ExistingAuthorizationRepairInput<'_>,
) -> Result<ExistingAuthorizationRepair, String> {
    if input.provision.fulfillment_pubkey != input.fulfillment_pubkey {
        return Err(
            "distribution provider returned a different active fulfillment key; reload the listing before publishing"
                .into(),
        );
    }
    let matches = input
        .provisioning_repo
        .for_fulfillment_scope(input.developer_hex, input.fulfillment_pubkey, input.scope)
        .await
        .map_err(|err| err.to_string())?
        .into_iter()
        .filter(|row| {
            row.revoked_at.is_none()
                && row.server_url == input.operator_url
                && row.authorization_profile_version >= 2
                && row.authorization_root_event_id.is_some()
        })
        .collect::<Vec<_>>();
    let [mut row] = matches.try_into().map_err(|matches: Vec<AdpProvisioning>| {
        format!(
            "expected exactly one active provisioning row for delegated authorization repair, found {}",
            matches.len()
        )
    })?;
    let valid_from: u64 = now_unix_i64()?
        .try_into()
        .map_err(|_| "current time is negative".to_string())?;
    let coordinate = format!("30402:{}:{}", input.developer_pubkey.to_hex(), input.scope);
    let terms = AuthorizationTerms {
        authorization_id: uuid::Uuid::new_v4().to_string(),
        coordinate,
        operator_pubkey: nostr::PublicKey::from_hex(&row.operator_pubkey)
            .map_err(|_| "stored operator pubkey is invalid".to_string())?,
        fulfillment_pubkey: nostr::PublicKey::from_hex(input.fulfillment_pubkey)
            .map_err(|_| "stored fulfillment pubkey is invalid".to_string())?,
        capabilities: BTreeSet::from([
            CAPABILITY_ISSUE_RECEIPT.into(),
            CAPABILITY_ISSUE_GRANT.into(),
            CAPABILITY_UPLOAD_BUILD.into(),
        ]),
        valid_from,
    };
    let builder =
        build_fulfillment_authorization_event_builder(&terms).map_err(|error| error.to_string())?;
    let authorization_event = input
        .signer
        .sign_event(builder.build(input.developer_pubkey))
        .await
        .map_err(|err| err.to_string())?;
    let authorization_event_id = authorization_event.id.to_hex();
    row.acceptance_event_id = authorization_event_id.clone();
    row.valid_from = valid_from
        .try_into()
        .map_err(|_| "authorization valid_from exceeds storage range".to_string())?;
    row.authorization_root_event_id = Some(authorization_event_id.clone());
    row.authorization_capabilities = terms.capabilities.into_iter().collect();
    row.authorization_profile_version = 2;
    row.attestation_event_id = input.provision.attestation_event_id.clone();

    Ok(ExistingAuthorizationRepair {
        authorization_event_id,
        authorization_event,
        attestation: input.provision.attestation.clone().map(Box::new),
        row,
    })
}

async fn resolve_provisioning(
    input: ResolveProvisioningInput<'_>,
) -> Result<ProvisioningDecision, String> {
    let listing_coordinate = format!("30402:{}:{}", input.developer_pubkey.to_hex(), input.scope);
    let provision = input
        .adp_client
        .provision(input.signer, Some(input.scope))
        .await
        .map_err(|err| err.to_string())?;

    if let Some(existing) = input
        .provisioning_repo
        .active_for_scope(input.developer_npub, input.server_url, Some(input.scope))
        .await
        .map_err(|err| err.to_string())?
        .filter(|existing| {
            existing.fulfillment_pubkey == provision.fulfillment_pubkey
                && existing.attestation_event_id == provision.attestation_event_id
        })
    {
        let valid_from: u64 = now_unix_i64()?
            .try_into()
            .map_err(|_| "current time is negative".to_string())?;
        let terms = AuthorizationTerms {
            authorization_id: uuid::Uuid::new_v4().to_string(),
            coordinate: listing_coordinate.clone(),
            operator_pubkey: nostr::PublicKey::from_hex(&existing.operator_pubkey)
                .map_err(|_| "stored operator pubkey is invalid".to_string())?,
            fulfillment_pubkey: nostr::PublicKey::from_hex(&existing.fulfillment_pubkey)
                .map_err(|_| "stored fulfillment pubkey is invalid".to_string())?,
            capabilities: BTreeSet::from([
                CAPABILITY_ISSUE_RECEIPT.into(),
                CAPABILITY_ISSUE_GRANT.into(),
                CAPABILITY_UPLOAD_BUILD.into(),
            ]),
            valid_from,
        };
        let authorization_builder = build_fulfillment_authorization_event_builder(&terms)
            .map_err(|error| error.to_string())?;
        let authorization_event = input
            .signer
            .sign_event(authorization_builder.build(input.developer_pubkey))
            .await
            .map_err(|err| err.to_string())?;
        let authorization_event_id = authorization_event.id.to_hex();
        let mut row = existing;
        row.acceptance_event_id = authorization_event_id.clone();
        row.authorization_root_event_id = Some(authorization_event_id.clone());
        row.authorization_capabilities = terms.capabilities.into_iter().collect();
        row.authorization_profile_version = 2;

        return Ok(ProvisioningDecision::Reused {
            fulfillment_pubkey: row.fulfillment_pubkey.clone(),
            authorization_event_id,
            valid_from: row.valid_from,
            authorization_event: Box::new(authorization_event),
            attestation: provision.attestation.clone().map(Box::new),
            row: Box::new(row),
        });
    }
    let now = now_unix_i64()?;
    let valid_from: u64 = now
        .try_into()
        .map_err(|_| "current time is negative".to_string())?;
    let terms = AuthorizationTerms {
        authorization_id: uuid::Uuid::new_v4().to_string(),
        coordinate: listing_coordinate,
        operator_pubkey: nostr::PublicKey::from_hex(&input.server_info.pubkey)
            .map_err(|_| "operator pubkey is invalid".to_string())?,
        fulfillment_pubkey: nostr::PublicKey::from_hex(&provision.fulfillment_pubkey)
            .map_err(|_| "provisioned fulfillment pubkey is invalid".to_string())?,
        capabilities: BTreeSet::from([
            CAPABILITY_ISSUE_RECEIPT.into(),
            CAPABILITY_ISSUE_GRANT.into(),
            CAPABILITY_UPLOAD_BUILD.into(),
        ]),
        valid_from,
    };
    let authorization_builder =
        build_fulfillment_authorization_event_builder(&terms).map_err(|error| error.to_string())?;
    let authorization_event = input
        .signer
        .sign_event(authorization_builder.build(input.developer_pubkey))
        .await
        .map_err(|err| err.to_string())?;
    let authorization_event_id = authorization_event.id.to_hex();
    let row = AdpProvisioning {
        id: format!(
            "{}:{}:{}",
            input.developer_npub, input.server_url, input.scope
        ),
        developer_npub: input.developer_npub.to_string(),
        server_url: input.server_url.to_string(),
        operator_pubkey: input.server_info.pubkey.clone(),
        scope: Some(input.scope.to_string()),
        fulfillment_pubkey: provision.fulfillment_pubkey.clone(),
        attestation_event_id: provision.attestation_event_id.clone(),
        acceptance_event_id: authorization_event_id.clone(),
        authorization_root_event_id: Some(authorization_event_id.clone()),
        authorization_capabilities: terms.capabilities.into_iter().collect(),
        authorization_profile_version: 2,
        valid_from: now,
        revoked_at: None,
        created_at: now,
    };

    Ok(ProvisioningDecision::Created {
        fulfillment_pubkey: provision.fulfillment_pubkey,
        authorization_event_id,
        valid_from: row.valid_from,
        authorization_event: Box::new(authorization_event),
        attestation: provision.attestation.map(Box::new),
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
    emit_server_progress(app, step, status, None, message)
}

fn emit_server_progress<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    step: &str,
    status: &str,
    server_url: Option<String>,
    message: Option<String>,
) -> Result<(), String> {
    app.emit(
        "publish-progress",
        PublishProgressPayload {
            step: step.to_string(),
            status: status.to_string(),
            server_url,
            message,
            bytes_uploaded: None,
            total_bytes: None,
        },
    )
    .map_err(|err| err.to_string())
}

fn emit_upload_progress<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    server_url: &str,
    bytes_uploaded: u64,
    total_bytes: u64,
) -> Result<(), String> {
    app.emit(
        "publish-progress",
        PublishProgressPayload {
            step: "upload".to_string(),
            status: "progress".to_string(),
            server_url: Some(server_url.to_string()),
            message: None,
            bytes_uploaded: Some(bytes_uploaded),
            total_bytes: Some(total_bytes),
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
    use arcadestr_core::authorization::{parse_authorization_event, AuthorizationTransition};
    use arcadestr_core::http_client::{HttpClient, ReqwestHttpClient};
    use arcadestr_core::marketplace_cache::MarketplaceCache;
    use arcadestr_core::nip05_validator::Nip05Validator;
    use arcadestr_core::nip46::AppSignerState;
    use arcadestr_core::nostr::{EventDeduplicator, NostrClient};
    use arcadestr_core::profile_fetcher::ProfileFetcher;
    use arcadestr_core::relay_cache::RelayCache;
    use arcadestr_core::relay_hints::RelayHints;
    use arcadestr_core::relay_manager::RelayManagerConfig;
    use arcadestr_core::signers::{LocalSigner, NostrSigner};

    use arcadestr_core::storage::Database;
    use arcadestr_core::subscriptions::SubscriptionRegistry;
    use arcadestr_core::user_cache::UserCache;
    use nostr::{nips::nip19::ToBech32, EventBuilder, Keys, Kind, Tag, TagKind, Timestamp};

    #[test]
    fn expected_publisher_must_match_resolved_signer() {
        let expected = nostr::Keys::generate();
        let other = nostr::Keys::generate();
        let expected_npub = expected
            .public_key()
            .to_bech32()
            .expect("test npub should encode");

        assert_eq!(
            verify_expected_publisher(&expected_npub, expected.public_key()),
            Ok(())
        );
        let error = verify_expected_publisher(&expected_npub, other.public_key())
            .expect_err("a changed account must be rejected");
        assert!(error.contains("Active account changed"));
        assert!(error.contains("expected publisher does not match signer pubkey"));
    }

    #[test]
    fn ordinary_listing_edits_preserve_unmanaged_tags() {
        assert!(is_publish_form_tag(&["title".into(), "Game".into()]));
        assert!(is_publish_form_tag(&[
            "fulfillment_pubkey".into(),
            "key".into()
        ]));
        assert!(!is_publish_form_tag(&["campaign".into(), "root".into()]));
        assert!(!is_publish_form_tag(&[
            "summary".into(),
            "Short description".into()
        ]));
        assert!(!is_publish_form_tag(&["status".into(), "active".into()]));
    }

    #[test]
    fn timed_acquisition_wire_format_deserializes_into_core_policy() {
        let policy: arcadestr_core::marketplace::AcquisitionPolicy = serde_json::from_value(
            serde_json::json!({ "TimedAccess": { "starts_at": 100, "ends_at": 200 } }),
        )
        .expect("app timed policy payload should deserialize");

        assert_eq!(
            policy,
            arcadestr_core::marketplace::AcquisitionPolicy::TimedAccess {
                starts_at: 100,
                ends_at: 200,
            }
        );
    }
    use serde_json::json;
    use tauri::Manager;
    use tokio::sync::{Mutex as AsyncMutex, RwLock};

    #[tokio::test]
    async fn restored_nip46_client_signer_is_used_when_legacy_auth_is_empty() {
        let keys = Keys::generate();
        let expected_pubkey = keys.public_key();
        let client = nostr_sdk::Client::new(keys);
        let signer_state = Arc::new(AsyncMutex::new(AppSignerState {
            active_client: Some(client),
            ..AppSignerState::new()
        }));

        let resolved = resolve_active_signer(&signer_state, &AuthState::new())
            .await
            .expect("active NIP-46 signer should be resolved");

        assert_eq!(
            resolved
                .get_public_key()
                .await
                .expect("resolved signer should expose public key"),
            expected_pubkey
        );
    }

    fn signed_listing_event(keys: &Keys, content: &str, created_at: u64) -> nostr::Event {
        EventBuilder::new(Kind::Custom(30402), content)
            .tags([Tag::custom(TagKind::d(), ["same-coordinate"])])
            .custom_created_at(Timestamp::from(created_at))
            .sign_with_keys(keys)
            .expect("listing event should sign")
    }

    #[test]
    fn replaceable_selector_uses_lower_id_for_equal_timestamp_in_any_arrival_order() {
        let keys = Keys::generate();
        let first = signed_listing_event(&keys, "first", 100);
        let second = signed_listing_event(&keys, "second", 100);
        let expected_id = std::cmp::min(first.id.to_hex(), second.id.to_hex());

        let forward = select_replaceable_event([first.clone(), second.clone()])
            .expect("forward events should select a winner");
        let reverse = select_replaceable_event([second, first])
            .expect("reverse events should select a winner");

        assert_eq!(forward.id.to_hex(), expected_id);
        assert_eq!(reverse.id.to_hex(), expected_id);
    }

    #[test]
    fn replaceable_selector_prefers_newer_timestamp() {
        let keys = Keys::generate();
        let older = signed_listing_event(&keys, "older", 100);
        let newer = signed_listing_event(&keys, "newer", 101);

        let selected = select_replaceable_event([newer.clone(), older])
            .expect("events should select a winner");

        assert_eq!(selected.id, newer.id);
    }

    #[test]
    fn reusable_file_hash_rejects_malformed_metadata_and_preserves_valid_hash() {
        let valid = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

        assert_eq!(reusable_file_hash(Some(valid)), Ok(valid.to_string()));
        assert_eq!(
            reusable_file_hash(Some("abc123")),
            Err("existing file hash is invalid; select a replacement build file".to_string())
        );
    }

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
        assert!(!format!("{response:?}")
            .contains("0000000000000000000000000000000000000000000000000000000000000002"));
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
            .manage(Arc::new(AsyncMutex::new(AppSignerState::new())))
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

    fn assert_live_listing_tags(
        event: &nostr::Event,
        result: &PublishAdpListingResult,
        lud16: &str,
    ) {
        let file_hash = result.file_hash.as_ref().expect("file hash should be set");
        let fulfillment_pubkey = result
            .fulfillment_pubkey
            .as_ref()
            .expect("fulfillment pubkey should be set");

        assert!(
            !tag_values(event, "server").is_empty(),
            "missing server tag"
        );
        assert_eq!(
            tag_values(event, "file_hash")[0],
            vec!["file_hash", file_hash.as_str()]
        );
        assert_eq!(
            tag_values(event, "version")[0],
            vec!["version", "0.0.1-live"]
        );
        assert_eq!(tag_values(event, "lud16")[0], vec!["lud16", lud16]);
        assert_eq!(
            tag_values(event, "platform")[0],
            vec!["platform", "linux-x86_64"]
        );

        let fulfillment_tags = tag_values(event, "fulfillment_pubkey");
        assert_eq!(fulfillment_tags.len(), 1);
        let fulfillment_tag = &fulfillment_tags[0];
        assert_eq!(fulfillment_tag.len(), 4);
        assert_eq!(fulfillment_tag[0], "fulfillment_pubkey");
        assert_eq!(&fulfillment_tag[1], fulfillment_pubkey);
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

        async fn download_to_path(
            &self,
            _url: &str,
            _headers: Vec<(String, String)>,
            _dest: &std::path::Path,
            _on_progress: &mut (dyn FnMut(u64, Option<u64>) + Send),
        ) -> Result<
            arcadestr_core::http_client::HttpDownloadOutcome,
            arcadestr_core::http_client::HttpClientError,
        > {
            Err(arcadestr_core::http_client::HttpClientError::Request(
                "download_to_path path should not be used in local desktop mock tests".to_string(),
            ))
        }
    }

    #[tokio::test]
    async fn publish_same_scope_reuses_provisioning_after_revalidating_with_operator() {
        let db = test_db().await;
        let repo = AdpProvisioningRepository::new(db.pool().clone());
        let provision_url = "https://dist.example.com/provision";
        let mock = Arc::new(LocalMockHttpClient::default().with_json_post_response(
            provision_url,
            json!({
                "fulfillment_pubkey": "b0822d6340862b961e88b983b9c3b434e1fc750a36e796ee10825e8778badacf",
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
            pubkey: Keys::generate().public_key().to_hex(),
            name: Some("Test ADP".to_string()),
            url: Some("https://dist.example.com".to_string()),
            event_kinds: None,
            event_kind_status: None,
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

        let ProvisioningDecision::Reused {
            fulfillment_pubkey,
            valid_from,
            authorization_event,
            row,
            ..
        } = second
        else {
            panic!("second call should reuse provisioning");
        };
        let parsed = parse_authorization_event(&authorization_event).unwrap_or_else(|error| {
            panic!(
                "reused authorization should parse: {error:?}; tags={:?}",
                authorization_event.tags
            )
        });
        let AuthorizationTransition::ActiveRoot = parsed.transition else {
            panic!("reused authorization should be an active root");
        };
        let terms = parsed.terms;
        assert_eq!(terms.coordinate, format!("30402:{developer_npub}:game"));
        assert_eq!(terms.fulfillment_pubkey.to_hex(), fulfillment_pubkey);
        assert_eq!(
            terms.valid_from,
            u64::try_from(valid_from).expect("valid time")
        );
        assert_eq!(row.fulfillment_pubkey, fulfillment_pubkey);
        assert_eq!(row.valid_from, valid_from);
        assert_eq!(row.acceptance_event_id, authorization_event.id.to_hex());
        assert_eq!(mock.post_call_count(provision_url), 2);
    }

    #[tokio::test]
    async fn existing_unrevoked_delegated_edit_repairs_authorization_with_fresh_evidence() {
        let db = test_db().await;
        let repo = AdpProvisioningRepository::new(db.pool().clone());
        let developer = LocalSigner::from_hex(
            "0000000000000000000000000000000000000000000000000000000000000001",
        )
        .expect("test key should be valid");
        let developer_pubkey = developer
            .get_public_key()
            .await
            .expect("developer key should load");
        let developer_hex = developer_pubkey.to_hex();
        let fulfillment_pubkey = Keys::generate().public_key().to_hex();
        let original_valid_from = 1_700_000_000;
        let row = AdpProvisioning {
            id: "existing-game-provisioning".to_string(),
            developer_npub: developer_hex.clone(),
            server_url: "https://operator.example.com".to_string(),
            operator_pubkey: Keys::generate().public_key().to_hex(),
            scope: Some("game".to_string()),
            fulfillment_pubkey: fulfillment_pubkey.clone(),
            attestation_event_id: "stable-authorization-id".to_string(),
            acceptance_event_id: "legacy-acceptance-id".to_string(),
            authorization_root_event_id: Some("11".repeat(32)),
            authorization_capabilities: vec![
                CAPABILITY_ISSUE_RECEIPT.into(),
                CAPABILITY_ISSUE_GRANT.into(),
                CAPABILITY_UPLOAD_BUILD.into(),
            ],
            authorization_profile_version: 2,
            valid_from: original_valid_from,
            revoked_at: None,
            created_at: original_valid_from,
        };
        repo.upsert(&row).await.expect("row should persist");
        let provision = ProvisionResponse {
            fulfillment_pubkey: fulfillment_pubkey.clone(),
            attestation_event_id: "refreshed-attestation-id".to_string(),
            attestation: None,
            scope: Some("game".to_string()),
        };

        let repair = repair_existing_authorization(ExistingAuthorizationRepairInput {
            provisioning_repo: &repo,
            signer: &developer,
            developer_pubkey,
            developer_hex: &developer_hex,
            operator_url: "https://operator.example.com",
            scope: "game",
            fulfillment_pubkey: &fulfillment_pubkey,
            provision: &provision,
        })
        .await
        .expect("existing authorization should repair");

        let parsed = parse_authorization_event(&repair.authorization_event)
            .expect("repaired authorization should parse");
        let AuthorizationTransition::ActiveRoot = parsed.transition else {
            panic!("repaired authorization should be active");
        };
        let terms = parsed.terms;
        assert!(!terms.authorization_id.is_empty());
        assert_eq!(terms.coordinate, format!("30402:{developer_hex}:game"));
        assert_eq!(terms.fulfillment_pubkey.to_hex(), fulfillment_pubkey);
        assert_eq!(terms.valid_from, repair.row.valid_from as u64);
        assert!(repair.row.valid_from >= original_valid_from);
        assert_eq!(repair.row.fulfillment_pubkey, fulfillment_pubkey);
        assert_eq!(repair.row.attestation_event_id, "refreshed-attestation-id");
        assert_eq!(
            repair.row.acceptance_event_id,
            repair.authorization_event.id.to_hex()
        );
        repo.upsert(&repair.row)
            .await
            .expect("repaired row should persist");
        let stored = repo
            .active_for_scope(&developer_hex, "https://operator.example.com", Some("game"))
            .await
            .expect("repaired row lookup should succeed")
            .expect("repaired row should remain active");
        assert_eq!(stored.valid_from, repair.row.valid_from);
        assert_eq!(stored.fulfillment_pubkey, fulfillment_pubkey);
        assert_eq!(stored.acceptance_event_id, repair.authorization_event_id);
    }

    #[test]
    fn delegated_edit_reuses_the_referenced_key_without_listing_timestamps() {
        let metadata = resolve_existing_fulfillment_metadata(
            &FulfillmentMode::Delegate,
            "developer-key",
            Some("delegated-key"),
        )
        .expect("existing delegated key is accepted");
        assert_eq!(
            metadata.fulfillment_pubkey.as_deref(),
            Some("delegated-key")
        );
        assert!(!metadata.should_provision);
    }

    #[test]
    fn direct_to_delegate_conversion_provisions_a_new_key() {
        let metadata = resolve_existing_fulfillment_metadata(
            &FulfillmentMode::Delegate,
            "developer-key",
            Some("developer-key"),
        )
        .expect("conversion is accepted");
        assert!(metadata.fulfillment_pubkey.is_none());
        assert!(metadata.should_provision);
    }

    #[test]
    fn ordinary_edit_cannot_silently_clear_existing_fulfillment() {
        let error = resolve_existing_fulfillment_metadata(
            &FulfillmentMode::None,
            "developer-key",
            Some("delegated-key"),
        )
        .expect_err("existing fulfillment reference must not be silently cleared");
        assert!(error.contains("cannot be cleared"));
    }

    #[test]
    fn publisher_npub_contract_converts_to_hex() {
        let keys = Keys::generate();
        let npub = keys
            .public_key()
            .to_bech32()
            .expect("publisher npub should encode");

        assert_eq!(
            publisher_hex_from_npub(&npub).expect("publisher npub should parse"),
            keys.public_key().to_hex()
        );
    }

    #[test]
    fn unique_operator_contract_requires_exactly_one_row() {
        let row = AdpProvisioning {
            id: "one".to_string(),
            developer_npub: "developer".to_string(),
            server_url: "https://operator.example.com".to_string(),
            operator_pubkey: "operator".to_string(),
            scope: Some("game".to_string()),
            fulfillment_pubkey: "fulfillment".to_string(),
            attestation_event_id: "attestation".to_string(),
            acceptance_event_id: "acceptance".to_string(),
            authorization_root_event_id: None,
            authorization_capabilities: Vec::new(),
            authorization_profile_version: 1,
            valid_from: 123,
            revoked_at: Some(456),
            created_at: 123,
        };
        let mut second = row.clone();
        second.id = "two".to_string();
        second.server_url = "https://other.example.com".to_string();

        assert_eq!(unique_operator_url(&[]), None);
        assert_eq!(
            unique_operator_url(std::slice::from_ref(&row)).as_deref(),
            Some("https://operator.example.com")
        );
        assert_eq!(unique_operator_url(&[row, second]), None);
    }

    #[tokio::test]
    #[ignore = "requires ADP_TEST_SERVER_URL and live local relays"]
    async fn live_publish_adp_listing_uploads_and_propagates_to_two_relays() {
        let server_url = std::env::var("ADP_TEST_SERVER_URL")
            .expect("ADP_TEST_SERVER_URL must be set for live ADP publish test");
        let lud16 =
            std::env::var("ADP_TEST_LUD16").unwrap_or_else(|_| "seller@example.com".to_string());
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
            expected_publisher_npub: nostr::Keys::parse(
                "0000000000000000000000000000000000000000000000000000000000000001",
            )
            .expect("live test key should parse")
            .public_key()
            .to_bech32()
            .expect("live test npub should encode"),
            existing_event_id: None,
            d_tag: d_tag.clone(),
            title: "Gate 3 Live Test Binary".to_string(),
            description: "Small live ADP command-level publish test fixture".to_string(),
            price_sats,
            lud16: Some(lud16.clone()),
            tags: vec![],
            images: vec![],
            fulfillment_mode: FulfillmentMode::Delegate,
            operator_url: Some(server_url.clone()),
            servers: vec![server_url.clone()],
            file_path: Some(file_path.display().to_string()),
            existing_file_hash: None,
            existing_fulfillment_pubkey: None,
            version: Some("0.0.1-live".to_string()),
            acquisition: arcadestr_core::marketplace::AcquisitionPolicy::Gated,
            platforms: vec!["linux-x86_64".to_string()],
            campaigns: Vec::new(),
            nip94_event_id: None,
        };

        let result = publish_adp_listing(
            request,
            app.handle().clone(),
            app.state::<AppState>(),
            app.state::<Arc<AsyncMutex<AppSignerState>>>(),
        )
        .await
        .expect("live ADP publish command should complete");

        let file_hash = result.file_hash.as_ref().expect("file hash should be set");
        let acceptance_event_id = result
            .acceptance_event_id
            .as_ref()
            .expect("acceptance event id should be set");
        let fulfillment_pubkey = result
            .fulfillment_pubkey
            .as_ref()
            .expect("fulfillment pubkey should be set");
        let upload = result
            .uploads
            .first()
            .and_then(|upload| upload.upload.as_ref())
            .expect("upload should succeed");

        assert_eq!(file_hash.len(), 64);
        assert!(!result.event_id.is_empty());
        assert!(!acceptance_event_id.is_empty());
        assert!(!fulfillment_pubkey.is_empty());
        assert!(!upload.download_url.is_empty());
        assert_eq!(&upload.file_hash, file_hash);
        assert!(
            upload.game_coordinate.contains(&d_tag),
            "upload response should reference the published listing coordinate"
        );
        println!("ADP_LIVE_EVENT_ID={}", result.event_id);
        println!("ADP_LIVE_GAME_COORDINATE={}", upload.game_coordinate);
        println!("ADP_LIVE_DOWNLOAD_URL={}", upload.download_url);

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

    fn live_app_listing_from_coordinate(coordinate: &str) -> arcadestr_app::models::GameListing {
        let mut parts = coordinate.splitn(3, ':');
        assert_eq!(parts.next(), Some("30402"));
        let publisher_hex = parts.next().expect("coordinate publisher should exist");
        let listing_id = parts.next().expect("coordinate d tag should exist");
        let publisher_npub = nostr::PublicKey::from_hex(publisher_hex)
            .expect("coordinate publisher should parse")
            .to_bech32()
            .expect("coordinate publisher should encode as npub");

        arcadestr_app::models::GameListing {
            id: listing_id.to_string(),
            source: arcadestr_app::models::ListingSource::Nip99Listing,
            title: format!("ADP live fixture {listing_id}"),
            description: "ADP Gate 5 live install fixture".to_string(),
            images: Vec::new(),
            download_url: String::new(),
            price: 1.0,
            currency: "SATS".to_string(),
            price_sats: 1,
            quantity: None,
            tags: Vec::new(),
            specs: Vec::new(),
            publisher_npub,
            stall_id: String::new(),
            stall_name: None,
            lud16: String::new(),
            event_id: None,
            created_at: 1_700_000_000,
            platforms: Vec::new(),
            nip94_event_id: None,
            acquisition: arcadestr_app::models::AcquisitionPolicy::Gated,
            campaigns: Vec::new(),
            is_owned: true,
            #[cfg(debug_assertions)]
            nip99_raw_event_json: None,
        }
    }

    fn live_listing_file_hash(event: &nostr::Event) -> String {
        tag_values(event, "file_hash")
            .into_iter()
            .find_map(|values| values.get(1).cloned())
            .expect("live listing should include file_hash tag")
    }

    fn live_listing_server_url(event: &nostr::Event) -> String {
        tag_values(event, "server")
            .into_iter()
            .find_map(|values| values.get(1).cloned())
            .expect("live listing should include server tag")
    }

    struct LiveConfirmPurchaseInput<'a> {
        app: &'a tauri::App<tauri::test::MockRuntime>,
        server_url: &'a str,
        coordinate: &'a str,
        lud16: &'a str,
        nwc_connection: &'a arcadestr_core::nwc_client::NwcConnection,
        use_nwc_payment: bool,
    }

    async fn live_confirm_purchase_pass(
        input: LiveConfirmPurchaseInput<'_>,
    ) -> ConfirmPurchaseResponse {
        eprintln!(
            "ADP_GATE4_STAGE=confirm_pass_start nwc={}",
            input.use_nwc_payment
        );
        let (publisher_npub, listing_id) = listing_parts_from_coordinate(input.coordinate);
        eprintln!(
            "ADP_GATE4_STAGE=request_invoice_start nwc={}",
            input.use_nwc_payment
        );
        let invoice = request_lnurl_invoice(
            RequestLnurlInvoiceRequest {
                lud16: input.lud16.to_string(),
                amount_sats: 1,
            },
            input.app.state::<AppState>(),
        )
        .await
        .expect("LNURL invoice should resolve and request");
        eprintln!(
            "ADP_GATE4_STAGE=request_invoice_ok nwc={}",
            input.use_nwc_payment
        );
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
        eprintln!(
            "ADP_GATE4_STAGE=pay_preimage_ok nwc={}",
            input.use_nwc_payment
        );

        eprintln!(
            "ADP_GATE4_STAGE=confirm_purchase_start nwc={}",
            input.use_nwc_payment
        );
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
        let nwc_connection =
            arcadestr_core::nwc_client::NwcConnection::parse(&nwc_connection_string)
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

        let token_repo =
            DownloadTokensRepository::new(app.state::<AppState>().database.pool().clone());
        let token = token_repo
            .valid_token(
                &buyer_pubkey,
                &coordinate,
                &server_url,
                now_unix_i64().expect("clock should work"),
            )
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

        let token_repo =
            DownloadTokensRepository::new(app.state::<AppState>().database.pool().clone());
        let token = token_repo
            .valid_token(
                &buyer_pubkey,
                &coordinate,
                &server_url,
                now_unix_i64().expect("clock should work"),
            )
            .await
            .expect("download token lookup should succeed")
            .expect("download token should be stored");
        assert_eq!(token.token, response.download_token);
        println!(
            "ADP_GATE4_MANUAL_DOWNLOAD_TOKEN={}",
            response.download_token
        );
    }

    #[tokio::test]
    #[ignore = "requires ADP_TEST_SERVER_URL, ARCADESTR_RELAYS, ADP_TEST_GAME_COORDINATE, ADP_TEST_MANUAL_BOLT11, and ADP_TEST_MANUAL_PREIMAGE"]
    async fn live_install_game_path_a_and_path_b() {
        eprintln!("ADP_GATE5_STAGE=test_start");
        let server_url = std::env::var("ADP_TEST_SERVER_URL")
            .expect("ADP_TEST_SERVER_URL must be set for live Gate 5 test");
        let coordinate = std::env::var("ADP_TEST_GAME_COORDINATE")
            .expect("ADP_TEST_GAME_COORDINATE must be set for live Gate 5 test");
        let bolt11 = std::env::var("ADP_TEST_MANUAL_BOLT11")
            .expect("ADP_TEST_MANUAL_BOLT11 must be set for live Gate 5 test");
        let preimage = std::env::var("ADP_TEST_MANUAL_PREIMAGE")
            .expect("ADP_TEST_MANUAL_PREIMAGE must be set for live Gate 5 test");
        let relays = live_relays_from_env();

        eprintln!("ADP_GATE5_STAGE=app_init_start");
        let app = live_test_app(relays).await;
        eprintln!("ADP_GATE5_STAGE=app_init_ok");
        let state = app.state::<AppState>();
        let buyer_pubkey = {
            let auth = state.auth.lock().await;
            auth.public_key()
                .expect("live buyer should be authenticated")
                .to_hex()
        };
        let listing_event = fetch_listing_event_by_coordinate(&state, &coordinate)
            .await
            .expect("live listing event should fetch before install");
        let expected_file_hash = live_listing_file_hash(&listing_event);
        let listing_server_url = live_listing_server_url(&listing_event);
        assert_eq!(
            listing_server_url, server_url,
            "ADP_TEST_SERVER_URL must match the listing server tag used by install_game"
        );
        let listing = live_app_listing_from_coordinate(&coordinate);
        let (publisher_npub, listing_id) = listing_parts_from_coordinate(&coordinate);
        let fresh_listing_fetcher = crate::RelayFreshListingFetcher {
            state: state.clone(),
        };
        let app_data_dir = app
            .path()
            .app_data_dir()
            .expect("live test app data dir should resolve");

        eprintln!("ADP_GATE5_STAGE=manual_confirm_purchase_start");
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
        eprintln!("ADP_GATE5_STAGE=manual_confirm_purchase_ok");
        assert!(!response.download_token.is_empty());
        assert!(response.token_expires_at > 0);

        let token_repo =
            DownloadTokensRepository::new(app.state::<AppState>().database.pool().clone());
        let cached_token = token_repo
            .valid_token(
                &buyer_pubkey,
                &coordinate,
                &server_url,
                now_unix_i64().expect("clock should work"),
            )
            .await
            .expect("download token lookup should succeed")
            .expect("download token should be stored before Path A");
        assert_eq!(cached_token.token, response.download_token);
        println!("ADP_GATE5_PATH_A_TOKEN_PRESENT=true");

        eprintln!("ADP_GATE5_STAGE=path_a_install_start");
        crate::install_game_with_fetcher(
            listing.clone(),
            &state,
            Some(app.handle()),
            app_data_dir.clone(),
            &fresh_listing_fetcher,
        )
        .await
        .expect("Path A install with cached token should succeed");
        let installed_repo = arcadestr_core::adp_storage::InstalledGamesRepository::new(
            app.state::<AppState>().database.pool().clone(),
        );
        let path_a_row = installed_repo
            .get(&coordinate)
            .await
            .expect("Path A installed game lookup should succeed")
            .expect("Path A installed_games row should exist");
        assert!(
            path_a_row.file_path.exists(),
            "Path A artifact should exist at {}",
            path_a_row.file_path.display()
        );
        let path_a_hash = sha256_file(&path_a_row.file_path)
            .await
            .expect("Path A artifact hash should compute");
        assert_eq!(path_a_hash, expected_file_hash);
        assert_eq!(path_a_row.file_hash, expected_file_hash);
        let installed_games = crate::get_installed_games(app.state::<AppState>())
            .await
            .expect("get_installed_games should be callable in live test");
        assert!(installed_games
            .iter()
            .any(|game| game.game_coordinate == coordinate));
        println!(
            "ADP_GATE5_PATH_A_INSTALL_OK=true artifact_exists=true sha256_match=true \
             installed_games_row=true get_installed_games_includes_coordinate=true"
        );

        token_repo
            .delete(&buyer_pubkey, &coordinate, &server_url)
            .await
            .expect("Path B should delete local token row");
        let token_after_delete = token_repo
            .valid_token(
                &buyer_pubkey,
                &coordinate,
                &server_url,
                now_unix_i64().expect("clock should work"),
            )
            .await
            .expect("download token lookup after delete should succeed");
        assert!(token_after_delete.is_none());
        println!("ADP_GATE5_PATH_B_LOCAL_TOKEN_DELETED=true");
        tokio::fs::remove_file(&path_a_row.file_path)
            .await
            .expect("Path B should remove Path A artifact before reinstall");
        tokio::time::sleep(Duration::from_secs(1)).await;

        eprintln!("ADP_GATE5_STAGE=path_b_install_start");
        crate::install_game_with_fetcher(
            listing,
            &state,
            Some(app.handle()),
            app_data_dir,
            &fresh_listing_fetcher,
        )
        .await
        .expect("Path B install without local token should succeed");
        let path_b_row = installed_repo
            .get(&coordinate)
            .await
            .expect("Path B installed game lookup should succeed")
            .expect("Path B installed_games row should exist");
        assert!(
            path_b_row.file_path.exists(),
            "Path B artifact should exist at {}",
            path_b_row.file_path.display()
        );
        let path_b_hash = sha256_file(&path_b_row.file_path)
            .await
            .expect("Path B artifact hash should compute");
        assert_eq!(path_b_hash, expected_file_hash);
        assert_eq!(path_b_row.file_hash, expected_file_hash);
        assert!(
            path_b_row.installed_at > path_a_row.installed_at,
            "Path B should replace installed_games row after reinstall"
        );
        let token_after_path_b = token_repo
            .valid_token(
                &buyer_pubkey,
                &coordinate,
                &server_url,
                now_unix_i64().expect("clock should work"),
            )
            .await
            .expect("download token lookup after Path B should succeed");
        assert!(token_after_path_b.is_none());
        println!(
            "ADP_GATE5_PATH_B_NIP98_INSTALL_OK=true artifact_exists=true sha256_match=true \
             installed_games_row=true local_token_deleted_before_install=true \
             local_token_absent_after_install=true"
        );
    }
}
