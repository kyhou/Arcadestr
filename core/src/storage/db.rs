use sqlx::sqlite::{SqlitePool, SqlitePoolOptions};
use sqlx::Row;
use std::path::Path;
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

// List of all migrations in order
const MIGRATIONS: &[&str] = &[
    MIGRATION_1_INITIAL,
    MIGRATION_2_GAMES_TABLE,
    MIGRATION_3_RELAYS_TABLE,
    MIGRATION_4_USERS_TABLE,
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

        let pool = SqlitePoolOptions::new()
            .max_connections(5)
            .min_connections(1)
            .acquire_timeout(std::time::Duration::from_secs(30))
            .connect(&format!("sqlite://{}?mode=rwc", db_path.display()))
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
}
