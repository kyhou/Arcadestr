# Relay Connection Indicator Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a UI indicator showing the number of currently connected relays near the user profile in the top left.

**Architecture:** Add Tauri command to get relay count from NostrClient, add frontend signal with polling, display in header next to user info.

**Tech Stack:** Rust (Tauri), Leptos (Rust UI), CSS

---

## Context

- Worktree: `.worktrees/nip65-relay-gossip`
- The user profile section is in `app/src/lib.rs` around line 2360-2397
- The backend NostrClient is in `desktop/src/main.rs` with state management

---

## Task 1: Add Tauri command to get relay count

**Files:**
- Modify: `desktop/src/main.rs`

- [ ] **Step 1: Add get_connected_relay_count command**

Add before tauri::Builder (around line 570):

```rust
/// Get the number of currently connected relays
#[tauri::command]
async fn get_connected_relay_count(
    state: tauri::State<'_, AppState>,
) -> Result<usize, String> {
    let nostr = state.nostr.lock().await;
    let relays = nostr.inner.relays().await;
    Ok(relays.len())
}
```

- [ ] **Step 2: Add to invoke_handler**

Find the `tauri::Builder::default().invoke_handler(tauri::generate_handler![...])` line (around line 559) and add `get_connected_relay_count` to the list:

```rust
.invoke_handler(tauri::generate_handler![
    wait_for_nostrconnect_signer,
    generate_nostrconnect_uri,
    connect_nip46,
    connect_with_key,
    reconnect_relays,
    get_public_key,
    is_authenticated,
    disconnect,
    publish_listing,
    fetch_listings,
    // ... other commands
    get_connected_relay_count,  // Add here
])
```

- [ ] **Step 3: Commit**

```bash
cd .worktrees/nip65-relay-gossip && git add desktop/src/main.rs && git commit -m "feat(ui): add get_connected_relay_count command"
```

---

## Task 2: Add frontend signal and polling

**Files:**
- Modify: `app/src/lib.rs`

- [ ] **Step 1: Add relay_count signal**

Find where signals are created (around line 40-60, look for `let profile = ...` or similar). Add:

```rust
let relay_count = Signal::new(0);
```

- [ ] **Step 2: Add invoke function**

Add after other invoke functions (around line 180-200):

```rust
/// Get the current number of connected relays
async fn invoke_get_relay_count() -> Result<usize, String> {
    invoke("get_connected_relay_count").await
}
```

- [ ] **Step 3: Add polling in on_mount**

Find the `on_mount` function (around line 2495). Inside the async block, add polling:

```rust
// Poll relay count every 5 seconds
let relay_count_clone = relay_count.clone();
let _ = window.set_interval(move || {
    let relay_count_local = relay_count_clone.clone();
    spawn_local(async move {
        match invoke_get_relay_count().await {
            Ok(count) => {
                relay_count_local.set(count);
            }
            Err(e) => {
                error!("Failed to get relay count: {}", e);
            }
        }
    });
});
```

- [ ] **Step 4: Commit**

```bash
cd .worktrees/nip65-relay-gossip && git add app/src/lib.rs && git commit -m "feat(ui): add relay count signal and polling"
```

---

## Task 3: Add UI indicator in header

**Files:**
- Modify: `app/src/lib.rs`

- [ ] **Step 1: Add indicator to user-info div**

Find the user-info div (around line 2364). Modify to include relay count:

```rust
view! {
    <div class="main-view">
        <header class="header">
            <h2 class="header-title">"Arcadestr"</h2>
            <div class="user-info">
                <button class="user-profile-btn" on:click={on_profile}>
                    // ... existing avatar and name ...
                </button>
                <span class="relay-count-indicator">
                    {move || {
                        format!("{} relays", relay_count.get())
                    }}
                </span>
                <button class="disconnect-button" on:click=on_disconnect>
                    "Disconnect"
                </button>
            </div>
        </header>
```

- [ ] **Step 2: Add CSS styling**

Find the CSS section (around line 850-900 where `.user-profile-btn` is defined). Add:

```css
.relay-count-indicator {
    font-size: 0.8em;
    color: #666;
    margin-left: 10px;
    align-self: center;
}
```

- [ ] **Step 3: Commit**

```bash
cd .worktrees/nip65-relay-gossip && git add app/src/lib.rs && git commit -m "feat(ui): display relay count indicator in header"
```

---

## Task 4: Verify build and test

**Files:**
- Test: Build both desktop and web

- [ ] **Step 1: Check desktop build**

```bash
cd .worktrees/nip65-relay-gossip && cargo check -p arcadestr-desktop
```

- [ ] **Step 2: Check web build**

```bash
cd .worktrees/nip65-relay-gossip && trunk check --target web
```

- [ ] **Step 3: Commit final**

```bash
cd .worktrees/nip65-relay-gossip && git add . && git commit -m "feat(ui): add relay connection indicator"
```

- [ ] **Step 4: Push**

```bash
cd .worktrees/nip65-relay-gossip && git push origin feature/nip65-relay-gossip
```

---

## Summary

| Task | Description | Files Changed |
|------|-------------|---------------|
| 1 | Tauri command for relay count | desktop/src/main.rs |
| 2 | Frontend signal + polling | app/src/lib.rs |
| 3 | UI indicator in header | app/src/lib.rs |
| 4 | Verify build | - |
