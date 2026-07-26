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

1. **Browse Listings**: View published games with metadata (title, description, price, tags, images)
2. **Publisher Studio**: Create or replacement-edit NIP-99 kind-30402 listings, preserve unmanaged metadata, provision ADP fulfillment, upload builds, and create/update/cancel claim campaigns
3. **Acquire Games**: Buy paid games over Lightning, claim active claim-and-keep promotions, or use explicitly public/half-open timed access; zero price alone never grants access
4. **Ownership, History & Install**: Track durable NIP-102 receipts and provisional NIP-103-style grants, show account-scoped purchase/claim history, stream authenticated ADP downloads, verify SHA-256 hashes, quarantine corrupt artifacts, and list device installs separately from ownership
5. **Authentication**: NIP-07 browser extensions, NIP-46 remote signers, and encrypted local nsec accounts
6. **Zap Payments**: NIP-57 Lightning zaps for game purchases
7. **Marketplace Cache**: Persistent SQLite storage for offline browsing
8. **Real-time Relay Status**: Live connection indicators with latency metrics
9. **Achievements & Badges**: NIP-58 badge display on profiles, earned badge tracking, and achievement showcase
10. **Platform Filtering**: Browse games filtered by host platform (OS/arch) with automatic platform detection
11. **Marketplace Empty States**: Graceful UI when no listings are found, with distinct messages for browse vs store front
12. **Account & Network Settings**: Switch/remove accounts, reconnect signers and relays, export local keys, and inspect non-sensitive diagnostics
13. **Truthful Capability States**: Unsupported web, community, profile-editing, backup, appearance, and install-lifecycle actions are hidden or presented as unavailable rather than fabricated

### Desktop vs Web Target

Arcadestr is a **dual-target application**:

| Target | Technology | Authentication | Use Case |
|--------|-----------|----------------|----------|
| **Desktop** | Tauri v2 + Leptos | NIP-46 or encrypted local nsec | Native app experience, ADP file selection/downloads, and local install registry |
| **Web** | Leptos (WASM) | NIP-07 or browser-encrypted nsec | Browser marketplace access without native installation |

Both targets share UI components from the `app` crate. The desktop target delegates NOSTR, ADP, storage, and local signing to Rust commands; the web target uses browser APIs for NIP-07 and browser-encrypted local signing and returns desktop-only fallbacks for native installation operations.

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
| **rusqlite** | 0.32 | Synchronous SQLite for relay hints/cache and social graph data | `core/src/relay_cache.rs`, `core/src/relay_hints.rs`, `core/src/social_graph.rs` |
| **keyring** | 3.6 | Direct OS keychain access for NIP-46/ncryptsec storage | `core/src/nip46/storage.rs` |
| **tauri-plugin-keyring** | 0.1 | Present in the desktop manifest but not registered as a Tauri plugin | `desktop/Cargo.toml` |
| **argon2** | 0.5 | Password hashing for encryption | `core/src/storage/encryption.rs` |
| **aes-gcm** | 0.10 | AES-256-GCM encryption | `core/src/storage/encryption.rs` |
| **scrypt** | 0.11 | NIP-49 KDF for ncryptsec | `core/src/storage/encryption.rs` |
| **chacha20poly1305** | 0.10 | XChaCha20-Poly1305 for NIP-49 | `core/src/storage/encryption.rs` |
| **Bech32 encoding** | internal | Manual NIP-49 `ncryptsec1...` encoding/decoding | `core/src/storage/encryption.rs` |
| **reqwest** | 0.12 | HTTP, multipart upload, and streamed ADP downloads | `core/src/http_client.rs`, `core/src/adp_client.rs` |
| **lightning-invoice** | 0.32 | Bolt11 invoice parsing for purchase receipts | `core/src/purchases.rs` |
| **sha2** | 0.10 | Payment-proof hashing, build verification, and deterministic install paths | `core/src/purchases.rs`, `desktop/src/install.rs`, `desktop/src/main.rs` |
| **wasm-bindgen** | 0.2 | WASM/JavaScript interop | `app/`, `web/` crates |
| **web-sys** | 0.3 | Browser API bindings | `app/src/web_auth.rs` |
| **qrcode** | 0.14 | QR code generation for NIP-46 | `app/Cargo.toml` |
| **gloo-timers** | 0.3 | WASM timer utilities | `app/Cargo.toml` |
| **send_wrapper** | 0.6 | Safe Leptos cleanup wrapper for async Tauri event listeners | `app/src/ui_v2/views/game_detail.rs` |
| **url** | 2 | Structural HTTP(S) validation for listing media, ADP servers, and profile links | `app/src/campaign_management.rs`, `app/src/ui_v2/`, `core/src/adp_publish.rs` |
| **tauri-plugin-dialog** | 2 | Native build archive selection | `desktop/src/adp_commands.rs` |
| **tauri-plugin-mcp-bridge** | 0.12 | Debug-only desktop/WebView automation and IPC inspection | `desktop/src/main.rs` |
| **aes-gcm** | 0.10 | AES-256-GCM encryption (WASM) | `app/Cargo.toml` |
| **bitcoin** | 0.32 | Bitcoin types for payment proofs (dev) | `core/Cargo.toml` |
| **lightning-types** | 0.1 | Lightning types for invoice validation (dev) | `core/Cargo.toml` |
| **tailwindcss** | latest | Noir OKLCH design tokens, semantic utilities, gradients, shadows, and responsive CSS | `web/tailwind.config.js`, `web/style/tailwind.css` |
| **trunk** | latest | WASM build tool and dev server | `web/Trunk.toml` |
| **tracing** | 0.1 | Structured logging | Throughout codebase |
| **thiserror** | 1.0 | Error type derivation | `core/` crate |

### Why Tauri over Pure Web or Pure Native?

The choice of Tauri v2 is driven by several factors evident in the codebase:

1. **Security**: NIP-46 keeps remote keys in signer apps, while local-account nsecs are handled by Arcadestr, encrypted with AES-256-GCM, and stored in `accounts.db`. The desktop master key is a mode-`0600` file in the app data directory; it is not currently backed by the OS keyring.

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
├── ADP-01.md               # ADP protocol specification
├── ADP-01-amendment-free-acquisitions.md # Explicit access/campaign amendment
├── NIP-102.md              # Purchase receipt specification used by ownership
├── NIP-103.md              # Experimental entitlement/campaign proposal notes
├── Arcadestr_UI_Screen_Inventory.md # Product screen/capability inventory
├── Arcadestr_UI_Screen_Inventory_Revised.md # Revised acquisition-aware inventory
├── opencode.json           # Repository-local OpenCode MCP/watch configuration
├── COMMANDS.md             # Available CLI commands
├── test_nostrconnect.sh    # Test script for NIP-46
├── .vscode/                # VS Code workspace config and debug launcher
│   ├── launch.json         # LLDB backend launch and attach configurations
│   ├── tasks.json          # Build tasks for debugging
│   ├── settings.json       # Workspace settings
│   ├── extensions.json     # Recommended extensions
│   ├── README.md           # Workspace debugging overview
│   └── BACKEND_DEBUG.md    # LLDB/rr backend debugging walkthrough
│
├── core/                   # LIBRARY: Core business logic (NOSTR, storage, crypto)
│   ├── Cargo.toml          # Native-only dependencies (tokio, sqlx, etc.)
│   ├── src/
│   │   ├── lib.rs          # Module exports, feature-gated (native vs wasm)
│   │   ├── nostr.rs        # NOSTR client, event handling, relay management
│   │   ├── achievements.rs # NIP-58 badge parsing, validation, and caching
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
│   │   │   ├── backup.rs       # Backup/restore functionality
│   │   │   └── marketplace_cache.rs # Storage-layer marketplace helpers
│   │   ├── relay_cache.rs    # NIP-65 relay list caching
│   │   ├── relay_hints.rs    # Relay discovery from p-tags
│   │   ├── relay_events.rs   # Real-time relay connection events
│   │   ├── relay_manager.rs  # Background relay pool management
│   │   ├── relay_pool.rs     # Relay connection pooling
│   │   ├── profile_fetcher.rs # Batched profile fetching
│   │   ├── marketplace_cache.rs # Persistent marketplace listing cache
│   │   ├── marketplace.rs    # NIP-99 parsing and streaming marketplace fetch
│   │   ├── nip05_validator.rs # NIP-05 validation service
│   │   ├── user_cache.rs      # Persistent user profile cache
│   │   ├── social_graph.rs   # Extended network discovery
│   │   ├── extended_network.rs # 2nd-degree follow discovery
│   │   ├── subscriptions.rs  # Relay subscription management
│   │   ├── lightning.rs      # NIP-57 zap payments
│   │   ├── saved_users.rs    # Legacy saved users (JSON file)
│   │   ├── purchases.rs      # NIP-102 purchase receipt persistence and verification
│   │   ├── adp_protocol.rs   # Shared provisional campaign/grant kinds and tags
│   │   ├── ownership.rs      # Unified receipt/grant ownership and durable acquisition history
│   │   ├── campaign.rs       # Implementation-local provisional campaign chain validation
│   │   ├── campaign_discovery.rs # Pointer plus authoritative campaign discovery
│   │   ├── entitlements.rs   # Implementation-local provisional grant parsing and validation
│   │   ├── entitlements_repository.rs # Validated grant persistence and history
│   │   ├── authorization.rs  # Kind-30406 fulfillment authorization lifecycles
│   │   ├── adp_client.rs     # ADP HTTP client: provision, upload, purchase, download
│   │   ├── adp_discovery.rs  # Kind-30403 ADP server discovery and parsing
│   │   ├── adp_publish.rs    # Validated kind-30402 listing and kind-30406 authorization builders
│   │   ├── adp_storage.rs    # Provisioning, download-token, and installed-game repositories
│   │   ├── file_hash.rs      # Streaming SHA-256 file hashing
│   │   ├── hash_validation.rs # Shared exact SHA-256 hex validation
│   │   ├── replaceable_event.rs # Central timestamp/event-ID replacement ordering
│   │   ├── http_client.rs    # Shared HTTP abstraction and atomic streamed downloads
│   │   ├── nip98_client.rs   # NIP-98 HTTP authorization
│   │   ├── nwc_client.rs     # Nostr Wallet Connect payment client
│   │   ├── lnurlp.rs         # LNURL-pay address resolution and invoice request
│   │   ├── version.rs        # Version constants
│   │   ├── test_helpers.rs   # Shared test-helper exports
│   │   ├── test_helpers/     # HTTP and NIP-46 mocks
│   │   │   ├── http_mocks.rs
│   │   │   └── nip46_mocks.rs
│   │   └── wasm_stub.rs      # WASM-compatible stubs
│   ├── migrations/           # SQLx database migrations
│   │   ├── 001_initial_schema.sql  # Initial database schema
│   │   ├── 002_achievements.sql    # NIP-58 badge achievements tables
│   │   ├── 003_purchases.sql       # NIP-102 purchase receipts
│   │   ├── 004_adp_provisioning.sql # ADP delegated fulfillment records
│   │   ├── 005_download_tokens.sql # Buyer-scoped ADP download credentials
│   │   ├── 006_installed_games.sql # Verified local install registry
│   │   └── 007_entitlements.sql # Immutable validated entitlement-grant events
│   └── tests/
│       ├── integration.rs          # Core integration coverage
│       ├── integration_nip05.rs    # NIP-05 HTTP integration tests
│       ├── adp_entitlement_protocol.rs # Campaign/grant adversarial protocol tests
│       ├── authorization.rs        # Delegated fulfillment authority tests
│       ├── campaign_discovery.rs   # Pointer/fallback campaign discovery tests
│       └── entitlements_repository.rs # Validated grant persistence tests
│
├── app/                    # LIBRARY: Leptos UI components (shared)
│   ├── Cargo.toml          # Leptos, wasm-bindgen dependencies
│   └── src/
│       ├── lib.rs          # Main app component, auth context, styles
│       ├── campaign_management.rs # Publisher campaign validation, requests, and pointer repair
│       ├── models.rs       # GameListing, UserProfile, ZapRequest, Badge types + NIP-49/NIP-05 IPC types
│       ├── tauri_bridge.rs # Tauri command wrappers (invoke_*), badge fetch functions
│       ├── tauri_invoke.rs # Low-level Tauri IPC (wasm-bindgen)
│       ├── web_auth.rs     # NIP-07 browser extension auth (web target)
│       ├── web_secure_store.rs # Secure storage for web target (AES-GCM encrypted)
│       ├── qr.rs           # QR code generation for NIP-46 login
│       ├── relay_state.rs  # Relay connection state management helpers
│       ├── ui_v2/          # Stitch-based UI v2 components
│       │   ├── mod.rs      # UI v2 module exports
│       │   ├── shell.rs    # Main application shell
│       │   ├── theme.rs    # Theme configuration
│       │   ├── components/ # Reusable UI components
│       │   │   ├── mod.rs  # Component exports
│       │   │   ├── game_card.rs # Truthful access/campaign/action card presentation
│       │   │   ├── nav_item.rs   # Navigation item component
│       │   │   ├── page_header.rs # Shared page heading component
│       │   │   └── topbar.rs     # Top navigation bar
│       │   └── views/      # Page-level views
│       │       ├── mod.rs  # View exports
│       │       ├── achievements.rs    # User achievements/badges view
│       │       ├── browse_games.rs    # Game browsing view
│       │       ├── game_detail.rs     # Game detail view
│       │       ├── library.rs         # User's game library
│       │       ├── login.rs          # Authentication view
│       │       ├── marketplace_loader.rs # Marketplace loading screen
│       │       ├── profile.rs         # User profile view
│       │       ├── publish.rs        # Game publishing view
│       │       ├── purchases.rs      # Durable purchase and promotion-claim history
│       │       ├── settings.rs       # Account, signer, network, export, diagnostics
│       │       ├── social.rs         # Explicit community-unavailable view
│       │       └── store_front.rs    # Main store front view
│       ├── components/     # UI components
│       │   ├── mod.rs      # Component exports
│       │   ├── account_selector.rs   # Login/account switching UI
│       │   ├── backup_manager.rs     # Backup/restore UI
│       │   ├── badge_earned_modal.rs # Badge earned celebration modal
│       │   ├── badge_showcase.rs     # Profile badge display component
│       │   ├── browse.rs             # Game listing grid
│       │   ├── detail.rs             # Game detail view with buy flow
│       │   ├── nip49_modal.rs        # NIP-49 export modal (password confirm + copy)
│       │   ├── nip05_badge.rs        # NIP-05 status badge (unverified/verifying/verified/failed)
│       │   ├── profile.rs            # User profile view
│       │   ├── profile_avatar.rs     # Avatar component with fallback
│       │   ├── profile_display.rs    # Profile name/display components
│       │   └── publish.rs            # Game publishing form
│       └── store/          # Global state management
│           ├── mod.rs      # Store exports
│           ├── profiles.rs # ProfileStore (reactive cache)
│           └── marketplace.rs # MarketplaceStore listing cache and refresh TTL
│
├── desktop/                # BINARY: Tauri v2 desktop application
│   ├── Cargo.toml          # Tauri v2, tauri-build dependencies
│   ├── tauri.conf.json     # Tauri configuration (window, security, build)
│   ├── build.rs            # Build script for Tauri
│   ├── src/
│   │   ├── main.rs         # Entry point, Tauri setup, commands
│   │   ├── adp_commands.rs # ADP discovery, publishing, purchase, and wallet commands
│   │   ├── install.rs      # Artifact verification, quarantine, and install recording
│   │   ├── nip46_commands.rs # NIP-46 specific Tauri commands
│   │   └── command_contracts.rs # Pure command logic for testability
│   ├── capabilities/
│   │   └── default.json    # Core, dialog, event, and debug MCP bridge permissions
│   └── tests/              # Desktop command layer tests
│       ├── section6_command_layer_tests.rs # Auth and command contract tests
│       ├── section7_nip49_commands.rs      # NIP-49/NIP-05 command contract tests
│       └── section8_badge_command_tests.rs # NIP-58 badge command tests
│
├── web/                    # BINARY: WASM web target (Trunk)
│   ├── Cargo.toml          # WASM-only dependencies
│   ├── Trunk.toml          # Trunk build configuration
│   ├── index.html          # HTML entry point
│   ├── tailwind.config.js  # Tailwind CSS configuration
│   ├── style/
│   │   └── tailwind.css    # Tailwind CSS entry point
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
│  │  │  • login_with_nsec()  • publish_adp_listing() • install_game()│  │   │
│  │  │  • connect_bunker()   • confirm_purchase()    • fetch_profile()│  │   │
│  │  └────────────────────────┬────────────────────────────────────────┘  │   │
│  │                           │                                           │   │
│  │  ┌────────────────────────▼────────────────────────────────────────┐  │   │
│  │  │                    core crate (arcadestr_core)                 │  │   │
│  │  │  ┌────────────┐ ┌────────────┐ ┌────────────┐ ┌────────────┐ │  │   │
│  │  │  │NostrClient │ │ ADP client │ │  storage   │ │ NWC/LNURL  │ │  │   │
│  │  │  │(nostr-sdk) │ │(HTTP/NIP-98)│ │  (sqlx)    │ │(payments)  │ │  │   │
│  │  │  └────────────┘ └────────────┘ └────────────┘ └────────────┘ │  │   │
│  │  └────────────────────────────────────────────────────────────────│  │   │
│  │                           │                                        │  │   │
│  │  ┌────────────────────────▼────────────────────────────────────────┐  │   │
│  │  │              External Services                                   │  │   │
│  │  │  • NOSTR Relays (wss://*)    • Lightning Network (LNURL/NWC)    │  │   │
│  │  │  • NIP-46 Signer Apps        • ADP distribution servers (HTTP) │  │   │
│  │  └──────────────────────────────────────────────────────────────────┘  │   │
│  └────────────────────────────────────────────────────────────────────────┘   │
└─────────────────────────────────────────────────────────────────────────────┘

┌─────────────────────────────────────────────────────────────────────────────┐
│                            WEB TARGET                                        │
│  ┌─────────────────────────────────────────────────────────────────────┐     │
│  │                     Browser (Leptos WASM)                          │     │
│  │  ┌──────────────┐  ┌──────────────┐  ┌──────────────┐              │     │
│  │  │ Login/NIP-07 │  │ Store shell  │  │ Local nsec   │              │     │
│  │  └──────┬───────┘  └──────┬───────┘  └──────┬───────┘              │     │
│  │         │                 │                 │                     │     │
│  │  web_auth.rs              │          web_secure_store.rs          │     │
│  │         │                 │                                       │     │
│  │  window.nostr             │ Native bridge fallbacks return        │     │
│  │  extension                │ explicit unavailable errors           │     │
│  └───────────────────────────┼───────────────────────────────────────┘     │
│                              │                                              │
│  Marketplace streaming, install registry, purchase history, publishing,    │
│  campaigns, badge relay fetches, and native network controls are not        │
│  implemented directly in the standalone web target.                         │
└─────────────────────────────────────────────────────────────────────────────┘
```

### 4.2 Request Lifecycle Example: Publishing a Game

Let's trace through the current ADP-aware flow when a user clicks "Publish":

**Step 1: UI Event (Leptos)**
```rust
// app/src/components/publish.rs
let request = PublishAdpListingRequest {
    expected_publisher_npub,
    existing_event_id,
    d_tag,
    title,
    description,
    price_sats,
    lud16,
    tags,
    images,
    fulfillment_mode, // None, Direct, or Delegate
    operator_url,
    servers,
    file_path,
    existing_file_hash,
    existing fulfillment authorization references,
    version,
    acquisition, // Gated, Public, or half-open TimedAccess
    platforms,
    campaigns,
    nip94_event_id,
};
spawn_local(async move {
    let result = invoke_publish_adp_listing(request).await;
});
```

**Step 2: Tauri Bridge (WASM)**
```rust
// app/src/tauri_bridge.rs
pub async fn invoke_publish_adp_listing(
    request: PublishAdpListingRequest,
) -> Result<PublishAdpListingResult, String> {
    tauri_invoke::invoke(
        "publish_adp_listing",
        serde_json::json!({ "request": request }),
    ).await
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
// desktop/src/adp_commands.rs
#[tauri::command]
pub async fn publish_adp_listing<R: tauri::Runtime>(
    request: PublishAdpListingRequest,
    app: tauri::AppHandle<R>,
    state: tauri::State<'_, AppState>,
    signer_state: tauri::State<'_, Arc<Mutex<AppSignerState>>>,
) -> Result<PublishAdpListingResult, String> {
    // Verify the expected publisher and optimistic edit event ID, preserve
    // unmanaged tags, prepare or reuse fulfillment authorization, recheck the
    // event ID, publish, confirm propagation, then upload only a new file.
    /* ... */
}
```

**Step 5: Core Business Logic**
```rust
// core/src/adp_publish.rs
pub fn build_adp_listing_event_builder(
    input: &AdpListingInput,
) -> Result<EventBuilder, AdpPublishError> {
    // Emits validated identity/title/price/game, acquisition, campaign,
    // platform, NIP-94, and all-or-none fulfillment metadata while retaining
    // caller-filtered preserved tags during replacement edits.
    /* ... */
}
```

**Step 6: NOSTR Network**
- A delegated publisher validates the operator attestation, then signs and broadcasts a coordinate-scoped kind-30406 root or reuses a suitable immutable active root. Listings carry repeatable root-ID/key references without lifecycle timestamps.
- The kind-30402 listing is broadcast and must become visible on at least two relays.
- New fulfilled listings upload the selected archive sequentially to every selected ADP server using NIP-98 authentication. An edit may reuse a validated existing hash and skip upload when no replacement file is selected.
- Replacement edits use `expected_publisher_npub` plus two `existing_event_id` checks as optimistic concurrency control and preserve tags not managed by the form.
- Publication and uploads are not transactional: a later upload failure does not roll back the relay event or earlier successful uploads.

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
         │ WebSocket                         HTTP + NIP-98
         ▼                                      ▼
┌─────────────────┐
│  NOSTR Relays    │
│  (wss://*)       │
└─────────────────┘
                                         ┌─────────────────┐
                                         │  ADP Servers     │
                                         │ provision/upload │
                                         │ purchase/download│
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
        nip46_commands::connect_bunker,
        nip46_commands::login_with_nsec,
        adp_commands::publish_adp_listing,
        adp_commands::confirm_purchase,
        install_game,
        get_installed_games,
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
| `connect_bunker` | `nip46_commands.rs` | `identifier: String`, `display_name: String` | `serde_json::Value` | Connects to NIP-46 and synchronizes the SDK signer into shared `AppState.auth` |
| `login_with_nsec` | `nip46_commands.rs` | `nsec: String`, `name: Option<String>` | `serde_json::Value` | Encrypts, stores, and activates a local direct-signing account |
| `start_qr_login` | `nip46_commands.rs` | None | `String` | Generates nostrconnect:// URI for QR login |
| `check_qr_connection` | `nip46_commands.rs` | None | `Option<serde_json::Value>` | Polls for QR connection completion |
| `list_saved_profiles` | `nip46_commands.rs` | None | `serde_json::Value` | Returns deduplicated NIP-46 profiles and encrypted local accounts |
| `switch_profile` | `nip46_commands.rs` | `profile_id: String` | `serde_json::Value` | Activates a NIP-46 profile or local account |
| `delete_profile` | `nip46_commands.rs` | `profile_id: String` | `()` | Removes a NIP-46 profile or local account |
| `logout_nip46` | `nip46_commands.rs` | None | `()` | Clears NIP-46 state, persisted active-account flags, and shared authentication while retaining saved accounts |
| `get_connection_status` | `nip46_commands.rs` | None | `serde_json::Value` | Returns connection state |
| `has_accounts` | `nip46_commands.rs` | None | `bool` | Checks for either NIP-46 profiles or local accounts |
| `load_active_account` | `nip46_commands.rs` | None | `serde_json::Value` | Restores local or NIP-46 state and synchronizes the signer/pubkey into shared auth |
| `generate_nostrconnect_uri` | `main.rs` | `relay: String` | `String` | Creates nostrconnect:// URI |
| `connect_nip46` | `main.rs` | `uri: String`, `relay: String` | `String` | Connects via NIP-46 URI |
| `connect_with_key` | `main.rs` | `key: String` | `String` | Activates a direct-key signer without account persistence |
| `wait_for_nostrconnect_signer` | `main.rs` | `timeout_secs: u64` | `String` | Waits for signer connection |
| `get_public_key` | `main.rs` | None | `String` | Returns authenticated npub |
| `is_authenticated` | `main.rs` | None | `bool` | Checks auth status |
| `disconnect` | `main.rs` | None | `()` | Clears auth state |
| `nip49_import` | `main.rs` | `request: Nip49ImportRequest` | `String` | Imports and activates an encrypted NIP-49 private key |
| `nip49_export` | `main.rs` | `npub: String`, `password: String` | `Nip49ExportResult` | Exports the selected local key as ncryptsec |
| `export_encrypted_key` | `main.rs` | `request: ExportKeyRequest` | `ExportKeyResult` | Runs the generic encrypted-key export contract |
| `import_encrypted_key` | `main.rs` | `request: ImportKeyRequest` | `ImportKeyResult` | Runs the generic encrypted-key import contract |
| `verify_nip05` | `main.rs` | `identifier: String`, `expected_npub: String` | `Nip05Status` | Verifies and normalizes a NIP-05 identifier |
| `verify_nip05_identity` | `main.rs` | `request: VerifyNip05Request` | `VerifyNip05Result` | Executes the typed NIP-05 identity contract |
| `fetch_listings` | `main.rs` | `limit: usize` | `Vec<GameListing>` | Fetches recent listings |
| `fetch_listing_by_id` | `main.rs` | `publisher_npub: String`, `listing_id: String` | `GameListing` | Fetches specific listing |
| `fetch_marketplace` | `main.rs` | `limit: usize`, `since_days: Option<u64>`, `filter: Option<MarketplaceFilter>` | `Vec<AppGameListing>` | Fetches NIP-99 marketplace listings with optional filtering and ownership enrichment |
| `fetch_profile` | `main.rs` | `npub: String`, `additional_relays: Option<Vec<String>>` | `UserProfile` | Fetches NIP-01 metadata |
| `request_invoice` | `main.rs` | `zap_request: ZapRequest` | `ZapInvoice` | Generates Lightning invoice |
| `get_saved_users` | `main.rs` | None | `String` | Returns saved users JSON |
| `get_saved_user` | `main.rs` | `user_id: String` | `String` | Get specific saved user by ID |
| `add_saved_user` | `main.rs` | `method: String`, `relay: Option<String>`, `uri: Option<String>`, `private_key: Option<String>`, `npub: String` | `String` | Adds saved user |
| `remove_saved_user` | `main.rs` | `user_id: String` | `String` | Removes saved user |
| `rename_saved_user` | `main.rs` | `user_id: String`, `new_name: String` | `String` | Update user alias/name |
| `connect_saved_user` | `main.rs` | `user_id: String` | `serde_json::Value` | Reconnects saved user |
| `get_connected_relay_count` | `main.rs` | None | `usize` | Returns relay count |
| `get_connected_relays` | `main.rs` | None | `Vec<String>` | Returns connected relay URLs |
| `reconnect_relays` | `main.rs` | None | `String` | Reconnect to default relays |
| `fetch_and_save_user_profile` | `main.rs` | None (uses app handle) | `UserProfile` | Fetch and save current user's profile |
| `get_extended_network_stats` | `main.rs` | None | `Option<NetworkStats>` | Returns extended network info |
| `get_relay_hints_for_pubkey` | `main.rs` | `pubkey: String` | `Vec<String>` | Returns relay hints for pubkey |
| `fetch_profile_with_hints` | `main.rs` | `identifier: String` | `UserProfile` | Fetch profile using NIP-19 hints |
| `get_cached_profiles` | `main.rs` | None | `Vec<UserProfile>` | Get all cached profiles |
| `get_cached_profile` | `main.rs` | `npub: String` | `Option<UserProfile>` | Get single cached profile |
| `get_cached_earned_badges` | `main.rs` | `profile_pubkey: String` | `Vec<EarnedBadgeSummary>` | Loads cached badge awards and definitions |
| `get_cached_profile_badges` | `main.rs` | `profile_pubkey: String` | `Vec<ProfileBadgeEntry>` | Loads the cached profile badge display list |
| `get_version_info` | `main.rs` | None | `VersionInfo` | Get app version info |
| `fetch_marketplace_stream` | `main.rs` | `limit: usize`, `since_days: Option<u64>`, `until_secs: Option<u64>`, `request_id: String` | `()` | Streams cached and relay listings on request-scoped events |
| `get_platform_info` | `main.rs` | None | `PlatformInfo` | Returns host OS/arch for platform-aware filtering |
| `get_listing_ownership` | `main.rs` | `buyer_npub: String`, `publisher_npub: String`, `listing_id: String` | `bool` | Checks receipt-or-grant ownership for an explicit buyer and full listing coordinate |
| `get_purchase_records` | `main.rs` | None | `Vec<DurableAcquisitionRecord>` | Returns account-scoped purchase and promotion-claim history for the active key |
| `install_game` | `main.rs` | `listing: GameListing` | `()` | Refetches signed policy/delivery metadata, checks active-account ownership or current explicit access, then verifies/quarantines and records the artifact |
| `get_installed_games` | `main.rs` | None | `Vec<InstalledGame>` | Lists verified device installs newest-first |
| `ingest_receipt` | `main.rs` | `raw_event_json: String` | `()` | Parses and persists a NIP-102 kind-1020 purchase receipt |
| `check_adp_server` | `adp_commands.rs` | `server_url: String` | `AdpServerInfo` | Fetches and decodes `/.well-known/adp` |
| `discover_adp_servers` | `adp_commands.rs` | None | `Vec<AdpServerAnnouncement>` | Discovers and deduplicates kind-30403 server announcements from connected relays |
| `resolve_adp_operator` | `adp_commands.rs` | `request: ResolveAdpOperatorRequest` | `Option<String>` | Resolves exactly one locally known operator for a publisher/key/scope |
| `discover_campaigns` | `adp_commands.rs` | `request: DiscoverCampaignsRequest` | `Vec<DiscoveredCampaign>` | Resolves campaign chains, including tip/predecessor IDs and lifecycle state |
| `discover_campaign_summaries` | `adp_commands.rs` | `request: DiscoverCampaignSummariesRequest` | `Vec<CampaignSummary>` | Batch-counts active/upcoming campaigns for a publisher's listings |
| `publish_campaign` | `adp_commands.rs` | `request: PublishCampaignRequest` | `PublishCampaignResponse` | Publishes create/update/cancel events; listing-pointer failure is typed partial success |
| `update_campaign_pointer` | `adp_commands.rs` | `request: UpdateCampaignPointerRequest` | `String` | Adds/removes one campaign pointer on the current replacement listing |
| `claim_entitlement` | `adp_commands.rs` | `request: ClaimEntitlementRequest` | `ClaimEntitlementResponse` | Claims a campaign grant, validates/persists it, and caches a buyer-scoped token |
| `select_build_file` | `adp_commands.rs` | None | `Option<String>` | Opens the native archive picker; cancellation is not an error |
| `hash_build_file` | `adp_commands.rs` | `request: HashBuildFileRequest` | `String` | Computes the selected build's SHA-256 digest |
| `publish_adp_listing` | `adp_commands.rs` | `request: PublishAdpListingRequest` | `PublishAdpListingResult` | Publishes buy-only/direct/delegated kind-30402 listings and uploads fulfilled builds |
| `request_lnurl_invoice` | `adp_commands.rs` | `request: RequestLnurlInvoiceRequest` | `RequestLnurlInvoiceResponse` | Requests the listing's fixed-amount LNURL invoice |
| `connect_nwc_wallet` | `adp_commands.rs` | `request: ConnectNwcWalletRequest` | `ConnectNwcWalletResponse` | Connects NWC and returns safe wallet metadata without exposing the secret |
| `pay_nwc_invoice` | `adp_commands.rs` | `request: PayNwcInvoiceRequest` | `PayNwcInvoiceResponse` | Pays through NWC and returns the preimage and optional fee |
| `confirm_purchase` | `adp_commands.rs` | `request: ConfirmPurchaseRequest` | `ConfirmPurchaseResponse` | Refetches the listing, confirms proof with ADP, validates/persists the receipt, and caches its token |
| `fetch_profile_badges` | `main.rs` | `profile_pubkey: String` | `Vec<ProfileBadgeEntry>` | Fetch NIP-58 badges for a profile (desktop only) |
| `fetch_earned_badges` | `main.rs` | `profile_pubkey: String` | `Vec<EarnedBadgeSummary>` | Fetch earned badges with definitions (desktop only) |
| `publish_game_score` | `nip46_commands.rs` | `score: u64` | `String` | Placeholder command; requires NIP-46 session but does not yet publish an event |
| `ping_bunker` | `nip46_commands.rs` | None | `serde_json::Value` | Pings the active signer and emits `bunker-heartbeat` |
| `attempt_reconnect` | `nip46_commands.rs` | None | `serde_json::Value` | Manually reconnects an offline NIP-46 session |
| `get_network_discovery_settings` | `main.rs` | None | `NetworkDiscoverySettings` | Get relay discovery settings |
| `set_allow_insecure_public_ws` | `main.rs` | `allow: bool` | `()` | Toggle allowing insecure ws:// relays for public hosts |
| `test_extended_network_discovery` | `main.rs` | None | `serde_json::Value` | Debug-only forced extended-network discovery with detailed statistics |

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
| `auth_success` | `switch_profile`, `connect_with_key` contract | `String` (npub) | Saved-profile or direct-key activation completed |
| `auth_logout` | `logout_nip46` | `()` | User logged out |
| `profile_fetched` | `initialize_relay_gossip` | `UserProfile` | New profile available |
| `profile_fetch_progress` | `initialize_relay_gossip` | `ProfileFetchProgress` | Batch fetch progress |
| `user_profile_loaded` | `initialize_relay_gossip` | `UserProfile` | Current user profile loaded |
| `extended_network_discovered` | `initialize_extended_network` | `NetworkStats` | Extended network ready |
| `bunker-auth-challenge` | `connect_bunker` | `String` (authorization URL) | Prompts the frontend to complete bunker authorization |
| `bunker_reconnected` | `attempt_reconnect` | `String` (npub) | Manual reconnect success |
| `bunker-heartbeat` | `ping_bunker` | `serde_json::Value` | Connection health check |
| `qr-login-complete` | `check_qr_connection` | `String` (npub) | QR login finished |
| `relay-connection` | `RelayManager` bridge | JSON `{ type, url, reason? }` | Unified connected/disconnected relay state update |
| `marketplace-product-{request_id}` | `fetch_marketplace_stream` | `GameListing` | Cached or relay listing isolated to one concurrent request |
| `marketplace-complete-{request_id}` | `fetch_marketplace_stream` | `()` | Completion isolated to the matching request |
| `publish-progress` | `publish_adp_listing` | `PublishProgressPayload` | Stage and per-server status for provisioning, propagation, and upload |
| `download-progress` | `install_game` | `DownloadProgressPayload` | Cumulative downloaded bytes and optional content length |
| `download-complete` | `install_game` | `DownloadCompletePayload` | Verified install path keyed by coordinate and listing ID |
| `session_restoring` | Startup | `()` | Session restore in progress |
| `session_restored` | Startup | `()` | Session restore complete |
| `session_offline_mode` | Startup | `()` | Saved NIP-46 session exists but the bunker is unreachable |
| `show_login` | Startup | `()` | No saved session exists |
| `session_restore_failed` | Startup | `String` | Session restoration failed |
| `nostr_event` | Subscription notification loop | `SerializableEvent` | Forwards subscribed relay events to the WebView |

### 5.4 Permissions & Capabilities

Tauri v2 uses a **capability-based security model**. Runtime permissions are declared in `desktop/capabilities/default.json`:

```json
{
  "permissions": [
    "core:default",
    "core:window:default",
    "core:webview:default",
    "core:event:default",
    "core:event:allow-listen",
    "core:event:allow-emit",
    "dialog:default",
    "mcp-bridge:default"
  ]
}
```

`tauri-plugin-dialog` is always registered for build selection. `tauri-plugin-mcp-bridge` is registered only under `#[cfg(debug_assertions)]`, although its capability remains listed unconditionally. `desktop/tauri.conf.json` also has `csp: null` and `withGlobalTauri: true` because the Leptos bridge calls `window.__TAURI__`.

**Production hardening still required:**

1. Define a strict CSP
2. Scope allowed domains and APIs
3. Ensure MCP bridge permissions cannot become reachable in release builds

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
App (app/src/lib.rs) - Chooses LoginV2View or UiV2Root from AuthContext
│
├── AuthContext (global state - provided at root)
│
├── AccountSelector (app/src/components/account_selector.rs) - Legacy/embedded account UI
│   ├── Login View (nostrconnect/bunker input)
│   ├── QR Login View (QR code display)
│   ├── Nsec Login View (encrypted local account)
│   └── Account List View (switch between saved accounts)
│
├── MainView (app/src/lib.rs) - Legacy main application view
│
├── BrowseView (app/src/components/browse.rs)
│   ├── ListingCard (repeated for each game)
│   └── Loading states
│
├── DetailView (app/src/components/detail.rs)
│   ├── Game details (title, description, screenshots, platform tags)
│   ├── Publisher profile (ProfileDisplayName, ProfileAvatar)
│   ├── Buy flow (ZapRequest → Lightning invoice)
│   ├── Install button (paid listings require ownership receipts)
│   └── Debug: Raw NIP-99 JSON collapsible panel (debug builds only)
│
├── ProfileView (app/src/components/profile.rs)
│   ├── User metadata (name, picture, nip05)
│   └── Published games list (sourced from marketplace_loader shared state via `use_marketplace_listings_with_limit`)
│
├── PublishView (app/src/components/publish.rs)
│   ├── Form fields (title, description, price, etc.)
│   └── Publish button
│
├── BackupManager (app/src/components/backup_manager.rs)
│   └── Backup/restore UI with inline forms
│
├── ProfileAvatar (app/src/components/profile_avatar.rs)
│   └── Avatar with fallback placeholder
│
├── ProfileDisplayName (app/src/components/profile_display.rs)
│   └── Display name with fallback to npub
│
├── ProfileRow (app/src/components/profile_display.rs)
│   └── Combined avatar + display name row
│
└── DebugOverlay (app/src/lib.rs) - Debug information overlay

UiV2Root (app/src/ui_v2/shell.rs) - Active authenticated Noir interface
│   ├── In-memory `RwSignal<UiV2View>` navigation; there is no URL router
│   ├── DetailOrigin preserves Store/Library back navigation
│   └── Desktop/mobile navigation covers Store, Browse, Library, Community,
│       Achievements, Purchases, Publish, Profile, and Settings
│
├── TopBar / NavItem / MobileNavItem (app/src/ui_v2/components/)
│   └── Responsive navigation with focus-trapped mobile menu and Escape restoration
│
├── PageHeader / GameCard (app/src/ui_v2/components/)
│   └── Shared heading and truthful access/campaign/compatibility/action presentation
│
├── StoreFrontView (app/src/ui_v2/views/store_front.rs)
│   └── Valid-cover hero, categories, promotion-aware cards, and truthful empty/error states
│
├── BrowseGamesView (app/src/ui_v2/views/browse_games.rs)
│   ├── Search plus category, access, platform, and sort filters
│   ├── Campaign/install enrichment, 50-item pagination, and bounded sparse auto-fetch
│   └── Uses full-coordinate `GameCard` state rather than inferring access from price
│
├── GameDetailView (app/src/ui_v2/views/game_detail.rs)
│   ├── Rechecks active-account durable ownership and campaign availability
│   ├── Selects buy, claim, install, timed-state, incompatible, sign-in, or unavailable action
│   ├── LNURL/NWC/manual-preimage purchase and campaign-claim flows
│   └── Scoped download listener, seller profile, specs, gallery, and debug raw event data
│
├── LibraryView (app/src/ui_v2/views/library.rs)
│   └── Reconciles device installs by stable coordinate without presenting install as ownership
│
├── PurchasesView (app/src/ui_v2/views/purchases.rs)
│   └── Account-scoped purchase/claim history with active/refunded/revoked/unverified states
│
├── LoginV2View (app/src/ui_v2/views/login.rs)
│   ├── Desktop bunker/QR/local-nsec and web NIP-07/local-nsec capabilities
│   └── NIP-49 validates/decrypts only; active selection follows current identity, not stale flags
│
├── ProfileV2View / AchievementsView
│   └── Read-only profile, truthful badge visibility, cached-first badge refresh, shared listings
│
├── PublishV2View (app/src/ui_v2/views/publish.rs)
│   └── `PublishViewState` studio for games, new/edit publication, game management, and campaigns
│
├── SettingsView (app/src/ui_v2/views/settings.rs)
│   └── Account switching/removal, reconnect, NIP-49 export, relay policy, and diagnostics
│
├── SocialView (app/src/ui_v2/views/social.rs)
│   └── Explicit unavailable state; no feed, composer, trends, recommendations, or zap activity
│
└── MarketplaceLoader (app/src/ui_v2/views/marketplace_loader.rs)
    └── Cached-first request generations, pagination cursors, 50ms batches, and account isolation

Badge Components:
├── BadgeShowcase (app/src/components/badge_showcase.rs)
│   └── Horizontal badge row for profile/game detail views
│
└── BadgeEarnedModal (app/src/components/badge_earned_modal.rs)
    └── Accessible modal dialog for newly earned badges
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
| `StoredValue<T>` | Non-reactive retained state/handles | Account snapshots and event cleanup |
| `Effect` | React to identity/request changes | Clear or refetch account-scoped data |

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
    flow_generation: RwSignal<u64>,                // Reject stale identity-flow responses
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
    profiles: RwSignal<HashMap<String, UserProfile>>,
}

impl ProfileStore {
    pub fn put(&self, profile: UserProfile) {
        self.profiles.update(|map| {
            map.insert(profile.npub.clone(), profile);
        });
    }
    
    pub fn get(&self, npub: &str) -> Option<UserProfile> {
        self.profiles.get().get(npub).cloned()
    }
    
    pub fn has(&self, npub: &str) -> bool {
        self.profiles.get().contains_key(npub)
    }
}

// Provide at app root
pub fn provide_profile_store() {
    provide_context(ProfileStore::new());
}
```

`MarketplaceStore` in `app/src/store/marketplace.rs` is the second context-provided cache. It keys kind-30402 listings by full coordinate (`30402:<publisher>:<d>`) and tracks the last fetch as epoch milliseconds. `put_streaming()` applies centralized replaceable ordering: a newer timestamp wins, while equal timestamps choose the lexicographically lower event ID. The default refresh TTL is 300 seconds, and marketplace view state is cleared/reseeded when the active npub changes.

ADP-heavy views keep operation state local rather than adding another global store. Current flows primarily combine `RwSignal`, `Signal::derive`, `Effect::new`, `StoredValue`, `spawn_local`, explicit operation enums, and monotonic request generations. `AuthContext::begin_auth_flow()` and per-view expected-npub/generation checks discard late responses after account changes. `GameDetailView` still unregisters its `download-complete` listener safely even when registration finishes after unmount.

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
let publish_action = Action::new(move |request: &PublishAdpListingRequest| {
    let request = request.clone();
    async move {
        invoke_publish_adp_listing(request).await
    }
});

// In view
view! {
    <button
        on:click=move |_| publish_action.dispatch(request.clone())
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

`desktop/src/main.rs` is the Tauri application entry point. The following is a condensed architecture sketch; the source contains explicit fallbacks and early returns for initialization failures:

```rust
fn main() {
    // 1. Initialize logging and the application data directory.
    tracing_subscriber::fmt::init();
    let keys_dir = dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("arcadestr");

    // 2. Open the shared SQLite database and encrypted account store.
    let db_path = keys_dir.join("arcadestr.db");
    let runtime = tokio::runtime::Runtime::new().expect("failed to create Tokio runtime");
    let database = runtime.block_on(async {
        arcadestr_core::storage::Database::new(&db_path).await
            .expect("failed to initialize database")
    });
    let account_manager = runtime.block_on(AccountManager::new(&keys_dir))
        .expect("failed to initialize account manager");

    // 3. Build caches, relay services, HTTP client, and AppState.
    let user_cache = Arc::new(UserCache::new(database.pool().clone()));
    let nostr_client = /* ... */;
    let relay_cache = RelayCache::new(keys_dir.join("relay_cache.db")).unwrap();
    let relay_hints = Arc::new(RelayHints::new(keys_dir.join("relay_hints.db")).unwrap());
    let app_state = AppState {
        auth: Arc::new(Mutex::new(AuthState::new())),
        nostr: Arc::new(Mutex::new(nostr_client)),
        database: Arc::new(database),
        relay_cache: Arc::new(relay_cache),
        marketplace_cache: /* ... */,
        purchases: /* ... */,
        http_client: Arc::new(
            ReqwestHttpClient::new(Duration::from_secs(10))
                .expect("failed to initialize HTTP client")
        ),
        user_cache,
        relay_hints: Some(relay_hints),
        /* relay/profile/network services ... */
    };

    // 4. Register native plugins, managed state, and the IPC surface.
    let mut builder = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init());
    #[cfg(debug_assertions)]
    {
        builder = builder.plugin(tauri_plugin_mcp_bridge::init());
    }
    builder
        .manage(app_state)
        .manage(Arc::new(account_manager))
        .manage(Arc::new(Mutex::new(AppSignerState::default())))
        .invoke_handler(tauri::generate_handler![
            nip46_commands::login_with_nsec,
            adp_commands::publish_adp_listing,
            adp_commands::confirm_purchase,
            install_game,
            get_installed_games,
            /* ... */
        ])
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
| `core::relay_manager` | Background relay pool | `RelayManager`, `RelayManagerConfig` | `fetch_streaming()`, `add_relay()`, `normalize_relay_urls()` |
| `core::relay_events` | Real-time relay status | `RelayConnectionEvent`, `RelayStatus` | Event broadcast channel |
| `core::marketplace_cache` | Listing persistence | `MarketplaceCache`, `UpsertOutcome` | `upsert_listing()`, `load_listings()` |
| `core::profile_fetcher` | Batched profile fetching | `ProfileFetcher`, `LruProfileCache` | `enqueue_many()`, `fetch_batch()` |
| `core::nip05_validator` | NIP-05 verification | `Nip05Validator`, `ValidationResult` | `validate()`, `check_cache()` |
| `core::lightning` | Lightning payments | `ZapRequest`, `ZapInvoice` | `request_zap_invoice()` |
| `core::social_graph` | Extended network | `SocialGraphDb` | `discover_network()` |
| `core::achievements` | NIP-58 badges | `BadgeDefinition`, `BadgeAward`, `ProfileBadgeEntry` | `parse_badge_definition()`, `fetch_profile_badges()` |
| `core::purchases` | NIP-102 purchase receipts | `PurchasesRepository`, `StoredReceipt` | `parse_and_validate_receipt()`, `upsert_receipt()`, `is_owned()` |
| `core::ownership` | Unified durable ownership/history | `OwnershipService`, `DurableAcquisitionRecord` | `source_for()`, `durable_records_for()` |
| `core::campaign` | Implementation-local provisional campaign chains | `CampaignEvent`, `CampaignChain` | Parse, resolve, and classify campaign lifecycles |
| `core::campaign_discovery` | Pointer plus authoritative campaign lookup | `CampaignDiscoveryService` | Resolve hinted roots and publisher `#a` fallback |
| `core::entitlements` | Implementation-local provisional grant chains | Grant event/chain types | Validate immutable buyer/game/source chains |
| `core::entitlements_repository` | Validated grant persistence | `EntitlementsRepository`, `EntitlementHistoryRecord` | `ingest_event()`, `history_for_buyer()` |
| `core::authorization` | Kind-30406 fulfillment authority | Authorization lifecycle types | Resolve active/revoked delegated authority |
| `core::adp_protocol` | Shared experimental protocol constants | Kinds 1030/1031 and tag names | Central vocabulary for campaigns and grants |
| `core::adp_client` | ADP HTTP protocol | `AdpClient`, `DownloadAuth`, `AdpClientError` | `well_known()`, `provision()`, `upload()`, `purchase_confirm()`, `download()` |
| `core::adp_discovery` | ADP server discovery | `AdpServerAnnouncement` | `discover_adp_servers()`, `parse_adp_server_announcements()` |
| `core::adp_publish` | Validated ADP listing/authorization construction | `AdpListingInput`, `AdpPublishError` | `build_adp_listing_event_builder()`, `build_fulfillment_authorization_event_builder()` |
| `core::adp_storage` | ADP persistence | `AdpProvisioningRepository`, `DownloadTokensRepository`, `InstalledGamesRepository` | `for_fulfillment_scope()`, buyer-scoped `valid_token()`, `record()`, `list()` |
| `core::hash_validation` | Exact delivery-hash checks | — | `is_sha256_hex()` |
| `core::replaceable_event` | Shared replacement ordering | — | `is_replaceable_event_newer()` |
| `core::http_client` | Testable HTTP and atomic downloads | `HttpClient`, `ReqwestHttpClient`, `HttpDownloadOutcome` | `get()`, `post_json()`, `download_to_path()` |
| `core::nip98_client` | Signed HTTP requests | NIP-98 event/header helpers | `build_nip98_auth_header()` |
| `core::nwc_client` | Nostr Wallet Connect | Wallet connection/payment types | Connect and pay invoice operations |
| `core::subscriptions` | Relay subscription lifecycle | `SubscriptionRegistry` | `dispatch_ephemeral_reads_batch_with_policy()`, `dispatch_permanent_subscriptions()`, `run_notification_loop()` |
| `desktop::adp_commands` | ADP IPC orchestration | `PublishAdpListingRequest`, `ConfirmPurchaseRequest` | `publish_adp_listing()`, `confirm_purchase()`, `discover_adp_servers()` |
| `desktop::install` | Artifact integrity and quarantine | Installed-game verification helpers | `verify_and_record_downloaded_game()`, `quarantine_corrupt_artifact()` |

**Recent `core::storage::encryption` public surface additions (NIP-49 backend):**
- `ScryptParams { n, r, p }`
- `Ncryptsec { version, scrypt_n, scrypt_r, scrypt_p, salt, nonce, ciphertext }`
- `derive_key_scrypt(password, salt, params) -> Result<[u8; 32], EncryptionError>`
- `encrypt_private_key_nip49(private_key_hex, password, params) -> Result<Ncryptsec, EncryptionError>`
- `decrypt_private_key_nip49(ncryptsec, password) -> Result<String, EncryptionError>`
- `parse_ncryptsec(ncryptsec_str) -> Result<Ncryptsec, EncryptionError>`
- `serialize_ncryptsec(ncryptsec) -> Result<String, EncryptionError>`
- Existing validation helpers retained: `validate_nip49_format`, `extract_nip49_version`, `validate_nip49_password`

**Recent `core::nip46::storage` additions (desktop keychain):**
- `store_ncryptsec_in_keychain(entry_id, ncryptsec) -> Result<(), StorageError>`
- `get_ncryptsec_from_keychain(entry_id) -> Result<String, StorageError>`
- `delete_ncryptsec_from_keychain(entry_id) -> Result<(), StorageError>`
- `ncryptsec_entry_exists(entry_id) -> bool`

**Recent `core::nostr` additions (NIP-05):**
- `build_nip05_url(domain, local_part) -> String`
- `verify_nip05_identity(http_client, nip05, expected_pubkey) -> Result<Nip05Verification, NostrError>`

**New test files:**
- `core/tests/integration_nip05.rs` (NIP-05 lookup/verification with `MockHttpClient`)
- `desktop/tests/section7_nip49_commands.rs` (desktop command-layer NIP-49/NIP-05 tests)

### 7.3 Key Data Structures

#### GameListing
```rust
// app/src/models.rs
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GameListing {
    // Identity
    pub id: String,                    // d-tag value (unique slug)
    pub source: ListingSource,         // Nip15Product, Nip99Listing, or Legacy
    
    // Core metadata
    pub title: String,
    pub description: String,
    pub images: Vec<String>,           // Screenshot/cover art URLs
    pub download_url: String,            // Primary download link
    
    // Pricing (NIP-15/NIP-99 compatible)
    pub price: f64,                    // Raw price in declared currency
    pub currency: String,              // Currency code ("SATS", "USD", etc.)
    pub price_sats: u64,               // Best-effort satoshi equivalent
    
    // Stock
    pub quantity: Option<u64>,         // None = unlimited (digital)
    
    // Taxonomy
    pub tags: Vec<String>,             // Categories from t-tags
    
    // Specs (NIP-15 product attributes)
    pub specs: Vec<(String, String)>,  // Key-value product attributes
    
    // Publisher / stall
    pub publisher_npub: String,        // bech32 npub of merchant
    pub stall_id: String,              // UUID of parent stall
    pub stall_name: Option<String>,  // Human-readable stall name
    pub lud16: String,                 // Lightning address for payments
    
    // Event identity and timestamps
    pub event_id: Option<String>,      // NOSTR event ID (hex)
    pub created_at: u64,

    // Platform and delivery metadata
    pub platforms: Vec<String>,        // Platform compatibility tags (e.g. "linux-x86_64")
    pub nip94_event_id: Option<String>, // NIP-94 file metadata event id for delivery
    pub acquisition: AcquisitionPolicy, // Gated, Public, or TimedAccess
    pub campaigns: Vec<CampaignPointer>, // Advisory provisional campaign root pointers
    pub is_owned: bool,                // Valid receipt or active entitlement grant
    
    // Debug (debug builds only)
    #[cfg(debug_assertions)]
    pub nip99_raw_event_json: Option<String>,  // Raw event JSON for debug
}

/// Source of the listing data
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ListingSource {
    Nip15Product,   // Kind 30018 - Current NIP-15 standard
    Nip99Listing,   // Kind 30402/30403 - NIP-99 classified listings
    Legacy,         // Kind 30078 - Legacy Arcadestr format
}
```

`AcquisitionPolicy::{Gated, Public, TimedAccess { starts_at, ends_at }}` describes current signed-listing access; timed windows are half-open. It is separate from `DurableAcquisitionRecord`, which normalizes account history across `Purchase` and `PromotionClaim` records with `Active`, `Disputed`, `Refunded`, `Revoked`, or `Unverified` status. Public/timed access never creates one of these durable records.

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

#### PlatformInfo
```rust
// app/src/models.rs
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlatformInfo {
    pub os: String,
    pub arch: String,
}

impl PlatformInfo {
    pub fn tag(&self) -> String {
        format!("{}-{}", self.os, self.arch)
    }
}
```

#### NIP-49 / NIP-05 IPC Models (UI-first desktop flow)
```rust
// app/src/models.rs
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Nip49ImportRequest {
    pub ncryptsec: String,
    pub password: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Nip49ExportResult {
    pub ncryptsec: String,
    pub npub: String,
    pub deferred: bool,
    pub message: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Nip05Status {
    pub identifier: String,
    pub normalized_identifier: String,
    pub local_part: String,
    pub domain: String,
    pub verified: bool,
    pub status: String,
    pub message: String,
}
```

#### NIP-58 Badge Models
```rust
// app/src/models.rs
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BadgeDefinition {
    pub coordinate: String,           // "30009:<issuer>:<badge_id>"
    pub issuer_pubkey: String,        // Badge creator
    pub badge_id: String,             // d-tag identifier
    pub name: Option<String>,
    pub description: Option<String>,
    pub image_url: Option<String>,
    pub image_dimensions: Option<String>,
    pub thumb_url: Option<String>,
    pub thumb_dimensions: Option<String>,
    pub relay_url: Option<String>,
    pub event_id: String,
    pub created_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BadgeAward {
    pub event_id: String,
    pub issuer_pubkey: String,
    pub recipient_pubkey: String,
    pub badge_coordinate: String,
    pub relay_url: Option<String>,
    pub created_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProfileBadgeEntry {
    pub definition: BadgeDefinition,
    pub award: BadgeAward,
    pub display_order: usize,
    pub visible: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EarnedBadgeSummary {
    pub definition: BadgeDefinition,
    pub award: BadgeAward,
    pub visible_on_profile: bool,
}
```

#### AuthState
```rust
// core/src/auth/auth_state.rs
pub struct AuthState {
    signer: Option<ActiveSigner>,
    public_key: Option<PublicKey>,
    pending_nostrconnect: Option<PendingNostrConnectState>,
}

pub struct PendingNostrConnectState {
    pub client_keys: Keys,
    pub relay: String,
    pub secret: String,
}
```

#### AppState (Tauri managed state)
```rust
// desktop/src/main.rs
pub struct AppState {
    pub auth: Arc<Mutex<AuthState>>,
    pub nostr: Arc<Mutex<NostrClient>>,
    pub database: Arc<arcadestr_core::storage::Database>,  // Shared SQLite for command contracts
    pub relay_cache: Arc<RelayCache>,
    pub deduplicator: Arc<Mutex<EventDeduplicator>>,
    pub subscription_registry: Arc<SubscriptionRegistry>,
    pub profile_fetcher: Arc<ProfileFetcher>,
    pub user_cache: Arc<UserCache>,
    pub marketplace_cache: Arc<MarketplaceCache>,  // Persistent listing storage
    pub purchases: Arc<arcadestr_core::purchases::PurchasesRepository>,  // NIP-102 purchase receipt store
    pub extended_network: Arc<RwLock<Option<Arc<Mutex<ExtendedNetworkRepository>>>>>,
    pub extended_network_follows: Arc<RwLock<Vec<String>>>,
    pub relay_hints: Option<Arc<RelayHints>>,
    pub nip05_validator: Arc<std::sync::Mutex<Nip05Validator>>,  // Background NIP-05 verification
    pub http_client: Arc<dyn HttpClient>,  // Shared HTTP client for NIP-05, LNURL, and ADP
}
```

ADP delivery data is split between frontend IPC models and core persistence records:

```rust
// app/src/tauri_bridge.rs
pub struct InstalledGame {
    pub game_coordinate: String,
    pub file_path: String,
    pub file_hash: String,
    pub version: Option<String>,
    pub server_url: String,
    pub installed_at: i64,
}

pub struct DownloadCompletePayload {
    pub game_coordinate: String,
    pub listing_id: String,
    pub file_path: String,
}

// core/src/adp_client.rs
pub enum DownloadAuth<'a> {
    Token(String),
    Nip98 { signer: &'a dyn NostrSigner },
}

// core/src/adp_storage.rs
pub struct DownloadToken { /* buyer, coordinate, server, token, expiry */ }
pub struct AdpProvisioning { /* developer/operator/scope and authorization IDs */ }
```

`InstalledGamesRepository` replaces records by game coordinate and lists them newest-first. `DownloadTokensRepository::valid_token()` requires buyer pubkey, coordinate, and server and accepts only `expires_at > now`; expired records are not automatically removed. Runtime migration deliberately drops legacy unscoped bearer tokens rather than assigning them to an account.

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

ADP code preserves more actionable protocol failures before the IPC boundary. `AdpClientError::DownloadOwnership` maps HTTP 403, `DownloadDistribution` maps HTTP 451, and `DownloadProtocol` covers other unsuccessful statuses. `HttpClient::download_to_path()` writes to a hidden sibling temporary file and renames only after a complete stream, so an interrupted response does not replace an existing destination. Hash mismatches are returned with expected/actual digests after the file is moved to an available `.corrupt`, `.corrupt.1`, ... quarantine path.

---

## 8. Key Abstractions & Patterns

### 8.1 Signer Abstraction

The `signers` module abstracts over different signing methods:

```rust
// core/src/signers/nip46.rs
#[async_trait]
pub trait NostrSigner: Send + Sync {
    async fn get_public_key(&self) -> Result<PublicKey, SignerError>;
    async fn sign_event(&self, unsigned: UnsignedEvent) -> Result<Event, SignerError>;
}

// Implementations:
// - LocalSigner: Direct private key for encrypted local accounts
// - Nip46Signer: Remote signer via NIP-46
// - Nip07Signer: Browser extension (web target)
```

This allows the same `AuthState` to work with local and remote signers. `SdkSignerAdapter` and `ActiveSigner::Sdk` synchronize restored NIP-46 clients into shared command state. `login_with_nsec` persists the encrypted account through `AccountManager`, clears active NIP-46 retry/client state, decrypts the key into a zeroizing value, and activates the local signer. Logout clears the active database flag without deleting saved accounts, then disconnects shared auth.

### 8.2 Feature-Gated Compilation

The `core` crate uses Cargo features to support both native and WASM:

```rust
// core/src/lib.rs
#[cfg(feature = "native")]
pub mod auth;

#[cfg(feature = "native")]
pub mod storage;

#[cfg(feature = "native")]
pub mod adp_client;
#[cfg(feature = "native")]
pub mod adp_discovery;
#[cfg(feature = "native")]
pub mod adp_storage;

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
    pending: Arc<Mutex<VecDeque<String>>>,
    in_flight: Arc<Mutex<HashSet<String>>>,
    failed_attempts: Arc<Mutex<HashMap<String, u32>>>,
    cache: Arc<dyn ProfileCache>,
    persistent_cache: Option<Arc<UserCache>>,
    max_attempts: u32,
    batch_size: usize,
    nip05_validator: Option<Arc<Mutex<Nip05Validator>>>,
}

impl ProfileFetcher {
    pub fn enqueue_many(&self, pubkeys: Vec<String>) {
        let mut pending = self.pending.lock().unwrap();
        for pubkey in pubkeys {
            if !self.cache.contains(&pubkey) && !pending.contains(&pubkey) {
                pending.push_back(pubkey);
            }
        }
    }
    
    pub async fn fetch_batch(&self, nostr: &NostrClient) -> (Vec<UserProfile>, usize) {
        let batch: Vec<String> = {
            let mut pending = self.pending.lock().unwrap();
            let mut in_flight = self.in_flight.lock().unwrap();
            let count = pending.len().min(self.batch_size);
            let batch: Vec<_> = pending.drain(..count).collect();
            in_flight.extend(batch.iter().cloned());
            batch
        };
        // Fetches indexers/relays, updates memory + optional SQLite cache,
        // tracks failures, and removes the batch from in_flight.
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

Despite this section's historical heading, the active bunker-login path is now a **blocking handshake**. `connect_bunker` keeps the UI in "Connecting..." until signer approval, then persists the profile, updates `AppSignerState`, and synchronizes the SDK signer/pubkey into `AppState.auth`:

```rust
// desktop/src/nip46_commands.rs
pub async fn connect_bunker(/* identifier, state, app handles */) -> Result<Value, String> {
    let (mut profile, client) = init_signer_session(&bunker_uri.to_string(), auth_handler)
        .await?; // waits for signer approval
    save_profile_to_keyring(&profile)?;
    // Store the connected client without retaining the mutex across signer awaits.
    let signer = client.signer().await?;
    activate_remote_account(signer, &app_state).await?;
    Ok(/* connected profile JSON */)
}
```

`init_signer_session_fast` and `LazyNip46Signer` still exist as library surface, but the desktop `connect_bunker` command does not use that deferred path. QR polling remains a separate flow.

### 8.6 Relay Events (Real-time Connection Status)

The relay manager emits events via broadcast channels instead of polling:

```rust
// core/src/relay_events.rs
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
```

The frontend listens via Tauri's event system:

```rust
// app/src/tauri_invoke.rs
pub async fn listen<F>(event: &str, mut callback: F) -> Result<impl FnOnce(), String>
where
    F: FnMut(serde_json::Value) + 'static,
{
    // Uses window.__TAURI__.event.listen()
    // Returns cleanup function for unlistening
}
```

### 8.7 Marketplace Cache with Upsert

Listings are cached in SQLite with change detection. Schema includes `platforms_json`, `nip94_event_id`, `specs_json`, and source event IDs. Upsert uses the same replaceable-event invariant as relay streaming: newer timestamp first, then lexicographically lower event ID at equal timestamps:

```rust
// core/src/marketplace_cache.rs
pub enum UpsertOutcome {
    Inserted,
    Updated,
    Unchanged,
}

pub struct MarketplaceCache {
    db: Pool<Sqlite>,
}

impl MarketplaceCache {
    pub async fn latest_created_at(&self) -> Result<Option<u64>, sqlx::Error> {
        // Returns the newest created_at from cached listings
        // Used for incremental refresh cursor
    }

    pub async fn upsert_listing(
        &self,
        listing: &GameListing,
        source_event_id: Option<&str>,
    ) -> Result<UpsertOutcome, sqlx::Error> {
        // Uses INSERT ... ON CONFLICT with replacement ordering.
        // Stale candidates leave both listing data and updated_at unchanged.
    }
}
```

Current cache reconstruction is intentionally fail-closed but incomplete: `load_listings()` restores `AcquisitionPolicy::Gated` and an empty campaign-pointer list. Public/timed access and campaign pointers therefore do not survive an offline cache round trip, although delivery specs and source event identity do.

### 8.8 Streaming Marketplace Fetch

The marketplace uses streaming instead of batch fetch, with **incremental cache refresh**:

```rust
// core/src/marketplace.rs
pub async fn fetch_nip99_listings_streaming_since<F>(
    relay_manager: &Arc<tokio::sync::Mutex<RelayManager>>,
    limit: usize,
    since_secs: Option<u64>,
    until_secs: Option<u64>,
    mut on_product: F,
) -> Result<u32, String>
where
    F: FnMut(Nip99Listing) + Send + 'static,
{
    // Fetches from multiple relays concurrently
    // Calls callback for the current event at each full listing coordinate
    // Applies centralized timestamp/event-ID replacement ordering
    // Filters relay queries to only ['game'] t-tags
}
```

The Tauri command orchestrates an **incremental refresh**:

```rust
// desktop/src/main.rs
#[tauri::command]
async fn fetch_marketplace_stream(
    window: tauri::Window,
    state: tauri::State<'_, AppState>,
    limit: usize,
    since_days: Option<u64>,
    until_secs: Option<u64>,
    request_id: String,
) -> Result<(), String> {
    // 1. Load cached listings first (fast initial render)
    // 2. Compute refresh cursor from latest_cached_created_at with 24h overlap
    // 3. Stream from relays using fetch_nip99_listings_streaming_since
    // 4. Enrich each listing with ownership data via async emit tasks
    // 5. Wait for all ownership lookups before marking complete
    // 6. Emit "marketplace-complete-{request_id}" when done
}
```

**Key improvements over batch fetch:**

- **Cursor-based refresh**: Uses `marketplace_cache.latest_created_at()` to compute `since_secs`, fetching only events newer than the latest cached item with a 24-hour overlap (`MARKETPLACE_REFRESH_OVERLAP_SECS = 86400`) to catch updates to existing listings.
- **Ownership enrichment**: Each listing is checked through `OwnershipService`, setting `is_owned=true` for a valid receipt or active grant belonging to the current account.
- **Async emit tasks**: Ownership lookups are spawned as background tasks so they don't block the streaming pipeline. All tasks are awaited before completion.
- **Request isolation**: Product and completion channels include the frontend-generated request ID, preventing overlapping account refreshes from consuming each other's events.
- **Platform tag filtering**: Relay queries filter to `['game']` t-tags, reducing noise from non-game NIP-99 listings.

### 8.9 NIP-58 Badge System

NIP-58 defines badges/achievements as three event kinds working together:

```rust
// core/src/achievements.rs
pub const KIND_BADGE_DEFINITION: u16 = 30009;    // Badge template (name, image, description)
pub const KIND_BADGE_AWARD: u16 = 8;             // Award instance (who earned it)
pub const KIND_PROFILE_BADGES_CURRENT: u16 = 10008;  // Which badges user displays
```

**Badge Definition** (Kind 30009):
```rust
pub struct BadgeDefinition {
    pub coordinate: String,      // "30009:<issuer_pubkey>:<badge_id>"
    pub name: Option<String>,
    pub description: Option<String>,
    pub image_url: Option<String>,
    pub thumb_url: Option<String>,
    // ...
}
```

**Validation Flow**:
```rust
// core/src/achievements.rs
pub fn parse_and_validate_profile_badges(
    profile_badges_event: &Event,
    badge_definitions: &HashMap<String, BadgeDefinition>,
    badge_awards: &HashMap<String, BadgeAward>,
) -> Result<Vec<ProfileBadgeEntry>, AchievementError> {
    // 1. Verify event kind is 10008 or 30008
    // 2. Verify a-tags reference valid badge definitions
    // 3. Verify award issuer matches definition issuer
    // 4. Verify award recipient matches profile owner
    // 5. Return validated entries with display order
}
```

**Fetching Strategy**:
```rust
// Try profile badges event first (kind 10008)
let profile_badges = fetch_profile_badges_event(pubkey).await;
if let Ok(entries) = profile_badges {
    return entries;
}

// Fallback: fetch all award events (kind 8) where recipient is the user
let earned_badges = fetch_badge_awards_for_recipient(pubkey).await;
let definitions = fetch_badge_definitions(&earned_badges).await;
merge_into_summaries(earned_badges, definitions)
```

**Caching**: Badge definitions and awards are cached in SQLite to avoid repeated relay fetches:
- `badge_definitions` table: Stores kind 30009 events
- `badge_awards` table: Stores kind 8 events with recipient index
- `profile_badge_lists` table: Stores kind 10008/30008 events

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
cargo test -p arcadestr-core --features native

# Run specific test
cargo test -p arcadestr-core --features native --lib test_insert_and_query

# Run with single thread (for SQLite tests)
cargo test -p arcadestr-core --features native --lib -- --test-threads=1

# Desktop command, ADP install, and local-account tests
cargo test -p arcadestr-desktop

# Leptos state, presentation, and navigation tests
cargo test -p arcadestr-app
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

The desktop manifest additionally carries `tauri-plugin-dialog = "2"` for build selection, `tauri-plugin-mcp-bridge = "0.12"` for debug automation, and `sha2 = "0.10"` for deterministic artifact directories. The app manifest uses unconditional `nostr = "0.44"`, `url = "2"`, `HtmlDialogElement`, and `send_wrapper = "0.6"`; its `web` feature is now an empty target marker rather than a dependency-enabling feature. `desktop/capabilities/default.json` grants the corresponding dialog and MCP bridge capabilities; only the MCP plugin registration is compile-time debug-gated. `web/tailwind.config.js` scans `../app/src/**/*.rs` and maps semantic classes to the Noir OKLCH variables defined in `web/style/tailwind.css`, including reduced-motion rules, glass/text-gradient utilities, ambient/glow shadows, and primary/hero gradients. Root `opencode.json` enables Serena and excludes generated/build/database artifacts from watching.

### 9.3 Feature Flags

| Flag | Crate | Purpose |
|------|-------|---------|
| `native` | `core` | Enables native-only modules (tokio, sqlx, etc.) |
| `wasm` | `core` | Enables WASM stubs |
| `csr` | `app` | Client-side rendering mode |
| `hydrate` | `app` | Hydration mode (available but unused) |
| `web` | `app` | Empty target marker selecting standalone-browser code paths; NIP-07 support is conditionally compiled in the app |

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
-- core/migrations/008_favorites.sql
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
    let user_npub = {
        let auth = state.auth.lock().await;
        auth.public_key()
            .ok_or("Not authenticated")?
            .to_bech32()
            .map_err(|e| e.to_string())?
    }; // Never retain the guard across repository awaits.
    
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
    let user_npub = {
        let auth = state.auth.lock().await;
        auth.public_key()
            .ok_or("Not authenticated")?
            .to_bech32()
            .map_err(|e| e.to_string())?
    };
    
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
// app/src/tauri_bridge.rs
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

Keep typed request/response models and target-specific fallbacks in `tauri_bridge.rs`. For account-scoped operations, derive identity in the backend where possible; in the UI, capture the expected npub and request generation and ignore stale responses after account switches.

### Step 4: Create UI Component (app)

```rust
// app/src/ui_v2/views/favorites.rs
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

Export the view from `app/src/ui_v2/views/mod.rs`. If favorites are rendered as marketplace cards, construct `GameCardPresentation` rather than inventing a second set of access, campaign, ownership, and compatibility rules.

### Step 5: Add to Main App

```rust
// app/src/ui_v2/shell.rs
// Add UiV2View::Favorites, its title, desktop/mobile NavItems, and render arm.
view! {
    {move || match current_view.get() {
        UiV2View::Browse(request) => view! { <BrowseGamesView request /> }.into_any(),
        UiV2View::Favorites => view! { <FavoritesView /> }.into_any(),
        // ...
    }}
}
```

### Step 6: Test

```bash
# Run core tests
cargo test -p arcadestr-core --features native favorites

# Run app and desktop contract tests
cargo test -p arcadestr-app favorites
cargo test -p arcadestr-desktop favorites

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

**Debug Relay Selection**:
```bash
# Override relay selection during development via environment variables
ARCADESTR_RELAYS="wss://relay.example.com" cargo tauri dev

# Block runtime relay discovery (NIP-65 gossip) when using debug relays
ARCADESTR_BLOCK_DISCOVERY=1 cargo tauri dev

# Or use CLI args (highest precedence over env vars and settings):
cargo tauri dev -- --relay "wss://debug.local" --block-discovery

# Precedence chain: CLI args > Environment vars > NetworkDiscoverySettings > Defaults
```
Configuration is resolved at startup via `build_startup_relay_config()` in `desktop/src/main.rs`. Three tiers are checked:
1. CLI args (`--relay <url>`, `--block-discovery`, `--allow-discovery`)
2. Environment variables (`ARCADESTR_RELAYS`, `ARCADESTR_BLOCK_DISCOVERY`)
3. `NetworkDiscoverySettings` JSON (`debug_relays`, `block_discovery` fields)

When debug relays are active, `dispatch_ephemeral_reads_batch_with_policy()` skips relay discovery for uncovered pubkeys, and `RelayManagerConfig.block_discovery` prevents NIP-65 gossip from adding new relays at runtime.

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
Effect::new(move |_| {
    web_sys::console::log_1(&format!("Signal value: {}", signal.get()).into());
});
```

Account-sensitive effects should capture the expected npub and a flow/request generation before spawning work, then discard a response when either no longer matches. Use `.get_untracked()` when reading `ProfileStore` or `MarketplaceStore` specifically to avoid creating a reactive dependency. Debug standalone-web WASM can bypass authentication for storefront inspection with local-storage key `arcadestr.debug.storefront=1`; native-only operations remain unavailable.

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

Marketplace streams no longer use static event names. Inspect the `requestId` argument and matching `marketplace-product-{request_id}` / `marketplace-complete-{request_id}` listeners when concurrent refreshes appear stalled.

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
   cargo test -p arcadestr-core --features native -- --test-threads=1
   ```

3. **WASM bundle size**: Trunk builds can be large. Use `--release` for production.

4. **NIP-46 connection timing**: The fast connection flow returns before handshake completes. Always check connection status before signing.

5. **Relay connection failures**: Relays may return HTML instead of WebSocket responses if down. Check logs for "expected ident" errors.

6. **Web feature check command**: `cargo check -p arcadestr-app --features web` without wasm target hits a pre-existing `web_auth` cfg mismatch in `app/src/lib.rs` and is not a valid standalone web validation. Use:
   ```bash
   cargo check -p arcadestr-app --target wasm32-unknown-unknown --features web
   ```

7. **ADP download authentication**: Installation uses a non-expired token scoped by buyer, coordinate, and server, then otherwise signs a NIP-98 GET. A rejected cached token is not deleted and retried automatically. NIP-98 signs the decoded coordinate URL while the actual request percent-encodes the coordinate.

8. **Install trust boundary and failover**: `install_game` refetches the signed kind-30402 policy and delivery metadata, then requires active-account durable ownership or currently allowed public/timed access. It still uses only the first `server` tag; downloads do not resume or fail over, and all installs require an authenticated signer.

9. **Artifact scope**: A successful install stores a verified `artifact.bin` and an `installed_games` row. Extraction, launch, uninstall, repair, and reveal-in-file-manager are not implemented. A hash mismatch preserves the artifact under a unique `.corrupt*` name and does not create an install row.

10. **Publish/edit partial success**: Relay publication and multi-server uploads are not transactional. The listing can already be visible, and earlier servers can already contain the build, when a later upload fails. Campaign publication can also succeed while `pointer_update_error` reports a failed listing-pointer update. Existing-listing edits may intentionally reuse a valid hash and emit no upload progress.

11. **Local-key threat model**: Desktop local nsecs are AES-256-GCM encrypted, but the encryption master key is a mode-`0600` file in the same app data area rather than an OS-keyring secret. Web encrypted nsecs keep ciphertext and encryption key in browser storage, which does not protect against malicious JavaScript or full profile compromise.

12. **Debug MCP capability**: `tauri-plugin-mcp-bridge` registration is debug-only, but `mcp-bridge:default` remains in the default capability file. Keep release configuration under review because that permission set includes WebView script execution and backend command access.

13. **Offline policy cache is fail-closed**: `MarketplaceCache::load_listings()` currently reconstructs cached listings as gated with no campaign pointers. Public/timed access and campaign links return only after relay refresh.

14. **Provisional campaign/grant kinds**: Kinds `1030` and `1031` are experimental allocations, not finalized interoperable NIP numbers. Grant parsing validates signed tags and context but does not inspect encrypted event content.

15. **Preserved edit tags are caller-filtered**: The core builder appends `preserved_tags` verbatim. Callers must remove form-managed tags or they can produce duplicates.

16. **Remote-auth synchronization**: Connect, switch, and restore paths must update both NIP-46 client state and `AppState.auth`. Logout can fail while clearing the persisted active flag before shared auth is disconnected or `auth_logout` is emitted.

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
| **NIP-98** | HTTP authentication using a signed NOSTR event in the `Authorization` header |
| **npub** | Bech32-encoded public key (starts with `npub1`) |
| **nsec** | Bech32-encoded private key (starts with `nsec1`) |
| **bunker** | A NIP-46 signer service that holds private keys and signs on behalf of users |
| **nostrconnect://** | URI scheme for initiating NIP-46 connections |
| **Kind** | Event type identifier in NOSTR (e.g., kind 0 = metadata, kind 30402 = current game listing; kind 30078 is legacy) |
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
| **NIP-15** | NOSTR Marketplace protocol - stalls and products |
| **NIP-99** | Classified Listings - enhanced marketplace events |
| **ListingSource** | Enum indicating event kind (Nip15Product, Nip99Listing, Legacy) |
| **MarketplaceCache** | SQLite-backed persistent listing storage |
| **RelayStatus** | Real-time relay connection state with latency |
| **Upsert** | Insert or update with change detection |
| **Streaming fetch** | Progressive data loading with callbacks |
| **Tailwind CSS** | Utility-first CSS framework |
| **Stitch** | AI-powered UI generation system |
| **NIP-58** | Badges protocol - defines badge definitions, awards, and profile displays |
| **BadgeDefinition** | Kind 30009 event defining a badge (name, image, description) |
| **BadgeAward** | Kind 8 event awarding a badge to a specific user |
| **ProfileBadgeEntry** | Combined badge definition + award for display |
| **EarnedBadgeSummary** | Simplified badge data for achievements lists |
| **BadgeShowcase** | UI component displaying a row of profile badges |
| **BadgeEarnedModal** | Modal dialog celebrating newly earned badges |
| **allow_insecure_public_ws** | Setting to allow ws:// (non-TLS) relays for public hosts |
| **Relay snapshot** | Initial relay state fetched on UI startup to recover missed events |
| **Relay state helpers** | `merge_relay_snapshot()` and `apply_relay_event()` for relay UI state |
| **Platform tag** | NOSTR listing tag `['platform', '<os>-<arch>']` declaring compatible runtime targets. Values follow Rust `std::env::consts::{OS, ARCH}`; no platform tags means compatible everywhere. |
| **PlatformInfo** | Struct returning host OS and arch from `std::env::consts`, used to offer an opt-in "My Platform" browse filter |
| **NIP-102** | Purchase Receipt protocol — defines kind-1020 events that prove a buyer purchased a specific listing |
| **Purchase receipt** | Kind 1020 event containing order id, listing coordinate, payment proof (bolt11 + preimage), and status (`paid`, `fulfilled`, etc.) |
| **PurchasesRepository** | SQLite-backed store for NIP-102 receipts, providing `is_owned()` checks for ownership enrichment |
| **Batch flusher** | Debounce pattern in `marketplace_loader.rs` that buffers incoming listings for 50ms before updating the reactive signal, reducing grid re-renders during relay streaming |
| **Incremental refresh** | Cache-aware fetch strategy using `latest_created_at()` cursor with overlap to only pull new/changed listings from relays |
| **Ownership enrichment** | The process of setting `is_owned=true` on listings by checking `PurchasesRepository` before presenting to the user |
| **NIP-94** | File metadata events (kind 1063) referenced via `["nip94", "<event-id>"]` tag for verifiable delivery metadata |
| **ADP** | Arcadestr Distribution Protocol: HTTP APIs plus NOSTR metadata for provisioning, upload, purchase confirmation, and authenticated game delivery |
| **ADP server announcement** | Kind-30403 event advertising an HTTP(S) ADP endpoint, optional name/contact, and supported protocol version |
| **Fulfillment authorization** | Coordinate-scoped kind-30406 lifecycle by which a developer authorizes a delegated fulfillment key to issue delivery credentials |
| **Fulfillment mode** | Publishing choice: `none` (buy-only), `direct` (developer signer), or `delegate` (provisioned operator signer) |
| **Download token** | Expiring ADP credential scoped by buyer pubkey, game coordinate, and server after purchase confirmation or campaign claim |
| **NWC** | Nostr Wallet Connect, used to connect a wallet and pay a Bolt11 invoice without exposing its connection secret to the UI response |
| **Install registry** | SQLite `installed_games` records for artifacts that passed SHA-256 verification |
| **Artifact quarantine** | Preservation of a hash-mismatched download as `.corrupt`, `.corrupt.1`, and so on, without recording an install |
| **Local account** | Direct-signing account whose nsec is encrypted by Arcadestr rather than held by a NIP-46 signer |
| **Acquisition policy** | Signed kind-30402 current-access policy: `Gated`, `Public`, or half-open `TimedAccess`; zero price has no authorization meaning |
| **Campaign** | Publisher-controlled append-only provisional claim policy; cancellation blocks later claims but preserves prior grants |
| **Campaign pointer** | Advisory listing tag referencing an immutable campaign root; authoritative discovery still falls back to publisher/coordinate queries |
| **Entitlement grant** | Buyer-specific append-only provisional durable credential issued by a publisher or properly authorized fulfillment key |
| **Durable acquisition record** | Account-scoped normalized purchase or promotion-claim history with active, disputed, refunded, revoked, or unverified status |
| **Device registry** | Local `installed_games` artifact inventory shared on a device; an install row is not proof of durable ownership |
| **Replaceable ordering** | Central invariant: later `created_at` wins; equal timestamps choose the lexicographically lower event ID |
| **Request generation** | Monotonic UI token paired with expected identity to reject stale asynchronous responses |
| **Request-scoped event** | Marketplace Tauri event named `marketplace-product-{id}` or `marketplace-complete-{id}` to isolate concurrent streams |
| **Publisher studio** | `PublishViewState` workflow for listing creation/editing, game management, and campaign lifecycle operations |
| **Noir theme** | OKLCH token-based visual system implemented by Tailwind configuration, global CSS, and `UI_V2_STYLES` |

### Platform tag examples

| OS | Architecture | Platform tag |
|----|--------------|--------------|
| `linux` | `x86_64` | `linux-x86_64` |
| `linux` | `aarch64` | `linux-aarch64` |
| `windows` | `x86_64` | `windows-x86_64` |
| `windows` | `aarch64` | `windows-aarch64` |
| `macos` | `x86_64` | `macos-x86_64` |
| `macos` | `aarch64` | `macos-aarch64` |

---

## The 5 Most Important Things to Understand

Before diving into the codebase, ensure you deeply understand these concepts:

### 1. **Dual-Target Architecture**
The same Leptos UI runs in both Tauri (desktop) and browser (web), but native ADP file selection, downloads, and install persistence are desktop-only. Authentication can be remote (NIP-46 desktop, NIP-07 web) or a locally encrypted nsec. The `app` crate uses conditional compilation (`#[cfg(target_arch = "wasm32")]`) for the bridge fallbacks.

### 2. **NIP-46 Fast Connection Flow**
The active `connect_bunker` command waits for signer approval and returns only after the profile is saved and both NIP-46 and shared `AuthState` are synchronized. The deferred `init_signer_session_fast` helper remains available but is not used by this command; QR login still uses polling.

### 3. **Relay Gossip (NIP-65)**
The app doesn't just connect to hardcoded relays. It fetches each user's relay list, builds a coverage map, and uses greedy set cover to select optimal relays. This is the key to efficient decentralized communication.

### 4. **Feature-Gated Core**
The `core` crate compiles differently for native (desktop) and WASM (web) targets. Native gets tokio, sqlx, and full NOSTR functionality. WASM gets stubs and relies on browser APIs. Understanding `#[cfg(feature = "native")]` guards is essential.

### 5. **Tauri IPC Pattern**
Frontend wrappers in `tauri_bridge.rs` call the low-level `tauri_invoke.rs` bridge, which invokes `window.__TAURI__.core.invoke()`. Commands are registered in `main.rs`; ADP command implementations live primarily in `desktop/src/adp_commands.rs`, while download orchestration remains in `main.rs`. Overlapping marketplace streams use request-scoped event names; other backend progress and completion return through fixed Tauri events.

---

## Recent Major Features (2026-07)

**ADP marketplace completion and Gate 5 delivery:**
- `core/src/adp_discovery.rs` discovers kind-30403 server announcements; the publish UI supports buy-only, direct, and delegated fulfillment with native file selection, SHA-256 hashing, provisioning, two-relay propagation confirmation, and sequential multi-server upload.
- `core/src/adp_client.rs` now streams token- or NIP-98-authenticated downloads through `HttpClient::download_to_path()` using an atomic temporary-file rename.
- `desktop/src/main.rs` refetches authoritative policy/server/hash/version tags, requires durable active-account ownership or explicit current public/timed access, downloads to a deterministic app-data path, and emits `download-progress`/`download-complete`.
- `desktop/src/install.rs` verifies the listing hash, preserves mismatches under unique `.corrupt*` paths, and records only successful installs. `LibraryView` renders those `installed_games` rows; extraction and execution remain out of scope.

**Encrypted local nsec accounts:**
- `login_with_nsec` creates an `AccountManager` record, encrypts the nsec, switches `AuthState` to a local signer, and clears active NIP-46 retry/client state.
- Account listing, switching, deletion, startup restoration, and logout now cover both NIP-46 and local accounts. The desktop master key is currently a protected app-data file rather than an OS-keyring secret.

### Game Detail View Consolidation
The GameDetail view was rewritten as a fully standalone component:
- **Standalone implementation**: No longer wraps `DetailView` — directly renders hero section, buy panel, metadata, specs, gallery, and seller profile
- **Buy flow**: LNURL invoice creation, copy/open-wallet actions, NWC payment or manual-preimage confirmation, and ADP purchase confirmation
- **Install flow**: Owned, explicitly public, and active timed-access listings call `install_game`; zero price alone remains gated. Button state tracks installing/completed and a scoped `download-complete` listener is cleaned up safely on unmount.
- **Seller profile**: Fetches publisher profile via `ProfileRow` on mount, caches in `ProfileStore`, displays about, NIP-05, lud16, website
- **Gradient hero**: Background image with `linear-gradient` overlay and metadata (publisher, release date, protocol)
- **Overflow-safe metadata**: Buy panel CSS (`min-width: 0; max-width: 100%; overflow-wrap: anywhere;`) protects against long values breaking layout
- **Debug badge preview**: `debug_badge_preview()` mock shown in debug builds after invoice creation

### Centralized Npub Fallback Display
Truncated npub formatting centralized into `npub_fallback_label()` at `app/src/models.rs:279`:
- Returns first 12 characters + `...` + last 8 characters of bech32 npub, e.g. `npub1vcq8nv3...6syj3d9l`
- Replaces 6 inline truncation sites across the UI (account selector, profile display, marketplace loader, login, shell, app root)
- Also used by `listing_publisher()` for stall-name-less games
- Skips truncation for npubs ≤ 24 characters

### Debug Relay Selection
New development workflow for testing with specific relays:
- **Three-tier configuration**: CLI args (`--relay`, `--block-discovery`, `--allow-discovery`) override env vars (`ARCADESTR_RELAYS`, `ARCADESTR_BLOCK_DISCOVERY`), which override `NetworkDiscoverySettings` JSON
- **`RelayManagerConfig.debug_relays`** / **`RelayManagerConfig.block_discovery`** : Runtime config fields that restrict the relay pool to specified relays and disable NIP-65 gossip discovery
- **`normalize_relay_urls()`**: New public validator in `relay_manager` — rejects non-websocket schemes, deduplicates, strips trailing slashes
- **`dispatch_ephemeral_reads_batch_with_policy()`**: Subscription dispatch that skips relay discovery when `block_discovery` is true

### Platform Ownership & Filtering
The codebase now supports platform-aware game publishing and browsing:
- **Platform tags**: Publishers declare OS/arch compatibility via `["platform", "<os>-<arch>"]` tags on kind 30402 events
- **PlatformInfo type**: `app/src/models.rs` — `PlatformInfo { os, arch }` with `tag()` helper for comparison
- **Platform filter**: Browse view auto-detects host platform and offers "My Platform" filter dropdown; auto-fetches more listings when active filter yields fewer than 50 results
- **Platform validation**: Comma-separated input in publish form with `parse_platform_tags()` — rejects whitespace and missing OS-arch separator
- **Desktop install command**: `install_game` ignores caller price and authorizes only from a freshly fetched, signed kind-30402 listing, cached token, durable receipt/grant ownership, or explicit current public/timed-access policy.

### NIP-102 Purchase Receipts
New purchase ownership system with NIP-102 kind-1020 receipts:
- **`core/src/purchases.rs`**: Full receipt parsing, validation, and persistence (698 lines)
- **`core/migrations/003_purchases.sql`**: `purchases` table with indexes on buyer, order, and coordinate
- **Parsing**: Validates event kind 1020, buyer p-tag, merchant p-tag, listing coordinate, and payment proof (bolt11 + preimage or zap receipt e-tag)
- **Ownership enrichment**: `enrich_listing_ownership()` uses the shared `OwnershipService`; `is_owned` is true for either a valid NIP-102 receipt or active Entitlement Grant.
- **Tauri commands**: `ingest_receipt` persists external receipts; `confirm_purchase` validates an ADP response and stores both receipt and token; `install_game` consumes ownership/token state
- **Bridge functions**: `invoke_ingest_receipt`, `invoke_confirm_purchase`, `invoke_install_game`, and `invoke_get_platform_info` in `app/src/tauri_bridge.rs`
- **Dependencies**: `lightning-invoice`, `sha2` for Bolt11 parsing and payment proof hashing

### Marketplace Incremental Refresh
Cache-aware streaming reduces relay load and startup time:
- **Cursor-based**: Uses `marketplace_cache.latest_created_at()` to fetch only events newer than the latest cached item
- **24h overlap**: `MARKETPLACE_REFRESH_OVERLAP_SECS = 86400` ensures updates to existing listings aren't missed
- **`fetch_nip99_listings_streaming_since`**: New streaming function accepting raw `since_secs` instead of `since_days`
- **`marketplace_refresh_since_secs`**: Helper that computes the effective since value, preferring cache cursor over user-specified window
- **Cache signature**: `listing_signature()` includes platforms, NIP-94, specs, and source event identity; SQL upsert rejects stale replaceable events with deterministic tie-breaking

### Marketplace Empty States
Graceful UI when no listings are found:
- **Browse empty state**: `show_browse_empty_state()` in `browse_games.rs` — shown after loading completes with no platform-matching results
- **Store front empty state**: `show_store_front_empty_state()` in `store_front.rs` — shown when loading finishes without errors and zero listings
- Both states show contextual messages guiding the user

### Profile View Refactor
Profile view now shares marketplace state instead of fetching independently:
- Uses `use_marketplace_listings_with_limit(50)` from `marketplace_loader.rs`
- Filters to current user's npub via derived signal
- Eliminates redundant relay fetch on profile page load

### Relay Manager Fix: Premature Connected Events
Relay manager no longer emits `RelayConnectionEvent::Connected` before the monitor confirms the connection:
- Connected events only fire after the monitor tick validates the relay is actually responding
- Fixes race condition where UI shows a relay as connected while it's still handshaking

### VS Code Debug Configuration
New `.vscode/` workspace with full debug setup:
- **launch.json**: Backend launch, attach-to-process, and launch-existing-binary LLDB configurations
- **tasks.json**: Build tasks that compile before debugging
- **settings.json**: Workspace-specific Rust analyzer and editor settings

### Debug NIP-99 Raw JSON Panel
New debug-only UI component in `detail.rs` for inspecting NIP-99 events:
- Collapsible panel with pretty-printed raw event JSON
- Visible only in `#[cfg(debug_assertions)]` builds
- Fallback to reconstructed payload when no raw JSON is available

### NIP-103 Entitlements And ADP Campaigns
- **Provisional kinds**: `core/src/adp_protocol.rs` centralizes Entitlement Grant kind `1030`, campaign kind `1031`, and shared tags. These are development allocations; kind `1021` is never used.
- **Protocol validation**: `campaign.rs`, `authorization.rs`, and `entitlements.rs` verify signatures, transition-specific authority, valid-prefix chains, half-open windows, prospective cancellation, and fail-closed valid forks. Delegated grants reference the exact developer-signed kind `30406` root with `authorization`; historical verification does not depend on current listing membership or operator attestation state.
- **Listing policy**: NIP-99 parsing supports explicit `acquisition: public`, `acquisition: timed-access <starts> <ends>`, and repeatable advisory `campaign` pointers. Missing or malformed policy is gated; zero price has no authorization meaning.
- **Discovery**: `CampaignDiscoveryService` tries immutable pointer IDs and relay hints, always performs the `#a` fallback, fetches complete publisher `#d` chains, and reports invalid chains separately.
- **Migration 7**: `core/migrations/007_entitlements.sql` adds append-only `entitlement_events` history indexed by buyer/game, grant ID, and predecessor.
- **Ownership**: `OwnershipService` resolves `PurchaseReceipt | EntitlementGrant | None`. Public and timed access are current access modes, not stored ownership.
- **ADP route**: `AdpClient::entitlement_claim` calls exact `POST /entitlement/claim` with NIP-98 and typed campaign, coordinate, grant, distribution, and protocol errors.
- **Commands/bridges**: Discovery, summary, publish, pointer-repair, claim, ownership, and purchase-history commands cover buyer and publisher flows; campaign publication reports pointer-update partial success explicitly.
- **Publisher authority**: Campaign create/update/cancel requires the active publisher identity signer. A fulfillment key may issue grants only when both the historical listing delegation and referenced kind `30406` authorization lifecycle were active at issuance; either source alone fails closed. Direct publisher grants require neither proof. Fulfillment keys cannot control campaigns or revoke grants.
- **Install trust boundary**: The current signed listing controls server, hash, version, and current access policy. Historical validated grants/receipts remain durable after price, policy, version, or campaign cancellation changes, and cached tokens are buyer-scoped.

---
