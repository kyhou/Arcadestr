use std::collections::HashMap;

use nostr::Event;
use sqlx::{Row, SqlitePool};
use thiserror::Error;

use crate::authorization::ResolvedAuthorization;
use crate::campaign::ResolvedCampaign;
use crate::entitlements::{
    parse_entitlement_event, resolve_entitlement_grant, validate_adp_entitlement, EntitlementError,
    GrantStatus, IssuanceDelegation, ParsedEntitlementGrant, ResolvedEntitlementGrant,
};

#[derive(Debug, Error)]
pub enum EntitlementsRepositoryError {
    #[error(transparent)]
    Entitlement(#[from] EntitlementError),
    #[error("invalid entitlement event: {0}")]
    InvalidEvent(String),
    #[error(transparent)]
    Database(#[from] sqlx::Error),
    #[error(transparent)]
    Serialization(#[from] serde_json::Error),
}

#[derive(Clone)]
pub struct EntitlementsRepository {
    db: SqlitePool,
}

impl EntitlementsRepository {
    pub fn new(db: SqlitePool) -> Self {
        Self { db }
    }

    pub async fn ingest_event(
        &self,
        event: &Event,
        campaign: &ResolvedCampaign,
        authorization: Option<&ResolvedAuthorization>,
        delegations: &[IssuanceDelegation],
    ) -> Result<(), EntitlementsRepositoryError> {
        let parsed = parse_entitlement_event(event)
            .map_err(|error| EntitlementsRepositoryError::InvalidEvent(error.to_string()))?;
        let mut nodes = self
            .parsed_events(
                &parsed.recipient.to_hex(),
                &parsed.coordinate,
                Some(&parsed.grant_id),
            )
            .await?;
        nodes.push(parsed.clone());
        let resolved = resolve_entitlement_grant(&nodes)?;
        validate_adp_entitlement(&resolved, campaign, authorization, delegations)?;
        let raw_event_json = serde_json::to_string(event)?;

        sqlx::query(
            r#"
            INSERT INTO entitlement_events (
                event_id, grant_id, buyer_pubkey, game_coordinate,
                campaign_root_id, issuer_pubkey, status, predecessor_event_id,
                created_at, raw_event_json, validated
            )
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 1)
            ON CONFLICT(event_id) DO NOTHING
            "#,
        )
        .bind(event.id.to_hex())
        .bind(&parsed.grant_id)
        .bind(parsed.recipient.to_hex())
        .bind(&parsed.coordinate)
        .bind(parsed.source_event.to_hex())
        .bind(event.pubkey.to_hex())
        .bind(parsed.status.as_str())
        .bind(parsed.predecessor.map(|id| id.to_hex()))
        .bind(event.created_at.as_secs() as i64)
        .bind(raw_event_json)
        .execute(&self.db)
        .await?;
        Ok(())
    }

    pub async fn chains_for(
        &self,
        buyer_pubkey: &str,
        game_coordinate: &str,
    ) -> Result<Vec<ResolvedEntitlementGrant>, EntitlementsRepositoryError> {
        let parsed = self
            .parsed_events(buyer_pubkey, game_coordinate, None)
            .await?;
        let mut groups: HashMap<String, Vec<ParsedEntitlementGrant>> = HashMap::new();
        for event in parsed {
            groups
                .entry(event.grant_id.clone())
                .or_default()
                .push(event);
        }
        Ok(groups
            .into_values()
            .filter_map(|events| resolve_entitlement_grant(&events).ok())
            .collect())
    }

    pub async fn is_owned(
        &self,
        buyer_pubkey: &str,
        game_coordinate: &str,
    ) -> Result<bool, EntitlementsRepositoryError> {
        Ok(self
            .chains_for(buyer_pubkey, game_coordinate)
            .await?
            .iter()
            .any(|grant| grant.status() == Some(GrantStatus::Granted)))
    }

    async fn parsed_events(
        &self,
        buyer_pubkey: &str,
        game_coordinate: &str,
        grant_id: Option<&str>,
    ) -> Result<Vec<ParsedEntitlementGrant>, EntitlementsRepositoryError> {
        let rows = if let Some(grant_id) = grant_id {
            sqlx::query(
                r#"
                SELECT raw_event_json FROM entitlement_events
                WHERE buyer_pubkey = ? AND game_coordinate = ?
                  AND grant_id = ? AND validated = 1
                ORDER BY created_at, event_id
                "#,
            )
            .bind(buyer_pubkey)
            .bind(game_coordinate)
            .bind(grant_id)
            .fetch_all(&self.db)
            .await?
        } else {
            sqlx::query(
                r#"
                SELECT raw_event_json FROM entitlement_events
                WHERE buyer_pubkey = ? AND game_coordinate = ? AND validated = 1
                ORDER BY created_at, event_id
                "#,
            )
            .bind(buyer_pubkey)
            .bind(game_coordinate)
            .fetch_all(&self.db)
            .await?
        };

        Ok(rows
            .into_iter()
            .filter_map(|row| {
                let raw: String = row.get("raw_event_json");
                serde_json::from_str::<Event>(&raw)
                    .ok()
                    .and_then(|event| parse_entitlement_event(&event).ok())
            })
            .collect())
    }
}
