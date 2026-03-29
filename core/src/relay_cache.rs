// RelayCache module - SQLite-based storage for NIP-65 relay lists

use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::Mutex;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum RelayCacheError {
    #[error("SQLite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("Lock error")]
    Lock,
}

/// Relay type (write or read)
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum RelayType {
    Write,
    Read,
}

/// Source of relay discovery
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RelayDiscoverySource {
    RelayList,
    SeenOn,
    GlobalFallback,
}

impl std::fmt::Display for RelayType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RelayType::Write => write!(f, "write"),
            RelayType::Read => write!(f, "read"),
        }
    }
}

/// Cached relay list for a pubkey
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CachedRelayList {
    pub pubkey: String,
    pub write_relays: Vec<String>,
    pub read_relays: Vec<String>,
    pub updated_at: u64,
}

/// Relay cache storage
pub struct RelayCache {
    conn: Mutex<Connection>,
}

impl RelayCache {
    /// Create a new relay cache at the given path
    pub fn new<P: AsRef<Path>>(path: P) -> Result<Self, RelayCacheError> {
        let conn = Connection::open(path)?;

        // Create tables
        conn.execute(
            "CREATE TABLE IF NOT EXISTS relay_lists (
                pubkey TEXT PRIMARY KEY,
                write_relays TEXT NOT NULL,
                read_relays TEXT NOT NULL,
                updated_at INTEGER NOT NULL
            )",
            [],
        )?;

        conn.execute(
            "CREATE TABLE IF NOT EXISTS seen_on (
                pubkey TEXT NOT NULL,
                relay_url TEXT NOT NULL,
                last_seen INTEGER NOT NULL,
                PRIMARY KEY (pubkey, relay_url)
            )",
            [],
        )?;

        conn.execute(
            "CREATE TABLE IF NOT EXISTS relay_health (
                relay_url TEXT PRIMARY KEY,
                latency_ms INTEGER NOT NULL,
                error_rate REAL NOT NULL,
                last_checked INTEGER NOT NULL
            )",
            [],
        )?;

        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    /// Get cached relay list for a pubkey
    pub fn get_relay_list(&self, pubkey: &str) -> Option<CachedRelayList> {
        let conn = self.conn.lock().ok()?;

        let mut stmt = conn
            .prepare("SELECT pubkey, write_relays, read_relays, updated_at FROM relay_lists WHERE pubkey = ?")
            .ok()?;

        let result = stmt.query_row([pubkey], |row| {
            let write_relays_json: String = row.get(1)?;
            let read_relays_json: String = row.get(2)?;

            Ok(CachedRelayList {
                pubkey: row.get(0)?,
                write_relays: serde_json::from_str(&write_relays_json).unwrap_or_default(),
                read_relays: serde_json::from_str(&read_relays_json).unwrap_or_default(),
                updated_at: row.get(3)?,
            })
        });

        result.ok()
    }

    /// Save relay list for a pubkey
    pub fn save_relay_list(&self, relay_list: &CachedRelayList) -> Result<(), RelayCacheError> {
        let conn = self.conn.lock().map_err(|_| RelayCacheError::Lock)?;

        let write_relays_json = serde_json::to_string(&relay_list.write_relays)?;
        let read_relays_json = serde_json::to_string(&relay_list.read_relays)?;

        conn.execute(
            "INSERT OR REPLACE INTO relay_lists (pubkey, write_relays, read_relays, updated_at) 
             VALUES (?, ?, ?, ?)",
            rusqlite::params![
                relay_list.pubkey,
                write_relays_json,
                read_relays_json,
                relay_list.updated_at
            ],
        )?;

        Ok(())
    }

    /// Check if relay list is stale (>7 days old)
    pub fn is_stale(&self, pubkey: &str) -> bool {
        let Some(relay_list) = self.get_relay_list(pubkey) else {
            return true;
        };

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);

        let seven_days: u64 = 7 * 24 * 60 * 60;
        now.saturating_sub(relay_list.updated_at) > seven_days
    }

    /// Update seen_on tracker
    pub fn update_seen_on(&self, pubkey: &str, relay_url: &str) -> Result<(), RelayCacheError> {
        let conn = self.conn.lock().map_err(|_| RelayCacheError::Lock)?;

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);

        conn.execute(
            "INSERT OR REPLACE INTO seen_on (pubkey, relay_url, last_seen) VALUES (?, ?, ?)",
            rusqlite::params![pubkey, relay_url, now],
        )?;

        Ok(())
    }

    /// Get relays where we've seen this pubkey's events
    pub fn get_seen_on(&self, pubkey: &str) -> Vec<String> {
        let Ok(conn) = self.conn.lock() else {
            return vec![];
        };

        let Ok(mut stmt) = conn.prepare(
            "SELECT relay_url FROM seen_on WHERE pubkey = ? ORDER BY last_seen DESC LIMIT 10",
        ) else {
            return vec![];
        };

        let relays = stmt
            .query_map([pubkey], |row| row.get(0))
            .ok()
            .map(|rows| rows.filter_map(|r| r.ok()).collect())
            .unwrap_or_default();

        relays
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn test_relay_cache_initialization() {
        let temp_dir = std::env::temp_dir();
        let db_path = temp_dir.join("test_relay_cache.db");

        // Clean up from previous runs
        let _ = fs::remove_file(&db_path);

        let cache = RelayCache::new(db_path.to_str().unwrap()).unwrap();

        // Verify tables exist by attempting a query
        let result = cache.get_relay_list("test_pubkey");
        assert!(result.is_none());

        // Clean up
        let _ = fs::remove_file(&db_path);
    }
}
