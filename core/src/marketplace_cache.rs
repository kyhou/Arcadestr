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
                       publisher_npub, created_at, tags_json, lud16,
                       images_json, summary, published_at, location, geohash, status
                FROM marketplace_listings
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
                       publisher_npub, created_at, tags_json, lud16,
                       images_json, summary, published_at, location, geohash, status
                FROM marketplace_listings
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
                let images_json: String = row.get("images_json");
                let images: Vec<String> = serde_json::from_str(&images_json).unwrap_or_default();

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
                    images,
                    summary: row.get("summary"),
                    published_at: row
                        .get::<Option<i64>, _>("published_at")
                        .map(|v| v.max(0) as u64),
                    location: row.get("location"),
                    geohash: row.get("geohash"),
                    status: row.get("status"),
                }
            })
            .collect())
    }

    pub async fn upsert_listing(
        &self,
        listing: &GameListing,
        source_event_id: Option<&str>,
    ) -> Result<UpsertOutcome, sqlx::Error> {
        let existed = sqlx::query(
            r#"
            SELECT 1
            FROM marketplace_listings
            WHERE publisher_npub = ? AND product_id = ?
            "#,
        )
        .bind(&listing.publisher_npub)
        .bind(&listing.id)
        .fetch_optional(&self.db)
        .await?
        .is_some();

        let tags_json = serde_json::to_string(&listing.tags).unwrap_or_else(|_| "[]".to_string());
        let images_json =
            serde_json::to_string(&listing.images).unwrap_or_else(|_| "[]".to_string());
        let now = now_secs();

        let result = sqlx::query(
            r#"
            INSERT INTO marketplace_listings (
                publisher_npub, product_id, title, description, price_sats,
                download_url, tags_json, lud16, created_at, updated_at, source_event_id,
                images_json, summary, published_at, location, geohash, status
            )
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            ON CONFLICT(publisher_npub, product_id) DO UPDATE SET
                title = excluded.title,
                description = excluded.description,
                price_sats = excluded.price_sats,
                download_url = excluded.download_url,
                tags_json = excluded.tags_json,
                lud16 = excluded.lud16,
                created_at = excluded.created_at,
                updated_at = excluded.updated_at,
                source_event_id = excluded.source_event_id,
                images_json = excluded.images_json,
                summary = excluded.summary,
                published_at = excluded.published_at,
                location = excluded.location,
                geohash = excluded.geohash,
                status = excluded.status
            WHERE
                marketplace_listings.title <> excluded.title OR
                marketplace_listings.description <> excluded.description OR
                marketplace_listings.price_sats <> excluded.price_sats OR
                marketplace_listings.download_url <> excluded.download_url OR
                marketplace_listings.tags_json <> excluded.tags_json OR
                marketplace_listings.lud16 <> excluded.lud16 OR
                marketplace_listings.created_at <> excluded.created_at OR
                IFNULL(marketplace_listings.source_event_id, '') <> IFNULL(excluded.source_event_id, '') OR
                marketplace_listings.images_json <> excluded.images_json OR
                IFNULL(marketplace_listings.summary, '') <> IFNULL(excluded.summary, '') OR
                IFNULL(marketplace_listings.published_at, 0) <> IFNULL(excluded.published_at, 0) OR
                IFNULL(marketplace_listings.location, '') <> IFNULL(excluded.location, '') OR
                IFNULL(marketplace_listings.geohash, '') <> IFNULL(excluded.geohash, '') OR
                IFNULL(marketplace_listings.status, '') <> IFNULL(excluded.status, '')
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
        .bind(images_json)
        .bind(&listing.summary)
        .bind(listing.published_at.map(|v| v as i64))
        .bind(&listing.location)
        .bind(&listing.geohash)
        .bind(&listing.status)
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
            images: vec!["https://example.com/image1.png".to_string()],
            summary: Some("A test game".to_string()),
            published_at: Some(created_at),
            location: Some("Online".to_string()),
            geohash: None,
            status: Some("active".to_string()),
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

    #[tokio::test]
    async fn test_upsert_and_load_complete_nip99_listing() {
        let temp_dir = TempDir::new().expect("temp dir should be created");
        let db_path = temp_dir.path().join("marketplace_cache.db");
        let db = Database::new(&db_path)
            .await
            .expect("test database should initialize");

        let cache = MarketplaceCache::new(db.pool().clone());

        // Create a GameListing with all NIP-99 fields populated
        let listing = GameListing {
            id: "complete-game-v1".to_string(),
            title: "Complete NIP-99 Game".to_string(),
            description: "A fully featured game with all NIP-99 fields".to_string(),
            price_sats: 5000,
            download_url: "https://example.com/download".to_string(),
            publisher_npub: "npub1completepublisher".to_string(),
            created_at: 1_710_000_000,
            tags: vec![
                "rpg".to_string(),
                "action".to_string(),
                "multiplayer".to_string(),
            ],
            lud16: "seller@walletofsatoshi.com".to_string(),
            images: vec![
                "https://example.com/image1.png".to_string(),
                "https://example.com/image2.png".to_string(),
                "https://example.com/image3.png".to_string(),
            ],
            summary: Some("Epic adventure awaits".to_string()),
            published_at: Some(1_710_000_000),
            location: Some("San Francisco, CA".to_string()),
            geohash: Some("9q8yym".to_string()),
            status: Some("active".to_string()),
        };

        // Upsert it to cache
        let outcome = cache
            .upsert_listing(&listing, Some("event-complete-1"))
            .await
            .expect("upsert should succeed");
        assert_eq!(outcome, UpsertOutcome::Inserted);

        // Load it back
        let loaded = cache
            .load_listings(10, None)
            .await
            .expect("load should succeed");

        assert_eq!(loaded.len(), 1);
        let loaded_listing = &loaded[0];

        // Assert all fields match exactly
        assert_eq!(loaded_listing.id, listing.id);
        assert_eq!(loaded_listing.title, listing.title);
        assert_eq!(loaded_listing.description, listing.description);
        assert_eq!(loaded_listing.price_sats, listing.price_sats);
        assert_eq!(loaded_listing.download_url, listing.download_url);
        assert_eq!(loaded_listing.publisher_npub, listing.publisher_npub);
        assert_eq!(loaded_listing.created_at, listing.created_at);
        assert_eq!(loaded_listing.tags, listing.tags);
        assert_eq!(loaded_listing.lud16, listing.lud16);
        assert_eq!(loaded_listing.images, listing.images);
        assert_eq!(loaded_listing.summary, listing.summary);
        assert_eq!(loaded_listing.published_at, listing.published_at);
        assert_eq!(loaded_listing.location, listing.location);
        assert_eq!(loaded_listing.geohash, listing.geohash);
        assert_eq!(loaded_listing.status, listing.status);
    }

    #[tokio::test]
    async fn test_listing_with_empty_images() {
        let temp_dir = TempDir::new().expect("temp dir should be created");
        let db_path = temp_dir.path().join("marketplace_cache.db");
        let db = Database::new(&db_path)
            .await
            .expect("test database should initialize");

        let cache = MarketplaceCache::new(db.pool().clone());

        // Create a GameListing with empty images and None for optional fields
        let listing = GameListing {
            id: "minimal-game-v1".to_string(),
            title: "Minimal Game".to_string(),
            description: "A game with minimal fields".to_string(),
            price_sats: 1000,
            download_url: "https://example.com/minimal".to_string(),
            publisher_npub: "npub1minimalpublisher".to_string(),
            created_at: 1_710_000_001,
            tags: vec![],
            lud16: "minimal@example.com".to_string(),
            images: vec![], // Empty images
            summary: None,
            published_at: None,
            location: None,
            geohash: None,
            status: None,
        };

        // Upsert and load
        let outcome = cache
            .upsert_listing(&listing, Some("event-minimal-1"))
            .await
            .expect("upsert should succeed");
        assert_eq!(outcome, UpsertOutcome::Inserted);

        let loaded = cache
            .load_listings(10, None)
            .await
            .expect("load should succeed");

        assert_eq!(loaded.len(), 1);
        let loaded_listing = &loaded[0];

        // Assert empty images and None fields are preserved
        assert_eq!(loaded_listing.id, listing.id);
        assert_eq!(loaded_listing.images, Vec::<String>::new());
        assert!(loaded_listing.images.is_empty());
        assert_eq!(loaded_listing.summary, None);
        assert_eq!(loaded_listing.published_at, None);
        assert_eq!(loaded_listing.location, None);
        assert_eq!(loaded_listing.geohash, None);
        assert_eq!(loaded_listing.status, None);

        // Also verify required fields are correct
        assert_eq!(loaded_listing.title, listing.title);
        assert_eq!(loaded_listing.description, listing.description);
        assert_eq!(loaded_listing.price_sats, listing.price_sats);
        assert_eq!(loaded_listing.download_url, listing.download_url);
        assert_eq!(loaded_listing.publisher_npub, listing.publisher_npub);
        assert_eq!(loaded_listing.created_at, listing.created_at);
        assert_eq!(loaded_listing.tags, Vec::<String>::new());
        assert_eq!(loaded_listing.lud16, listing.lud16);
    }
}
