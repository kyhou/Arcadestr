# Arcadestr Nostr Integration Wiring Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Wire together Arcadestr's existing Nostr infrastructure (ProfileFetcher, UserCache, RelayCache, SubscriptionRegistry) into a cohesive, efficient system matching Amethyst/Wisp patterns with background NIP-05 validation, working notification loops, and ephemeral subscriptions.

**Architecture:** 
- Background worker pattern for async NIP-05 validation using tokio channels
- Direct Client access from RelayManager for notification loop (bypassing deprecated inner_clone)
- Subscription lifecycle management with registry tracking and automatic cleanup
- End-to-end profile flow: batch fetch → cache → background verify → UI update

**Tech Stack:** Rust, tokio, nostr-sdk 0.44, Tauri v2, Leptos 0.8, sqlx, rusqlite

---

## File Structure

### New Files
- `core/src/nip05_validator.rs` - Background NIP-05 validation worker with queue/channel
- `core/src/subscription_manager.rs` - High-level subscription management facade

### Modified Files
- `core/src/relay_manager.rs` - Add `get_client()` method to expose internal Client
- `core/src/nostr.rs` - Add `spawn_nip05_validator()` method to NostrClient
- `core/src/profile_fetcher.rs` - Emit events for NIP-05 validation queue
- `core/src/lib.rs` - Export new modules
- `desktop/src/main.rs` - Wire notification loop to actual Client, activate ephemeral subscriptions
- `core/src/subscriptions.rs` - Add subscription cleanup helpers

---

## Task 1: Add Client Access to RelayManager

**Files:**
- Modify: `core/src/relay_manager.rs:71-76` (add getter)
- Test: Existing tests in `core/src/relay_manager.rs`

**Purpose:** Enable notification loop to access the actual nostr_sdk Client instead of deprecated inner_clone() path.

- [ ] **Step 1: Add `get_client()` method to RelayManager**

Add a public method to expose the internal Client for subscription management:

```rust
/// Get a reference to the internal nostr_sdk Client.
/// Used for subscription management and notification loops.
pub fn get_client(&self) -> &Client {
    &self.client
}

/// Get an owned Arc<Client> for spawning notification loops.
pub fn get_client_arc(&self) -> Arc<Client> {
    Arc::new(self.client.clone())
}
```

Insert after line 76 (after the `shutdown` field declaration in the struct).

- [ ] **Step 2: Verify compilation**

Run: `cargo check -p arcadestr-core`
Expected: No errors

- [ ] **Step 3: Commit**

```bash
git add core/src/relay_manager.rs
git commit -m "feat: add Client access methods to RelayManager for notification loop"
```

---

## Task 2: Create NIP-05 Background Validator

**Files:**
- Create: `core/src/nip05_validator.rs`
- Modify: `core/src/lib.rs` (add module export)
- Test: `core/src/nip05_validator.rs` (add tests at end)

**Purpose:** Async background worker that validates NIP-05 identifiers without blocking UI or profile fetching.

- [ ] **Step 1: Create NIP-05 validator module**

Create `core/src/nip05_validator.rs`:

```rust
//! Background NIP-05 validation worker
//! 
//! Validates NIP-05 identifiers asynchronously without blocking the UI.
//! Uses a queue-based system where profiles are queued for validation
//! and processed in the background.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;
use tracing::{debug, info, warn};

use crate::nostr::NostrClient;
use crate::user_cache::UserCache;

/// Command sent to the validator worker
#[derive(Debug, Clone)]
pub enum ValidationCommand {
    /// Queue a profile for NIP-05 validation
    Validate { npub: String, nip05: String },
    /// Shutdown the worker
    Shutdown,
}

/// Result of NIP-05 validation
#[derive(Debug, Clone)]
pub struct ValidationResult {
    pub npub: String,
    pub nip05: String,
    pub verified: bool,
}

/// Background NIP-05 validation worker
pub struct Nip05Validator {
    command_tx: mpsc::UnboundedSender<ValidationCommand>,
    result_rx: mpsc::UnboundedReceiver<ValidationResult>,
}

impl Nip05Validator {
    /// Spawn a new validator worker
    pub fn spawn(client: Arc<NostrClient>, user_cache: Arc<UserCache>) -> Self {
        let (command_tx, mut command_rx) = mpsc::unbounded_channel();
        let (result_tx, result_rx) = mpsc::unbounded_channel();

        tokio::spawn(async move {
            let mut queue: VecDeque<(String, String)> = VecDeque::new();
            let mut shutdown = false;

            loop {
                // Process commands or wait
                tokio::select! {
                    Some(cmd) = command_rx.recv() => {
                        match cmd {
                            ValidationCommand::Validate { npub, nip05 } => {
                                debug!("Queued NIP-05 validation for {}", npub);
                                queue.push_back((npub, nip05));
                            }
                            ValidationCommand::Shutdown => {
                                info!("NIP-05 validator shutting down");
                                shutdown = true;
                            }
                        }
                    }
                    _ = tokio::time::sleep(tokio::time::Duration::from_secs(1)), if !queue.is_empty() => {
                        // Process next item in queue
                        if let Some((npub, nip05)) = queue.pop_front() {
                            debug!("Validating NIP-05 for {}: {}", npub, nip05);
                            
                            // Perform validation
                            let verified = client.verify_nip05(&npub, &nip05).await;
                            
                            if verified {
                                info!("NIP-05 verified for {}: {}", npub, nip05);
                                
                                // Update cache with verified status
                                if let Some(mut profile) = user_cache.get(&npub).await {
                                    profile.nip05_verified = true;
                                    if let Err(e) = user_cache.put(&npub, &profile).await {
                                        warn!("Failed to update verified status in cache: {}", e);
                                    }
                                }
                            } else {
                                warn!("NIP-05 verification failed for {}: {}", npub, nip05);
                            }
                            
                            // Send result
                            let _ = result_tx.send(ValidationResult {
                                npub: npub.clone(),
                                nip05: nip05.clone(),
                                verified,
                            });
                        }
                    }
                    else => {
                        if shutdown && queue.is_empty() {
                            break;
                        }
                        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
                    }
                }
            }
        });

        Self {
            command_tx,
            result_rx,
        }
    }

    /// Queue a profile for NIP-05 validation
    pub fn queue_validation(&self, npub: String, nip05: String) {
        let _ = self.command_tx.send(ValidationCommand::Validate { npub, nip05 });
    }

    /// Try to receive a validation result (non-blocking)
    pub fn try_recv_result(&mut self) -> Option<ValidationResult> {
        self.result_rx.try_recv().ok()
    }

    /// Shutdown the validator
    pub fn shutdown(&self) {
        let _ = self.command_tx.send(ValidationCommand::Shutdown);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validation_command_clone() {
        let cmd = ValidationCommand::Validate {
            npub: "test".to_string(),
            nip05: "test@example.com".to_string(),
        };
        let cloned = cmd.clone();
        
        match (cmd, cloned) {
            (ValidationCommand::Validate { npub: n1, nip05: nip1 }, 
             ValidationCommand::Validate { npub: n2, nip05: nip2 }) => {
                assert_eq!(n1, n2);
                assert_eq!(nip1, nip2);
            }
            _ => panic!("Clone mismatch"),
        }
    }
}
```

- [ ] **Step 2: Export module in lib.rs**

Modify `core/src/lib.rs` around line 51:

```rust
pub mod nip05_validator;
```

Add after `pub mod profile_fetcher;` on line 48.

Also add to the public exports around line 65:

```rust
pub use nip05_validator::{Nip05Validator, ValidationCommand, ValidationResult};
```

- [ ] **Step 3: Verify compilation**

Run: `cargo check -p arcadestr-core`
Expected: No errors

- [ ] **Step 4: Commit**

```bash
git add core/src/nip05_validator.rs core/src/lib.rs
git commit -m "feat: add background NIP-05 validation worker"
```

---

## Task 3: Integrate NIP-05 Validator with ProfileFetcher

**Files:**
- Modify: `core/src/profile_fetcher.rs:89-105` (add validator field)
- Modify: `core/src/profile_fetcher.rs:107-134` (update constructors)
- Modify: `core/src/profile_fetcher.rs:377-393` (queue for validation after parse)

**Purpose:** Automatically queue profiles with NIP-05 for background validation after fetching.

- [ ] **Step 1: Add validator field to ProfileFetcher**

Modify the struct definition (around line 89):

```rust
/// Batched profile fetcher with queue management
pub struct ProfileFetcher {
    /// Pending profiles to fetch
    pending: Arc<Mutex<VecDeque<String>>>,
    /// Currently in-flight fetches (prevents duplicates)
    in_flight: Arc<Mutex<HashSet<String>>>,
    /// Failed profiles with attempt count
    failed_attempts: Arc<Mutex<HashMap<String, u32>>>,
    /// Cache backend (swappable)
    cache: Arc<dyn ProfileCache>,
    /// Persistent SQLite cache for profile storage
    persistent_cache: Option<Arc<UserCache>>,
    /// Maximum retry attempts
    max_attempts: u32,
    /// Batch size for fetching
    batch_size: usize,
    /// Optional NIP-05 validator for background verification
    nip05_validator: Option<Arc<Mutex<Nip05Validator>>>,
}
```

- [ ] **Step 2: Add setter for NIP-05 validator**

Add a new method after `with_persistent_cache()` (around line 134):

```rust
/// Attach a NIP-05 validator for background verification
pub fn with_nip05_validator(&mut self, validator: Arc<Mutex<Nip05Validator>>) {
    self.nip05_validator = Some(validator);
}
```

- [ ] **Step 3: Queue profiles for NIP-05 validation after parsing**

Modify `parse_profile_event()` (around line 377) to queue for validation:

```rust
/// Parse a profile event into UserProfile
fn parse_profile_event(&self, event: &Event, npub: &str) -> Result<UserProfile, NostrError> {
    // Parse the event content as UserProfileContent
    let content: UserProfileContent = serde_json::from_str(&event.content).unwrap_or_default();

    let profile = UserProfile {
        npub: npub.to_string(),
        name: content.name,
        display_name: content.display_name,
        about: content.about,
        picture: content.picture,
        website: content.website,
        nip05: content.nip05.clone(),
        lud16: content.lud16,
        nip05_verified: false,
    };
    
    // Queue for NIP-05 validation if identifier present
    if let Some(ref nip05) = content.nip05 {
        if let Some(ref validator) = self.nip05_validator {
            if let Ok(v) = validator.lock() {
                v.queue_validation(npub.to_string(), nip05.clone());
                tracing::debug!("Queued NIP-05 validation for {}: {}", npub, nip05);
            }
        }
    }

    Ok(profile)
}
```

- [ ] **Step 4: Update Default impl**

Ensure `Default` impl sets `nip05_validator: None` (around line 396):

```rust
impl Default for ProfileFetcher {
    fn default() -> Self {
        Self {
            pending: Arc::new(Mutex::new(VecDeque::new())),
            in_flight: Arc::new(Mutex::new(HashSet::new())),
            failed_attempts: Arc::new(Mutex::new(HashMap::new())),
            cache: Arc::new(LruProfileCache::new(PROFILE_CACHE_SIZE, CACHE_TTL_SECONDS)),
            persistent_cache: None,
            max_attempts: MAX_PROFILE_ATTEMPTS,
            batch_size: BATCH_SIZE,
            nip05_validator: None,
        }
    }
}
```

- [ ] **Step 5: Verify compilation**

Run: `cargo check -p arcadestr-core`
Expected: No errors

- [ ] **Step 6: Commit**

```bash
git add core/src/profile_fetcher.rs
git commit -m "feat: integrate NIP-05 validator with ProfileFetcher"
```

---

## Task 4: Wire NIP-05 Validator in Desktop Main

**Files:**
- Modify: `desktop/src/main.rs:28-36` (add imports)
- Modify: `desktop/src/main.rs:525-540` (spawn validator)
- Modify: `desktop/src/main.rs:531-533` (attach to ProfileFetcher)

**Purpose:** Spawn the NIP-05 validator and attach it to the ProfileFetcher at startup.

- [ ] **Step 1: Add imports**

Add to existing imports around line 28:

```rust
use arcadestr_core::nip05_validator::Nip05Validator;
```

- [ ] **Step 2: Spawn validator after creating NostrClient**

After line 525 (after creating subscription_registry), add:

```rust
    // Spawn NIP-05 background validator
    let nip05_validator = {
        let client = nostr_client.lock().await;
        // Create a separate client instance for the validator
        // (to avoid lock contention during validation)
        let validator_client = match NostrClient::new_with_cache(
            user_id.clone().unwrap_or_else(|| "default".to_string()),
            vec![],
            user_cache.clone(),
            None,
        ).await {
            Ok(c) => Arc::new(c),
            Err(e) => {
                warn!("Failed to create validator client: {}", e);
                nostr_client.clone() // Fallback to shared client
            }
        };
        Arc::new(Mutex::new(Nip05Validator::spawn(validator_client, user_cache.clone())))
    };
    info!("NIP-05 background validator spawned");
```

- [ ] **Step 3: Attach validator to ProfileFetcher**

Modify the ProfileFetcher initialization (around line 531):

```rust
    // Initialize ProfileFetcher with persistent cache and NIP-05 validator
    let profile_fetcher = Arc::new({
        let mut fetcher = ProfileFetcher::with_persistent_cache(user_cache.clone());
        fetcher.with_nip05_validator(nip05_validator.clone());
        fetcher
    });
    info!("ProfileFetcher initialized with persistent cache and NIP-05 validator");
```

- [ ] **Step 4: Add validator to AppState**

Modify the `AppState` struct (around line 53) to include validator:

```rust
/// Application state shared across Tauri commands
pub struct AppState {
    /// Nostr client for relay operations
    pub nostr: Arc<Mutex<NostrClient>>,
    /// Relay cache for NIP-65 relay list management
    pub relay_cache: Arc<RelayCache>,
    /// Relay hints for pubkey discovery
    pub relay_hints: Option<Arc<RelayHints>>,
    /// Subscription registry for managing connection types
    pub subscription_registry: Arc<SubscriptionRegistry>,
    /// Profile fetcher for batched profile fetching
    pub profile_fetcher: Arc<ProfileFetcher>,
    /// User cache for persistent profile storage
    pub user_cache: Arc<UserCache>,
    /// NIP-05 validator for background verification
    pub nip05_validator: Arc<Mutex<Nip05Validator>>,
    /// Extended network repository for 2nd-degree follow discovery
    pub extended_network: Arc<RwLock<Option<Arc<Mutex<ExtendedNetworkRepository>>>>>,
    /// Extended network follows (cached)
    pub extended_network_follows: Arc<RwLock<Vec<String>>>,
}
```

- [ ] **Step 5: Update AppState construction**

Find where AppState is constructed (around line 1650) and add the validator field:

```rust
        AppState {
            nostr: nostr_client,
            relay_cache,
            relay_hints: Some(relay_hints),
            subscription_registry,
            profile_fetcher,
            user_cache,
            nip05_validator,
            extended_network: Arc::new(RwLock::new(None)),
            extended_network_follows: Arc::new(RwLock::new(Vec::new())),
        }
```

- [ ] **Step 6: Verify compilation**

Run: `cargo check -p arcadestr-desktop`
Expected: No errors

- [ ] **Step 7: Commit**

```bash
git add desktop/src/main.rs
git commit -m "feat: wire NIP-05 validator in desktop main"
```

---

## Task 5: Fix Notification Loop Client Access

**Files:**
- Modify: `desktop/src/main.rs:1914-1935` (notification loop spawning)

**Purpose:** Use the actual Client from RelayManager instead of deprecated inner_clone().

- [ ] **Step 1: Modify notification loop spawning**

Replace the notification loop spawning code (around line 1914):

```rust
            // Spawn notification loop for real-time events
            let nostr_client_clone = nostr.clone();
            let relay_cache_clone = relay_cache.clone();
            let registry_clone = subscription_registry.clone();
            let hints_for_loop = relay_hints.clone();
            
            tauri::async_runtime::spawn(async move {
                // Get the client directly from RelayManager
                let client = nostr_client_clone.lock().await;
                let manager = client.relay_manager().lock().await;
                let inner_client = Arc::new(manager.get_client().clone());
                drop(manager);
                drop(client);

                run_notification_loop(
                    inner_client,
                    relay_cache_clone,
                    registry_clone,
                    hints_for_loop,
                    Box::new(move |event| {
                        // Emit event to frontend
                        let _ = app_handle.emit("nostr_event", event);
                    }),
                ).await;
            });
```

- [ ] **Step 2: Remove deprecated warning log**

The warning at line 1933 about "RelayManager migration in progress" can now be removed since we're properly accessing the Client.

- [ ] **Step 3: Verify compilation**

Run: `cargo check -p arcadestr-desktop`
Expected: No errors

- [ ] **Step 4: Commit**

```bash
git add desktop/src/main.rs
git commit -m "fix: use RelayManager.get_client() for notification loop"
```

---

## Task 6: Activate Ephemeral Subscriptions

**Files:**
- Modify: `desktop/src/main.rs:1045-1058` (relay gossip initialization)
- Modify: `core/src/subscriptions.rs:224-261` (add batch dispatch)

**Purpose:** Actually use the ephemeral subscription system for uncovered pubkeys.

- [ ] **Step 1: Add batch ephemeral subscription dispatch**

Add a new function to `core/src/subscriptions.rs` after `dispatch_ephemeral_read()`:

```rust
/// Dispatch ephemeral read connections for multiple uncovered pubkeys (batch)
pub async fn dispatch_ephemeral_reads_batch(
    client: &Client,
    pubkeys: &[String],
    relay_cache: &Arc<RelayCache>,
    registry: &Arc<SubscriptionRegistry>,
) {
    for pubkey in pubkeys {
        // Get best relay for this pubkey
        let relay_url = if let Some(cached) = relay_cache.get_relay_list(pubkey) {
            cached.read_relays.first()
                .or(cached.write_relays.first())
                .cloned()
        } else {
            None
        };
        
        if let Some(url) = relay_url {
            dispatch_ephemeral_read(client, pubkey, &url, registry).await;
        } else {
            tracing::warn!("No relay found for ephemeral read: {}", pubkey);
        }
    }
}
```

- [ ] **Step 2: Export the new function**

Add to `core/src/lib.rs` exports:

```rust
pub use subscriptions::{
    dispatch_ephemeral_read,
    dispatch_ephemeral_reads_batch,  // Add this
    dispatch_permanent_subscriptions,
    // ... rest
};
```

- [ ] **Step 3: Activate ephemeral subscriptions in main**

Modify the relay gossip initialization in `desktop/src/main.rs` (around line 1055):

```rust
        // Activate ephemeral subscriptions for uncovered pubkeys
        if !selection.uncovered_pubkeys.is_empty() {
            let client = nostr.lock().await;
            let manager = client.relay_manager().lock().await;
            let inner_client = manager.get_client();
            
            dispatch_ephemeral_reads_batch(
                inner_client,
                &selection.uncovered_pubkeys,
                &relay_cache,
                &subscription_registry,
            ).await;
            
            info!("Activated ephemeral subscriptions for {} uncovered pubkeys", 
                  selection.uncovered_pubkeys.len());
        }
```

- [ ] **Step 4: Add import for batch function**

Add to imports in `desktop/src/main.rs`:

```rust
use arcadestr_core::{
    dispatch_ephemeral_read,
    dispatch_ephemeral_reads_batch,  // Add this
    dispatch_permanent_subscriptions,
    // ... rest
};
```

- [ ] **Step 5: Verify compilation**

Run: `cargo check -p arcadestr-desktop`
Expected: No errors

- [ ] **Step 6: Commit**

```bash
git add core/src/subscriptions.rs core/src/lib.rs desktop/src/main.rs
git commit -m "feat: activate ephemeral subscriptions for uncovered pubkeys"
```

---

## Task 7: Add Subscription Cleanup Helpers

**Files:**
- Modify: `core/src/subscriptions.rs:47-53` (add cleanup method)
- Modify: `core/src/subscriptions.rs:94-168` (add cleanup in loop)

**Purpose:** Provide explicit cleanup mechanism for subscriptions when views unmount.

- [ ] **Step 1: Add subscription cleanup method to Registry**

Add to `SubscriptionRegistry` impl (after `remove()` around line 53):

```rust
    /// Remove multiple subscriptions at once (for bulk cleanup)
    pub fn remove_many(&self, ids: &[String]) {
        if let Ok(mut entries) = self.entries.lock() {
            for id in ids {
                entries.remove(id);
            }
        }
    }
    
    /// Get all subscription IDs of a specific connection kind
    pub fn get_by_kind(&self, kind: ConnectionKind) -> Vec<String> {
        self.entries.lock()
            .ok()
            .map(|entries| {
                entries.iter()
                    .filter(|(_, k)| **k == kind)
                    .map(|(id, _)| id.clone())
                    .collect()
            })
            .unwrap_or_default()
    }
    
    /// Clear all ephemeral subscriptions (for view cleanup)
    pub fn clear_ephemeral(&self) -> Vec<String> {
        let to_remove: Vec<String> = self.entries.lock()
            .ok()
            .map(|entries| {
                entries.iter()
                    .filter(|(_, kind)| {
                        matches!(kind, ConnectionKind::EphemeralRead | ConnectionKind::EphemeralWrite)
                    })
                    .map(|(id, _)| id.clone())
                    .collect()
            })
            .unwrap_or_default();
        
        self.remove_many(&to_remove);
        to_remove
    }
```

- [ ] **Step 2: Add cleanup function for explicit UNREQ**

Add a new function to `core/src/subscriptions.rs`:

```rust
/// Close subscriptions by ID and send UNREQ to relays
pub async fn close_subscriptions(
    client: &Client,
    registry: &Arc<SubscriptionRegistry>,
    subscription_ids: Vec<String>,
) {
    for id in &subscription_ids {
        // Send UNREQ to all relays
        let sub_id = SubscriptionId::new(id);
        if let Err(e) = client.unsubscribe(&sub_id).await {
            tracing::warn!("Failed to send UNREQ for {}: {}", id, e);
        }
        
        // Remove from registry
        registry.remove(id);
        tracing::debug!("Closed subscription: {}", id);
    }
}

/// Cleanup all ephemeral subscriptions for a view/component
pub async fn cleanup_view_subscriptions(
    client: &Client,
    registry: &Arc<SubscriptionRegistry>,
    view_id: &str,
) {
    // Find subscriptions tagged with this view ID
    let to_close: Vec<String> = registry.entries.lock()
        .ok()
        .map(|entries| {
            entries.iter()
                .filter(|(id, _)| id.starts_with(&format!("{}_", view_id)))
                .map(|(id, _)| id.clone())
                .collect()
        })
        .unwrap_or_default();
    
    if !to_close.is_empty() {
        close_subscriptions(client, registry, to_close).await;
        tracing::info!("Cleaned up subscriptions for view: {}", view_id);
    }
}
```

- [ ] **Step 3: Export cleanup functions**

Add to `core/src/lib.rs`:

```rust
pub use subscriptions::{
    close_subscriptions,
    cleanup_view_subscriptions,
    // ... existing exports
};
```

- [ ] **Step 4: Verify compilation**

Run: `cargo check -p arcadestr-core`
Expected: No errors

- [ ] **Step 5: Commit**

```bash
git add core/src/subscriptions.rs core/src/lib.rs
git commit -m "feat: add subscription cleanup helpers for view lifecycle"
```

---

## Task 8: Verify End-to-End Profile Flow

**Files:**
- Test: Run full application and verify profile fetching
- Monitor: Check logs for batch fetching, NIP-05 validation, subscription activity

**Purpose:** Ensure all components work together correctly.

- [ ] **Step 1: Build and run the application**

Run: `cd /home/joel/Sync/Projetos/Arcadestr/desktop && timeout 60 cargo tauri dev 2>&1`
Expected: Application starts without errors

- [ ] **Step 2: Check logs for ProfileFetcher activity**

Look for log messages:
- "ProfileFetcher initialized with persistent cache and NIP-05 validator"
- "Fetching batch of X profiles"
- "Queued NIP-05 validation for"

- [ ] **Step 3: Check logs for NIP-05 validation**

Look for:
- "NIP-05 background validator spawned"
- "NIP-05 verified for"
- "NIP-05 verification failed for" (some failures expected)

- [ ] **Step 4: Check logs for subscription activity**

Look for:
- "Activated ephemeral subscriptions for X uncovered pubkeys"
- "Started ephemeral read for"
- "Subscribed to permanent relay"

- [ ] **Step 5: Verify UI receives profiles**

Check browser console (F12) for:
- "profile_fetched" events
- Profile data with names/pictures

- [ ] **Step 6: Commit verification results**

```bash
git log --oneline -10  # Show recent commits
git status  # Verify clean state
echo "Integration verified successfully" > /tmp/verification.log
git add -A
git commit -m "chore: verify end-to-end integration" || echo "Nothing to commit"
```

---

## Summary

This plan wires together Arcadestr's existing Nostr infrastructure:

1. **NIP-05 Background Validation** - New worker validates identifiers async without blocking
2. **Fixed Notification Loop** - Uses RelayManager.get_client() instead of deprecated path
3. **Active Ephemeral Subscriptions** - Actually dispatches subscriptions for uncovered pubkeys
4. **Subscription Cleanup** - Helpers for view lifecycle management
5. **Verified End-to-End** - Profile batching → cache → background verify → UI update

The result matches Amethyst/Wisp efficiency patterns while maintaining Rust's safety guarantees.
