# Dynamic Relay Connection Reporting - Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement real-time, dynamic relay connection reporting so relays appear in the UI immediately as they connect, rather than waiting for all connections to complete.

**Architecture:** Convert from polling-based updates (5s intervals) to event-driven architecture using Tauri's event system. Each relay connection will emit an event that the frontend immediately captures to update the UI. Replace batch connection waits with individual connection tracking.

**Tech Stack:** Rust (Tauri backend), Leptos (frontend), nostr-sdk, tokio

---

## Problem Analysis

**Current Behavior:**
- UI polls relay count every 5 seconds via `invoke_get_relay_count()`
- Backend adds all relays to pool, then calls `connect().await` which waits for batch completion
- Multiple fixed 500ms delays throughout connection flow
- `get_connected_relays()` returns empty `vec![]` (unimplemented)
- User sees no relays until all connections complete or timeout (up to 5+ seconds)

**Desired Behavior:**
- Each relay appears in UI within milliseconds of connecting
- No fixed delays - progress as soon as connections establish
- Real-time event stream from backend to frontend
- Progressive loading experience

---

## File Structure

### Files to Modify:

1. **`core/src/relay_manager.rs`** - Add event emission on connection status changes
2. **`core/src/nostr.rs`** - Add event channel and emit relay events
3. **`desktop/src/main.rs`** - Wire events from nostr client to Tauri, implement `get_connected_relays()`
4. **`app/src/lib.rs`** - Replace polling with event listeners, update UI in real-time
5. **`desktop/src/lib.rs`** - Export new types for TypeScript bindings

### New Types:

- `RelayConnectionEvent` - Enum for connect/disconnect events
- `RelayStatus` - Struct with URL + connected state

---

## Phase 1: Backend Event Infrastructure

### Task 1: Add Event Types to Core

**Files:**
- Create: `core/src/relay_events.rs`
- Modify: `core/src/lib.rs` (add module)

- [ ] **Step 1: Create event types file**

```rust
// core/src/relay_events.rs

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RelayConnectionEvent {
    Connected { url: String },
    Disconnected { url: String, reason: Option<String> },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RelayStatus {
    pub url: String,
    pub connected: bool,
    pub latency_ms: Option<u64>,
}

impl RelayConnectionEvent {
    pub fn connected(url: impl Into<String>) -> Self {
        Self::Connected { url: url.into() }
    }
    
    pub fn disconnected(url: impl Into<String>, reason: Option<String>) -> Self {
        Self::Disconnected { url: url.into(), reason }
    }
}
```

- [ ] **Step 2: Add module to core lib**

```rust
// core/src/lib.rs - add to existing module declarations

pub mod relay_events;
```

- [ ] **Step 3: Commit**

```bash
git add core/src/relay_events.rs core/src/lib.rs
git commit -m "feat: add relay connection event types"
```

---

### Task 2: Add Event Channel to NostrClient

**Files:**
- Modify: `core/src/nostr.rs`

- [ ] **Step 1: Add tokio broadcast channel to imports**

Add to existing imports at top of file:
```rust
use tokio::sync::broadcast;
```

- [ ] **Step 2: Add event sender field to NostrClient**

Find the `NostrClient` struct and add field:
```rust
pub struct NostrClient {
    // ... existing fields ...
    relay_event_sender: broadcast::Sender<RelayConnectionEvent>,
}
```

- [ ] **Step 3: Initialize channel in constructor**

In `NostrClient::new()`, add:
```rust
let (relay_event_sender, _) = broadcast::channel(100);
```

Add to struct initialization:
```rust
Ok(Self {
    // ... existing fields ...
    relay_event_sender,
})
```

- [ ] **Step 4: Add subscribe method**

```rust
impl NostrClient {
    /// Subscribe to relay connection events
    pub fn subscribe_relay_events(&self) -> broadcast::Receiver<RelayConnectionEvent> {
        self.relay_event_sender.subscribe()
    }
    
    /// Emit a relay connection event
    fn emit_relay_event(&self, event: RelayConnectionEvent) {
        let _ = self.relay_event_sender.send(event);
    }
}
```

- [ ] **Step 5: Commit**

```bash
git add core/src/nostr.rs
git commit -m "feat: add relay event channel to NostrClient"
```

---

### Task 3: Emit Events from RelayManager

**Files:**
- Modify: `core/src/relay_manager.rs`
- Modify: `core/src/nostr.rs`

- [ ] **Step 1: Modify NostrClient to pass event sender to RelayManager**

Find where `RelayManager::new()` is called in `nostr.rs`, change to:
```rust
let relay_manager = RelayManager::new(
    client.clone(),
    relay_pool.clone(),
    config.relay_manager_config.clone(),
    Some(self.relay_event_sender.clone()), // Add this parameter
)?;
```

- [ ] **Step 2: Add event sender field to RelayManager**

In `core/src/relay_manager.rs`, add field to struct:
```rust
pub struct RelayManager {
    // ... existing fields ...
    event_sender: Option<broadcast::Sender<RelayConnectionEvent>>,
}
```

- [ ] **Step 3: Update RelayManager constructor**

Change `RelayManager::new()` signature:
```rust
pub fn new(
    client: Client,
    pool: Arc<RelayPool>,
    config: RelayManagerConfig,
    event_sender: Option<broadcast::Sender<RelayConnectionEvent>>,
) -> Result<Self, RelayManagerError> {
    // ... existing validation ...
    
    Ok(Self {
        // ... existing fields ...
        event_sender,
    })
}
```

- [ ] **Step 4: Emit event when relay connects**

In `connect_all_relays()` method, after successful connection:
```rust
async fn connect_all_relays(&self) -> Result<(), RelayManagerError> {
    let relays = self.pool.get_relays().await;
    let total = relays.len();
    
    for relay in &relays {
        match self.client.add_relay(relay).await {
            Ok(_) => {
                // Emit event for each successfully added relay
                if let Some(sender) = &self.event_sender {
                    let _ = sender.send(RelayConnectionEvent::connected(relay));
                }
                info!("Added relay: {}", relay);
            }
            Err(e) => {
                warn!("Failed to add relay {}: {}", relay, e);
                if let Some(sender) = &self.event_sender {
                    let _ = sender.send(RelayConnectionEvent::disconnected(
                        relay, 
                        Some(e.to_string())
                    ));
                }
            }
        }
    }
    
    self.client.connect().await;
    Ok(())
}
```

- [ ] **Step 5: Emit events for discovered relays**

In `add_discovered_relay()` method:
```rust
pub async fn add_discovered_relay(&self, url: String) -> Result<(), RelayManagerError> {
    // ... existing capacity check ...
    
    if self.pool.add_relay(url.clone(), RelaySource::Discovered).await {
        match self.client.add_relay(&url).await {
            Ok(_) => {
                self.client.relay(&url).await?.connect().await;
                
                // Emit event immediately
                if let Some(sender) = &self.event_sender {
                    let _ = sender.send(RelayConnectionEvent::connected(&url));
                }
                
                info!("Connected to discovered relay: {}", url);
            }
            Err(e) => {
                warn!("Failed to add discovered relay {}: {}", url, e);
                if let Some(sender) = &self.event_sender {
                    let _ = sender.send(RelayConnectionEvent::disconnected(
                        &url,
                        Some(e.to_string())
                    ));
                }
            }
        }
    }
    Ok(())
}
```

- [ ] **Step 6: Commit**

```bash
git add core/src/relay_manager.rs core/src/nostr.rs
git commit -m "feat: emit relay connection events from RelayManager"
```

---

### Task 4: Implement get_connected_relays() Command

**Files:**
- Modify: `desktop/src/main.rs`

- [ ] **Step 1: Find the existing get_connected_relays command**

Search for the TODO comment showing empty implementation.

- [ ] **Step 2: Implement proper relay status retrieval**

Replace the empty implementation:
```rust
#[tauri::command]
async fn get_connected_relays(state: tauri::State<'_, AppState>) -> Result<Vec<RelayStatus>, String> {
    let nostr = state.nostr.lock().await;
    let manager = nostr.relay_manager.lock().await;
    let client = manager.get_client();
    
    let mut statuses = Vec::new();
    let relays = client.relays().await;
    
    for (url, relay) in relays {
        statuses.push(RelayStatus {
            url: url.to_string(),
            connected: relay.is_connected(),
            latency_ms: relay.stats().latency().map(|d| d.as_millis() as u64),
        });
    }
    
    Ok(statuses)
}
```

Note: You'll need to add `RelayStatus` import from `core::relay_events`.

- [ ] **Step 3: Commit**

```bash
git add desktop/src/main.rs
git commit -m "feat: implement get_connected_relays() command"
```

---

## Phase 2: Desktop Event Wiring

### Task 5: Wire Events to Tauri

**Files:**
- Modify: `desktop/src/main.rs`

- [ ] **Step 1: Add event listener spawning**

Find where `AppState` is created, add event listener setup:
```rust
// Spawn relay event listener
let nostr_for_events = app_state.nostr.clone();
let app_handle_for_events = app_handle.clone();

tauri::async_runtime::spawn(async move {
    let nostr = nostr_for_events.lock().await;
    let mut rx = nostr.subscribe_relay_events();
    drop(nostr); // Release lock
    
    while let Ok(event) = rx.recv().await {
        let payload = match &event {
            RelayConnectionEvent::Connected { url } => {
                serde_json::json!({
                    "type": "connected",
                    "url": url
                })
            }
            RelayConnectionEvent::Disconnected { url, reason } => {
                serde_json::json!({
                    "type": "disconnected",
                    "url": url,
                    "reason": reason
                })
            }
        };
        
        let _ = app_handle_for_events.emit_all("relay-connection", payload);
    }
});
```

- [ ] **Step 2: Add required imports**

```rust
use arcadestr_core::relay_events::RelayConnectionEvent;
```

- [ ] **Step 3: Commit**

```bash
git add desktop/src/main.rs
git commit -m "feat: wire relay events to Tauri frontend"
```

---

## Phase 3: Frontend Real-Time Updates

### Task 6: Add Event Listener to Frontend

**Files:**
- Modify: `app/src/lib.rs`

- [ ] **Step 1: Add Tauri event listener imports**

At the top of the file, check for existing imports. Add if not present:
```rust
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::spawn_local;
use leptos::*;
```

- [ ] **Step 2: Find the relay count polling code**

Search for the `Effect::new` that polls every 5 seconds (around line 4341).

- [ ] **Step 3: Replace polling with event-driven updates**

Replace the polling Effect with event listener:
```rust
// Listen for relay connection events from backend
spawn_local(async move {
    let window = web_sys::window().expect("no window");
    let tauri = js_sys::Reflect::get(&window, &"__TAURI__".into()).unwrap();
    let event_api = js_sys::Reflect::get(&tauri, &"event".into()).unwrap();
    
    // Create callback for relay events
    let closure = Closure::wrap(Box::new(move |event: JsValue| {
        let payload = js_sys::Reflect::get(&event, &"payload".into()).unwrap();
        let event_type = js_sys::Reflect::get(&payload, &"type".into())
            .unwrap()
            .as_string()
            .unwrap_or_default();
        let url = js_sys::Reflect::get(&payload, &"url".into())
            .unwrap()
            .as_string()
            .unwrap_or_default();
        
        match event_type.as_str() {
            "connected" => {
                // Update connected relays list
                connected_relays_local.update(|relays| {
                    if !relays.contains(&url) {
                        relays.push(url);
                    }
                });
                relay_count_local.set(connected_relays_local.get().len());
            }
            "disconnected" => {
                // Remove from connected list
                connected_relays_local.update(|relays| {
                    relays.retain(|r| r != &url);
                });
                relay_count_local.set(connected_relays_local.get().len());
            }
            _ => {}
        }
    }) as Box<dyn FnMut(JsValue)>);
    
    let listen_fn = js_sys::Reflect::get(&event_api, &"listen".into()).unwrap();
    let _ = listen_fn.call2(
        &event_api,
        &"relay-connection".into(),
        &closure.as_ref().into()
    );
    
    closure.forget(); // Keep closure alive
});

// Also do initial fetch of connected relays
spawn_local(async move {
    match invoke_get_connected_relays().await {
        Ok(relays) => {
            let count = relays.len();
            connected_relays_local.set(relays.into_iter().map(|r| r.url).collect());
            relay_count_local.set(count);
        }
        Err(e) => {
            warn!("Failed to get initial relay list: {}", e);
        }
    }
});
```

Note: You may need to adjust this based on the exact frontend patterns used.

- [ ] **Step 4: Commit**

```bash
git add app/src/lib.rs
git commit -m "feat: replace relay polling with event-driven updates"
```

---

### Task 7: Update Relay UI to Show Real-Time List

**Files:**
- Modify: `app/src/lib.rs` (relay dropdown/badge UI)

- [ ] **Step 1: Find relay dropdown rendering code**

Search for the dropdown that shows connected relays (around lines 4377-4403).

- [ ] **Step 2: Update to show live relay list with status**

If not already showing detailed status, update the dropdown:
```rust
// In the dropdown rendering
view! {
    <div class="relay-dropdown">
        <For
            each=connected_relays_local
            key=|url| url.clone()
            children=move |url| {
                view! {
                    <div class="relay-item">
                        <span class="relay-status-dot connected"></span>
                        <span class="relay-url">{url}</span>
                    </div>
                }
            }
        />
    </div>
}
```

- [ ] **Step 3: Add CSS for relay status**

Add styles if not present:
```css
.relay-status-dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    display: inline-block;
    margin-right: 8px;
}

.relay-status-dot.connected {
    background: #22c55e;
}

.relay-status-dot.disconnected {
    background: #ef4444;
}
```

- [ ] **Step 4: Commit**

```bash
git add app/src/lib.rs
git commit -m "feat: update relay UI for real-time display"
```

---

## Phase 4: Remove Fixed Delays

### Task 8: Eliminate 500ms Connection Delays

**Files:**
- Modify: `core/src/nostr.rs`

- [ ] **Step 1: Find all sleep(Duration::from_millis(500)) calls**

Search for these patterns in nostr.rs.

- [ ] **Step 2: Replace with event-driven waiting**

Instead of:
```rust
tokio::time::sleep(Duration::from_millis(500)).await;
```

Use condition-based waiting or remove entirely:
```rust
// Wait for at least one relay to connect, with short timeout
let timeout = Duration::from_millis(200);
match tokio::time::timeout(timeout, self.wait_for_at_least_one_connection()).await {
    Ok(_) => info!("At least one relay connected, proceeding"),
    Err(_) => info!("No relays connected yet, proceeding anyway"),
}
```

Or simply remove if the event-driven system makes it unnecessary.

- [ ] **Step 3: Commit**

```bash
git add core/src/nostr.rs
git commit -m "refactor: remove fixed 500ms delays, use event-driven flow"
```

---

### Task 9: Optimize wait_for_connections()

**Files:**
- Modify: `core/src/relay_manager.rs`

- [ ] **Step 1: Reduce polling interval and timeout**

Current settings in `RelayManagerConfig`:
```rust
connection_poll_timeout_ms: 5000,  // Reduce to 2000
connection_poll_interval_ms: 100,  // Keep or reduce to 50
```

- [ ] **Step 2: Or remove entirely if event-driven**

If the event system is working well, consider removing `wait_for_connections()` entirely and letting operations proceed optimistically.

- [ ] **Step 3: Commit**

```bash
git add core/src/relay_manager.rs
git commit -m "perf: reduce connection wait timeouts for faster startup"
```

---

## Phase 5: Testing and Polish

### Task 10: Add Logging for Debugging

**Files:**
- Modify: `core/src/relay_manager.rs`
- Modify: `desktop/src/main.rs`

- [ ] **Step 1: Add detailed relay connection logging**

In `connect_all_relays()`:
```rust
info!("Starting relay connections: {} relays to connect", total);
// ... in loop
info!("Relay {} connection attempt: {:?}", relay, result);
```

In event listener in main.rs:
```rust
tracing::info!("Emitting relay event: {:?}", event);
```

- [ ] **Step 2: Commit**

```bash
git add core/src/relay_manager.rs desktop/src/main.rs
git commit -m "chore: add relay connection logging"
```

---

### Task 11: Run Tests and Verify

**Files:**
- All modified files

- [ ] **Step 1: Build and check core crate**

```bash
cargo check -p arcadestr-core
```

Expected: No errors

- [ ] **Step 2: Build and check desktop crate**

```bash
cargo check -p arcadestr-desktop
```

Expected: No errors

- [ ] **Step 3: Run tests**

```bash
cargo test -p arcadestr-core --lib -- --test-threads=1
```

Expected: All tests pass

- [ ] **Step 4: Format code**

```bash
cargo fmt
```

- [ ] **Step 5: Commit**

```bash
git commit -m "chore: format code"
```

---

## Summary

After completing these tasks:

1. **Real-time updates**: Relays appear in UI within milliseconds of connecting
2. **No polling**: Removed 5-second polling, replaced with event-driven architecture  
3. **Progressive loading**: Users see relays as they connect, not all at once
4. **Faster startup**: Removed fixed 500ms delays
5. **Better UX**: Visual feedback showing individual relay connection status

**Testing approach:**
- Build and verify no compilation errors
- Manual test: Start app, observe relays appearing in UI progressively
- Check browser console for event flow
- Verify disconnect events also update UI
