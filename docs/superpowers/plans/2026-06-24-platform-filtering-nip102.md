# Platform Filtering and NIP-102 Ownership Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement platform filtering and NIP-102 ownership verification for Arcadestr listings.

**Architecture:** Core parses and verifies protocol data, desktop enriches listings and exposes commands, app renders platform-aware and ownership-aware storefront actions. Work proceeds in dependency order with validation after each layer.

**Tech Stack:** Rust, nostr/nostr-sdk 0.44, sqlx 0.8 SQLite, lightning-invoice 0.32, sha2 0.10, Tauri v2, Leptos 0.8.

---

## Files
- Create: `core/migrations/003_purchases.sql`
- Create: `core/src/purchases.rs`
- Modify: `core/Cargo.toml`
- Modify: `core/src/lib.rs`
- Modify: `core/src/storage/db.rs`
- Modify: `core/src/marketplace.rs`
- Modify: `desktop/src/main.rs`
- Modify: `app/src/models.rs`
- Modify: `app/src/tauri_bridge.rs`
- Modify: `app/src/ui_v2/views/browse_games.rs`
- Modify: `app/src/components/publish.rs`
- Modify: `CODEBASE.md`

### Task 1: Core purchases persistence and verification
- [ ] Add `lightning-invoice = "0.32"` and `sha2 = "0.10"` to native dependencies in `core/Cargo.toml`.
- [ ] Create `core/migrations/003_purchases.sql` with purchases table and indexes from the approved prompt.
- [ ] Register the migration in `core/src/storage/db.rs` with `include_str!("../../migrations/003_purchases.sql")` and append it to `MIGRATIONS`.
- [ ] Create `core/src/purchases.rs` with `PurchaseError`, `StoredReceipt`, `PurchasesRepository`, and `parse_and_validate_receipt`.
- [ ] Use `event.kind.as_u16()` and `event.created_at.as_secs()` for nostr 0.44.
- [ ] Compare invoice payment hash using `to_byte_array()` if available; otherwise use the available hash byte API at compile time.
- [ ] Add unit tests for wrong kind, missing p, buyer mismatch, missing order/a, invalid preimage hex, no proof, accepted zap e tag, paid ownership, refunded latest status.
- [ ] Export `pub mod purchases` behind `#[cfg(feature = "native")]` in `core/src/lib.rs`.
- [ ] Run `cargo test -p arcadestr-core`. Stop on failure and report.

### Task 2: NIP-99 platform and delivery metadata
- [ ] Extend `Nip99Listing` in `core/src/marketplace.rs` with `platforms: Vec<String>` and `nip94_event_id: Option<String>`.
- [ ] Extend `parse_listing` to collect all `platform` tag values and the first `nip94` event id.
- [ ] Extend `MarketplaceFilter` with `#[serde(default)] pub platforms: Vec<String>`.
- [ ] Update `passes_filter_nip99` so platform filtering includes unrestricted listings and filters incompatible restricted listings.
- [ ] Add parser/filter tests for platform tags, nip94 tag, matching platform, nonmatching platform, and unrestricted listing inclusion.
- [ ] Run `cargo test -p arcadestr-core`. Stop on failure and report.

### Task 3: Shared app models
- [ ] Add `platforms`, `nip94_event_id`, and `is_owned` fields with serde defaults to `app/src/models.rs::GameListing`.
- [ ] Update all `GameListing` literals and `from_nip15`/`from_listing` constructors.
- [ ] Add `PlatformInfo` with `tag()` method.
- [ ] Run `cargo check -p arcadestr-app --target wasm32-unknown-unknown`. Stop on failure and report.

### Task 4: Desktop state and commands
- [ ] Add `purchases: Arc<Mutex<arcadestr_core::purchases::PurchasesRepository>>` to `AppState`.
- [ ] Initialize the repository from `database.pool().clone()` where other database-backed repositories are initialized.
- [ ] Add `get_platform_info`, `install_game`, and `ingest_receipt` Tauri commands.
- [ ] Register all three commands in the existing single `tauri::generate_handler!` call.
- [ ] Update `fetch_marketplace` to enrich `is_owned`, preserve platform/nip94 metadata, and apply backend `filter.platforms` filtering.
- [ ] Run `cargo build -p arcadestr-desktop`. Stop on failure and report.

### Task 5: Frontend bridge and browse UI
- [ ] Add `invoke_get_platform_info` and `invoke_install_game` wrappers to `app/src/tauri_bridge.rs`, with web fallbacks if needed.
- [ ] Update `BrowseGamesView` to detect host platform using `leptos::task::spawn_local` and set default filter with `Effect::new`.
- [ ] Add a platform select with My Platform and All Platforms options.
- [ ] Derive `displayed_listings` from the loaded listings and active filter.
- [ ] Pass active platform filter into listing card render helpers.
- [ ] Render Download / Install for owned listings and Buy/detail action for unowned listings.
- [ ] Render Incompatible and Verified Delivery badges according to the approved rules.
- [ ] Run `cargo check -p arcadestr-app --target wasm32-unknown-unknown`. Stop on failure and report.

### Task 6: Publisher docs and form support
- [ ] Add platform input/help text to `app/src/components/publish.rs` and include selected platform tags in the listing model.
- [ ] Document platform tag convention in `CODEBASE.md` Glossary.
- [ ] Run `cargo check -p arcadestr-app --target wasm32-unknown-unknown`. Stop on failure and report.

### Task 7: Final validation
- [ ] Run `cargo test -p arcadestr-core`.
- [ ] Run `cargo build -p arcadestr-desktop`.
- [ ] Run `cargo check -p arcadestr-app --target wasm32-unknown-unknown`.
- [ ] If builds pass, run desktop smoke test with `cd desktop && timeout 60 cargo tauri dev 2>&1` and visually inspect platform filter/cards if startup succeeds.
