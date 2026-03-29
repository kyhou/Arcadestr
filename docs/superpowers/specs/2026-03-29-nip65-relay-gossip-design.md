# NIP-65 Relay Gossip (Outbox Model) Implementation

**Date**: 2026-03-29  
**Status**: Approved  
**Feature**: Relay Gossip / Outbox Model for efficient relay discovery

---

## 1. Overview

Implement NIP-65 (Relay List Metadata) to enable efficient relay discovery in Arcadestr. This allows the client to:

1. Discover where followed users publish their notes (write relays)
2. Discover where to send replies so authors receive them (read relays)
3. Reduce connection overhead by targeting specific relays instead of broadcasting to all

---

## 2. Architecture

### 2.1 Component Structure

```
core/src/
├── nostr.rs          # Extended with NIP-65 methods
├── relay_cache.rs    # NEW: SQLite-based relay list cache
└── lib.rs            # Export new types

core/Cargo.toml       # Add rusqlite dependency
```

### 2.2 Key Types

```rust
/// Relay list entry from Kind 10002
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RelayEntry {
    pub url: String,
    pub relay_type: RelayType,  // "write" or "read"
    pub last_seen: u64,         // unix timestamp
}

/// Cached relay list for a pubkey
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachedRelayList {
    pub pubkey: String,
    pub write_relays: Vec<String>,
    pub read_relays: Vec<String>,
    pub updated_at: u64,
}

/// NIP-19 parsed identifier
#[derive(Debug, Clone)]
pub struct Nip19Identifier {
    pub pubkey: String,
    pub relays: Vec<String>,  // relay hints
}
```

---

## 3. Functionality

### 3.1 SQLite Cache Storage

**Location**: `core/src/relay_cache.rs`

**Schema**:
```sql
-- Relay lists cache
CREATE TABLE relay_lists (
    pubkey TEXT PRIMARY KEY,
    write_relays TEXT,    -- JSON array
    read_relays TEXT,     -- JSON array
    updated_at INTEGER
);

-- Seen-on tracker (which relays delivered events from a pubkey)
CREATE TABLE seen_on (
    pubkey TEXT,
    relay_url TEXT,
    last_seen INTEGER,
    PRIMARY KEY (pubkey, relay_url)
);

-- Relay health metrics
CREATE TABLE relay_health (
    relay_url TEXT PRIMARY KEY,
    latency_ms INTEGER,
    error_rate REAL,
    last_checked INTEGER
);
```

**Methods**:
- `new(db_path) -> RelayCache` — Initialize SQLite cache
- `get_relay_list(pubkey) -> Option<CachedRelayList>` — Get cached relays
- `save_relay_list(pubkey, relays)` — Store relay list
- `is_stale(pubkey) -> bool` — Check if >7 days old
- `update_seen_on(pubkey, relay_url)` — Record event delivery
- `get_seen_on(pubkey) -> Vec<String>` — Get relays that delivered events

### 3.2 NostrClient Extensions

Add these methods to `NostrClient`:

```rust
impl NostrClient {
    /// Fetch and cache Kind 10002 (relay list metadata) for a pubkey
    pub async fn fetch_relay_list(&self, npub: &str) 
        -> Result<CachedRelayList, NostrError>;

    /// Fetch Kind 3 (follow list) and extract relay hints from content
    pub async fn fetch_follow_list(&self, npub: &str) 
        -> Result<Vec<String>, NostrError>;

    /// Get relays for a pubkey, using cache + fallback
    pub async fn get_relays_for_pubkey(&self, npub: &str) 
        -> Result<RelayDiscoveryResult, NostrError>;

    /// Parse NIP-19 identifier (nprofile/nevent) to extract relay hints
    pub fn parse_nip19(identifier: &str) 
        -> Result<Nip19Identifier, NostrError>;

    /// Publish to outbox relays (own write relays + target's read relays)
    pub async fn publish_to_outbox(
        &self, 
        event: Event, 
        reply_target: Option<&Nip19Identifier>
    ) -> Result<(), NostrError>;

    /// Update seen_on tracker when events arrive
    pub fn update_seen_on(&self, pubkey: &str, relay_url: &str);
}
```

### 3.3 Fallback Discovery Waterfall

When a pubkey has no Kind 10002:

1. **Kind 10002** — Query bootstrap relays for relay list
2. **Kind 3 content** — Some older clients store relay maps in follow list content
3. **seen_on tracker** — Use relays that previously delivered events from this pubkey
4. **User's read relays** — Query user's home relays for the target pubkey
5. **Global aggregators** — Last resort: `wss://nos.lol`, `wss://relay.damus.io`

### 3.4 Outbox Publishing

When publishing a reply:

```
Rule A: Always include user's own write relays
Rule B: If replying, include the replied-to author's READ relays
```

This ensures the author receives the notification even if they don't listen to the sender's write relays.

---

## 4. Dependencies

### 4.1 New Cargo Dependencies

```toml
# core/Cargo.toml
[dependencies]
rusqlite = { version = "0.32", features = ["bundled"] }
```

### 4.2 Existing Dependencies Used

- `nostr-gossip` — Already a dependency of nostr-sdk, provides NIP-65 types
- `nostr` — Provides Kind 10002, Kind 3 parsing
- `nostr-sdk` — Client operations

---

## 5. Implementation Phases

### Phase 1: Core Infrastructure

- [ ] Add `rusqlite` to `core/Cargo.toml`
- [ ] Create `core/src/relay_cache.rs` with SQLite storage
- [ ] Implement `RelayCache` struct with CRUD operations
- [ ] Add staleness checking (>7 days)

### Phase 2: NIP-65 Fetching

- [ ] Add `fetch_relay_list` to NostrClient
- [ ] Parse Kind 10002 event content (JSON with `read` and `write` arrays)
- [ ] Add `fetch_follow_list` to NostrClient
- [ ] Implement fallback discovery waterfall

### Phase 3: NIP-19 Parsing

- [ ] Add `parse_nip19` to NostrClient
- [ ] Parse nprofile TLV to extract pubkey + relay hints
- [ ] Parse nevent TLV to extract pubkey + relay hints

### Phase 4: Outbox Publishing

- [ ] Add `publish_to_outbox` method
- [ ] Implement Rule A + Rule B relay selection
- [ ] Add ephemeral connection management for targeted publishing

---

## 6. Testing Strategy

- Unit tests for relay cache operations
- Integration tests for Kind 10002 parsing
- Manual testing with known users who have relay lists
- Test fallback discovery with users lacking Kind 10002

---

## 7. Future Enhancements (Phase 2+)

- Full relay selector algorithm with greedy set cover
- Targeted REQ dispatch (one filter per relay)
- Relay health tracking and scoring
- Idle timeout for permanent connections
- Event de-duplication

---

## 8. References

- [NIP-65: Relay List Metadata](https://github.com/nostr-protocol/nips/blob/master/65.md)
- [nostr-gossip crate](https://docs.rs/nostr-gossip/0.44)
- Implementation spec: User-provided NIP-65 instructions
