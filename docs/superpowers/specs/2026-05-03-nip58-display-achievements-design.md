# NIP-58 Display Achievements Design

Date: 2026-05-03
Status: Approved for implementation planning
Scope: Display-only vertical slice

## Goal

Implement Arcadestr's display path for NIP-58 visual badges/achievements. The feature fetches verified kind-8 badge awards for a profile, lazily resolves referenced kind-30009 badge definitions, caches both in SQLite, and renders earned badges on the profile and achievements surfaces.

This session intentionally does not implement badge issuance, badge definition authoring, kind-10008 publishing, publisher badge management, or web relay-fetch infrastructure.

## Context and Constraints

Relevant context discovered before design:

- `/home/joel/.opencode/context/core/standards/code-quality.md`
- `/home/joel/.opencode/context/core/standards/test-coverage.md`
- `/home/joel/.opencode/context/core/standards/security-patterns.md`
- `/home/joel/.opencode/context/core/standards/documentation.md`
- `/home/joel/Sync/Projetos/Arcadestr/.opencode/context/project-intelligence/technical-domain.md`
- `/home/joel/Sync/Projetos/Arcadestr/AGENTS.md`
- `/home/joel/Sync/Projetos/Arcadestr/CODEBASE.md`

External documentation finding:

- Current NIP-58 uses kind `10008` for profile badge lists.
- `nostr-sdk` / `nostr` 0.44 maps `Kind::ProfileBadges` and `EventBuilder::profile_badges` to deprecated kind `30008` with `d=profile_badges`.
- Therefore current profile badge lists must use `Kind::Custom(10008)` for reading. Do not use `Kind::ProfileBadges` for current NIP-58 profile badges.

Current migration state checked during design: `core/migrations/001_initial_schema.sql` is the only existing migration, so the implementation should add the next project-appropriate achievements migration. If repository state changes before implementation, use the next available migration number rather than assuming `0004`.

## In Scope

- Parse and validate NIP-58 badge definitions, awards, and read-only profile badge lists:
  - kind-30009 badge definitions
  - kind-8 badge awards
  - kind-10008 profile badge lists
  - deprecated kind-30008 `d=profile_badges` as lower-priority read fallback only
- Fetch kind-8 awards for a profile pubkey from relays.
- Lazily fetch only kind-30009 definitions referenced by earned awards or profile badge lists.
- Cache definitions, awards, and profile badge list data in SQLite.
- Expose display-only desktop Tauri commands:
  - `fetch_earned_badges(profile_pubkey) -> Vec<EarnedBadgeSummary>`
  - `fetch_profile_badges(profile_pubkey) -> Vec<ProfileBadgeEntry>`
- Add typed app IPC wrappers for those commands.
- Add user-visible UI:
  - `BadgeShowcase` on profile pages
  - `AchievementsView` full page
  - `BadgeEarnedModal` component with disabled/debug-only post-purchase integration hook
- Keep web target compiling with explicit unavailable state for badge relay display.

## Out of Scope

- Badge definition publishing.
- Badge award publishing.
- kind-10008 update/publish commands.
- Publisher badge management UI.
- Web relay-fetch infrastructure.
- Local-only visibility toggles.
- `BadgeEarnedEvent` relay-subscription payloads. These belong to the issuance/subscription follow-up.

## Architecture

- `core` owns NIP-58 event parsing, validation, relay fetching, and SQLite cache behavior.
- `desktop/src/command_contracts.rs` owns pure command orchestration and converts core failures into command errors.
- `desktop/src/main.rs` adds thin Tauri wrappers only and registers commands in `invoke_handler!`.
- `app` owns IPC models/wrappers and Leptos UI components/views.

The feature follows the existing Rust/Tauri/Leptos layering. Business logic must not be placed in `desktop/src/main.rs`.

## Backend Data Flow

### `fetch_earned_badges(profile_pubkey)`

1. Fetch kind-8 awards where `#p = profile_pubkey`.
2. Parse each award into a normalized per-recipient `BadgeAward`.
3. Extract referenced `badge_coordinate` values.
4. Check SQLite for cached definitions by coordinate.
5. Lazily fetch missing kind-30009 definitions only for referenced coordinates.
6. Validate strict issuer match after definition resolution and before caching the award: `award.issuer_pubkey == definition.issuer_pubkey`.
7. Cache valid definitions and valid awards.
8. Return all verified earned badge summaries.

Awards that fail issuer validation must not be cached, even as failed entries. They should be silently excluded with a tracing log.

### `fetch_profile_badges(profile_pubkey)`

1. Fetch latest kind-10008 event for `profile_pubkey`.
2. If no kind-10008 exists, fetch deprecated kind-30008 with `d=profile_badges`, log a deprecation warning, and use it as read-only fallback.
3. If kind-10008 exists, ignore kind-30008 entirely for that pubkey. Do not merge both formats.
4. Parse only immediate consecutive `a` then `e` pairs. An `a` tag without a following `e` tag, or an `e` tag without a preceding `a` tag, is malformed and skipped.
5. Resolve referenced definitions and awards from cache/relay as needed.
6. Return ordered verified `ProfileBadgeEntry` rows.

### Showcase Hybrid Behavior

The profile showcase uses profile badge preferences when available, otherwise falls back to earned badges:

1. Fetch kind-10008 for the pubkey.
2. If found and non-empty, show only those badge references in display order.
3. If not found or empty, show verified kind-8 awards.
4. Cap fallback display to 8 badges maximum.
5. Achievements view shows all verified earned badges with no cap.

## Validation Rules

- Badge definition must be kind 30009.
- Badge definition must include a non-empty `d` tag.
- Definition coordinate must equal `30009:<definition_pubkey>:<d>`.
- Badge award must be kind 8.
- Badge award must include an `a` tag referencing the badge coordinate.
- Badge award must include at least one `p` tag.
- Awards with multiple `p` tags are normalized per matching recipient.
- Award event pubkey must match badge definition issuer pubkey.
- Profile badge event pubkey must equal the requested profile owner.
- Individual malformed events are logged and skipped, not fatal to the whole page.

## SQLite Cache Design

Add an achievements migration using the next available migration number at implementation time.

### `badge_definitions`

- `coordinate TEXT PRIMARY KEY`
- `issuer_pubkey TEXT NOT NULL`
- `badge_id TEXT NOT NULL`
- `event_id TEXT NOT NULL`
- `name TEXT`
- `description TEXT`
- `image_url TEXT`
- `image_dimensions TEXT`
- `thumb_url TEXT`
- `thumb_dimensions TEXT`
- `relay_url TEXT`
- `created_at INTEGER NOT NULL`
- `raw_event_json TEXT NOT NULL`
- unique index on `(issuer_pubkey, badge_id)`

The cache updates a definition for the same coordinate only when the incoming event has a newer `created_at`, or when timestamps tie and the incoming `event_id` is lexicographically lower.

### `badge_awards`

- `event_id TEXT PRIMARY KEY`
- `issuer_pubkey TEXT NOT NULL`
- `recipient_pubkey TEXT NOT NULL`
- `badge_coordinate TEXT NOT NULL`
- `relay_url TEXT`
- `created_at INTEGER NOT NULL`
- `raw_event_json TEXT NOT NULL`
- foreign key `badge_coordinate REFERENCES badge_definitions(coordinate)`
- index on `(recipient_pubkey, created_at DESC)`
- index on `badge_coordinate`

Only awards that pass issuer validation after definition resolution are cached.

### `profile_badge_lists`

- `profile_pubkey TEXT PRIMARY KEY`
- `event_id TEXT NOT NULL`
- `kind INTEGER NOT NULL`
- `created_at INTEGER NOT NULL`
- `raw_event_json TEXT NOT NULL`
- `updated_at INTEGER NOT NULL`

### `profile_badge_entries`

- `profile_pubkey TEXT NOT NULL`
- `badge_coordinate TEXT NOT NULL`
- `award_event_id TEXT NOT NULL`
- `relay_url TEXT`
- `display_order INTEGER NOT NULL`
- primary key `(profile_pubkey, display_order)`
- foreign key `profile_pubkey REFERENCES profile_badge_lists(profile_pubkey) ON DELETE CASCADE`
- index on `(profile_pubkey, badge_coordinate)`
- index on `award_event_id`

When a newer profile badge list is accepted for a pubkey, update the list and entries in a transaction: delete old entries, upsert the list row, then insert the new ordered entries.

### Replacement Rules

- kind-30009 definitions: keep latest per coordinate; timestamp tie keeps lowest lexicographic event id.
- profile badge lists: keep latest per `profile_pubkey`; prefer kind-10008 over kind-30008 when both exist; timestamp tie within same kind keeps lowest lexicographic event id.
- kind-8 awards: immutable regular events; cache by `event_id` after validation.

## IPC and Models

Add IPC-facing display models in `app/src/models.rs`, deriving `Serialize`, `Deserialize`, `Clone`, and `Debug`:

- `BadgeDefinition`
- `BadgeAward`
- `ProfileBadgeEntry`
- `EarnedBadgeSummary`

Do not add issuance/update request models in this session.

Add wrappers in `app/src/tauri_bridge.rs`:

- `fetch_earned_badges(profile_pubkey: String)`
- `fetch_profile_badges(profile_pubkey: String)`

Desktop wrappers call Tauri commands. Web wrappers return explicit unsupported errors so components can render the unavailable state.

## UI Design

### `BadgeShowcase`

File: `app/src/components/badge_showcase.rs`

- Match existing `profile.rs` pubkey wiring patterns rather than introducing a new incompatible prop shape.
- Desktop behavior:
  - Fetch profile badges for the profile when mounted or when viewed profile changes.
  - If profile badges exist, render them in display order.
  - If none exist, fetch earned badges and render up to 8 verified earned badges.
  - Show loading, empty, and error states.
- Web behavior:
  - Do not attempt relay fetching.
  - Render explicit unavailable state: badge relay display is not yet available on web.
- Embed below the profile bio section in `app/src/ui_v2/views/profile.rs`.

### Achievements View

File: `app/src/ui_v2/views/achievements.rs`

- Fetch all verified earned badges for the current profile/current user.
- Render badge cards with image/thumb, name, description, issuer pubkey, and award date.
- No show/hide toggles in this session.
- Empty state explains no verified badges were found.
- Error state surfaces relay/cache fetch failures.
- Export from `app/src/ui_v2/views/mod.rs`.
- Add `UiV2View::Achievements` and sidebar `NavItem` using the same callback/signal navigation pattern as existing views.
- Do not add badge displays to browse, store front, or library.

### Badge Earned Modal

File: `app/src/components/badge_earned_modal.rs`

- Fully renderable when passed an `EarnedBadgeSummary`.
- Shows badge image/thumb, name, description, issuer, and close action.
- Integrated at the post-purchase confirmation point in `game_detail.rs`.
- Production trigger disabled.
- Include this explicit follow-up comment at the hook:
  `// Follow-up: wire to kind-8 relay subscription when badge issuance lands.`
- Optional debug-only preview hook may be gated behind `#[cfg(debug_assertions)]`.

## Error Handling

Surface user-visible errors for:

- Relay fetch failure for achievements/profile badges.
- Malformed badge definitions when they prevent display.
- Invalid award proof when no verified badges remain.
- Web unsupported badge relay display state.

Log and skip without failing the entire page:

- Individual malformed badge events.
- Missing optional image/thumb tags.
- Orphan profile badge list tags.
- Deprecated kind-30008 fallback usage.
- Issuer mismatch / invalid award proof.
- Relay timeout from one relay when other relays succeed.

Core errors should use explicit `thiserror` variants. Desktop command boundaries should convert to serializable strings or the existing command error shape.

## Testing

Core tests should cover:

- Parse valid kind-30009 badge definition.
- Reject kind-30009 without non-empty `d`.
- Parse valid kind-8 award with `a` and `p`.
- Reject/exclude award where issuer pubkey does not match badge definition issuer.
- Parse kind-10008 ordered consecutive `a`/`e` pairs.
- Ignore orphan `a` and orphan `e` tags.
- Prefer kind-10008 over deprecated kind-30008.
- Log/use kind-30008 only when kind-10008 is absent.
- Latest profile badge list wins by timestamp.
- Timestamp tie chooses lowest lexicographic event id.
- Fetch earned badges lazily for a profile.
- Fetch definitions only for referenced earned badge coordinates.
- Cache fetched definitions and avoid redundant relay fetches where existing test seams support it.
- Definition cache updates by coordinate only for newer `created_at` or lexicographically lower event id on tie.

Desktop command tests should cover:

- `fetch_earned_badges` returns earned badges for a profile pubkey.
- `fetch_profile_badges` returns kind-10008 visible badges in display order.
- Unsupported/native-web paths do not panic and return clear errors where applicable.

UI tests, if an existing Leptos test pattern supports them, should cover:

- `BadgeShowcase` loading, empty, error, and web-unavailable states.
- Achievements view renders earned badge cards.
- Badge earned modal renders supplied badge data.

Validation commands:

- `cargo fmt`
- `cargo check -p arcadestr-core`
- `cargo check -p arcadestr-desktop`
- `cargo check -p arcadestr-app`
- `cargo test -p arcadestr-core -- --test-threads=1`
- `cargo test -p arcadestr-desktop`

## Follow-Up Session

A later issuance session should add:

- Badge definition authoring for publishers.
- Badge award publishing.
- kind-10008 update/publish support.
- Publisher Manage Badges UI.
- Player opt-in/show-hide controls backed by real kind-10008 publishing.
- Relay subscription/event payloads such as `BadgeEarnedEvent`.
- Web relay-fetch infrastructure if badge display should work in browser builds.
