# Arcadestr Codebase Documentation

## Table of Contents

1. [Project Identity & Purpose](#1-project-identity--purpose)
2. [Technology Stack](#2-technology-stack)
3. [Repository Layout](#3-repository-layout--the-complete-map)
4. [Architecture & Data Flow](#4-architecture--data-flow)
5. [Tauri Commands & IPC Bridge](#5-tauri-commands--the-frontendbackend-bridge)
6. [Leptos Frontend Deep Dive](#6-leptos-frontend--deep-dive)
7. [Backend - Rust/Tauri Host Process](#7-backend--rust-tauri-host-process)
8. [Key Abstractions & Patterns](#8-key-abstractions--patterns)
9. [Build System & Configuration](#9-the-build-system--configuration)
10. [How to Add a New Feature](#10-how-to-add-a-new-feature--step-by-step-workflow)
11. [Debugging Guide](#11-debugging-guide)
12. [Glossary](#12-glossary)

---

## 1. Project Identity & Purpose

### What is Arcadestr?

Arcadestr is a **decentralized game marketplace** built on the NOSTR protocol with Lightning Network payments. It enables indie game developers to publish and sell their games directly to buyers without intermediaries, platform fees, or custodial payment systems.

### Who is it for?

- **Game Publishers**: Indie developers who want to sell games directly to players while maintaining full custody of their earnings
- **Game Buyers**: Players who want to purchase games using Bitcoin Lightning payments with instant, peer-to-peer transactions
- **Privacy-conscious users**: Those who prefer decentralized, censorship-resistant platforms over traditional app stores

### Main User-Facing Features

1. **Browse Listings**: View published games with metadata (title, description, price, tags)
2. **Publish Games**: Create parameterized replaceable events (kind 30078) containing game metadata
3. **Buy Games**: Generate Lightning invoices via LNURL-pay for paid games; direct download for free games
4. **User Profiles**: Display NOSTR profiles (NIP-01 kind-0 metadata) with NIP-05 verification
5. **Authentication**: Multiple signer support (NIP-07 browser extensions, NIP-46 remote signers)
6. **Zap Payments**: NIP-57 Lightning zaps for game purchases

### Desktop vs Web Target

Arcadestr is a **dual-target application**:

| Target | Technology | Authentication | Use Case |
|--------|-----------|----------------|----------|
| **Desktop** | Tauri v2 + Leptos | NIP-46 (remote signers like Nsec.app, Amber) | Native app experience with OS integration |
| **Web** | Leptos (WASM) | NIP-07 (browser extensions like Alby) | Browser-based access without installation |

Both targets share the same UI components from the `app` crate, but the desktop target has a Rust backend for NOSTR operations, while the web target relies on browser extensions for signing.

---

## 2. Technology Stack

| Name | Version | Role in Project | Where it Appears |
|------|---------|-----------------|------------------|
| **Rust** | 1.75+ | Primary language for all crates | All `.rs` files |
| **Tauri** | v2 | Desktop app shell, native API bridge | `desktop/` crate |
| **Leptos** | 0.8 | Reactive UI framework (CSR mode) | `app/` and `web/` crates |
| **nostr-sdk** | 0.44 | NOSTR protocol implementation | `core/src/nostr.rs` |
| **nostr** | 0.44 | Core NOSTR types and crypto | `core/Cargo.toml` |
| **tokio** | 1.40 | Async runtime (native only) | `core/`, `desktop/` |
| **serde** | 1.0 | Serialization for IPC and events | Throughout codebase |
| **sqlx** | 0.8 | Async SQLite for persistent storage | `core/src/storage/` |
| **rusqlite** | 0.32 | Synchronous SQLite (native) | `core/src/storage/db.rs` |
| **tauri-plugin-keyring** | 0.1 | OS keychain integration | `desktop/Cargo.toml` |
| **argon2** | 0.5 | Password hashing for encryption | `core/src/storage/encryption.rs` |
| **aes-gcm** | 0.10 | AES-256-GCM encryption | `core/src/storage/encryption.rs` |
| **reqwest** | 0.12 | HTTP client for NIP-05/NIP-57 | `core/src/nostr.rs`, `core/src/lightning.rs` |
| **wasm-bindgen** | 0.2 | WASM/JavaScript interop | `app/`, `web/` crates |
| **web-sys** | 0.3 | Browser API bindings | `app/src/web_auth.rs` |
| **qrcode** | 0.14 | QR code generation for NIP-46 | `app/Cargo.toml` |
| **trunk** | latest | WASM build tool and dev server | `web/Trunk.toml` |
| **tracing** | 0.1 | Structured logging | Throughout codebase |
| **thiserror** | 1.0 | Error type derivation | `core/` crate |

### Why Tauri over Pure Web or Pure Native?

The choice of Tauri v2 is driven by several factors evident in the codebase:

1. **Security**: Private keys never touch the application code. NIP-46 keeps keys in signer apps (Nsec.app, Amber), while Tauri provides a secure bridge.

2. **Native Performance**: The `core` crate uses native SQLite (via sqlx), async networking (tokio), and OS keychain integration—impossible in a pure browser environment.

3. **Single Codebase**: The `app` crate's Leptos components work in both Tauri (WebView) and browser (WASM) targets, maximizing code reuse.

4. **Small Bundle Size**: Tauri apps use the system's WebView, resulting in smaller binaries than Electron (~600KB vs ~150MB).

### Leptos Rendering Mode

Arcadestr uses **Client-Side Rendering (CSR)** exclusively:

```toml
# app/Cargo.toml
[features]
default = ["csr"]
csr = ["leptos/csr"]
hydrate = ["leptos/hydrate"]  # Available but not used
```

This means:
- The browser/Tauri WebView downloads a WASM bundle
- Leptos mounts the application and handles all rendering client-side
- No server-side rendering (SSR) is performed
- All state lives in the browser/WebView memory

---

## 3. Repository Layout — The Complete Map

```
arcadestr/
├── Cargo.toml              # Workspace manifest - defines all 4 crates
├── Cargo.lock              # Dependency lock file
├── README.md               # Project overview and quickstart
├── CLAUDE.md               # Development guidelines for Claude Code
├── AGENTS.md               # Agent-specific build/test commands
├── RUST_GUIDELINES.md      # Microsoft Rust Guidelines reference
├── NOSTRCONNECT_IMPLEMENTATION.md  # NIP-46 implementation notes
├── COMMANDS.md             # Available CLI commands
├── test_nostrconnect.sh    # Test script for NIP-46
│
├── core/                   # LIBRARY: Core business logic (NOSTR, storage, crypto)
│   ├── Cargo.toml          # Native-only dependencies (tokio, sqlx, etc.)
│   ├── src/
│   │   ├── lib.rs          # Module exports, feature-gated (native vs wasm)
│   │   ├── nostr.rs        # NOSTR client, event handling, relay management
│   │   ├── auth/           # Authentication state and account management
│   │   │   ├── mod.rs      # AuthState, signer switching
│   │   │   ├── auth_state.rs  # Core authentication logic
│   │   │   ├── account.rs      # Account data structures
│   │   │   └── account_manager.rs  # Multi-account support
│   │   ├── signers/        # Signer abstractions (local, NIP-46)
│   │   │   ├── mod.rs      # Signer trait definitions
│   │   │   ├── local.rs    # Local private key signer
│   │   │   ├── nip46.rs    # NIP-46 remote signer
│   │   │   └── lazy_nip46.rs   # Deferred connection NIP-46
│   │   ├── nip46/          # NIP-46 implementation (native-only)
│   │   │   ├── mod.rs      # Session management, QR flows
│   │   │   ├── auth.rs     # Authentication flows
│   │   │   ├── methods.rs  # NIP-46 method handlers
│   │   │   ├── session.rs  # Session state
│   │   │   ├── storage.rs  # Profile persistence (keyring)
│   │   │   └── types.rs    # NIP-46 data structures
│   │   ├── storage/        # Persistent storage layer
│   │   │   ├── mod.rs      # Storage exports
│   │   │   ├── db.rs       # SQLite database (sqlx)
│   │   │   ├── encryption.rs   # AES-256-GCM encryption
│   │   │   ├── master_key.rs   # Master key derivation
│   │   │   ├── migration.rs    # Database migrations
│   │   │   └── backup.rs       # Backup/restore functionality
│   │   ├── relay_cache.rs    # NIP-65 relay list caching
│   │   ├── relay_hints.rs    # Relay discovery from p-tags
│   │   ├── profile_fetcher.rs # Batched profile fetching
│   │   ├── user_cache.rs      # Persistent user profile cache
│   │   ├── social_graph.rs   # Extended network discovery
│   │   ├── extended_network.rs # 2nd-degree follow discovery
│   │   ├── subscriptions.rs  # Relay subscription management
│   │   ├── lightning.rs      # NIP-57 zap payments
│   │   ├── saved_users.rs    # Legacy saved users (JSON file)
│   │   ├── version.rs        # Version constants
│   │   └── wasm_stub.rs      # WASM-compatible stubs
│   └── migrations/           # SQLx database migrations
│
├── app/                    # LIBRARY: Leptos UI components (shared)
│   ├── Cargo.toml          # Leptos, wasm-bindgen dependencies
│   └── src/
│       ├── lib.rs          # Main app component, auth context, styles
│       ├── models.rs       # GameListing, UserProfile, ZapRequest types
│       ├── tauri_bridge.rs # Tauri command wrappers (invoke_*)
│       ├── tauri_invoke.rs # Low-level Tauri IPC (wasm-bindgen)
│       ├── web_auth.rs     # NIP-07 browser extension auth (web target)
│       ├── components/     # UI components
│       │   ├── mod.rs      # Component exports
│       │   ├── account_selector.rs   # Login/account switching UI
│       │   ├── backup_manager.rs     # Backup/restore UI
│       │   ├── browse.rs             # Game listing grid
│       │   ├── detail.rs             # Game detail view with buy flow
│       │   ├── profile.rs            # User profile view
│       │   ├── profile_avatar.rs     # Avatar component with fallback
│       │   ├── profile_display.rs    # Profile name/display components
│       │   └── publish.rs            # Game publishing form
│       └── store/          # Global state management
│           ├── mod.rs      # Store exports
│           └── profiles.rs # ProfileStore (reactive cache)
│
├── desktop/                # BINARY: Tauri v2 desktop application
│   ├── Cargo.toml          # Tauri v2, tauri-build dependencies
│   ├── tauri.conf.json     # Tauri configuration (window, security, build)
│   ├── build.rs            # Build script for Tauri
│   └── src/
│       ├── main.rs         # Entry point, Tauri setup, commands
│       └── nip46_commands.rs # NIP-46 specific Tauri commands
│
├── web/                    # BINARY: WASM web target (Trunk)
│   ├── Cargo.toml          # WASM-only dependencies
│   ├── Trunk.toml          # Trunk build configuration
│   ├── index.html          # HTML entry point
│   └── src/
│       ├── main.rs         # WASM entry point (mount_to_body)
│       └── lib.rs          # Web-specific setup
│
└── docs/                   # Documentation (if any)
```

---

## 4. Architecture & Data Flow

### 4.1 The Big Picture

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                           DESKTOP TARGET                                     │
│  ┌─────────────────────────────────────────────────────────────────────┐   │
│  │                     Tauri WebView (Leptos UI)                      │   │
│  │  ┌──────────────┐  ┌──────────────┐  ┌──────────────┐              │   │
│  │  │  BrowseView  │  │  DetailView  │  │ ProfileView  │              │   │
│  │  └──────────────┘  └──────────────┘  └──────────────┘              │   │
│  │         │                 │                 │                     │   │
│  │         └─────────────────┴─────────────────┘                     │   │
│  │                         │                                         │   │
│  │              ┌──────────▼──────────┐                              │   │
│  │              │   AuthContext       │                              │   │
│  │              │   (RwSignal)        │                              │   │
│  │              └──────────┬──────────┘                              │   │
│  │                         │                                         │   │
│  │              ┌──────────▼──────────┐                              │   │
│  │              │  tauri_invoke.rs    │                              │   │
│  │              │  (WASM→JS bridge)    │                              │   │
│  │              └──────────┬──────────┘                              │   │
│  └─────────────────────────┼───────────────────────────────────────────┘   │
│                           │ IPC (invoke/listen)                            │
│  ┌─────────────────────────┼───────────────────────────────────────────┐   │
│  │           Tauri Host Process (Rust)                                  │   │
│  │  ┌──────────────────────▼─────────────────────────────────────────┐  │   │
│  │  │                    AppState                                     │  │   │
│  │  │  ┌────────────┐ ┌────────────┐ ┌────────────┐ ┌────────────┐  │  │   │
│  │  │  │   auth     │ │   nostr    │ │ relay_cache│ │profile_fetch│  │  │   │
│  │  │  │Arc<Mutex<>>│ │Arc<Mutex<>>│ │  Arc<RwLock>>│ │    Arc<>    │  │  │   │
│  │  │  └────────────┘ └────────────┘ └────────────┘ └────────────┘  │  │   │
│  │  └────────────────────────────────────────────────────────────────│  │   │
│  │                           │                                        │  │   │
│  │  ┌────────────────────────▼────────────────────────────────────────┐  │   │
│  │  │              Tauri Commands (#[tauri::command])               │  │   │
│  │  │  • connect_bunker()    • publish_listing()   • fetch_profile()│  │   │
│  │  │  • start_qr_login()    • list_saved_profiles()• request_invoice│  │   │
│  │  └────────────────────────┬────────────────────────────────────────┘  │   │
│  │                           │                                           │   │
│  │  ┌────────────────────────▼────────────────────────────────────────┐  │   │
│  │  │                    core crate (arcadestr_core)                 │  │   │
│  │  │  ┌────────────┐ ┌────────────┐ ┌────────────┐ ┌────────────┐ │  │   │
│  │  │  │NostrClient │ │  nip46     │ │  storage   │ │ lightning  │ │  │   │
│  │  │  │(nostr-sdk) │ │(signer mgmt)│ │  (sqlx)    │ │(NIP-57)    │ │  │   │
│  │  │  └────────────┘ └────────────┘ └────────────┘ └────────────┘ │  │   │
│  │  └────────────────────────────────────────────────────────────────│  │   │
│  │                           │                                        │  │   │
│  │  ┌────────────────────────▼────────────────────────────────────────┐  │   │
│  │  │              External Services                                   │  │   │
│  │  │  • NOSTR Relays (wss://*)    • Lightning Network (LNURL-pay)    │  │   │
│  │  │  • NIP-46 Signer Apps        • NIP-05 Identity Providers        │  │   │
│  │  └──────────────────────────────────────────────────────────────────┘  │   │
│  └────────────────────────────────────────────────────────────────────────┘   │
└─────────────────────────────────────────────────────────────────────────────┘

┌─────────────────────────────────────────────────────────────────────────────┐
│                            WEB TARGET                                        │
│  ┌─────────────────────────────────────────────────────────────────────┐     │
│  │                     Browser (Leptos WASM)                          │     │
│  │  ┌──────────────┐  ┌──────────────┐  ┌──────────────┐              │     │
│  │  │  BrowseView  │  │  DetailView  │  │ ProfileView  │              │     │
│  │  └──────────────┘  └──────────────┘  └──────────────┘              │     │
│  │         │                 │                 │                     │     │
│  │         └─────────────────┴─────────────────┘                     │     │
│  │                         │                                         │     │
│  │              ┌──────────▼──────────┐                              │     │
│  │              │   web_auth.rs       │                              │     │
│  │              │   (NIP-07 bridge)   │                              │     │
│  │              └──────────┬──────────┘                              │     │
│  │                         │ window.nostr (browser extension)        │     │
│  └─────────────────────────┼─────────────────────────────────────────┘     │
│                           │                                                  │
│  ┌────────────────────────▼──────────────────────────────────────────┐     │
│  │              NOSTR Relays (direct WebSocket from browser)         │     │
│  └───────────────────────────────────────────────────────────────────┘     │
└─────────────────────────────────────────────────────────────────────────────┘
```

### 4.2 Request Lifecycle Example: Publishing a Game

Let's trace through what happens when a user clicks "Publish Listing":

**Step 1: UI Event (Leptos)**
```rust
// app/src/components/publish.rs
view! {
    <button on:click=move |_| {
        spawn_local(async move {
            let result = invoke_publish_listing(listing).await;
            // ...
        });
    }>"Publish"</button>
}
```

**Step 2: Tauri Bridge (WASM)**
```rust
// app/src/lib.rs (tauri_bridge)
pub async fn invoke_publish_listing(listing: GameListing) -> Result<String, String> {
    use crate::tauri_invoke::invoke;
    let publish_args = serde_json::json!({"listing": listing});
    invoke("publish_listing", publish_args).await
}
```

**Step 3: Low-level IPC (wasm-bindgen)**
```rust
// app/src/tauri_invoke.rs
pub async fn invoke<T: serde::de::DeserializeOwned>(
    command: &str,
    args: serde_json::Value,
) -> Result<T, String> {
    // Calls window.__TAURI__.core.invoke() via JS eval
    let js_code = format!(
        "window.__TAURI__.core.invoke('{}', {})",
        command, args.to_string()
    );
    let promise = js_sys::eval(&js_code)?;
    // ... convert Promise to Future
}
```

**Step 4: Tauri Command Handler (Rust)**
```rust
// desktop/src/main.rs
#[tauri::command]
async fn publish_listing(
    listing: GameListing,
    state: tauri::State<'_, AppState>,
) -> Result<String, String> {
    let auth_snapshot = {
        let auth = state.auth.lock().await;
        auth.clone()
    };
    let nostr = state.nostr.lock().await;
    nostr.publish_listing(&listing, &auth_snapshot)
        .await
        .map(|id| id.to_hex())
        .map_err(|e| e.to_string())
}
```

**Step 5: Core Business Logic**
```rust
// core/src/nostr.rs
pub async fn publish_listing(
    &self,
    listing: &GameListing,
    auth: &AuthState,
) -> Result<EventId, NostrError> {
    let signer = auth.signer().ok_or(NostrError::NotAuthenticated)?;
    let builder = game_listing_to_event_builder(listing);
    let signed_event = sign_event_with_arcadestr_signer(builder, signer).await?;
    self.inner.send_event(&signed_event).await?;
    Ok(signed_event.id)
}
```

**Step 6: NOSTR Network**
- Event is broadcast to all connected relays
- Relays validate and store the kind-30078 event
- Other clients can now fetch this listing

### 4.3 Data Flow Diagram

```
User Input
    │
    ▼
┌─────────────────┐
│  Leptos Component│ (reactive update)
│  (RwSignal)      │
└────────┬────────┘
         │
         ▼
┌─────────────────┐
│  AuthContext    │ (global state)
│  (RwSignal)     │
└────────┬────────┘
         │
         ▼
┌─────────────────┐
│  Tauri Invoke   │ (WASM→JS)
│  (tauri_invoke) │
└────────┬────────┘
         │ IPC
         ▼
┌─────────────────┐
│  Tauri Command  │ (Rust)
│  (#[command])   │
└────────┬────────┘
         │
    ┌────┴────┐
    ▼         ▼
┌────────┐ ┌────────┐
│ AuthState│ │NostrClient│
│(Mutex)  │ │(Mutex)    │
└────┬────┘ └────┬────┘
     │           │
     ▼           ▼
┌─────────────────┐
│  nostr-sdk      │ (relay communication)
│  (Client)       │
└────────┬────────┘
         │ WebSocket
         ▼
┌─────────────────┐
│  NOSTR Relays    │
│  (wss://*)       │
└─────────────────┘
```

---

## 5. Tauri Commands & the Frontend↔Backend Bridge

### 5.1 How Tauri IPC Works

Tauri v2 uses a **command-based IPC system**:

1. **Backend**: Define commands with `#[tauri::command]`
2. **Frontend**: Call commands via `window.__TAURI__.core.invoke()`
3. **Events**: Backend can emit events; frontend listens via `window.__TAURI__.event.listen()`

**Command Registration** (in `desktop/src/main.rs`):
```rust
tauri::Builder::default()
    .invoke_handler(tauri::generate_handler![
        connect_bunker,
        start_qr_login,
        publish_listing,
        // ... more commands
    ])
```

**Frontend Invocation** (in `app/src/tauri_invoke.rs`):
```rust
fn tauri_invoke(command: &str, args: serde_json::Value) -> Result<js_sys::Promise, String> {
    let js_code = format!(
        "window.__TAURI__.core.invoke('{}', {})",
        command, args.to_string()
    );
    js_sys::eval(&js_code)
        .map(|v| v.unchecked_into::<js_sys::Promise>())
}
```

### 5.2 Command Inventory

| Command | File | Parameters | Return Type | What it does |
|---------|------|------------|-------------|--------------|
| `connect_bunker` | `nip46_commands.rs` | `identifier: String`, `display_name: String` | `serde_json::Value` | Connects to NIP-46 bunker URI or NIP-05 identifier |
| `start_qr_login` | `nip46_commands.rs` | None | `String` | Generates nostrconnect:// URI for QR login |
| `check_qr_connection` | `nip46_commands.rs` | None | `Option<serde_json::Value>` | Polls for QR connection completion |
| `list_saved_profiles` | `nip46_commands.rs` | None | `serde_json::Value` | Returns all saved profiles (deduplicated) |
| `switch_profile` | `nip46_commands.rs` | `profile_id: String` | `serde_json::Value` | Switches active profile |
| `delete_profile` | `nip46_commands.rs` | `profile_id: String` | `()` | Removes profile from keyring |
| `logout_nip46` | `nip46_commands.rs` | None | `()` | Logs out current session |
| `get_connection_status` | `nip46_commands.rs` | None | `serde_json::Value` | Returns connection state |
| `generate_nostrconnect_uri` | `main.rs` | `relay: String` | `String` | Creates nostrconnect:// URI |
| `connect_nip46` | `main.rs` | `uri: String`, `relay: String` | `String` | Connects via NIP-46 URI |
| `connect_with_key` | `main.rs` | `key: String` | `String` | Direct key auth (testing only) |
| `wait_for_nostrconnect_signer` | `main.rs` | `timeout_secs: u64` | `String` | Waits for signer connection |
| `get_public_key` | `main.rs` | None | `String` | Returns authenticated npub |
| `is_authenticated` | `main.rs` | None | `bool` | Checks auth status |
| `disconnect` | `main.rs` | None | `()` | Clears auth state |
| `publish_listing` | `main.rs` | `listing: GameListing` | `String` | Publishes kind-30078 event |
| `fetch_listings` | `main.rs` | `limit: usize` | `Vec<GameListing>` | Fetches recent listings |
| `fetch_listing_by_id` | `main.rs` | `publisher_npub: String`, `listing_id: String` | `GameListing` | Fetches specific listing |
| `fetch_profile` | `main.rs` | `npub: String`, `additional_relays: Option<Vec<String>>` | `UserProfile` | Fetches NIP-01 metadata |
| `request_invoice` | `main.rs` | `zap_request: ZapRequest` | `ZapInvoice` | Generates Lightning invoice |
| `get_saved_users` | `main.rs` | None | `String` | Returns saved users JSON |
| `add_saved_user` | `main.rs` | `method: String`, `relay: Option<String>`, `uri: Option<String>`, `private_key: Option<String>`, `npub: String` | `String` | Adds saved user |
| `remove_saved_user` | `main.rs` | `user_id: String` | `String` | Removes saved user |
| `connect_saved_user` | `main.rs` | `user_id: String` | `serde_json::Value` | Reconnects saved user |
| `get_connected_relay_count` | `main.rs` | None | `usize` | Returns relay count |
| `get_connected_relays` | `main.rs` | None | `Vec<String>` | Returns relay URLs |
| `get_extended_network_stats` | `main.rs` | None | `Option<NetworkStats>` | Returns extended network info |

### 5.3 Event System

Tauri events enable **push notifications** from backend to frontend:

**Backend Emission**:
```rust
// desktop/src/main.rs
app_handle.emit("auth_success", user_npub.clone());
app_handle.emit("profile_fetched", profile);
app_handle.emit("profile_fetch_progress", ProfileFetchProgress { completed, total });
```

**Frontend Listening**:
```rust
// app/src/lib.rs
pub fn setup_profile_event_handlers(profile_store: ProfileStore) {
    spawn_local(async move {
        let _ = crate::tauri_invoke::listen("profile_fetched", move |data| {
            if let Ok(profile) = serde_json::from_value::<UserProfile>(data.clone()) {
                profile_store.put(profile);
            }
        }).await;
    });
}
```

**Event Inventory**:

| Event | Emitted By | Payload | Purpose |
|-------|-----------|---------|---------|
| `auth_success` | `connect_bunker`, `switch_profile` | `String` (npub) | Authentication completed |
| `auth_logout` | `logout_nip46` | `()` | User logged out |
| `profile_fetched` | `initialize_relay_gossip` | `UserProfile` | New profile available |
| `profile_fetch_progress` | `initialize_relay_gossip` | `ProfileFetchProgress` | Batch fetch progress |
| `user_profile_loaded` | `initialize_relay_gossip` | `UserProfile` | Current user profile loaded |
| `extended_network_discovered` | `initialize_extended_network` | `NetworkStats` | Extended network ready |
| `bunker_reconnected` | `attempt_reconnect` | `serde_json::Value` | Manual reconnect success |
| `bunker-heartbeat` | `ping_bunker` | `serde_json::Value` | Connection health check |
| `qr-login-complete` | `check_qr_connection` | `String` (npub) | QR login finished |

### 5.4 Permissions & Capabilities

Tauri v2 uses a **capability-based security model**. The configuration is in `desktop/tauri.conf.json`:

```json
{
  "app": {
    "security": {
      "csp": null  // Content Security Policy (disabled for development)
    }
  }
}
```

**Note**: The current configuration has minimal security restrictions (`csp: null`). For production, you should:

1. Define a strict CSP
2. Use Tauri's capability files (`.json` in `capabilities/` directory)
3. Scope allowed domains and APIs

---

## 6. Leptos Frontend — Deep Dive

### 6.1 Leptos Rendering Mode

Arcadestr uses **Client-Side Rendering (CSR)**:

```rust
// web/src/main.rs
fn main() {
    console_error_panic_hook::set_once();
    mount_to_body(|| view! { <App/> });  // Mounts to <body>, renders entirely client-side
}
```

**Implications**:
- No HTML is pre-rendered on the server
- The WASM bundle contains all rendering logic
- Initial page load downloads the WASM, then renders
- All routing is handled client-side (no page refreshes)

### 6.2 Component Tree

```
App (app/src/lib.rs)
│
├── AuthContext (global state - provided at root)
│
├── AccountSelector (app/src/components/account_selector.rs)
│   ├── Login View (nostrconnect/bunker input)
│   ├── QR Login View (QR code display)
│   ├── Nsec Login View (direct key - testing only)
│   └── Account List View (switch between saved accounts)
│
├── BrowseView (app/src/components/browse.rs)
│   ├── ListingCard (repeated for each game)
│   │   ├── ProfileAvatar (publisher)
│   │   └── Game metadata (title, price, tags)
│   └── Loading states
│
├── DetailView (app/src/components/detail.rs)
│   ├── Game details (title, description, screenshots)
│   ├── Publisher profile (ProfileDisplayName, ProfileAvatar)
│   ├── Buy flow (ZapRequest → Lightning invoice)
│   └── Download button
│
├── ProfileView (app/src/components/profile.rs)
│   ├── User metadata (name, picture, nip05)
│   └── Published games list
│
├── PublishView (app/src/components/publish.rs)
│   ├── Form fields (title, description, price, etc.)
│   └── Publish button
│
└── BackupManager (app/src/components/backup_manager.rs)
    ├── Create backup
    └── Restore backup
```

### 6.3 State Management & Reactivity

Arcadestr uses Leptos's **fine-grained reactive signals**:

#### Signal Types Used

| Type | Purpose | Example |
|------|---------|---------|
| `RwSignal<T>` | Read-write state (can be updated) | `npub: RwSignal<Option<String>>` |
| `Signal<T>` | Read-only derived state | `is_logged_in: Signal<bool>` |
| `Memo<T>` | Computed/cached derived value | `display_name: Memo<String>` |
| `Resource<T>` | Async data loading | `listings: Resource<Vec<GameListing>>` |
| `Action<T>` | Async mutations (form submissions) | `publish_action: Action<GameListing, Result<String, String>>` |

#### Global State: AuthContext

```rust
// app/src/lib.rs
#[derive(Clone)]
pub struct AuthContext {
    pub npub: RwSignal<Option<String>>,           // Current user's npub
    pub profile: RwSignal<Option<UserProfile>>,   // Current user's profile
    pub is_loading: RwSignal<bool>,               // Loading state
    pub error: RwSignal<Option<String>>,          // Error message
    pub accounts: RwSignal<Vec<StoredAccount>>,   // All saved accounts
    pub active_account: RwSignal<Option<StoredAccount>>, // Current account
    pub has_secure_accounts: RwSignal<bool>,    // Has encrypted storage
    pub connection_status: RwSignal<String>,      // NIP-46 connection state
    pub connection_error: RwSignal<Option<String>>, // Connection error
}
```

**Providing Context** (at app root):
```rust
// app/src/lib.rs
pub fn App() -> impl IntoView {
    let auth = AuthContext::new();
    provide_context(auth.clone());
    
    // ... rest of app
}
```

**Consuming Context** (in any component):
```rust
// In any component
let auth = use_context::<AuthContext>().expect("AuthContext not found");

// Read signal value
let npub = auth.npub.get();  // Returns Option<String>

// Write to signal
auth.npub.set(Some(new_npub));

// Create derived signal
let is_logged_in = Signal::derive(move || auth.npub.get().is_some());
```

#### ProfileStore: Reactive Cache

```rust
// app/src/store/profiles.rs
#[derive(Clone)]
pub struct ProfileStore {
    inner: RwSignal<HashMap<String, UserProfile>>,
}

impl ProfileStore {
    pub fn put(&self, profile: UserProfile) {
        self.inner.update(|map| {
            map.insert(profile.npub.clone(), profile);
        });
    }
    
    pub fn get(&self, npub: &str) -> Option<UserProfile> {
        self.inner.get().get(npub).cloned()
    }
    
    pub fn has(&self, npub: &str) -> bool {
        self.inner.get().contains_key(npub)
    }
}

// Provide at app root
pub fn provide_profile_store() {
    provide_context(ProfileStore::new());
}
```

### 6.4 Async Operations in the UI

#### Pattern 1: Direct async/await with spawn_local

```rust
// For fire-and-forget async operations
spawn_local(async move {
    match invoke_fetch_profile(npub).await {
        Ok(profile) => {
            auth.profile.set(Some(profile));
        }
        Err(e) => {
            auth.error.set(Some(e));
        }
    }
});
```

#### Pattern 2: Action for user-triggered operations

```rust
// For button-click async operations with loading states
let publish_action = Action::new(move |listing: &GameListing| {
    let listing = listing.clone();
    async move {
        invoke_publish_listing(listing).await
    }
});

// In view
view! {
    <button
        on:click=move |_| publish_action.dispatch(listing.clone())
        disabled=publish_action.pending()  // Auto-disabled while loading
    >
        {move || if publish_action.pending().get() {
            "Publishing..."
        } else {
            "Publish"
        }}
    </button>
    
    // Show error
    {move || publish_action.value().get().map(|result| match result {
        Ok(_) => view! { <span>"Published!"</span> },
        Err(e) => view! { <span class="error">{e}</span> },
    })}
}
```

#### Pattern 3: Resource for data fetching

```rust
// For data that should be fetched when dependencies change
let listings = Resource::new(
    || (),  // Dependency (refetch when this changes)
    |_| async move {
        invoke_fetch_listings(20).await.ok()
    }
);

// In view - handles loading, success, error states
view! {
    <Suspense fallback=|| view! { <p>"Loading..."</p> }>
        {move || listings.get().map(|listings| view! {
            <div class="grid">
                {listings.into_iter().map(|listing| view! {
                    <ListingCard listing=listing />
                }).collect_view()}
            </div>
        })}
    </Suspense>
}
```

---

## 7. Backend — Rust / Tauri Host Process

### 7.1 Entry Point

`desktop/src/main.rs` is the Tauri application entry point:

```rust
fn main() {
    // 1. Initialize logging
    tracing_subscriber::fmt::init();
    
    // 2. Set up data directories
    let keys_dir = dirs::data_local_dir().unwrap().join("arcadestr");
    arcadestr_core::signers::set_keys_dir(keys_dir.clone());
    arcadestr_core::saved_users::set_users_dir(keys_dir.clone());
    arcadestr_core::nip46::set_profile_cache_dir(keys_dir.clone());
    
    // 3. Initialize database
    let db_path = keys_dir.join("arcadestr.db");
    let database = tokio::runtime::Runtime::new().unwrap().block_on(async {
        arcadestr_core::storage::Database::new(&db_path).await
            .expect("Failed to initialize database")
    });
    
    // 4. Initialize UserCache
    let user_cache = Arc::new(UserCache::new(database.pool().clone()));
    
    // 5. Initialize NostrClient
    let nostr_client = /* ... */;
    
    // 6. Initialize RelayCache and RelayHints
    let relay_cache = RelayCache::new(keys_dir.join("relay_cache.db")).unwrap();
    let relay_hints = Arc::new(RelayHints::new(keys_dir.join("relay_hints.db")).unwrap());
    
    // 7. Build AppState
    let app_state = AppState {
        auth: Arc::new(Mutex::new(AuthState::new())),
        nostr: Arc::new(Mutex::new(nostr_client)),
        relay_cache: Arc::new(relay_cache),
        deduplicator: Arc::new(Mutex::new(EventDeduplicator::new(10000))),
        subscription_registry: Arc::new(SubscriptionRegistry::new()),
        profile_fetcher: Arc::new(ProfileFetcher::with_persistent_cache(user_cache.clone())),
        user_cache,
        extended_network: Arc::new(RwLock::new(None)),
        extended_network_follows: Arc::new(RwLock::new(Vec::new())),
        relay_hints: Some(relay_hints),
    };
    
    // 8. Build and run Tauri app
    tauri::Builder::default()
        .manage(app_state)
        .invoke_handler(tauri::generate_handler![/* commands */])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
```

### 7.2 Module Structure

| Module | Responsibility | Key Types | Key Functions |
|--------|---------------|-----------|---------------|
| `core::nostr` | NOSTR protocol | `NostrClient`, `GameListing`, `UserProfile` | `publish_listing()`, `fetch_profile()` |
| `core::auth` | Authentication | `AuthState`, `Account` | `connect_nip46()`, `signer()` |
| `core::signers` | Signer abstraction | `NostrSigner`, `Nip46Signer` | `sign_event()`, `generate_nostrconnect_uri()` |
| `core::nip46` | NIP-46 implementation | `AppSignerState`, `ProfileMetadata` | `init_signer_session()`, `save_profile_to_keyring()` |
| `core::storage` | Persistent storage | `Database`, `MasterKey` | `new()`, `encrypt()`, `decrypt()` |
| `core::relay_cache` | NIP-65 relay caching | `RelayCache`, `CachedRelayList` | `save_relay_list()`, `get_relay_list()` |
| `core::profile_fetcher` | Batched profile fetching | `ProfileFetcher`, `LruProfileCache` | `enqueue_many()`, `fetch_batch()` |
| `core::lightning` | Lightning payments | `ZapRequest`, `ZapInvoice` | `request_zap_invoice()` |
| `core::social_graph` | Extended network | `SocialGraphDb` | `discover_network()` |

### 7.3 Key Data Structures

#### GameListing
```rust
// core/src/nostr.rs
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GameListing {
    pub id: String,              // d-tag value (unique slug)
    pub title: String,
    pub description: String,
    pub price_sats: u64,         // 0 = free
    pub download_url: String,    // HTTPS download link
    pub publisher_npub: String,  // bech32 npub
    pub created_at: u64,         // Unix timestamp
    pub tags: Vec<String>,       // Categories
    pub lud16: String,           // Lightning address for payments
}
```

#### UserProfile
```rust
// core/src/nostr.rs
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct UserProfile {
    pub npub: String,
    pub name: Option<String>,
    pub display_name: Option<String>,
    pub about: Option<String>,
    pub picture: Option<String>,
    pub website: Option<String>,
    pub nip05: Option<String>,
    pub lud16: Option<String>,
    pub nip05_verified: bool,
}
```

#### AuthState
```rust
// core/src/auth/auth_state.rs
pub struct AuthState {
    signer: Option<Box<dyn NostrSigner>>,
    public_key: Option<PublicKey>,
    pending_nostrconnect: Option<PendingNostrConnect>,
}

pub struct PendingNostrConnect {
    client_keys: Keys,
    relay: String,
    secret: String,
}
```

#### AppState (Tauri managed state)
```rust
// desktop/src/main.rs
pub struct AppState {
    pub auth: Arc<Mutex<AuthState>>,
    pub nostr: Arc<Mutex<NostrClient>>,
    pub relay_cache: Arc<RelayCache>,
    pub deduplicator: Arc<Mutex<EventDeduplicator>>,
    pub subscription_registry: Arc<SubscriptionRegistry>,
    pub profile_fetcher: Arc<ProfileFetcher>,
    pub user_cache: Arc<UserCache>,
    pub extended_network: Arc<RwLock<Option<Arc<Mutex<ExtendedNetworkRepository>>>>>,
    pub extended_network_follows: Arc<RwLock<Vec<String>>>,
    pub relay_hints: Option<Arc<RelayHints>>,
}
```

### 7.4 State Management in Tauri

Tauri uses **managed state** for sharing data across commands:

```rust
// 1. Define state struct
pub struct AppState { /* ... */ }

// 2. Register with Tauri builder
let app_state = AppState { /* ... */ };
tauri::Builder::default()
    .manage(app_state)  // <-- Registers state
    .invoke_handler(tauri::generate_handler![/* commands */])
    .run(/* ... */);

// 3. Access in commands
#[tauri::command]
async fn my_command(
    state: tauri::State<'_, AppState>,  // <-- Extracts state
) -> Result<String, String> {
    let auth = state.auth.lock().await;  // <-- Use state
    // ...
}
```

**Thread Safety**: All shared state uses `Arc<Mutex<T>>` or `Arc<RwLock<T>>`:
- `Mutex` for exclusive access (writes)
- `RwLock` for multiple readers/single writer
- `Arc` for shared ownership across async tasks

### 7.5 Error Handling

The `core` crate uses **thiserror** for structured errors:

```rust
// core/src/nostr.rs
#[derive(Debug, Error)]
pub enum NostrError {
    #[error("Relay error: {0}")]
    RelayError(String),
    #[error("Malformed event: {0}")]
    MalformedEvent(String),
    #[error("Signing error: {0}")]
    SigningError(String),
    #[error("Not authenticated")]
    NotAuthenticated,
}
```

Commands convert errors to strings for the frontend:
```rust
#[tauri::command]
async fn fetch_profile(npub: String, state: State<'_, AppState>) -> Result<UserProfile, String> {
    let nostr = state.nostr.lock().await;
    nostr.fetch_profile(&npub, None)
        .await
        .map_err(|e| e.to_string())  // Convert NostrError to String
}
```

---

## 8. Key Abstractions & Patterns

### 8.1 Signer Abstraction

The `signers` module abstracts over different signing methods:

```rust
// core/src/signers/mod.rs
#[async_trait]
pub trait NostrSigner: Send + Sync {
    async fn sign_event(&self, event: Event) -> Result<Event, SignerError>;
    fn public_key(&self) -> Option<PublicKey>;
}

// Implementations:
// - LocalSigner: Direct private key (testing only)
// - Nip46Signer: Remote signer via NIP-46
// - Nip07Signer: Browser extension (web target)
```

This allows the same `AuthState` to work with any signer type.

### 8.2 Feature-Gated Compilation

The `core` crate uses Cargo features to support both native and WASM:

```rust
// core/src/lib.rs
#[cfg(feature = "native")]
pub mod auth;

#[cfg(feature = "native")]
pub mod storage;

#[cfg(feature = "wasm")]
pub mod wasm_stub;  // Stubs for WASM-incompatible modules
```

```toml
# core/Cargo.toml
[features]
default = []
native = []
wasm = []

[target.'cfg(not(target_arch = "wasm32"))'.dependencies]
tokio = { workspace = true }
sqlx = { version = "0.8", features = ["runtime-tokio", "sqlite"] }

[target.'cfg(target_arch = "wasm32")'.dependencies]
wasm-bindgen = "0.2"
```

### 8.3 Batched Profile Fetching

To avoid overwhelming relays, profile fetching is batched:

```rust
// core/src/profile_fetcher.rs
pub struct ProfileFetcher {
    queue: Arc<Mutex<VecDeque<String>>>,
    cache: Arc<LruProfileCache>,
}

impl ProfileFetcher {
    pub fn enqueue_many(&self, pubkeys: Vec<String>) {
        let mut queue = self.queue.lock().unwrap();
        for pubkey in pubkeys {
            if !self.cache.has(&pubkey) && !queue.contains(&pubkey) {
                queue.push_back(pubkey);
            }
        }
    }
    
    pub async fn fetch_batch(&self, nostr: &NostrClient) -> (Vec<UserProfile>, usize) {
        const BATCH_SIZE: usize = 10;
        let batch: Vec<String> = {
            let mut queue = self.queue.lock().unwrap();
            queue.drain(..BATCH_SIZE.min(queue.len())).collect()
        };
        // Fetch batch...
    }
}
```

### 8.4 Relay Gossip (NIP-65)

The relay selection algorithm implements **outbox model** discovery:

```rust
// core/src/nostr.rs
pub fn select_relays(
    scored: Vec<ScoredRelay>,
    max_relays: usize,
    all_pubkeys: &HashSet<String>,
) -> RelaySelection {
    let mut selected: Vec<String> = Vec::new();
    let mut covered: HashSet<String> = HashSet::new();
    let mut uncovered: HashSet<String> = all_pubkeys.clone();
    
    // Greedy set cover: pick relay covering most uncovered pubkeys
    for relay in scored {
        if selected.len() >= max_relays || uncovered.is_empty() {
            break;
        }
        
        let marginal: HashSet<_> = relay.pubkeys.intersection(&uncovered).cloned().collect();
        if !marginal.is_empty() {
            selected.push(relay.url);
            covered.extend(marginal.clone());
            uncovered.retain(|p| !marginal.contains(p));
        }
    }
    
    RelaySelection { permanent: selected, uncovered_pubkeys: uncovered.into_iter().collect() }
}
```

### 8.5 NIP-46 Fast Connection Flow

The NIP-46 implementation uses **deferred connection** for speed:

```rust
// core/src/nip46/mod.rs
pub async fn init_signer_session_fast(
    bunker_uri: NostrConnectURI,
    user_pubkey: PublicKey,
) -> Result<(ProfileMetadata, NostrConnect), Nip46Error> {
    // 1. Create client immediately (no blocking handshake)
    let client = NostrConnect::new(client_keys, bunker_uri, None, None)?;
    
    // 2. Return immediately with "connecting" state
    // 3. Background task completes handshake
    tokio::spawn(async move {
        match client.signer().await {
            Ok(signer) => { /* transition to connected */ }
            Err(e) => { /* transition to failed */ }
        }
    });
    
    Ok((profile, client))
}
```

---

## 9. The Build System & Configuration

### 9.1 How to Build

**Prerequisites**:
```bash
# Install Rust (if not already installed)
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# Install WASM target
rustup target add wasm32-unknown-unknown

# Install Trunk (for web builds)
cargo install trunk

# Install Tauri CLI v2
cargo install tauri-cli --version "^2"
```

**Desktop Development**:
```bash
# Run with hot reload (60s timeout to prevent hanging)
cd desktop && timeout 60 cargo tauri dev 2>&1

# Production build
cargo tauri build
```

**Web Development**:
```bash
cd web
trunk serve              # Development server at http://localhost:5173
trunk build --release    # Production build to web/dist/
```

**Testing**:
```bash
# Run all core tests
cargo test -p arcadestr-core

# Run specific test
cargo test -p arcadestr-core --lib test_insert_and_query

# Run with single thread (for SQLite tests)
cargo test -p arcadestr-core --lib -- --test-threads=1
```

**Linting & Formatting**:
```bash
# Format code
cargo fmt

# Check with clippy
cargo clippy -p arcadestr-core -- -D warnings

# Fix auto-fixable issues
cargo fix --lib -p arcadestr-core
```

### 9.2 Configuration Files

#### Workspace Cargo.toml
```toml
[workspace]
members = ["app", "desktop", "web", "core"]
resolver = "2"

[workspace.dependencies]
serde = { version = "1.0", features = ["derive"] }
tokio = { version = "1.40", features = ["full"] }
leptos = { version = "0.8", features = ["csr"] }
tauri = "2"
```

#### Desktop tauri.conf.json
```json
{
  "productName": "Arcadestr",
  "version": "0.1.0",
  "identifier": "com.arcadestr.app",
  "build": {
    "beforeDevCommand": "cd web && trunk build",
    "beforeBuildCommand": "cd web && trunk build --release",
    "frontendDist": "../web/dist"
  },
  "app": {
    "windows": [{ "title": "Arcadestr", "width": 1280, "height": 800 }],
    "security": { "csp": null }
  }
}
```

#### Web Trunk.toml
```toml
[serve]
port = 5173

[build]
target = "index.html"
dist = "dist"
public_url = "/"
```

### 9.3 Feature Flags

| Flag | Crate | Purpose |
|------|-------|---------|
| `native` | `core` | Enables native-only modules (tokio, sqlx, etc.) |
| `wasm` | `core` | Enables WASM stubs |
| `csr` | `app` | Client-side rendering mode |
| `hydrate` | `app` | Hydration mode (available but unused) |
| `web` | `app` | Web target with NIP-07 support |

---

## 10. How to Add a New Feature — Step-by-Step Workflow

Let's walk through adding a **"Favorite Listings"** feature that allows users to bookmark games.

### Step 1: Define the Data Model (core)

```rust
// core/src/storage/favorites.rs
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Favorite {
    pub id: String,           // ULID
    pub user_npub: String,    // Owner's npub
    pub listing_id: String,   // Game listing d-tag
    pub publisher_npub: String,
    pub created_at: i64,
}

pub struct FavoritesRepository {
    pool: SqlitePool,
}

impl FavoritesRepository {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
    
    pub async fn add(&self, favorite: &Favorite) -> Result<(), sqlx::Error> {
        sqlx::query(
            "INSERT INTO favorites (id, user_npub, listing_id, publisher_npub, created_at)
             VALUES (?, ?, ?, ?, ?)"
        )
        .bind(&favorite.id)
        .bind(&favorite.user_npub)
        .bind(&favorite.listing_id)
        .bind(&favorite.publisher_npub)
        .bind(favorite.created_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }
    
    pub async fn list_for_user(&self, user_npub: &str) -> Result<Vec<Favorite>, sqlx::Error> {
        sqlx::query_as::<_, Favorite>(
            "SELECT * FROM favorites WHERE user_npub = ? ORDER BY created_at DESC"
        )
        .bind(user_npub)
        .fetch_all(&self.pool)
        .await
    }
    
    pub async fn remove(&self, id: &str) -> Result<(), sqlx::Error> {
        sqlx::query("DELETE FROM favorites WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}
```

Add migration:
```sql
-- core/migrations/003_favorites.sql
CREATE TABLE favorites (
    id TEXT PRIMARY KEY,
    user_npub TEXT NOT NULL,
    listing_id TEXT NOT NULL,
    publisher_npub TEXT NOT NULL,
    created_at INTEGER NOT NULL
);

CREATE INDEX idx_favorites_user ON favorites(user_npub);
```

### Step 2: Add Tauri Commands (desktop)

```rust
// desktop/src/main.rs
#[tauri::command]
async fn add_favorite(
    listing_id: String,
    publisher_npub: String,
    state: tauri::State<'_, AppState>,
) -> Result<String, String> {
    let auth = state.auth.lock().await;
    let user_npub = auth.public_key()
        .ok_or("Not authenticated")?
        .to_bech32()
        .map_err(|e| e.to_string())?;
    
    let favorite = Favorite {
        id: ulid::Ulid::new().to_string(),
        user_npub,
        listing_id,
        publisher_npub,
        created_at: chrono::Utc::now().timestamp(),
    };
    
    let repo = FavoritesRepository::new(state.user_cache.pool().clone());
    repo.add(&favorite).await.map_err(|e| e.to_string())?;
    
    Ok(favorite.id)
}

#[tauri::command]
async fn list_favorites(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<Favorite>, String> {
    let auth = state.auth.lock().await;
    let user_npub = auth.public_key()
        .ok_or("Not authenticated")?
        .to_bech32()
        .map_err(|e| e.to_string())?;
    
    let repo = FavoritesRepository::new(state.user_cache.pool().clone());
    repo.list_for_user(&user_npub).await.map_err(|e| e.to_string())
}

// Register commands
tauri::Builder::default()
    .invoke_handler(tauri::generate_handler![
        // ... existing commands
        add_favorite,
        list_favorites,
    ])
```

### Step 3: Add Frontend Bridge (app)

```rust
// app/src/lib.rs
#[derive(Serialize)]
struct AddFavoriteArgs {
    listing_id: String,
    publisher_npub: String,
}

#[cfg(any(target_arch = "wasm32", not(feature = "web")))]
pub async fn invoke_add_favorite(
    listing_id: String,
    publisher_npub: String,
) -> Result<String, String> {
    use crate::tauri_invoke::invoke;
    
    let args = serde_json::json!({
        "listingId": listing_id,
        "publisherNpub": publisher_npub,
    });
    
    invoke("add_favorite", args).await
}

#[cfg(any(target_arch = "wasm32", not(feature = "web")))]
pub async fn invoke_list_favorites() -> Result<Vec<Favorite>, String> {
    use crate::tauri_invoke::invoke;
    invoke("list_favorites", serde_json::json!({})).await
}
```

### Step 4: Create UI Component (app)

```rust
// app/src/components/favorites.rs
use leptos::prelude::*;
use crate::{invoke_list_favorites, invoke_add_favorite, GameListing, invoke_fetch_listing_by_id};

#[component]
pub fn FavoritesView() -> impl IntoView {
    let favorites = Resource::new(
        || (),
        |_| async move {
            invoke_list_favorites().await.ok()
        }
    );
    
    view! {
        <div class="favorites-view">
            <h2>"Your Favorites"</h2>
            <Suspense fallback=|| view! { <p>"Loading..."</p> }>
                {move || favorites.get().map(|favs| view! {
                    <div class="favorites-grid">
                        {favs.into_iter().map(|fav| view! {
                            <FavoriteCard favorite=fav />
                        }).collect_view()}
                    </div>
                })}
            </Suspense>
        </div>
    }
}

#[component]
fn FavoriteCard(favorite: Favorite) -> impl IntoView {
    let listing = Resource::new(
        || (),
        move |_| {
            let fav = favorite.clone();
            async move {
                invoke_fetch_listing_by_id(fav.publisher_npub, fav.listing_id).await.ok()
            }
        }
    );
    
    view! {
        <div class="favorite-card">
            <Suspense fallback=|| view! { <p>"Loading..."</p> }>
                {move || listing.get().map(|l| view! {
                    <h3>{l.title}</h3>
                    <p>{l.description}</p>
                })}
            </Suspense>
        </div>
    }
}
```

### Step 5: Add to Main App

```rust
// app/src/lib.rs
pub mod components;
// ...
pub use components::{FavoritesView, /* ... */};

// In App component view
view! {
    <div class="arcadestr-app">
        <nav>/* ... */</nav>
        <main>
            {move || match current_view.get() {
                View::Browse => view! { <BrowseView /> },
                View::Favorites => view! { <FavoritesView /> },  // NEW
                // ...
            }}
        </main>
    </div>
}
```

### Step 6: Test

```bash
# Run core tests
cargo test -p arcadestr-core favorites

# Run desktop
cargo tauri dev

# Test the feature:
# 1. Log in
# 2. Browse to a game
# 3. Click "Add to Favorites"
# 4. Navigate to Favorites view
# 5. Verify game appears
```

---

## 11. Debugging Guide

### 11.1 Backend Debugging

**Adding Log Output**:
```rust
use tracing::{info, warn, error, debug};

#[tauri::command]
async fn my_command() {
    info!("Starting operation");
    debug!("Detailed value: {:?}", some_value);
    
    match result {
        Ok(_) => info!("Success"),
        Err(e) => error!("Failed: {}", e),
    }
}
```

**View Logs**:
```bash
# Desktop app logs to terminal
cargo tauri dev 2>&1 | grep -E "(INFO|ERROR|WARN)"

# Or use RUST_LOG environment variable
RUST_LOG=debug cargo tauri dev
```

**Attaching a Debugger**:
```bash
# Build in debug mode
cargo build -p arcadestr-desktop

# Run with debugger (VS Code or terminal)
rust-gdb target/debug/arcadestr-desktop
```

### 11.2 Frontend Debugging

**Open DevTools in Tauri**:
```rust
// desktop/src/main.rs
tauri::Builder::default()
    .setup(|app| {
        #[cfg(debug_assertions)]
        {
            let window = app.get_webview_window("main").unwrap();
            window.open_devtools();
        }
        Ok(())
    })
```

**Inspect Leptos Reactive State**:
```rust
// In component
let signal = RwSignal::new(0);

// Log when signal changes
create_effect(move |_| {
    web_sys::console::log_1(&format!("Signal value: {}", signal.get()).into());
});
```

**Common Leptos Pitfalls**:

1. **Signal updates not triggering**: Ensure you're calling `.get()` inside an effect or view
   ```rust
   // Wrong
   let value = signal;  // Just copies the signal handle
   
   // Right
   let value = signal.get();  // Reads current value, establishes dependency
   ```

2. **Resource not refetching**: Change the dependency
   ```rust
   // Won't refetch when user changes
   let data = Resource::new(|| (), |_| fetch());
   
   // Will refetch when user_id changes
   let data = Resource::new(move || user_id.get(), |id| async move { fetch(id).await });
   ```

### 11.3 IPC Debugging

**Log IPC Calls**:
```rust
// app/src/tauri_invoke.rs
pub async fn invoke<T>(command: &str, args: serde_json::Value) -> Result<T, String> {
    web_sys::console::log_1(&format!("Invoking: {} with args: {}", command, args).into());
    // ... rest of function
}
```

**Check Tauri Availability**:
```javascript
// In browser DevTools console
console.log(window.__TAURI__);  // Should show Tauri API object
console.log(window.__TAURI__.core);  // Should show invoke function
console.log(window.__TAURI__.event);  // Should show listen/emit functions
```

### 11.4 Known Gotchas

1. **Mutex across await points**: Never hold a MutexGuard across an await
   ```rust
   // WRONG - will deadlock
   let auth = state.auth.lock().await;
   let result = some_async_fn().await;  // Deadlock here!
   drop(auth);
   
   // RIGHT - drop before await
   let auth_snapshot = {
       let auth = state.auth.lock().await;
       auth.clone()
   };
   let result = some_async_fn().await;
   ```

2. **SQLite thread safety**: Use `--test-threads=1` for tests
   ```bash
   cargo test -p arcadestr-core -- --test-threads=1
   ```

3. **WASM bundle size**: Trunk builds can be large. Use `--release` for production.

4. **NIP-46 connection timing**: The fast connection flow returns before handshake completes. Always check connection status before signing.

5. **Relay connection failures**: Relays may return HTML instead of WebSocket responses if down. Check logs for "expected ident" errors.

---

## 12. Glossary

| Term | Definition |
|------|------------|
| **NOSTR** | "Notes and Other Stuff Transmitted by Relays" - A decentralized protocol for publishing and subscribing to events |
| **NIP** | NOSTR Implementation Possibility - A specification document for NOSTR features (e.g., NIP-01, NIP-46) |
| **NIP-46** | Remote signer protocol allowing apps to request signatures from external signer apps |
| **NIP-07** | Browser extension signer protocol using `window.nostr` API |
| **NIP-57** | Lightning Zaps protocol for sending/receiving Bitcoin payments over Lightning |
| **NIP-65** | Relay List Metadata (Outbox Model) - specifies how users publish their preferred relays |
| **npub** | Bech32-encoded public key (starts with `npub1`) |
| **nsec** | Bech32-encoded private key (starts with `nsec1`) |
| **bunker** | A NIP-46 signer service that holds private keys and signs on behalf of users |
| **nostrconnect://** | URI scheme for initiating NIP-46 connections |
| **Kind** | Event type identifier in NOSTR (e.g., kind 0 = metadata, kind 30078 = game listing) |
| **d-tag** | Parameterized replaceable event identifier (e.g., `d:my-game-v1`) |
| **Relay** | WebSocket server that stores and forwards NOSTR events |
| **WebView** | Embedded browser component used by Tauri to render the UI |
| **WASM** | WebAssembly - compiled Rust code that runs in browsers |
| **Trunk** | Build tool for Rust/WASM web applications |
| **Tauri** | Framework for building desktop apps with web frontends |
| **Leptos** | Rust web framework using fine-grained reactivity |
| **RwSignal** | Read-write reactive signal in Leptos |
| **Resource** | Async data loader with loading/error states in Leptos |
| **Action** | Async mutation handler with pending state in Leptos |
| **Arc** | Atomically Reference Counted - shared ownership type in Rust |
| **Mutex** | Mutual exclusion lock for thread-safe access |
| **tokio** | Async runtime for Rust |
| **sqlx** | Async SQL library with compile-time checked queries |
| **thiserror** | Macro for deriving std::error::Error |
| **serde** | Serialization/deserialization framework |
| **zap** | Lightning payment sent via NIP-57 |
| **LNURL-pay** | Lightning Network URL protocol for generating invoices |
| **lud16** | Lightning address format (e.g., `user@walletofsatoshi.com`) |
| **CSP** | Content Security Policy - browser security mechanism |
| **CSR** | Client-Side Rendering - UI rendered entirely in browser |
| **SSR** | Server-Side Rendering - UI rendered on server (not used in Arcadestr) |

---

## The 5 Most Important Things to Understand

Before diving into the codebase, ensure you deeply understand these concepts:

### 1. **Dual-Target Architecture**
The same Leptos UI runs in both Tauri (desktop) and browser (web), but with different authentication mechanisms. The `app` crate uses conditional compilation (`#[cfg(target_arch = "wasm32")]`) to handle both cases.

### 2. **NIP-46 Fast Connection Flow**
Authentication is asynchronous and deferred. The `connect_bunker` command returns immediately with a "connecting" state, while a background task completes the WebSocket handshake. The UI must poll `get_connection_status` to track progress.

### 3. **Relay Gossip (NIP-65)**
The app doesn't just connect to hardcoded relays. It fetches each user's relay list, builds a coverage map, and uses greedy set cover to select optimal relays. This is the key to efficient decentralized communication.

### 4. **Feature-Gated Core**
The `core` crate compiles differently for native (desktop) and WASM (web) targets. Native gets tokio, sqlx, and full NOSTR functionality. WASM gets stubs and relies on browser APIs. Understanding `#[cfg(feature = "native")]` guards is essential.

### 5. **Tauri IPC Pattern**
All frontend→backend communication goes through `tauri_invoke.rs`, which uses `js_sys::eval` to call `window.__TAURI__.core.invoke()`. Commands are registered in `main.rs` with `#[tauri::command]` and state is shared via `AppState` with `Arc<Mutex<T>>` wrappers.

---

*Documentation generated for Arcadestr codebase. Last updated: 2026-04-02*
