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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntitlementHistoryRecord {
    pub grant_id: String,
    pub game_coordinate: String,
    pub campaign_id: Option<String>,
    pub event_id: String,
    pub acquired_at: u64,
    pub status: Option<GrantStatus>,
    pub validation_error: Option<String>,
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

    pub async fn history_for_buyer(
        &self,
        buyer_pubkey: &str,
    ) -> Result<Vec<EntitlementHistoryRecord>, EntitlementsRepositoryError> {
        let rows = sqlx::query(
            r#"
            SELECT event_id, grant_id, buyer_pubkey, game_coordinate,
                   campaign_root_id, status, created_at, raw_event_json
            FROM entitlement_events
            WHERE buyer_pubkey = ? AND validated = 1
            ORDER BY created_at, event_id
            "#,
        )
        .bind(buyer_pubkey)
        .fetch_all(&self.db)
        .await?;

        let mut groups: HashMap<
            (String, String, String),
            Vec<(String, String, String, String, i64, String)>,
        > = HashMap::new();
        for row in rows {
            let grant_id: String = row.get("grant_id");
            let coordinate: String = row.get("game_coordinate");
            let campaign: String = row.get("campaign_root_id");
            groups
                .entry((grant_id, coordinate.clone(), campaign.clone()))
                .or_default()
                .push((
                    row.get("event_id"),
                    row.get("buyer_pubkey"),
                    coordinate,
                    campaign,
                    row.get("created_at"),
                    row.get("raw_event_json"),
                ));
        }

        let mut history = groups
            .into_iter()
            .map(|((grant_id, _, _), rows)| map_entitlement_history(grant_id, rows))
            .collect::<Vec<_>>();
        history.sort_by(|left, right| {
            right
                .acquired_at
                .cmp(&left.acquired_at)
                .then_with(|| left.grant_id.cmp(&right.grant_id))
        });
        Ok(history)
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

fn map_entitlement_history(
    grant_id: String,
    rows: Vec<(String, String, String, String, i64, String)>,
) -> EntitlementHistoryRecord {
    let fallback = rows.last();
    let fallback_event_id = fallback.map(|row| row.0.clone()).unwrap_or_default();
    let fallback_coordinate = fallback.map(|row| row.2.clone()).unwrap_or_default();
    let fallback_campaign = fallback
        .map(|row| row.3.clone())
        .filter(|id| !id.is_empty());
    let fallback_created_at = fallback.map(|row| row.4.max(0) as u64).unwrap_or_default();

    let parsed = rows
        .iter()
        .map(|(event_id, buyer, coordinate, campaign, _, raw)| {
            let event = serde_json::from_str::<Event>(raw).map_err(|_| ())?;
            let parsed = parse_entitlement_event(&event).map_err(|_| ())?;
            if event.id.to_hex() != *event_id
                || parsed.grant_id != grant_id
                || parsed.recipient.to_hex() != *buyer
                || parsed.coordinate != *coordinate
                || parsed.source_event.to_hex() != *campaign
            {
                return Err(());
            }
            Ok(parsed)
        })
        .collect::<Result<Vec<_>, _>>();

    let Ok(parsed) = parsed else {
        return EntitlementHistoryRecord {
            grant_id,
            game_coordinate: fallback_coordinate,
            campaign_id: fallback_campaign,
            event_id: fallback_event_id,
            acquired_at: fallback_created_at,
            status: None,
            validation_error: Some("Stored promotion claim could not be verified.".to_string()),
        };
    };
    let Ok(resolved) = resolve_entitlement_grant(&parsed) else {
        return EntitlementHistoryRecord {
            grant_id,
            game_coordinate: fallback_coordinate,
            campaign_id: fallback_campaign,
            event_id: fallback_event_id,
            acquired_at: fallback_created_at,
            status: None,
            validation_error: Some("Stored promotion claim could not be verified.".to_string()),
        };
    };
    let Some(root) = resolved.events.first() else {
        return EntitlementHistoryRecord {
            grant_id,
            game_coordinate: fallback_coordinate,
            campaign_id: fallback_campaign,
            event_id: fallback_event_id,
            acquired_at: fallback_created_at,
            status: None,
            validation_error: Some("Stored promotion claim is incomplete.".to_string()),
        };
    };
    let event_id = resolved
        .events
        .last()
        .map(|event| event.event.id.to_hex())
        .unwrap_or_else(|| root.event.id.to_hex());

    EntitlementHistoryRecord {
        grant_id,
        game_coordinate: root.coordinate.clone(),
        campaign_id: Some(root.source_event.to_hex()),
        event_id,
        acquired_at: root.event.created_at.as_secs(),
        status: resolved.status(),
        validation_error: None,
    }
}
