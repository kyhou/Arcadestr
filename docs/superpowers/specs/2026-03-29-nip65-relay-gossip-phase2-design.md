# NIP-65 Relay Gossip Phase 2 Implementation

**Date**: 2026-03-29  
**Status**: Approved  
**Feature**: Event deduplication, relay health tracking, idle timeout

---

## 1. Overview

Phase 2 enhances the NIP-65 implementation with:

1. **Event De-duplication** — Prevent duplicate events from multiple relay connections
2. **Relay Health Tracking** — Track latency and error rates per relay
3. **Idle Timeout** — Close connections after 5 minutes of inactivity

---

## 2. Architecture

### 2.1 Component Structure

```
core/src/
├── nostr.rs          # Extended with dedup wrapper, idle timeout
├── relay_cache.rs    # Extended with health tracking methods
└── lib.rs            # Export new types
```

### 2.2 Key Types

```rust
/// Relay health metrics
#[derive(Debug, Clone)]
pub struct RelayHealth {
    pub relay_url: String,
    pub latency_ms: u32,
    pub error_rate: f32,
    pub last_checked: u64,
}

/// Event deduplication state
pub struct EventDeduplicator {
    seen_ids: std::collections::HashSet<String>,
    max_size: usize,
}
```

---

## 3. Functionality

### 3.1 Event De-duplication

**Purpose**: The same event may arrive from multiple relays simultaneously. The client must maintain an in-memory set of seen event IDs.

**Implementation**:

```rust
pub struct EventDeduplicator {
    seen_ids: HashSet<String>,
    max_size: usize,  // Default: 10,000 events
}

impl EventDeduplicator {
    pub fn new(max_size: usize) -> Self;
    
    /// Check if event was already seen, add if not
    /// Returns true if this is a duplicate (already seen)
    pub fn check_and_insert(&mut self, event_id: &str) -> bool;
    
    /// Clear all seen events (e.g., on app restart)
    pub fn clear(&mut self);
}
```

**Usage**: Every time an event arrives from any relay:
```rust
if deduplicator.check_and_insert(&event.id) {
    // Duplicate - discard
    return;
}
// Process event...
```

### 3.2 Relay Health Tracking

**Purpose**: Track relay performance for scoring and selection.

**Schema** (extends existing relay_health table):

| Column | Type | Description |
|--------|------|-------------|
| relay_url | TEXT | Relay URL (primary key) |
| latency_ms | INTEGER | Average latency in milliseconds |
| error_rate | REAL | Fraction of requests that failed (0.0-1.0) |
| last_checked | INTEGER | Unix timestamp |

**Implementation**:

```rust
impl RelayCache {
    /// Update relay health after a request
    pub fn update_relay_health(
        &self,
        relay_url: &str,
        latency_ms: u32,
        success: bool,
    ) -> Result<(), RelayCacheError>;

    /// Get health for a specific relay
    pub fn get_relay_health(&self, relay_url: &str) -> Option<RelayHealth>;

    /// Get all relay health metrics, sorted by score
    pub fn get_all_relay_health(&self) -> Vec<RelayHealth>;
}
```

**Scoring Formula**:
```
score = base_score * health_factor * staleness_factor

health_factor = 
  - error_rate > 20% → × 0.7
  - otherwise → × 1.0

staleness_factor =
  - last_checked > 7 days ago → × 0.5
  - otherwise → × 1.0
```

### 3.3 Idle Timeout

**Purpose**: Close connections that haven't received events for N minutes to prevent resource exhaustion.

**Implementation**:

```rust
pub struct RelayConnectionManager {
    /// Track last activity time per relay
    last_activity: HashMap<String, std::time::Instant>,
    idle_timeout: std::time::Duration,
}

impl RelayConnectionManager {
    pub fn new(idle_timeout: std::time::Duration) -> Self;
    
    /// Update last activity for a relay
    pub fn touch(&mut self, relay_url: &str);
    
    /// Get relays that should be disconnected due to idle
    pub fn get_idle_relays(&self) -> Vec<String>;
    
    /// Run cleanup, returns relays to disconnect
    pub fn cleanup(&mut self) -> Vec<String>;
}
```

**Default**: 5 minutes (300 seconds) idle timeout

---

## 4. Integration with Existing Code

### 4.1 NostrClient Extensions

```rust
impl NostrClient {
    /// Create a new client with deduplication and connection management
    pub async fn new_with_features(
        relays: Vec<String>,
        relay_cache: RelayCache,
    ) -> Result<Self, NostrError>;

    /// Process incoming event with deduplication
    pub fn process_event(&mut self, event: Event) -> Option<Event>;
}
```

---

## 5. Dependencies

No new dependencies required. Uses existing:
- `rusqlite` for health metrics storage
- `std::collections::HashSet` for deduplication
- `std::time` for idle tracking

---

## 6. Implementation Phases

### Phase 2a: Event De-duplication

- [ ] Add `EventDeduplicator` struct
- [ ] Add `check_and_insert()` method
- [ ] Integrate with event processing
- [ ] Add unit tests

### Phase 2b: Relay Health Tracking

- [ ] Add health update methods to RelayCache
- [ ] Add latency measurement to relay operations
- [ ] Persist health metrics to SQLite
- [ ] Add retrieval methods

### Phase 2c: Idle Timeout

- [ ] Add `RelayConnectionManager` struct
- [ ] Implement 5-minute idle detection
- [ ] Add cleanup method for connection management
- [ ] Integrate with nostr-sdk's connection handling

---

## 7. Testing Strategy

- Unit tests for deduplication (insert, duplicate detection, clear)
- Unit tests for health tracking (update, retrieve)
- Unit tests for idle timeout (touch, get_idle_relays)
- Integration tests with mock relay responses

---

## 8. References

- NIP-65 Implementation Instructions (Section 9: De-duplication)
- NIP-65 Implementation Instructions (Section 8: Connection Lifecycle)
- Existing Phase 1 implementation in `core/src/nostr.rs`
