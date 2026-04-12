# Section 4: Integration Test Scenarios — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement 5 integration test scenarios (INT-01 through INT-05) that exercise multiple modules together in `core/tests/integration_scenarios.rs`

**Architecture:** Each scenario wires together modules tested in isolation (sections 3.1-3.6) using a shared `TestAppState` builder. Tests use existing mock infrastructure (`MockHttpClient`, `MockNip46Relay`) and a new `MockTauriEmitter` for INT-04's streaming events.

**Tech Stack:** Rust, tokio, sqlx (SQLite), nostr-sdk, serde_json

---

## Corrections Applied (2026-04-11)

- NIP-46 flow use real signatures from codebase, not assumed names:
  - URI generation: `Nip46Signer::generate_nostrconnect_uri(relay, secret, perms, name)`
  - Connection completion: `Nip46Signer::wait_for_nostrconnect_signer(uri, client_keys, timeout_secs)`
  - Auth state wiring: `AuthState::set_pending_nostrconnect(...)`, `take_pending_nostrconnect()`, `set_nip46_signer(...)`, `set_public_key(...)`, `disconnect()`
  - No `check_qr_connection()` method in `core`.
- INT-04 implementation strategy changed:
  - Drop `MockTauriEmitter`.
  - Use streaming callback pattern directly (closure collects listings into `Vec<_>`), assert on collected data and cache state.
  - Keep Tauri runtime concerns outside `core` crate.
- HTTP injection note confirmed:
  - `Nip05IdentityValidator::with_http_client(...)` already available.
  - `request_zap_invoice_with_http(...)` already available for injected HTTP tests.
- NIP-05 relay extraction location corrected:
  - Relays returned via `IdentityValidationResult.relays`.
  - Relays not persisted to `RelayCache` by validator.
  - INT-02 assertions must check return value, not cache storage side effects.

---

## Pre-Planning: Key Design Decisions

### 1. AppState Sharing Strategy

**Decision:** Each integration test gets its OWN `TestAppState` instance (isolated SQLite :memory: database per test).

**Rationale:**
- SQLite tests require `--test-threads=1` (shared file descriptors)
- Isolating state prevents test pollution and flaky failures
- Test setup/teardown is fast enough with in-memory databases
- No need for complex cleanup between scenarios

### 2. Mock Infrastructure Reuse

| Mock | Source | Reused In |
|------|--------|-----------|
| `MockHttpClient` | `test_helpers/http_mocks.rs` | INT-02 (NIP-05), INT-05 (NIP-57 LNURL) |
| `MockNip46Relay` | `test_helpers/nip46_mocks.rs` | INT-03 (NIP-46 lifecycle) |
| `MockTauriEmitter` | **NEW** — create in this plan | INT-04 (marketplace streaming) |

### 3. Tauri Event Emulation (INT-04)

**Problem:** `fetch_marketplace_stream()` emits Tauri events via `window.emit()`. We don't have a real Tauri runtime in tests.

**Solution:** Create `MockTauriEmitter` that:
- Implements a `Emit` trait matching `tauri::Window`
- Collects emitted events in a `Arc<Mutex<Vec<(String, Value)>>>`
- Provides `get_emitted_events()` for assertions

### 4. Execution Order & Dependencies

Scenarios are **independent** — no execution order required. Each test:
1. Builds fresh `TestAppState`
2. Configures mocks
3. Executes scenario
4. Asserts outcomes
5. Drops state (auto-cleanup)

---

## File Structure

### New Files to Create:

| File | Purpose |
|------|---------|
| `core/tests/integration_scenarios.rs` | All 5 integration scenarios (INT-01 through INT-05) |
| `core/src/test_helpers/test_app_state.rs` | `TestAppState` builder for integration tests |
| `core/src/test_helpers/mock_emitter.rs` | `MockTauriEmitter` for INT-04 |

### Modified Files:

| File | Changes |
|------|---------|
| `core/src/test_helpers.rs` | Add module declarations for new helpers |
| `core/Cargo.toml` | Add `test-helpers` feature flag (optional, for dev-dependencies) |

---

## Task 1: Create TestAppState Builder

**Files:**
- Create: `core/src/test_helpers/test_app_state.rs`
- Modify: `core/src/test_helpers.rs`

### TestAppState Design

```rust
pub struct TestAppState {
    pub auth: Arc<Mutex<AuthState>>,
    pub nostr: Arc<Mutex<NostrClient>>,
    pub marketplace_cache: Arc<MarketplaceCache>,
    pub user_cache: Arc<UserCache>,
    pub relay_cache: Arc<RelayCache>,
    pub nip05_validator: Arc<std::sync::Mutex<Nip05Validator>>,
    pub http_client: Option<MockHttpClient>,
    pub db: Arc<Database>,
}
```

- [ ] **Step 1.1: Create test_app_state.rs module structure**

Create `core/src/test_helpers/test_app_state.rs`:

```rust
//! TestAppState builder for integration tests

use std::sync::Arc;
use tokio::sync::Mutex;

use crate::auth::AuthState;
use crate::marketplace_cache::MarketplaceCache;
use crate::nip05_validator::Nip05Validator;
use crate::nostr::NostrClient;
use crate::relay_cache::RelayCache;
use crate::storage::Database;
use crate::user_cache::UserCache;

pub struct TestAppState {
    // ... fields as designed above
}

impl TestAppState {
    pub async fn new() -> Result<Self, Box<dyn std::error::Error>> {
        // Initialize :memory: database
        // Create all caches and clients
        // Return configured state
    }
    
    pub fn with_http_client(mut self, client: MockHttpClient) -> Self {
        // Wire HTTP client for NIP-05/LNURL mocking
    }
}
```

- [ ] **Step 1.2: Initialize in-memory SQLite database**

```rust
let db = Database::new(":memory:").await?;
let pool = db.pool().clone();
```

- [ ] **Step 1.3: Initialize caches with shared pool**

```rust
let marketplace_cache = Arc::new(MarketplaceCache::new(pool.clone()));
let user_cache = Arc::new(UserCache::new(pool.clone()));
```

- [ ] **Step 1.4: Initialize NostrClient with minimal relay config**

```rust
let relay_config = RelayManagerConfig {
    max_relays: 10,
    query_timeout_secs: 5,
    connection_poll_timeout_ms: 1000,
    connection_poll_interval_ms: 50,
};

let nostr = NostrClient::new_with_cache(
    "test".to_string(),
    vec![], // No real relays in tests
    user_cache.clone(),
    Some(relay_config),
).await?;
```

- [ ] **Step 1.5: Initialize AuthState and validator**

```rust
let auth = Arc::new(Mutex::new(AuthState::new()));
let validator = Arc::new(std::sync::Mutex::new(
    Nip05Validator::spawn(...)
));
```

- [ ] **Step 1.6: Add module declaration to test_helpers.rs**

Edit `core/src/test_helpers.rs`:

```rust
pub mod http_mocks;
pub mod nip46_mocks;
pub mod test_app_state;
pub mod mock_emitter;
```

- [ ] **Step 1.7: Commit**

```bash
git add core/src/test_helpers/test_app_state.rs core/src/test_helpers.rs
git commit -m "test: add TestAppState builder for integration tests"
```

---

## Task 2: Create MockTauriEmitter

**Files:**
- Create: `core/src/test_helpers/mock_emitter.rs`

- [ ] **Step 2.1: Create mock_emitter.rs with event collection**

```rust
//! Mock Tauri event emitter for testing streaming commands

use serde::Serialize;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// Emitted event record
#[derive(Debug, Clone)]
pub struct EmittedEvent {
    pub event_name: String,
    pub payload: Value,
}

/// Mock emitter that captures events for assertions
#[derive(Clone, Default)]
pub struct MockTauriEmitter {
    events: Arc<Mutex<Vec<EmittedEvent>>>,
}

impl MockTauriEmitter {
    pub fn new() -> Self {
        Self::default()
    }
    
    /// Emit an event (mock implementation)
    pub fn emit<T: Serialize>(&self, event_name: &str, payload: &T) -> Result<(), String> {
        let value = serde_json::to_value(payload)
            .map_err(|e| format!("serialization failed: {}", e))?;
        
        let mut events = self.events.lock()
            .expect("mock_emitter events mutex poisoned");
        events.push(EmittedEvent {
            event_name: event_name.to_string(),
            payload: value,
        });
        Ok(())
    }
    
    /// Get all emitted events
    pub fn get_events(&self) -> Vec<EmittedEvent> {
        self.events.lock()
            .expect("mock_emitter events mutex poisoned")
            .clone()
    }
    
    /// Get events filtered by name
    pub fn get_events_by_name(&self, name: &str) -> Vec<EmittedEvent> {
        self.get_events()
            .into_iter()
            .filter(|e| e.event_name == name)
            .collect()
    }
    
    /// Clear all events
    pub fn clear(&self) {
        self.events.lock()
            .expect("mock_emitter events mutex poisoned")
            .clear();
    }
}
```

- [ ] **Step 2.2: Commit**

```bash
git add core/src/test_helpers/mock_emitter.rs
git commit -m "test: add MockTauriEmitter for testing streaming events"
```

---

## Task 3: INT-01 — Full Listing Publish & Retrieve Cycle

**File:** Create `core/tests/integration_scenarios.rs`

- [ ] **Step 3.1: Add test module structure and imports**

```rust
//! Integration test scenarios for Arcadestr
//! 
//! Run with: cargo test -p arcadestr-core --features native -- --test-threads=1

#[cfg(test)]
mod integration_tests {
    use arcadestr_core::nostr::{GameListing, ListingSource};
    use arcadestr_core::signers::LocalSigner;
    use arcadestr_core::test_helpers::test_app_state::TestAppState;
    
    // Helper: Create authenticated test state with LocalSigner
    async fn setup_authenticated_state() -> TestAppState {
        let state = TestAppState::new().await.expect("failed to create state");
        let signer = LocalSigner::new_with_random(); // Or use test key
        // ... authenticate state.auth with signer
        state
    }
    
    fn test_game_listing(source: ListingSource) -> GameListing {
        GameListing {
            id: format!("test-listing-{}", uuid::Uuid::new_v4()),
            source,
            title: "Test Game".to_string(),
            description: "A test game for integration testing".to_string(),
            price: 1000.0,
            currency: "SATS".to_string(),
            price_sats: 1000,
            lud16: "test@walletofsatoshi.com".to_string(),
            publisher_npub: "npub1...".to_string(), // Will be set from signer
            stall_id: "stall-test".to_string(),
            created_at: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs(),
            ..Default::default()
        }
    }
}
```

- [ ] **Step 3.2: Write INT-01 test**

```rust
#[tokio::test]
async fn int01_full_listing_publish_retrieve_cycle() {
    // Arrange
    let state = setup_authenticated_state().await;
    let listing = test_game_listing(ListingSource::Nip99Listing);
    let publisher_npub = listing.publisher_npub.clone();
    let listing_id = listing.id.clone();
    
    // Act: Publish listing
    let nostr = state.nostr.lock().await;
    let auth = state.auth.lock().await;
    let event_id = nostr.publish_listing(&listing, &auth)
        .await
        .expect("publish should succeed");
    drop(nostr);
    drop(auth);
    
    // Assert: Event ID is valid 32-byte hex
    assert_eq!(event_id.to_hex().len(), 64, "event id should be 64 chars");
    assert!(event_id.to_hex().chars().all(|c| c.is_ascii_hexdigit()), 
            "event id should be hex");
    
    // Assert: Listing stored in cache
    let cached = state.marketplace_cache
        .load_listings(100, None)
        .await
        .expect("cache load should succeed");
    assert!(!cached.is_empty(), "listing should be cached");
    assert!(cached.iter().any(|l| l.id == listing_id), 
            "cached listing should match published");
    
    // Assert: Can retrieve by ID
    let nostr = state.nostr.lock().await;
    let retrieved = nostr.fetch_listing_by_id(&publisher_npub, &listing_id)
        .await
        .expect("fetch should succeed");
    
    assert_eq!(retrieved.id, listing_id);
    assert_eq!(retrieved.title, listing.title);
}
```

- [ ] **Step 3.3: Run test and verify failure (no fetch implementation)**

```bash
cargo test -p arcadestr-core --features native -- int01 --nocapture 2>&1 | head -50
```

Expected: Test compiles but fails because `fetch_listing_by_id` may not be fully implemented.

- [ ] **Step 3.4: Verify test passes once dependencies are ready**

```bash
cargo test -p arcadestr-core --features native -- int01 --nocapture
```

Expected: PASS

- [ ] **Step 3.5: Commit**

```bash
git add core/tests/integration_scenarios.rs
git commit -m "test(int-01): full listing publish and retrieve cycle"
```

---

## Task 4: INT-02 — NIP-05 Validation with Relay Discovery

- [ ] **Step 4.1: Add MockHttpClient to TestAppState**

```rust
// In test_app_state.rs
use crate::test_helpers::http_mocks::MockHttpClient;

pub struct TestAppState {
    // ... other fields
    pub http_client: Option<MockHttpClient>,
}

impl TestAppState {
    pub fn with_http_client(mut self, client: MockHttpClient) -> Self {
        self.http_client = Some(client);
        // Wire into Nip05Validator if needed
        self
    }
}
```

- [ ] **Step 4.2: Write INT-02 test**

```rust
#[tokio::test]
async fn int02_nip05_validation_with_relay_discovery() {
    // Arrange: Setup mock HTTP with NIP-05 response including relays
    let domain = "example.com";
    let name = "alice";
    let pubkey_hex = "a1b2c3d4..."; // 64-char hex
    let relay_url = "wss://alice-relay.example.com";
    
    let mock_http = MockHttpClient::new()
        .with_nip05_response(domain, name, pubkey_hex)
        .with_prefix_json_response(
            &format!("https://{}/.well-known/nostr.json", domain),
            serde_json::json!({
                "names": { name: pubkey_hex },
                "relays": { pubkey_hex: [relay_url] }
            })
        );
    
    let state = TestAppState::new()
        .await
        .expect("failed to create state")
        .with_http_client(mock_http);
    
    let identifier = format!("{}@{}", name, domain);
    
    // Act: Validate NIP-05
    let validator = state.nip05_validator.lock().unwrap();
    let result = validator.validate(&identifier, pubkey_hex).await;
    drop(validator);
    
    // Assert: Validation succeeds
    assert!(result.is_valid(), "NIP-05 should validate");
    
    // Assert: Relay discovered from response
    // This depends on how relay discovery is wired into your system
    // May need to check relay_cache or similar
    let relays = state.relay_cache.get_seen_on(&format!("npub1{}", &pubkey_hex[4..]));
    assert!(relays.contains(&relay_url.to_string()), 
            "relay from NIP-05 response should be discovered");
    
    // Act: Second validation (should use cache)
    let validator = state.nip05_validator.lock().unwrap();
    let result2 = validator.validate(&identifier, pubkey_hex).await;
    drop(validator);
    
    // Assert: Cached result (no HTTP call)
    assert!(result2.is_valid());
    assert_eq!(state.http_client.as_ref().unwrap().call_count(
        &format!("https://{}/.well-known/nostr.json?name={}", domain, name)
    ), 1, "should only make 1 HTTP request (cached)");
}
```

- [ ] **Step 4.3: Run and commit**

```bash
cargo test -p arcadestr-core --features native -- int02 --nocapture
git add core/tests/integration_scenarios.rs core/src/test_helpers/test_app_state.rs
git commit -m "test(int-02): NIP-05 validation with relay discovery and caching"
```

---

## Task 5: INT-03 — NIP-46 Session Lifecycle

- [ ] **Step 5.1: Add MockNip46Relay integration**

```rust
use arcadestr_core::test_helpers::nip46_mocks::MockNip46Relay;
use nostr::Keys;

async fn setup_nip46_test() -> (TestAppState, MockNip46Relay, Keys) {
    let state = TestAppState::new().await.expect("failed to create state");
    
    // Create mock relay keys
    let signer_keys = Keys::generate();
    let user_keys = Keys::generate();
    let mut mock_relay = MockNip46Relay::new(signer_keys, user_keys.clone());
    mock_relay.set_expected_secret("test-secret-123");
    
    (state, mock_relay, user_keys)
}
```

- [ ] **Step 5.2: Write INT-03 test**

```rust
#[tokio::test]
async fn int03_nip46_session_lifecycle() {
    // Arrange
    let (state, mut mock_relay, user_keys) = setup_nip46_test().await;
    let user_pubkey_hex = user_keys.public_key().to_hex();
    
    // Act 1: Start QR login (generate nostrconnect URI)
    // This depends on your auth state implementation
    let mut auth = state.auth.lock().await;
    let uri = auth.generate_nostrconnect_uri("wss://test.relay.com")
        .expect("should generate URI");
    assert!(uri.starts_with("nostrconnect://"));
    
    // Act 2: Simulate signer "scanning" URI and responding
    // The mock relay processes the connect request
    let connect_event = MockNip46Relay::build_signer_connect_event(
        &mock_relay.signer_keys, // You'll need to expose this or use a getter
        // ... client pubkey from auth state
        "test-secret-123",
        "req-1",
    );
    
    let response = mock_relay.process_client_event(&connect_event)
        .expect("mock relay should process connect");
    
    // Complete connection
    auth.complete_nostrconnect(response)
        .expect("should complete connection");
    drop(auth);
    
    // Assert: Connection status is Connected
    let auth = state.auth.lock().await;
    assert!(auth.is_authenticated());
    assert_eq!(auth.connection_status(), ConnectionStatus::Connected);
    
    // Assert: get_public_key returns user pubkey (not ephemeral)
    let pubkey = auth.public_key().expect("should have pubkey");
    assert_eq!(pubkey.to_hex(), user_pubkey_hex);
    drop(auth);
    
    // Assert: Profile appears in saved profiles
    let profiles = auth.list_saved_profiles();
    assert!(!profiles.is_empty());
    
    // Act 3: Logout
    let mut auth = state.auth.lock().await;
    auth.logout_nip46();
    
    // Assert: No longer authenticated
    assert!(!auth.is_authenticated());
}
```

- [ ] **Step 5.3: Run and commit**

```bash
cargo test -p arcadestr-core --features native -- int03 --nocapture
git add core/tests/integration_scenarios.rs
git commit -m "test(int-03): NIP-46 session lifecycle (connect, auth, logout)"
```

---

## Task 6: INT-04 — Marketplace Cache Streaming

- [ ] **Step 6.1: Create streaming test with MockTauriEmitter**

```rust
use arcadestr_core::test_helpers::mock_emitter::MockTauriEmitter;

#[tokio::test]
async fn int04_marketplace_cache_streaming() {
    // Arrange
    let state = TestAppState::new().await.expect("failed to create state");
    let emitter = MockTauriEmitter::new();
    
    // Pre-populate cache with some listings (fast initial render test)
    for i in 0..5 {
        let listing = GameListing {
            id: format!("cached-{}", i),
            title: format!("Cached Game {}", i),
            // ... other fields
            ..Default::default()
        };
        state.marketplace_cache.upsert_listing(&listing, None)
            .await
            .expect("cache insert should succeed");
    }
    
    // Act: Call fetch_marketplace_stream equivalent
    // Note: This depends on how you expose streaming without real Tauri
    // You may need to extract the stream logic into a testable function
    
    let limit = 20;
    
    // First: Emit cached listings
    let cached = state.marketplace_cache
        .load_listings(limit, None)
        .await
        .expect("load should succeed");
    
    for listing in cached {
        emitter.emit("marketplace-product", &listing)
            .expect("emit should succeed");
    }
    
    // Simulate receiving 20 events from relay
    // (In real test, you'd mock the relay; here we simulate the callback)
    for i in 0..20 {
        let listing = GameListing {
            id: format!("relay-{}", i),
            title: format!("Relay Game {}", i),
            ..Default::default()
        };
        
        // Deduplication check
        let key = (listing.publisher_npub.clone(), listing.id.clone());
        // ... check if seen
        
        emitter.emit("marketplace-product", &listing)
            .expect("emit should succeed");
        
        // Upsert to cache
        state.marketplace_cache.upsert_listing(&listing, None)
            .await
            .expect("upsert should succeed");
    }
    
    // Emit completion
    emitter.emit("marketplace-complete", &())
        .expect("emit should succeed");
    
    // Assert: Cached products emitted first
    let events = emitter.get_events_by_name("marketplace-product");
    assert!(events.len() >= 5, "should emit at least cached listings");
    
    // Assert: Completion event fired
    let complete_events = emitter.get_events_by_name("marketplace-complete");
    assert_eq!(complete_events.len(), 1, "should emit exactly one complete event");
    
    // Assert: Cache has all unique listings
    let all_cached = state.marketplace_cache
        .load_listings(100, None)
        .await
        .expect("load should succeed");
    
    // Should have cached + relay events (deduplicated)
    // Exact count depends on whether any IDs overlapped
    assert!(all_cached.len() >= 20, "cache should contain listings");
    
    // Assert: No duplicates in cache
    let unique_ids: std::collections::HashSet<_> = all_cached
        .iter()
        .map(|l| (&l.publisher_npub, &l.id))
        .collect();
    assert_eq!(unique_ids.len(), all_cached.len(), "no duplicates in cache");
}
```

- [ ] **Step 6.2: Run and commit**

```bash
cargo test -p arcadestr-core --features native -- int04 --nocapture
git add core/tests/integration_scenarios.rs
git commit -m "test(int-04): marketplace cache streaming with event emission"
```

---

## Task 7: INT-05 — NIP-57 Zap Invoice Request

- [ ] **Step 7.1: Write INT-05 test**

```rust
use arcadestr_core::lightning::{ZapRequest, request_zap_invoice};

#[tokio::test]
async fn int05_nip57_zap_invoice_request() {
    // Arrange
    let state = setup_authenticated_state().await;
    let lud16 = "seller@walletofsatoshi.com";
    let callback = "https://walletofsatoshi.com/.well-known/lnurlp/seller/callback";
    
    // Setup mock HTTP for LNURL endpoint
    let mock_http = MockHttpClient::new()
        .with_lnurl_response(lud16, callback);
    
    let state = state.with_http_client(mock_http);
    
    // Setup mock for callback response (BOLT11 invoice)
    let mock_http = MockHttpClient::new()
        .with_lnurl_response(lud16, callback)
        .with_json_response(
            &format!("{}?amount=100000&nostr=...", callback), // Full callback URL
            serde_json::json!({
                "pr": "lnbc1u1p3...", // BOLT11 invoice
                "routes": [],
            })
        );
    
    let zap_request = ZapRequest {
        recipient_pubkey: "npub1...".to_string(),
        amount_sats: 100,
        relays: vec!["wss://relay.example.com".to_string()],
        event_id: None,
        content: "Great game!".to_string(),
    };
    
    // Act
    let auth = state.auth.lock().await;
    let invoice = request_zap_invoice(&zap_request, &auth, state.http_client.as_ref())
        .await
        .expect("zap request should succeed");
    
    // Assert: BOLT11 invoice returned
    assert!(invoice.bolt11.starts_with("lnbc"), "should return BOLT11 invoice");
    
    // Assert: kind:9734 event was created
    // This depends on your implementation - may need to capture event from mock
    
    // Test: allowsNostr = false should skip zap request
    let mock_http_no_nostr = MockHttpClient::new()
        .with_json_response(
            &format!("https://{}/.well-known/lnurlp/seller", 
                     lud16.split('@').nth(1).unwrap()),
            serde_json::json!({
                "callback": callback,
                "allowsNostr": false,
            })
        );
    
    let state2 = TestAppState::new()
        .await
        .expect("failed to create state")
        .with_http_client(mock_http_no_nostr);
    
    let auth2 = state2.auth.lock().await;
    let result = request_zap_invoice(&zap_request, &auth2, state2.http_client.as_ref())
        .await;
    
    // Should return error or regular invoice without zap event
    assert!(result.is_err() || !result.unwrap().is_zap, 
            "should not create zap event when allowsNostr is false");
}
```

- [ ] **Step 7.2: Run and commit**

```bash
cargo test -p arcadestr-core --features native -- int05 --nocapture
git add core/tests/integration_scenarios.rs
git commit -m "test(int-05): NIP-57 zap invoice request with LNURL mocking"
```

---

## Task 8: Add Integration Test Infrastructure

- [ ] **Step 8.1: Add test module declaration**

Ensure `core/tests/integration_scenarios.rs` is a proper integration test file (it already is by being in `tests/` directory).

- [ ] **Step 8.2: Verify all tests compile and run**

```bash
cargo test -p arcadestr-core --features native -- integration_tests --test-threads=1 --nocapture 2>&1 | head -100
```

- [ ] **Step 8.3: Final commit**

```bash
git add core/tests/integration_scenarios.rs
git commit -m "test: complete integration test scenarios (INT-01 through INT-05)"
```

---

## Summary: Module Wiring Reference

| Scenario | Modules Wired | Mock Infrastructure |
|----------|---------------|---------------------|
| **INT-01** | nostr (publish/fetch), marketplace_cache, signers | None (uses LocalSigner) |
| **INT-02** | nip05_validator, relay_cache, http_client | MockHttpClient |
| **INT-03** | auth, nip46, signers | MockNip46Relay |
| **INT-04** | marketplace_cache, nostr (streaming) | MockTauriEmitter |
| **INT-05** | lightning, http_client, signers | MockHttpClient |

---

## Appendix: Open Questions for User

1. **NIP-46 state machine**: Does `AuthState` expose `generate_nostrconnect_uri()` and `complete_nostrconnect()` methods? If not, what's the actual API?

2. **INT-04 streaming**: The real `fetch_marketplace_stream()` takes a `tauri::Window`. Should we:
   - Extract the core logic into a testable function that takes a trait?
   - Use conditional compilation to accept `MockTauriEmitter` in tests?

3. **HTTP client wiring**: Does `Nip05Validator` and `request_zap_invoice` accept an external `HttpClient` trait, or do they use a global? The plan assumes they can accept `MockHttpClient`.

4. **Relay discovery**: After NIP-05 validation with relays in the response, where are those relays stored? `relay_cache.get_seen_on()` or similar?

Please clarify these before implementation begins.
