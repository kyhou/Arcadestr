//! Read-only portability integration. Never touches wallet credentials, payment
//! journals, quote APIs, signing secrets or receipt publication.
use crate::AppState;
use arcadestr_core::{
    http_client::HttpClient,
    receipt_portability::{ReceiptPortability, ReceiptSource},
    receipt_sync_contract::{ReceiptImportRequest, ReceiptSyncReport, ReceiptSyncRequest},
    relay_manager::RelayManager,
};
use async_trait::async_trait;
use nostr::{Event, Filter, Kind, PublicKey};
use std::sync::Arc;
use tauri::{AppHandle, Emitter, State};

struct RelayReceiptSource {
    manager: RelayManager,
    urls: Vec<String>,
    http: Arc<dyn HttpClient>,
}
#[async_trait]
impl ReceiptSource for RelayReceiptSource {
    fn relay_urls(&self) -> Vec<String> {
        self.urls.clone()
    }
    async fn query(&self, relay: &str, filter: Filter) -> Result<Vec<Event>, String> {
        self.manager
            .fetch_events_from_subset(filter, vec![relay.to_string()])
            .await
            .map_err(|_| "Receipt relay unavailable".into())
    }
    async fn lsp_pubkey(&self, coordinate: &str) -> Result<Option<PublicKey>, String> {
        let mut parts = coordinate.splitn(3, ':');
        if parts.next() != Some("30402") {
            return Ok(None);
        }
        let developer = parts
            .next()
            .and_then(|s| PublicKey::from_hex(s).ok())
            .ok_or("Invalid listing coordinate")?;
        let id = parts.next().ok_or("Invalid listing coordinate")?;
        let events = self
            .manager
            .fetch_events_best_effort(
                Filter::new()
                    .kind(Kind::Custom(30402))
                    .author(developer)
                    .identifier(id),
            )
            .await
            .map_err(|_| "Listing unavailable")?;
        let listing = events
            .into_iter()
            .filter(|e| {
                let identifiers = e
                    .tags
                    .iter()
                    .filter(|t| t.as_slice().first().is_some_and(|n| n == "d"))
                    .collect::<Vec<_>>();
                e.verify().is_ok()
                    && e.kind.as_u16() == 30402
                    && e.pubkey == developer
                    && identifiers.len() == 1
                    && matches!(identifiers[0].as_slice(),[_,value] if value==id)
            })
            .max_by(|a, b| {
                a.created_at
                    .cmp(&b.created_at)
                    .then_with(|| b.id.cmp(&a.id))
            });
        let Some(listing) = listing else {
            return Ok(None);
        };
        let addresses = listing
            .tags
            .iter()
            .filter_map(|t| match t.as_slice() {
                [name, value] if name == "lud16" => Some(value.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>();
        if addresses.len() != 1 {
            return Ok(None);
        }
        // Resolves public metadata only. No callback/invoice request is made.
        let metadata = arcadestr_core::lnurlp::resolve_lud16(self.http.as_ref(), addresses[0])
            .await
            .map_err(|_| "Listing LSP metadata unavailable")?;
        Ok(metadata
            .nostr_pubkey
            .and_then(|s| PublicKey::from_hex(&s).ok()))
    }
    async fn aggregate_lsp(
        &self,
        receipt: &arcadestr_core::purchases::ParsedReceipt,
        leg: &arcadestr_core::adp_settlement::AggregateProofLeg,
    ) -> Result<Option<arcadestr_core::receipt_portability::AggregateLspEvidence>, String> {
        let aggregate = receipt
            .aggregate
            .as_ref()
            .ok_or("Missing aggregate profile")?;
        let mut ids = vec![aggregate.listing_event_id];
        if let Some(id) = &leg.accepted_terms_id {
            ids.push(nostr::EventId::from_hex(id).map_err(|_| "Invalid accepted terms reference")?);
        }
        let events = self
            .manager
            .fetch_events_best_effort(Filter::new().ids(ids))
            .await
            .map_err(|_| "Aggregate public provenance unavailable")?;
        let listing = events
            .iter()
            .find(|e| e.id == aggregate.listing_event_id)
            .ok_or("Original aggregate listing unavailable")?;
        let terms = leg
            .accepted_terms_id
            .as_ref()
            .and_then(|id| events.iter().find(|e| e.id.to_hex() == *id));
        let address = arcadestr_core::receipt_portability::aggregate_zap_destination(
            receipt, leg, listing, terms,
        )?;
        let metadata = arcadestr_core::lnurlp::resolve_lud16(self.http.as_ref(), &address)
            .await
            .map_err(|_| "Aggregate LSP metadata unavailable")?;
        let Some(key) = metadata
            .nostr_pubkey
            .and_then(|s| PublicKey::from_hex(&s).ok())
        else {
            return Ok(None);
        };
        Ok(Some(
            arcadestr_core::receipt_portability::AggregateLspEvidence {
                key,
                signed_events: events,
            },
        ))
    }
}

async fn source(state: &AppState) -> RelayReceiptSource {
    let shared = { state.nostr.lock().await.get_relay_manager().clone() };
    let manager = { shared.lock().await.clone() };
    let urls = manager
        .get_client()
        .relays()
        .await
        .into_keys()
        .map(|u| u.to_string())
        .collect();
    RelayReceiptSource {
        manager,
        urls,
        http: Arc::clone(&state.http_client),
    }
}

fn buyer(expected: &str) -> Result<PublicKey, String> {
    PublicKey::from_hex(expected).map_err(|_| "Invalid buyer account".into())
}
async fn lease(
    state: &AppState,
    buyer: PublicKey,
) -> Result<arcadestr_core::settlement_recovery::RecoveryLease, String> {
    let auth = state.auth.lock().await;
    if auth.public_key() != Some(buyer) {
        return Err("Active receipt account changed".into());
    }
    state.settlement_recovery.observe(Some(buyer.to_hex()));
    state
        .settlement_recovery
        .claim(&buyer.to_hex(), "receipt-portability")
}

#[tauri::command]
pub async fn sync_purchase_receipts(
    app: AppHandle,
    request: ReceiptSyncRequest,
    state: State<'_, AppState>,
) -> Result<ReceiptSyncReport, String> {
    let buyer = buyer(&request.expected_account_pubkey)?;
    let lease = lease(&state, buyer).await?;
    let source = source(&state).await;
    let now = nostr::Timestamp::now().as_secs();
    let report = ReceiptPortability::new(state.database.pool().clone())
        .synchronize(buyer, &source, request.full_rescan, now, &|| {
            lease.ensure_current()
        })
        .await?;
    lease.ensure_current()?;
    let _ = app.emit(
        "receipt-ownership-updated",
        serde_json::json!({"account":buyer.to_hex()}),
    );
    Ok(report)
}

#[tauri::command]
pub async fn import_purchase_receipts(
    app: AppHandle,
    request: ReceiptImportRequest,
    state: State<'_, AppState>,
) -> Result<ReceiptSyncReport, String> {
    let buyer = buyer(&request.expected_account_pubkey)?;
    let report = import_for_account(buyer, &request.events_json, &state).await?;
    let _ = app.emit(
        "receipt-ownership-updated",
        serde_json::json!({"account":buyer.to_hex()}),
    );
    Ok(report)
}

pub(crate) async fn import_for_account(
    buyer: PublicKey,
    json: &str,
    state: &AppState,
) -> Result<ReceiptSyncReport, String> {
    let lease = lease(state, buyer).await?;
    // Offline import is independent of relay availability. Missing public proof
    // or authority evidence can be supplied in the array or fetched by Sync.
    let report = ReceiptPortability::new(state.database.pool().clone())
        .import_json(buyer, json, None, &|| lease.ensure_current())
        .await?;
    lease.ensure_current()?;
    Ok(report)
}

/// ADP confirmation already validated private evidence and listing context.
/// Archive its public receipt/authorization through the same portability path,
/// so future offline restarts do not need to rediscover historical authority.
pub(crate) async fn archive_confirmation(
    state: &AppState,
    buyer: PublicKey,
    receipt: &Event,
    authorization: Option<&arcadestr_core::authorization::ResolvedAuthorization>,
) -> Result<(), String> {
    let mut events = vec![receipt.clone()];
    if let Some(authorization) = authorization {
        events.extend(authorization.events.iter().map(|a| a.event.clone()));
    }
    let report = ReceiptPortability::new(state.database.pool().clone())
        .ingest(buyer, events, None, &|| {
            let auth = state
                .auth
                .try_lock()
                .map_err(|_| "Account state busy; original receipt remains recoverable")?;
            if auth.public_key() != Some(buyer) {
                return Err("Account changed; original receipt remains recoverable".into());
            }
            Ok(())
        })
        .await?;
    let parsed = arcadestr_core::purchases::parse_receipt_event(receipt)
        .map_err(|_| "Invalid confirmed receipt")?;
    if !report.chains.iter().any(|c| {
        c.listing_coordinate == parsed.coordinate
            && c.order_id == parsed.order_id
            && c.verification
                == arcadestr_core::receipt_sync_contract::ReceiptVerification::Verified
    }) {
        return Err(
            "Receipt chain needs verification; recover the original purchase without repayment"
                .into(),
        );
    }
    Ok(())
}
