//! Read-only Nostr reconciliation of *previously validated* local grants.
//! Roots never enter through this path. Their original campaign/authorization
//! validation remains historical; cancelling either later is not a revocation.
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
