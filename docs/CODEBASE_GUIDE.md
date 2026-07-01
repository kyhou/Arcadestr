# Arcadestr Codebase Guide

> A step-by-step walkthrough for Rust developers new to the project.
> **Goal:** After reading this guide, you should be able to confidently modify, create, and audit code across the entire stack.

---

## Table of Contents

1. [Bird's Eye View](#1-birds-eye-view)
2. [Workspace Topography](#2-workspace-topography)
3. [The Dual-Target Pattern](#3-the-dual-target-pattern)
4. [Core Crate Deep Dive](#4-core-crate-deep-dive)
5. [Tauri Desktop Shell](#5-tauri-desktop-shell)
6. [Frontend: Leptos App](#6-frontend-leptos-app)
7. [Testing & Mocking Infrastructure](#7-testing--mocking-infrastructure)
8. [Common Tasks (Cookbook)](#8-common-tasks-cookbook)
9. [Reference](#9-reference)

---

## 1. Bird's Eye View

### What is Arcadestr?

Arcadestr is a **decentralized indie game marketplace** built on the Nostr protocol. Users can browse, publish, and purchase games using Lightning (Bitcoin) payments, all authenticated through Nostr keys.

### The Four Crates

```
arcadestr-workspace/
├── core/        # Library — all business logic (Nostr, relays, storage, NIPs)
├── desktop/     # Binary — Tauri v2 desktop app (Rust host + WebView)
├── app/         # Library — Leptos frontend components (shared by web + desktop)
└── web/         # Binary — WASM entrypoint for web browser builds
```

### Technology Stack

| Layer | Technology | Purpose |
|-------|-----------|---------|
| Frontend | **Leptos 0.8** (Rust WASM framework) | UI components, routing, state |
| Desktop Shell | **Tauri 2** | Native window, OS keychain, IPC bridge |
| Binary protocol | **Nostr** (via `nostr-sdk` + `nostr` crates) | Events, relays, signatures |
| Storage | **SQLite** (`sqlx` + `rusqlite`) | User accounts, caches, marketplace listings |
| Encryption | **AES-256-GCM**, **XChaCha20-Poly1305**, **scrypt/Argon2** | Key storage, NIP-44, NIP-49 |
| Payments | **LNURL** + **NIP-57 Zaps** | Lightning invoices |

### What NIPs are implemented?

See [Section 9: Reference](#9-reference) for the full map.

---

## 2. Workspace Topography

### Dependency Graph

```
web (bin, WASM)
  └── app (lib) ──── core (lib)
                        ↑
desktop (bin, native) ──┘
  └── app (lib, feat=native)
       └── core (lib, feat=native)
```

- **`desktop`** always compiles with `features = ["native"]`, giving it full access to SQLite, OS keychain, relay connections, HTTP client, etc.
- **`web`** compiles with `features = ["wasm"]`, where `core` provides stub implementations for native-dependent modules.
- **`app`** is the shared UI layer. Components are inactive until mounted by either `desktop` (via Tauri's WebView) or `web` (via Trunk/`wasm-bindgen`).

### What each crate provides

**`arcadestr-core`** (`core/`) — The engine room:
- Nostr event construction, signing, and publishing
- NIP-46 remote signer integration (Amber, Nsec.app, etc.)
- NIP-57 Lightning zap request generation
- Marketplace listing parsing (NIP-15, NIP-99)
- NIP-05 identity verification
- NIP-58 badge/achievement parsing and caching
- NIP-65 relay hint extraction
- NIP-78 encrypted relay backup
- SQLite database: accounts, caches, migrations
- Relay connection management (pool, hints, discovery, subscriptions)
- Profile fetching (batched, cached)
- Social graph / extended network discovery

**`arcadestr-app`** (`app/`) — The UI layer:
- Leptos reactive components
- `tauri_invoke.rs`: Low-level JS interop with `window.__TAURI__`
- `tauri_bridge.rs`: Typed wrappers with dual-target dispatch
- Two UI variants: legacy (`components/`) and v2 (`ui_v2/`)
- Web-auth helpers (`web_auth.rs`, `web_secure_store.rs`)

**`arcadestr-desktop`** (`desktop/`) — The native host:
- `main.rs`: Tauri builder, AppState assembly, 53 registered commands
- `command_contracts.rs`: Orchestration layer between Tauri and core
- `nip46_commands.rs`: NIP-46 command handlers (profile lifecycle, QR login)
- `tauri.conf.json`: Window dimensions, build steps, frontend dist path

**`arcadestr-web`** (`web/`) — The browser entrypoint:
- `main.rs`: `leptos::mount_to_body(App)`
- `Trunk.toml`: dev server on port 5173
- `index.html`: HTML shell with Tailwind + Google Fonts

---

## 3. The Dual-Target Pattern

This is the single most important architectural pattern to understand. Arcadestr compiles to **two targets** from the same codebase:

| Target | Platform | Binary | Frontend Host |
|--------|----------|--------|---------------|
| Native | macOS/Linux/Windows | `arcadestr-desktop` | Tauri WebView |
| WASM | Any browser | `arcadestr-web` | Browser DOM |

### How it works

Three feature flags control what code compiles:

```
desktop/Cargo.toml:
  arcadestr-core = { features = ["native"] }   → enables all real implementations
  arcadestr-app  = { features = ["native"] }   → enables Tauri IPC calls

web/Cargo.toml:
  arcadestr-app = { features = ["wasm"] }       → enables WASM stubs

app/Cargo.toml:
  native = ["arcadestr-core/native"]
  wasm  = ["arcadestr-core/wasm"]
```

### The gating patterns you'll see

**Pattern A: `#[cfg(feature = "native")]` on modules** (`core/src/lib.rs`)

```rust
pub mod signers;          // Always compiled — types shared by both targets

#[cfg(feature = "native")]
pub mod storage;          // Only compiled for desktop — requires sqlx

#[cfg(feature = "native")]
pub mod relay_manager;    // Only compiled for desktop — requires tokio + nostr-sdk

#[cfg(feature = "wasm")]
pub mod wasm_stub;        // WASM placeholder — returns Err("...unsupported...")
```

**Pattern B: `#[cfg(target_arch = "wasm32")]` for conditional exports inside a module** (`core/src/signers/mod.rs`)

```rust
#[cfg(not(target_arch = "wasm32"))]
pub use nip46::{ActiveSigner, DirectKeySigner, Nip46Signer, NostrSigner, SignerError};

#[cfg(target_arch = "wasm32")]
pub use nip46::{Nip07Signer, NostrSigner, SignerError};
```

**Pattern C: Dual implementations in `app/`** (`app/src/tauri_bridge.rs`)

```rust
#[cfg(not(feature = "web"))]
pub async fn invoke_nip49_import(request: Nip49ImportRequest) -> Result<String, String> {
    crate::tauri_invoke::invoke("nip49_import", serde_json::json!({ "request": request })).await
}

#[cfg(feature = "web")]     // ← web target gets a stub
pub async fn invoke_nip49_import(_request: Nip49ImportRequest) -> Result<String, String> {
    Err("nip49_import is only available in desktop builds".to_string())
}
```

**Pattern D: WASM stubs** (`core/src/wasm_stub.rs`)

```rust
pub fn nip49_import(_ncryptsec: &str, _password: &str) -> Result<String, String> {
    Err("NIP-49 import unsupported on wasm target".to_string())
}
```

### Where to gate what

| What needs gating | Correct attribute | Example |
|---|---|---|
| Entire module needs OS APIs | `#[cfg(feature = "native")]` | `storage`, `relay_manager` |
| Same module, different exports per target | `#[cfg(not(target_arch = "wasm32"))]` vs `#[cfg(target_arch = "wasm32")]` | `signers/mod.rs` |
| Frontend calls Tauri API | `#[cfg(not(feature = "web"))]` vs `#[cfg(feature = "web")]` | `tauri_bridge.rs` |
| Stub functions for WASM | `#[cfg(feature = "wasm")]` | `wasm_stub.rs` |

> **Rule of thumb:** If a function opens a file, a socket, or an OS keychain, it's native-only. If it calls `window.__TAURI__`, it's desktop-only. If it's pure data processing, it's both.

---

## 4. Core Crate Deep Dive

### 4.1 Module Map

```
core/src/
├── lib.rs                 # Module declarations + feature gating + public re-exports
│
├── signers/               # [always compiled] Signing abstraction
│   ├── mod.rs             #   Conditional re-exports per target
│   ├── local.rs           #   LocalSigner: nsec → signing
│   ├── nip46.rs           #   ActiveSigner enum, NostrSigner trait, Nip46Signer
│   └── lazy_nip46.rs      #   [native] Deferred handshake NIP-46 signer
│
├── nostr.rs               # [native] Central NostrClient:
│                          #   relay management, event ops, profile fetch, relay discovery
│
├── auth/                  # [native] Account lifecycle
│   ├── auth_state.rs      #   AuthState: signer selection, connection state
│   ├── account.rs         #   Account struct + SigningMode enum
│   └── account_manager.rs #   AccountManager: CRUD + encryption
│
├── storage/               # [native] SQLite persistence
│   ├── db.rs              #   Database: pool, migrations, badge/marketplace CRUD
│   ├── encryption.rs      #   AES-256-GCM, XChaCha20Poly1305, scrypt (NIP-49)
│   ├── master_key.rs      #   MasterKeyManager: file-based key, Argon2 KDF
│   ├── backup.rs          #   NIP-78 encrypted relay backup
│   ├── marketplace_cache.rs # DB-backed marketplace listing cache
│   └── migration.rs       #   Legacy saved_users.json → SQLite migration
│
├── nip46/                 # [native] NIP-46 remote signer protocol
│   ├── types.rs           #   ConnectionState, Nip46UriType, AppSignerState
│   ├── session.rs         #   activate_profile, restore_session_on_startup, logout
│   ├── storage.rs         #   OS keyring persistence for profiles
│   ├── auth.rs            #   QR login flow, init_signer_session
│   └── methods.rs         #   get_public_key, sign_event, encrypt/decrypt
│
├── relay_cache.rs         # [native] SQLite relay list cache (NIP-65)
├── relay_hints.rs         # [native] Extract relay URLs from event tags
├── relay_pool.rs          # [native] Unified relay set per profile
├── relay_manager.rs       # [native] Persistent pool: connect, retry, stream
├── relay_events.rs        # [native] Real-time relay status broadcast channels
├── subscriptions.rs       # [native] REQ subscription lifecycle + notification loop
├── profile_fetcher.rs     # [native] Batched profile fetch with LRU cache
├── user_cache.rs          # [native] SQLite user profile cache (24h TTL)
│
├── marketplace.rs         # [native] NIP-15 + NIP-99 listing parsing + streaming fetch
├── marketplace_cache.rs   # [native] SQLite marketplace cache
├── lightning.rs           # [native] NIP-57 zap invoice generation
├── nip05_validator.rs     # [native] DNS-based identity verification
├── http_client.rs         # [native] HttpClient trait + ReqwestHttpClient
│
├── achievements.rs        # [native] NIP-58 badge parsing, caching, queries
├── social_graph.rs        # [native] SQLite follower graph (rusqlite)
├── extended_network.rs    # [native] 2nd-degree follow discovery (NIP-02)
│
├── saved_users.rs         # [always] Legacy saved users JSON persistence
├── version.rs             # [always] VERSION + REVISION constants
│
├── wasm_stub.rs           # [wasm] Placeholder structs for non-native modules
│
└── test_helpers/          # [test+native] Mock infrastructure
    ├── http_mocks.rs      # MockHttpClient — in-process HTTP mock
    └── nip46_mocks.rs     # MockNip46Relay — in-process NIP-46 relay mock
```

### 4.2 The API Surface

`core/src/lib.rs` re-exports the key types consumers need:

```rust
// Consumers (desktop) use:
use arcadestr_core::nostr::NostrClient;
use arcadestr_core::auth::AuthState;
use arcadestr_core::storage::Database;
use arcadestr_core::relay_cache::RelayCache;
use arcadestr_core::marketplace::GameListing;
use arcadestr_core::signers::NostrSigner;
use arcadestr_core::nip46::AppSignerState;
```

### 4.3 Relay Discovery (4-tier cascade)

When Arcadestr needs to find which relays a user is on, it uses this cascade in `NostrClient`:

```
Tier 1: NIP-65 Kind 10002 relay list metadata (preferred)
Tier 2: "Seen on" relays — which relays have events for this pubkey
Tier 3: Relay hints from p-tags / e-tags in events
Tier 4: Global fallbacks (wss://relay.damus.io, wss://relay.primal.net)
```

### 4.4 Event Kinds Used

| Kind | Constant | NIP | Purpose |
|------|----------|-----|---------|
| 0 | `Kind::Metadata` | NIP-01 | Profile metadata |
| 3 | `KIND_FOLLOW_LIST` / `KIND_CONTACT_LIST` | NIP-02 | Follow list |
| 7 | `Kind::Reaction` | NIP-25 | Reactions |
| 8 | `KIND_BADGE_AWARD` (30009) | NIP-58 | Badge awards |
| 9734 | `Kind::ZapRequest` | NIP-57 | Zap request |
| 10002 | `KIND_RELAY_LIST` | NIP-65 | Relay list metadata |
| 10008 | `KIND_PROFILE_BADGES_CURRENT` | NIP-58 | Current profile badges |
| 30008 | `KIND_PROFILE_BADGES_DEPRECATED` | NIP-58 | Deprecated profile badges |
| 30009 | `KIND_BADGE_DEFINITION` | NIP-58 | Badge definitions |
| 30017 | `Kind::Stall` | NIP-15 | Marketplace stall |
| 30018 | `Kind::Product` | NIP-15 | Marketplace product |
| 30078 | `KIND_GAME_LISTING` | custom | Game listing (Arcadestr-specific) |
| 30402 | `Kind::ClassifiedListing` (deprecated) | NIP-99 | Classified listing |
| 30403 | `Kind::DraftClassifiedListing` (deprecated) | NIP-99 | Draft listing |

### 4.5 Key Error Types

```rust
// Core error patterns (all use thiserror):
#[derive(Error, Debug)]
pub enum NostrError { ... }     // 14 variants
pub enum SignerError { ... }    // 9 variants
pub enum RelayManagerError { ... }
pub enum DatabaseError { ... }
pub enum SocialGraphError { ... }
pub enum LightningError { ... }  // 7 variants
pub enum AchievementError { ... } // 10 variants
pub enum BackupError { ... }
pub enum HttpClientError { ... } // Build, Request, RedirectBlocked, Status, Json

// Application-level error:
pub enum CommandError { ... }   // in desktop/command_contracts.rs, uses thiserror
```

### 4.6 Key Abstractions

**`NostrSigner` trait** (in `signers/nip46.rs`):
```rust
#[async_trait]
pub trait NostrSigner: Send + Sync {
    async fn get_public_key(&self) -> Result<PublicKey, SignerError>;
    async fn sign_event(&self, event: &mut Event) -> Result<(), SignerError>;
    async fn nip44_encrypt(&self, content: &str, pubkey: &PublicKey) -> Result<String, SignerError>;
    async fn nip44_decrypt(&self, content: &str, pubkey: &PublicKey) -> Result<String, SignerError>;
}
```
Implementations: `DirectKeySigner` (local nsec), `Nip46Signer` (remote bunker), `LazyNip46Signer` (deferred remote), `Nip07Signer` (WASM browser extension).

**`HttpClient` trait** (in `http_client.rs`):
```rust
#[async_trait]
pub trait HttpClient: Send + Sync {
    async fn get_json(&self, url: &str) -> Result<Value, HttpClientError>;
    async fn get_json_no_redirects(&self, url: &str) -> Result<Value, HttpClientError>;
}
```
Implementations: `ReqwestHttpClient` (production), `MockHttpClient` (tests).

---

## 5. Tauri Desktop Shell

### 5.1 AppState — The shared context

`desktop/src/main.rs` defines `AppState`, a struct holding every shared resource. It's injected into Tauri commands via `tauri::State<'_, AppState>`:

```rust
pub struct AppState {
    pub auth: Arc<Mutex<AuthState>>,
    pub nostr: Arc<Mutex<NostrClient>>,
    pub database: Arc<Database>,
    pub relay_cache: Arc<RelayCache>,
    pub deduplicator: Arc<Mutex<EventDeduplicator>>,
    pub subscription_registry: Arc<SubscriptionRegistry>,
    pub profile_fetcher: Arc<ProfileFetcher>,
    pub user_cache: Arc<UserCache>,
    pub marketplace_cache: Arc<MarketplaceCache>,
    pub extended_network: Arc<RwLock<Option<Arc<Mutex<ExtendedNetworkRepository>>>>>,
    pub extended_network_follows: Arc<RwLock<Vec<String>>>,
    pub relay_hints: Option<Arc<RelayHints>>,
    pub nip05_validator: Arc<Mutex<Nip05Validator>>,
    pub http_client: Arc<dyn HttpClient>,
}
```

Every field is wrapped in `Arc` for thread-safe sharing. `Mutex` for mutable sync, `RwLock` for read-heavy access.

### 5.2 Startup flow

When the desktop app starts, `main.rs`:

1. Initializes tracing (`tracing-subscriber`)
2. Creates `AppState` with all components
3. Creates `AppSignerState` separately (managed by `nip46_commands.rs`)
4. In `.setup()` closure, spawns an async task that calls `restore_session_on_startup()`:
   - Emits `session_restoring` event to frontend
   - On success: restores relays → fetches follow list → spawns extended network
   - On failure: emits `session_offline_mode`, `show_login`, or `session_restore_failed`
5. Spawns background tasks:
   - `run_notification_loop` — processes incoming events, emits `nostr_event` to frontend
   - Relay event listener — emits `relay-connection` (connected/disconnected) to frontend
   - Relay hint flush task (every 60 seconds)

### 5.3 The IPC Pipeline (end-to-end)

When the UI calls a Tauri command, here's the full path:

```
Leptos Component
  ↓  calls typed wrapper
tauri_bridge.rs (typed, dual-target)
  ↓  calls generic invoke
tauri_invoke.rs (JS interop)
  ↓  window.__TAURI__.core.invoke("command_name", {...})
  ↓  Tauri process deserializes args, resolves command
#[tauri::command] in main.rs (thin handler)
  ↓  delegates to business logic
command_contracts.rs (orchestration)
  ↓  calls core library
arcadestr-core module (pure business logic)
  ↓  returns Result<T, Error>
Response serialized back up through the same layers
```

**Concrete example: `verify_nip05`**

1. **Leptos**: Calls `invoke_verify_nip05(identifier, expected_npub)` from `tauri_bridge.rs`
2. **Bridge**: `crate::tauri_invoke::invoke("verify_nip05", json!({...}))`
3. **Tauri invoke**: `window.__TAURI__.core.invoke("verify_nip05", {...})`
4. **Tauri command** (`main.rs`):
   ```rust
   #[tauri::command]
   async fn verify_nip05(identifier: String, expected_npub: String,
       state: tauri::State<'_, AppState>) -> Result<Nip05Status, String> {
       command_contracts::verify_nip05(identifier, expected_npub, &state).await
           .map_err(|e| e.to_string())
   }
   ```
5. **Command contract** (`command_contracts.rs`): Parses npub → hex, calls `core_verify_nip05_identity`
6. **Core** (`nostr.rs`): HTTP GET to `https://<domain>/.well-known/nostr.json`, returns verification result

### 5.4 Command Registration

All 53 commands are registered in `main.rs` via `tauri::generate_handler![]`. They come from two sources:
- Inline `#[tauri::command]` functions in `main.rs` (~40 commands)
- The `nip46_commands` module (~13 commands: `connect_bunker`, `get_connection_status`, `start_qr_login`, etc.)

### 5.5 Events (Tauri Emit → Frontend Listen)

The backend pushes real-time data to the frontend via `app_handle.emit("event_name", payload)`:

| Event | Direction | Purpose |
|-------|-----------|---------|
| `nostr_event` | backend → frontend | Raw Nostr events from subscription loop |
| `relay-connection` | backend → frontend | Relay connected/disconnected status |
| `auth_success` | backend → frontend | NIP-46 auth completed successfully |
| `bunker-auth-challenge` | backend → frontend | Browser approval URL for remote signer |
| `bunker-heartbeat` | backend → frontend | Periodic connection health check |
| `qr-login-complete` | backend → frontend | QR code login Flow B complete |
| `session_restoring` | backend → frontend | Startup session is being restored |
| `session_offline_mode` | backend → frontend | No stored session, show login |
| `show_login` | backend → frontend | Need user to log in |
| `session_restore_failed` | backend → frontend | Could not restore previous session |
| `marketplace-product` | backend → frontend | Single product from streaming fetch |
| `marketplace-complete` | backend → frontend | Streaming fetch complete |

---

## 6. Frontend: Leptos App

### 6.1 Overview

The `app/` crate contains all UI code using Leptos 0.8 (CSR mode). It has two UI variants:

```
app/src/
├── lib.rs                 # App component, routing, 40+ invoke_* wrappers
├── models.rs              # Shared types: ListingSummary, ProfileInfo, etc.
│
├── components/            # Legacy UI (v1 components)
│   ├── mod.rs
│   ├── profile.rs         # Profile display
│   ├── profile_display.rs, profile_avatar.rs
│   ├── browse.rs, detail.rs, publish.rs  # Marketplace pages
│   ├── account_selector.rs, backup_manager.rs
│   ├── badge_earned_modal.rs, badge_showcase.rs
│   ├── nip05_badge.rs, nip49_modal.rs
│   └── ...
│
├── ui_v2/                 # Current UI (v2, the main codebase)
│   ├── mod.rs
│   ├── shell.rs           # Main app shell (sidebar + content area)
│   ├── theme.rs           # Dark theme CSS variables
│   ├── components/
│   │   ├── nav_item.rs    # Sidebar navigation item
│   │   └── topbar.rs      # Top bar with relay status
│   └── views/
│       ├── login.rs       # NIP-46 login (QR + bunker URI)
│       ├── browse_games.rs, game_detail.rs
│       ├── library.rs, store_front.rs
│       ├── profile.rs, social.rs
│       ├── publish.rs, achievements.rs
│       └── marketplace_loader.rs (streaming)
│
├── store/                 # Leptos signals (shared reactive state)
│   ├── mod.rs             #   Store: profiles, marketplace, relay_state
│   ├── profiles.rs        #   ProfileStore: reactive profile cache
│   └── marketplace.rs     #   MarketplaceStore: reactive listing cache
│
├── tauri_invoke.rs        # Low-level: window.__TAURI__.core.invoke() via js_sys
├── tauri_bridge.rs        # Typed IPC wrappers (NIP-49, NIP-05, badges)
├── qr.rs                  # QR code generation (SVG)
├── relay_state.rs         # Relay connection status tracking
├── web_auth.rs            # Web auth helpers (NIP-07 browser extension)
└── web_secure_store.rs    # Web localStorage secure storage
```

### 6.2 IPC from the frontend side

**For most commands** (desktop only): Call `invoke("command_name", args)` from `tauri_invoke.rs`:
```rust
pub async fn invoke<T: DeserializeOwned>(command: &str, args: Value) -> Result<T, String>
```
This uses `js_sys::eval` to call `window.__TAURI__.core.invoke()` and converts the JS Promise to a Rust Future.

**For event listening**: `listen(event_name, callback)` from `tauri_invoke.rs`:
```rust
pub async fn listen<F>(event: &str, callback: F) -> Result<impl FnOnce(), String>
```
Returns a cleanup `FnOnce` to unsubscribe.

**Typed wrappers**: `tauri_bridge.rs` wraps specific commands with proper types and dual-target gating.

### 6.3 Streaming pattern

For large data (marketplace listings), the app uses a streaming pattern:

```
1. Frontend calls: invoke_fetch_marketplace_stream()
2. Frontend starts listening: listen("marketplace-product", callback)
3. Backend emits events as products are fetched
4. Frontend appends each product to reactive signal
5. Backend emits: listen("marketplace-complete")
```

### 6.4 The v1 vs v2 split

The project has two generations of UI:
- **`components/`**: Older components, gradually being replaced
- **`ui_v2/`**: Current UI with sidebar navigation, dark theme, modern layout

When adding new UI, add to `ui_v2/`. The `lib.rs` `App` component likely wraps `ui_v2::shell::AppShell` or switches between them.

---

## 7. Testing & Mocking Infrastructure

### 7.1 Testing targets

```
cargo test -p arcadestr-core --lib -- --test-threads=1   # unit tests
cargo test -p arcadestr-core                              # + integration tests
cargo test -p arcadestr-desktop                            # desktop tests
```

### 7.2 MockHttpClient

Defined in `core/src/test_helpers/http_mocks.rs`.

Replaces real HTTP for any code path using the `HttpClient` trait. Use this for:
- NIP-05 verification tests
- LNURL/zap endpoint tests
- Any test that makes HTTP requests

```rust
use arcadestr_core::test_helpers::http_mocks::MockHttpClient;

let mock = MockHttpClient::new()
    .with_nip05_response("example.com", "bob", pubkey_hex)  // /.well-known/nostr.json
    .with_lnurl_response("bob@example.com", "https://callback.com/zap");  // LNURL pay

mock.call_count("https://example.com/.well-known/nostr.json");  // → verify it was called
mock.last_requested_url();  // → inspect what URL was hit
```

### 7.3 MockNip46Relay

Defined in `core/src/test_helpers/nip46_mocks.rs`.

Simulates a NIP-46 relay in-process without WebSocket. Use for:
- NIP-46 connection lifecycle tests
- Signer method call tests (get_public_key, sign_event)
- End-to-end auth flow tests

Supports `MethodBehavior` (Default/Result/Error/Disconnect) for testing error paths.

### 7.4 Testing patterns by area

| Area | Mock strategy | Key file |
|------|--------------|----------|
| NIP-05 verification | `MockHttpClient` | `tests/integration_nip05.rs` |
| Lightning zaps | `MockHttpClient` | `tests/integration.rs` (int_05) |
| NIP-46 signer | `MockNip46Relay` | `tests/integration.rs` (int_03) |
| Marketplace fetch | `MockHttpClient` + relay events | `tests/integration.rs` (int_04) |
| Badge parsing | Pure data — no mocks | `tests/integration.rs` |
| Database operations | `tempfile::TempDir` | `tests/integration.rs` |
| Command layer | Tauri test harness | `desktop/tests/` |

### 7.5 Database testing convention

```rust
#[tokio::test]
async fn test_database_operation() {
    let tmp = tempfile::TempDir::new().unwrap();
    let db_path = tmp.path().join("test.db");
    let db = Database::new(db_path.to_str().unwrap()).await.unwrap();

    // ... test operations ...

    // Cleanup: tmp drops, file is removed
}
```

### 7.6 Key test plan

See `docs/TEST_PLAN.md` for the full plan. Sections:
- 3.1–3.12: Unit test sections per NIP/feature
- Section 4: Integration scenarios (INT-01 through INT-05)
- Section 5: Error & edge case tests
- Section 6: Tauri command layer tests

---

## 8. Common Tasks (Cookbook)

### 8.1 Add a new Tauri command

**1. Core layer** — Add the business logic in `core/src/`:

```rust
// core/src/my_feature.rs
pub fn my_operation(input: &str) -> Result<OutputType, MyFeatureError> { ... }
```

Gate it in `core/src/lib.rs`:
```rust
#[cfg(feature = "native")]
pub mod my_feature;
```

**2. Command contract** — Add an orchestration function in `desktop/src/command_contracts.rs`:
```rust
pub async fn my_operation(input: String, state: &AppState) -> Result<OutputType, CommandError> {
    // parse, validate, call core, handle errors
    arcadestr_core::my_feature::my_operation(&input)
        .map_err(|e| CommandError::MyFeature(e.to_string()))
}
```

**3. Tauri command** — Add the handler in `desktop/src/main.rs`:
```rust
#[tauri::command]
async fn my_operation(input: String, state: tauri::State<'_, AppState>) -> Result<OutputType, String> {
    command_contracts::my_operation(input, state.inner()).await
        .map_err(|e| e.to_string())
}
```

**4. Register** — Add to `generate_handler![]` in `main.rs`.

**5. Frontend** — Add typed wrapper (optional) or call directly:
```rust
// app/src/lib.rs or wherever
pub async fn invoke_my_operation(input: String) -> Result<OutputType, String> {
    #[cfg(not(feature = "web"))]
    { crate::tauri_invoke::invoke("my_operation", json!({ "input": input })).await }
    #[cfg(feature = "web")]
    { Err("my_operation requires desktop".to_string()) }
}
```

### 8.2 Add a new database migration

**1.** Add SQL to `core/migrations/003_your_feature.sql`.

**2.** Add a constant in `core/src/storage/db.rs`:
```rust
const MIGRATION_6_YOUR_FEATURE: &str = include_str!("../../migrations/003_your_feature.sql");
```

**3.** Add to the `MIGRATIONS` slice and bump the `PRAGMA user_version` check in `run_migrations()`.

### 8.3 Add a new NIP handler

1. Create `core/src/nip_xxx.rs` (or add to existing module)
2. Define event parsing functions (e.g., `parse_xxx_event(event) -> Result<XxxType, XxxError>`)
3. Define event building functions (e.g., `build_xxx_event(content) -> EventBuilder`)
4. Add `pub use` re-exports in `core/src/lib.rs`
5. If it needs relay ops, integrate with `NostrClient` in `core/src/nostr.rs`
6. If it needs storage, add tables to migrations and CRUD to `core/src/storage/db.rs`
7. Wire up Tauri commands if it needs frontend access

### 8.4 Add a new frontend view

1. Create `app/src/ui_v2/views/your_feature.rs`
2. Add it to `app/src/ui_v2/views/mod.rs`
3. Add the view component to the router in `app/src/ui_v2/shell.rs`
4. Add a nav item in the sidebar (follow `nav_item.rs` pattern)
5. If it needs backend data, wire up `invoke_*` calls

### 8.5 Write a test

See the patterns in [Section 7](#7-testing--mocking-infrastructure):

```rust
#[tokio::test]    // for async tests
async fn test_feature_works() {
    // Arrange
    let mock = MockHttpClient::new()
        .with_nip05_response("example.com", "user", pubkey_hex);

    // Act
    let result = verify_nip05_identity(&mock, "user@example.com", &pubkey_hex).await;

    // Assert
    assert!(result.is_ok());
    assert_eq!(mock.call_count(url), 1);
}
```

---

## 9. Reference

### 9.1 NIP Implementation Map

| NIP | Status | Module | Notes |
|-----|--------|--------|-------|
| NIP-01 | ✅ | `nostr.rs` | Base protocol, event format |
| NIP-02 | ✅ | `extended_network.rs`, `social_graph.rs` | Contact list, follow graph |
| NIP-05 | ✅ | `nip05_validator.rs` | DNS identity verification |
| NIP-07 | ✅ | `signers/nip46.rs` (WASM) | Browser extension signer |
| NIP-15 | ✅ | `marketplace.rs` | Marketplace stalls/products |
| NIP-19 | ✅ | (nostr-sdk) | bech32 encoding (npub, nsec, nprofile) |
| NIP-25 | ✅ | (nostr-sdk) | Reactions |
| NIP-42 | ✅ | (nostr-sdk) | Relay authentication |
| NIP-44 | ✅ | `nip46/methods.rs` | Versioned encryption (ECDH + XChaCha20) |
| NIP-46 | ✅ | `nip46/`, `signers/` | Remote signer (full implementation) |
| NIP-49 | ✅ | `storage/encryption.rs` | Encrypted private key (ncryptsec) |
| NIP-57 | ✅ | `lightning.rs` | Lightning zaps |
| NIP-58 | ✅ | `achievements.rs` | Badge definitions + awards |
| NIP-59 | ⬜ | — | Gift wrap (not yet implemented) |
| NIP-65 | ✅ | `relay_cache.rs`, `relay_hints.rs` | Relay list metadata |
| NIP-78 | ✅ | `storage/backup.rs` | Application-specific data (kind 30078) |
| NIP-99 | ✅ | `marketplace.rs` | Classified listings (deprecated) |
| NIP-102 | ⬜ | — | Marketplace receipt (spec exists) |

### 9.2 Key Constants

```rust
// core/src/nostr.rs
pub const KIND_GAME_LISTING: u16 = 30078;
pub const KIND_FOLLOW_LIST: u16 = 3;
pub const KIND_RELAY_LIST: u16 = 10002;
pub const KIND_CONTACT_LIST: u16 = 3;
pub const DEFAULT_RELAYS: [&str; 4];
pub const DISCOVERY_RELAYS: [&str; 3];
pub const INDEXER_RELAYS: [&str; 2];

// core/src/achievements.rs
pub const KIND_BADGE_AWARD: u16 = 8;
pub const KIND_PROFILE_BADGES_CURRENT: u16 = 10008;
pub const KIND_PROFILE_BADGES_DEPRECATED: u16 = 30008;
pub const KIND_BADGE_DEFINITION: u16 = 30009;

// core/src/profile_fetcher.rs
pub const BATCH_SIZE: usize = 10;
pub const MAX_PROFILE_ATTEMPTS: usize = 2;

// core/src/relay_hints.rs
pub const MAX_HINTS_PER_PUBKEY: usize = 5;
pub const MAX_PERSISTED: usize = 2000;

// core/src/version.rs
pub const VERSION: &str = "0.1.0";
pub const REVISION: u32 = 35;
```

### 9.3 Important Crates (external)

| Crate | Version | Purpose |
|-------|---------|---------|
| `nostr` | 0.44 | Core Nostr types (Event, Keys, Kind, PublicKey) |
| `nostr-sdk` | 0.44 | Client, relay pool, subscriptions (native only) |
| `nostr-connect` | 0.44 | NIP-46 protocol messages |
| `leptos` | 0.8 | Reactive UI framework (CSR mode) |
| `tauri` | 2 | Native app shell + IPC |
| `sqlx` | 0.8 | Async SQLite (sqlx::sqlite) |
| `rusqlite` | 0.32 | Sync SQLite (social graph) |
| `reqwest` | 0.12 | HTTP client |
| `keyring` | 3.6 | OS keychain access |
| `aes-gcm` | 0.10 | AES-256-GCM for nsec encryption |
| `chacha20poly1305` | 0.10 | XChaCha20-Poly1305 for NIP-44 |
| `argon2` | 0.5 | Master key derivation |
| `scrypt` | 0.11 | NIP-49 KDF |
| `qrcode` | 0.14 | QR code SVG generation (no default features) |

### 9.4 File naming conventions

| Pattern | Example | Rule |
|---------|---------|------|
| Source files | `relay_cache.rs` | `snake_case` |
| Feature-gated modules | `wasm_stub.rs` | Prefix `wasm_` for WASM-only |
| Test helpers | `nip46_mocks.rs` | `_mocks` suffix |
| Command contracts | `nip46_commands.rs` | `_commands` suffix |
| Integration tests | `integration_nip05.rs` | `integration_` prefix |
| UI components | `account_selector.rs` | Same as component name |
| Views | `browse_games.rs` | Plural, descriptive |
| Stores | `marketplace.rs` | Domain name |

### 9.5 Error handling conventions

**Core library** (`arcadestr-core`): Use `thiserror` for all error types:
```rust
#[derive(Error, Debug)]
pub enum MyError {
    #[error("Invalid input: {0}")]
    InvalidInput(String),
    #[cfg(feature = "native")]
    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),
}
```

**Desktop app** (`arcadestr-desktop`): Uses `anyhow` for flexibility:
```rust
pub async fn my_operation(input: String) -> anyhow::Result<Output> { ... }
```

**Tauri commands**: Return `Result<T, String>` — errors are serialized as strings for the frontend.

**Never use `.unwrap()`** in production code. Use `.expect("context message")` or proper error propagation.

### 9.6 Mutex pattern

```rust
// For mutable shared state:
Arc<Mutex<T>>   // .expect("... mutex poisoned") on lock

// For read-heavy shared state:
Arc<RwLock<T>>  // .read().expect("...") / .write().expect("...")

// Never hold a lock across an await point:
// ❌ BAD:
let guard = state.lock().await;
some_async_fn().await;   // lock still held!
drop(guard);

// ✅ GOOD:
let value = { state.lock().await.field.clone() };
some_async_fn().await;
```

### 9.7 Key files index

| File | What it contains |
|------|-----------------|
| `core/src/lib.rs` | Module declarations, feature gates, public API |
| `core/src/nostr.rs` | NostrClient, relay discovery, event helpers |
| `core/src/storage/db.rs` | SQLite pool, migrations, CRUD |
| `core/src/signers/nip46.rs` | NostrSigner trait, signer implementations |
| `core/src/nip46/session.rs` | Session lifecycle (activate, restore, logout) |
| `core/src/test_helpers/http_mocks.rs` | MockHttpClient builder |
| `desktop/src/main.rs` | AppState, Tauri setup, 53 command handlers |
| `desktop/src/command_contracts.rs` | Orchestration layer |
| `desktop/src/nip46_commands.rs` | NIP-46 Tauri command handlers |
| `app/src/tauri_invoke.rs` | Low-level JS interop |
| `app/src/tauri_bridge.rs` | Typed IPC wrappers |
| `app/src/lib.rs` | 40+ invoke_* wrappers, App component |
| `docs/TEST_PLAN.md` | Full test specification |
| `docs/SECURE_STORAGE_API.md` | Tauri command API docs |
| `AGENTS.md` | Agent development guidelines |
| `COMMANDS.md` | Build/test command reference |

### 9.8 Where to look for common operations

| You need to... | Look in |
|----------------|---------|
| Parse a Nostr event | `core/src/nostr.rs` or the relevant NIP module |
| Build/sign an event | `core/src/nostr.rs` + signer |
| Store something in SQLite | `core/src/storage/db.rs` |
| Add a database table | `core/migrations/` + `core/src/storage/db.rs` |
| Call from the frontend | Add to `app/src/lib.rs` invoke_* + `desktop/src/main.rs` command |
| Mock an HTTP call | `core/src/test_helpers/http_mocks.rs` |
| Mock NIP-46 | `core/src/test_helpers/nip46_mocks.rs` |
| Add a relay method | `core/src/relay_manager.rs` |
| Handle a new event kind | Add constant to `core/src/nostr.rs` + parser |
| Add a UI view | `app/src/ui_v2/views/` + register in `shell.rs` |
| Debug the backend | `.vscode/BACKEND_DEBUG.md` |

---

> **Final advice:** Start by reading `core/src/nostr.rs` and `desktop/src/main.rs` — they're the two most central files. Then read one complete integration test (e.g., `int_01` in `tests/integration.rs`) to see how pieces connect. Everything else is specialization of patterns you'll find there.
