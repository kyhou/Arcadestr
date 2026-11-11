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
2. **Publisher Studio**: Create or replacement-edit NIP-99 kind-30402 listings, inspect selected ZIP/tar archives without exposing native paths, author signed package/launch profiles and per-platform defaults, publish rich kind-30407 Store Pages with Blossom media, provision ADP fulfillment, upload launch-ready archives, and retry typed partial publications without losing current form edits
3. **Acquire Games**: Buy paid games over Lightning, claim active claim-and-keep promotions, or use explicitly public/half-open timed access; zero price alone never grants access
4. **Ownership, History & Install**: Track durable NIP-102 receipts and provisional NIP-103-style grants, resolve signed delivery metadata into a host-specific install plan, retain authenticated packages, verify digest and format, capability-extract into an atomic installation tree, persist immutable launch/manifest snapshots, and expose reinstall, uninstall, and interrupted-operation recovery separately from ownership
5. **Authentication**: NIP-07 browser extensions, NIP-46 remote signers, and encrypted local nsec accounts
6. **Zap Payments**: NIP-57 Lightning zaps for game purchases
7. **Marketplace Cache**: Persistent SQLite storage for offline browsing
8. **Real-time Relay Status**: Live connection indicators with latency metrics
9. **Achievements & Badges**: NIP-58 badge display on profiles, earned badge tracking, and achievement showcase
10. **Platform Filtering**: Browse games filtered by host platform (OS/arch) with automatic platform detection
11. **Marketplace Empty States**: Graceful UI when no listings are found, with distinct messages for browse vs store front
12. **Account & Network Settings**: Switch/remove accounts, reconnect signers and relays, export local keys, and inspect non-sensitive diagnostics
13. **Truthful Capability States**: Unsupported web, community, profile-editing, backup, appearance, and install-lifecycle actions are hidden or presented as unavailable rather than fabricated
14. **Store Pages**: Publisher-authored kind-30407 presentation pages (schema `io.arcadestr.store-page`, version 1) with sanitized Markdown description, feature sections, media gallery, requirements, languages, accessibility, and links, rendered against game listings with cache-first discovery and reciprocal listing pointers
15. **Blossom Media Uploads**: Account-scoped Blossom server settings (add/remove/reorder/preferred), health probes, upload-authorization signing, and sequential multi-server media uploads with SHA-256 verification and progress events
16. **Account Library Saves**: Bookmark games into an account-scoped `library_games` list without downloading, reconciled against device installs in the Library view
17. **Direct Launcher**: Play available `game` profiles directly without a shell. Selection precedence is an explicit profile, then saved per-game preference, publisher default for the canonical host target, and finally publisher order; non-game profiles remain installed for diagnostics but are not Play candidates

### Desktop vs Web Target

Arcadestr is a **dual-target application**:

| Target | Technology | Authentication | Use Case |
|--------|-----------|----------------|----------|
| **Desktop** | Tauri v2 + Leptos | NIP-46 or encrypted local nsec | Native archive inspection/publication, verified retained downloads and extraction, installation lifecycle/recovery, and shell-free launching |
| **Web** | Leptos (WASM) | NIP-07 or browser-encrypted nsec | Browser marketplace access without native installation |

Both targets share UI components from the `app` crate. The desktop target delegates NOSTR, ADP, opaque archive handles, native extraction, lifecycle persistence, and process launch to Rust commands. The web target uses browser APIs for NIP-07 and browser-encrypted local signing and returns explicit desktop-only fallbacks for archive, install, retained-package, recovery, and launcher operations.

---

## 2. Technology Stack

| Name | Version | Role in Project | Where it Appears |
|------|---------|-----------------|------------------|
| **Rust** | 1.89+ | Workspace MSRV and primary language for all five crates | All `.rs` files, `Cargo.toml`, `.github/workflows/msrv.yml` |
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
| **reqwest** | 0.12 | HTTP, multipart upload, streamed ADP downloads, and Blossom media uploads | `core/src/http_client.rs`, `core/src/adp_client.rs`, `desktop/src/blossom_upload.rs` |
| **lightning-invoice** | 0.32 | Bolt11 invoice parsing for purchase receipts | `core/src/purchases.rs` |
| **sha2** | 0.10 | Payment proofs, archive/package verification, frontend fingerprints, retained downloads, and deterministic identities | `core/`, `desktop/`, `build-support/frontend_fingerprint.rs`, `xtask/` |
| **pulldown-cmark** | 0.13 | Store Page Markdown parsing and HTML sanitization (blocklist + limit enforced) | `core/src/store_page_content_policy.rs`, `core/src/store_page.rs` |
| **time** | 0.3 | WASM-safe date/time handling for the reusable date/time picker | `app/Cargo.toml`, `app/src/components/date_time_picker.rs` |
| **uuid** | 1 | Backend-minted game IDs plus opaque archive, installation, retained-package, launch-session, Blossom, and operation identifiers | `core/`, `app/`, `desktop/` |
| **zip / tar / zstd / flate2** | 8.6.0 / 0.4.46 / 0.13.3 / 1.1.9 | Bounded ZIP, tar.zst, and tar.gz inspection, verification, and extraction | `core/src/archive_inspection/`, `core/src/package_extraction.rs` |
| **cap-std / cap-primitives** | 4.0.3 | Capability-rooted extraction and no-follow managed-path operations | `core/src/package_extraction.rs`, `desktop/src/install.rs` |
| **infer** | 0.22 | Advisory media magic-byte detection before Blossom upload | `desktop/src/blossom_upload.rs` |
| **imagesize** | 0.15 | Image dimension extraction from the bounded inspection prefix | `desktop/src/blossom_upload.rs` |
| **tokio-util** | 0.7 | Cancellation tokens for interrupted Blossom uploads | `desktop/src/blossom_upload.rs` |
| **futures-util** | 0.3 | Concurrent Blossom server uploads and stream handling | `desktop/src/blossom_upload.rs`, `desktop/src/blossom_commands.rs` |
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
| **xtask** | workspace crate | Reproducible Trunk build plus atomic `FrontendBuildStamp` generation | `xtask/`, `build-support/frontend_fingerprint.rs` |
| **tracing** | 0.1 | Structured logging | Throughout codebase |
| **thiserror** | 1.0 | Error type derivation | `core/` crate |

### Why Tauri over Pure Web or Pure Native?

The choice of Tauri v2 is driven by several factors evident in the codebase:

1. **Security**: NIP-46 keeps remote keys in signer apps, while local-account nsecs are handled by Arcadestr, encrypted with AES-256-GCM, and stored in `accounts.db`. The desktop master key is a mode-`0600` file in the app data directory; it is not currently backed by the OS keyring.

2. **Native Performance**: The `core` and `desktop` crates use SQLite, async networking, bounded native archive readers, capability extraction, filesystem locks, and direct process spawning—capabilities unavailable to the standalone browser target.

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
├── Cargo.toml              # Workspace manifest - defines all 5 crates and Rust 1.89 MSRV
├── Cargo.lock              # Dependency lock file
├── .gitignore              # Generated output, local databases, caches, and environment exclusions
├── README.md               # Project overview and quickstart
├── CLAUDE.md               # Development guidelines for Claude Code
├── AGENTS.md               # Agent-specific build/test commands
├── RUST_GUIDELINES.md      # Microsoft Rust Guidelines reference
├── ADP-01.md               # ADP protocol specification
├── ADP-01-amendment-free-acquisitions.md # Explicit access/campaign amendment
├── ADP-01-amendment-launch-profiles.md # Signed package, platform, launch profile/default amendment
├── NIP-102.md              # Purchase receipt specification used by ownership
├── NIP-103.md              # Experimental entitlement/campaign proposal notes
├── Arcadestr_UI_Screen_Inventory_Revised.md # Maintained acquisition-aware inventory
├── docs/README.md           # Documentation status and repository contents policy
├── docs/archive/            # Original inventory, client integration plans, NIP-46 report
├── opencode.json           # Repository-local OpenCode MCP/watch configuration
├── COMMANDS.md             # Available CLI commands
├── test_nostrconnect.sh    # Test script for NIP-46
├── build-support/
│   └── frontend_fingerprint.rs # Shared source/dist fingerprint and FrontendBuildStamp logic
├── xtask/                  # BINARY: reproducible project automation
│   ├── Cargo.toml
│   └── src/main.rs         # `frontend-build [--release]`: Trunk build + atomic stamp
├── .github/workflows/msrv.yml # Rust 1.89 native, Windows, and WASM checks
├── scripts/clear_arcadestr_data.sh # Comprehensive local DB/files/cache/keyring reset
├── tests/fixtures/
│   ├── a3-launch-profiles/ # Valid and adversarial archive/extraction fixtures
│   └── launch/fixture.rs   # Launcher process fixture
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
│   │   ├── adp_settlement.rs # Pure experimental fee/terms/quote/leg/recovery invariants
│   │   ├── ownership.rs      # Unified receipt/grant ownership and durable acquisition history
│   │   ├── campaign.rs       # Implementation-local provisional campaign chain validation
│   │   ├── campaign_discovery.rs # Pointer plus authoritative campaign discovery
│   │   ├── entitlements.rs   # Implementation-local provisional grant parsing and validation
│   │   ├── entitlements_repository.rs # Validated grant persistence and history
│   │   ├── authorization.rs  # Kind-30406 fulfillment authorization lifecycles
│   │   ├── adp_client.rs     # ADP HTTP client: provision, upload, purchase, download
│   │   ├── adp_discovery.rs  # Kind-30403 ADP server discovery and parsing
│   │   ├── adp_publish.rs    # Validated kind-30402 listing and kind-30406 authorization builders
│   │   ├── adp_publish_contract.rs # Typed publication/edit/partial-success contracts
│   │   ├── archive_inspection_contract.rs # Opaque selection/query IPC contracts and typed failures
│   │   ├── archive_inspection/ # Bounded native archive indexing
│   │   │   ├── mod.rs        # Inspection facade
│   │   │   ├── model.rs      # Safe indexed archive model
│   │   │   ├── detect.rs     # Content-based package detection
│   │   │   ├── limits.rs     # Entry/path/size/ratio limits
│   │   │   ├── candidates.rs # Launch path candidate discovery
│   │   │   ├── zip.rs        # ZIP inspection
│   │   │   └── tar.rs        # tar.gz/tar.zst inspection
│   │   ├── distribution/     # Signed package/launch resolution boundary
│   │   │   ├── mod.rs
│   │   │   ├── domain.rs     # ResolvedInstallPlan, artifacts, profiles, package formats
│   │   │   ├── listing.rs    # Signed tags -> host-specific install plan
│   │   │   ├── platform.rs   # Canonical OS/architecture vocabulary
│   │   │   ├── validation.rs # Path/profile/argument validation
│   │   │   └── errors.rs     # Typed resolution failures
│   │   ├── install_contract.rs # Typed installation command errors
│   │   ├── launch_contract.rs # Launch options/configuration/session/outcome/errors
│   │   ├── package_extraction.rs # Verified bounded capability extraction and manifest
│   │   ├── adp_storage.rs    # Provisioning, token, installation/snapshot, retained-package, launch-preference, and library repositories
│   │   ├── settlement_repository.rs # Account-scoped resumable quote/leg/attempt persistence
│   │   ├── settlement_recovery.rs # Account/session fencing and exclusive recovery leases
│   │   ├── durable_payments.rs # Journaled NWC dispatch and proof reconciliation
│   │   ├── purchase_contract.rs # Plan review, explicit approval, network/fee policy wire types
│   │   ├── purchase_execution.rs # Verified plan approvals, admissions, and guarded leg execution
│   │   ├── receipt_portability.rs # Receipt import, relay discovery, and authority verification
│   │   ├── receipt_sync_contract.rs # Buyer-scoped receipt sync/import request and report types
│   │   ├── entitlement_reconciliation.rs # Revalidate known grant chains and update projections
│   │   ├── entitlement_sync_contract.rs # Expected-account reconciliation request/report
│   │   ├── publisher_recovery.rs # Draft revisions and durable publisher operation checkpoints
│   │   ├── publisher_management.rs # Credential revocation and distribution-evidence rules
│   │   ├── blossom.rs        # Blossom protocol types: server origins, upload authorizations, blob descriptors, MIME policy
│   │   ├── store_page.rs     # Store Page (kind 30407) schema, validation, sanitized rendering, and listing pointers
│   │   ├── store_page_content_policy.rs # Store Page Markdown sanitizer and content-size limits
│   │   ├── store_page_discovery.rs # Cache-first Store Page lookup, enrichment, and relay fallback
│   │   ├── store_page_repository.rs # Validated Store Page and listing-event persistence
│   │   ├── file_hash.rs      # Streaming SHA-256 file hashing
│   │   ├── hash_validation.rs # Shared exact SHA-256 hex validation
│   │   ├── replaceable_event.rs # Central timestamp/event-ID replacement ordering
│   │   ├── http_client.rs    # Shared HTTP abstraction and private retained-package downloads
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
│   │   ├── 001_initial_schema.sql # Initial database schema
│   │   ├── 005_achievements.sql # NIP-58 badge tables
│   │   ├── 006_purchases.sql # NIP-102 purchase receipts
│   │   ├── 007_adp_provisioning.sql # Delegated fulfillment records
│   │   ├── 008_download_tokens.sql # Buyer/server-scoped credentials
│   │   ├── 009_installations.sql # Installations, profiles, manifests, retained packages
│   │   ├── 010_entitlements.sql # Validated grant events
│   │   ├── 011_authorization_roots.sql # Kind-30406 audit roots
│   │   ├── 012_library_games.sql # Account library
│   │   ├── 013_store_pages.sql # Validated Store Pages
│   │   ├── 014_store_page_listing_events.sql # Signed reciprocal listings
│   │   ├── 015_blossom_server_settings.sql # Account Blossom policy
│   │   ├── 016_launch_profile_preferences.sql # Per-coordinate launcher preference
│   │   ├── 017_split_settlement.sql # Account-scoped quotes, legs, invoices, and recovery
│   │   ├── 018_payment_durability.sql # Durable NWC payment journal and intent checkpoints
│   │   ├── 019_settlement_leg_projection.sql # Repair derived leg state from persisted evidence
│   │   ├── 020_client_refund_recovery.sql # Encrypted refund invoices, proofs, and local recovery
│   │   ├── 021_purchase_execution.sql # Explicit plan approvals and irreversible dispatch admissions
│   │   ├── 022_purchase_verification.sql # Signed listing/terms sources for plan review
│   │   ├── 023_receipt_portability.sql # Buyer-scoped receipt discovery and chain verification
│   │   ├── 024_publisher_recovery.sql # Revisioned publisher drafts and operation journals
│   │   ├── 025_marketplace_campaign_pointers.sql # Advisory listing campaign-pointer projection
│   │   └── 026_quote_initiations.sql # Durable purchase IDs and quote-request commitments
│   └── tests/
│       ├── integration.rs          # Core integration coverage
│       ├── integration_nip05.rs    # NIP-05 HTTP integration tests
│       ├── receipt_protocol.rs     # NIP-102 receipt parsing/persistence tests
│       ├── adp_entitlement_protocol.rs # Campaign/grant adversarial protocol tests
│       ├── adp_publish_protocol.rs # ADP listing/authorization publish contract tests
│       ├── archive_inspection_zip.rs # ZIP limits, hazards, and pagination
│       ├── archive_inspection_tar.rs # tar.gz/tar.zst limits and hazards
│       ├── relay_fetch_outcomes.rs # EOSE, timeout, failure, and retained partial results
│       ├── authorization.rs        # Delegated fulfillment authority tests
│       ├── campaign_discovery.rs   # Pointer/fallback campaign discovery tests
│       └── entitlements_repository.rs # Validated grant persistence tests
│
├── app/                    # LIBRARY: Leptos UI components (shared)
│   ├── Cargo.toml          # Leptos, wasm-bindgen dependencies
│   └── src/
│       ├── lib.rs          # Main app component, auth context, styles
│       ├── campaign_management.rs # Publisher campaign validation, requests, and pointer repair
│       ├── publish_state.rs # Archive/query generations, launch drafts, authoritative edit and retry state
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
│       │   │   ├── aria.rs # Enumerated ARIA state serialization helpers
│       │   │   ├── blossom_media_upload.rs # Blossom media selection/upload UI
│       │   │   ├── button.rs # Button/IconButton primitives and variants
│       │   │   ├── dialog.rs # Canonical modal dialog primitive (focus, Escape, close policy)
│       │   │   ├── feedback.rs # Inline status/feedback presentation
│       │   │   ├── game_artwork.rs # Artwork loading/missing/failed states
│       │   │   ├── game_card.rs # Truthful access/campaign/action card presentation
│       │   │   ├── installation_launcher.rs # Play/properties, profile preference, and session UI
│       │   │   ├── logo.rs # Arcadestr logo mark + wordmark
│       │   │   ├── modal_background.rs # Modal scrim/backdrop
│       │   │   ├── nav_item.rs   # Navigation item component
│       │   │   ├── page_container.rs # PageContainer + ClippedPanel layout primitives
│       │   │   ├── page_header.rs # Shared page heading component
│       │   │   ├── status_chip.rs # StatusChip variants (owned, installed, gated, etc.)
│       │   │   ├── store_page_detail.rs # Store Page rich detail rendering
│       │   │   ├── tabs.rs # Accessible tab group primitive
│       │   │   ├── topbar.rs     # Top navigation bar
│       │   │   ├── transient.rs # Transient menu/popover dismissal coordination
│       │   │   └── unsaved_changes_dialog.rs # Unsaved-navigation guard dialog
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
│       │       ├── store_front.rs    # Main store front view
│       │       └── store_page_publish.rs # Publisher Store Page editor (8-tab studio)
│       ├── components/     # UI components
│       │   ├── mod.rs      # Component exports
│       │   ├── account_selector.rs   # Login/account switching UI
│       │   ├── backup_manager.rs     # Backup/restore UI
│       │   ├── badge_earned_modal.rs # Badge earned celebration modal
│       │   ├── badge_showcase.rs     # Profile badge display component
│       │   ├── browse.rs             # Game listing grid
│       │   ├── date_time_picker.rs   # Reusable date/time picker (publish + store page)
│       │   ├── publish_launch.rs # Archive safety review and launch-profile editor
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
│   ├── windows-app-manifest.xml # Windows long-path awareness
│   ├── icons/              # PNG/ICO bundle icons
│   ├── src/
│   │   ├── main.rs         # Entry point, Tauri setup, commands
│   │   ├── adp_commands.rs # ADP discovery, publishing, purchase, and wallet commands
│   │   ├── publisher_commands.rs # Draft persistence and explicitly confirmed operation resume
│   │   ├── publisher_management.rs # Managed distribution and credential lifecycle IPC
│   │   ├── settlement_ui.rs # Pending settlement/refund reconciliation and recipient history
│   │   ├── receipt_commands.rs # Receipt sync/import and ownership update events
│   │   ├── entitlement_commands.rs # Known-grant reconciliation for the active buyer
│   │   ├── archive_commands.rs # Opaque archive selection, inspection, query, hash, discard IPC
│   │   ├── archive_selection.rs # Managed staged selections and publication leases
│   │   ├── store_page_commands.rs # Store Page enrichment, editor, validation, and publishing commands
│   │   ├── blossom_commands.rs # Blossom media selection, upload, and server-settings commands
│   │   ├── blossom_settings.rs # Account-scoped Blossom server settings persistence
│   │   ├── blossom_upload.rs   # Native Blossom upload service (inspection, auth, streaming)
│   │   ├── install.rs      # Capability extraction, atomic placement, lifecycle recovery, safe deletion
│   │   ├── launcher.rs     # Shell-free profile resolution, spawn, supervision, and status
│   │   ├── marketplace_stream.rs # Full-success completion gating
│   │   ├── startup_runtime.rs # Startup async bridge and recovery support
│   │   ├── nip46_commands.rs # NIP-46 specific Tauri commands
│   │   └── command_contracts.rs # Pure command logic for testability
│   ├── examples/
│   │   └── blossom_fixture.rs # Loopback Blossom upload fixture (axum test server)
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
│  │  │  • publish_adp_listing_with_contract() • archive/install IPC │  │   │
│  │  │  • lifecycle/reinstall   • launch_game()    • fetch_profile() │  │   │
│  │  └────────────────────────┬────────────────────────────────────────┘  │   │
│  │                           │                                           │   │
│  │  ┌────────────────────────▼────────────────────────────────────────┐  │   │
│  │  │                    core crate (arcadestr_core)                 │  │   │
│  │  │  ┌────────────┐ ┌────────────┐ ┌────────────┐ ┌────────────┐ │  │   │
│  │  │  │NostrClient │ │distribution│ │install DB  │ │ extraction │ │  │   │
│  │  │  │(nostr-sdk) │ │plan/NIP-98 │ │snapshots   │ │ + launcher │ │  │   │
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
│  Marketplace streaming, native archive inspection/publication, installation,│
│  extraction, recovery, and launching are not implemented directly in the    │
│  standalone web target.                                                      │
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
    existing_d_tag, // edit-only; authoritative event must agree
    pending_d_tag,  // backend-minted coordinate resumed after a failed first attempt
    title,
    description,
    price_sats,
    lud16,
    tags,
    images,
    fulfillment_mode, // None, Direct, or Delegate
    operator_url,
    servers,
    file_path: None, // legacy native paths are rejected
    selected_build, // opaque BuildSelectionRef issued by ArchiveSelectionService
    existing_file_hash,
    existing fulfillment authorization references,
    version,
    acquisition, // Gated, Public, or half-open TimedAccess
    platforms,
    campaigns,
    nip94_event_id,
};
let extension = PublishAdpListingRequestExtension {
    launch_tier: Some(AdpLaunchTierIntent {
        package_format,
        platforms: canonical_targets,
        launch_profiles,
        publisher_defaults: vec![PublisherLaunchDefault { profile_id, targets }],
    }),
    acknowledged_removed_profile_ids,
};
spawn_local(async move {
    let result = invoke_publish_adp_listing_with_contract(request, extension).await;
});
```

**Step 2: Tauri Bridge (WASM)**
```rust
// app/src/tauri_bridge.rs
pub async fn invoke_publish_adp_listing_with_contract(
    request: PublishAdpListingRequest,
    extension: PublishAdpListingRequestExtension,
) -> Result<PublishAdpListingResult, PublishAdpListingError> {
    tauri_invoke::invoke(
        "publish_adp_listing_with_contract",
        serde_json::json!({ "request": request, "extension": extension }),
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
pub async fn publish_adp_listing_with_contract<R: tauri::Runtime>(
    request: PublishAdpListingRequest,
    extension: PublishAdpListingRequestExtension,
    app: tauri::AppHandle<R>,
    state: tauri::State<'_, AppState>,
    signer_state: tauri::State<'_, Arc<Mutex<AppSignerState>>>,
    archive_service: tauri::State<'_, ArchiveSelectionService>,
) -> Result<PublishAdpListingResult, PublishAdpListingError> {
    // Acquire a publication lease for the opaque build; validate authoritative
    // edit metadata and profile removals; preflight every server's capacity;
    // then sign/publish once and upload from the lease.
    /* ... */
}
```

**Step 5: Core Business Logic**
```rust
// core/src/adp_publish.rs
pub fn build_adp_listing_event_builder(
    input: AdpListingPublishInput<'_>,
) -> Result<EventBuilder, AdpPublishError> {
    // Emits validated identity, acquisition, fulfillment, package, canonical
    // platform, launch profile, target-scoped default, and preserved metadata.
    /* ... */
}
```

**Step 6: Native publication and network effects**
- `ArchiveSelectionService` stages a private copy, returns only canonical UUID handles, inspects and hashes it, and grants a `PublicationLease`. The lease blocks discard/expiry, rechecks the file identity and digest, and is consumed only after the publish attempt finishes.
- New publications mint one UUID v4 before coordinate-dependent work. `pending_d_tag` resumes only a coordinate this app minted and relays confirm is not already live; an occupied or unconfirmed pending coordinate is never blindly reused.
- Edits load and signature/filter-verify the authoritative listing. Its identity, fulfillment metadata, package/launch tier, and profile removals are authoritative; unmanaged tags survive, while current form-owned pricing, policy, campaigns, platforms, package, launch, and defaults replace their prior values.
- Artifact reuse is allowed only for one authoritative hash with no newly added server. Any launch tier requires a resolved file hash. A legacy `file_path` or unleased replacement archive is rejected.
- For a new upload, `preflight_upload_capacity` checks **all** selected servers' `max_upload_bytes` before signer interaction, provisioning, authorization, relay publication, or upload. Legacy servers with no declared maximum remain usable.
- Delegated fulfillment validates or repairs kind-30406 authority. The final `AdpListingPublishInput` is validated, signed once, broadcast, and confirmed on at least two relays before the leased archive uploads sequentially with NIP-98.
- Uploads remain non-transactional. `PublishAdpListingError::PartialPublication { cause, successful_servers }` records exact progress. Retry state merges those servers with the current form and authoritative partial listing: user changes win for managed fields, unknown tags survive, and the same generated coordinate/hash is reused without duplicating completed uploads.

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

The native delivery path is deliberately deeper than a download callback:

```text
signed kind-30402 listing
  -> signature/filter verification + canonical-host resolve_install_plan()
  -> ResolvedInstallPlan { artifact, launch_profiles, publisher default, event JSON }
  -> retained private download (cached token from any signed server, otherwise NIP-98 on first)
  -> verify SHA-256 + declared PackageFormat
  -> bounded capability extraction into a reserved staging root
  -> atomic placement under app-data/installations/<installation-id>
  -> one transaction commits InstallationRecord + profile availability + exact manifest
  -> InstallationSnapshot drives Library and InstallationLauncher
  -> direct shell-free spawn; in-memory LaunchSession supervision
```

The retained package is independent of the installed tree, enabling offline reinstall. Startup recovery locks stale `Installing` rows, records whether the final root is present or absent, and exposes explicit cleanup/resume actions instead of guessing. Platform-less listings can remain visible during browsing, but resolving a launch tier requires canonical package/platform metadata and an exact host match.

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
        adp_commands::publish_adp_listing_with_contract,
        archive_commands::inspect_archive,
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

This inventory covers the handlers registered in `desktop/src/main.rs`'s
`tauri::generate_handler!`. Parameters omit Tauri-injected state, app handles,
and windows; Rust parameter names are shown (the JavaScript bridge uses Tauri's
camelCase argument convention). Return types show the successful value unless
an explicit `Result` documents a typed error contract. Other fallible commands
return `Result<T, String>`; `get_platform_info` returns its value directly.
`CoreGameListing` is the desktop alias for the core listing model. Types shown
without module qualification are defined or imported in the named source file.

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
| `fetch_listings` | `main.rs` | `limit: usize` | `Vec<CoreGameListing>` | Fetches recent listings |
| `fetch_listing_by_id` | `main.rs` | `publisher_npub: String`, `listing_id: String` | `CoreGameListing` | Fetches specific listing |
| `fetch_marketplace` | `main.rs` | `limit: usize`, `since_days: Option<u64>`, `filter: Option<MarketplaceFilter>` | `Vec<AppGameListing>` | Fetches NIP-99 marketplace listings with optional filtering and ownership enrichment |
| `fetch_profile` | `main.rs` | `npub: String`, `_additional_relays: Option<Vec<String>>` | `UserProfile` | Fetches NIP-01 metadata using relay discovery; the reserved additional-relays argument is currently ignored |
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
| `fetch_and_save_user_profile` | `main.rs` | None | `UserProfile` | Fetch and save current user's profile |
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
| `install_game` | `main.rs` | `listing: AppGameListing` | `Result<(), InstallCommandError>` | Refetches and verifies the signed listing, resolves the canonical-host plan, downloads/retains and verifies the package, capability-extracts it, atomically places it, and commits the installation snapshot |
| `get_installed_games` | `main.rs` | None | `Vec<InstalledGame>` | Lists only ready installs with installation ID/root, package digest/format, retained-package and launchable flags, state/recovery data, and timestamp |
| `list_installation_lifecycle_issues` | `main.rs` | None | `Vec<InstallationLifecycleIssue>` | Lists non-ready installing/failed/uninstalling records for recovery UI |
| `retry_installation_recovery` | `main.rs` | None | `RecoveryReport` | Locks and classifies interrupted installations without deleting an uncertain final tree |
| `list_retained_packages` | `main.rs` | None | `Vec<RetainedPackageDto>` | Lists coordinate-keyed verified packages and their pinned reinstall snapshot identity |
| `get_uninstall_sizes` | `main.rs` | `installation_id: String` | `UninstallSizesDto` | Measures the no-follow install tree and optional retained package before confirmation |
| `uninstall_game` | `main.rs` | `installation_id: String`, `retain_package: bool` | `()` | Transitions ready → uninstalling, removes the locked managed tree, and optionally deletes the retained package |
| `resume_uninstall` | `main.rs` | `installation_id: String` | `()` | Safely resumes an interrupted uninstall while retaining the package |
| `remove_failed_installation` | `main.rs` | `installation_id: String` | `()` | Removes a failed row/tree according to its persisted recovery discriminator |
| `delete_retained_package` | `main.rs` | `game_coordinate: String`, `expected_package_id: String` | `()` | Deletes the exact retained package only if its optimistic identity still matches |
| `reinstall_retained_package` | `main.rs` | `game_coordinate: String`, `expected_package_id: String` | `()` | Re-verifies and extracts the pinned package snapshot offline, without account or relay access |
| `get_launch_options` | `launcher.rs` | `installation_id: String` | `Result<LaunchConfiguration, LaunchError>` | Returns persisted options plus saved/publisher preference status, effective profile, and fallback reasons |
| `set_launch_profile_preference` | `launcher.rs` | `installation_id: String`, `profile_id: Option<String>`, `preference_updated_at: i64` | `Result<LaunchConfiguration, LaunchError>` | Sets or clears a coordinate-scoped available game-profile preference under the installation lock |
| `launch_game` | `launcher.rs` | `installation_id: String`, `profile_id: Option<String>` | `Result<LaunchSession, LaunchError>` | Revalidates the snapshot/root and directly spawns an available game profile without a shell |
| `get_launch_status` | `launcher.rs` | `installation_id: String` | `Result<Option<LaunchSession>, LaunchError>` | Returns this client session's latest supervised launch status |
| `ingest_receipt` | `main.rs` | `raw_event_json: String` | `()` | Parses and persists a NIP-102 kind-1020 purchase receipt |
| `check_adp_server` | `adp_commands.rs` | `server_url: String` | `AdpServerInfo` | Fetches and decodes `/.well-known/adp` |
| `discover_adp_servers` | `adp_commands.rs` | None | `Vec<AdpServerAnnouncement>` | Discovers and deduplicates kind-30403 server announcements from connected relays |
| `resolve_adp_operator` | `adp_commands.rs` | `request: ResolveAdpOperatorRequest` | `Option<String>` | Resolves exactly one locally known operator for a publisher/key/scope |
| `discover_campaigns` | `adp_commands.rs` | `request: DiscoverCampaignsRequest` | `Vec<DiscoveredCampaign>` | Resolves campaign chains, including tip/predecessor IDs and lifecycle state |
| `discover_campaign_summaries` | `adp_commands.rs` | `request: DiscoverCampaignSummariesRequest` | `Vec<CampaignSummary>` | Batch-counts active/upcoming campaigns for a publisher's listings |
| `publish_campaign` | `adp_commands.rs` | `request: PublishCampaignRequest` | `PublishCampaignResponse` | Publishes create/update/cancel events; listing-pointer failure is typed partial success |
| `update_campaign_pointer` | `adp_commands.rs` | `request: UpdateCampaignPointerRequest` | `String` | Adds/removes one campaign pointer on the current replacement listing |
| `claim_entitlement` | `adp_commands.rs` | `request: ClaimEntitlementRequest` | `ClaimEntitlementResponse` | Claims a campaign grant, validates/persists it, and caches a buyer-scoped token |
| `select_archive_build` | `archive_commands.rs` | `request: SelectArchiveRequest` | `Result<SelectArchiveResponse, ArchiveSelectionError>` | Copies a picked archive into private managed staging and returns an opaque account-bound handle; cancellation preserves the prior UI selection |
| `inspect_archive` | `archive_commands.rs` | `request: InspectArchiveRequest` | `Result<InspectArchiveResponse, ArchiveSelectionError>` | Content-detects and indexes a staged archive under strict path/count/size/ratio limits |
| `hash_selected_archive` | `archive_commands.rs` | `request: HashSelectedArchiveRequest` | `Result<HashSelectedArchiveResponse, ArchiveSelectionError>` | Hashes the staged identity and returns a `BuildSelectionRef` suitable for publication |
| `query_archive_files` | `archive_commands.rs` | `request: QueryArchiveFilesRequest` | `Result<QueryArchiveFilesResponse, ArchiveSelectionError>` | Pages safe file choices without returning host paths |
| `query_archive_working_directories` | `archive_commands.rs` | `request: QueryArchiveWorkingDirectoriesRequest` | `Result<QueryArchiveWorkingDirectoriesResponse, ArchiveSelectionError>` | Pages safe working-directory choices without returning host paths |
| `query_archive_candidates` | `archive_commands.rs` | `request: QueryArchiveCandidatesRequest` | `Result<QueryArchiveCandidatesResponse, ArchiveSelectionError>` | Returns ranked launch executable candidates for one canonical platform |
| `query_archive_hazards` | `archive_commands.rs` | `request: QueryArchiveHazardsRequest` | `Result<QueryArchiveHazardsResponse, ArchiveSelectionError>` | Pages blocking archive/path hazards |
| `query_archive_warnings` | `archive_commands.rs` | `request: QueryArchiveWarningsRequest` | `Result<QueryArchiveWarningsResponse, ArchiveSelectionError>` | Pages non-blocking archive warnings |
| `discard_archive_selection` | `archive_commands.rs` | `request: DiscardArchiveSelectionRequest` | `Result<DiscardArchiveSelectionResponse, ArchiveSelectionError>` | Explicitly deletes an idle staged selection; active publication leases win |
| `load_authoritative_adp_edit_metadata` | `adp_commands.rs` | `request: LoadAuthoritativeAdpEditMetadataRequest` | `Result<AuthoritativeAdpEditMetadata, PublishAdpListingError>` | Refetches and verifies the exact signed edit revision, preserving raw launch tags and typed defects |
| `select_build_file` | `adp_commands.rs` | None | `Option<String>` | Legacy native path picker retained for compatibility; launch-tier publication uses `select_archive_build` instead |
| `hash_build_file` | `adp_commands.rs` | `request: HashBuildFileRequest` | `String` | Legacy path-based SHA-256 helper retained for compatibility; opaque selections use `hash_selected_archive` |
| `publish_adp_listing` | `adp_commands.rs` | `request: PublishAdpListingRequest` | `Result<PublishAdpListingResult, PublishAdpListingError>` | Compatibility entry point that runs the same pipeline with a default, non-launch-tier `PublishAdpListingRequestExtension` |
| `publish_adp_listing_with_contract` | `adp_commands.rs` | `request: PublishAdpListingRequest`, `extension: PublishAdpListingRequestExtension` | `Result<PublishAdpListingResult, PublishAdpListingError>` | Publishes validated package/launch metadata from an opaque build lease; mints/resumes backend UUID identity and reports typed partial publication |
| `request_lnurl_invoice` | `adp_commands.rs` | `request: RequestLnurlInvoiceRequest` | `RequestLnurlInvoiceResponse` | Requests the listing's fixed-amount LNURL invoice |
| `connect_nwc_wallet` | `adp_commands.rs` | `request: ConnectNwcWalletRequest` | `ConnectNwcWalletResponse` | Connects NWC and returns safe wallet metadata without exposing the secret |
| `pay_nwc_invoice` | `adp_commands.rs` | `request: PayNwcInvoiceRequest` | `PayNwcInvoiceResponse` | Pays through NWC and returns the preimage and optional fee |
| `confirm_purchase` | `adp_commands.rs` | `request: ConfirmPurchaseRequest` | `ConfirmPurchaseResponse` | Refetches the listing, confirms proof with ADP, validates/persists the receipt, and caches its token |
| `start_split_purchase` | `adp_commands.rs` | `request: StartSplitPurchaseRequest` | `StartSplitPurchaseResponse` | Refetches listing/terms, validates the refund LNURL bounds, creates and decrypts the all-leg-preflighted private quote, then persists it under the active account |
| `next_split_invoice` | `adp_commands.rs` | `request: SplitQuoteRequest` | `SplitInvoiceResponse` | Requests only the next committed leg and stores the invoice attempt NIP-44 encrypted to the active account |
| `submit_split_payment` | `adp_commands.rs` | `request: SubmitSplitPaymentRequest` | `SplitSettlementResponse` | Submits one proof, persists leg state, and validates/persists the aggregate receipt and download token after the final leg |
| `settlement_history` | `adp_commands.rs` | None | `Vec<SettlementHistoryItem>` | Lists account-scoped durable quote and recovery state |
| `refresh_settlement` | `adp_commands.rs` | `request: SplitQuoteRequest` | `SettlementStatusResponse` | Refreshes one quote from its processor with account-switch rejection |
| `replace_split_refund_destination` | `adp_commands.rs` | `request: ReplaceSplitRefundDestinationRequest` | `serde_json::Value` | Resolves and authorizes a replacement refund destination without resetting failure/deadline state |
| `prepare_split_recovery` | `adp_commands.rs` | `request: SplitQuoteRequest` | `RecoveryPreparationResponse` | Obtains processor evidence or signs/encrypts buyer fallback kind-1034 evidence and persists every paid-leg obligation |
| `prepare_split_refund` | `adp_commands.rs` | `request: SplitRefundRequest` | `RefundInvoiceResponse` | Prepares an exact-principal refund invoice and persists the recipient's original attempt |
| `confirm_split_refund` | `adp_commands.rs` | `request: ConfirmSplitRefundRequest` | `serde_json::Value` | Submits refund proof and records verified recovery state |
| `fetch_profile_badges` | `main.rs` | `profile_pubkey: String` | `Vec<ProfileBadgeEntry>` | Fetch NIP-58 badges for a profile (desktop only) |
| `fetch_earned_badges` | `main.rs` | `profile_pubkey: String` | `Vec<EarnedBadgeSummary>` | Fetch earned badges with definitions (desktop only) |
| `publish_game_score` | `nip46_commands.rs` | `score: u64` | `String` | Placeholder command; requires NIP-46 session but does not yet publish an event |
| `ping_bunker` | `nip46_commands.rs` | None | `serde_json::Value` | Pings the active signer and emits `bunker-heartbeat` |
| `attempt_reconnect` | `nip46_commands.rs` | None | `serde_json::Value` | Manually reconnects an offline NIP-46 session |
| `get_network_discovery_settings` | `main.rs` | None | `NetworkDiscoverySettings` | Get relay discovery settings |
| `set_allow_insecure_public_ws` | `main.rs` | `allow: bool` | `()` | Toggle allowing insecure ws:// relays for public hosts |
| `test_extended_network_discovery` | `main.rs` | None | `serde_json::Value` | Debug-only forced extended-network discovery with detailed statistics |
| `select_blossom_media_file` | `blossom_commands.rs` | `request: ExpectedBlossomPublisherRequest` | `Result<Option<BlossomMediaSelectionDto>, BlossomCommandError>` | Selects and inspects media for the expected publisher; cancellation returns `None` |
| `start_blossom_upload` | `blossom_commands.rs` | `request: StartBlossomUploadRequest` | `Result<BlossomUploadResponse, BlossomCommandError>` | Starts account-scoped upload with signed authorization, verification, and progress events |
| `retry_blossom_upload` | `blossom_commands.rs` | `request: RetryBlossomUploadRequest` | `Result<BlossomUploadResponse, BlossomCommandError>` | Retries a retained media selection without requiring another file pick |
| `cancel_blossom_upload` | `blossom_commands.rs` | `request: CancelBlossomUploadRequest` | `Result<CancelBlossomUploadResponse, BlossomCommandError>` | Requests cancellation of the matching account-bound upload |
| `discard_blossom_media_selection` | `blossom_commands.rs` | `request: DiscardBlossomMediaRequest` | `Result<DiscardBlossomMediaResponse, BlossomCommandError>` | Discards a selection and cleans up its managed upload state |
| `get_blossom_server_settings` | `blossom_commands.rs` | `request: ExpectedBlossomPublisherRequest` | `Result<BlossomServerSettingsDto, BlossomCommandError>` | Reads the active publisher's server configuration |
| `probe_blossom_server_health` | `blossom_commands.rs` | `request: ExpectedBlossomPublisherRequest` | `Result<BlossomServerHealthResponse, BlossomCommandError>` | Probes configured servers and returns health observations |
| `replace_blossom_server_settings` | `blossom_commands.rs` | `request: ReplaceBlossomServerSettingsRequest` | `Result<BlossomServerSettingsDto, BlossomCommandError>` | Validates and replaces the publisher's server configuration |
| `add_blossom_server` | `blossom_commands.rs` | `request: AddBlossomServerRequest` | `Result<BlossomServerSettingsDto, BlossomCommandError>` | Adds a normalized server origin to publisher settings |
| `update_blossom_server` | `blossom_commands.rs` | `request: UpdateBlossomServerRequest` | `Result<BlossomServerSettingsDto, BlossomCommandError>` | Updates a configured server's editable settings |
| `remove_blossom_server` | `blossom_commands.rs` | `request: BlossomServerOriginRequest` | `Result<BlossomServerSettingsDto, BlossomCommandError>` | Removes one configured server |
| `reorder_blossom_servers` | `blossom_commands.rs` | `request: ReorderBlossomServersRequest` | `Result<BlossomServerSettingsDto, BlossomCommandError>` | Changes publisher-defined server order |
| `set_preferred_blossom_server` | `blossom_commands.rs` | `request: SetPreferredBlossomServerRequest` | `Result<BlossomServerSettingsDto, BlossomCommandError>` | Sets or clears the preferred server |
| `resolve_blossom_server_candidates` | `blossom_commands.rs` | `request: ResolveBlossomServerCandidatesRequest` | `Result<Vec<String>, BlossomCommandError>` | Resolves configured upload candidates for the publisher |
| `preview_publisher_terms` | `adp_commands.rs` | `request: PublisherTermsContext` | `PublisherTermsReview` | Reviews commercial terms before publisher side effects |
| `save_publisher_draft` | `publisher_commands.rs` | `request: SavePublisherDraftRequest` | `u64` | Saves a revisioned account-scoped publisher draft |
| `publisher_recovery_state` | `publisher_commands.rs` | `expected_publisher_npub: String` | `PublisherRecoveryState` | Loads the expected publisher's drafts and operation journals |
| `publisher_distribution_state` | `publisher_management.rs` | `expected_publisher_npub: String`, `coordinate: String` | `Vec<DistributionEvidence>` | Resolves distribution evidence for a managed listing |
| `review_publisher_credential` | `publisher_management.rs` | `expected_publisher_npub: String`, `coordinate: String`, `root_event_id: String`, `action: RevocationAction` | `CredentialReview` | Reviews exact authorization/attestation evidence before a lifecycle action |
| `revoke_publisher_credential` | `publisher_management.rs` | `request: RevokePublisherCredentialRequest` | `Event` | Publishes the explicitly reviewed publisher credential action |
| `resume_publisher_operation` | `publisher_commands.rs` | `request: ResumePublisherOperationRequest` | `serde_json::Value` | Resumes a journaled publisher operation with explicit confirmation and account/session fencing |
| `get_nwc_wallet` | `adp_commands.rs` | `expected_account_pubkey: String` | `Option<ConnectNwcWalletResponse>` | Returns safe connection metadata for the expected account |
| `disconnect_nwc_wallet` | `adp_commands.rs` | `expected_account_pubkey: String` | `()` | Removes the expected account's saved wallet connection |
| `verify_nwc_wallet` | `adp_commands.rs` | `expected_account_pubkey: String` | `String` | Performs a non-paying wallet verification request |
| `get_purchase_plan` | `adp_commands.rs` | `request: PurchasePlanRequest` | `PurchasePlanReview` | Reviews the original quote and signed terms without dispatching payment |
| `approve_purchase_plan` | `adp_commands.rs` | `request: ApprovePurchasePlanRequest` | `PurchasePlanReview` | Persists explicit full-plan approval with payment limits, network, and fee policy |
| `resume_split_payment` | `adp_commands.rs` | `request: ResumeSplitPaymentRequest` | `SplitSettlementResponse` | Reconciles an existing attempt; never requests a replacement invoice or dispatches payment |
| `reconcile_pending_settlements` | `settlement_ui.rs` | None | `RecoveryReport` | Reconciles pending account-scoped payments and refunds without new spending |
| `recipient_refund_history` | `settlement_ui.rs` | `request: RefundHistoryRequest` | `Vec<RefundHistoryItem>` | Reads processor obligations and merges local recipient payment/proof checkpoints |
| `pay_split_refund` | `settlement_ui.rs` | `request: RefundPaymentRequest` | `serde_json::Value` | Dispatches the original refund attempt only after explicit payment confirmation |
| `resume_split_refund` | `settlement_ui.rs` | `request: RefundPaymentRequest` | `serde_json::Value` | Reconciles the original refund attempt without a new payment dispatch |
| `enrich_store_pages` | `store_page_commands.rs` | `request: StorePageEnrichmentRequest` | `StorePageBatchEnrichment` | Batch-enriches listings with validated Store Page presentation data |
| `enrich_store_page_detail` | `store_page_commands.rs` | `request: StorePageDetailRequest` | `StorePageDetailEnrichment` | Resolves validated detail-page content for a listing |
| `load_publisher_store_page_editor` | `store_page_commands.rs` | `request: LoadPublisherStorePageRequest` | `PublisherStorePageEditorState` | Loads current publisher Store Page and listing association evidence |
| `validate_store_page_draft_command` | `store_page_commands.rs` | `request: ValidateStorePageDraftRequest` | `ValidateStorePageDraftResponse` | Validates a draft against Store Page content and association policy |
| `publish_store_page` | `store_page_commands.rs` | `request: PublishStorePageRequest` | `PublishStorePageResponse` | Signs/publishes a Store Page and reports reciprocal listing-pointer status |
| `clone_store_page` | `store_page_commands.rs` | `request: CloneStorePageRequest` | `StorePageDraft` | Produces a reusable draft from an existing Store Page |
| `retry_store_page_pointer_sync` | `store_page_commands.rs` | `request: RetryStorePagePointersRequest` | `PublishStorePageResponse` | Retries listing-pointer synchronization after partial publication |
| `add_game_to_library` | `main.rs` | `game_coordinate: String`, `expected_buyer_npub: String` | `()` | Saves a listing to the expected buyer's account library |
| `is_game_in_library` | `main.rs` | `game_coordinate: String` | `bool` | Checks the active account's library-save state |
| `get_library_games` | `main.rs` | None | `Vec<LibraryGame>` | Lists games saved to the active account's library |
| `sync_purchase_receipts` | `receipt_commands.rs` | `request: ReceiptSyncRequest` | `ReceiptSyncReport` | Synchronizes buyer-scoped public receipt/authority evidence and emits ownership updates |
| `reconcile_known_entitlements` | `entitlement_commands.rs` | `request: EntitlementSyncRequest` | `EntitlementSyncReport` | Rechecks known grants for the expected account and signals revoked ownership |
| `import_purchase_receipts` | `receipt_commands.rs` | `request: ReceiptImportRequest` | `ReceiptSyncReport` | Imports signed receipt evidence offline and emits account-scoped ownership updates |
| `regtest_nip44_probe` | `nip46_commands.rs` | None | `String` | Non-paying local remote-signer challenge; runtime-rejected unless debug and `ARCADESTR_REGTEST_NIP46_PROBE` are enabled |

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
| `publish-progress` | `publish_adp_listing`, `publish_adp_listing_with_contract` | `PublishProgressPayload { step, status, server_url, message, bytes_uploaded, total_bytes }` | Archive lease/hash checks, all-server capacity preflight, authority/provisioning, relay propagation, and per-server upload state |
| `install-progress` | `install_game`, retained reinstall | `InstallProgressPayload { game_coordinate, phase, bytes, total }` | One lifecycle stream whose `phase` is `Preparing`, `Downloading`, `Verifying`, `Installing`, or `Complete` |
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
│   ├── Retains legacy LNURL purchases and adds account-scoped split quote resume, sequential NWC/manual-preimage legs, and refund-address preflight
│   └── Scoped install-progress listener, launcher state, seller profile, specs, gallery, and debug raw event data
│
├── LibraryView (app/src/ui_v2/views/library.rs)
│   ├── Reconciles saves, ready installs, retained packages, and lifecycle issues by coordinate
│   ├── Offers retained reinstall/delete, uninstall retention choice, and explicit recovery actions
│   └── Embeds `InstallationLauncher` for Play, Properties, preference provenance, and session status
│
├── PurchasesView (app/src/ui_v2/views/purchases.rs)
│   ├── Account-scoped purchase/claim history with active/refunded/revoked/unverified states
│   └── Durable split status, partial-payment refresh, recovery evidence, destination replacement, and per-leg refund obligation status
│
├── LoginV2View (app/src/ui_v2/views/login.rs)
│   ├── Desktop bunker/QR/local-nsec and web NIP-07/local-nsec capabilities
│   └── NIP-49 validates/decrypts only; active selection follows current identity, not stale flags
│
├── ProfileV2View / AchievementsView
│   └── Read-only profile, truthful badge visibility, cached-first badge refresh, shared listings
│
├── PublishV2View (app/src/ui_v2/views/publish.rs)
│   ├── `PublishViewState` studio for games, new/edit publication, game management, and campaigns
│   └── `PublishLaunchTierEditor` selects/inspects archives, edits validated profiles/defaults,
│       loads authoritative metadata, reviews hazards/defects, and preserves current-form retry intent
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

ADP-heavy views keep operation state local rather than adding another global store. Current flows primarily combine `RwSignal`, `Signal::derive`, `Effect::new`, `StoredValue`, `spawn_local`, explicit operation enums, and monotonic request generations. `AuthContext::begin_auth_flow()` and per-view expected-npub/generation checks discard late responses after account changes. `PublishViewState` gives archive selection and each files/directories/candidates/hazards/warnings query its own owner/request generation: picker cancellation and retryable transport failures retain the previous selection, replacement invalidates path bindings, platform changes invalidate candidate generations, account changes and terminal handle/mutation failures clear the build, and successful publication consumes it. `GameDetailView` unregisters its scoped `install-progress` listener safely even when registration finishes after unmount.

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
        invoke_publish_adp_listing_with_contract(request, extension.clone()).await
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
        .manage(ArchiveSelectionService::new(/* private session cache */))
        .manage(Arc::new(Launcher::new()))
        .invoke_handler(tauri::generate_handler![
            nip46_commands::login_with_nsec,
            adp_commands::publish_adp_listing_with_contract,
            archive_commands::inspect_archive,
            adp_commands::confirm_purchase,
            install_game,
            get_installed_games,
            /* ... */
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
```

Startup also runs interrupted-installation recovery before stale `Installing` rows can reach the UI, starts the archive-selection janitor, and detaches supervised launcher children when the Tauri run loop exits.

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
| `core::adp_publish` | Validated ADP listing/authorization construction | `AdpListingPublishInput`, `AdpPublishError` | `build_adp_listing_event_builder()`, `build_fulfillment_authorization_event_builder()` |
| `core::adp_publish_contract` | Typed publish/edit wire boundary | `PublishAdpListingRequestExtension`, `AdpLaunchTierIntent`, `PublishAdpListingError` | Authoritative metadata and typed partial publication |
| `core::archive_inspection_contract` | Path-safe archive IPC | `BuildSelectionRef`, selection/page DTOs, `ArchiveSelectionError` | Opaque IDs and validated pagination/search |
| `core::archive_inspection` | Bounded ZIP/tar inspection | Archive model, hazards, warnings, candidates | `inspect_archive()`, content detection, paged projections |
| `core::distribution` | Signed delivery resolution | `ResolvedInstallPlan`, `ResolvedArtifact`, `LaunchProfile`, `PackageFormat` | `resolve_install_plan()`, canonical host matching |
| `core::package_extraction` | Verified package extraction | `VerifiedPackage`, `CapabilityExtraction`, `ExtractionOutcome` | Digest/format verification, preflight, capability extraction |
| `core::install_contract` | Stable installation IPC failures | `InstallCommandError` | Preserves typed archive/resource rejection categories |
| `core::launch_contract` | Offline launch contract | `LaunchConfiguration`, `LaunchSession`, `LaunchError` | Preference resolution, session/outcome wire types |
| `core::adp_storage` | ADP and device persistence | `InstallationsRepository`, `InstallationSnapshot`, `RetainedPackage`, `LaunchProfilePreferencesRepository` | Atomic lifecycle transitions, snapshot/manifest reads, retained reinstall state |
| `core::hash_validation` | Exact delivery-hash checks | — | `is_sha256_hex()` |
| `core::replaceable_event` | Shared replacement ordering | — | `is_replaceable_event_newer()` |
| `core::http_client` | Testable HTTP and atomic downloads | `HttpClient`, `ReqwestHttpClient`, `HttpDownloadOutcome` | `get()`, `post_json()`, `download_to_path()` |
| `core::nip98_client` | Signed HTTP requests | NIP-98 event/header helpers | `build_nip98_auth_header()` |
| `core::nwc_client` | Nostr Wallet Connect | Wallet connection/payment types | Connect and pay invoice operations |
| `core::durable_payments` | Restart-safe payment journal | `PaymentWallet`, `DurableNwcPayments`, `DurablePaymentError` | `pay_or_resume_guarded()`, `reconcile()`, `validate_payment_proof()` |
| `core::purchase_contract` | Explicit purchase-plan review and approval | `PurchasePlanReview`, `ApprovePurchasePlanRequest`, `InvoiceNetwork`, `RoutingFeePolicy` | `dispatch_ready()`, `require_live_purchase_authorization()` |
| `core::purchase_execution` | Verified approvals and irreversible dispatch admissions | `PurchaseExecutor`, `ExecutionError` | `review()`, `verify_sources()`, `approve_verified_with_fee_policy()`, `pay_leg_guarded()`, `reconcile_leg()` |
| `core::settlement_recovery` | Account/session-fenced recovery coordination | `RecoveryCoordinator`, `SessionFence`, `RecoveryLease` | `activate()`, `observe()`, `claim()`, `ensure_current()` |
| `core::receipt_portability` | Portable receipt discovery and verification | `ReceiptPortability`, `ReceiptSource` | `import_json()`, `ingest()`, `synchronize()` |
| `core::receipt_sync_contract` | Buyer-scoped receipt IPC | `ReceiptSyncRequest`, `ReceiptImportRequest`, `ReceiptSyncReport`, `ReceiptVerification` | Request/report and chain-verification wire types |
| `core::entitlement_reconciliation` | Reconcile known grant evidence | `EntitlementSource`, `EntitlementEvidence` | `EntitlementsRepository::reconcile_known_grants()` |
| `core::entitlement_sync_contract` | Buyer-scoped grant reconciliation IPC | `EntitlementSyncRequest`, `EntitlementSyncReport` | Expected-account request and reconciliation counters |
| `core::publisher_recovery` | Durable publisher intent and operations | `repository::PublisherRepository`, `PublisherDraft`, `PublisherOperation`, `PublisherRecoveryState` | Revisioned drafts and journal checkpoints |
| `core::publisher_management` | Distribution roles and credential lifecycle | `RevocationAction`, `CredentialReview`, `DistributionEvidence` | `operator_plans()`, `distribution_evidence()`, `validate_revocation_submission()` |
| `core::subscriptions` | Relay subscription lifecycle | `SubscriptionRegistry` | `dispatch_ephemeral_reads_batch_with_policy()`, `dispatch_permanent_subscriptions()`, `run_notification_loop()` |
| `desktop::adp_commands` | ADP IPC orchestration | `PublishAdpListingRequest`, `ConfirmPurchaseRequest` | `publish_adp_listing_with_contract()`, `confirm_purchase()`, `discover_adp_servers()` |
| `desktop::publisher_commands` | Publisher drafts and resumable operations | `SavePublisherDraftRequest`, `ResumePublisherOperationRequest` | `save_publisher_draft()`, `publisher_recovery_state()`, `resume_publisher_operation()` |
| `desktop::publisher_management` | Publisher distribution and revocation IPC | `CredentialReview`, `DistributionEvidence` | `publisher_distribution_state()`, `review_publisher_credential()`, `revoke_publisher_credential()` |
| `desktop::settlement_ui` | Settlement and recipient-refund recovery | `RefundHistoryRequest`, `RefundPaymentRequest`, `RefundHistoryItem` | `reconcile_pending_settlements()`, `recipient_refund_history()`, `pay_split_refund()`, `resume_split_refund()` |
| `desktop::receipt_commands` | Receipt portability IPC | `ReceiptSyncRequest`, `ReceiptImportRequest`, `ReceiptSyncReport` | `sync_purchase_receipts()`, `import_purchase_receipts()` |
| `desktop::entitlement_commands` | Known-grant reconciliation IPC | `EntitlementSyncRequest`, `EntitlementSyncReport` | `reconcile_known_entitlements()` |
| `desktop::archive_selection` | Managed archive capabilities | `ArchiveSelectionService`, `PublicationLease` | Stage/inspect/hash/query/discard and leased upload |
| `desktop::archive_commands` | Archive IPC adapters | Typed archive request/response models | Selection, inspection, paged query, hash, discard commands |
| `desktop::install` | Atomic placement and recovery | `OperationLock`, `PlacedInstallation`, `RecoveryReport` | Capability extraction, commit, startup recovery, no-follow removal |
| `desktop::launcher` | Direct native launch | `Launcher`, supervised child/session state | Options/preferences, shell-free spawn, status, detach |
| `desktop::marketplace_stream` | Completion truthfulness | — | Emits completion only on full relay success |
| `desktop::startup_runtime` | Startup async bridge | — | Runs recovery and relay startup work without nested-runtime assumptions |

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

Signed distribution metadata becomes an immutable install-time snapshot before filesystem effects:

```rust
// core/src/distribution/domain.rs
pub struct ResolvedInstallPlan {
    pub game_coordinate: GameCoordinate,
    pub artifact: ResolvedArtifact, // digest, PackageFormat, ordered ADP sources
    pub launch_profiles: Vec<LaunchProfile>,
    pub publisher_default_profile_id: Option<ProfileId>,
    pub source_event_json: String,
}

pub enum PackageFormat { Zip, TarZstd, TarGz }
pub enum LaunchType { Game, Config, Server, Editor, Tool }

// core/src/adp_storage.rs
pub struct InstallationRecord { /* UUID, coordinate/root, package identity,
    canonical target, signed event/default, lifecycle state, recovery marker */ }
pub struct InstallationSnapshot { /* record + validated profiles + exact manifest */ }
pub struct RetainedPackage { /* coordinate/package UUID/path/size + pinned reinstall snapshot */ }

// app/src/tauri_bridge.rs / desktop IPC projection
pub struct InstalledGame {
    pub installation_id: String,
    pub game_coordinate: String,
    pub installation_root: String,
    pub package_digest: String,
    pub package_format: String,
    pub state: InstallationState,
    pub recovery_discriminator: Option<InstallationRecoveryDiscriminator>,
    pub retained_package: bool,
    pub launchable: bool,
    pub installed_at: i64,
}
```

`InstallationState` is `Installing`, `Ready`, `Failed`, or `Uninstalling`; failed recovery records distinguish `FinalRootPresent` from `FinalRootAbsent`. `InstallationManifestEntry` records every installed directory/regular file and digest, while each `InstallationProfile` persists install-time availability rather than reinterpreting relay metadata later. Ready commits atomically replace profiles and the complete manifest. Retained packages survive uninstall by explicit choice and support offline reinstall against the pinned signed snapshot.

`LaunchConfiguration` reports all persisted profiles, publisher and saved-reference status, effective profile, source precedence, and fallback reasons. Only available `LaunchType::Game` profiles are Play candidates; config/server/editor/tool profiles remain queryable diagnostics. `LaunchSession` stores session UUID, installation/profile, PID, timestamps, and `Exited { code, signal }` or `Detached`; sessions are memory-only and are not durable process identities. `LaunchError` distinguishes stale installation/root/path/profile/permission/spawn/running/shutdown failures.

`DownloadTokensRepository::valid_token()` remains buyer-, coordinate-, and server-scoped and accepts only `expires_at > now`; legacy unscoped credentials are dropped. Installation, archive, and launch boundaries use typed `InstallCommandError`, `ArchiveSelectionError`, `ExtractionError`, `ResolveError`, and `LaunchError` internally, converting to strings only where older frontend wrappers require it.

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

ADP code preserves actionable failures before the IPC boundary. `AdpClientError::DownloadOwnership` maps HTTP 403, `DownloadDistribution` maps HTTP 451, and `DownloadProtocol` covers other unsuccessful statuses. Publishing returns `PublishAdpListingError`, including nested `PartialPublication` progress; archive IPC returns structured account/handle/mutation/detection/storage/pagination/platform failures; installation preserves typed archive-policy and insufficient-storage classes; and the launcher exposes stable `LaunchError` discriminants. Retained downloads are written through a private open handle, removed on truncation/cancellation, digest- and format-verified, and only then attached to installation state. Extraction failures clean staging while preserving any uncertain final tree for typed recovery rather than silently replacing it.

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

Signer-adjacent authorization checks distinguish writer and observer clocks. A kind-30406 attestation observed from the network may have `valid_from` up to 300 seconds in the reader's future, but that skew applies only to activation. Revocation is effective immediately at `revoked_at`; it is never delayed by the observer allowance. NIP-98 similarly signs the **exact URL sent on the wire**, including the percent-encoded coordinate/raw request target, so decoded and encoded forms cannot drift between the `u` tag and HTTP request.

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
#[cfg(feature = "native")]
pub mod archive_inspection;
#[cfg(feature = "native")]
pub mod package_extraction;

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

Device delivery follows the same snapshot principle as replaceable-event caching but does not depend on cache freshness: a verified signed listing resolves once into `ResolvedInstallPlan`; its digest, package format, canonical host target, profiles, publisher default, and raw event are persisted with the installation. `VerifiedPackage` then enforces the resolved digest/format, bounded archive policy, safe portable paths, no links/case collisions, and declared extraction size. Capability-rooted writes plus an operation lock expose either the old state or a complete atomically placed tree. The launcher consumes only that persisted snapshot, validates containment again, builds a sanitized child environment, and invokes the executable directly rather than through a shell.

### 8.8 Streaming Marketplace Fetch

The marketplace uses streaming instead of batch fetch, with **incremental cache refresh** and explicit per-relay completion semantics:

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
- **EOSE is per relay**: A relay succeeds only after its own end-of-stored-events response, including a valid empty result. Closed, disconnected, or silent relays are failed/timed out rather than reclassified as empty.
- **Safe partial retention**: Events received before EOSE or another relay's failure are retained only after signature and query-filter verification; malformed/tampered events never reach callbacks or persistence.
- **Truthful completion**: Any failed or timed-out relay returns `RelayManagerError::FetchIncomplete { succeeded, failed, timed_out, events }`. The frontend keeps already emitted listings and an error, but `marketplace-complete-*` and cache-freshness state are emitted/advanced only when every queried relay succeeds.

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
The workspace MSRV is Rust **1.89** (`workspace.package.rust-version`) and CI verifies native Linux, Windows desktop, and WASM with that exact compiler floor.
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
# From the repository root, run with hot reload
(cd desktop && cargo tauri dev)

# Production build
(cd desktop && cargo tauri build)
```

**Web Development**:
```bash
cd web
trunk serve --features web # Standalone browser frontend at http://localhost:5173
cd ..
cargo run -p arcadestr-xtask -- frontend-build
cargo run -p arcadestr-xtask -- frontend-build --release
```

Desktop development uses the object-form `beforeDevCommand` to run `trunk serve` without waiting and loads `devUrl`. Production invokes the release xtask before Tauri packaging. Running `trunk build --release` directly can create `web/dist`, but it does **not** write the Arcadestr freshness stamp and therefore is not a reproducible desktop release build.

The Tauri CLI is invoked from `desktop/`, where `tauri.conf.json` lives; it does
not accept Cargo's `--manifest-path` flag. Plain `trunk serve` intentionally
builds the desktop WebView frontend, while `--features web` enables browser
authentication and native-command fallbacks for standalone use. The xtask
commands above build the desktop frontend, not the standalone browser variant.

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
members = ["app", "desktop", "web", "core", "xtask"]
resolver = "2"

[workspace.package]
rust-version = "1.89"

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
    "beforeDevCommand": {
      "script": "trunk serve",
      "cwd": "../web",
      "wait": false
    },
    "beforeBuildCommand": {
      "script": "cargo run -p arcadestr-xtask -- frontend-build --release",
      "cwd": ".."
    },
    "devUrl": "http://localhost:5173/?tauri-dev=1",
    "frontendDist": "../web/dist"
  },
  "app": {
    "withGlobalTauri": true,
    "windows": [{ "title": "Arcadestr", "width": 1280, "height": 800 }],
    "security": { "csp": null }
  },
  "bundle": { "active": true, "targets": "all", "icon": ["icons/icon.png", "icons/icon.ico"] }
}
```

#### Web Trunk.toml
```toml
[serve]
port = 5173
headers = { "Cache-Control" = "no-store" }

[build]
target = "index.html"
dist = "dist"
public_url = "/"
```

`build-support/frontend_fingerprint.rs` hashes tracked app/core/web sources and manifests separately from `web/dist`. The xtask runs Trunk, computes both hashes, and atomically writes `FrontendBuildStamp` to `web/.arcadestr-frontend-build.txt` with build profile and timestamp. `desktop/build.rs` rejects a missing, debug-profile, source-stale, or distribution-stale stamp in release; debug embedded builds compile but print a prominent freshness diagnostic, while `trunk serve` development bypasses embedded assets. The build script also embeds `windows-app-manifest.xml` with `longPathAware=true`, and Tauri bundles the checked-in PNG/ICO icons. `.github/workflows/msrv.yml` reads Rust 1.89 from the workspace and checks native, Windows, xtask, and WASM targets with `--locked`.

The desktop manifest additionally carries the native archive/extraction and disk-capacity dependencies, `tauri-plugin-dialog = "2"` for archive selection, and debug-only `tauri-plugin-mcp-bridge`. The app manifest uses unconditional `nostr = "0.44"`, `url = "2"`, `HtmlDialogElement`, and `send_wrapper = "0.6"`; its `web` feature is an empty target marker. `desktop/capabilities/default.json` grants dialog/event and MCP capabilities; only MCP registration is compile-time debug-gated. `web/tailwind.config.js` scans `../app/src/**/*.rs`. Root `opencode.json` excludes generated/build/database artifacts from watching.

Root `.gitignore` keeps `target/`, `node_modules/`, `desktop/gen/`, `web/dist/`, and the frontend build stamp out of version control; generated Tauri schemas and built frontend assets must be regenerated locally. It also excludes local worktrees, temporary context, compiler/script caches, root-level SQLite databases and sidecars, `desktop/arcadestr.db` and its sidecars, and `/data/`. Database exclusions are scoped rather than a global `*.db` rule, leaving fixture directories visible. `.env` and `.env.*` are ignored except `.env.example`; `docs/` is no longer blanket-excluded, and the two `.vscode/` debugging Markdown guides are explicitly allowed alongside the shared editor configuration.

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
-- core/migrations/017_favorites.sql
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
(cd desktop && cargo tauri dev)

# Test the feature:
# 1. Log in
# 2. Browse to a game
# 3. Click "Add to Favorites"
# 4. Navigate to Favorites view
# 5. Verify game appears
```

For protocol, archive, filesystem, or launch work, happy-path tests are insufficient. Add contract serialization tests and adversarial cases for malformed signatures/tags, stale or forked replaceable events, wrong publisher/coordinate/host, traversal and absolute paths, backslashes, links, duplicate/case-colliding paths, reserved Windows names, decompression ratios, partial writes, ENOSPC, symlink swaps, stale opaque handles, lock contention, interrupted placement/uninstall, and shell metacharacters in executable/arguments. Assert that rejection occurs before signer, network, persistence, or final-tree side effects as appropriate.

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
(cd desktop && cargo tauri dev) 2>&1 | grep -E "(INFO|ERROR|WARN)"

# Or use RUST_LOG environment variable
(cd desktop && RUST_LOG=debug cargo tauri dev)
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
(cd desktop && ARCADESTR_RELAYS="wss://relay.example.com" cargo tauri dev)

# Block runtime relay discovery (NIP-65 gossip) when using debug relays
(cd desktop && ARCADESTR_BLOCK_DISCOVERY=1 cargo tauri dev)

# Or use CLI args (highest precedence over env vars and settings):
(cd desktop && cargo tauri dev -- -- --relay "wss://debug.local" --block-discovery)

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

If desktop behavior appears older than the Rust source, inspect the startup `ARCADESTR FRONTEND VERIFICATION WARNING`. A debug binary using embedded assets reports stamp status, build time, fingerprint, and the exact xtask remediation. Release build failure with `STALE FRONTEND` is intentional: run `cargo run -p arcadestr-xtask -- frontend-build --release`, then rebuild the desktop binary. A direct Trunk build does not refresh this stamp.

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

4. **NIP-46 connection timing**: Despite the historical “fast flow” name, active `connect_bunker` is blocking: it returns only after signer approval, profile persistence, and shared-auth synchronization. Only the unused library helper `init_signer_session_fast` is deferred; QR remains polled.

5. **Relay connection failures**: Relays may return HTML instead of WebSocket responses if down. Check logs for "expected ident" errors.

6. **Web feature check command**: `cargo check -p arcadestr-app --features web` without wasm target hits a pre-existing `web_auth` cfg mismatch in `app/src/lib.rs` and is not a valid standalone web validation. Use:
   ```bash
   cargo check -p arcadestr-app --target wasm32-unknown-unknown --features web
   ```

7. **ADP authentication and server choice**: Installation scans the signed server order for a valid buyer/coordinate/server token and uses the first server with one. Without a token it signs NIP-98 for the first signed server. The `u` tag exactly matches the encoded/raw request target. A rejected token is not deleted/retried; downloads neither resume nor fail over.

8. **Install trust boundary**: `install_game` refetches and signature/filter-verifies kind 30402, requires durable ownership or current explicit public/timed access, and resolves package/hash/platform/profiles from that event. Platform-less browsing may be permissive, but launch-tier installation requires a canonical host match. The package is retained, digest/format verified, bounded capability-extracted, atomically placed, and committed as an immutable snapshot.

9. **Installation lifecycle limits**: Retained-package reinstall, uninstall, and interrupted-operation recovery are implemented. There is still no general per-file repair, reveal-in-file-manager, or automatic replacement-version update. Recovery never guesses whether an uncertain final root is safe; it persists a discriminator and asks the UI to clean up explicitly.

10. **Publish/edit partial success**: Relay publication and sequential uploads are not transactional. `PartialPublication` carries nested cause plus successful servers; retry merges that progress with current form-owned fields and authoritative unknown tags while preserving the generated coordinate/hash. Campaign pointer updates retain their own typed partial-success result.

11. **Publication preflight and opaque archives**: A replacement artifact must come from a backend-issued `BuildSelectionRef`; legacy `file_path` is rejected. A publication lease prevents discard/expiry and rechecks identity/digest. Every selected ADP server is capacity-checked before signer, provisioning, relay, or upload side effects; servers without `max_upload_bytes` cannot be preflighted but remain eligible.

12. **Local-key threat model**: Desktop local nsecs are AES-256-GCM encrypted, but the encryption master key is a mode-`0600` file in the same app data area rather than an OS-keyring secret. Web encrypted nsecs keep ciphertext and encryption key in browser storage, which does not protect against malicious JavaScript or full profile compromise.

13. **Debug MCP capability**: `tauri-plugin-mcp-bridge` registration is debug-only, but `mcp-bridge:default` remains in the default capability file. Keep release configuration under review because that permission set includes WebView script execution and backend command access.

14. **Offline policy cache is fail-closed**: `MarketplaceCache::load_listings()` currently reconstructs cached listings as gated with no campaign pointers. Public/timed access and campaign links return only after relay refresh.

15. **Provisional campaign/grant kinds**: Kinds `1030` and `1031` are experimental allocations, not finalized interoperable NIP numbers. Grant parsing validates signed tags and context but does not inspect encrypted event content.

16. **Preserved edit tags are caller-filtered**: The core builder appends `preserved_tags` verbatim. Callers must remove form-managed tags or they can produce duplicates.

17. **Remote-auth synchronization**: Connect, switch, and restore paths must update both NIP-46 client state and `AppState.auth`. Logout can fail while clearing the persisted active flag before shared auth is disconnected or `auth_logout` is emitted.

18. **Blossom lifetime and transport policy**: Media selections are in-memory, retained until explicit discard or successful upload, and the oldest idle selection is evicted at the cap of 32. Debug builds seed/allow only explicit HTTP loopback (`localhost`/loopback IP); release is HTTPS-only and rejects private/non-global destinations.

19. **Authorization clocks**: Observer skew is 300 seconds only for a valid attestation's future `valid_from`. A `revoked_at` boundary is never widened, so revocation remains immediate.

20. **Launcher semantics**: Play considers available game profiles only, follows explicit → saved preference → publisher default → publisher order, and executes directly without a shell. Non-game/unavailable profiles remain diagnostic. Session/PID/outcome state is memory-only; shutdown detaches children rather than claiming durable supervision.

21. **Frontend freshness**: Release desktop builds reject a missing/stale/non-release frontend stamp or changed `web/dist`; debug embedded builds warn. Use the xtask, not direct Trunk, before validating embedded UI.

22. **Reset and migration identity**: `scripts/clear_arcadestr_data.sh [--dry-run] [--force]` clears both app-data identities, archive cache, legacy config, installs/retained packages/WebView data, and known keyring services. Database migration identity is filename/version sensitive: never renumber applied migrations; `017_split_settlement.sql` is now applied.

23. **Split settlement remains experimental and desktop-only**: `adp_settlement.rs` validates version-1 fee math, exact currency conversion, coverage, ordering, quote activation, deadlines, signed experimental events, and aggregate commitments; migration 017 and `SettlementRepository` retain account-scoped resumable state. Publisher Studio requires explicit fee consent, game detail drives sequential legs and resume, and Purchases exposes recovery/refund controls. Legacy single-payment, free/public/timed/claim, install, and web behavior remain available; web does not execute or recover split settlements.

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
| **NIP-98** | HTTP authentication whose signed `u` tag must exactly equal the encoded/raw request URL actually sent, with method in the `method` tag |
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
| **Streaming fetch** | Per-relay query delivery that retains verified partial events but reports success/completion only after every relay reaches EOSE |
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
| **Platform tag** | NOSTR listing tag `['platform', '<os>-<arch>']`. Browsing may treat absence permissively, but package/launch-tier resolution requires canonical targets and an exact normalized host match. |
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
| **ADP commercial terms** | Experimental immutable operator-signed kind 1032 root, accepted by exact `adp_terms` reference in the developer's signed listing. |
| **Split quote** | Private experimental kind 1033 settlement snapshot with fixed buyer/listing/terms/processor/legs and deadlines; payable only after all-leg invoice preflight. |
| **Recovery evidence** | Experimental append-only kind 1034 evidence for incomplete settlement and per-recipient refund obligations; not ownership. |
| **Fulfillment authorization** | Coordinate-scoped kind-30406 lifecycle by which a developer authorizes a delegated fulfillment key to issue delivery credentials |
| **Fulfillment mode** | Publishing choice: `none` (buy-only), `direct` (developer signer), or `delegate` (provisioned operator signer) |
| **Download token** | Expiring ADP credential scoped by buyer pubkey, game coordinate, and server after purchase confirmation or campaign claim |
| **NWC** | Nostr Wallet Connect, used to connect a wallet and pay a Bolt11 invoice without exposing its connection secret to the UI response |
| **Install registry** | SQLite installation records, launch-profile availability, exact manifests, lifecycle/recovery state, and retained-package links committed after verified extraction |
| **Artifact quarantine** | Preservation of a hash-mismatched download as `.corrupt`, `.corrupt.1`, and so on, without recording an install |
| **Local account** | Direct-signing account whose nsec is encrypted by Arcadestr rather than held by a NIP-46 signer |
| **Acquisition policy** | Signed kind-30402 current-access policy: `Gated`, `Public`, or half-open `TimedAccess`; zero price has no authorization meaning |
| **Game identifier** | Backend-minted UUID v4 that distinguishes generated coordinates from legacy or external NIP-99 `d` tags; rendered as `30402:<publisher-hex>:<d-tag>` |
| **Campaign** | Publisher-controlled append-only provisional claim policy; cancellation blocks later claims but preserves prior grants |
| **Campaign pointer** | Advisory listing tag referencing an immutable campaign root; authoritative discovery still falls back to publisher/coordinate queries |
| **Entitlement grant** | Buyer-specific append-only provisional durable credential issued by a publisher or properly authorized fulfillment key |
| **Durable acquisition record** | Account-scoped normalized purchase or promotion-claim history with active, disputed, refunded, revoked, or unverified status |
| **Device registry** | Local installation-snapshot inventory shared on a device; a ready installation is not proof of durable ownership |
| **Replaceable ordering** | Central invariant: later `created_at` wins; equal timestamps choose the lexicographically lower event ID |
| **Request generation** | Monotonic UI token paired with expected identity to reject stale asynchronous responses |
| **Request-scoped event** | Marketplace Tauri event named `marketplace-product-{id}` or `marketplace-complete-{id}` to isolate concurrent streams |
| **Publisher studio** | `PublishViewState` workflow for authoritative listing edits, opaque archive inspection, package/launch metadata, partial-publication retry, game management, and campaigns |
| **Package format** | Signed archive encoding resolved as `Zip`, `TarZstd`, or `TarGz`; content detection and extraction must agree with it |
| **Launch profile / type** | Persisted executable, working directory, arguments, display name, and ID classified as game/config/server/editor/tool; only available game profiles are Play candidates |
| **Publisher launch default** | Signed profile choice scoped to canonical platform targets; it follows a saved user preference and precedes publisher order |
| **Resolved install plan** | Verified signed-listing boundary containing coordinate, artifact digest/format/sources, canonical-host profiles/default, and raw event snapshot |
| **Archive handle** | Account-bound canonical UUID referencing a private staged archive; never a caller-supplied filesystem path |
| **Retained package** | Verified downloaded archive stored independently from an installation with a pinned reinstall snapshot and optimistic package UUID |
| **Installation snapshot** | Immutable read model combining an installation record, install-time profile availability, exact manifest, target/default, and signed event evidence |
| **Installation lifecycle / recovery** | `Installing → Ready` or explicit `Failed`/`Uninstalling` handling under locks; interrupted state records whether the final root is present or absent |
| **Launch preference** | Per-game-coordinate saved game profile; resolution falls back to publisher default and then publisher order when stale or unavailable |
| **Launch session** | In-memory start/status record with session ID, PID, timestamps, and exited/detached outcome; not a durable process identity |
| **EOSE-complete relay** | Relay that ended its own query with end-of-stored-events, including a valid empty result |
| **Partial relay result** | Signature/filter-verified events retained from an incomplete multi-relay fetch that still returns `FetchIncomplete` and no completion/freshness signal |
| **Frontend build stamp** | Source fingerprint, distribution fingerprint, profile, and timestamp written only by `arcadestr-xtask frontend-build` |
| **MSRV** | Minimum supported Rust version, currently 1.89 and checked on native, Windows, and WASM CI jobs |
| **Blossom policy** | Account-scoped ordered/preferred HTTPS server configuration; debug alone permits explicit loopback HTTP and seeds localhost |
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
The same Leptos UI runs in both Tauri (desktop) and browser (web), but opaque archive selection/inspection, retained downloads, capability extraction, lifecycle/recovery, and shell-free launching are desktop-only. Authentication can be remote (NIP-46 desktop, NIP-07 web) or a locally encrypted nsec. The `app` crate provides explicit web fallbacks for every native bridge.

### 2. **NIP-46 Fast Connection Flow**
The active `connect_bunker` command waits for signer approval and returns only after the profile is saved and both NIP-46 and shared `AuthState` are synchronized. The deferred `init_signer_session_fast` helper remains available but is not used by this command; QR login still uses polling.

### 3. **Relay Gossip (NIP-65)**
The app doesn't just connect to hardcoded relays. It fetches each user's relay list, builds a coverage map, and uses greedy set cover to select optimal relays. This is the key to efficient decentralized communication.

### 4. **Feature-Gated Core**
The `core` crate compiles differently for native (desktop) and WASM (web) targets. Native gets tokio, sqlx, and full NOSTR functionality. WASM gets stubs and relies on browser APIs. Understanding `#[cfg(feature = "native")]` guards is essential.

### 5. **Tauri IPC Pattern**
Frontend wrappers in `tauri_bridge.rs` call `tauri_invoke.rs`, which invokes `window.__TAURI__.core.invoke()`. Commands are registered in `main.rs`; publication/archive logic lives in `adp_commands.rs`, `archive_commands.rs`, and `archive_selection.rs`, installation lifecycle in `main.rs`/`install.rs`, and direct launch in `launcher.rs`. Marketplace streams use request-scoped product/completion events; installs use scoped filtering over the fixed `install-progress` event.

---

## Recent Major Features (2026-07)

**ADP marketplace completion and Gate 5 delivery:**
- The publisher studio stages archives behind opaque account-bound UUIDs, performs bounded ZIP/tar inspection and paged safety/candidate review, and publishes signed package, canonical platform, launch profile, and target-scoped default tags from `PublishAdpListingRequestExtension`.
- Publication leases protect selected bytes; launch tiers require a hash; all servers are capacity-preflighted before side effects. Typed `PartialPublication` records successful uploads, and retry merges that progress with current form intent and authoritative metadata on the same backend-minted coordinate.
- Installation refetches and verifies the signed listing, resolves a host-specific `ResolvedInstallPlan`, scans signed servers for a scoped cached token (otherwise NIP-98 uses the first), retains and verifies the package, capability-extracts under archive limits, atomically places the tree, and commits profiles plus exact manifest.
- `LibraryView` exposes ready installs, lifecycle issues, retained-package offline reinstall/delete, safe uninstall, and startup recovery. `InstallationLauncher` directly starts available game profiles with saved → publisher-default → publisher-order preference fallback and memory-only session supervision.
- Relay streaming now requires per-relay EOSE for full success. Verified partial events remain visible on `FetchIncomplete`, but completion and cache freshness are withheld.
- Authorization observers allow 300 seconds only on future `valid_from`; revocation remains immediate. NIP-98 signs the exact encoded request URL.
- Blossom keeps selections in memory through retry, cleans them on explicit discard/success, evicts oldest at 32, and permits HTTP only for explicit loopback in debug; release remains HTTPS-only.
- Rust 1.89 is the CI-enforced MSRV. The frontend xtask stamps source and `web/dist` fingerprints, release rejects stale assets, Windows embeds long-path awareness/icons, and `clear_arcadestr_data.sh` resets both data identities, native artifacts/caches, and known keyring services.

**Encrypted local nsec accounts:**
- `login_with_nsec` creates an `AccountManager` record, encrypts the nsec, switches `AuthState` to a local signer, and clears active NIP-46 retry/client state.
- Account listing, switching, deletion, startup restoration, and logout now cover both NIP-46 and local accounts. The desktop master key is currently a protected app-data file rather than an OS-keyring secret.

### Game Detail View Consolidation
The GameDetail view was rewritten as a fully standalone component:
- **Standalone implementation**: No longer wraps `DetailView` — directly renders hero section, buy panel, metadata, specs, gallery, and seller profile
- **Buy flow**: LNURL invoice creation, copy/open-wallet actions, NWC payment or manual-preimage confirmation, and ADP purchase confirmation
- **Install flow**: Owned, explicitly public, and active timed-access listings call `install_game`; zero price alone remains gated. Button state follows scoped `install-progress` phases and cleans up its listener safely on unmount; ready installs expose the launcher.
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
- **`core/migrations/006_purchases.sql`**: `purchases` table with indexes on buyer, order, and coordinate
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
- **Entitlement migration**: `core/migrations/010_entitlements.sql` adds append-only `entitlement_events` history indexed by buyer/game, grant ID, and predecessor.
- **Ownership**: `OwnershipService` resolves `PurchaseReceipt | EntitlementGrant | None`. Public and timed access are current access modes, not stored ownership.
- **ADP route**: `AdpClient::entitlement_claim` calls exact `POST /entitlement/claim` with NIP-98 and typed campaign, coordinate, grant, distribution, and protocol errors.
- **Commands/bridges**: Discovery, summary, publish, pointer-repair, claim, ownership, and purchase-history commands cover buyer and publisher flows; campaign publication reports pointer-update partial success explicitly.
- **Publisher authority**: Campaign create/update/cancel requires the active publisher identity signer. A fulfillment key may issue grants only when both the historical listing delegation and referenced kind `30406` authorization lifecycle were active at issuance; either source alone fails closed. Direct publisher grants require neither proof. Fulfillment keys cannot control campaigns or revoke grants.
- **Install trust boundary**: The current signed listing controls server, hash, version, and current access policy. Historical validated grants/receipts remain durable after price, policy, version, or campaign cancellation changes, and cached tokens are buyer-scoped.

### Experimental ADP Split-Settlement Groundwork
- **Shared pure model**: `core/src/adp_settlement.rs` mirrors the server's version-1 constants and rules for `none`/fixed/basis-point fees, checked gross allocation, operator deduplication, SAT/SATS/exact-BTC conversion, deterministic legs, all-leg preflight, immutable failure/refund deadlines, and aggregate receipt commitments.
- **Discovery/listings**: `AdpServerInfo` understands settlement fields and provisional kinds 1032-1034; signed announcement parsing exposes split/terms/legacy policy. NIP-99 parsing retains valid and malformed `adp_terms` references, while `apply_managed_adp_terms` replaces only this managed tag family and preserves unrelated extensions.
- **Persistence**: `017_split_settlement.sql` adds account-scoped quotes, ordered legs, encrypted attempts/destination revisions, and recovery rows. `SettlementRepository` supports durable resume and explicitly gates side effects on the initiating account.
- **Desktop orchestration**: `desktop/src/adp_commands.rs` creates/preflights quotes, discloses one invoice at a time, stores attempts encrypted with signer NIP-44, submits proofs, persists aggregate receipts, refreshes status, and drives recovery/destination/refund endpoints. Every mutating continuation rechecks the active account.
- **Publisher Studio**: paid listings show the server/operator fee economics and require explicit acceptance of the exact terms roots; edits and typed retries preserve those references, while publication rejects uncovered operators or malformed terms.
- **Buyer UI**: `GameDetailView` resumes active account/listing/processor quotes and advances NWC or manual-preimage legs without replacing legacy checkout. `PurchasesView` presents active/failed/settled states, partial-payment refresh, immutable recovery deadlines, destination replacement, and per-leg refund status; it does not misrepresent recipient-owned refund payment/confirmation as a buyer action. Split execution and recovery remain desktop-only.

---
