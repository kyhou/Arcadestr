# Debug NIP-99 Raw Listing Box Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a debug-only product detail panel showing original NIP-99 event JSON when present, with parsed listing fields as fallback.

**Architecture:** Preserve raw event JSON in debug builds at the NIP-99 parser boundary, pass it through existing `GameListing` IPC models, and render a pure formatted JSON string in the Leptos detail component. Release builds omit the raw field and debug panel with compile-time cfg gates.

**Tech Stack:** Rust, serde/serde_json, Tauri IPC models, Leptos `view!`.

---

### Task 1: Preserve raw event JSON

**Files:**
- Modify: `core/src/marketplace.rs`
- Modify: `core/src/nostr.rs`
- Modify: `app/src/models.rs`

- [ ] Add failing core test proving parsed NIP-99 listings retain raw event JSON in debug builds.
- [ ] Add debug-only `raw_event_json` to `Nip99Listing` and populate it in `parse_listing`.
- [ ] Add debug-only raw field to core and app `GameListing`, with serde default.
- [ ] Map the debug field through both `from_listing` implementations.

### Task 2: Render debug detail panel

**Files:**
- Modify: `app/src/components/detail.rs`

- [ ] Add a pure `nip99_debug_payload` formatter behind `#[cfg(debug_assertions)]`.
- [ ] Render a debug-only info box only for NIP-99 listings.
- [ ] Show pretty raw event JSON when available and valid; otherwise show parsed fields from `GameListing`.

### Task 3: Verify

- [ ] Run targeted failing test before implementation.
- [ ] Run `cargo fmt`.
- [ ] Run `cargo test -p arcadestr-core --lib nip99_listing_preserves_raw_event_json_for_debug_panel`.
- [ ] Run `cargo check -p arcadestr-core`.
- [ ] Run `cargo check -p arcadestr-app --features native`.
