//! Read-only Nostr reconciliation of *previously validated* local grants, plus
//! recipient-based discovery of publisher-signed grants issued without a
//! campaign (ADP-01 §Entitlement Grants, §Portable grant verification).
//!
//! Campaign-anchored roots never enter through this path; they arrive through
//! the claim flow with their campaign evidence. Their original campaign and
//! authorization validation remains historical; cancelling either later is not
//! a revocation.
use crate::{
    adp_protocol::{coordinate_publisher, ENTITLEMENT_GRANT_KIND},
    entitlement_sync_contract::EntitlementSyncReport,
    entitlements_repository::EntitlementsRepository,
};
use async_trait::async_trait;
use nostr::{Event, Filter, Kind, PublicKey};

pub struct EntitlementEvidence {
    pub events: Vec<Event>,
    pub incomplete: bool,
}

#[async_trait]
pub trait EntitlementSource: Send + Sync {
    async fn query(&self, filter: Filter) -> Result<EntitlementEvidence, String>;
}

impl EntitlementsRepository {
    pub async fn reconcile_known_grants(
        &self,
        buyer: PublicKey,
        source: &dyn EntitlementSource,
        ensure_current: &(dyn Fn() -> Result<(), String> + Send + Sync),
    ) -> Result<EntitlementSyncReport, String> {
        ensure_current()?;
        let history = self
            .history_for_buyer(&buyer.to_hex())
            .await
            .map_err(|e| e.to_string())?;
        let mut report = EntitlementSyncReport {
            incomplete: true,
            ..Default::default()
        };
        for grant in history {
            ensure_current()?;
            if grant.status != Some(crate::entitlements::GrantStatus::Granted) {
                continue;
            }
            let developer =
                coordinate_publisher(&grant.game_coordinate).ok_or("Invalid stored coordinate")?;
            report.checked += 1;
            // No cursor: a revocation may predate the most recent receipt sync.
            let evidence = source
                .query(
                    Filter::new()
                        .kind(Kind::Custom(ENTITLEMENT_GRANT_KIND))
                        .author(developer)
                        .pubkey(buyer)
                        .identifier(&grant.grant_id),
                )
                .await;
            ensure_current()?;
            let evidence = match evidence {
                Ok(e) => e,
                Err(_) => {
                    report.indeterminate += 1;
                    continue;
                }
            };
            match self
                .merge_known_revocations(
                    buyer,
                    &grant.game_coordinate,
                    &grant.grant_id,
                    &evidence.events,
                    ensure_current,
                )
                .await?
            {
                Some((changed, rejected)) => {
                    report.revoked += usize::from(changed);
                    report.rejected_events += rejected;
                    if !changed || evidence.incomplete {
                        report.indeterminate += 1;
                    }
                }
                None => report.indeterminate += 1,
            }
        }
        ensure_current()?;
        Ok(report)
    }
}

/// Outcome of [`EntitlementsRepository::discover_campaignless_grants`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CampaignlessDiscoveryReport {
    /// Campaignless grant chains addressed to the buyer that were examined.
    pub examined: usize,
    /// Grants stored for the first time.
    pub discovered: usize,
    /// Previously stored, still-granted grants now revoked.
    pub revoked: usize,
    /// Chains rejected: forks, multiple roots, a root not signed by the
    /// coordinate's publisher, or an authorization anchor.
    pub rejected_chains: usize,
    /// Events that failed signature or tag validation.
    pub malformed_events: usize,
    /// No relay answered; nothing can be concluded, nothing changed.
    pub unavailable: bool,
    /// Some relays did not answer; found grants count, absence does not.
    pub incomplete: bool,
}

impl EntitlementsRepository {
    /// Discovers publisher-signed grants issued without a campaign by querying
    /// grant events addressed to `buyer` (`#p`).
    ///
    /// Each chain is grouped by coordinate and grant `d`, merged with any
    /// stored events, resolved with the NIP-103 chain rules, and validated
    /// with [`crate::entitlements::validate_campaignless_entitlement`]:
    /// publisher-signed root, no `authorization` anchor, publisher-only
    /// revocation. Only resolved chain members are stored.
    ///
    /// Relay evidence is positive only. A found grant is stored even if some
    /// relays did not answer; a grant that is not found is never removed.
    /// Campaign-anchored grants are ignored here.
    pub async fn discover_campaignless_grants(
        &self,
        buyer: PublicKey,
        source: &dyn EntitlementSource,
        ensure_current: &(dyn Fn() -> Result<(), String> + Send + Sync),
    ) -> Result<CampaignlessDiscoveryReport, String> {
        use crate::entitlements::{
            parse_entitlement_event, resolve_entitlement_grant, validate_campaignless_entitlement,
            GrantStatus,
        };
        ensure_current()?;
        let mut report = CampaignlessDiscoveryReport::default();
        let evidence = source
            .query(
                Filter::new()
                    .kind(Kind::Custom(ENTITLEMENT_GRANT_KIND))
                    .pubkey(buyer),
            )
            .await;
        ensure_current()?;
        let evidence = match evidence {
            Ok(evidence) => evidence,
            Err(_) => {
                report.unavailable = true;
                report.incomplete = true;
                return Ok(report);
            }
        };
        report.incomplete = evidence.incomplete;

        let mut chains: std::collections::BTreeMap<(String, String), Vec<_>> =
            std::collections::BTreeMap::new();
        for event in &evidence.events {
            match parse_entitlement_event(event) {
                Ok(parsed) if parsed.recipient == buyer => chains
                    .entry((parsed.coordinate.clone(), parsed.grant_id.clone()))
                    .or_default()
                    .push(parsed),
                Ok(_) => {}
                Err(_) => report.malformed_events += 1,
            }
        }

        let buyer_hex = buyer.to_hex();
        for ((coordinate, grant_id), mut nodes) in chains {
            ensure_current()?;
            // Campaign-anchored grants belong to the claim flow.
            if !nodes
                .iter()
                .any(|node| node.predecessor.is_none() && node.source_event.is_none())
            {
                continue;
            }
            report.examined += 1;
            let stored = self
                .stored_grant_events(&buyer_hex, &coordinate, &grant_id)
                .await
                .map_err(|e| e.to_string())?;
            let stored_ids = stored
                .iter()
                .map(|node| node.event.id)
                .collect::<std::collections::HashSet<_>>();
            let previously_granted = !stored.is_empty()
                && resolve_entitlement_grant(&stored)
                    .is_ok_and(|chain| chain.status() == Some(GrantStatus::Granted));
            nodes.extend(stored);
            let Ok(resolved) = resolve_entitlement_grant(&nodes) else {
                report.rejected_chains += 1;
                continue;
            };
            if validate_campaignless_entitlement(&resolved).is_err() {
                report.rejected_chains += 1;
                continue;
            }
            let mut inserted_root = false;
            for member in &resolved.events {
                if stored_ids.contains(&member.event.id) {
                    continue;
                }
                ensure_current()?;
                self.ingest_campaignless_event(&member.event)
                    .await
                    .map_err(|e| e.to_string())?;
                inserted_root |= member.predecessor.is_none();
            }
            report.discovered += usize::from(inserted_root);
            if previously_granted && resolved.status() == Some(GrantStatus::Revoked) {
                report.revoked += 1;
            }
        }
        ensure_current()?;
        Ok(report)
    }
}
