# NIP-58 Display Achievements Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the display-only NIP-58 achievements path: fetch verified earned badges, cache awards/definitions, and render profile showcase, achievements page, and a disabled post-purchase modal hook.

**Architecture:** Core owns parsing, validation, relay/cache orchestration, and storage APIs. Desktop adds thin command wrappers over pure command-contract functions. App adds display models, IPC wrappers, and Leptos views/components; web builds render an explicit unsupported badge relay state.

**Tech Stack:** Rust 2021, nostr/nostr-sdk 0.44, Tauri 2, Leptos 0.8 CSR, SQLite via existing storage layer, serde, thiserror, tracing.

---

## Required Context Before Coding

Load these files before any implementation edit:

- `/home/joel/.opencode/context/core/standards/code-quality.md`
- `/home/joel/.opencode/context/core/standards/test-coverage.md`
- `/home/joel/.opencode/context/core/standards/security-patterns.md`
- `/home/joel/.opencode/context/core/standards/documentation.md`
- `/home/joel/Sync/Projetos/Arcadestr/.opencode/context/project-intelligence/technical-domain.md`
- `/home/joel/Sync/Projetos/Arcadestr/AGENTS.md`
- `/home/joel/Sync/Projetos/Arcadestr/CODEBASE.md`
- `docs/superpowers/specs/2026-05-03-nip58-display-achievements-design.md`

NIP-58 / SDK constraint to preserve in implementation:

```rust
// Current NIP-58 profile badges use kind 10008.
// nostr-sdk 0.44 Kind::ProfileBadges maps to deprecated kind 30008.
let current_profile_badges_kind = Kind::Custom(10008);
```

---

## Step-Zero Lookup Result: Profile Pubkey Wiring

`app/src/ui_v2/views/profile.rs` currently derives the active profile pubkey as:

```rust
let current_npub = Signal::derive(move || auth.npub.get().unwrap_or_default());
```

Existing profile identity components such as `ProfileAvatar` and `ProfileDisplayName` accept an owned `String` prop named `npub`. `ProfileV2View` also passes a current pubkey value to embedded components with `current_npub.get()`.

`BadgeShowcase` needs to refetch when the profile pubkey changes, so use the existing `current_npub` signal directly instead of converting it to a one-time `String`:

```rust
#[component]
pub fn BadgeShowcase(profile_npub: Signal<String>) -> impl IntoView {
    // fetch reacts to profile_npub.get()
}
```

Embed it in `ProfileV2View` below the profile hero/bio section as:

```rust
<BadgeShowcase profile_npub=current_npub />
```

This follows the existing `Signal::derive` source of truth in `profile.rs` while preserving the refetch-on-change requirement.

---

## File Structure

Create:

- `core/src/achievements.rs` — NIP-58 models, parser, validation, relay/cache orchestration.
- `core/migrations/002_achievements.sql` — achievements cache schema, unless a newer migration exists when implementation starts; then use the next available number.
- `app/src/components/badge_showcase.rs` — profile badge showcase row.
- `app/src/components/badge_earned_modal.rs` — renderable badge-earned modal with disabled production trigger.
- `app/src/ui_v2/views/achievements.rs` — full achievements page.
- `desktop/tests/section8_badge_command_tests.rs` — command contract tests for display commands.

Modify:

- `core/src/lib.rs` — export `achievements` for native builds.
- `core/src/storage/db.rs` — apply schema and add cache methods if this file remains the current storage API entrypoint.
- `core/src/storage/migration.rs` — register or expose the achievements migration if the existing migration mechanism requires explicit registration.
- `core/src/wasm_stub.rs` — add display-safe stubs only if shared app imports require them.
- `core/tests/integration.rs` — add parser/cache/fetch tests, or create an achievements-specific integration test if the existing file is too large.
- `app/src/models.rs` — add IPC-facing display models.
- `app/src/tauri_bridge.rs` — add typed wrappers with desktop and web implementations.
- `app/src/components/mod.rs` — export new components.
- `app/src/ui_v2/views/profile.rs` — embed `BadgeShowcase`.
- `app/src/ui_v2/views/game_detail.rs` — add disabled/debug-only modal integration point.
- `app/src/ui_v2/views/mod.rs` — export `AchievementsView`.
- `app/src/ui_v2/shell.rs` — add `UiV2View::Achievements` and sidebar navigation.
- `desktop/src/command_contracts.rs` — add pure display command functions.
- `desktop/src/main.rs` — add thin Tauri command wrappers and register them.

Naming conventions:

- File: `app/src/ui_v2/views/achievements.rs`
- Component export: `AchievementsView`
- Enum variant: `UiV2View::Achievements`
- Sidebar label: `"Achievements"`
- Command strings: `fetch_earned_badges`, `fetch_profile_badges`

---

## Task 1: Core NIP-58 Models and Parser Tests

**Files:**

- Create/modify: `core/src/achievements.rs`
- Modify: `core/src/lib.rs`
- Test: `core/tests/integration.rs`

- [ ] **Step 1: Add failing parser tests**

Add tests that construct nostr events with `EventBuilder` and assert parsing behavior:

```rust
#[test]
fn parse_valid_badge_definition_extracts_nip58_tags() {
    let keys = nostr::Keys::generate();
    let event = nostr::EventBuilder::new(
        nostr::Kind::Custom(30009),
        "",
    )
    .tags([
        nostr::Tag::custom(nostr::TagKind::d(), ["first_clear"]),
        nostr::Tag::custom(nostr::TagKind::Custom("name".into()), ["First Clear"]),
        nostr::Tag::custom(nostr::TagKind::Custom("description".into()), ["Finished a game once"]),
        nostr::Tag::custom(nostr::TagKind::Custom("image".into()), ["https://example.com/badge.png", "1024x1024"]),
        nostr::Tag::custom(nostr::TagKind::Custom("thumb".into()), ["https://example.com/badge-thumb.png", "256x256"]),
    ])
    .sign_with_keys(&keys)
    .expect("test event signs");

    let definition = arcadestr_core::achievements::parse_badge_definition(&event, Some("wss://relay.example.com".to_string()))
        .expect("definition parses");

    assert_eq!(definition.badge_id, "first_clear");
    assert_eq!(definition.issuer_pubkey, keys.public_key().to_hex());
    assert_eq!(definition.coordinate, format!("30009:{}:first_clear", keys.public_key().to_hex()));
    assert_eq!(definition.name.as_deref(), Some("First Clear"));
    assert_eq!(definition.image_dimensions.as_deref(), Some("1024x1024"));
}

#[test]
fn reject_badge_definition_without_non_empty_d_tag() {
    let keys = nostr::Keys::generate();
    let event = nostr::EventBuilder::new(nostr::Kind::Custom(30009), "")
        .sign_with_keys(&keys)
        .expect("test event signs");

    let error = arcadestr_core::achievements::parse_badge_definition(&event, None)
        .expect_err("missing d tag should fail");

    assert!(error.to_string().contains("d tag"));
}

#[test]
fn parse_profile_badges_requires_immediate_a_then_e_pairs() {
    let owner = nostr::Keys::generate();
    let event = nostr::EventBuilder::new(nostr::Kind::Custom(10008), "")
        .tags([
            nostr::Tag::custom(nostr::TagKind::a(), ["30009:issuer:first"]),
            nostr::Tag::custom(nostr::TagKind::e(), ["aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa", "wss://relay.example.com"]),
            nostr::Tag::custom(nostr::TagKind::e(), ["bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"]),
            nostr::Tag::custom(nostr::TagKind::a(), ["30009:issuer:orphan"]),
        ])
        .sign_with_keys(&owner)
        .expect("test event signs");

    let list = arcadestr_core::achievements::parse_profile_badge_list(
        &event,
        &owner.public_key().to_hex(),
    )
    .expect("profile list parses");

    assert_eq!(list.entries.len(), 1);
    assert_eq!(list.entries[0].badge_coordinate, "30009:issuer:first");
    assert_eq!(list.entries[0].display_order, 0);
}
```

- [ ] **Step 2: Run parser tests and verify they fail**

Run:

```bash
cargo test -p arcadestr-core parse_valid_badge_definition_extracts_nip58_tags -- --test-threads=1
```

Expected: fail because `arcadestr_core::achievements` does not exist.

- [ ] **Step 3: Implement minimal core models and parsing API**

Create `core/src/achievements.rs` with these public types and parser functions:

```rust
//! NIP-58 badge parsing, validation, relay fetching, and cache coordination.

use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const KIND_BADGE_AWARD: u16 = 8;
pub const KIND_PROFILE_BADGES_CURRENT: u16 = 10008;
pub const KIND_PROFILE_BADGES_DEPRECATED: u16 = 30008;
pub const KIND_BADGE_DEFINITION: u16 = 30009;
pub const PROFILE_BADGES_DEPRECATED_D: &str = "profile_badges";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BadgeDefinition {
    pub coordinate: String,
    pub issuer_pubkey: String,
    pub badge_id: String,
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BadgeAward {
    pub event_id: String,
    pub issuer_pubkey: String,
    pub recipient_pubkey: String,
    pub badge_coordinate: String,
    pub relay_url: Option<String>,
    pub created_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProfileBadgeSelection {
    pub badge_coordinate: String,
    pub award_event_id: String,
    pub relay_url: Option<String>,
    pub display_order: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProfileBadgeList {
    pub profile_pubkey: String,
    pub event_id: String,
    pub kind: u16,
    pub created_at: u64,
    pub entries: Vec<ProfileBadgeSelection>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProfileBadgeEntry {
    pub definition: BadgeDefinition,
    pub award: BadgeAward,
    pub display_order: usize,
    pub visible: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EarnedBadgeSummary {
    pub definition: BadgeDefinition,
    pub award: BadgeAward,
    pub visible_on_profile: bool,
}

#[derive(Debug, Error)]
pub enum AchievementError {
    #[error("badge definition must be kind 30009")]
    InvalidDefinitionKind,
    #[error("badge definition missing non-empty d tag")]
    MissingDefinitionDTag,
    #[error("badge award must be kind 8")]
    InvalidAwardKind,
    #[error("badge award missing a tag")]
    MissingAwardCoordinate,
    #[error("badge award missing p tag for recipient")]
    MissingAwardRecipient,
    #[error("profile badge event pubkey does not match profile owner")]
    ProfileOwnerMismatch,
    #[error("award issuer does not match definition issuer")]
    IssuerMismatch,
    #[error("relay error: {0}")]
    Relay(String),
    #[error("storage error: {0}")]
    Storage(String),
}
```

Implement tag helpers using `event.tags.iter()` and string matching for `d`, `a`, `e`, `p`, `name`, `description`, `image`, and `thumb`. The parser for profile badges must walk the ordered tag list by index and only accept an `a` tag immediately followed by an `e` tag.

- [ ] **Step 4: Export the module**

Modify `core/src/lib.rs`:

```rust
#[cfg(feature = "native")]
pub mod achievements;
```

- [ ] **Step 5: Run parser tests**

Run:

```bash
cargo test -p arcadestr-core parse_valid_badge_definition_extracts_nip58_tags -- --test-threads=1
cargo test -p arcadestr-core reject_badge_definition_without_non_empty_d_tag -- --test-threads=1
cargo test -p arcadestr-core parse_profile_badges_requires_immediate_a_then_e_pairs -- --test-threads=1
```

Expected: all three tests pass.

- [ ] **Step 6: Commit parser foundation**

```bash
git add core/src/lib.rs core/src/achievements.rs core/tests/integration.rs
git commit -m "feat: parse NIP-58 badge events"
```

---

## Task 2: Award Validation and Cache Schema

**Files:**

- Modify: `core/src/achievements.rs`
- Create: `core/migrations/002_achievements.sql` or next available migration number
- Modify: `core/src/storage/db.rs`
- Test: `core/tests/integration.rs`

- [ ] **Step 1: Add failing validation and schema tests**

Add tests for issuer mismatch and profile-list replacement:

```rust
#[test]
fn issuer_mismatch_excludes_award_before_cache() {
    let issuer = nostr::Keys::generate();
    let attacker = nostr::Keys::generate();
    let recipient = nostr::Keys::generate();

    let definition = arcadestr_core::achievements::BadgeDefinition {
        coordinate: format!("30009:{}:first_clear", issuer.public_key().to_hex()),
        issuer_pubkey: issuer.public_key().to_hex(),
        badge_id: "first_clear".to_string(),
        name: Some("First Clear".to_string()),
        description: None,
        image_url: None,
        image_dimensions: None,
        thumb_url: None,
        thumb_dimensions: None,
        relay_url: None,
        event_id: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string(),
        created_at: 1,
    };

    let award = arcadestr_core::achievements::BadgeAward {
        event_id: "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".to_string(),
        issuer_pubkey: attacker.public_key().to_hex(),
        recipient_pubkey: recipient.public_key().to_hex(),
        badge_coordinate: definition.coordinate.clone(),
        relay_url: None,
        created_at: 2,
    };

    let result = arcadestr_core::achievements::validate_award_issuer(&award, &definition);
    assert!(result.is_err());
}
```

- [ ] **Step 2: Run validation test and verify it fails**

Run:

```bash
cargo test -p arcadestr-core issuer_mismatch_excludes_award_before_cache -- --test-threads=1
```

Expected: fail because `validate_award_issuer` does not exist.

- [ ] **Step 3: Add achievements migration**

Create the next migration. If only `001_initial_schema.sql` exists, use `core/migrations/002_achievements.sql`:

```sql
-- NIP-58 achievement badge cache

CREATE TABLE IF NOT EXISTS badge_definitions (
    coordinate TEXT PRIMARY KEY,
    issuer_pubkey TEXT NOT NULL,
    badge_id TEXT NOT NULL,
    event_id TEXT NOT NULL,
    name TEXT,
    description TEXT,
    image_url TEXT,
    image_dimensions TEXT,
    thumb_url TEXT,
    thumb_dimensions TEXT,
    relay_url TEXT,
    created_at INTEGER NOT NULL,
    raw_event_json TEXT NOT NULL,
    UNIQUE(issuer_pubkey, badge_id)
);

CREATE INDEX IF NOT EXISTS idx_badge_definitions_issuer_badge
    ON badge_definitions(issuer_pubkey, badge_id);

CREATE TABLE IF NOT EXISTS badge_awards (
    event_id TEXT PRIMARY KEY,
    issuer_pubkey TEXT NOT NULL,
    recipient_pubkey TEXT NOT NULL,
    badge_coordinate TEXT NOT NULL REFERENCES badge_definitions(coordinate),
    relay_url TEXT,
    created_at INTEGER NOT NULL,
    raw_event_json TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_badge_awards_recipient_created
    ON badge_awards(recipient_pubkey, created_at DESC);

CREATE INDEX IF NOT EXISTS idx_badge_awards_coordinate
    ON badge_awards(badge_coordinate);

CREATE TABLE IF NOT EXISTS profile_badge_lists (
    profile_pubkey TEXT PRIMARY KEY,
    event_id TEXT NOT NULL,
    kind INTEGER NOT NULL,
    created_at INTEGER NOT NULL,
    raw_event_json TEXT NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS profile_badge_entries (
    profile_pubkey TEXT NOT NULL REFERENCES profile_badge_lists(profile_pubkey) ON DELETE CASCADE,
    badge_coordinate TEXT NOT NULL,
    award_event_id TEXT NOT NULL,
    relay_url TEXT,
    display_order INTEGER NOT NULL,
    PRIMARY KEY(profile_pubkey, display_order)
);

CREATE INDEX IF NOT EXISTS idx_profile_badge_entries_coordinate
    ON profile_badge_entries(profile_pubkey, badge_coordinate);

CREATE INDEX IF NOT EXISTS idx_profile_badge_entries_award
    ON profile_badge_entries(award_event_id);
```

- [ ] **Step 4: Implement validation and cache-method signatures**

Add to `core/src/achievements.rs`:

```rust
pub fn validate_award_issuer(
    award: &BadgeAward,
    definition: &BadgeDefinition,
) -> Result<(), AchievementError> {
    if award.issuer_pubkey == definition.issuer_pubkey {
        Ok(())
    } else {
        Err(AchievementError::IssuerMismatch)
    }
}
```

Add storage methods in the existing storage API. Method names to expose:

```rust
pub async fn cache_badge_definition(
    &self,
    definition: &crate::achievements::BadgeDefinition,
    raw_event_json: &str,
) -> Result<(), DatabaseError>;

pub async fn cache_badge_award(
    &self,
    award: &crate::achievements::BadgeAward,
    raw_event_json: &str,
) -> Result<(), DatabaseError>;

pub async fn cache_profile_badge_list(
    &self,
    list: &crate::achievements::ProfileBadgeList,
    raw_event_json: &str,
) -> Result<(), DatabaseError>;

pub async fn earned_badges_for_profile(
    &self,
    profile_pubkey: &str,
) -> Result<Vec<crate::achievements::EarnedBadgeSummary>, DatabaseError>;

pub async fn profile_badges_for_profile(
    &self,
    profile_pubkey: &str,
) -> Result<Vec<crate::achievements::ProfileBadgeEntry>, DatabaseError>;
```

Definition upsert condition must preserve latest-by-coordinate behavior:

```sql
ON CONFLICT(coordinate) DO UPDATE SET
    issuer_pubkey = excluded.issuer_pubkey,
    badge_id = excluded.badge_id,
    event_id = excluded.event_id,
    name = excluded.name,
    description = excluded.description,
    image_url = excluded.image_url,
    image_dimensions = excluded.image_dimensions,
    thumb_url = excluded.thumb_url,
    thumb_dimensions = excluded.thumb_dimensions,
    relay_url = excluded.relay_url,
    created_at = excluded.created_at,
    raw_event_json = excluded.raw_event_json
WHERE excluded.created_at > badge_definitions.created_at
   OR (excluded.created_at = badge_definitions.created_at
       AND excluded.event_id < badge_definitions.event_id)
```

Profile list cache must run in one transaction: delete entries for `profile_pubkey`, upsert `profile_badge_lists`, insert new `profile_badge_entries`.

- [ ] **Step 5: Run storage tests**

Run:

```bash
cargo test -p arcadestr-core issuer_mismatch_excludes_award_before_cache -- --test-threads=1
cargo test -p arcadestr-core -- --test-threads=1
```

Expected: targeted test passes; full core tests pass or reveal unrelated failures to report before fixing.

- [ ] **Step 6: Commit storage foundation**

```bash
git add core/src/achievements.rs core/src/storage/db.rs core/src/storage/migration.rs core/migrations/*_achievements.sql core/tests/integration.rs
git commit -m "feat: cache NIP-58 badge data"
```

---

## Task 3: Relay Fetch Orchestration and Desktop Commands

**Files:**

- Modify: `core/src/achievements.rs`
- Modify: `desktop/src/command_contracts.rs`
- Modify: `desktop/src/main.rs`
- Test: `desktop/tests/section8_badge_command_tests.rs`

- [ ] **Step 1: Add failing command contract tests**

Create `desktop/tests/section8_badge_command_tests.rs`:

```rust
use serde_json::json;

#[path = "../src/command_contracts.rs"]
mod command_contracts;

#[test]
fn fetch_earned_badges_command_name_serializes_empty_vec() {
    let payload: Vec<arcadestr_core::achievements::EarnedBadgeSummary> = Vec::new();
    let value = command_contracts::serialize_fetch_earned_badges_result(&payload)
        .expect("earned badge payload serializes");

    assert_eq!(value, json!([]));
}

#[test]
fn fetch_profile_badges_command_name_serializes_empty_vec() {
    let payload: Vec<arcadestr_core::achievements::ProfileBadgeEntry> = Vec::new();
    let value = command_contracts::serialize_fetch_profile_badges_result(&payload)
        .expect("profile badge payload serializes");

    assert_eq!(value, json!([]));
}
```

- [ ] **Step 2: Run command tests and verify they fail**

Run:

```bash
cargo test -p arcadestr-desktop fetch_earned_badges_command_name_serializes_empty_vec
```

Expected: fail because serializer helpers do not exist.

- [ ] **Step 3: Implement relay fetch functions**

In `core/src/achievements.rs`, add native functions with signatures:

```rust
#[cfg(feature = "native")]
pub async fn fetch_user_badges(
    relay_manager: &crate::relay_manager::RelayManager,
    database: &crate::storage::Database,
    profile_pubkey: &str,
) -> Result<Vec<EarnedBadgeSummary>, AchievementError>;

#[cfg(feature = "native")]
pub async fn fetch_profile_badges(
    relay_manager: &crate::relay_manager::RelayManager,
    database: &crate::storage::Database,
    profile_pubkey: &str,
) -> Result<Vec<ProfileBadgeEntry>, AchievementError>;
```

Fetch filters must include:

```rust
let awards_filter = nostr_sdk::Filter::new()
    .kind(nostr_sdk::Kind::BadgeAward)
    .pubkey(profile_pubkey_public_key);

let profile_badges_filter = nostr_sdk::Filter::new()
    .kind(nostr_sdk::Kind::Custom(KIND_PROFILE_BADGES_CURRENT))
    .author(profile_pubkey_public_key)
    .limit(1);

let deprecated_profile_badges_filter = nostr_sdk::Filter::new()
    .kind(nostr_sdk::Kind::ProfileBadges)
    .author(profile_pubkey_public_key)
    .identifier(PROFILE_BADGES_DEPRECATED_D)
    .limit(1);
```

Do not hold a mutex guard across `.await`: clone or capture the `RelayManager`/client handle before awaiting. Fetch missing definitions only for coordinates referenced by awards/profile lists. Exclude and log issuer mismatches before caching awards.

- [ ] **Step 4: Implement command contract helpers**

In `desktop/src/command_contracts.rs` add:

```rust
pub fn serialize_fetch_earned_badges_result(
    badges: &[arcadestr_core::achievements::EarnedBadgeSummary],
) -> Result<serde_json::Value, CommandError> {
    serde_json::to_value(badges).map_err(|error| CommandError::InvalidInput(error.to_string()))
}

pub fn serialize_fetch_profile_badges_result(
    badges: &[arcadestr_core::achievements::ProfileBadgeEntry],
) -> Result<serde_json::Value, CommandError> {
    serde_json::to_value(badges).map_err(|error| CommandError::InvalidInput(error.to_string()))
}

pub async fn fetch_earned_badges(
    state: &crate::AppState,
    profile_pubkey: String,
) -> Result<Vec<arcadestr_core::achievements::EarnedBadgeSummary>, CommandError>;

pub async fn fetch_profile_badges(
    state: &crate::AppState,
    profile_pubkey: String,
) -> Result<Vec<arcadestr_core::achievements::ProfileBadgeEntry>, CommandError>;
```

Implementation must snapshot relay/database handles before awaiting and map `AchievementError` into `CommandError::InvalidInput` or a new `CommandError::Achievements(String)` variant.

- [ ] **Step 5: Add thin Tauri wrappers and register commands**

In `desktop/src/main.rs` add wrappers matching the command strings:

```rust
#[tauri::command]
async fn fetch_earned_badges(
    state: tauri::State<'_, AppState>,
    profile_pubkey: String,
) -> Result<Vec<arcadestr_core::achievements::EarnedBadgeSummary>, String> {
    command_contracts::fetch_earned_badges(state.inner(), profile_pubkey)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
async fn fetch_profile_badges(
    state: tauri::State<'_, AppState>,
    profile_pubkey: String,
) -> Result<Vec<arcadestr_core::achievements::ProfileBadgeEntry>, String> {
    command_contracts::fetch_profile_badges(state.inner(), profile_pubkey)
        .await
        .map_err(|error| error.to_string())
}
```

Add both names to `tauri::generate_handler![...]`.

- [ ] **Step 6: Run command checks**

Run:

```bash
cargo test -p arcadestr-desktop fetch_earned_badges_command_name_serializes_empty_vec
cargo check -p arcadestr-desktop
```

Expected: tests and check pass.

- [ ] **Step 7: Commit command layer**

```bash
git add core/src/achievements.rs desktop/src/command_contracts.rs desktop/src/main.rs desktop/tests/section8_badge_command_tests.rs
git commit -m "feat: add badge display commands"
```

---

## Task 4: App Models and IPC Wrappers

**Files:**

- Modify: `app/src/models.rs`
- Modify: `app/src/tauri_bridge.rs`

- [ ] **Step 1: Add app models**

In `app/src/models.rs`, add display-only IPC structs:

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BadgeDefinition {
    pub coordinate: String,
    pub issuer_pubkey: String,
    pub badge_id: String,
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

Do not add event-notification payloads or issuance/update request models in this session.

- [ ] **Step 2: Add typed bridge wrappers**

In `app/src/tauri_bridge.rs`:

```rust
use crate::models::{
    EarnedBadgeSummary, ProfileBadgeEntry,
    Nip05Status, Nip49ExportResult, Nip49ImportRequest,
};

#[cfg(not(feature = "web"))]
pub async fn fetch_earned_badges(profile_pubkey: String) -> Result<Vec<EarnedBadgeSummary>, String> {
    crate::tauri_invoke::invoke(
        "fetch_earned_badges",
        serde_json::json!({ "profilePubkey": profile_pubkey }),
    )
    .await
}

#[cfg(feature = "web")]
pub async fn fetch_earned_badges(_profile_pubkey: String) -> Result<Vec<EarnedBadgeSummary>, String> {
    Err("Badge relay display is not yet available on the web target.".to_string())
}

#[cfg(not(feature = "web"))]
pub async fn fetch_profile_badges(profile_pubkey: String) -> Result<Vec<ProfileBadgeEntry>, String> {
    crate::tauri_invoke::invoke(
        "fetch_profile_badges",
        serde_json::json!({ "profilePubkey": profile_pubkey }),
    )
    .await
}

#[cfg(feature = "web")]
pub async fn fetch_profile_badges(_profile_pubkey: String) -> Result<Vec<ProfileBadgeEntry>, String> {
    Err("Badge relay display is not yet available on the web target.".to_string())
}
```

- [ ] **Step 3: Run app check**

Run:

```bash
cargo check -p arcadestr-app
```

Expected: app crate compiles or reports model import errors to fix in this task.

- [ ] **Step 4: Commit IPC models**

```bash
git add app/src/models.rs app/src/tauri_bridge.rs
git commit -m "feat: add badge IPC models"
```

---

## Task 5: BadgeShowcase Component and Profile Integration

**Files:**

- Create: `app/src/components/badge_showcase.rs`
- Modify: `app/src/components/mod.rs`
- Modify: `app/src/ui_v2/views/profile.rs`

- [ ] **Step 1: Create BadgeShowcase component**

Create `app/src/components/badge_showcase.rs`:

```rust
//! Profile badge showcase for NIP-58 achievements.

use crate::models::{EarnedBadgeSummary, ProfileBadgeEntry};
use leptos::prelude::*;
use wasm_bindgen_futures::spawn_local;

const SHOWCASE_FALLBACK_LIMIT: usize = 8;

#[derive(Debug, Clone)]
enum BadgeShowcaseState {
    Loading,
    Empty,
    Error(String),
    Ready(Vec<EarnedBadgeSummary>),
    WebUnavailable,
}

#[component]
pub fn BadgeShowcase(profile_npub: Signal<String>) -> impl IntoView {
    let state = RwSignal::new(BadgeShowcaseState::Loading);

    #[cfg(feature = "web")]
    {
        state.set(BadgeShowcaseState::WebUnavailable);
    }

    #[cfg(not(feature = "web"))]
    Effect::new(move |_| {
        let npub = profile_npub.get();
        if npub.is_empty() {
            state.set(BadgeShowcaseState::Empty);
            return;
        }

        state.set(BadgeShowcaseState::Loading);
        spawn_local(async move {
            match crate::tauri_bridge::fetch_profile_badges(npub.clone()).await {
                Ok(profile_entries) if !profile_entries.is_empty() => {
                    state.set(BadgeShowcaseState::Ready(profile_entries_to_summaries(profile_entries)));
                }
                Ok(_) => match crate::tauri_bridge::fetch_earned_badges(npub).await {
                    Ok(mut earned) => {
                        earned.truncate(SHOWCASE_FALLBACK_LIMIT);
                        if earned.is_empty() {
                            state.set(BadgeShowcaseState::Empty);
                        } else {
                            state.set(BadgeShowcaseState::Ready(earned));
                        }
                    }
                    Err(error) => state.set(BadgeShowcaseState::Error(error)),
                },
                Err(error) => state.set(BadgeShowcaseState::Error(error)),
            }
        });
    });

    view! {
        <section class="v2-panel v2-badge-showcase">
            <div class="v2-profile-listings-header">
                <h3>"Achievements"</h3>
            </div>
            {move || match state.get() {
                BadgeShowcaseState::Loading => view! { <p>"Loading achievements..."</p> }.into_any(),
                BadgeShowcaseState::Empty => view! { <p>"No verified badges yet."</p> }.into_any(),
                BadgeShowcaseState::Error(error) => view! { <p>{format!("Failed to load badges: {error}")}</p> }.into_any(),
                BadgeShowcaseState::WebUnavailable => view! {
                    <p class="badge-showcase-unavailable">
                        "Badge relay display is not yet available on the web target. Badges will appear here once web relay support is added."
                    </p>
                }.into_any(),
                BadgeShowcaseState::Ready(badges) => view! {
                    <div class="v2-badge-showcase-row">
                        {badges.into_iter().map(render_badge_chip).collect::<Vec<_>>()}
                    </div>
                }.into_any(),
            }}
        </section>
    }
}

fn profile_entries_to_summaries(entries: Vec<ProfileBadgeEntry>) -> Vec<EarnedBadgeSummary> {
    entries
        .into_iter()
        .map(|entry| EarnedBadgeSummary {
            definition: entry.definition,
            award: entry.award,
            visible_on_profile: entry.visible,
        })
        .collect()
}

fn render_badge_chip(badge: EarnedBadgeSummary) -> impl IntoView {
    let image = badge
        .definition
        .thumb_url
        .clone()
        .or_else(|| badge.definition.image_url.clone());
    let name = badge
        .definition
        .name
        .clone()
        .unwrap_or_else(|| badge.definition.badge_id.clone());

    view! {
        <article class="v2-badge-chip">
            {image.map(|src| view! { <img src=src alt=name.clone() /> })}
            <div>
                <strong>{name}</strong>
                <span>{short_pubkey(&badge.definition.issuer_pubkey)}</span>
            </div>
        </article>
    }
}

fn short_pubkey(pubkey: &str) -> String {
    if pubkey.len() <= 12 {
        pubkey.to_string()
    } else {
        format!("{}…{}", &pubkey[..6], &pubkey[pubkey.len() - 6..])
    }
}
```

- [ ] **Step 2: Export component**

Modify `app/src/components/mod.rs`:

```rust
pub mod badge_showcase;
pub use badge_showcase::BadgeShowcase;
```

- [ ] **Step 3: Embed below profile hero section**

Modify `app/src/ui_v2/views/profile.rs` imports:

```rust
#[path = "../../components/badge_showcase.rs"]
mod badge_showcase;
use badge_showcase::BadgeShowcase;
```

Add immediately after the profile hero `</header>`:

```rust
<BadgeShowcase profile_npub=current_npub />
```

- [ ] **Step 4: Run app check**

Run:

```bash
cargo check -p arcadestr-app
```

Expected: compiles. If Leptos view type errors occur, fix only this component and profile integration.

- [ ] **Step 5: Commit showcase UI**

```bash
git add app/src/components/badge_showcase.rs app/src/components/mod.rs app/src/ui_v2/views/profile.rs
git commit -m "feat: show profile badges"
```

---

## Task 6: Achievements Page and Navigation

**Files:**

- Create: `app/src/ui_v2/views/achievements.rs`
- Modify: `app/src/ui_v2/views/mod.rs`
- Modify: `app/src/ui_v2/shell.rs`

- [ ] **Step 1: Create AchievementsView**

Create `app/src/ui_v2/views/achievements.rs`:

```rust
//! Full achievements page for verified NIP-58 badges.

use crate::models::EarnedBadgeSummary;
use crate::AuthContext;
use leptos::prelude::*;
use wasm_bindgen_futures::spawn_local;

#[derive(Debug, Clone)]
enum AchievementsState {
    Loading,
    Empty,
    Error(String),
    Ready(Vec<EarnedBadgeSummary>),
    WebUnavailable,
}

#[component]
pub fn AchievementsView() -> impl IntoView {
    let auth = use_context::<AuthContext>().expect("AuthContext not provided");
    let state = RwSignal::new(AchievementsState::Loading);

    #[cfg(feature = "web")]
    {
        state.set(AchievementsState::WebUnavailable);
    }

    #[cfg(not(feature = "web"))]
    Effect::new(move |_| {
        let npub = auth.npub.get().unwrap_or_default();
        if npub.is_empty() {
            state.set(AchievementsState::Empty);
            return;
        }

        state.set(AchievementsState::Loading);
        spawn_local(async move {
            match crate::tauri_bridge::fetch_earned_badges(npub).await {
                Ok(badges) if badges.is_empty() => state.set(AchievementsState::Empty),
                Ok(badges) => state.set(AchievementsState::Ready(badges)),
                Err(error) => state.set(AchievementsState::Error(error)),
            }
        });
    });

    view! {
        <section class="v2-achievements-view">
            <div class="v2-panel-glass">
                <h1 class="v2-display">"Achievements"</h1>
                <p>"Verified NIP-58 badges awarded by game publishers and other Nostr identities."</p>
            </div>

            {move || match state.get() {
                AchievementsState::Loading => view! { <div class="v2-panel"><p>"Loading achievements..."</p></div> }.into_any(),
                AchievementsState::Empty => view! { <div class="v2-panel"><p>"No verified badges found for this profile."</p></div> }.into_any(),
                AchievementsState::Error(error) => view! { <div class="v2-panel"><p>{format!("Failed to load achievements: {error}")}</p></div> }.into_any(),
                AchievementsState::WebUnavailable => view! {
                    <div class="v2-panel"><p>"Badge relay display is not yet available on the web target."</p></div>
                }.into_any(),
                AchievementsState::Ready(badges) => view! {
                    <div class="v2-achievements-grid">
                        {badges.into_iter().map(render_achievement_card).collect::<Vec<_>>()}
                    </div>
                }.into_any(),
            }}
        </section>
    }
}

fn render_achievement_card(badge: EarnedBadgeSummary) -> impl IntoView {
    let image = badge
        .definition
        .image_url
        .clone()
        .or_else(|| badge.definition.thumb_url.clone());
    let name = badge
        .definition
        .name
        .clone()
        .unwrap_or_else(|| badge.definition.badge_id.clone());
    let description = badge
        .definition
        .description
        .clone()
        .unwrap_or_else(|| "Verified badge award".to_string());
    let award_date = format!("Awarded at {}", badge.award.created_at);

    view! {
        <article class="v2-panel v2-achievement-card">
            {image.map(|src| view! { <img src=src alt=name.clone() /> })}
            <h3>{name}</h3>
            <p>{description}</p>
            <p>{format!("Issuer: {}", badge.definition.issuer_pubkey)}</p>
            <p>{award_date}</p>
        </article>
    }
}
```

- [ ] **Step 2: Export view**

Modify `app/src/ui_v2/views/mod.rs`:

```rust
pub mod achievements;
pub use achievements::AchievementsView;
```

- [ ] **Step 3: Add navigation variant and sidebar item**

Modify `app/src/ui_v2/shell.rs` imports:

```rust
use crate::ui_v2::views::{
    AchievementsView, BrowseGamesView, GameDetailView, LibraryView, ProfileV2View,
    PublishV2View, SocialView, StoreFrontView,
};
```

Add enum variant:

```rust
Achievements,
```

Add setter:

```rust
let set_achievements = move |_| current_view.set(UiV2View::Achievements);
```

Add sidebar item after Profile:

```rust
<NavItem
    label="Achievements"
    icon="emoji_events"
    active={Signal::derive(move || current_view.get() == UiV2View::Achievements)}
    on_click={Callback::new(set_achievements)}
/>
```

Add match arm:

```rust
UiV2View::Achievements => {
    view! { <div class="max-w-[1600px] mx-auto p-8"><AchievementsView /></div> }
        .into_any()
}
```

- [ ] **Step 4: Run app check**

Run:

```bash
cargo check -p arcadestr-app
```

Expected: app crate compiles.

- [ ] **Step 5: Commit achievements page**

```bash
git add app/src/ui_v2/views/achievements.rs app/src/ui_v2/views/mod.rs app/src/ui_v2/shell.rs
git commit -m "feat: add achievements page"
```

---

## Task 7: BadgeEarnedModal and Disabled Game Detail Hook

**Files:**

- Create: `app/src/components/badge_earned_modal.rs`
- Modify: `app/src/components/mod.rs`
- Modify: `app/src/ui_v2/views/game_detail.rs`

- [ ] **Step 1: Create modal component**

Create `app/src/components/badge_earned_modal.rs`:

```rust
//! Renderable badge-earned celebration modal.

use crate::models::EarnedBadgeSummary;
use leptos::prelude::*;

#[component]
pub fn BadgeEarnedModal(
    badge: ReadSignal<Option<EarnedBadgeSummary>>,
    on_close: Callback<()>,
) -> impl IntoView {
    view! {
        <Show when=move || badge.get().is_some()>
            <div class="v2-modal-backdrop">
                <div class="v2-panel v2-badge-earned-modal">
                    {move || badge.get().map(|badge| {
                        let image = badge.definition.image_url.clone().or_else(|| badge.definition.thumb_url.clone());
                        let name = badge.definition.name.clone().unwrap_or_else(|| badge.definition.badge_id.clone());
                        let description = badge.definition.description.clone().unwrap_or_else(|| "A verified NIP-58 badge was awarded.".to_string());
                        view! {
                            <>
                                <p class="v2-kicker">"Achievement unlocked"</p>
                                {image.map(|src| view! { <img src=src alt=name.clone() /> })}
                                <h2>{name}</h2>
                                <p>{description}</p>
                                <p>{format!("Issuer: {}", badge.definition.issuer_pubkey)}</p>
                                <button class="v2-btn-primary" on:click=move |_| on_close.run(())>
                                    "Close"
                                </button>
                            </>
                        }
                    })}
                </div>
            </div>
        </Show>
    }
}
```

- [ ] **Step 2: Export modal**

Modify `app/src/components/mod.rs`:

```rust
pub mod badge_earned_modal;
pub use badge_earned_modal::BadgeEarnedModal;
```

- [ ] **Step 3: Integrate disabled hook in game detail**

In `app/src/ui_v2/views/game_detail.rs`, create a signal near other purchase state:

```rust
let earned_badge_preview = RwSignal::new(None::<crate::models::EarnedBadgeSummary>);
let close_badge_modal = Callback::new(move |_| earned_badge_preview.set(None));
```

At the post-purchase confirmation point add this exact comment and keep production trigger disabled:

```rust
// Follow-up: wire to kind-8 relay subscription when badge issuance lands.
#[cfg(debug_assertions)]
{
    let _badge_modal_preview_signal = earned_badge_preview;
}
```

Render the modal in the view tree:

```rust
<crate::components::BadgeEarnedModal
    badge=earned_badge_preview.read_only()
    on_close=close_badge_modal
/>
```

- [ ] **Step 4: Run app check**

Run:

```bash
cargo check -p arcadestr-app
```

Expected: app crate compiles.

- [ ] **Step 5: Commit modal hook**

```bash
git add app/src/components/badge_earned_modal.rs app/src/components/mod.rs app/src/ui_v2/views/game_detail.rs
git commit -m "feat: add badge earned modal"
```

---

## Task 8: WASM Stubs and Full Validation

**Files:**

- Modify: `core/src/wasm_stub.rs` only if shared imports require explicit stubs.
- Modify: any file with compilation errors from prior tasks.

- [ ] **Step 1: Add WASM unsupported functions only if needed**

If `arcadestr-core` WASM exports are required by app code, add display-safe stubs in `core/src/wasm_stub.rs`:

```rust
pub async fn fetch_earned_badges<T>(_profile_pubkey: T) -> Result<Vec<()>, String>
where
    T: Into<String>,
{
    Err("Badge relay display is not yet available on the web target.".to_string())
}

pub async fn fetch_profile_badges<T>(_profile_pubkey: T) -> Result<Vec<()>, String>
where
    T: Into<String>,
{
    Err("Badge relay display is not yet available on the web target.".to_string())
}
```

If app code uses only `app/src/tauri_bridge.rs` web wrappers, leave `core/src/wasm_stub.rs` unchanged.

- [ ] **Step 2: Run formatting**

Run:

```bash
cargo fmt
```

Expected: command exits 0.

- [ ] **Step 3: Run core check**

Run:

```bash
cargo check -p arcadestr-core
```

Expected: command exits 0.

- [ ] **Step 4: Run desktop check**

Run:

```bash
cargo check -p arcadestr-desktop
```

Expected: command exits 0.

- [ ] **Step 5: Run app check**

Run:

```bash
cargo check -p arcadestr-app
```

Expected: command exits 0.

- [ ] **Step 6: Run core tests**

Run:

```bash
cargo test -p arcadestr-core -- --test-threads=1
```

Expected: command exits 0. On failure, stop, report the exact failure, propose a fix, and request approval before changing code.

- [ ] **Step 7: Run desktop tests**

Run:

```bash
cargo test -p arcadestr-desktop
```

Expected: command exits 0. On failure, stop, report the exact failure, propose a fix, and request approval before changing code.

- [ ] **Step 8: Commit validation fixes if any were needed**

If Step 1 changed stubs or validation fixes were approved and made:

```bash
git add core/src/wasm_stub.rs app/src core/src desktop/src desktop/tests core/tests
git commit -m "fix: finalize badge display validation"
```

If no files changed, skip this commit.

---

## Final Handoff Checklist

- [ ] `fetch_earned_badges` command registered in `desktop/src/main.rs`.
- [ ] `fetch_profile_badges` command registered in `desktop/src/main.rs`.
- [ ] Profile showcase uses kind-10008 if present and non-empty.
- [ ] Profile showcase fallback caps earned badges at 8.
- [ ] Achievements page shows all verified earned badges with no cap.
- [ ] Web target does not attempt relay fetching and shows explicit unavailable copy.
- [ ] No badge issuance request models were added.
- [ ] No kind-10008 publishing/update command was added.
- [ ] No local-only visibility toggles were added.
- [ ] `Kind::Custom(10008)` is used for current profile badge lists.
- [ ] Deprecated kind-30008 is read only when kind-10008 is absent.
