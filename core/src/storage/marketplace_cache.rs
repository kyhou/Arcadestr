// Marketplace cache repository - SQLite-backed cache for GameListing startup hydration.

use crate::nostr::GameListing;
use sqlx::sqlite::SqlitePool;
use sqlx::Row;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum MarketplaceCacheError {
    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),
    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
}

pub struct MarketplaceCache {
    pool: SqlitePool,
}

impl MarketplaceCache {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn load_cached_products(
        &self,
        limit: usize,
    ) -> Result<Vec<GameListing>, MarketplaceCacheError> {
        let rows = sqlx::query(
            "SELECT payload_json
             FROM marketplace_products
             ORDER BY created_at DESC
             LIMIT ?1",
        )
        .bind(limit as i64)
        .fetch_all(&self.pool)
        .await?;

        let mut listings = Vec::with_capacity(rows.len());
        for row in rows {
            let payload_json: String = row.try_get("payload_json")?;
            let listing: GameListing = serde_json::from_str(&payload_json)?;
            listings.push(listing);
        }

        Ok(listings)
    }

    pub async fn load_payload_map(
        &self,
    ) -> Result<std::collections::HashMap<String, String>, MarketplaceCacheError> {
        let rows = sqlx::query("SELECT id, payload_json FROM marketplace_products")
            .fetch_all(&self.pool)
            .await?;

        let mut map = std::collections::HashMap::with_capacity(rows.len());
        for row in rows {
            let id: String = row.try_get("id")?;
            let payload_json: String = row.try_get("payload_json")?;
            map.insert(id, payload_json);
        }

        Ok(map)
    }

    pub async fn upsert_changed(
        &self,
        listings: &[GameListing],
        now_unix: u64,
    ) -> Result<(), MarketplaceCacheError> {
        let mut tx = self.pool.begin().await?;

        for listing in listings {
            let payload_json = serde_json::to_string(listing)?;
            sqlx::query(
                "INSERT INTO marketplace_products (
                    id, payload_json, publisher_npub, created_at, last_seen_at, updated_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT(id) DO UPDATE SET
                    payload_json = excluded.payload_json,
                    publisher_npub = excluded.publisher_npub,
                    created_at = excluded.created_at,
                    last_seen_at = excluded.last_seen_at,
                    updated_at = excluded.updated_at",
            )
            .bind(&listing.id)
            .bind(payload_json)
            .bind(&listing.publisher_npub)
            .bind(listing.created_at as i64)
            .bind(now_unix as i64)
            .bind(now_unix as i64)
            .execute(&mut *tx)
            .await?;
        }

        tx.commit().await?;
        Ok(())
    }

    pub async fn touch_seen(
        &self,
        listing_ids: &[String],
        now_unix: u64,
    ) -> Result<(), MarketplaceCacheError> {
        if listing_ids.is_empty() {
            return Ok(());
        }

        let mut tx = self.pool.begin().await?;
        for listing_id in listing_ids {
            sqlx::query(
                "UPDATE marketplace_products
                 SET last_seen_at = ?1
                 WHERE id = ?2",
            )
            .bind(now_unix as i64)
            .bind(listing_id)
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    pub async fn prune_stale(
        &self,
        max_age_secs: u64,
        now_unix: u64,
    ) -> Result<u64, MarketplaceCacheError> {
        let cutoff = now_unix.saturating_sub(max_age_secs) as i64;
        let result = sqlx::query("DELETE FROM marketplace_products WHERE last_seen_at < ?1")
            .bind(cutoff)
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected())
    }
}
