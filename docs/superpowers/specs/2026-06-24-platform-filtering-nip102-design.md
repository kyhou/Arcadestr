# Platform Filtering and NIP-102 Ownership Design

## Goal
Add platform-aware storefront filtering and receipt-backed ownership so Arcadestr can distinguish listings that should show Buy from listings that should show Download / Install.

## Architecture
The feature is layered. The core crate owns protocol parsing, receipt verification, database persistence, and NIP-99 listing tag extraction. The desktop crate owns authenticated state, ownership enrichment, and Tauri commands. The app crate owns shared IPC models, bridge wrappers, and Leptos UI filtering/actions.

## Core Data Flow
Kind 30402 listing events are parsed in `core/src/marketplace.rs`. The parser captures `platform` tags into `Nip99Listing.platforms` and the first `nip94` tag into `Nip99Listing.nip94_event_id`. These fields map into `app::models::GameListing` with serde defaults so cached and older payloads remain compatible.

NIP-102 kind 1020 receipt events are parsed in `core/src/purchases.rs`. Validation checks kind, buyer `p` tag, listing `a` tag, `order`, and either a self-contained `bolt11` plus `preimage` proof or an optimistic zap receipt `e` tag. Receipts are stored idempotently in SQLite. Ownership uses the latest receipt per order and returns true only for `paid` or `fulfilled`.

## Desktop Integration
`AppState` receives a `PurchasesRepository` created from the existing database pool. `fetch_marketplace` fetches NIP-99 listings, maps platform and NIP-94 fields, checks ownership for authenticated users, and applies backend platform filtering. New Tauri commands expose host platform detection, receipt ingestion, and a placeholder install action.

## Frontend Integration
`PlatformInfo` is added to the shared model layer. The Tauri bridge exposes `invoke_get_platform_info` and `invoke_install_game`. `BrowseGamesView` detects the host platform on mount, defaults the active filter to that tag, provides an All Platforms escape hatch, derives displayed listings reactively, and shows incompatibility and verified-delivery badges. Owned listings call install; unowned listings retain the existing buy/detail flow.

## Publisher Convention
Arcadestr listing events declare platform compatibility with `['platform', '<os>-<arch>']`, where values use Rust `std::env::consts` names such as `linux-x86_64` and `macos-aarch64`. Listings without platform tags are compatible everywhere. `CODEBASE.md` and the publish form document/support this convention.

## Validation
Each layer is validated before proceeding: `cargo test -p arcadestr-core`, `cargo build -p arcadestr-desktop`, `cargo check -p arcadestr-app --target wasm32-unknown-unknown`, then a Tauri dev visual smoke test if earlier checks pass.
