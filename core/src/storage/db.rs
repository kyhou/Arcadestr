use sqlx::sqlite::{SqlitePool, SqlitePoolOptions};
use std::path::Path;
use thiserror::Error;

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
        // Create tables if they don't exist
        sqlx::query(
            r#"
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
            "#,
        )
        .execute(pool)
        .await
        .map_err(|e| DatabaseError::Migration(e.to_string()))?;

        Ok(())
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
    async fn test_database_creation() {
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
}
