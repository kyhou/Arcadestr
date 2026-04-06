use crate::marketplace::Nip15Stall;
use crate::nostr::GameListing;
use sqlx::{Pool, Row, Sqlite};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpsertOutcome {
    Inserted,
    Updated,
    Unchanged,
}

pub struct MarketplaceCache {
    db: Pool<Sqlite>,
}

impl MarketplaceCache {
    pub fn new(db: Pool<Sqlite>) -> Self {
        Self { db }
    }

    pub async fn load_listings(
        &self,
        limit: usize,
        since_days: Option<u64>,
    ) -> Result<Vec<GameListing>, sqlx::Error> {
        let since_cutoff = since_days.map(|days| {
            let now = now_secs();
            now.saturating_sub((days as i64) * 86_400)
        });

        let rows = if let Some(cutoff) = since_cutoff {
            sqlx::query(
                r#"
                SELECT product_id, title, description, price_sats, download_url,
                       publisher_npub, created_at, tags_json, lud16
                FROM marketplace_products
                WHERE created_at >= ?
                ORDER BY updated_at DESC
                LIMIT ?
                "#,
            )
            .bind(cutoff)
            .bind(limit as i64)
            .fetch_all(&self.db)
            .await?
        } else {
            sqlx::query(
                r#"
                SELECT product_id, title, description, price_sats, download_url,
                       publisher_npub, created_at, tags_json, lud16
                FROM marketplace_products
                ORDER BY updated_at DESC
                LIMIT ?
                "#,
            )
            .bind(limit as i64)
            .fetch_all(&self.db)
            .await?
        };

        Ok(rows
            .into_iter()
            .map(|row| {
                let tags_json: String = row.get("tags_json");
                let tags: Vec<String> = serde_json::from_str(&tags_json).unwrap_or_default();

                GameListing {
                    id: row.get("product_id"),
                    title: row.get("title"),
                    description: row.get("description"),
                    price_sats: row.get::<i64, _>("price_sats").max(0) as u64,
                    download_url: row.get("download_url"),
                    publisher_npub: row.get("publisher_npub"),
                    created_at: row.get::<i64, _>("created_at").max(0) as u64,
                    tags,
                    lud16: row.get("lud16"),
                }
            })
            .collect())
    }

    pub async fn upsert_stalls(&self, stalls: &[Nip15Stall]) -> Result<(), sqlx::Error> {
        let now = now_secs();
        let mut tx = self.db.begin().await?;

        for stall in stalls {
            let shipping_json = serde_json::to_string(&stall.shipping).unwrap_or_else(|_| "[]".to_string());

            sqlx::query(
                r#"
                INSERT INTO marketplace_stalls (
                    merchant_npub, stall_id, name, description, currency,
                    shipping_json, created_at, updated_at
                )
                VALUES (?, ?, ?, ?, ?, ?, ?, ?)
                ON CONFLICT(merchant_npub, stall_id) DO UPDATE SET
                    name = excluded.name,
                    description = excluded.description,
                    currency = excluded.currency,
                    shipping_json = excluded.shipping_json,
                    created_at = excluded.created_at,
                    updated_at = excluded.updated_at
                "#,
            )
            .bind(&stall.merchant_npub)
            .bind(&stall.id)
            .bind(&stall.name)
            .bind(&stall.description)
            .bind(&stall.currency)
            .bind(shipping_json)
            .bind(stall.created_at as i64)
            .bind(now)
            .execute(&mut *tx)
            .await?;
        }

        tx.commit().await?;
        Ok(())
    }

    pub async fn upsert_listing(
        &self,
        listing: &GameListing,
        source_event_id: Option<&str>,
    ) -> Result<UpsertOutcome, sqlx::Error> {
        let existed = sqlx::query(
            r#"
            SELECT 1
            FROM marketplace_products
            WHERE publisher_npub = ? AND product_id = ?
            "#,
        )
        .bind(&listing.publisher_npub)
        .bind(&listing.id)
        .fetch_optional(&self.db)
        .await?
        .is_some();

        let tags_json = serde_json::to_string(&listing.tags).unwrap_or_else(|_| "[]".to_string());
        let now = now_secs();

        let result = sqlx::query(
            r#"
            INSERT INTO marketplace_products (
                publisher_npub, product_id, title, description, price_sats,
                download_url, tags_json, lud16, created_at, updated_at, source_event_id
            )
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            ON CONFLICT(publisher_npub, product_id) DO UPDATE SET
                title = excluded.title,
                description = excluded.description,
                price_sats = excluded.price_sats,
                download_url = excluded.download_url,
                tags_json = excluded.tags_json,
                lud16 = excluded.lud16,
                created_at = excluded.created_at,
                updated_at = excluded.updated_at,
                source_event_id = excluded.source_event_id
            WHERE
                marketplace_products.title <> excluded.title OR
                marketplace_products.description <> excluded.description OR
                marketplace_products.price_sats <> excluded.price_sats OR
                marketplace_products.download_url <> excluded.download_url OR
                marketplace_products.tags_json <> excluded.tags_json OR
                marketplace_products.lud16 <> excluded.lud16 OR
                marketplace_products.created_at <> excluded.created_at OR
                IFNULL(marketplace_products.source_event_id, '') <> IFNULL(excluded.source_event_id, '')
            "#,
        )
        .bind(&listing.publisher_npub)
        .bind(&listing.id)
        .bind(&listing.title)
        .bind(&listing.description)
        .bind(listing.price_sats as i64)
        .bind(&listing.download_url)
        .bind(tags_json)
        .bind(&listing.lud16)
        .bind(listing.created_at as i64)
        .bind(now)
        .bind(source_event_id)
        .execute(&self.db)
        .await?;

        let affected = result.rows_affected();
        if affected == 0 {
            return Ok(UpsertOutcome::Unchanged);
        }

        if existed {
            Ok(UpsertOutcome::Updated)
        } else {
            Ok(UpsertOutcome::Inserted)
        }
    }
}

fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::Database;
    use tempfile::TempDir;

    fn make_listing(id: &str, created_at: u64, title: &str) -> GameListing {
        GameListing {
            id: id.to_string(),
            title: title.to_string(),
            description: "desc".to_string(),
            price_sats: 100,
            download_url: "https://example.com".to_string(),
            publisher_npub: "npub1merchant".to_string(),
            created_at,
            tags: vec!["rpg".to_string()],
            lud16: "merchant@example.com".to_string(),
        }
    }

    #[tokio::test]
    async fn upsert_and_load_roundtrip() {
        let temp_dir = TempDir::new().expect("temp dir should be created");
        let db_path = temp_dir.path().join("marketplace_cache.db");
        let db = Database::new(&db_path)
            .await
            .expect("test database should initialize");

        let cache = MarketplaceCache::new(db.pool().clone());
        let listing = make_listing("game-1", 1_710_000_000, "Game One");

        let outcome = cache
            .upsert_listing(&listing, None)
            .await
            .expect("upsert should succeed");
        assert_eq!(outcome, UpsertOutcome::Inserted);

        let loaded = cache
            .load_listings(10, None)
            .await
            .expect("load should succeed");
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].id, "game-1");
    }

    #[tokio::test]
    async fn upsert_detects_unchanged_and_updated() {
        let temp_dir = TempDir::new().expect("temp dir should be created");
        let db_path = temp_dir.path().join("marketplace_cache.db");
        let db = Database::new(&db_path)
            .await
            .expect("test database should initialize");

        let cache = MarketplaceCache::new(db.pool().clone());
        let listing = make_listing("game-2", 1_710_000_001, "Game Two");

        let first = cache
            .upsert_listing(&listing, Some("event-1"))
            .await
            .expect("first upsert should succeed");
        assert_eq!(first, UpsertOutcome::Inserted);

        let unchanged = cache
            .upsert_listing(&listing, Some("event-1"))
            .await
            .expect("second upsert should succeed");
        assert_eq!(unchanged, UpsertOutcome::Unchanged);

        let mut updated_listing = listing.clone();
        updated_listing.title = "Game Two Updated".to_string();
        let updated = cache
            .upsert_listing(&updated_listing, Some("event-2"))
            .await
            .expect("updated upsert should succeed");
        assert_eq!(updated, UpsertOutcome::Updated);
    }

    #[tokio::test]
    async fn load_respects_since_days() {
        let temp_dir = TempDir::new().expect("temp dir should be created");
        let db_path = temp_dir.path().join("marketplace_cache.db");
        let db = Database::new(&db_path)
            .await
            .expect("test database should initialize");

        let cache = MarketplaceCache::new(db.pool().clone());
        let now = now_secs().max(86_400) as u64;
        let old = now.saturating_sub(40 * 86_400);
        let recent = now.saturating_sub(2 * 86_400);

        cache
            .upsert_listing(&make_listing("old", old, "Old"), None)
            .await
            .expect("old upsert should succeed");
        cache
            .upsert_listing(&make_listing("recent", recent, "Recent"), None)
            .await
            .expect("recent upsert should succeed");

        let loaded = cache
            .load_listings(10, Some(30))
            .await
            .expect("load should succeed");

        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].id, "recent");
    }
}
