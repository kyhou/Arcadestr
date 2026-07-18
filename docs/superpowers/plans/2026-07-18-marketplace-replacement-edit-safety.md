# Marketplace Replacement and Edit Safety Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Enforce identical replaceable-event ordering across every layer while preserving safe ADP edit and campaign state.

**Architecture:** A public core comparator owns NIP replaceable ordering and all in-memory consumers call it. SQLite mirrors the exact predicate in its conflict guard. Edit-only metadata and operator resolution flow through explicit IPC contracts, while pure frontend helpers update campaign state and safely format hashes.

**Tech Stack:** Rust, nostr-sdk, SQLx/SQLite, Leptos, Tauri IPC, Tokio tests.

**Workspace constraint:** Keep all changes uncommitted. Do not create a worktree or commit intermediate tasks.

---

### Task 1: Central Replaceable Ordering

**Files:**
- Modify: `core/src/marketplace.rs`
- Modify: `app/src/store/marketplace.rs`
- Modify: `app/src/ui_v2/views/marketplace_loader.rs`
- Modify: `app/src/campaign_management.rs`

- [ ] **Step 1: Add failing core comparator tests**

Add table-driven tests asserting newer timestamps win, older timestamps lose, lower IDs win timestamp ties, higher IDs lose ties, known IDs beat missing IDs on ties, and two missing IDs do not replace each other.

```rust
#[test]
fn replaceable_order_uses_timestamp_then_lowest_event_id() {
    assert!(is_replaceable_event_newer(11, Some("ff"), 10, Some("00")));
    assert!(!is_replaceable_event_newer(9, Some("00"), 10, Some("ff")));
    assert!(is_replaceable_event_newer(10, Some("00"), 10, Some("ff")));
    assert!(!is_replaceable_event_newer(10, Some("ff"), 10, Some("00")));
    assert!(is_replaceable_event_newer(10, Some("00"), 10, None));
    assert!(!is_replaceable_event_newer(10, None, 10, Some("00")));
    assert!(!is_replaceable_event_newer(10, None, 10, None));
}
```

- [ ] **Step 2: Verify the comparator test fails**

Run: `cargo test -p arcadestr-core --features native replaceable_order_uses_timestamp_then_lowest_event_id`

Expected: compilation fails because `is_replaceable_event_newer` does not exist.

- [ ] **Step 3: Implement the public comparator**

```rust
pub fn is_replaceable_event_newer(
    candidate_created_at: u64,
    candidate_event_id: Option<&str>,
    current_created_at: u64,
    current_event_id: Option<&str>,
) -> bool {
    match candidate_created_at.cmp(&current_created_at) {
        std::cmp::Ordering::Greater => true,
        std::cmp::Ordering::Less => false,
        std::cmp::Ordering::Equal => match (candidate_event_id, current_event_id) {
            (Some(candidate), Some(current)) => candidate < current,
            (Some(_), None) => true,
            (None, _) => false,
        },
    }
}
```

- [ ] **Step 4: Replace timestamp-only comparisons**

Store `(created_at, event_id)` in `fetch_nip99_listings_streaming_since`. Call the comparator from `MarketplaceStore::put_streaming`, `upsert_latest_listing`, and `current_user_listings`. Preserve coordinate keys and reject losing candidates before mutation.

- [ ] **Step 5: Add both-arrival-order tests to every consumer**

Each consumer test must feed lower-ID/higher-ID equal-timestamp listings in both orders and assert the lower ID remains. Also feed new-then-stale and assert the newer listing remains.

- [ ] **Step 6: Run focused ordering tests**

Run: `cargo test -p arcadestr-core --features native marketplace`

Run: `cargo test -p arcadestr-app marketplace`

Expected: all focused tests pass.

### Task 2: SQLite Stale-Replacement Guard

**Files:**
- Modify: `core/src/marketplace_cache.rs`

- [ ] **Step 1: Add failing cache regression tests**

Create one coordinate and upsert: newer then stale, equal timestamp lower ID then higher ID, and equal timestamp higher ID then lower ID. Assert `UpsertOutcome::Unchanged` for losing candidates and reload the winning title/event ID from SQLite.

- [ ] **Step 2: Verify the stale-cache test fails**

Run: `cargo test -p arcadestr-core --features native marketplace_cache::tests -- --test-threads=1`

Expected: the stale or higher-ID candidate incorrectly returns `Updated` and overwrites the row.

- [ ] **Step 3: Apply the exact ordering predicate to `ON CONFLICT`**

Keep the existing field-difference predicate, but require it inside this ordering guard:

```sql
WHERE (
    excluded.created_at > marketplace_listings.created_at
    OR (
        excluded.created_at = marketplace_listings.created_at
        AND excluded.source_event_id IS NOT NULL
        AND (
            marketplace_listings.source_event_id IS NULL
            OR excluded.source_event_id < marketplace_listings.source_event_id
        )
    )
)
AND (
    -- existing field-difference predicates
)
```

- [ ] **Step 4: Run cache tests**

Run: `cargo test -p arcadestr-core --features native marketplace_cache::tests -- --test-threads=1`

Expected: all cache tests pass and stale candidates report `Unchanged`.

### Task 3: Fulfillment Metadata and Operator Resolution

**Files:**
- Modify: `core/src/adp_publish.rs`
- Modify: `core/src/adp_storage.rs`
- Modify: `app/src/tauri_bridge.rs`
- Modify: `app/src/components/publish.rs`
- Modify: `desktop/src/adp_commands.rs`
- Modify: `desktop/src/main.rs`

- [ ] **Step 1: Add failing ADP event-builder test**

Extend `AdpListingInput` with `fulfillment_revoked_at: Option<u64>`, pass both existing timestamps, and assert the builder emits `fulfillment_valid_from` and `fulfillment_revoked_at` unchanged.

- [ ] **Step 2: Add failing provisioning lookup tests**

Add `AdpProvisioningRepository::for_fulfillment_scope(developer_npub, fulfillment_pubkey, scope) -> Result<Vec<AdpProvisioning>, AdpStorageError>`. Tests insert zero, one, and two matching rows and assert result counts `0`, `1`, and `2`; revoked rows remain eligible because edit restoration must preserve revocation.

- [ ] **Step 3: Add failing operator-resolution contract tests**

Define `ResolveAdpOperatorRequest { publisher_npub, fulfillment_pubkey, scope }`. The command converts publisher npub to hex, calls `for_fulfillment_scope`, and returns `Some(server_url)` only for exactly one match; zero or multiple matches return `None` so the form requires explicit selection.

- [ ] **Step 4: Verify the new tests fail**

Run: `cargo test -p arcadestr-core --features native adp_`

Run: `cargo test -p arcadestr-desktop resolve_adp_operator`

Expected: missing fields, repository method, and command cause failures.

- [ ] **Step 5: Preserve timestamps through publish IPC**

Add these fields to both frontend and desktop `PublishAdpListingRequest`:

```rust
pub existing_fulfillment_valid_from: Option<u64>,
pub existing_fulfillment_revoked_at: Option<u64>,
```

`PublishView` reads both specs and sends them on edit. `publish_adp_listing` uses the existing validity timestamp instead of `now` when supplied and always forwards existing revocation. `build_adp_listing_event_builder` emits the revocation tag. No normal edit path clears revocation.

- [ ] **Step 6: Resolve operator asynchronously on delegated edits**

Initialize `operator_url` empty. When an edited listing is delegated, invoke `resolve_adp_operator` with publisher, fulfillment key, and listing ID scope. Set the signal only for `Some(url)`; otherwise keep it empty and let existing validation require explicit selection. Do not mark every published server `auto_operator`.

- [ ] **Step 7: Run fulfillment tests**

Run: `cargo test -p arcadestr-core --features native adp_`

Run: `cargo test -p arcadestr-app publish`

Run: `cargo test -p arcadestr-desktop adp_`

Expected: all focused tests pass, including revoked metadata and ambiguous lookup cases.

### Task 4: Safe Hash and Campaign Local State

**Files:**
- Modify: `app/src/components/publish.rs`
- Modify: `app/src/campaign_management.rs`
- Modify: `app/src/ui_v2/views/publish.rs`

- [ ] **Step 1: Add failing hash-format tests**

Add `format_sha256(hash: &str) -> String`. Assert a valid 64-character ASCII hex hash renders as `first12...last12`; short, non-hex, and non-ASCII values render `Invalid SHA-256 metadata`.

- [ ] **Step 2: Add failing campaign-pointer mutation tests**

Add a pure helper:

```rust
pub fn apply_campaign_pointer_mutation(
    listing: &GameListing,
    root_event_id: &str,
    listing_event_id: &str,
    remove: bool,
) -> GameListing
```

Assert add is idempotent, remove deletes only the requested pointer, and both set `event_id` to the returned replacement event ID.

- [ ] **Step 3: Verify frontend tests fail**

Run: `cargo test -p arcadestr-app campaign_management`

Run: `cargo test -p arcadestr-app format_sha256`

Expected: helpers are missing.

- [ ] **Step 4: Implement safe formatting and local pointer updates**

Validate with `hash.len() == 64 && hash.bytes().all(|byte| byte.is_ascii_hexdigit())` before slicing. Replace direct slicing in `PublishView`.

For `invoke_update_campaign_pointer`, apply the helper with the returned event ID and navigate/update state. For campaign publish/cancel, apply the pointer mutation only when `listing_event_id` is present and `pointer_update_error` is absent. Keep campaign success visible when pointer publication fails.

- [ ] **Step 5: Run frontend tests and WASM check**

Run: `cargo test -p arcadestr-app`

Run: `cargo check -p arcadestr-app --target wasm32-unknown-unknown`

Expected: tests and WASM compilation pass.

### Task 5: Full Verification

**Files:**
- Verify all modified files.

- [ ] **Step 1: Format and inspect whitespace**

Run: `cargo fmt --all -- --check`

Run: `git diff --check`

Expected: both exit successfully without output.

- [ ] **Step 2: Run all target suites**

Run: `cargo test -p arcadestr-core --features native --all-targets`

Run: `cargo test -p arcadestr-app`

Run: `cargo test -p arcadestr-desktop`

Run: `cargo check -p arcadestr-app --target wasm32-unknown-unknown`

Expected: all commands exit successfully; existing warnings may remain.

- [ ] **Step 3: Validate the desktop flow**

Use Tauri MCP to confirm the edit form restores the existing hash/version/timestamps, resolves the unique local operator, never infers the first distribution server, and opens campaign creation without `Query timeout`. Confirm a malformed hash fixture renders safely if one is available without altering real relay data.

- [ ] **Step 4: Request final code review**

Review the working-tree diff against HEAD for ordering equivalence, revoked-key safety, operator ambiguity, campaign local state, and tests. Resolve all critical and important findings before completion.
