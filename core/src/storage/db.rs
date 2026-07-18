use sqlx::sqlite::{SqliteConnectOptions, SqlitePool, SqlitePoolOptions};
use sqlx::Row;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};
use thiserror::Error;

// Migration 1: Initial schema (accounts, relay_backups, remote_uris, secure_storage)
const MIGRATION_1_INITIAL: &str = r#"
CREATE TABLE IF NOT EXISTS accounts (
    id TEXT PRIMARY KEY,
    pubkey TEXT UNIQUE NOT NULL,
    npub TEXT UNIQUE NOT NULL,
    signing_mode TEXT NOT NULL CHECK (signing_mode IN ('Local', 'Remote', 'ReadOnly')),
    encrypted_nsec BLOB,
    display_name TEXT,
    picture TEXT,
    created_at INTEGER NOT NULL,
    last_used INTEGER NOT NULL,
    is_active BOOLEAN NOT NULL DEFAULT 0
);

CREATE INDEX IF NOT EXISTS idx_accounts_pubkey ON accounts(pubkey);
CREATE INDEX IF NOT EXISTS idx_accounts_npub ON accounts(npub);
CREATE INDEX IF NOT EXISTS idx_accounts_active ON accounts(is_active) WHERE is_active = 1;
CREATE INDEX IF NOT EXISTS idx_accounts_last_used ON accounts(last_used DESC);

CREATE TABLE IF NOT EXISTS relay_backups (
    id TEXT PRIMARY KEY,
    account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    relay_url TEXT NOT NULL,
    encrypted_data BLOB NOT NULL,
    created_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_backups_account ON relay_backups(account_id);

CREATE TABLE IF NOT EXISTS remote_uris (
    account_id TEXT PRIMARY KEY REFERENCES accounts(id) ON DELETE CASCADE,
    uri TEXT NOT NULL,
    client_key TEXT NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS secure_storage (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    salt BLOB NOT NULL,
    created_at INTEGER NOT NULL
);
"#;

// Migration 2: Marketplace cache table (listings)
const MIGRATION_2_GAMES_TABLE: &str = r#"
CREATE TABLE IF NOT EXISTS marketplace_listings (
    publisher_npub TEXT NOT NULL,
    product_id TEXT NOT NULL,
    title TEXT NOT NULL,
    summary TEXT,
    description TEXT NOT NULL,
    status TEXT,
    published_at INTEGER,
    price_sats INTEGER NOT NULL,
    price_amount TEXT,
    price_currency TEXT,
    price_frequency TEXT,
    download_url TEXT NOT NULL,
    tags_json TEXT NOT NULL DEFAULT '[]',
    images_json TEXT NOT NULL DEFAULT '[]',
    platforms_json TEXT NOT NULL DEFAULT '[]',
    nip94_event_id TEXT,
    lud16 TEXT NOT NULL DEFAULT '',
    location TEXT,
    geohash TEXT,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    source_event_id TEXT,
    PRIMARY KEY (publisher_npub, product_id)
);

CREATE INDEX IF NOT EXISTS idx_marketplace_listings_updated_at
ON marketplace_listings(updated_at DESC);

CREATE INDEX IF NOT EXISTS idx_marketplace_listings_publisher
ON marketplace_listings(publisher_npub);
"#;

// Migration 3: Marketplace indexes
const MIGRATION_3_RELAYS_TABLE: &str = r#"
CREATE INDEX IF NOT EXISTS idx_marketplace_listings_created_at
ON marketplace_listings(created_at DESC);
"#;

// Migration 4: Add users table for profile caching
const MIGRATION_4_USERS_TABLE: &str = r#"
CREATE TABLE IF NOT EXISTS users (
    id INTEGER PRIMARY KEY,
    npub TEXT NOT NULL UNIQUE,
    name TEXT,
    display_name TEXT,
    picture TEXT,
    about TEXT,
    nip05 TEXT,
    lud16 TEXT,
    website TEXT,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    expires_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_users_npub ON users(npub);
CREATE INDEX IF NOT EXISTS idx_users_expires ON users(expires_at);
"#;

// Migration 5: Add complete NIP-99 fields to marketplace listings
// NOTE: This migration is disabled - columns were added directly to MIGRATION_2_GAMES_TABLE
// const MIGRATION_5_NIP99_COMPLETE: &str = r#"
// ALTER TABLE marketplace_listings ADD COLUMN IF NOT EXISTS images_json TEXT NOT NULL DEFAULT '[]';
// ALTER TABLE marketplace_listings ADD COLUMN IF NOT EXISTS location TEXT;
// ALTER TABLE marketplace_listings ADD COLUMN IF NOT EXISTS geohash TEXT;
// "#;

// Migration 5: NIP-58 achievement badge cache
const MIGRATION_5_ACHIEVEMENTS: &str = include_str!("../../migrations/002_achievements.sql");

// Migration 6: NIP-102 purchase receipts
const MIGRATION_6_PURCHASES: &str = include_str!("../../migrations/003_purchases.sql");

// Migration 7: ADP provisioning relationships
const MIGRATION_7_ADP_PROVISIONING: &str =
    include_str!("../../migrations/004_adp_provisioning.sql");

// Migration 8: ADP download token cache
const MIGRATION_8_DOWNLOAD_TOKENS: &str = include_str!("../../migrations/005_download_tokens.sql");

// Migration 9: ADP installed games
const MIGRATION_9_INSTALLED_GAMES: &str = include_str!("../../migrations/006_installed_games.sql");

// Migration 10: NIP-103 entitlement grant history
const MIGRATION_10_ENTITLEMENTS: &str = include_str!("../../migrations/007_entitlements.sql");

// List of all migrations in applied order; migration filenames currently lag user_version numbers.
const MIGRATIONS: &[&str] = &[
    MIGRATION_1_INITIAL,
    MIGRATION_2_GAMES_TABLE,
    MIGRATION_3_RELAYS_TABLE,
    MIGRATION_4_USERS_TABLE,
    MIGRATION_5_ACHIEVEMENTS,
    MIGRATION_6_PURCHASES,
    MIGRATION_7_ADP_PROVISIONING,
    MIGRATION_8_DOWNLOAD_TOKENS,
    MIGRATION_9_INSTALLED_GAMES,
    MIGRATION_10_ENTITLEMENTS,
];

/// Database connection pool for SQLite
pub struct Database {
    pool: SqlitePool,
}

#[derive(Debug, Error)]
pub enum DatabaseError {
    #[error("Connection failed: {0}")]
    Connection(#[from] sqlx::Error),
    #[error("Migration failed: {0}")]
    Migration(String),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

impl Database {
    /// Create a new database connection pool
    pub async fn new(db_path: &Path) -> Result<Self, DatabaseError> {
        // Ensure parent directory exists
        if let Some(parent) = db_path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }

        let connect_options = SqliteConnectOptions::new()
            .filename(db_path)
            .create_if_missing(true)
            .foreign_keys(true);

        let pool = SqlitePoolOptions::new()
            .max_connections(5)
            .min_connections(1)
            .acquire_timeout(std::time::Duration::from_secs(30))
            .connect_with(connect_options)
            .await?;

        // Run migrations
        Self::run_migrations(&pool).await?;

        Ok(Self { pool })
    }

    /// Run database migrations
    async fn run_migrations(pool: &SqlitePool) -> Result<(), DatabaseError> {
        for (idx, migration) in MIGRATIONS.iter().enumerate() {
            let migration_num = idx + 1;
            sqlx::query(*migration).execute(pool).await.map_err(|e| {
                DatabaseError::Migration(format!("Migration {} failed: {}", migration_num, e))
            })?;

            sqlx::query(&format!("PRAGMA user_version = {}", migration_num))
                .execute(pool)
                .await
                .map_err(|e| {
                    DatabaseError::Migration(format!(
                        "Setting schema version for migration {} failed: {}",
                        migration_num, e
                    ))
                })?;
        }

        Self::ensure_marketplace_cache_schema(pool).await?;

        Ok(())
    }

    async fn ensure_marketplace_cache_schema(pool: &SqlitePool) -> Result<(), DatabaseError> {
        let listings_needs_reset = Self::table_needs_reset(
            pool,
            "marketplace_listings",
            &[
                "publisher_npub",
                "product_id",
                "title",
                "summary",
                "description",
                "status",
                "published_at",
                "price_amount",
                "price_currency",
                "price_frequency",
            ],
        )
        .await?;

        if listings_needs_reset {
            sqlx::query("DROP TABLE IF EXISTS marketplace_listings")
                .execute(pool)
                .await
                .map_err(|e| {
                    DatabaseError::Migration(format!(
                        "Failed to reset marketplace_listings table: {}",
                        e
                    ))
                })?;

            sqlx::query(MIGRATION_2_GAMES_TABLE)
                .execute(pool)
                .await
                .map_err(|e| {
                    DatabaseError::Migration(format!(
                        "Failed to recreate marketplace cache tables: {}",
                        e
                    ))
                })?;

            sqlx::query(MIGRATION_3_RELAYS_TABLE)
                .execute(pool)
                .await
                .map_err(|e| {
                    DatabaseError::Migration(format!(
                        "Failed to recreate marketplace cache indexes: {}",
                        e
                    ))
                })?;
        }

        Self::ensure_marketplace_cache_platforms_column(pool).await?;
        Self::ensure_marketplace_cache_nip94_event_id_column(pool).await?;

        Ok(())
    }

    async fn ensure_marketplace_cache_platforms_column(
        pool: &SqlitePool,
    ) -> Result<(), DatabaseError> {
        let column_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM pragma_table_info('marketplace_listings') WHERE name = 'platforms_json'",
        )
        .fetch_one(pool)
        .await
        .map_err(|e| {
            DatabaseError::Migration(format!(
                "Failed checking marketplace platforms_json column: {}",
                e
            ))
        })?;

        if column_count == 0 {
            sqlx::query(
                "ALTER TABLE marketplace_listings ADD COLUMN platforms_json TEXT NOT NULL DEFAULT '[]'",
            )
            .execute(pool)
            .await
            .map_err(|e| {
                DatabaseError::Migration(format!(
                    "Failed adding marketplace platforms_json column: {}",
                    e
                ))
            })?;
        }

        Ok(())
    }

    async fn ensure_marketplace_cache_nip94_event_id_column(
        pool: &SqlitePool,
    ) -> Result<(), DatabaseError> {
        let column_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM pragma_table_info('marketplace_listings') WHERE name = 'nip94_event_id'",
        )
        .fetch_one(pool)
        .await
        .map_err(|e| {
            DatabaseError::Migration(format!(
                "Failed checking marketplace nip94_event_id column: {}",
                e
            ))
        })?;

        if column_count == 0 {
            sqlx::query("ALTER TABLE marketplace_listings ADD COLUMN nip94_event_id TEXT")
                .execute(pool)
                .await
                .map_err(|e| {
                    DatabaseError::Migration(format!(
                        "Failed adding marketplace nip94_event_id column: {}",
                        e
                    ))
                })?;
        }

        Ok(())
    }

    async fn table_needs_reset(
        pool: &SqlitePool,
        table_name: &str,
        required_columns: &[&str],
    ) -> Result<bool, DatabaseError> {
        let exists: Option<i64> = sqlx::query_scalar(
            "SELECT 1 FROM sqlite_master WHERE type='table' AND name = ? LIMIT 1",
        )
        .bind(table_name)
        .fetch_optional(pool)
        .await
        .map_err(|e| {
            DatabaseError::Migration(format!("Failed checking table {}: {}", table_name, e))
        })?;

        if exists.is_none() {
            return Ok(false);
        }

        let pragma = format!("PRAGMA table_info({})", table_name);
        let columns = sqlx::query(&pragma).fetch_all(pool).await.map_err(|e| {
            DatabaseError::Migration(format!("Failed reading schema for {}: {}", table_name, e))
        })?;

        for required in required_columns {
            let has_column = columns
                .iter()
                .any(|row| row.get::<String, _>("name") == *required);
            if !has_column {
                return Ok(true);
            }
        }

        Ok(false)
    }

    /// Get a reference to the connection pool
    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }

    /// Close the database pool
    pub async fn close(&self) {
        self.pool.close().await;
    }

    /// Cache or update a badge definition by coordinate.
    ///
    /// # Errors
    /// Returns `DatabaseError` when SQLite rejects the write.
    pub async fn cache_badge_definition(
        &self,
        definition: &crate::achievements::BadgeDefinition,
        raw_event_json: &str,
    ) -> Result<(), DatabaseError> {
        sqlx::query(
            r#"
            INSERT INTO badge_definitions (
                coordinate, issuer_pubkey, badge_id, event_id, name, description,
                image_url, image_dimensions, thumb_url, thumb_dimensions, relay_url,
                created_at, raw_event_json
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            ON CONFLICT(coordinate) DO UPDATE SET
                issuer_pubkey = excluded.issuer_pubkey,
                badge_id = excluded.badge_id,
                event_id = excluded.event_id,
                name = excluded.name,
                description = excluded.description,
                image_url = excluded.image_url,
                image_dimensions = excluded.image_dimensions,
                thumb_url = excluded.thumb_url,
                thumb_dimensions = excluded.thumb_dimensions,
                relay_url = excluded.relay_url,
                created_at = excluded.created_at,
                raw_event_json = excluded.raw_event_json
            WHERE excluded.created_at > badge_definitions.created_at
               OR (excluded.created_at = badge_definitions.created_at
                   AND excluded.event_id < badge_definitions.event_id)
            "#,
        )
        .bind(&definition.coordinate)
        .bind(&definition.issuer_pubkey)
        .bind(&definition.badge_id)
        .bind(&definition.event_id)
        .bind(&definition.name)
        .bind(&definition.description)
        .bind(&definition.image_url)
        .bind(&definition.image_dimensions)
        .bind(&definition.thumb_url)
        .bind(&definition.thumb_dimensions)
        .bind(&definition.relay_url)
        .bind(unix_to_i64(definition.created_at)?)
        .bind(raw_event_json)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    /// Cache an immutable kind-8 badge award by event id.
    ///
    /// # Errors
    /// Returns `DatabaseError` when SQLite rejects the write.
    pub async fn cache_badge_award(
        &self,
        award: &crate::achievements::BadgeAward,
        raw_event_json: &str,
    ) -> Result<(), DatabaseError> {
        sqlx::query(
            r#"
            INSERT OR IGNORE INTO badge_awards (
                event_id, issuer_pubkey, recipient_pubkey, badge_coordinate,
                relay_url, created_at, raw_event_json
            ) VALUES (?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(&award.event_id)
        .bind(&award.issuer_pubkey)
        .bind(&award.recipient_pubkey)
        .bind(&award.badge_coordinate)
        .bind(&award.relay_url)
        .bind(unix_to_i64(award.created_at)?)
        .bind(raw_event_json)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    /// Replace cached profile badge entries in a single transaction.
    ///
    /// # Errors
    /// Returns `DatabaseError` when SQLite rejects the transaction.
    pub async fn cache_profile_badge_list(
        &self,
        list: &crate::achievements::ProfileBadgeList,
        raw_event_json: &str,
    ) -> Result<(), DatabaseError> {
        let mut tx = self.pool.begin().await?;

        let result = sqlx::query(
            r#"
            INSERT INTO profile_badge_lists (
                profile_pubkey, event_id, kind, created_at, raw_event_json, updated_at
            ) VALUES (?, ?, ?, ?, ?, ?)
            ON CONFLICT(profile_pubkey) DO UPDATE SET
                event_id = excluded.event_id,
                kind = excluded.kind,
                created_at = excluded.created_at,
                raw_event_json = excluded.raw_event_json,
                updated_at = excluded.updated_at
            WHERE (excluded.kind = 10008 AND profile_badge_lists.kind = 30008)
               OR (profile_badge_lists.kind = excluded.kind
                   AND excluded.created_at > profile_badge_lists.created_at)
               OR (profile_badge_lists.kind = excluded.kind
                   AND excluded.created_at = profile_badge_lists.created_at
                   AND excluded.event_id < profile_badge_lists.event_id)
            "#,
        )
        .bind(&list.profile_pubkey)
        .bind(&list.event_id)
        .bind(i64::from(list.kind))
        .bind(unix_to_i64(list.created_at)?)
        .bind(raw_event_json)
        .bind(now_unix_i64()?)
        .execute(&mut *tx)
        .await?;

        if result.rows_affected() == 0 {
            tx.commit().await?;
            return Ok(());
        }

        sqlx::query("DELETE FROM profile_badge_entries WHERE profile_pubkey = ?")
            .bind(&list.profile_pubkey)
            .execute(&mut *tx)
            .await?;

        for entry in &list.entries {
            sqlx::query(
                r#"
                INSERT INTO profile_badge_entries (
                    profile_pubkey, badge_coordinate, award_event_id, relay_url, display_order
                ) VALUES (?, ?, ?, ?, ?)
                "#,
            )
            .bind(&list.profile_pubkey)
            .bind(&entry.badge_coordinate)
            .bind(&entry.award_event_id)
            .bind(&entry.relay_url)
            .bind(usize_to_i64(entry.display_order)?)
            .execute(&mut *tx)
            .await?;
        }

        tx.commit().await?;
        Ok(())
    }

    /// Return verified earned badges joined to cached definitions.
    ///
    /// # Errors
    /// Returns `DatabaseError` when SQLite query execution fails.
    pub async fn earned_badges_for_profile(
        &self,
        profile_pubkey: &str,
    ) -> Result<Vec<crate::achievements::EarnedBadgeSummary>, DatabaseError> {
        let rows = sqlx::query(EARNED_BADGES_QUERY)
            .bind(profile_pubkey)
            .fetch_all(&self.pool)
            .await?;

        rows.iter().map(earned_badge_summary_from_row).collect()
    }

    /// Return profile-selected badges in display order.
    ///
    /// # Errors
    /// Returns `DatabaseError` when SQLite query execution fails.
    pub async fn profile_badges_for_profile(
        &self,
        profile_pubkey: &str,
    ) -> Result<Vec<crate::achievements::ProfileBadgeEntry>, DatabaseError> {
        let rows = sqlx::query(PROFILE_BADGES_QUERY)
            .bind(profile_pubkey)
            .fetch_all(&self.pool)
            .await?;

        rows.iter().map(profile_badge_entry_from_row).collect()
    }
}

const EARNED_BADGES_QUERY: &str = r#"
    SELECT
        d.coordinate AS definition_coordinate,
        d.issuer_pubkey AS definition_issuer_pubkey,
        d.badge_id AS definition_badge_id,
        d.event_id AS definition_event_id,
        d.name AS definition_name,
        d.description AS definition_description,
        d.image_url AS definition_image_url,
        d.image_dimensions AS definition_image_dimensions,
        d.thumb_url AS definition_thumb_url,
        d.thumb_dimensions AS definition_thumb_dimensions,
        d.relay_url AS definition_relay_url,
        d.created_at AS definition_created_at,
        a.event_id AS award_event_id,
        a.issuer_pubkey AS award_issuer_pubkey,
        a.recipient_pubkey AS award_recipient_pubkey,
        a.badge_coordinate AS award_badge_coordinate,
        a.relay_url AS award_relay_url,
        a.created_at AS award_created_at,
        CASE WHEN pbe.award_event_id IS NULL THEN 0 ELSE 1 END AS visible_on_profile
    FROM badge_awards a
    INNER JOIN badge_definitions d ON d.coordinate = a.badge_coordinate
    LEFT JOIN profile_badge_entries pbe
        ON pbe.profile_pubkey = a.recipient_pubkey
       AND pbe.award_event_id = a.event_id
       AND pbe.badge_coordinate = a.badge_coordinate
    WHERE a.recipient_pubkey = ?
    ORDER BY a.created_at DESC, a.event_id ASC
"#;

const PROFILE_BADGES_QUERY: &str = r#"
    SELECT
        d.coordinate AS definition_coordinate,
        d.issuer_pubkey AS definition_issuer_pubkey,
        d.badge_id AS definition_badge_id,
        d.event_id AS definition_event_id,
        d.name AS definition_name,
        d.description AS definition_description,
        d.image_url AS definition_image_url,
        d.image_dimensions AS definition_image_dimensions,
        d.thumb_url AS definition_thumb_url,
        d.thumb_dimensions AS definition_thumb_dimensions,
        d.relay_url AS definition_relay_url,
        d.created_at AS definition_created_at,
        a.event_id AS award_event_id,
        a.issuer_pubkey AS award_issuer_pubkey,
        a.recipient_pubkey AS award_recipient_pubkey,
        a.badge_coordinate AS award_badge_coordinate,
        a.relay_url AS award_relay_url,
        a.created_at AS award_created_at,
        pbe.display_order AS display_order
    FROM profile_badge_entries pbe
    INNER JOIN badge_definitions d ON d.coordinate = pbe.badge_coordinate
    INNER JOIN badge_awards a
        ON a.event_id = pbe.award_event_id
       AND a.recipient_pubkey = pbe.profile_pubkey
       AND a.badge_coordinate = pbe.badge_coordinate
    WHERE pbe.profile_pubkey = ?
    ORDER BY pbe.display_order ASC
"#;

fn earned_badge_summary_from_row(
    row: &sqlx::sqlite::SqliteRow,
) -> Result<crate::achievements::EarnedBadgeSummary, DatabaseError> {
    Ok(crate::achievements::EarnedBadgeSummary {
        definition: definition_from_row(row)?,
        award: award_from_row(row)?,
        visible_on_profile: row.try_get::<i64, _>("visible_on_profile")? != 0,
    })
}

fn profile_badge_entry_from_row(
    row: &sqlx::sqlite::SqliteRow,
) -> Result<crate::achievements::ProfileBadgeEntry, DatabaseError> {
    Ok(crate::achievements::ProfileBadgeEntry {
        definition: definition_from_row(row)?,
        award: award_from_row(row)?,
        display_order: i64_to_usize(row.try_get::<i64, _>("display_order")?)?,
        visible: true,
    })
}

fn definition_from_row(
    row: &sqlx::sqlite::SqliteRow,
) -> Result<crate::achievements::BadgeDefinition, DatabaseError> {
    Ok(crate::achievements::BadgeDefinition {
        coordinate: row.try_get("definition_coordinate")?,
        issuer_pubkey: row.try_get("definition_issuer_pubkey")?,
        badge_id: row.try_get("definition_badge_id")?,
        name: row.try_get("definition_name")?,
        description: row.try_get("definition_description")?,
        image_url: row.try_get("definition_image_url")?,
        image_dimensions: row.try_get("definition_image_dimensions")?,
        thumb_url: row.try_get("definition_thumb_url")?,
        thumb_dimensions: row.try_get("definition_thumb_dimensions")?,
        relay_url: row.try_get("definition_relay_url")?,
        event_id: row.try_get("definition_event_id")?,
        created_at: i64_to_u64(row.try_get("definition_created_at")?)?,
    })
}

fn award_from_row(
    row: &sqlx::sqlite::SqliteRow,
) -> Result<crate::achievements::BadgeAward, DatabaseError> {
    Ok(crate::achievements::BadgeAward {
        event_id: row.try_get("award_event_id")?,
        issuer_pubkey: row.try_get("award_issuer_pubkey")?,
        recipient_pubkey: row.try_get("award_recipient_pubkey")?,
        badge_coordinate: row.try_get("award_badge_coordinate")?,
        relay_url: row.try_get("award_relay_url")?,
        created_at: i64_to_u64(row.try_get("award_created_at")?)?,
    })
}

fn now_unix_i64() -> Result<i64, DatabaseError> {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| DatabaseError::Migration(format!("System clock before Unix epoch: {e}")))?;
    unix_to_i64(duration.as_secs())
}

fn unix_to_i64(value: u64) -> Result<i64, DatabaseError> {
    i64::try_from(value)
        .map_err(|_| DatabaseError::Migration(format!("Timestamp out of range: {value}")))
}

fn usize_to_i64(value: usize) -> Result<i64, DatabaseError> {
    i64::try_from(value)
        .map_err(|_| DatabaseError::Migration(format!("Display order out of range: {value}")))
}

fn i64_to_usize(value: i64) -> Result<usize, DatabaseError> {
    usize::try_from(value)
        .map_err(|_| DatabaseError::Migration(format!("Display order out of range: {value}")))
}

fn i64_to_u64(value: i64) -> Result<u64, DatabaseError> {
    u64::try_from(value)
        .map_err(|_| DatabaseError::Migration(format!("Timestamp out of range: {value}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[tokio::test]
    async fn database_initializes_successfully() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test.db");

        let db = Database::new(&db_path).await.unwrap();

        // Verify tables exist
        let row: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM sqlite_master WHERE type='table'")
            .fetch_one(db.pool())
            .await
            .unwrap();

        assert!(row.0 >= 4); // accounts, relay_backups, remote_uris, secure_storage
    }

    #[tokio::test]
    async fn migrations_run_idempotently() {
        let temp_dir = TempDir::new().expect("temp dir should be created");
        let db_path = temp_dir.path().join("idempotent.db");

        let first = Database::new(&db_path)
            .await
            .expect("first initialization should succeed");

        let first_tables: (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='marketplace_listings'",
        )
        .fetch_one(first.pool())
        .await
        .expect("table count query should succeed");
        assert_eq!(first_tables.0, 1);
        first.close().await;

        let second = Database::new(&db_path)
            .await
            .expect("second initialization should also succeed");

        let second_tables: (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='marketplace_listings'",
        )
        .fetch_one(second.pool())
        .await
        .expect("table count query should succeed after re-init");

        assert_eq!(second_tables.0, 1);
    }

    #[tokio::test]
    async fn opening_legacy_marketplace_cache_adds_platforms_column_without_dropping_rows() {
        let temp_dir = TempDir::new().expect("temp dir should be created");
        let db_path = temp_dir.path().join("legacy-marketplace-cache.db");

        let legacy_pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(
                SqliteConnectOptions::new()
                    .filename(&db_path)
                    .create_if_missing(true)
                    .foreign_keys(true),
            )
            .await
            .expect("legacy database should open");

        sqlx::query(
            r#"
            CREATE TABLE marketplace_listings (
                publisher_npub TEXT NOT NULL,
                product_id TEXT NOT NULL,
                title TEXT NOT NULL,
                summary TEXT,
                description TEXT NOT NULL,
                status TEXT,
                published_at INTEGER,
                price_sats INTEGER NOT NULL,
                price_amount TEXT,
                price_currency TEXT,
                price_frequency TEXT,
                download_url TEXT NOT NULL,
                tags_json TEXT NOT NULL DEFAULT '[]',
                images_json TEXT NOT NULL DEFAULT '[]',
                lud16 TEXT NOT NULL DEFAULT '',
                location TEXT,
                geohash TEXT,
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL,
                source_event_id TEXT,
                PRIMARY KEY (publisher_npub, product_id)
            )
            "#,
        )
        .execute(&legacy_pool)
        .await
        .expect("legacy marketplace table should be created");

        sqlx::query(
            r#"
            INSERT INTO marketplace_listings (
                publisher_npub, product_id, title, summary, description, status,
                published_at, price_sats, price_amount, price_currency, price_frequency,
                download_url, tags_json, images_json, lud16, location, geohash,
                created_at, updated_at, source_event_id
            )
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind("npub1legacy")
        .bind("legacy-game")
        .bind("Legacy Game")
        .bind("Legacy summary")
        .bind("Legacy description")
        .bind("active")
        .bind(1_710_000_000_i64)
        .bind(2100_i64)
        .bind("2100")
        .bind("SATS")
        .bind("once")
        .bind("https://example.com/legacy.zip")
        .bind(r#"["retro"]"#)
        .bind(r#"["https://example.com/legacy.png"]"#)
        .bind("merchant@example.com")
        .bind("Online")
        .bind("9q8yym")
        .bind(1_710_000_001_i64)
        .bind(1_710_000_002_i64)
        .bind("event-legacy")
        .execute(&legacy_pool)
        .await
        .expect("legacy row should be inserted");
        legacy_pool.close().await;

        let db = Database::new(&db_path)
            .await
            .expect("database upgrade should not drop legacy cache rows");
        let cache = crate::marketplace_cache::MarketplaceCache::new(db.pool().clone());
        let listings = cache
            .load_listings(10, None, None)
            .await
            .expect("legacy row should load through marketplace cache");

        assert_eq!(listings.len(), 1);
        let listing = &listings[0];
        assert_eq!(listing.id, "legacy-game");
        assert_eq!(listing.title, "Legacy Game");
        assert_eq!(listing.publisher_npub, "npub1legacy");
        assert_eq!(listing.platforms, Vec::<String>::new());
        assert_eq!(listing.nip94_event_id, None);
    }

    #[tokio::test]
    async fn schema_version_increments() {
        let temp_dir = TempDir::new().expect("temp dir should be created");
        let db_path = temp_dir.path().join("schema-version.db");
        let db = Database::new(&db_path)
            .await
            .expect("database should initialize");

        let user_version: i64 = sqlx::query_scalar("PRAGMA user_version")
            .fetch_one(db.pool())
            .await
            .expect("pragma user_version query should succeed");

        assert_eq!(user_version as usize, MIGRATIONS.len());
    }

    #[tokio::test]
    async fn adp_gate2_tables_exist_after_migrations() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test.db");
        let db = Database::new(&db_path).await.unwrap();

        for table in ["adp_provisioning", "download_tokens", "installed_games"] {
            let exists: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?",
            )
            .bind(table)
            .fetch_one(db.pool())
            .await
            .unwrap();
            assert_eq!(exists, 1, "{table} table should exist");
        }
    }
}
