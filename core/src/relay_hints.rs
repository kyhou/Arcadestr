// RelayHints module - Extract and cache relay URLs from p-tags and e-tags
// Used as fallback when NIP-65 relay lists are unavailable

use nostr_sdk::Event;
use rusqlite::Connection;
use std::collections::HashMap;
use std::num::NonZeroUsize;
use std::path::Path;
use std::sync::{Arc, Mutex};
use thiserror::Error;
use tracing::debug;

#[derive(Debug, Error)]
pub enum RelayHintError {
    #[error("SQLite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("Lock error")]
    Lock,
}

/// Maximum hints stored per pubkey
const MAX_HINTS_PER_PUBKEY: usize = 5;

/// Maximum total persisted hints
const MAX_PERSISTED: usize = 2000;

/// LRU in-memory cache size
const CACHE_SIZE: usize = 5000;

/// Stores relay hints extracted from p-tags and e-tags in events.
/// Used as fallback when NIP-65 relay lists are unavailable.
pub struct RelayHints {
    conn: Mutex<Connection>,
    /// In-memory LRU cache: pubkey -> set of relay URLs
    cache: Arc<Mutex<lru::LruCache<String, Vec<String>>>>,
    /// Track dirty entries needing persistence
    dirty: Arc<Mutex<bool>>,
}

impl RelayHints {
    /// Create/open relay hint store at path
    pub fn new<P: AsRef<Path>>(path: P) -> Result<Self, RelayHintError> {
        let conn = Connection::open(path)?;

        // Create table for hints
        conn.execute(
            "CREATE TABLE IF NOT EXISTS relay_hints (
                pubkey TEXT NOT NULL,
                relay_url TEXT NOT NULL,
                last_seen INTEGER NOT NULL,
                PRIMARY KEY (pubkey, relay_url)
            )",
            [],
        )?;

        // Index for fast pubkey lookups
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_hint_pubkey ON relay_hints (pubkey)",
            [],
        )?;

        let cache = lru::LruCache::new(NonZeroUsize::new(CACHE_SIZE).unwrap());

        let hints = RelayHints {
            conn: Mutex::new(conn),
            cache: Arc::new(Mutex::new(cache)),
            dirty: Arc::new(Mutex::new(false)),
        };

        // Load existing hints into cache
        hints.load_from_db()?;

        Ok(hints)
    }

    /// Add a relay hint for a pubkey.
    /// Silently ignores invalid/blank URLs. Caps at MAX_HINTS_PER_PUBKEY.
    pub fn add_hint(&self, pubkey: &str, relay_url: &str) -> Result<(), RelayHintError> {
        let normalized = Self::normalize_relay_url(relay_url);
        if normalized.is_none() {
            return Ok(());
        }
        let url = normalized.unwrap();

        let mut cache = self.cache.lock().map_err(|_| RelayHintError::Lock)?;

        let hints = cache.get(pubkey).cloned().unwrap_or_default();

        // Check capacity
        if hints.len() >= MAX_HINTS_PER_PUBKEY && !hints.contains(&url) {
            return Ok(()); // At capacity and hint is new, skip
        }

        // Add hint
        let mut new_hints = hints;
        if !new_hints.contains(&url) {
            new_hints.push(url);
            cache.put(pubkey.to_string(), new_hints);

            let mut dirty = self.dirty.lock().map_err(|_| RelayHintError::Lock)?;
            *dirty = true;
        }

        Ok(())
    }

    /// Record that an event was delivered from a specific relay (author provenance)
    pub fn add_author_relay(&self, pubkey: &str, relay_url: &str) -> Result<(), RelayHintError> {
        self.add_hint(pubkey, relay_url)
    }

    /// Extract relay hints from p-tags and e-tags in an event.
    /// Nostr tags with format ["p", "<pubkey>", "<relay_url>"] or ["e", "<event_id>", "<relay_url>"]
    pub fn extract_hints_from_event(&self, event: &Event) -> Result<(), RelayHintError> {
        for tag in event.tags.iter() {
            // Convert tag to a vector of strings
            let tag_vec: Vec<String> = tag
                .clone()
                .to_vec()
                .into_iter()
                .map(|s| s.to_string())
                .collect();

            if tag_vec.len() >= 3 && (tag_vec[0] == "p" || tag_vec[0] == "e") {
                let pubkey_or_id = &tag_vec[1];
                let relay_url = &tag_vec[2];

                // Only add hints for p-tags (pubkeys)
                if tag_vec[0] == "p" {
                    self.add_hint(pubkey_or_id, relay_url)?;
                }
            }
        }
        Ok(())
    }

    /// Get accumulated hints for a pubkey
    pub fn get_hints(&self, pubkey: &str) -> Result<Vec<String>, RelayHintError> {
        let mut cache = self.cache.lock().map_err(|_| RelayHintError::Lock)?;
        Ok(cache.get(pubkey).cloned().unwrap_or_default())
    }

    /// Persist dirty hints to database.
    /// Call periodically (e.g., every 60 seconds) from background task.
    pub fn flush(&self) -> Result<(), RelayHintError> {
        let is_dirty = *self.dirty.lock().map_err(|_| RelayHintError::Lock)?;
        if !is_dirty {
            return Ok(());
        }

        let mut dirty = self.dirty.lock().map_err(|_| RelayHintError::Lock)?;
        *dirty = false;

        let cache = self.cache.lock().map_err(|_| RelayHintError::Lock)?;
        let snapshot: Vec<(String, Vec<String>)> =
            cache.iter().map(|(k, v)| (k.clone(), v.clone())).collect();

        // Cap to MAX_PERSISTED entries to avoid unbounded growth
        let entries: Vec<(String, Vec<String>)> = if snapshot.len() > MAX_PERSISTED {
            snapshot.into_iter().take(MAX_PERSISTED).collect()
        } else {
            snapshot
        };

        drop(cache); // Release lock before DB write

        let conn = self.conn.lock().map_err(|_| RelayHintError::Lock)?;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);

        let tx = conn.unchecked_transaction()?;

        // Clear and repopulate (simple approach for now)
        tx.execute("DELETE FROM relay_hints", [])?;

        {
            let mut stmt = tx.prepare(
                "INSERT INTO relay_hints (pubkey, relay_url, last_seen) VALUES (?, ?, ?)",
            )?;

            for (pubkey, urls) in &entries {
                for url in urls {
                    stmt.execute(rusqlite::params![pubkey, url, now])?;
                }
            }
        }

        tx.commit()?;

        debug!("Flushed {} hint entries to database", entries.len());
        Ok(())
    }

    /// Clear all hints (e.g., on logout)
    pub fn clear(&self) -> Result<(), RelayHintError> {
        let mut cache = self.cache.lock().map_err(|_| RelayHintError::Lock)?;
        cache.clear();

        let mut dirty = self.dirty.lock().map_err(|_| RelayHintError::Lock)?;
        *dirty = false;

        let conn = self.conn.lock().map_err(|_| RelayHintError::Lock)?;
        conn.execute("DELETE FROM relay_hints", [])?;

        Ok(())
    }

    /// Load hints from database into memory cache
    fn load_from_db(&self) -> Result<(), RelayHintError> {
        let conn = self.conn.lock().map_err(|_| RelayHintError::Lock)?;

        let mut stmt =
            conn.prepare("SELECT pubkey, relay_url FROM relay_hints ORDER BY last_seen DESC")?;

        let rows = stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;

        let mut hints_map: HashMap<String, Vec<String>> = HashMap::new();

        for row in rows {
            let (pubkey, url) = row?;
            let entry = hints_map.entry(pubkey).or_default();
            if entry.len() < MAX_HINTS_PER_PUBKEY {
                entry.push(url);
            }
        }

        let mut cache = self.cache.lock().map_err(|_| RelayHintError::Lock)?;
        for (pubkey, urls) in hints_map {
            cache.put(pubkey, urls);
        }

        debug!("Loaded {} hint entries from database", cache.len());
        Ok(())
    }

    /// Normalize relay URL (trim whitespace and trailing slashes)
    fn normalize_relay_url(url: &str) -> Option<String> {
        let trimmed = url.trim().trim_end_matches('/');
        if trimmed.is_empty() {
            return None;
        }

        // Basic validation: must start with ws:// or wss://
        if !trimmed.starts_with("ws://") && !trimmed.starts_with("wss://") {
            return None;
        }

        Some(trimmed.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr_sdk::{EventBuilder, Keys, Tag, TagKind};
    use std::fs;

    fn temp_db_path() -> std::path::PathBuf {
        let temp_dir = std::env::temp_dir();
        let unique_name = format!("test_hints_{}.db", std::process::id());
        temp_dir.join(unique_name)
    }

    fn cleanup(path: &std::path::Path) {
        let _ = fs::remove_file(path);
    }

    #[test]
    fn test_add_and_get_hints() {
        let path = temp_db_path();
        cleanup(&path);

        let store = RelayHints::new(&path).unwrap();

        store.add_hint("pubkey1", "wss://relay1.com").unwrap();
        store.add_hint("pubkey1", "wss://relay2.com").unwrap();
        store.add_hint("pubkey2", "wss://relay3.com").unwrap();

        let hints1 = store.get_hints("pubkey1").unwrap();
        assert_eq!(hints1.len(), 2);
        assert!(hints1.contains(&"wss://relay1.com".to_string()));
        assert!(hints1.contains(&"wss://relay2.com".to_string()));

        let hints2 = store.get_hints("pubkey2").unwrap();
        assert_eq!(hints2.len(), 1);

        cleanup(&path);
    }

    #[test]
    fn test_capacity_limit() {
        let path = temp_db_path();
        cleanup(&path);

        let store = RelayHints::new(&path).unwrap();

        // Add more than MAX_HINTS_PER_PUBKEY
        for i in 0..10 {
            store
                .add_hint("pubkey1", &format!("wss://relay{}.com", i))
                .unwrap();
        }

        let hints = store.get_hints("pubkey1").unwrap();
        assert_eq!(hints.len(), MAX_HINTS_PER_PUBKEY);

        cleanup(&path);
    }

    #[test]
    fn test_extract_from_event() {
        let path = temp_db_path();
        cleanup(&path);

        let store = RelayHints::new(&path).unwrap();

        // Build event with p-tags containing relay hints
        let keys = Keys::generate();
        let event = EventBuilder::text_note("Test")
            .tag(Tag::custom(
                TagKind::p(),
                vec!["target_pubkey", "wss://relay.hint.com"],
            ))
            .sign_with_keys(&keys)
            .unwrap();

        store.extract_hints_from_event(&event).unwrap();

        let hints = store.get_hints("target_pubkey").unwrap();
        assert_eq!(hints.len(), 1);
        assert_eq!(hints[0], "wss://relay.hint.com");

        cleanup(&path);
    }

    #[test]
    fn test_persistence() {
        let path = temp_db_path();
        cleanup(&path);

        {
            let store = RelayHints::new(&path).unwrap();
            store.add_hint("pubkey1", "wss://relay1.com").unwrap();
            store.add_hint("pubkey1", "wss://relay2.com").unwrap();
            store.flush().unwrap();
        }

        // Reopen and verify
        {
            let store = RelayHints::new(&path).unwrap();
            let hints = store.get_hints("pubkey1").unwrap();
            assert_eq!(hints.len(), 2);
        }

        cleanup(&path);
    }
}
