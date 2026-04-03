// RelayCache module - SQLite-based storage for NIP-65 relay lists

// 1. Standard library
use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

// 2. External crates
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tracing::info;

// 3. Internal crate imports
use crate::relay_pool::RelayPool;

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
    RelayList,      // NIP-65 Kind 10002
    SeenOn,         // Events seen on this relay
    RelayHints,     // From p-tag/e-tag relay hints
    GlobalFallback, // Default hardcoded relays
}

impl std::fmt::Display for RelayType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RelayType::Write => write!(f, "write"),
            RelayType::Read => write!(f, "read"),
        }
    }
}

/// Entry for a relay in the pubkey_relays map
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RelayEntry {
    pub url: String,
    pub relay_type: String, // "read" or "write"
    pub last_seen: u64,     // unix timestamp
}

/// Simplified RelayHealth for in-memory storage
#[derive(Debug, Clone)]
pub struct RelayHealthData {
    pub latency_ms: u64,
    pub error_rate: f64,
}

/// Cached relay list for a pubkey
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CachedRelayList {
    pub pubkey: String,
    pub write_relays: Vec<String>,
    pub read_relays: Vec<String>,
    pub updated_at: u64,
}

/// Relay health metrics
#[derive(Debug, Clone)]
pub struct RelayHealth {
    pub relay_url: String,
    pub latency_ms: u32,
    pub error_rate: f32,
    pub last_checked: u64,
    pub total_requests: u32,
    pub failed_requests: u32,
}

/// Relay cache storage
pub struct RelayCache {
    conn: Mutex<Connection>,
    /// In-memory cache: pubkey → list of relay entries
    pub pubkey_relays: Arc<Mutex<HashMap<String, Vec<RelayEntry>>>>,
    /// In-memory cache: relay_url → health data
    pub relay_health: Arc<Mutex<HashMap<String, RelayHealthData>>>,
    /// In-memory cache: pubkey → list of relay URLs where seen
    pub seen_on: Arc<Mutex<HashMap<String, Vec<String>>>>,
    /// Pubkeys that need immediate background refresh (accessed while stale)
    pending_refresh: Arc<Mutex<Vec<String>>>,
    /// Counter for permanent WebSocket connections (max 10)
    pub permanent_connection_count: Arc<AtomicUsize>,
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
                last_checked INTEGER NOT NULL,
                total_requests INTEGER NOT NULL DEFAULT 0,
                failed_requests INTEGER NOT NULL DEFAULT 0
            )",
            [],
        )?;

        conn.execute(
            "CREATE TABLE IF NOT EXISTS seen_events (
                event_id TEXT PRIMARY KEY,
                seen_at INTEGER NOT NULL
            )",
            [],
        )?;

        let cache = Self {
            conn: Mutex::new(conn),
            pubkey_relays: Arc::new(Mutex::new(HashMap::new())),
            relay_health: Arc::new(Mutex::new(HashMap::new())),
            seen_on: Arc::new(Mutex::new(HashMap::new())),
            pending_refresh: Arc::new(Mutex::new(Vec::new())),
            permanent_connection_count: Arc::new(AtomicUsize::new(0)),
        };

        // Load existing data from SQLite into in-memory maps
        cache.load_from_db()?;

        Ok(cache)
    }

    /// Load all existing data from SQLite into in-memory maps
    fn load_from_db(&self) -> Result<(), RelayCacheError> {
        // Load relay_lists into pubkey_relays
        {
            let conn = self.conn.lock().map_err(|_| RelayCacheError::Lock)?;
            let mut stmt = conn
                .prepare("SELECT pubkey, write_relays, read_relays, updated_at FROM relay_lists")?;

            let rows = stmt.query_map([], |row| {
                let pubkey: String = row.get(0)?;
                let write_relays_json: String = row.get(1)?;
                let read_relays_json: String = row.get(2)?;
                let updated_at: u64 = row.get(3)?;

                let write_relays: Vec<String> =
                    serde_json::from_str(&write_relays_json).unwrap_or_default();
                let read_relays: Vec<String> =
                    serde_json::from_str(&read_relays_json).unwrap_or_default();

                Ok((pubkey, write_relays, read_relays, updated_at))
            })?;

            let mut pubkey_relays = self
                .pubkey_relays
                .lock()
                .map_err(|_| RelayCacheError::Lock)?;
            for row in rows {
                if let Ok((pubkey, write_relays, read_relays, updated_at)) = row {
                    let mut entries = Vec::new();
                    for url in write_relays {
                        entries.push(RelayEntry {
                            url,
                            relay_type: "write".to_string(),
                            last_seen: updated_at,
                        });
                    }
                    for url in read_relays {
                        entries.push(RelayEntry {
                            url,
                            relay_type: "read".to_string(),
                            last_seen: updated_at,
                        });
                    }
                    pubkey_relays.insert(pubkey, entries);
                }
            }
        }

        // Load seen_on into memory
        {
            let conn = self.conn.lock().map_err(|_| RelayCacheError::Lock)?;
            let mut stmt =
                conn.prepare("SELECT pubkey, relay_url FROM seen_on ORDER BY last_seen DESC")?;

            let rows = stmt.query_map([], |row| {
                let pubkey: String = row.get(0)?;
                let relay_url: String = row.get(1)?;
                Ok((pubkey, relay_url))
            })?;

            let mut seen_on = self.seen_on.lock().map_err(|_| RelayCacheError::Lock)?;
            for row in rows {
                if let Ok((pubkey, relay_url)) = row {
                    seen_on
                        .entry(pubkey)
                        .or_insert_with(Vec::new)
                        .push(relay_url);
                }
            }
        }

        // Load relay_health into memory
        {
            let conn = self.conn.lock().map_err(|_| RelayCacheError::Lock)?;
            let mut stmt =
                conn.prepare("SELECT relay_url, latency_ms, error_rate FROM relay_health")?;

            let rows = stmt.query_map([], |row| {
                let relay_url: String = row.get(0)?;
                let latency_ms: u32 = row.get(1)?;
                let error_rate: f32 = row.get(2)?;
                Ok((relay_url, latency_ms, error_rate))
            })?;

            let mut relay_health = self
                .relay_health
                .lock()
                .map_err(|_| RelayCacheError::Lock)?;
            for row in rows {
                if let Ok((relay_url, latency_ms, error_rate)) = row {
                    relay_health.insert(
                        relay_url,
                        RelayHealthData {
                            latency_ms: latency_ms as u64,
                            error_rate: error_rate as f64,
                        },
                    );
                }
            }
        }

        Ok(())
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

    /// Get all pubkeys with stale relay lists (>7 days old)
    pub fn get_stale_pubkeys(&self) -> Vec<String> {
        let conn = match self.conn.lock() {
            Ok(c) => c,
            Err(_) => return vec![],
        };

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);

        let seven_days: u64 = 7 * 24 * 60 * 60;
        let threshold = now.saturating_sub(seven_days);

        let mut stmt = match conn.prepare("SELECT pubkey FROM relay_lists WHERE updated_at < ?") {
            Ok(s) => s,
            Err(_) => return vec![],
        };

        let pubkeys: Vec<String> = stmt
            .query_map([threshold], |row| row.get(0))
            .ok()
            .map(|rows| rows.filter_map(|r| r.ok()).collect())
            .unwrap_or_default();

        // Also include pubkeys marked for immediate refresh
        let pending = self.get_and_clear_pending_refresh();
        let mut all_pubkeys = pubkeys;
        all_pubkeys.extend(pending);
        all_pubkeys.dedup();

        all_pubkeys
    }

    /// Mark a pubkey as needing immediate background refresh
    pub fn mark_for_refresh(&self, pubkey: &str) {
        if let Ok(mut pending) = self.pending_refresh.lock() {
            if !pending.contains(&pubkey.to_string()) {
                pending.push(pubkey.to_string());
            }
        }
    }

    /// Get and clear the list of pubkeys pending immediate refresh
    pub fn get_and_clear_pending_refresh(&self) -> Vec<String> {
        if let Ok(mut pending) = self.pending_refresh.lock() {
            let result = pending.clone();
            pending.clear();
            result
        } else {
            vec![]
        }
    }

    /// Update seen_on tracker
    /// Writes to both SQLite (persistence) and in-memory map (performance)
    pub fn update_seen_on(&self, pubkey: &str, relay_url: &str) -> Result<(), RelayCacheError> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);

        // Write to SQLite first (source of truth)
        {
            let conn = self.conn.lock().map_err(|_| RelayCacheError::Lock)?;
            conn.execute(
                "INSERT OR REPLACE INTO seen_on (pubkey, relay_url, last_seen) VALUES (?, ?, ?)",
                rusqlite::params![pubkey, relay_url, now],
            )?;
        }

        // Update in-memory map
        if let Ok(mut seen_on) = self.seen_on.lock() {
            let entries = seen_on.entry(pubkey.to_string()).or_insert_with(Vec::new);
            // Add relay if not already present
            if !entries.contains(&relay_url.to_string()) {
                entries.push(relay_url.to_string());
            }
        }

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

    /// Update relay health after a request
    pub fn update_relay_health(
        &self,
        relay_url: &str,
        latency_ms: u32,
        success: bool,
    ) -> Result<(), RelayCacheError> {
        let conn = self.conn.lock().map_err(|_| RelayCacheError::Lock)?;

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);

        // Get existing stats or use defaults
        let (total, failed) = conn
            .query_row(
                "SELECT total_requests, failed_requests FROM relay_health WHERE relay_url = ?",
                [relay_url],
                |row| Ok((row.get::<_, u32>(0)?, row.get::<_, u32>(1)?)),
            )
            .unwrap_or((0, 0));

        let new_total = total.saturating_add(1);
        let new_failed = if success {
            failed
        } else {
            failed.saturating_add(1)
        };
        let new_error_rate = if new_total > 0 {
            new_failed as f32 / new_total as f32
        } else {
            0.0
        };

        conn.execute(
            "INSERT OR REPLACE INTO relay_health 
             (relay_url, latency_ms, error_rate, last_checked, total_requests, failed_requests) 
             VALUES (?, ?, ?, ?, ?, ?)",
            rusqlite::params![
                relay_url,
                latency_ms,
                new_error_rate,
                now,
                new_total,
                new_failed
            ],
        )?;

        Ok(())
    }

    /// Get health for a specific relay
    pub fn get_relay_health(&self, relay_url: &str) -> Option<RelayHealth> {
        let conn = self.conn.lock().ok()?;

        conn.query_row(
            "SELECT relay_url, latency_ms, error_rate, last_checked, total_requests, failed_requests 
             FROM relay_health WHERE relay_url = ?",
            [relay_url],
            |row| {
                Ok(RelayHealth {
                    relay_url: row.get(0)?,
                    latency_ms: row.get(1)?,
                    error_rate: row.get(2)?,
                    last_checked: row.get(3)?,
                    total_requests: row.get(4)?,
                    failed_requests: row.get(5)?,
                })
            },
        )
        .ok()
    }

    /// Calculate health score (0.0 - 1.0, higher is better)
    pub fn get_health_score(&self, relay_url: &str) -> f32 {
        let Some(health) = self.get_relay_health(relay_url) else {
            return 1.0; // Unknown relays get neutral score
        };

        // Error penalty
        let error_penalty = if health.error_rate > 0.2 { 0.7 } else { 1.0 };

        // Staleness penalty
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);

        let seven_days: u64 = 7 * 24 * 60 * 60;
        let staleness_penalty = if now.saturating_sub(health.last_checked) > seven_days {
            0.5
        } else {
            1.0
        };

        error_penalty * staleness_penalty
    }

    /// Check if we can open a new permanent connection (max 10)
    pub fn can_open_permanent_connection(&self) -> bool {
        let count = self.permanent_connection_count.load(Ordering::SeqCst);
        count < 10
    }

    /// Increment permanent connection count, returns true if successful
    pub fn increment_permanent_connection(&self) -> bool {
        let current = self.permanent_connection_count.load(Ordering::SeqCst);
        if current >= 10 {
            return false;
        }
        self.permanent_connection_count
            .fetch_add(1, Ordering::SeqCst);
        true
    }

    /// Decrement permanent connection count
    pub fn decrement_permanent_connection(&self) {
        let current = self.permanent_connection_count.load(Ordering::SeqCst);
        if current > 0 {
            self.permanent_connection_count
                .fetch_sub(1, Ordering::SeqCst);
        }
    }

    /// Get current permanent connection count
    pub fn get_permanent_connection_count(&self) -> usize {
        self.permanent_connection_count.load(Ordering::SeqCst)
    }

    // ============================================
    // Event De-duplication (for notification loop)
    // ============================================

    /// Check if an event has already been seen
    pub fn is_seen_event(&self, event_id: &str) -> bool {
        let Ok(conn) = self.conn.lock() else {
            return false;
        };
        conn.query_row(
            "SELECT 1 FROM seen_events WHERE event_id = ?1",
            [event_id],
            |_| Ok(()),
        )
        .is_ok()
    }

    /// Mark an event as seen
    pub fn mark_event_seen(&self, event_id: &str) -> Result<(), RelayCacheError> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);

        let conn = self.conn.lock().map_err(|_| RelayCacheError::Lock)?;
        conn.execute(
            "INSERT OR IGNORE INTO seen_events (event_id, seen_at) VALUES (?1, ?2)",
            rusqlite::params![event_id, now],
        )?;
        Ok(())
    }

    /// Save relay pool for a profile.
    ///
    /// Persists the list of relays to SQLite for the given profile.
    /// Existing relays for this profile are replaced.
    ///
    /// # Arguments
    /// * `profile_id` - The profile identifier
    /// * `relays` - List of relay URLs to save
    ///
    /// # Errors
    /// Returns `RelayCacheError` if database operations fail.
    pub fn save_relay_pool(
        &self,
        profile_id: &str,
        relays: &[String],
    ) -> Result<(), RelayCacheError> {
        let conn = self.conn.lock().map_err(|_| RelayCacheError::Lock)?;

        // Create table if not exists
        conn.execute(
            "CREATE TABLE IF NOT EXISTS profile_relay_pools (
                profile_id TEXT NOT NULL,
                relay_url TEXT NOT NULL,
                source TEXT NOT NULL,
                added_at INTEGER NOT NULL,
                PRIMARY KEY (profile_id, relay_url)
            )",
            [],
        )?;

        // Clear existing relays for this profile
        conn.execute(
            "DELETE FROM profile_relay_pools WHERE profile_id = ?1",
            [profile_id],
        )?;

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        for url in relays {
            conn.execute(
                "INSERT INTO profile_relay_pools (profile_id, relay_url, source, added_at)
                 VALUES (?1, ?2, 'discovered', ?3)",
                [profile_id, url, &now.to_string()],
            )?;
        }

        info!("Saved {} relays for profile {}", relays.len(), profile_id);
        Ok(())
    }

    /// Load relay pool for a profile.
    ///
    /// Retrieves the list of persisted relays for the given profile.
    ///
    /// # Arguments
    /// * `profile_id` - The profile identifier
    ///
    /// # Returns
    /// Returns `Vec<String>` containing all relay URLs for the profile.
    /// Returns empty vector if no relays are persisted.
    ///
    /// # Errors
    /// Returns `RelayCacheError` if database query fails.
    pub fn load_relay_pool(&self, profile_id: &str) -> Result<Vec<String>, RelayCacheError> {
        let conn = self.conn.lock().map_err(|_| RelayCacheError::Lock)?;

        let mut stmt =
            conn.prepare("SELECT relay_url FROM profile_relay_pools WHERE profile_id = ?1")?;

        let relay_iter = stmt.query_map([profile_id], |row| row.get::<_, String>(0))?;

        let mut relays = Vec::new();
        for relay in relay_iter {
            relays.push(relay?);
        }

        info!("Loaded {} relays for profile {}", relays.len(), profile_id);
        Ok(relays)
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
