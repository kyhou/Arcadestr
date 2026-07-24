use crate::nostr::{GameListing, ListingSource};
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
        until_secs: Option<u64>,
    ) -> Result<Vec<GameListing>, sqlx::Error> {
        let since_cutoff = since_days.map(|days| {
            let now = now_secs();
            now.saturating_sub((days as i64) * 86_400)
        });
        let until_cutoff = until_secs.map(|secs| secs as i64);

        let rows = match (since_cutoff, until_cutoff) {
            (Some(since), Some(until)) => {
                sqlx::query(
                    r#"
                    SELECT product_id, title, description, price_sats, download_url,
                           publisher_npub, created_at, tags_json, lud16,
                           images_json, platforms_json, nip94_event_id, specs_json, acquisition_json, source_event_id, summary, published_at, location, geohash, status
                    FROM marketplace_listings
                    WHERE acquisition_json IS NOT NULL AND created_at >= ? AND created_at <= ?
                    ORDER BY created_at DESC
                    LIMIT ?
                    "#,
                )
                .bind(since)
                .bind(until)
                .bind(limit as i64)
                .fetch_all(&self.db)
                .await?
            }
            (Some(since), None) => {
                sqlx::query(
                    r#"
                    SELECT product_id, title, description, price_sats, download_url,
                           publisher_npub, created_at, tags_json, lud16,
                           images_json, platforms_json, nip94_event_id, specs_json, acquisition_json, source_event_id, summary, published_at, location, geohash, status
                    FROM marketplace_listings
                    WHERE acquisition_json IS NOT NULL AND created_at >= ?
                    ORDER BY created_at DESC
                    LIMIT ?
                    "#,
                )
                .bind(since)
                .bind(limit as i64)
                .fetch_all(&self.db)
                .await?
            }
            (None, Some(until)) => {
                sqlx::query(
                    r#"
                    SELECT product_id, title, description, price_sats, download_url,
                           publisher_npub, created_at, tags_json, lud16,
                           images_json, platforms_json, nip94_event_id, specs_json, acquisition_json, source_event_id, summary, published_at, location, geohash, status
                    FROM marketplace_listings
                    WHERE acquisition_json IS NOT NULL AND created_at <= ?
                    ORDER BY created_at DESC
                    LIMIT ?
                    "#,
                )
                .bind(until)
                .bind(limit as i64)
                .fetch_all(&self.db)
                .await?
            }
            (None, None) => {
                sqlx::query(
                    r#"
                    SELECT product_id, title, description, price_sats, download_url,
                           publisher_npub, created_at, tags_json, lud16,
                           images_json, platforms_json, nip94_event_id, specs_json, acquisition_json, source_event_id, summary, published_at, location, geohash, status
                    FROM marketplace_listings
                    WHERE acquisition_json IS NOT NULL
                    ORDER BY created_at DESC
                    LIMIT ?
                    "#,
                )
                .bind(limit as i64)
                .fetch_all(&self.db)
                .await?
            }
        };

        Ok(rows
            .into_iter()
            .map(|row| {
                let tags_json: String = row.get("tags_json");
                let tags: Vec<String> = serde_json::from_str(&tags_json).unwrap_or_default();
                let images_json: String = row.get("images_json");
                let images: Vec<String> = serde_json::from_str(&images_json).unwrap_or_default();
                let platforms_json: String = row.get("platforms_json");
                let platforms: Vec<String> =
                    serde_json::from_str(&platforms_json).unwrap_or_default();
                let specs_json: String = row.get("specs_json");
                let specs: Vec<(String, String)> =
                    serde_json::from_str(&specs_json).unwrap_or_default();
                let acquisition_json: String = row.get("acquisition_json");
                let acquisition = serde_json::from_str(&acquisition_json).unwrap_or_default();

                GameListing {
                    id: row.get("product_id"),
                    event_id: row.get("source_event_id"),
                    source: ListingSource::Nip99Listing,
                    title: row.get("title"),
                    description: row.get("description"),
                    price_sats: row.get::<i64, _>("price_sats").max(0) as u64,
                    download_url: row.get("download_url"),
                    publisher_npub: row.get("publisher_npub"),
                    created_at: row.get::<i64, _>("created_at").max(0) as u64,
                    tags,
                    specs,
                    lud16: row.get("lud16"),
                    platforms,
                    nip94_event_id: row.get("nip94_event_id"),
                    acquisition,
                    campaigns: Vec::new(),
                    images,
                    summary: row.get("summary"),
                    published_at: row
                        .get::<Option<i64>, _>("published_at")
                        .map(|v| v.max(0) as u64),
                    location: row.get("location"),
                    geohash: row.get("geohash"),
                    status: row.get("status"),
                    #[cfg(debug_assertions)]
                    nip99_raw_event_json: None,
                }
            })
            .collect())
    }

    pub async fn latest_created_at(&self) -> Result<Option<u64>, sqlx::Error> {
        let row = sqlx::query(
            r#"
            SELECT MAX(created_at) AS latest_created_at
            FROM marketplace_listings
            WHERE acquisition_json IS NOT NULL
            "#,
        )
        .fetch_one(&self.db)
        .await?;

        Ok(row
            .get::<Option<i64>, _>("latest_created_at")
            .map(|created_at| created_at.max(0) as u64))
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
        let specs_json = serde_json::to_string(&listing.specs).unwrap_or_else(|_| "[]".to_string());
        let images_json =
            serde_json::to_string(&listing.images).unwrap_or_else(|_| "[]".to_string());
        let platforms_json =
            serde_json::to_string(&listing.platforms).unwrap_or_else(|_| "[]".to_string());
        let acquisition_json =
            serde_json::to_string(&listing.acquisition).unwrap_or_else(|_| "\"Gated\"".to_string());
        let now = now_secs();

        let result = sqlx::query(
            r#"
            INSERT INTO marketplace_listings (
                publisher_npub, product_id, title, description, price_sats,
                download_url, tags_json, specs_json, lud16, created_at, updated_at, source_event_id,
                images_json, platforms_json, nip94_event_id, acquisition_json, summary, published_at, location, geohash, status
            )
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            ON CONFLICT(publisher_npub, product_id) DO UPDATE SET
                title = excluded.title,
                description = excluded.description,
                price_sats = excluded.price_sats,
                download_url = excluded.download_url,
                tags_json = excluded.tags_json,
                specs_json = excluded.specs_json,
                lud16 = excluded.lud16,
                created_at = excluded.created_at,
                updated_at = excluded.updated_at,
                source_event_id = excluded.source_event_id,
                images_json = excluded.images_json,
                platforms_json = excluded.platforms_json,
                nip94_event_id = excluded.nip94_event_id,
                acquisition_json = excluded.acquisition_json,
                summary = excluded.summary,
                published_at = excluded.published_at,
                location = excluded.location,
                geohash = excluded.geohash,
                status = excluded.status
            WHERE
                (
                    marketplace_listings.acquisition_json IS NULL OR
                    excluded.created_at > marketplace_listings.created_at OR
                    (
                        excluded.created_at = marketplace_listings.created_at AND
                        excluded.source_event_id IS NOT NULL AND
                        (
                            marketplace_listings.source_event_id IS NULL OR
                            excluded.source_event_id < marketplace_listings.source_event_id
                        )
                    )
                ) AND
                (
                    marketplace_listings.title <> excluded.title OR
                    marketplace_listings.description <> excluded.description OR
                    marketplace_listings.price_sats <> excluded.price_sats OR
                    marketplace_listings.download_url <> excluded.download_url OR
                    marketplace_listings.tags_json <> excluded.tags_json OR
                    marketplace_listings.specs_json <> excluded.specs_json OR
                    marketplace_listings.lud16 <> excluded.lud16 OR
                    marketplace_listings.created_at <> excluded.created_at OR
                    marketplace_listings.source_event_id IS NOT excluded.source_event_id OR
                    marketplace_listings.images_json <> excluded.images_json OR
                    marketplace_listings.platforms_json <> excluded.platforms_json OR
                    IFNULL(marketplace_listings.nip94_event_id, '') <> IFNULL(excluded.nip94_event_id, '') OR
                    IFNULL(marketplace_listings.acquisition_json, '') <> excluded.acquisition_json OR
                    IFNULL(marketplace_listings.summary, '') <> IFNULL(excluded.summary, '') OR
                    IFNULL(marketplace_listings.published_at, 0) <> IFNULL(excluded.published_at, 0) OR
                    IFNULL(marketplace_listings.location, '') <> IFNULL(excluded.location, '') OR
                    IFNULL(marketplace_listings.geohash, '') <> IFNULL(excluded.geohash, '') OR
                    IFNULL(marketplace_listings.status, '') <> IFNULL(excluded.status, '')
                )
            "#,
        )
        .bind(&listing.publisher_npub)
        .bind(&listing.id)
        .bind(&listing.title)
        .bind(&listing.description)
        .bind(listing.price_sats as i64)
        .bind(&listing.download_url)
        .bind(tags_json)
        .bind(specs_json)
        .bind(&listing.lud16)
        .bind(listing.created_at as i64)
        .bind(now)
        .bind(source_event_id)
        .bind(images_json)
        .bind(platforms_json)
        .bind(&listing.nip94_event_id)
        .bind(acquisition_json)
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
            event_id: None,
            source: ListingSource::Nip99Listing,
            title: title.to_string(),
            description: "desc".to_string(),
            price_sats: 100,
            download_url: "https://example.com".to_string(),
            publisher_npub: "npub1merchant".to_string(),
            created_at,
            tags: vec!["rpg".to_string()],
            specs: Vec::new(),
            lud16: "merchant@example.com".to_string(),
            platforms: Vec::new(),
            nip94_event_id: None,
            acquisition: crate::marketplace::AcquisitionPolicy::Gated,
            campaigns: Vec::new(),
            images: vec!["https://example.com/image1.png".to_string()],
            summary: Some("A test game".to_string()),
            published_at: Some(created_at),
            location: Some("Online".to_string()),
            geohash: None,
            status: Some("active".to_string()),
            #[cfg(debug_assertions)]
            nip99_raw_event_json: None,
        }
    }

    #[tokio::test]
    async fn upsert_new_listing_returns_inserted() {
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
            .load_listings(10, None, None)
            .await
            .expect("load should succeed");
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].id, "game-1");
    }

    #[tokio::test]
    async fn acquisition_policy_round_trips_and_malformed_cache_data_is_gated() {
        let temp_dir = TempDir::new().expect("temp dir should be created");
        let db_path = temp_dir.path().join("marketplace_cache.db");
        let db = Database::new(&db_path)
            .await
            .expect("test database should initialize");
        let cache = MarketplaceCache::new(db.pool().clone());
        let mut listing = make_listing("public-game", 1_710_000_000, "Public Game");
        listing.acquisition = crate::marketplace::AcquisitionPolicy::Public;

        cache
            .upsert_listing(&listing, Some("public-event"))
            .await
            .expect("public listing should be cached");
        let loaded = cache
            .load_listings(10, None, None)
            .await
            .expect("public listing should load");
        assert_eq!(
            loaded[0].acquisition,
            crate::marketplace::AcquisitionPolicy::Public
        );

        sqlx::query("UPDATE marketplace_listings SET acquisition_json = NULL WHERE product_id = ?")
            .bind("public-game")
            .execute(db.pool())
            .await
            .expect("cached acquisition should be cleared to simulate a legacy row");
        assert!(cache
            .load_listings(10, None, None)
            .await
            .expect("incomplete cache should remain readable")
            .is_empty());
        assert_eq!(
            cache
                .upsert_listing(&listing, Some("public-event"))
                .await
                .expect("the signed event should enrich its incomplete cache row"),
            UpsertOutcome::Updated
        );

        sqlx::query(
            "UPDATE marketplace_listings SET acquisition_json = 'invalid' WHERE product_id = ?",
        )
        .bind("public-game")
        .execute(db.pool())
        .await
        .expect("cached acquisition should be corrupted for the test");
        let loaded = cache
            .load_listings(10, None, None)
            .await
            .expect("malformed cached policy should not prevent loading");
        assert_eq!(
            loaded[0].acquisition,
            crate::marketplace::AcquisitionPolicy::Gated
        );
    }

    #[tokio::test]
    async fn upsert_unchanged_listing_returns_unchanged() {
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
    }

    #[tokio::test]
    async fn upsert_changed_listing_returns_updated() {
        let temp_dir = TempDir::new().expect("temp dir should be created");
        let db_path = temp_dir.path().join("marketplace_cache.db");
        let db = Database::new(&db_path)
            .await
            .expect("test database should initialize");

        let cache = MarketplaceCache::new(db.pool().clone());
        let listing = make_listing("game-3", 1_710_000_002, "Game Three");

        let first = cache
            .upsert_listing(&listing, Some("event-1"))
            .await
            .expect("first upsert should succeed");
        assert_eq!(first, UpsertOutcome::Inserted);

        let mut updated_listing = listing.clone();
        updated_listing.title = "Game Three Updated".to_string();
        updated_listing.created_at += 1;
        let updated = cache
            .upsert_listing(&updated_listing, Some("event-2"))
            .await
            .expect("updated upsert should succeed");
        assert_eq!(updated, UpsertOutcome::Updated);
    }

    #[tokio::test]
    async fn upsert_changed_nip94_event_id_returns_updated() {
        let temp_dir = TempDir::new().expect("temp dir should be created");
        let db_path = temp_dir.path().join("marketplace_cache.db");
        let db = Database::new(&db_path)
            .await
            .expect("test database should initialize");
        let cache = MarketplaceCache::new(db.pool().clone());
        let mut listing = make_listing("game1", 1_700_000_000, "Game One");

        let inserted = cache
            .upsert_listing(&listing, Some("event1"))
            .await
            .expect("initial upsert should succeed");
        assert_eq!(inserted, UpsertOutcome::Inserted);

        listing.nip94_event_id = Some("nip94-event-1".to_string());
        listing.created_at += 1;
        let updated = cache
            .upsert_listing(&listing, Some("event1"))
            .await
            .expect("nip94 update should be detected");
        assert_eq!(updated, UpsertOutcome::Updated);

        let loaded = cache
            .load_listings(10, None, None)
            .await
            .expect("listing should reload");
        assert_eq!(loaded[0].nip94_event_id, Some("nip94-event-1".to_string()));
    }

    #[tokio::test]
    async fn stale_listing_does_not_replace_newer_listing() {
        let temp_dir = TempDir::new().expect("temp dir should be created");
        let db_path = temp_dir.path().join("marketplace_cache.db");
        let db = Database::new(&db_path)
            .await
            .expect("test database should initialize");
        let cache = MarketplaceCache::new(db.pool().clone());
        let newer_event_id = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
        let stale_event_id = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let newer = make_listing("game-stale", 1_710_000_100, "Newer title");
        let stale = make_listing("game-stale", 1_710_000_099, "Stale title");

        let inserted = cache
            .upsert_listing(&newer, Some(newer_event_id))
            .await
            .expect("newer listing should insert");
        assert_eq!(inserted, UpsertOutcome::Inserted);
        sqlx::query(
            "UPDATE marketplace_listings SET updated_at = 42 WHERE publisher_npub = ? AND product_id = ?",
        )
        .bind(&newer.publisher_npub)
        .bind(&newer.id)
        .execute(db.pool())
        .await
        .expect("updated_at sentinel should be stored");

        let unchanged = cache
            .upsert_listing(&stale, Some(stale_event_id))
            .await
            .expect("stale upsert should succeed");
        assert_eq!(unchanged, UpsertOutcome::Unchanged);

        let loaded = cache
            .load_listings(1, None, None)
            .await
            .expect("listing should reload");
        assert_eq!(loaded[0].title, "Newer title");
        assert_eq!(loaded[0].event_id.as_deref(), Some(newer_event_id));
        let updated_at: i64 = sqlx::query_scalar(
            "SELECT updated_at FROM marketplace_listings WHERE publisher_npub = ? AND product_id = ?",
        )
        .bind(&newer.publisher_npub)
        .bind(&newer.id)
        .fetch_one(db.pool())
        .await
        .expect("updated_at should reload");
        assert_eq!(updated_at, 42);
    }

    #[tokio::test]
    async fn equal_timestamp_higher_event_id_does_not_replace_lower_event_id() {
        let temp_dir = TempDir::new().expect("temp dir should be created");
        let db_path = temp_dir.path().join("marketplace_cache.db");
        let db = Database::new(&db_path)
            .await
            .expect("test database should initialize");
        let cache = MarketplaceCache::new(db.pool().clone());
        let lower_event_id = "0000000000000000000000000000000000000000000000000000000000000001";
        let higher_event_id = "0000000000000000000000000000000000000000000000000000000000000002";
        let lower = make_listing("game-tie-lower", 1_710_000_200, "Lower ID title");
        let higher = make_listing("game-tie-lower", 1_710_000_200, "Higher ID title");

        cache
            .upsert_listing(&lower, Some(lower_event_id))
            .await
            .expect("lower event ID should insert");
        sqlx::query(
            "UPDATE marketplace_listings SET updated_at = 43 WHERE publisher_npub = ? AND product_id = ?",
        )
        .bind(&lower.publisher_npub)
        .bind(&lower.id)
        .execute(db.pool())
        .await
        .expect("updated_at sentinel should be stored");

        let unchanged = cache
            .upsert_listing(&higher, Some(higher_event_id))
            .await
            .expect("higher event ID upsert should succeed");
        assert_eq!(unchanged, UpsertOutcome::Unchanged);

        let loaded = cache
            .load_listings(1, None, None)
            .await
            .expect("listing should reload");
        assert_eq!(loaded[0].title, "Lower ID title");
        assert_eq!(loaded[0].event_id.as_deref(), Some(lower_event_id));
        let updated_at: i64 = sqlx::query_scalar(
            "SELECT updated_at FROM marketplace_listings WHERE publisher_npub = ? AND product_id = ?",
        )
        .bind(&lower.publisher_npub)
        .bind(&lower.id)
        .fetch_one(db.pool())
        .await
        .expect("updated_at should reload");
        assert_eq!(updated_at, 43);
    }

    #[tokio::test]
    async fn equal_timestamp_lower_event_id_replaces_higher_event_id() {
        let temp_dir = TempDir::new().expect("temp dir should be created");
        let db_path = temp_dir.path().join("marketplace_cache.db");
        let db = Database::new(&db_path)
            .await
            .expect("test database should initialize");
        let cache = MarketplaceCache::new(db.pool().clone());
        let lower_event_id = "0000000000000000000000000000000000000000000000000000000000000001";
        let higher_event_id = "0000000000000000000000000000000000000000000000000000000000000002";
        let higher = make_listing("game-tie-update", 1_710_000_300, "Higher ID title");
        let lower = make_listing("game-tie-update", 1_710_000_300, "Lower ID title");

        cache
            .upsert_listing(&higher, Some(higher_event_id))
            .await
            .expect("higher event ID should insert");
        let updated = cache
            .upsert_listing(&lower, Some(lower_event_id))
            .await
            .expect("lower event ID upsert should succeed");
        assert_eq!(updated, UpsertOutcome::Updated);

        let loaded = cache
            .load_listings(1, None, None)
            .await
            .expect("listing should reload");
        assert_eq!(loaded[0].title, "Lower ID title");
        assert_eq!(loaded[0].event_id.as_deref(), Some(lower_event_id));
    }

    #[tokio::test]
    async fn equal_timestamp_empty_event_id_replaces_null_event_id() {
        let temp_dir = TempDir::new().expect("temp dir should be created");
        let db_path = temp_dir.path().join("marketplace_cache.db");
        let db = Database::new(&db_path)
            .await
            .expect("test database should initialize");
        let cache = MarketplaceCache::new(db.pool().clone());
        let listing = make_listing("game-null-event-id", 1_710_000_400, "Event ID title");

        let inserted = cache
            .upsert_listing(&listing, None)
            .await
            .expect("null event ID should insert");
        assert_eq!(inserted, UpsertOutcome::Inserted);

        let updated = cache
            .upsert_listing(&listing, Some(""))
            .await
            .expect("empty event ID upsert should succeed");
        assert_eq!(updated, UpsertOutcome::Updated);

        let loaded = cache
            .load_listings(1, None, None)
            .await
            .expect("listing should reload");
        assert_eq!(loaded[0].event_id.as_deref(), Some(""));
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
            .load_listings(10, Some(30), None)
            .await
            .expect("load should succeed");

        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].id, "recent");
    }

    #[tokio::test]
    async fn load_listings_returns_all_upserted() {
        let temp_dir = TempDir::new().expect("temp dir should be created");
        let db_path = temp_dir.path().join("marketplace_cache.db");
        let db = Database::new(&db_path)
            .await
            .expect("test database should initialize");

        let cache = MarketplaceCache::new(db.pool().clone());

        for i in 0..5 {
            let listing = make_listing(&format!("game-{}", i), 1_710_000_100 + i, "Any");
            cache
                .upsert_listing(&listing, None)
                .await
                .expect("upsert should succeed");
        }

        let loaded = cache
            .load_listings(10, None, None)
            .await
            .expect("load should succeed");

        assert_eq!(loaded.len(), 5);
    }

    #[tokio::test]
    async fn listing_identity_is_publisher_plus_d_tag() {
        let temp_dir = TempDir::new().expect("temp dir should be created");
        let db_path = temp_dir.path().join("marketplace_cache.db");
        let db = Database::new(&db_path)
            .await
            .expect("test database should initialize");

        let cache = MarketplaceCache::new(db.pool().clone());

        let first = make_listing("game-a", 1_710_001_000, "Game A");
        let second = make_listing("game-b", 1_710_001_001, "Game B");

        cache
            .upsert_listing(&first, None)
            .await
            .expect("first upsert should succeed");
        cache
            .upsert_listing(&second, None)
            .await
            .expect("second upsert should succeed");

        let loaded = cache
            .load_listings(10, None, None)
            .await
            .expect("load should succeed");

        assert_eq!(loaded.len(), 2);
    }

    #[tokio::test]
    async fn listing_ordering_by_created_at_desc() {
        let temp_dir = TempDir::new().expect("temp dir should be created");
        let db_path = temp_dir.path().join("marketplace_cache.db");
        let db = Database::new(&db_path)
            .await
            .expect("test database should initialize");

        let cache = MarketplaceCache::new(db.pool().clone());
        cache
            .upsert_listing(&make_listing("old", 1_710_010_000, "Old"), None)
            .await
            .expect("old upsert should succeed");
        cache
            .upsert_listing(&make_listing("new", 1_710_020_000, "New"), None)
            .await
            .expect("new upsert should succeed");

        let loaded = cache
            .load_listings(10, None, None)
            .await
            .expect("load should succeed");

        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded[0].id, "new");
        assert_eq!(loaded[1].id, "old");
    }

    #[tokio::test]
    async fn load_listings_respects_until_secs_cursor() {
        let temp_dir = TempDir::new().expect("temp dir should be created");
        let db_path = temp_dir.path().join("marketplace_cache.db");
        let db = Database::new(&db_path)
            .await
            .expect("test database should initialize");

        let cache = MarketplaceCache::new(db.pool().clone());
        cache
            .upsert_listing(&make_listing("old", 1_710_010_000, "Old"), None)
            .await
            .expect("old upsert should succeed");
        cache
            .upsert_listing(&make_listing("middle", 1_710_020_000, "Middle"), None)
            .await
            .expect("middle upsert should succeed");
        cache
            .upsert_listing(&make_listing("new", 1_710_030_000, "New"), None)
            .await
            .expect("new upsert should succeed");

        let loaded = cache
            .load_listings(10, None, Some(1_710_019_999))
            .await
            .expect("load should succeed");

        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].id, "old");
    }

    #[tokio::test]
    async fn latest_created_at_returns_none_for_empty_cache() {
        let temp_dir = TempDir::new().expect("temp dir should be created");
        let db_path = temp_dir.path().join("marketplace_cache.db");
        let db = Database::new(&db_path)
            .await
            .expect("test database should initialize");

        let cache = MarketplaceCache::new(db.pool().clone());
        let latest = cache
            .latest_created_at()
            .await
            .expect("latest timestamp query should succeed");

        assert_eq!(latest, None);
    }

    #[tokio::test]
    async fn latest_created_at_returns_newest_cached_listing() {
        let temp_dir = TempDir::new().expect("temp dir should be created");
        let db_path = temp_dir.path().join("marketplace_cache.db");
        let db = Database::new(&db_path)
            .await
            .expect("test database should initialize");

        let cache = MarketplaceCache::new(db.pool().clone());
        cache
            .upsert_listing(&make_listing("old", 1_710_010_000, "Old"), None)
            .await
            .expect("old upsert should succeed");
        cache
            .upsert_listing(&make_listing("new", 1_710_030_000, "New"), None)
            .await
            .expect("new upsert should succeed");
        cache
            .upsert_listing(&make_listing("middle", 1_710_020_000, "Middle"), None)
            .await
            .expect("middle upsert should succeed");

        let latest = cache
            .latest_created_at()
            .await
            .expect("latest timestamp query should succeed");

        assert_eq!(latest, Some(1_710_030_000));
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
            event_id: Some("event-complete-1".to_string()),
            source: ListingSource::Nip99Listing,
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
            specs: vec![("version".to_string(), "1.4.2".to_string())],
            lud16: "seller@walletofsatoshi.com".to_string(),
            platforms: vec!["linux-x86_64".to_string()],
            nip94_event_id: Some("nip94-event-complete".to_string()),
            acquisition: crate::marketplace::AcquisitionPolicy::Gated,
            campaigns: Vec::new(),
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
            #[cfg(debug_assertions)]
            nip99_raw_event_json: None,
        };

        // Upsert it to cache
        let outcome = cache
            .upsert_listing(&listing, Some("event-complete-1"))
            .await
            .expect("upsert should succeed");
        assert_eq!(outcome, UpsertOutcome::Inserted);

        // Load it back
        let loaded = cache
            .load_listings(10, None, None)
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
        assert_eq!(loaded_listing.specs, listing.specs);
        assert_eq!(loaded_listing.event_id, listing.event_id);
        assert_eq!(loaded_listing.lud16, listing.lud16);
        assert_eq!(loaded_listing.images, listing.images);
        assert_eq!(loaded_listing.platforms, listing.platforms);
        assert_eq!(loaded_listing.nip94_event_id, listing.nip94_event_id);
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
            event_id: Some("event-minimal-1".to_string()),
            source: ListingSource::Nip99Listing,
            title: "Minimal Game".to_string(),
            description: "A game with minimal fields".to_string(),
            price_sats: 1000,
            download_url: "https://example.com/minimal".to_string(),
            publisher_npub: "npub1minimalpublisher".to_string(),
            created_at: 1_710_000_001,
            tags: vec![],
            specs: Vec::new(),
            lud16: "minimal@example.com".to_string(),
            platforms: Vec::new(),
            nip94_event_id: None,
            acquisition: crate::marketplace::AcquisitionPolicy::Gated,
            campaigns: Vec::new(),
            images: vec![], // Empty images
            summary: None,
            published_at: None,
            location: None,
            geohash: None,
            status: None,
            #[cfg(debug_assertions)]
            nip99_raw_event_json: None,
        };

        // Upsert and load
        let outcome = cache
            .upsert_listing(&listing, Some("event-minimal-1"))
            .await
            .expect("upsert should succeed");
        assert_eq!(outcome, UpsertOutcome::Inserted);

        let loaded = cache
            .load_listings(10, None, None)
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
