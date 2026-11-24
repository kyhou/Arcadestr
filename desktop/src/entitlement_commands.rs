//! Known-grant revocation reconciliation and discovery of publisher-signed
//! grants issued without a campaign. Reads configured relays only.
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
            // An unreachable relay answers with an empty result; never count it
            // as a successful query.
            let connected = self
                .manager
                .get_client()
                .relay(url.as_str())
                .await
                .is_ok_and(|relay| relay.is_connected());
            if !connected {
                incomplete = true;
                continue;
            }
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
    let (report, discovered) = sync_for_account(buyer, &state).await?;
    if report.revoked > 0 || discovered > 0 {
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
    sync_for_account(buyer, state)
        .await
        .map(|(report, _)| report)
}

/// Discovers campaignless grants addressed to `buyer`, then reconciles every
/// known grant. Returns the merged report and the number of newly stored grants.
pub(crate) async fn sync_for_account(
    buyer: PublicKey,
    state: &AppState,
) -> Result<(EntitlementSyncReport, usize), String> {
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
    let repository = EntitlementsRepository::new(state.database.pool().clone());
    let ensure_current = || {
        session.ensure_current()?;
        lease.ensure_current()
    };
    // Discovery first, so a newly found grant is also checked for revocations.
    let discovery = repository
        .discover_campaignless_grants(buyer, &source, &ensure_current)
        .await?;
    let mut report = repository
        .reconcile_known_grants(buyer, &source, &ensure_current)
        .await?;
    report.checked += discovery.examined;
    report.revoked += discovery.revoked;
    report.rejected_events += discovery.rejected_chains + discovery.malformed_events;
    report.indeterminate += usize::from(discovery.unavailable);
    report.incomplete |= discovery.incomplete;
    session.ensure_current()?;
    Ok((report, discovery.discovered))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn unreachable_entitlement_relays_are_unavailable_not_empty_evidence() {
        let (manager, url) = crate::receipt_commands::tests::unreachable_relay().await;
        let source = RelayEntitlementSource {
            manager,
            urls: vec![url],
        };
        let result = source
            .query(Filter::new().kind(nostr::Kind::Custom(1030)))
            .await;
        assert_eq!(
            result.err().as_deref(),
            Some("Entitlement relays unavailable; known ownership retained")
        );
    }
}
