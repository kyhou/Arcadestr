# Extended Network Discovery & Relay Hints Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add 2nd-degree extended network discovery and p-tag relay hints to Arcadestr's relay system, enabling discovery of new users through friends-of-friends and accumulating relay URLs from events as fallback sources.

**Architecture:** Two complementary systems: (1) **ExtendedNetworkRepository** - Periodically fetches follow lists from 1st-degree follows, counts 2nd-degree appearances with a threshold, fetches their relay lists, and computes optimal relay coverage using greedy set-cover. (2) **RelayHintStore** - Lightweight cache that extracts relay URLs from p-tags and e-tags in events, storing up to 5 hints per pubkey as fallback when NIP-65 relay lists are unavailable. Both systems integrate with the existing relay selection algorithm and follow the established SQLite + in-memory caching patterns.

**Tech Stack:** Rust, SQLite (rusqlite), nostr-sdk, Arc/Mutex for thread safety, serde for serialization

**Files to Create:**
- `core/src/extended_network.rs` - Extended network discovery and 2nd-degree follow tracking
- `core/src/relay_hint_store.rs` - p-tag/e-tag relay hint extraction and storage
- `core/src/social_graph.rs` - SQLite storage for followed-by relationships

**Files to Modify:**
- `core/src/relay_cache.rs` - Add integration with relay hints as fallback tier
- `core/src/nostr.rs` - Add extended network discovery calls, integrate hints into relay selection
- `core/src/lib.rs` - Export new modules
- `desktop/src/main.rs` - Initialize extended network on auth, periodic refresh

---

## Task 1: Social Graph Database

**Files:**
- Create: `core/src/social_graph.rs`
- Test: `core/src/social_graph.rs` (inline tests at bottom)

**Purpose:** Store "followed-by" relationships from 2nd-degree follows for extended network discovery. Maintains which 1st-degree follows follow which 2nd-degree pubkeys.

- [ ] **Step 1: Write the social graph SQLite schema and API**

```rust
use rusqlite::{Connection, Result as SqliteResult};
use std::collections::HashSet;
use std::path::Path;
use std::sync::{Arc, Mutex};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum SocialGraphError {
    #[error("SQLite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("Lock error")]
    Lock,
}

/// Stores "followed-by" relationships for extended network discovery.
/// Tracks which 1st-degree follows follow which 2nd-degree pubkeys.
pub struct SocialGraphDb {
    conn: Mutex<Connection>,
}

impl SocialGraphDb {
    /// Create/open social graph database at path
    pub fn new<P: AsRef<Path>>(path: P) -> Result<Self, SocialGraphError> {
        let conn = Connection::open(path)?;
        
        // Create table for followed-by relationships
        conn.execute(
            "CREATE TABLE IF NOT EXISTS followed_by (
                target_pubkey TEXT NOT NULL,
                follower_pubkey TEXT NOT NULL,
                PRIMARY KEY (target_pubkey, follower_pubkey)
            )",
            [],
        )?;
        
        // Index for fast lookups
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_target ON followed_by (target_pubkey)",
            [],
        )?;
        
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }
    
    /// Insert a batch of followed-by relationships
    /// Each pair is (target_pubkey, follower_pubkey) meaning follower follows target
    pub fn insert_batch(&self, pairs: &[(String, String)]) -> Result<(), SocialGraphError> {
        if pairs.is_empty() {
            return Ok(());
        }
        
        let conn = self.conn.lock().map_err(|_| SocialGraphError::Lock)?;
        
        let tx = conn.unchecked_transaction()?;
        
        {
            let mut stmt = tx.prepare(
                "INSERT OR REPLACE INTO followed_by (target_pubkey, follower_pubkey) VALUES (?, ?)"
            )?;
            
            for (target, follower) in pairs {
                stmt.execute(rusqlite::params![target, follower])?;
            }
        }
        
        tx.commit()?;
        Ok(())
    }
    
    /// Get all followers (1st-degree) who follow the given target pubkey
    pub fn get_followers(&self, target_pubkey: &str) -> Result<Vec<String>, SocialGraphError> {
        let conn = self.conn.lock().map_err(|_| SocialGraphError::Lock)?;
        
        let mut stmt = conn.prepare(
            "SELECT follower_pubkey FROM followed_by WHERE target_pubkey = ?"
        )?;
        
        let followers: Result<Vec<String>, _> = stmt
            .query_map([target_pubkey], |row| row.get(0))?
            .collect();
        
        Ok(followers?)
    }
    
    /// Count how many followers each target pubkey has
    /// Returns map of target_pubkey -> follower_count
    pub fn count_followers(&self, target_pubkeys: &[String]) -> Result<HashMap<String, i32>, SocialGraphError> {
        let conn = self.conn.lock().map_err(|_| SocialGraphError::Lock)?;
        
        let placeholders: Vec<&str> = target_pubkeys.iter().map(|_| "?").collect();
        let query = format!(
            "SELECT target_pubkey, COUNT(follower_pubkey) as count 
             FROM followed_by 
             WHERE target_pubkey IN ({}) 
             GROUP BY target_pubkey",
            placeholders.join(",")
        );
        
        let mut stmt = conn.prepare(&query)?;
        let params: Vec<&dyn rusqlite::ToSql> = target_pubkeys
            .iter()
            .map(|p| p as &dyn rusqlite::ToSql)
            .collect();
        
        let counts: Result<HashMap<String, i32>, _> = stmt
            .query_map(rusqlite::params_from_iter(params.iter()), |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, i32>(1)?))
            })?
            .collect();
        
        Ok(counts?)
    }
    
    /// Clear all data (e.g., on logout or fresh discovery)
    pub fn clear_all(&self) -> Result<(), SocialGraphError> {
        let conn = self.conn.lock().map_err(|_| SocialGraphError::Lock)?;
        conn.execute("DELETE FROM followed_by", [])?;
        Ok(())
    }
    
    /// Get total relationship count
    pub fn get_relationship_count(&self) -> Result<i64, SocialGraphError> {
        let conn = self.conn.lock().map_err(|_| SocialGraphError::Lock)?;
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM followed_by",
            [],
            |row| row.get(0)
        )?;
        Ok(count)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;
    
    #[test]
    fn test_insert_and_query() {
        let temp = TempDir::new().unwrap();
        let db = SocialGraphDb::new(temp.path().join("test.db")).unwrap();
        
        let pairs = vec![
            ("pubkey_a".to_string(), "pubkey_1".to_string()), // 1 follows a
            ("pubkey_a".to_string(), "pubkey_2".to_string()), // 2 follows a
            ("pubkey_b".to_string(), "pubkey_1".to_string()), // 1 follows b
        ];
        
        db.insert_batch(&pairs).unwrap();
        
        let followers_a = db.get_followers("pubkey_a").unwrap();
        assert_eq!(followers_a.len(), 2);
        assert!(followers_a.contains(&"pubkey_1".to_string()));
        assert!(followers_a.contains(&"pubkey_2".to_string()));
        
        let followers_b = db.get_followers("pubkey_b").unwrap();
        assert_eq!(followers_b.len(), 1);
        assert!(followers_b.contains(&"pubkey_1".to_string()));
    }
    
    #[test]
    fn test_count_followers() {
        let temp = TempDir::new().unwrap();
        let db = SocialGraphDb::new(temp.path().join("test.db")).unwrap();
        
        let pairs: Vec<(String, String)> = (0..100)
            .map(|i| ("target".to_string(), format!("follower_{}", i)))
            .collect();
        
        db.insert_batch(&pairs).unwrap();
        
        let counts = db.count_followers(&["target".to_string()]).unwrap();
        assert_eq!(counts.get("target"), Some(&100));
    }
    
    #[test]
    fn test_clear_all() {
        let temp = TempDir::new().unwrap();
        let db = SocialGraphDb::new(temp.path().join("test.db")).unwrap();
        
        let pairs = vec![
            ("a".to_string(), "1".to_string()),
        ];
        db.insert_batch(&pairs).unwrap();
        
        assert_eq!(db.get_relationship_count().unwrap(), 1);
        db.clear_all().unwrap();
        assert_eq!(db.get_relationship_count().unwrap(), 0);
    }
}
```

- [ ] **Step 2: Add to core/src/lib.rs exports**

```rust
// Add these to the existing exports in core/src/lib.rs
pub mod extended_network;
pub mod relay_hint_store;
pub mod social_graph;
```

- [ ] **Step 3: Verify compilation**

Run: `cargo check -p arcadestr-core`
Expected: No errors

- [ ] **Step 4: Commit**

```bash
git add core/src/social_graph.rs core/src/lib.rs
git commit -m "feat: add social graph database for 2nd-degree follow tracking"
```

---

## Task 2: Relay Hint Store

**Files:**
- Create: `core/src/relay_hint_store.rs`
- Modify: `core/src/relay_cache.rs` - Add hint integration to discovery tiers
- Test: `core/src/relay_hint_store.rs` (inline tests at bottom)

**Purpose:** Extract relay URLs from p-tags and e-tags in events, storing them as fallback when NIP-65 relay lists are unavailable. Cap at 5 hints per pubkey, 2000 total persisted.

- [ ] **Step 1: Write the relay hint store module**

```rust
use nostr_sdk::Event;
use rusqlite::{Connection, Result as SqliteResult};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::num::NonZeroUsize;
use std::path::Path;
use std::sync::{Arc, Mutex};
use thiserror::Error;
use tracing::{debug, warn};

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
pub struct RelayHintStore {
    conn: Mutex<Connection>,
    /// In-memory LRU cache: pubkey -> set of relay URLs
    cache: Arc<Mutex<lru::LruCache<String, Vec<String>>>>,
    /// Track dirty entries needing persistence
    dirty: Arc<Mutex<bool>>,
}

impl RelayHintStore {
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
        
        let store = Self {
            conn: Mutex::new(conn),
            cache: Arc::new(Mutex::new(cache)),
            dirty: Arc::new(Mutex::new(false)),
        };
        
        // Load existing hints into cache
        store.load_from_db()?;
        
        Ok(store)
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
            let tag_vec: Vec<String> = tag.as_vec().iter().map(|s| s.to_string()).collect();
            
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
        let cache = self.cache.lock().map_err(|_| RelayHintError::Lock)?;
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
        let snapshot: Vec<(String, Vec<String>)> = cache
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        
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
                "INSERT INTO relay_hints (pubkey, relay_url, last_seen) VALUES (?, ?, ?)"
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
        
        let mut stmt = conn.prepare(
            "SELECT pubkey, relay_url FROM relay_hints ORDER BY last_seen DESC"
        )?;
        
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
    use nostr_sdk::{Keys, EventBuilder};
    use tempfile::TempDir;
    
    #[test]
    fn test_add_and_get_hints() {
        let temp = TempDir::new().unwrap();
        let store = RelayHintStore::new(temp.path().join("hints.db")).unwrap();
        
        store.add_hint("pubkey1", "wss://relay1.com").unwrap();
        store.add_hint("pubkey1", "wss://relay2.com").unwrap();
        store.add_hint("pubkey2", "wss://relay3.com").unwrap();
        
        let hints1 = store.get_hints("pubkey1").unwrap();
        assert_eq!(hints1.len(), 2);
        assert!(hints1.contains(&"wss://relay1.com".to_string()));
        assert!(hints1.contains(&"wss://relay2.com".to_string()));
        
        let hints2 = store.get_hints("pubkey2").unwrap();
        assert_eq!(hints2.len(), 1);
    }
    
    #[test]
    fn test_capacity_limit() {
        let temp = TempDir::new().unwrap();
        let store = RelayHintStore::new(temp.path().join("hints.db")).unwrap();
        
        // Add more than MAX_HINTS_PER_PUBKEY
        for i in 0..10 {
            store.add_hint("pubkey1", &format!("wss://relay{}.com", i)).unwrap();
        }
        
        let hints = store.get_hints("pubkey1").unwrap();
        assert_eq!(hints.len(), MAX_HINTS_PER_PUBKEY);
    }
    
    #[test]
    fn test_extract_from_event() {
        let temp = TempDir::new().unwrap();
        let store = RelayHintStore::new(temp.path().join("hints.db")).unwrap();
        
        // Build event with p-tags containing relay hints
        let keys = Keys::generate();
        let event = EventBuilder::text_note("Test")
            .tag(Tag::custom(
                TagKind::p(),
                vec!["target_pubkey", "wss://relay.hint.com"]
            ))
            .sign_with_keys(&keys)
            .unwrap();
        
        store.extract_hints_from_event(&event).unwrap();
        
        let hints = store.get_hints("target_pubkey").unwrap();
        assert_eq!(hints.len(), 1);
        assert_eq!(hints[0], "wss://relay.hint.com");
    }
    
    #[test]
    fn test_persistence() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("hints.db");
        
        {
            let store = RelayHintStore::new(&path).unwrap();
            store.add_hint("pubkey1", "wss://relay1.com").unwrap();
            store.add_hint("pubkey1", "wss://relay2.com").unwrap();
            store.flush().unwrap();
        }
        
        // Reopen and verify
        {
            let store = RelayHintStore::new(&path).unwrap();
            let hints = store.get_hints("pubkey1").unwrap();
            assert_eq!(hints.len(), 2);
        }
    }
}
```

- [ ] **Step 2: Add lru crate to Cargo.toml**

In `core/Cargo.toml`, add under `[dependencies]`:

```toml
lru = "0.12"
```

- [ ] **Step 3: Modify relay_cache.rs to add hint discovery source**

Add to the `RelayDiscoverySource` enum in `core/src/relay_cache.rs`:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RelayDiscoverySource {
    RelayList,        // NIP-65 Kind 10002
    SeenOn,          // Events seen on this relay
    RelayHints,      // From p-tag/e-tag relay hints
    GlobalFallback,  // Default hardcoded relays
}
```

- [ ] **Step 4: Verify compilation and tests**

Run: `cargo check -p arcadestr-core`
Expected: No errors

Run: `cargo test -p arcadestr-core relay_hint_store`
Expected: All 4 tests pass

- [ ] **Step 5: Commit**

```bash
git add core/src/relay_hint_store.rs core/Cargo.toml core/src/relay_cache.rs core/src/lib.rs
git commit -m "feat: add relay hint store for p-tag relay extraction"
```

---

## Task 3: Extended Network Repository

**Files:**
- Create: `core/src/extended_network.rs`
- Modify: `core/src/nostr.rs` - Add extended network integration
- Test: `core/src/extended_network.rs` (inline tests at bottom)

**Purpose:** Discover 2nd-degree follows (friends of friends), filter by threshold (10+ followers), fetch their relay lists, and compute optimal relay coverage using greedy set-cover algorithm.

- [ ] **Step 1: Write the extended network module**

```rust
use crate::nostr::{NostrClient, KIND_FOLLOW_LIST, KIND_RELAY_LIST};
use crate::relay_cache::{RelayCache, CachedRelayList};
use crate::relay_hint_store::RelayHintStore;
use crate::social_graph::SocialGraphDb;
use nostr_sdk::{Event, Filter, Timestamp, Kind};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use thiserror::Error;
use tokio::time::{timeout, sleep};
use tracing::{debug, info, warn, error};

#[derive(Debug, Error)]
pub enum ExtendedNetworkError {
    #[error("Nostr client error: {0}")]
    Nostr(String),
    #[error("Timeout: {0}")]
    Timeout(String),
    #[error("Database error: {0}")]
    Database(String),
}

/// Threshold for qualifying as extended network member (must be followed by >= 10 1st-degree)
const QUALIFYING_THRESHOLD: usize = 10;

/// Maximum relays to add for extended network
const MAX_EXTENDED_RELAYS: usize = 100;

/// Maximum authors per relay cap
const MAX_AUTHORS_PER_RELAY: usize = 300;

/// Cache TTL: 24 hours
const CACHE_TTL_HOURS: u64 = 24;

/// Discovery timeout: 30 seconds
const DISCOVERY_TIMEOUT_SECS: u64 = 30;

/// Follow list fetch timeout: 5 seconds
const FOLLOW_LIST_TIMEOUT_SECS: u64 = 5;

/// Relay list fetch timeout: 8 seconds
const RELAY_LIST_TIMEOUT_SECS: u64 = 8;

/// Extended network discovery state
#[derive(Debug, Clone, PartialEq)]
pub enum DiscoveryState {
    Idle,
    FetchingFollowLists { fetched: usize, total: usize },
    BuildingGraph { processed: usize, total: usize },
    ComputingNetwork { unique_users: usize },
    Filtering { qualified: usize },
    FetchingRelayLists { fetched: usize, total: usize },
    Complete { stats: NetworkStats },
    Failed { reason: String },
}

/// Statistics about the extended network
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkStats {
    pub first_degree_count: usize,
    pub total_second_degree: usize,
    pub qualified_count: usize,
    pub relays_covered: usize,
    pub computed_at: u64,
}

/// Cached extended network data
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtendedNetworkCache {
    pub qualified_pubkeys: HashSet<String>,
    pub first_degree_pubkeys: HashSet<String>,
    pub relay_urls: Vec<String>,
    pub relay_hints: HashMap<String, Vec<String>>, // pubkey -> relay hints
    pub stats: NetworkStats,
}

/// Extended network discovery manager
pub struct ExtendedNetworkRepository {
    my_pubkey: Option<String>,
    social_graph: Arc<SocialGraphDb>,
    discovery_state: Arc<Mutex<DiscoveryState>>,
    cached_network: Arc<Mutex<Option<ExtendedNetworkCache>>>,
    discovery_in_progress: Arc<Mutex<bool>>,
}

impl ExtendedNetworkRepository {
    pub fn new(social_graph: Arc<SocialGraphDb>) -> Self {
        Self {
            my_pubkey: None,
            social_graph,
            discovery_state: Arc::new(Mutex::new(DiscoveryState::Idle)),
            cached_network: Arc::new(Mutex::new(None)),
            discovery_in_progress: Arc::new(Mutex::new(false)),
        }
    }
    
    pub fn set_pubkey(&mut self, pubkey: String) {
        self.my_pubkey = Some(pubkey);
    }
    
    pub fn clear(&self) {
        let _ = self.social_graph.clear_all();
        *self.discovery_state.lock().unwrap() = DiscoveryState::Idle;
        *self.cached_network.lock().unwrap() = None;
        *self.discovery_in_progress.lock().unwrap() = false;
    }
    
    pub fn get_state(&self) -> DiscoveryState {
        self.discovery_state.lock().unwrap().clone()
    }
    
    pub fn get_cached_network(&self) -> Option<ExtendedNetworkCache> {
        self.cached_network.lock().unwrap().clone()
    }
    
    pub fn is_cache_stale(&self) -> bool {
        let cache = self.cached_network.lock().unwrap();
        match cache.as_ref() {
            None => true,
            Some(c) => {
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs())
                    .unwrap_or(0);
                let age_hours = (now - c.stats.computed_at) / 3600;
                age_hours >= CACHE_TTL_HOURS
            }
        }
    }
    
    /// Main discovery workflow
    pub async fn discover_network(
        &self,
        nostr_client: &NostrClient,
        relay_cache: &RelayCache,
        first_degree_follows: Vec<String>,
    ) -> Result<NetworkStats, ExtendedNetworkError> {
        // Prevent concurrent discoveries
        {
            let mut in_progress = self.discovery_in_progress.lock().unwrap();
            if *in_progress {
                return Err(ExtendedNetworkError::Nostr("Discovery already in progress".to_string()));
            }
            *in_progress = true;
        }
        
        // Clear previous data
        let _ = self.social_graph.clear_all();
        
        let result = self._discover_network(nostr_client, relay_cache, first_degree_follows).await;
        
        *self.discovery_in_progress.lock().unwrap() = false;
        
        result
    }
    
    async fn _discover_network(
        &self,
        nostr_client: &NostrClient,
        relay_cache: &RelayCache,
        first_degree_follows: Vec<String>,
    ) -> Result<NetworkStats, ExtendedNetworkError> {
        let my_pubkey = self.my_pubkey.clone()
            .ok_or_else(|| ExtendedNetworkError::Nostr("No pubkey set".to_string()))?;
        
        if first_degree_follows.is_empty() {
            return Err(ExtendedNetworkError::Nostr("Follow list is empty".to_string()));
        }
        
        let first_degree_set: HashSet<String> = first_degree_follows.iter().cloned().collect();
        let total_first = first_degree_follows.len();
        
        info!("Starting extended network discovery with {} first-degree follows", total_first);
        
        // Step 1: Fetch kind 3 follow lists from all 1st-degree follows
        *self.discovery_state.lock().unwrap() = DiscoveryState::FetchingFollowLists {
            fetched: 0,
            total: total_first,
        };
        
        let follow_lists = self.fetch_follow_lists(
            nostr_client,
            &first_degree_follows,
        ).await?;
        
        info!("Fetched {} follow lists", follow_lists.len());
        
        // Step 2: Parse follow lists, count 2nd-degree appearances
        *self.discovery_state.lock().unwrap() = DiscoveryState::BuildingGraph {
            processed: 0,
            total: follow_lists.len(),
        };
        
        let (second_degree_counts, relay_hints) = self.build_social_graph(
            &follow_lists,
            &my_pubkey,
            &first_degree_set,
        ).await?;
        
        info!("Built social graph: {} unique 2nd-degree follows", second_degree_counts.len());
        
        // Step 3: Filter to qualified pubkeys (threshold >= 10) and exclude self
        *self.discovery_state.lock().unwrap() = DiscoveryState::ComputingNetwork {
            unique_users: second_degree_counts.len(),
        };
        
        let qualified: HashSet<String> = second_degree_counts
            .iter()
            .filter(|(pubkey, count)| {
                *count >= QUALIFYING_THRESHOLD && **pubkey != my_pubkey && !first_degree_set.contains(*pubkey)
            })
            .map(|(pubkey, _)| pubkey.clone())
            .collect();
        
        info!("Qualified {} pubkeys (threshold >= {})", qualified.len(), QUALIFYING_THRESHOLD);
        
        if qualified.is_empty() {
            let stats = NetworkStats {
                first_degree_count: total_first,
                total_second_degree: second_degree_counts.len(),
                qualified_count: 0,
                relays_covered: 0,
                computed_at: std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs())
                    .unwrap_or(0),
            };
            
            *self.discovery_state.lock().unwrap() = DiscoveryState::Complete { stats: stats.clone() };
            return Ok(stats);
        }
        
        // Step 4: Fetch relay lists for qualified pubkeys
        *self.discovery_state.lock().unwrap() = DiscoveryState::FetchingRelayLists {
            fetched: 0,
            total: qualified.len(),
        };
        
        let qualified_list: Vec<String> = qualified.iter().cloned().collect();
        self.fetch_relay_lists_for_pubkeys(nostr_client, relay_cache, &qualified_list).await?;
        
        // Step 5: Compute optimal relay set using greedy set-cover
        let qualified_hints: HashMap<String, Vec<String>> = relay_hints
            .into_iter()
            .filter(|(k, _)| qualified.contains(k))
            .collect();
        
        let relay_urls = self.compute_relay_set_cover(&qualified, &qualified_hints, relay_cache);
        
        info!("Computed {} relays for extended network coverage", relay_urls.len());
        
        let stats = NetworkStats {
            first_degree_count: total_first,
            total_second_degree: second_degree_counts.len(),
            qualified_count: qualified.len(),
            relays_covered: relay_urls.len(),
            computed_at: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0),
        };
        
        // Save to cache
        let cache = ExtendedNetworkCache {
            qualified_pubkeys: qualified,
            first_degree_pubkeys: first_degree_set,
            relay_urls: relay_urls.clone(),
            relay_hints: qualified_hints,
            stats: stats.clone(),
        };
        *self.cached_network.lock().unwrap() = Some(cache);
        
        *self.discovery_state.lock().unwrap() = DiscoveryState::Complete { stats: stats.clone() };
        
        Ok(stats)
    }
    
    async fn fetch_follow_lists(
        &self,
        nostr_client: &NostrClient,
        first_degree: &[String],
    ) -> Result<HashMap<String, Event>, ExtendedNetworkError> {
        let mut follow_lists = HashMap::new();
        
        // Create subscription for kind 3 events from first-degree follows
        let filter = Filter::new()
            .kind(Kind::Custom(KIND_FOLLOW_LIST))
            .authors(first_degree.iter().cloned().map(|p| nostr_sdk::PublicKey::from_hex(&p).unwrap()).collect::<Vec<_>>());
        
        // Subscribe and collect events with timeout
        let timeout_duration = Duration::from_secs(FOLLOW_LIST_TIMEOUT_SECS);
        let subscription_id = format!("extnet-follow-lists-{}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0));
        
        // Fetch via NostrClient
        match timeout(timeout_duration, nostr_client.fetch_events(filter)).await {
            Ok(Ok(events)) => {
                for event in events {
                    follow_lists.insert(event.pubkey.to_hex(), event);
                    
                    let mut state = self.discovery_state.lock().unwrap();
                    if let DiscoveryState::FetchingFollowLists { fetched, total } = *state {
                        *state = DiscoveryState::FetchingFollowLists {
                            fetched: fetched + 1,
                            total,
                        };
                    }
                }
            }
            Ok(Err(e)) => {
                warn!("Error fetching follow lists: {}", e);
            }
            Err(_) => {
                warn!("Timeout fetching follow lists after {}s", FOLLOW_LIST_TIMEOUT_SECS);
            }
        }
        
        Ok(follow_lists)
    }
    
    async fn build_social_graph(
        &self,
        follow_lists: &HashMap<String, Event>,
        my_pubkey: &str,
        first_degree_set: &HashSet<String>,
    ) -> Result<(HashMap<String, usize>, HashMap<String, Vec<String>>), ExtendedNetworkError> {
        let mut second_degree_counts: HashMap<String, usize> = HashMap::new();
        let mut relay_hints: HashMap<String, Vec<String>> = HashMap::new();
        let mut batch: Vec<(String, String)> = Vec::new();
        
        let total = follow_lists.len();
        let mut processed = 0;
        
        for (follower_pubkey, event) in follow_lists {
            // Parse follow list entries
            for tag in event.tags.iter() {
                let tag_vec: Vec<String> = tag.as_vec().iter().map(|s| s.to_string()).collect();
                
                if tag_vec.len() >= 2 && tag_vec[0] == "p" {
                    let target_pubkey = &tag_vec[1];
                    
                    // Record followed-by relationship
                    batch.push((target_pubkey.clone(), follower_pubkey.clone()));
                    
                    // Count 2nd-degree (exclude self and 1st-degree)
                    if target_pubkey != my_pubkey && !first_degree_set.contains(target_pubkey) {
                        *second_degree_counts.entry(target_pubkey.clone()).or_insert(0) += 1;
                        
                        // Extract relay hint from p-tag if present
                        if tag_vec.len() >= 3 {
                            let hint = &tag_vec[2];
                            if hint.starts_with("ws://") || hint.starts_with("wss://") {
                                relay_hints.entry(target_pubkey.clone())
                                    .or_default()
                                    .push(hint.clone());
                            }
                        }
                    }
                }
            }
            
            processed += 1;
            if batch.len() >= 5000 {
                self.social_graph.insert_batch(&batch)
                    .map_err(|e| ExtendedNetworkError::Database(e.to_string()))?;
                batch.clear();
            }
            
            // Update state periodically
            if processed % 100 == 0 {
                let mut state = self.discovery_state.lock().unwrap();
                *state = DiscoveryState::BuildingGraph { processed, total };
            }
        }
        
        // Insert remaining batch
        if !batch.is_empty() {
            self.social_graph.insert_batch(&batch)
                .map_err(|e| ExtendedNetworkError::Database(e.to_string()))?;
        }
        
        *self.discovery_state.lock().unwrap() = DiscoveryState::BuildingGraph { processed: total, total };
        
        Ok((second_degree_counts, relay_hints))
    }
    
    async fn fetch_relay_lists_for_pubkeys(
        &self,
        nostr_client: &NostrClient,
        relay_cache: &RelayCache,
        pubkeys: &[String],
    ) -> Result<(), ExtendedNetworkError> {
        // Find pubkeys missing from cache
        let missing: Vec<String> = pubkeys.iter()
            .filter(|p| relay_cache.get_relay_list(p).is_none())
            .cloned()
            .collect();
        
        if missing.is_empty() {
            return Ok(());
        }
        
        // Fetch in chunks of 500
        let chunk_size = 500;
        let chunks: Vec<Vec<String>> = missing.chunks(chunk_size)
            .map(|c| c.to_vec())
            .collect();
        
        let mut total_fetched = 0;
        let total = missing.len();
        
        for (i, chunk) in chunks.iter().enumerate() {
            let filter = Filter::new()
                .kind(Kind::Custom(KIND_RELAY_LIST))
                .authors(chunk.iter().cloned().map(|p| nostr_sdk::PublicKey::from_hex(&p).unwrap()).collect::<Vec<_>>());
            
            let timeout_duration = Duration::from_secs(RELAY_LIST_TIMEOUT_SECS);
            
            match timeout(timeout_duration, nostr_client.fetch_events(filter)).await {
                Ok(Ok(events)) => {
                    for event in events {
                        // Parse and cache relay list
                        if let Ok(relay_list) = crate::nostr::parse_relay_list_from_event(&event) {
                            let _ = relay_cache.cache_relay_list(relay_list);
                            total_fetched += 1;
                        }
                    }
                }
                Ok(Err(e)) => {
                    warn!("Error fetching relay list chunk {}: {}", i, e);
                }
                Err(_) => {
                    warn!("Timeout fetching relay list chunk {}", i);
                }
            }
            
            *self.discovery_state.lock().unwrap() = DiscoveryState::FetchingRelayLists {
                fetched: total_fetched,
                total,
            };
        }
        
        Ok(())
    }
    
    /// Greedy set-cover algorithm: pick relay covering most uncovered qualified pubkeys, repeat
    fn compute_relay_set_cover(
        &self,
        qualified: &HashSet<String>,
        relay_hints: &HashMap<String, Vec<String>>,
        relay_cache: &RelayCache,
    ) -> Vec<String> {
        let mut relay_to_authors: HashMap<String, HashSet<String>> = HashMap::new();
        let mut from_relay_lists = 0usize;
        let mut from_hints = 0usize;
        
        // Build relay -> authors mapping
        for pubkey in qualified {
            if let Some(cached) = relay_cache.get_relay_list(pubkey) {
                // From NIP-65 relay list
                from_relay_lists += 1;
                for relay in &cached.write_relays {
                    relay_to_authors.entry(relay.clone())
                        .or_default()
                        .insert(pubkey.clone());
                }
            } else if let Some(hints) = relay_hints.get(pubkey) {
                // From relay hints
                from_hints += 1;
                for hint in hints {
                    relay_to_authors.entry(hint.clone())
                        .or_default()
                        .insert(pubkey.clone());
                }
            }
        }
        
        debug!(
            "Set-cover input: {} from relay lists, {} from hints",
            from_relay_lists, from_hints
        );
        
        if relay_to_authors.is_empty() {
            return Vec::new();
        }
        
        let mut uncovered = qualified.clone();
        let mut selected: Vec<String> = Vec::new();
        let mut remaining = relay_to_authors.clone();
        
        while !uncovered.is_empty() 
            && selected.len() < MAX_EXTENDED_RELAYS 
            && !remaining.is_empty() {
            
            // Find relay covering most uncovered pubkeys
            let mut best_url: Option<String> = None;
            let mut best_cover_size = 0usize;
            
            for (url, authors) in &remaining {
                let cover_size = authors.iter().filter(|a| uncovered.contains(*a)).count();
                if cover_size > best_cover_size {
                    best_url = Some(url.clone());
                    best_cover_size = cover_size;
                }
            }
            
            if best_url.is_none() || best_cover_size == 0 {
                break;
            }
            
            let url = best_url.unwrap();
            selected.push(url.clone());
            
            // Remove covered pubkeys (with cap per relay)
            let covered: Vec<String> = remaining[&url]
                .iter()
                .filter(|a| uncovered.contains(*a))
                .take(MAX_AUTHORS_PER_RELAY)
                .cloned()
                .collect();
            
            for pubkey in covered {
                uncovered.remove(&pubkey);
            }
            
            remaining.remove(&url);
        }
        
        info!(
            "Set-cover: {} relays cover {}/{} pubkeys",
            selected.len(),
            qualified.len() - uncovered.len(),
            qualified.len()
        );
        
        selected
    }
    
    /// Get relay configurations for extended network (read-only)
    pub fn get_relay_configs(&self) -> Vec<String> {
        self.cached_network.lock()
            .unwrap()
            .as_ref()
            .map(|c| c.relay_urls.clone())
            .unwrap_or_default()
    }
    
    /// Get followers who follow a specific pubkey (from social graph)
    pub fn get_followed_by(&self, pubkey: &str) -> Vec<String> {
        self.social_graph.get_followers(pubkey)
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;
    
    fn create_test_repo() -> (ExtendedNetworkRepository, Arc<SocialGraphDb>) {
        let temp = TempDir::new().unwrap();
        let social_graph = Arc::new(SocialGraphDb::new(temp.path().join("social.db")).unwrap());
        let repo = ExtendedNetworkRepository::new(social_graph.clone());
        (repo, social_graph)
    }
    
    #[test]
    fn test_compute_set_cover_basic() {
        let (repo, _) = create_test_repo();
        repo.set_pubkey("me".to_string());
        
        let mut qualified: HashSet<String> = HashSet::new();
        qualified.insert("a".to_string());
        qualified.insert("b".to_string());
        qualified.insert("c".to_string());
        
        let mut hints: HashMap<String, Vec<String>> = HashMap::new();
        hints.insert("a".to_string(), vec!["wss://relay1.com".to_string(), "wss://relay2.com".to_string()]);
        hints.insert("b".to_string(), vec!["wss://relay1.com".to_string()]);
        hints.insert("c".to_string(), vec!["wss://relay2.com".to_string()]);
        
        // Create mock relay_cache that returns None for all
        // For this test, we rely on hints only
        let temp = TempDir::new().unwrap();
        let relay_cache = RelayCache::new(temp.path().join("relay.db")).unwrap();
        
        let result = repo.compute_relay_set_cover(&qualified, &hints, &relay_cache);
        
        // relay1 covers a and b, relay2 covers a and c
        // Greedy picks relay1 first (covers 2), then relay2 (covers c)
        assert_eq!(result.len(), 2);
        assert!(result.contains(&"wss://relay1.com".to_string()));
        assert!(result.contains(&"wss://relay2.com".to_string()));
    }
    
    #[test]
    fn test_qualifying_threshold() {
        let (repo, social_graph) = create_test_repo();
        repo.set_pubkey("me".to_string());
        
        // Create follow relationships
        // pubkey_x is followed by 15 first-degree follows (qualifies)
        // pubkey_y is followed by 5 first-degree follows (doesn't qualify)
        let mut pairs = Vec::new();
        for i in 0..15 {
            pairs.push(("pubkey_x".to_string(), format!("follower_{}", i)));
        }
        for i in 0..5 {
            pairs.push(("pubkey_y".to_string(), format!("follower_{}", i)));
        }
        
        social_graph.insert_batch(&pairs).unwrap();
        
        let counts = social_graph.count_followers(&["pubkey_x".to_string(), "pubkey_y".to_string()]).unwrap();
        
        assert_eq!(counts.get("pubkey_x"), Some(&15));
        assert_eq!(counts.get("pubkey_y"), Some(&5));
        
        // Only pubkey_x qualifies
        let qualified: Vec<String> = counts.iter()
            .filter(|(_, count)| **count >= QUALIFYING_THRESHOLD as i32)
            .map(|(k, _)| k.clone())
            .collect();
        
        assert_eq!(qualified.len(), 1);
        assert!(qualified.contains(&"pubkey_x".to_string()));
    }
}
```

- [ ] **Step 2: Verify compilation and tests**

Run: `cargo check -p arcadestr-core`
Expected: No errors

Run: `cargo test -p arcadestr-core extended_network`
Expected: Tests pass

- [ ] **Step 3: Commit**

```bash
git add core/src/extended_network.rs core/src/lib.rs
git commit -m "feat: add extended network discovery for 2nd-degree follows"
```

---

## Task 4: Integration with NostrClient and Relay Selection

**Files:**
- Modify: `core/src/nostr.rs` - Add integration methods
- Modify: `core/src/relay_cache.rs` - Add get_relay_list method if missing
- Test: Run existing tests

**Purpose:** Wire extended network and relay hints into the existing relay discovery waterfall and selection algorithm.

- [ ] **Step 1: Add method to fetch relay list with hint fallback in nostr.rs**

Add this method to `NostrClient` impl in `core/src/nostr.rs`:

```rust
/// Get relays for a pubkey with full fallback chain (Tier 1-4)
/// 
/// Tier 1: NIP-65 Kind 10002 (from cache or fetch)
/// Tier 2: Kind 3 content field (legacy)
/// Tier 3: Relay hints (from p-tags)
/// Tier 4: Global fallbacks
pub async fn get_relays_for_pubkey_with_hints(
    &self,
    pubkey: &str,
    relay_cache: &RelayCache,
    hint_store: Option<&RelayHintStore>,
) -> RelayDiscoveryResult {
    // Tier 1: NIP-65 from cache
    if let Some(cached) = relay_cache.get_relay_list(pubkey) {
        return RelayDiscoveryResult {
            write_relays: cached.write_relays,
            read_relays: cached.read_relays,
            source: RelayDiscoverySource::RelayList,
        };
    }
    
    // Try fetching from network (indexers first)
    match self.fetch_relay_list(pubkey).await {
        Ok(relay_list) => {
            let _ = relay_cache.cache_relay_list(relay_list.clone());
            return RelayDiscoveryResult {
                write_relays: relay_list.write_relays,
                read_relays: relay_list.read_relays,
                source: RelayDiscoverySource::RelayList,
            };
        }
        Err(_) => {
            // Continue to fallback tiers
        }
    }
    
    // Tier 3: Relay hints
    if let Some(hint_store) = hint_store {
        if let Ok(hints) = hint_store.get_hints(pubkey) {
            if !hints.is_empty() {
                return RelayDiscoveryResult {
                    write_relays: hints.clone(),
                    read_relays: hints,
                    source: RelayDiscoverySource::RelayHints,
                };
            }
        }
    }
    
    // Tier 4: Global fallbacks
    RelayDiscoveryResult {
        write_relays: DEFAULT_RELAYS.iter().map(|s| s.to_string()).collect(),
        read_relays: DEFAULT_RELAYS.iter().map(|s| s.to_string()).collect(),
        source: RelayDiscoverySource::GlobalFallback,
    }
}
```

- [ ] **Step 2: Add RelayDiscoveryResult struct if not present**

Add to `core/src/nostr.rs`:

```rust
/// Result of relay discovery with source attribution
#[derive(Debug, Clone)]
pub struct RelayDiscoveryResult {
    pub write_relays: Vec<String>,
    pub read_relays: Vec<String>,
    pub source: RelayDiscoverySource,
}
```

- [ ] **Step 3: Verify relay_cache has get_relay_list method**

Check that `core/src/relay_cache.rs` has:

```rust
/// Get cached relay list for a pubkey
pub fn get_relay_list(&self, pubkey: &str) -> Option<CachedRelayList> {
    let pubkey_relays = self.pubkey_relays.lock().ok()?;
    let entries = pubkey_relays.get(pubkey)?;
    
    let mut write = Vec::new();
    let mut read = Vec::new();
    
    for entry in entries {
        match entry.relay_type.as_str() {
            "write" => write.push(entry.url.clone()),
            "read" => read.push(entry.url.clone()),
            _ => {}
        }
    }
    
    Some(CachedRelayList {
        pubkey: pubkey.to_string(),
        write_relays: write,
        read_relays: read,
        updated_at: entries.first().map(|e| e.last_seen).unwrap_or(0),
    })
}
```

If not present, add it to `RelayCache` impl in `relay_cache.rs`.

- [ ] **Step 4: Commit**

```bash
git add core/src/nostr.rs core/src/relay_cache.rs
git commit -m "feat: integrate relay hints into discovery waterfall"
```

---

## Task 5: Desktop App Integration

**Files:**
- Modify: `desktop/src/main.rs` - Initialize extended network on auth
- Modify: `desktop/src/main.rs` - Add periodic refresh task
- Test: Manual verification

**Purpose:** Wire the extended network repository into the desktop app's lifecycle.

- [ ] **Step 1: Add extended network initialization on authentication**

In `desktop/src/main.rs`, find where `AppState` is created and add initialization:

```rust
// Around line 150-200 in the auth success handler:

// Initialize extended network discovery
if let Some(ref pubkey) = app_state.current_user_pubkey {
    let social_graph = Arc::new(
        SocialGraphDb::new(app_state.config_dir.join("social_graph.db"))
            .map_err(|e| format!("Failed to create social graph DB: {}", e))?
    );
    
    let extended_network = Arc::new(Mutex::new(
        ExtendedNetworkRepository::new(social_graph)
    ));
    
    extended_network.lock().unwrap().set_pubkey(pubkey.clone());
    
    app_state.extended_network = Some(extended_network.clone());
    
    // Spawn discovery task
    let nostr = app_state.nostr.clone();
    let first_degree = app_state.follows.iter().map(|f| f.pubkey.clone()).collect();
    let relay_cache = app_state.relay_cache.clone();
    let en_repo = extended_network.clone();
    
    tokio::spawn(async move {
        let repo = en_repo.lock().unwrap();
        match repo.discover_network(&nostr.lock().await, &relay_cache, first_degree).await {
            Ok(stats) => {
                info!("Extended network discovery complete: {} qualified, {} relays",
                    stats.qualified_count, stats.relays_covered);
            }
            Err(e) => {
                warn!("Extended network discovery failed: {}", e);
            }
        }
    });
}
```

- [ ] **Step 2: Add extended_network field to AppState**

In `desktop/src/main.rs`, add to `AppState` struct:

```rust
pub struct AppState {
    pub nostr: Arc<Mutex<NostrClient>>,
    pub relay_cache: Arc<RelayCache>,
    pub subscription_registry: Arc<SubscriptionRegistry>,
    pub extended_network: Option<Arc<Mutex<ExtendedNetworkRepository>>>, // NEW
    pub relay_hint_store: Option<Arc<RelayHintStore>>, // NEW
    // ... existing fields
}
```

- [ ] **Step 3: Initialize relay hint store**

In `desktop/src/main.rs`, during app initialization:

```rust
// Initialize relay hint store
let relay_hint_store = Arc::new(
    RelayHintStore::new(config_dir.join("relay_hints.db"))
        .map_err(|e| format!("Failed to create relay hint store: {}", e))?
);

app_state.relay_hint_store = Some(relay_hint_store);
```

- [ ] **Step 4: Wire relay hint extraction into event processing**

In the event processing loop (where events are received from relays), add:

```rust
// Extract relay hints from events
if let Some(ref hint_store) = app_state.relay_hint_store {
    if let Err(e) = hint_store.extract_hints_from_event(&event) {
        debug!("Failed to extract hints from event: {}", e);
    }
    
    // Also record author provenance (which relay delivered this event)
    // This requires passing the relay URL along with the event
    // hint_store.add_author_relay(&event.pubkey.to_hex(), relay_url);
}
```

- [ ] **Step 5: Add periodic flush task for relay hints**

Spawn a background task that flushes hints periodically:

```rust
// Spawn periodic hint flush
let hint_store = app_state.relay_hint_store.clone();
tokio::spawn(async move {
    let mut interval = tokio::time::interval(Duration::from_secs(60));
    loop {
        interval.tick().await;
        if let Some(ref store) = hint_store {
            if let Err(e) = store.flush() {
                warn!("Failed to flush relay hints: {}", e);
            } else {
                debug!("Flushed relay hints to database");
            }
        }
    }
});
```

- [ ] **Step 6: Add periodic extended network refresh**

Add a task that refreshes extended network every 24 hours:

```rust
// Spawn periodic extended network refresh
let extended_network = app_state.extended_network.clone();
let nostr = app_state.nostr.clone();
let relay_cache = app_state.relay_cache.clone();
let follows = app_state.follows.clone(); // Need to access follows

tokio::spawn(async move {
    let mut interval = tokio::time::interval(Duration::from_secs(86400)); // 24 hours
    loop {
        interval.tick().await;
        
        if let Some(ref en) = extended_network {
            let repo = en.lock().unwrap();
            
            // Only refresh if stale
            if repo.is_cache_stale() {
                info!("Extended network cache is stale, refreshing...");
                
                let first_degree = follows.iter().map(|f| f.pubkey.clone()).collect();
                match repo.discover_network(&nostr.lock().await, &relay_cache, first_degree).await {
                    Ok(stats) => {
                        info!("Extended network refresh complete: {} qualified, {} relays",
                            stats.qualified_count, stats.relays_covered);
                    }
                    Err(e) => {
                        warn!("Extended network refresh failed: {}", e);
                    }
                }
            }
        }
    }
});
```

- [ ] **Step 7: Commit**

```bash
git add desktop/src/main.rs
git commit -m "feat: integrate extended network and relay hints into desktop app"
```

---

## Task 6: Testing & Verification

**Files:**
- Test all modified modules
- Manual: Run desktop app and verify

- [ ] **Step 1: Run all core tests**

Run: `cargo test -p arcadestr-core`
Expected: All tests pass

- [ ] **Step 2: Verify compilation of entire workspace**

Run: `cargo check`
Expected: No errors

- [ ] **Step 3: Manual verification checklist**

1. Build desktop app: `cargo build -p arcadestr-desktop`
2. Run and authenticate with a NIP-46 signer
3. Check logs for extended network discovery progress
4. Verify no crashes during relay operations
5. Check that social_graph.db is created and populated
6. Check that relay_hints.db is created

- [ ] **Step 4: Add integration tests (optional)**

Create `core/tests/extended_network_integration.rs`:

```rust
//! Integration tests for extended network discovery

use arcadestr_core::extended_network::ExtendedNetworkRepository;
use arcadestr_core::social_graph::SocialGraphDb;
use std::sync::Arc;
use tempfile::TempDir;

#[tokio::test]
async fn test_end_to_end_discovery_flow() {
    let temp = TempDir::new().unwrap();
    let social_graph = Arc::new(SocialGraphDb::new(temp.path().join("social.db")).unwrap());
    let repo = ExtendedNetworkRepository::new(social_graph);
    
    // This would require a mock NostrClient
    // Just verify the structure compiles and basic operations work
    repo.set_pubkey("test_pubkey".to_string());
    assert!(repo.get_cached_network().is_none());
}
```

- [ ] **Step 5: Final commit**

```bash
git add core/tests/
git commit -m "test: add extended network integration tests"
```

---

## Summary

This plan implements:

1. **SocialGraphDb** - SQLite storage for 2nd-degree follow relationships (followed-by tracking)
2. **RelayHintStore** - LRU + SQLite cache for p-tag/e-tag relay hints (5 per pubkey, 2000 total)
3. **ExtendedNetworkRepository** - Full discovery pipeline:
   - Fetches kind 3 follow lists from 1st-degree follows
   - Counts 2nd-degree appearances (threshold >= 10)
   - Builds social graph in batches (5000 rows/batch)
   - Fetches relay lists for qualified pubkeys
   - Computes optimal relay coverage using greedy set-cover
   - Caches results with 24h TTL
4. **Integration** - Wires into existing relay discovery waterfall and desktop app lifecycle

**Key architectural decisions:**
- SQLite persistence with in-memory LRU caches (follows existing patterns)
- Batched operations to avoid blocking (5000 relationships/batch)
- Timeouts on all network operations (5-30s depending on phase)
- Max 100 extended network relays, 300 authors per relay cap
- Parallel to existing relay selection (doesn't replace it, augments it)
