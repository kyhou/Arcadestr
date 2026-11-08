use sha2::{Digest, Sha256};
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
    specs_json TEXT NOT NULL DEFAULT '[]',
    images_json TEXT NOT NULL DEFAULT '[]',
    platforms_json TEXT NOT NULL DEFAULT '[]',
    nip94_event_id TEXT,
    acquisition_json TEXT,
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
const MIGRATION_5_ACHIEVEMENTS: &str = include_str!("../../migrations/005_achievements.sql");

// Migration 6: NIP-102 purchase receipts
const MIGRATION_6_PURCHASES: &str = include_str!("../../migrations/006_purchases.sql");

// Migration 7: ADP provisioning relationships
const MIGRATION_7_ADP_PROVISIONING: &str =
    include_str!("../../migrations/007_adp_provisioning.sql");

// Migration 8: ADP download token cache
const MIGRATION_8_DOWNLOAD_TOKENS: &str = include_str!("../../migrations/008_download_tokens.sql");

// Migration 9: installation snapshots
const MIGRATION_9_INSTALLATIONS: &str = include_str!("../../migrations/009_installations.sql");

// Migration 10: NIP-103 entitlement grant history
const MIGRATION_10_ENTITLEMENTS: &str = include_str!("../../migrations/010_entitlements.sql");

// Migration 11: immutable authorization-root provisioning profile
const MIGRATION_11_AUTHORIZATION_ROOTS: &str =
    include_str!("../../migrations/011_authorization_roots.sql");

// Migration 12: account-scoped user library
const MIGRATION_12_LIBRARY_GAMES: &str = include_str!("../../migrations/012_library_games.sql");

// Migration 13: validated Store Page presentation cache
const MIGRATION_13_STORE_PAGES: &str = include_str!("../../migrations/013_store_pages.sql");

// Migration 14: signed listings required for offline Store Page reciprocity checks
const MIGRATION_14_STORE_PAGE_LISTINGS: &str =
    include_str!("../../migrations/014_store_page_listing_events.sql");

// Migration 15: account-scoped Blossom server settings
const MIGRATION_15_BLOSSOM_SERVER_SETTINGS: &str =
    include_str!("../../migrations/015_blossom_server_settings.sql");

// Migration 16: pinned publisher defaults and device-local launch preferences
const MIGRATION_16_LAUNCH_PROFILE_PREFERENCES: &str =
    include_str!("../../migrations/016_launch_profile_preferences.sql");
const MIGRATION_17_SPLIT_SETTLEMENT: &str =
    include_str!("../../migrations/017_split_settlement.sql");
const MIGRATION_18_PAYMENT_DURABILITY: &str =
    include_str!("../../migrations/018_payment_durability.sql");
const MIGRATION_19_SETTLEMENT_LEG_PROJECTION: &str =
    include_str!("../../migrations/019_settlement_leg_projection.sql");
const MIGRATION_20_CLIENT_REFUND_RECOVERY: &str =
    include_str!("../../migrations/020_client_refund_recovery.sql");
const MIGRATION_21_PURCHASE_EXECUTION: &str =
    include_str!("../../migrations/021_purchase_execution.sql");
const MIGRATION_22_PURCHASE_VERIFICATION: &str =
    include_str!("../../migrations/022_purchase_verification.sql");
const MIGRATION_23_RECEIPT_PORTABILITY: &str =
    include_str!("../../migrations/023_receipt_portability.sql");
const MIGRATION_24_PUBLISHER_RECOVERY: &str =
    include_str!("../../migrations/024_publisher_recovery.sql");
const MIGRATION_25_MARKETPLACE_CAMPAIGNS: &str =
    include_str!("../../migrations/025_marketplace_campaign_pointers.sql");

#[cfg(test)]
const EXTERNAL_MIGRATIONS: &[(usize, &str, &str)] = &[
    (5, "005_achievements.sql", MIGRATION_5_ACHIEVEMENTS),
    (6, "006_purchases.sql", MIGRATION_6_PURCHASES),
    (7, "007_adp_provisioning.sql", MIGRATION_7_ADP_PROVISIONING),
    (8, "008_download_tokens.sql", MIGRATION_8_DOWNLOAD_TOKENS),
    (9, "009_installations.sql", MIGRATION_9_INSTALLATIONS),
    (10, "010_entitlements.sql", MIGRATION_10_ENTITLEMENTS),
    (
        11,
        "011_authorization_roots.sql",
        MIGRATION_11_AUTHORIZATION_ROOTS,
    ),
    (12, "012_library_games.sql", MIGRATION_12_LIBRARY_GAMES),
    (13, "013_store_pages.sql", MIGRATION_13_STORE_PAGES),
    (
        14,
        "014_store_page_listing_events.sql",
        MIGRATION_14_STORE_PAGE_LISTINGS,
    ),
    (
        15,
        "015_blossom_server_settings.sql",
        MIGRATION_15_BLOSSOM_SERVER_SETTINGS,
    ),
    (
        16,
        "016_launch_profile_preferences.sql",
        MIGRATION_16_LAUNCH_PROFILE_PREFERENCES,
    ),
    (
        17,
        "017_split_settlement.sql",
        MIGRATION_17_SPLIT_SETTLEMENT,
    ),
    (
        18,
        "018_payment_durability.sql",
        MIGRATION_18_PAYMENT_DURABILITY,
    ),
    (
        19,
        "019_settlement_leg_projection.sql",
        MIGRATION_19_SETTLEMENT_LEG_PROJECTION,
    ),
    (
        20,
        "020_client_refund_recovery.sql",
        MIGRATION_20_CLIENT_REFUND_RECOVERY,
    ),
    (
        21,
        "021_purchase_execution.sql",
        MIGRATION_21_PURCHASE_EXECUTION,
    ),
    (
        22,
        "022_purchase_verification.sql",
        MIGRATION_22_PURCHASE_VERIFICATION,
    ),
    (
        23,
        "023_receipt_portability.sql",
        MIGRATION_23_RECEIPT_PORTABILITY,
    ),
    (
        24,
        "024_publisher_recovery.sql",
        MIGRATION_24_PUBLISHER_RECOVERY,
    ),
    (
        25,
        "025_marketplace_campaign_pointers.sql",
        MIGRATION_25_MARKETPLACE_CAMPAIGNS,
    ),
];

#[derive(Clone, Copy)]
struct Migration {
    name: &'static str,
    sql: &'static str,
}

// List of all migrations in durable applied order. Names and SQL are migration identity.
const MIGRATIONS: &[Migration] = &[
    Migration {
        name: "001_initial_schema",
        sql: MIGRATION_1_INITIAL,
    },
    Migration {
        name: "002_marketplace_listings",
        sql: MIGRATION_2_GAMES_TABLE,
    },
    Migration {
        name: "003_marketplace_indexes",
        sql: MIGRATION_3_RELAYS_TABLE,
    },
    Migration {
        name: "004_users",
        sql: MIGRATION_4_USERS_TABLE,
    },
    Migration {
        name: "005_achievements.sql",
        sql: MIGRATION_5_ACHIEVEMENTS,
    },
    Migration {
        name: "006_purchases.sql",
        sql: MIGRATION_6_PURCHASES,
    },
    Migration {
        name: "007_adp_provisioning.sql",
        sql: MIGRATION_7_ADP_PROVISIONING,
    },
    Migration {
        name: "008_download_tokens.sql",
        sql: MIGRATION_8_DOWNLOAD_TOKENS,
    },
    Migration {
        name: "009_installations.sql",
        sql: MIGRATION_9_INSTALLATIONS,
    },
    Migration {
        name: "010_entitlements.sql",
        sql: MIGRATION_10_ENTITLEMENTS,
    },
    Migration {
        name: "011_authorization_roots.sql",
        sql: MIGRATION_11_AUTHORIZATION_ROOTS,
    },
    Migration {
        name: "012_library_games.sql",
        sql: MIGRATION_12_LIBRARY_GAMES,
    },
    Migration {
        name: "013_store_pages.sql",
        sql: MIGRATION_13_STORE_PAGES,
    },
    Migration {
        name: "014_store_page_listing_events.sql",
        sql: MIGRATION_14_STORE_PAGE_LISTINGS,
    },
    Migration {
        name: "015_blossom_server_settings.sql",
        sql: MIGRATION_15_BLOSSOM_SERVER_SETTINGS,
    },
    Migration {
        name: "016_launch_profile_preferences.sql",
        sql: MIGRATION_16_LAUNCH_PROFILE_PREFERENCES,
    },
    Migration {
        name: "017_split_settlement.sql",
        sql: MIGRATION_17_SPLIT_SETTLEMENT,
    },
    Migration {
        name: "018_payment_durability.sql",
        sql: MIGRATION_18_PAYMENT_DURABILITY,
    },
    Migration {
        name: "019_settlement_leg_projection.sql",
        sql: MIGRATION_19_SETTLEMENT_LEG_PROJECTION,
    },
    Migration {
        name: "020_client_refund_recovery.sql",
        sql: MIGRATION_20_CLIENT_REFUND_RECOVERY,
    },
    Migration {
        name: "021_purchase_execution.sql",
        sql: MIGRATION_21_PURCHASE_EXECUTION,
    },
    Migration {
        name: "022_purchase_verification.sql",
        sql: MIGRATION_22_PURCHASE_VERIFICATION,
    },
    Migration {
        name: "023_receipt_portability.sql",
        sql: MIGRATION_23_RECEIPT_PORTABILITY,
    },
    Migration {
        name: "024_publisher_recovery.sql",
        sql: MIGRATION_24_PUBLISHER_RECOVERY,
    },
    Migration {
        name: "025_marketplace_campaign_pointers.sql",
        sql: MIGRATION_25_MARKETPLACE_CAMPAIGNS,
    },
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
            .foreign_keys(true)
            // A successful intent/proof commit must survive process/power interruption.
            .synchronous(sqlx::sqlite::SqliteSynchronous::Full);

        let pool = SqlitePoolOptions::new()
            .max_connections(5)
            .min_connections(1)
            .acquire_timeout(std::time::Duration::from_secs(30))
            .connect_with(connect_options)
            .await?;

        // Run migrations
        Self::run_migrations(db_path, &pool, MIGRATIONS).await?;

        Ok(Self { pool })
    }

    /// Run database migrations
    async fn run_migrations(
        db_path: &Path,
        pool: &SqlitePool,
        migrations: &[Migration],
    ) -> Result<(), DatabaseError> {
        let (current_version,): (i64,) = sqlx::query_as("PRAGMA user_version")
            .fetch_one(pool)
            .await
            .map_err(|e| DatabaseError::Migration(format!("Reading schema version failed: {e}")))?;
        let identity_table_exists: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'schema_migrations')",
        )
        .fetch_one(pool)
        .await
        .map_err(|e| {
            DatabaseError::Migration(format!("Reading migration identity state failed: {e}"))
        })?;

        if current_version > 0 && !identity_table_exists {
            return Err(DatabaseError::Migration(format!(
                "database {} predates migration identity tracking (schema version {current_version}); this pre-release database can be deleted and recreated",
                db_path.display()
            )));
        }

        if !identity_table_exists {
            sqlx::query(
                "CREATE TABLE schema_migrations (
                    version INTEGER PRIMARY KEY,
                    name TEXT NOT NULL UNIQUE,
                    sha256 TEXT NOT NULL
                )",
            )
            .execute(pool)
            .await
            .map_err(|e| {
                DatabaseError::Migration(format!("Creating migration identity table failed: {e}"))
            })?;
        }

        Self::validate_migration_identities(pool, current_version, migrations).await?;
        for (idx, migration) in migrations.iter().enumerate().skip(current_version as usize) {
            Self::apply_migration(pool, idx + 1, migration).await?;
        }

        Self::ensure_marketplace_cache_schema(pool).await?;
        Self::ensure_download_tokens_schema(pool).await?;

        Ok(())
    }

    async fn validate_migration_identities(
        pool: &SqlitePool,
        current_version: i64,
        migrations: &[Migration],
    ) -> Result<(), DatabaseError> {
        if current_version < 0 || current_version as usize > migrations.len() {
            return Err(DatabaseError::Migration(format!(
                "schema version {current_version} is outside the known migration range 0..={}",
                migrations.len()
            )));
        }

        let applied: Vec<(i64, String, String)> =
            sqlx::query_as("SELECT version, name, sha256 FROM schema_migrations ORDER BY version")
                .fetch_all(pool)
                .await
                .map_err(|e| {
                    DatabaseError::Migration(format!("Reading migration identities failed: {e}"))
                })?;
        if applied.len() != current_version as usize {
            return Err(DatabaseError::Migration(format!(
                "migration identity count {} does not match schema version {current_version}",
                applied.len()
            )));
        }

        for (index, (version, stored_name, stored_sha256)) in applied.iter().enumerate() {
            let expected_version = index + 1;
            let migration = &migrations[index];
            let expected_sha256 = Self::migration_sha256(migration.sql);
            if *version != expected_version as i64
                || stored_name != migration.name
                || stored_sha256 != &expected_sha256
            {
                return Err(DatabaseError::Migration(format!(
                    "migration {expected_version} identity mismatch: stored name={stored_name:?} sha256={stored_sha256}, embedded name={:?} sha256={expected_sha256}",
                    migration.name
                )));
            }
        }
        Ok(())
    }

    fn migration_sha256(sql: &str) -> String {
        hex::encode(Sha256::digest(sql.as_bytes()))
    }

    async fn apply_migration(
        pool: &SqlitePool,
        migration_num: usize,
        migration: &Migration,
    ) -> Result<(), DatabaseError> {
        let mut transaction = pool.begin().await.map_err(|error| {
            DatabaseError::Migration(format!(
                "Starting migration {migration_num} failed: {error}"
            ))
        })?;
        sqlx::query(migration.sql)
            .execute(&mut *transaction)
            .await
            .map_err(|error| {
                DatabaseError::Migration(format!("Migration {migration_num} failed: {error}"))
            })?;
        sqlx::query("INSERT INTO schema_migrations (version, name, sha256) VALUES (?, ?, ?)")
            .bind(migration_num as i64)
            .bind(migration.name)
            .bind(Self::migration_sha256(migration.sql))
            .execute(&mut *transaction)
            .await
            .map_err(|error| {
                DatabaseError::Migration(format!(
                    "Recording migration {migration_num} identity failed: {error}"
                ))
            })?;
        sqlx::query(&format!("PRAGMA user_version = {migration_num}"))
            .execute(&mut *transaction)
            .await
            .map_err(|error| {
                DatabaseError::Migration(format!(
                    "Setting schema version for migration {migration_num} failed: {error}"
                ))
            })?;
        transaction.commit().await.map_err(|error| {
            DatabaseError::Migration(format!(
                "Committing migration {migration_num} failed: {error}"
            ))
        })?;
        Ok(())
    }

    async fn ensure_download_tokens_schema(pool: &SqlitePool) -> Result<(), DatabaseError> {
        if !Self::table_needs_reset(pool, "download_tokens", &["buyer_pubkey"]).await? {
            return Ok(());
        }

        // Legacy bearer tokens cannot be assigned safely to an account. The cache is disposable.
        sqlx::query("DROP TABLE download_tokens")
            .execute(pool)
            .await
            .map_err(|e| {
                DatabaseError::Migration(format!(
                    "Failed to reset legacy download token cache: {}",
                    e
                ))
            })?;
        sqlx::query(MIGRATION_8_DOWNLOAD_TOKENS)
            .execute(pool)
            .await
            .map_err(|e| {
                DatabaseError::Migration(format!(
                    "Failed to recreate account-scoped download token cache: {}",
                    e
                ))
            })?;
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
        Self::ensure_marketplace_cache_specs_column(pool).await?;
        Self::ensure_marketplace_cache_acquisition_column(pool).await?;

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

    async fn ensure_marketplace_cache_specs_column(pool: &SqlitePool) -> Result<(), DatabaseError> {
        let column_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM pragma_table_info('marketplace_listings') WHERE name = 'specs_json'",
        )
        .fetch_one(pool)
        .await
        .map_err(|error| {
            DatabaseError::Migration(format!(
                "Failed checking marketplace specs_json column: {error}"
            ))
        })?;

        if column_count == 0 {
            sqlx::query(
                "ALTER TABLE marketplace_listings ADD COLUMN specs_json TEXT NOT NULL DEFAULT '[]'",
            )
            .execute(pool)
            .await
            .map_err(|error| {
                DatabaseError::Migration(format!(
                    "Failed adding marketplace specs_json column: {error}"
                ))
            })?;
        }

        Ok(())
    }

    async fn ensure_marketplace_cache_acquisition_column(
        pool: &SqlitePool,
    ) -> Result<(), DatabaseError> {
        let column_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM pragma_table_info('marketplace_listings') WHERE name = 'acquisition_json'",
        )
        .fetch_one(pool)
        .await
        .map_err(|error| {
            DatabaseError::Migration(format!(
                "Failed checking marketplace acquisition_json column: {error}"
            ))
        })?;

        if column_count == 0 {
            sqlx::query("ALTER TABLE marketplace_listings ADD COLUMN acquisition_json TEXT")
                .execute(pool)
                .await
                .map_err(|error| {
                    DatabaseError::Migration(format!(
                        "Failed adding marketplace acquisition_json column: {error}"
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
    async fn legacy_download_tokens_are_replaced_with_account_scoped_cache() {
        let temp_dir = TempDir::new().expect("temp dir should be created");
        let db_path = temp_dir.path().join("legacy-download-tokens.db");
        let options = SqliteConnectOptions::new()
            .filename(&db_path)
            .create_if_missing(true);
        let pool = SqlitePoolOptions::new()
            .connect_with(options)
            .await
            .expect("legacy database should open");
        sqlx::query(
            "CREATE TABLE download_tokens (game_coordinate TEXT NOT NULL, server_url TEXT NOT NULL, token TEXT NOT NULL, expires_at INTEGER NOT NULL, PRIMARY KEY (game_coordinate, server_url))",
        )
        .execute(&pool)
        .await
        .expect("legacy token table should be created");
        sqlx::query("INSERT INTO download_tokens VALUES ('game', 'server', 'token', 100)")
            .execute(&pool)
            .await
            .expect("legacy token should be inserted");
        pool.close().await;

        let db = Database::new(&db_path)
            .await
            .expect("legacy database should migrate");
        let buyer_column_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM pragma_table_info('download_tokens') WHERE name = 'buyer_pubkey'",
        )
        .fetch_one(db.pool())
        .await
        .expect("token schema should be readable");
        let token_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM download_tokens")
            .fetch_one(db.pool())
            .await
            .expect("token cache should be readable");

        assert_eq!(buyer_column_count, 1);
        assert_eq!(token_count, 0);
    }

    #[tokio::test]
    async fn opening_legacy_marketplace_cache_marks_rows_incomplete_without_dropping_them() {
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
        let row: (String, Option<String>) = sqlx::query_as(
            "SELECT title, acquisition_json FROM marketplace_listings WHERE product_id = ?",
        )
        .bind("legacy-game")
        .fetch_one(db.pool())
        .await
        .expect("legacy cache row should be preserved");
        assert_eq!(row.0, "Legacy Game");
        assert_eq!(row.1, None);

        let cache = crate::marketplace_cache::MarketplaceCache::new(db.pool().clone());
        let listings = cache
            .load_listings(10, None, None)
            .await
            .expect("incomplete legacy cache should remain readable");
        assert!(listings.is_empty());
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

    #[test]
    fn migration_filenames_match_applied_numbers() {
        assert_eq!(MIGRATIONS[0].sql, MIGRATION_1_INITIAL);
        assert_eq!(MIGRATIONS[1].sql, MIGRATION_2_GAMES_TABLE);
        assert_eq!(MIGRATIONS[2].sql, MIGRATION_3_RELAYS_TABLE);
        assert_eq!(MIGRATIONS[3].sql, MIGRATION_4_USERS_TABLE);
        for (applied_number, filename, sql) in EXTERNAL_MIGRATIONS {
            let prefix = filename
                .split_once('_')
                .expect("migration filename has a numeric prefix")
                .0
                .parse::<usize>()
                .expect("migration prefix is numeric");
            assert_eq!(prefix, *applied_number, "wrong prefix for {filename}");
            assert_eq!(MIGRATIONS[*applied_number - 1].name, *filename);
            assert_eq!(MIGRATIONS[*applied_number - 1].sql, *sql);
        }
        assert_eq!(EXTERNAL_MIGRATIONS.len() + 4, MIGRATIONS.len());
    }

    #[tokio::test]
    async fn empty_database_migration_creates_expected_schema() {
        let temp_dir = TempDir::new().expect("temp dir should be created");
        let db = Database::new(&temp_dir.path().join("empty-schema.db"))
            .await
            .expect("empty database should migrate");
        let mut tables: Vec<String> = sqlx::query_scalar(
            "SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%' ORDER BY name",
        )
        .fetch_all(db.pool())
        .await
        .expect("schema table query should succeed");
        let mut expected = vec![
            "accounts",
            "adp_provisioning",
            "badge_awards",
            "badge_definitions",
            "blossom_server_settings",
            "client_refund_attempts",
            "client_settlement_progress",
            "download_tokens",
            "entitlement_events",
            "installation_manifest_entries",
            "installation_profiles",
            "installation_updates",
            "installations",
            "library_games",
            "launch_profile_preferences",
            "marketplace_listings",
            "nwc_payment_journal",
            "profile_badge_entries",
            "profile_badge_lists",
            "publisher_checkpoints",
            "publisher_drafts",
            "publisher_operations",
            "purchase_execution_admissions",
            "purchase_execution_approvals",
            "purchase_quote_verification",
            "purchases",
            "receipt_chain_verifications",
            "receipt_sync_cursors",
            "receipt_sync_events",
            "receipt_sync_leg_lsps",
            "relay_backups",
            "remote_uris",
            "schema_migrations",
            "retained_packages",
            "secure_storage",
            "settlement_invoice_attempts",
            "settlement_legs",
            "settlement_quotes",
            "settlement_recovery",
            "settlement_refund_destinations",
            "store_page_listing_events",
            "store_pages",
            "users",
        ]
        .into_iter()
        .map(str::to_string)
        .collect::<Vec<_>>();
        tables.sort();
        expected.sort();
        assert_eq!(tables, expected);

        let installation_sql: String = sqlx::query_scalar(
            "SELECT sql FROM sqlite_master WHERE type = 'table' AND name = 'installations'",
        )
        .fetch_one(db.pool())
        .await
        .expect("installation schema query");
        for state in ["installing", "ready", "failed", "uninstalling"] {
            assert!(installation_sql.contains(state));
        }
        assert!(!installation_sql.contains("installed"));
    }

    #[tokio::test]
    async fn launch_profile_migration_adds_nullable_defaults_and_unlinked_preferences() {
        let temp_dir = TempDir::new().expect("temp directory");
        let db = Database::new(&temp_dir.path().join("launch-profile-schema.db"))
            .await
            .expect("database migration");
        for table in ["installations", "retained_packages"] {
            let column: (i64, Option<String>) = sqlx::query_as(&format!(
                "SELECT [notnull], dflt_value FROM pragma_table_info('{table}') \
                 WHERE name = 'publisher_default_profile_id'"
            ))
            .fetch_one(db.pool())
            .await
            .expect("publisher default column");
            assert_eq!(
                column,
                (0, None),
                "legacy {table} rows must remain nullable"
            );
        }
        let foreign_keys: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM pragma_foreign_key_list('launch_profile_preferences')",
        )
        .fetch_one(db.pool())
        .await
        .expect("preference foreign keys");
        assert_eq!(
            foreign_keys, 0,
            "preferences must survive installation deletion"
        );
    }

    #[tokio::test]
    async fn payment_durability_upgrade_preserves_existing_account_and_invoice_bytes() {
        let directory = TempDir::new().expect("tempdir");
        let path = directory.path().join("upgrade-17.db");
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(
                SqliteConnectOptions::new()
                    .filename(&path)
                    .create_if_missing(true)
                    .foreign_keys(true),
            )
            .await
            .expect("old db");
        Database::run_migrations(&path, &pool, &MIGRATIONS[..17])
            .await
            .expect("version 17");
        sqlx::query("INSERT INTO accounts (id,pubkey,npub,signing_mode,encrypted_nsec,created_at,last_used,is_active) VALUES ('existing','pub','npub','Local',X'0708',1,1,1)")
            .execute(&pool).await.expect("existing account");
        sqlx::query("INSERT INTO settlement_quotes (account_pubkey,quote_id,processor_url,quote_event_json,listing_event_id,state,updated_at) VALUES ('pub','q','https://processor','{}','listing','active',1)")
            .execute(&pool).await.expect("existing quote");
        sqlx::query("INSERT INTO settlement_legs (account_pubkey,quote_id,leg_id,leg_order,recipient_pubkey,amount_msat,state) VALUES ('pub','q','dev',0,'dev',1000,'pending')")
            .execute(&pool).await.expect("existing leg");
        sqlx::query("INSERT INTO settlement_invoice_attempts (account_pubkey,quote_id,leg_id,attempt_id,encrypted_attempt,payment_hash,state,updated_at) VALUES ('pub','q','dev','attempt',X'CAFE','hash','disclosed',1)")
            .execute(&pool).await.expect("existing invoice");
        pool.close().await;
        let upgraded = Database::new(&path).await.expect("non-destructive upgrade");
        let row:(Vec<u8>,Vec<u8>,Option<Vec<u8>>,i64)=sqlx::query_as("SELECT a.encrypted_nsec,i.encrypted_attempt,i.encrypted_proof,a.is_active FROM accounts a JOIN settlement_invoice_attempts i ON i.account_pubkey=a.pubkey WHERE a.id='existing'")
            .fetch_one(upgraded.pool()).await.expect("preserved data");
        assert_eq!(row, (vec![7, 8], vec![0xCA, 0xFE], None, 1));
    }

    #[tokio::test]
    async fn leg_projection_upgrade_preserves_evidence_and_never_promotes_unpaid_quotes() {
        let directory = TempDir::new().expect("tempdir");
        let path = directory.path().join("upgrade-18.db");
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(
                SqliteConnectOptions::new()
                    .filename(&path)
                    .create_if_missing(true)
                    .foreign_keys(true),
            )
            .await
            .expect("old db");
        Database::run_migrations(&path, &pool, &MIGRATIONS[..18])
            .await
            .expect("version 18");
        for (id, state, receipt) in [
            ("paid", "settled", Some("unchanged-signed-receipt")),
            ("partial", "failed", None),
            ("unpaid", "active", None),
        ] {
            sqlx::query("INSERT INTO settlement_quotes (account_pubkey,quote_id,processor_url,quote_event_json,listing_event_id,state,receipt_event_json,updated_at) VALUES ('alice',?,'https://processor','unchanged-signed-quote','listing',?,?,1)").bind(id).bind(state).bind(receipt).execute(&pool).await.expect("quote");
            sqlx::query("INSERT INTO settlement_legs (account_pubkey,quote_id,leg_id,leg_order,recipient_pubkey,amount_msat,state) VALUES ('alice',?,'dev',0,'dev',1000,'pending')").bind(id).execute(&pool).await.expect("leg");
        }
        sqlx::query("INSERT INTO settlement_invoice_attempts (account_pubkey,quote_id,leg_id,attempt_id,encrypted_attempt,payment_hash,state,updated_at,encrypted_proof) VALUES ('alice','partial','dev','original',X'CAFE','original-hash','settled',1,X'0708')").execute(&pool).await.expect("proof");
        pool.close().await;
        let db = Database::new(&path).await.expect("additive upgrade");
        let states: Vec<(String, String)> =
            sqlx::query_as("SELECT quote_id,state FROM settlement_legs ORDER BY quote_id")
                .fetch_all(db.pool())
                .await
                .expect("states");
        assert_eq!(
            states,
            vec![
                ("paid".into(), "settled".into()),
                ("partial".into(), "settled".into()),
                ("unpaid".into(), "pending".into())
            ]
        );
        let quotes: Vec<(String, String)> =
            sqlx::query_as("SELECT quote_id,state FROM settlement_quotes ORDER BY quote_id")
                .fetch_all(db.pool())
                .await
                .expect("quotes");
        assert_eq!(
            quotes,
            vec![
                ("paid".into(), "settled".into()),
                ("partial".into(), "failed".into()),
                ("unpaid".into(), "active".into())
            ]
        );
        let evidence:(Vec<u8>,Vec<u8>)=sqlx::query_as("SELECT encrypted_attempt,encrypted_proof FROM settlement_invoice_attempts WHERE attempt_id='original'").fetch_one(db.pool()).await.expect("evidence");
        assert_eq!(evidence, (vec![0xca, 0xfe], vec![7, 8]));
        let receipt: String = sqlx::query_scalar(
            "SELECT receipt_event_json FROM settlement_quotes WHERE quote_id='paid'",
        )
        .fetch_one(db.pool())
        .await
        .expect("receipt");
        assert_eq!(receipt, "unchanged-signed-receipt");
        let intents: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM nwc_payment_journal")
            .fetch_one(db.pool())
            .await
            .expect("journal");
        assert_eq!(intents, 0);
    }

    #[tokio::test]
    async fn failed_migration_rolls_back_schema_and_version() {
        let temp_dir = TempDir::new().expect("temp dir should be created");
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(
                SqliteConnectOptions::new()
                    .filename(temp_dir.path().join("rollback.db"))
                    .create_if_missing(true),
            )
            .await
            .expect("test pool should open");

        let result = Database::apply_migration(
            &pool,
            1,
            &Migration {
                name: "001_invalid",
                sql: "CREATE TABLE rolled_back (id INTEGER); THIS IS NOT SQL;",
            },
        )
        .await;
        assert!(matches!(result, Err(DatabaseError::Migration(_))));
        let table_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'rolled_back'",
        )
        .fetch_one(&pool)
        .await
        .expect("table lookup should succeed");
        let version: i64 = sqlx::query_scalar("PRAGMA user_version")
            .fetch_one(&pool)
            .await
            .expect("version lookup should succeed");
        assert_eq!(table_count, 0);
        assert_eq!(version, 0);
    }

    #[tokio::test]
    async fn untracked_positive_version_database_fails_with_pre_release_guidance() {
        let temp_dir = TempDir::new().expect("temp dir should be created");
        let db_path = temp_dir.path().join("untracked.db");
        let pool = SqlitePoolOptions::new()
            .connect_with(
                SqliteConnectOptions::new()
                    .filename(&db_path)
                    .create_if_missing(true),
            )
            .await
            .expect("untracked database should open");
        sqlx::query("PRAGMA user_version = 15")
            .execute(&pool)
            .await
            .expect("schema version should be set");
        pool.close().await;

        let error = Database::new(&db_path)
            .await
            .err()
            .expect("untracked database must be rejected");
        let message = error.to_string();
        assert!(message.contains(db_path.to_string_lossy().as_ref()));
        assert!(message.contains("predates migration identity tracking"));
        assert!(message.contains("pre-release database can be deleted"));
    }

    #[tokio::test]
    async fn rewritten_applied_migration_is_rejected() {
        let temp_dir = TempDir::new().expect("temp dir should be created");
        let db_path = temp_dir.path().join("rewritten.db");
        let db = Database::new(&db_path)
            .await
            .expect("database should initialize");
        let pool = db.pool().clone();
        let mut rewritten = MIGRATIONS.to_vec();
        rewritten[8].sql = "SELECT 'rewritten migration 9'";

        let error = Database::run_migrations(&db_path, &pool, &rewritten)
            .await
            .expect_err("rewritten migration must be rejected");
        assert!(error.to_string().contains("migration 9 identity mismatch"));
    }

    #[tokio::test]
    async fn tampered_migration_identity_is_rejected() {
        let temp_dir = TempDir::new().expect("temp dir should be created");
        let db_path = temp_dir.path().join("tampered.db");
        let db = Database::new(&db_path)
            .await
            .expect("database should initialize");
        sqlx::query("UPDATE schema_migrations SET sha256 = 'tampered' WHERE version = 9")
            .execute(db.pool())
            .await
            .expect("migration identity should be tampered");
        db.close().await;

        let error = Database::new(&db_path)
            .await
            .err()
            .expect("tampered identity must be rejected");
        assert!(error.to_string().contains("migration 9 identity mismatch"));
    }

    #[tokio::test]
    async fn adp_gate2_and_store_page_tables_exist_after_migrations() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test.db");
        let db = Database::new(&db_path).await.unwrap();

        for table in [
            "adp_provisioning",
            "download_tokens",
            "installations",
            "installation_profiles",
            "installation_manifest_entries",
            "retained_packages",
            "library_games",
            "store_pages",
            "store_page_listing_events",
        ] {
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
