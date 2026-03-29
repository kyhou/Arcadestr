# NIP-65 Relay Gossip Phase 2 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement event deduplication, relay health tracking, and idle timeout for NIP-65 relay gossip.

**Architecture:** Add EventDeduplicator struct to nostr.rs, extend RelayCache with health tracking, add RelayConnectionManager for idle timeout. Use nostr-sdk's built-in relay pool.

**Tech Stack:** Rust, nostr-sdk 0.44, rusqlite

---

## File Structure

```
core/
├── src/
│   ├── nostr.rs          # Add EventDeduplicator
│   ├── relay_cache.rs   # Add health tracking methods
│   └── lib.rs           # Export new types
```

---

## Task 1: Event De-duplication

**Files:**
- Modify: `core/src/nostr.rs`
- Test: Unit tests

- [ ] **Step 1: Add EventDeduplicator to nostr.rs**

Add after existing types:

```rust
use std::collections::HashSet;

/// Event deduplicator to prevent processing duplicate events from multiple relays
pub struct EventDeduplicator {
    seen_ids: HashSet<String>,
    max_size: usize,
}

impl EventDeduplicator {
    /// Create a new deduplicator with specified max size
    pub fn new(max_size: usize) -> Self {
        Self {
            seen_ids: HashSet::new(),
            max_size,
        }
    }

    /// Check if event was already seen, insert if not
    /// Returns true if this is a duplicate (already seen)
    pub fn check_and_insert(&mut self, event_id: &str) -> bool {
        // If we're at capacity, clear half the entries (simple eviction)
        if self.seen_ids.len() >= self.max_size {
            let half = self.max_size / 2;
            let ids: Vec<String> = self.seen_ids.iter().take(half).cloned().collect();
            self.seen_ids.clear();
            self.seen_ids.extend(ids);
        }
        
        // Check and insert
        !self.seen_ids.insert(event_id.to_string())
    }

    /// Clear all seen events
    pub fn clear(&mut self) {
        self.seen_ids.clear();
    }

    /// Get current count of seen events
    pub fn len(&self) -> usize {
        self.seen_ids.len()
    }
}
```

- [ ] **Step 2: Add unit tests**

```rust
#[cfg(test)]
mod dedup_tests {
    use super::*;

    #[test]
    fn test_deduplicator_new_event() {
        let mut dedup = EventDeduplicator::new(100);
        let is_dup = dedup.check_and_insert("event123");
        assert!(!is_dup); // First time, not a duplicate
    }

    #[test]
    fn test_deduplicator_duplicate_event() {
        let mut dedup = EventDeduplicator::new(100);
        let _ = dedup.check_and_insert("event123");
        let is_dup = dedup.check_and_insert("event123");
        assert!(is_dup); // Second time, is a duplicate
    }

    #[test]
    fn test_deduplicator_clear() {
        let mut dedup = EventDeduplicator::new(100);
        let _ = dedup.check_and_insert("event123");
        dedup.clear();
        let is_dup = dedup.check_and_insert("event123");
        assert!(!is_dup); // After clear, not a duplicate
    }
}
```

- [ ] **Step 3: Run tests**

```bash
cd .worktrees/nip65-relay-gossip && cargo test -p arcadestr-core dedup_tests
```

- [ ] **Step 4: Commit**

```bash
git add core/src/nostr.rs && git commit -m "feat(nip65): add EventDeduplicator for duplicate detection"
```

---

## Task 2: Relay Health Tracking

**Files:**
- Modify: `core/src/relay_cache.rs`
- Test: Unit tests

- [ ] **Step 1: Add RelayHealth struct and methods to relay_cache.rs**

Add after existing types:

```rust
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

impl RelayCache {
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
        let new_failed = if success { failed } else { failed.saturating_add(1) };
        let new_error_rate = if new_total > 0 {
            new_failed as f32 / new_total as f32
        } else {
            0.0
        };
        
        conn.execute(
            "INSERT OR REPLACE INTO relay_health 
             (relay_url, latency_ms, error_rate, last_checked, total_requests, failed_requests) 
             VALUES (?, ?, ?, ?, ?, ?)",
            rusqlite::params![relay_url, latency_ms, new_error_rate, now, new_total, new_failed],
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
}
```

- [ ] **Step 2: Update lib.rs exports**

```rust
pub use relay_cache::{RelayCache, CachedRelayList, RelayCacheError, RelayType, RelayHealth};
```

- [ ] **Step 3: Run tests**

```bash
cd .worktrees/nip65-relay-gossip && cargo test -p arcadestr-core relay_cache
```

- [ ] **Step 4: Commit**

```bash
git add core/src/relay_cache.rs core/src/lib.rs && git commit -m "feat(nip65): add relay health tracking"
```

---

## Task 3: Idle Timeout Management

**Files:**
- Modify: `core/src/nostr.rs`
- Test: Unit tests

- [ ] **Step 1: Add RelayConnectionManager to nostr.rs**

Add after EventDeduplicator:

```rust
use std::collections::HashMap;
use std::time::{Duration, Instant};

/// Manages relay connection idle timeouts
pub struct RelayConnectionManager {
    last_activity: HashMap<String, Instant>,
    idle_timeout: Duration,
}

impl RelayConnectionManager {
    /// Create a new manager with specified idle timeout
    /// Default: 5 minutes
    pub fn new(idle_timeout: Duration) -> Self {
        Self {
            last_activity: HashMap::new(),
            idle_timeout,
        }
    }

    /// Create with default 5-minute timeout
    pub fn with_default_timeout() -> Self {
        Self::new(Duration::from_secs(300))
    }

    /// Update last activity time for a relay
    pub fn touch(&mut self, relay_url: &str) {
        self.last_activity.insert(relay_url.to_string(), Instant::now());
    }

    /// Get relays that have been idle too long
    pub fn get_idle_relays(&self) -> Vec<String> {
        let now = Instant::now();
        self.last_activity
            .iter()
            .filter(|(_, last_seen)| now.duration_since(**last_seen) > self.idle_timeout)
            .map(|(url, _)| url.clone())
            .collect()
    }

    /// Clean up idle relays and return them
    pub fn cleanup(&mut self) -> Vec<String> {
        let idle = self.get_idle_relays();
        for url in &idle {
            self.last_activity.remove(url);
        }
        idle
    }

    /// Remove a specific relay
    pub fn remove(&mut self, relay_url: &str) {
        self.last_activity.remove(relay_url);
    }
}
```

- [ ] **Step 2: Add unit tests**

```rust
#[cfg(test)]
mod idle_timeout_tests {
    use super::*;
    use std::thread;
    use std::time::Duration;

    #[test]
    fn test_touch_updates_last_activity() {
        let mut manager = RelayConnectionManager::with_default_timeout();
        manager.touch("wss://relay.example.com");
        
        let idle = manager.get_idle_relays();
        assert!(idle.is_empty()); // Just touched, not idle
    }

    #[test]
    fn test_cleanup_removes_idle_relays() {
        let mut manager = RelayConnectionManager::new(Duration::from_millis(1));
        manager.touch("wss://relay.example.com");
        
        // Wait a bit
        thread::sleep(Duration::from_millis(10));
        
        let idle = manager.cleanup();
        assert!(idle.contains(&"wss://relay.example.com".to_string()));
        
        // Should be removed now
        let idle = manager.get_idle_relays();
        assert!(idle.is_empty());
    }
}
```

- [ ] **Step 3: Run tests**

```bash
cd .worktrees/nip65-relay-gossip && cargo test -p arcadestr-core idle_timeout_tests
```

- [ ] **Step 4: Commit**

```bash
git add core/src/nostr.rs && git commit -m "feat(nip65): add RelayConnectionManager for idle timeout"
```

---

## Task 4: Final Integration

**Files:**
- Test: Full test suite

- [ ] **Step 1: Run full test suite**

```bash
cd .worktrees/nip65-relay-gossip && cargo test -p arcadestr-core
```

- [ ] **Step 2: Commit**

```bash
git commit -m "feat(nip65): Phase 2 complete - deduplication, health tracking, idle timeout"
```

- [ ] **Step 3: Push branch**

```bash
git push origin feature/nip65-relay-gossip
```

---

## Summary

| Task | Description | Files Changed |
|------|-------------|---------------|
| 1 | Event De-duplication | core/src/nostr.rs |
| 2 | Relay Health Tracking | core/src/relay_cache.rs, lib.rs |
| 3 | Idle Timeout Management | core/src/nostr.rs |
| 4 | Final integration | - |
