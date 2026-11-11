//! Known-grant revocation reconciliation. Reads configured relays only.
use crate::AppState;
use arcadestr_core::{
    entitlement_reconciliation::{EntitlementEvidence, EntitlementSource},
    entitlement_sync_contract::{EntitlementSyncReport, EntitlementSyncRequest},
    entitlements_repository::EntitlementsRepository,
    relay_manager::RelayManager,
};
use nostr::{Filter, PublicKey};
use tauri::{AppHandle, Emitter, State};

struct RelayEntitlementSource {
    manager: RelayManager,
    urls: Vec<String>,
}
#[async_trait::async_trait]
impl EntitlementSource for RelayEntitlementSource {
    async fn query(&self, filter: Filter) -> Result<EntitlementEvidence, String> {
        let mut events = Vec::new();
        let mut successes = 0;
        let mut incomplete = false;
        for url in &self.urls {
            match self
                .manager
                .fetch_events_from_subset(filter.clone(), vec![url.clone()])
                .await
            {
                Ok(found) => {
                    successes += 1;
                    events.extend(found);
                }
                Err(_) => incomplete = true,
            }
        }
        if successes == 0 {
            return Err("Entitlement relays unavailable; known ownership retained".into());
        }
        Ok(EntitlementEvidence { events, incomplete })
    }
}

#[tauri::command]
pub async fn reconcile_known_entitlements(
    app: AppHandle,
    request: EntitlementSyncRequest,
    state: State<'_, AppState>,
) -> Result<EntitlementSyncReport, String> {
    let buyer = PublicKey::from_hex(&request.expected_account_pubkey)
        .map_err(|_| "Invalid entitlement account")?;
    let report = reconcile_for_account(buyer, &state).await?;
    if report.revoked > 0 {
        let _ = app.emit(
            "receipt-ownership-updated",
            serde_json::json!({"account":buyer.to_hex()}),
        );
    }
    Ok(report)
}

pub(crate) async fn reconcile_for_account(
    buyer: PublicKey,
    state: &AppState,
) -> Result<EntitlementSyncReport, String> {
    let (session, lease) = {
        let auth = state.auth.lock().await;
        if auth.public_key() != Some(buyer) {
            return Err("Active entitlement account changed".into());
        }
        state.settlement_recovery.observe(Some(buyer.to_hex()));
        (
            state.settlement_recovery.session_fence()?,
            state
                .settlement_recovery
                .claim(&buyer.to_hex(), "known-entitlement-reconciliation")?,
        )
    };
    let shared = { state.nostr.lock().await.get_relay_manager().clone() };
    let manager = { shared.lock().await.clone() };
    let urls = manager
        .get_client()
        .relays()
        .await
        .into_keys()
        .map(|url| url.to_string())
        .collect();
    let source = RelayEntitlementSource { manager, urls };
    let report = EntitlementsRepository::new(state.database.pool().clone())
        .reconcile_known_grants(buyer, &source, &|| {
            session.ensure_current()?;
            lease.ensure_current()
        })
        .await?;
    session.ensure_current()?;
    Ok(report)
}
