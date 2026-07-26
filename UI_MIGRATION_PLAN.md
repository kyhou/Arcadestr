# UI Migration Plan

The migration is incremental. Each phase is independently testable and must preserve existing backend-backed behavior. Reference mock data and React-specific runtime code are excluded.

## Verified Validation Topology

Workspace packages from the manifests:

- `arcadestr-app`
- `arcadestr-core`
- `arcadestr-desktop`
- `arcadestr-web`

`arcadestr-web` always enables `arcadestr-app/wasm`. Its optional `web` feature additionally enables standalone browser behavior through `arcadestr-app/web`.

Valid WASM checks:

```bash
cargo check -p arcadestr-web --target wasm32-unknown-unknown
cargo check -p arcadestr-web --target wasm32-unknown-unknown --features web
```

Valid finite Trunk builds, run from `web/`:

```bash
trunk build
trunk build --features web
```

The Tauri CLI discovers `desktop/tauri.conf.json` when invoked at the repository root. Its configured `beforeDevCommand` is `cd web && trunk build`, so the configured invocation is:

```bash
cargo tauri dev
cargo tauri build
```

from the repository root. Existing documentation claiming the command must run from `desktop/` conflicts with the current `beforeDevCommand`; from `desktop/`, `cd web` would address a nonexistent directory. Long-running development servers are not validation commands in this plan.

## Protocol and Ownership Invariants

All phases preserve four acquisition categories:

1. **Paid purchase:** durable ownership through a NIP-102 receipt.
2. **Claim-and-keep campaign:** durable ownership through an Entitlement Grant.
3. **Public access:** temporary access based on current listing policy; no durable ownership.
4. **Timed access:** temporary access during the configured interval; no durable ownership.

No UI state may infer public access from a zero price. Public and timed access must not be presented as permanent ownership.

## Phase 1: Design Tokens

Goal: adopt the Noir visual language without changing application structure or behavior.

Expected files:

- `web/style/tailwind.css`
- `web/tailwind.config.js`
- `app/src/ui_v2/theme.rs`

Work:

- Add the reference OKLCH palette, surface hierarchy, radii, borders, glow, glass, shadows, typography, and motion tokens.
- Keep Inter for body text and Space Grotesk for display text with resilient system fallbacks.
- Add reduced-motion handling before enabling hover, pulse, scale, glow, or transition effects.
- Preserve existing Arcadestr utility names and color aliases required by current markup.
- Do not change layouts, component markup, data flow, or business logic.

Validation:

```bash
cargo fmt --all --check
cargo check -p arcadestr-app
cargo test -p arcadestr-app
cargo check -p arcadestr-desktop
cargo check -p arcadestr-web --target wasm32-unknown-unknown
cargo check -p arcadestr-web --target wasm32-unknown-unknown --features web
git diff --check
```

From `web/`:

```bash
trunk build
trunk build --features web
```

## Phase 2: Shell and Shared Primitives

Expected files:

- `app/src/ui_v2/shell.rs`
- `app/src/ui_v2/components/mod.rs`
- `app/src/ui_v2/components/topbar.rs`
- `app/src/ui_v2/components/nav_item.rs`
- New `app/src/ui_v2/components/page_header.rs`
- New `app/src/ui_v2/components/game_card.rs`

Work:

- Rebuild shell, top bar, sidebar, and mobile navigation in Leptos.
- Use real profile and relay data.
- Keep Store, Browse, Library, Social, Publish, Profile, Achievements, and Settings reachable.
- Do not display unconditional Online status.
- Consolidate duplicated card presentation without changing acquisition behavior.

Validation:

```bash
cargo fmt --all --check
cargo check -p arcadestr-app
cargo test -p arcadestr-app
cargo check -p arcadestr-desktop
cargo check -p arcadestr-web --target wasm32-unknown-unknown
git diff --check
```

Desktop flow: authenticate, switch accounts, navigate every item, inspect relay state, and verify narrow-window navigation.

## Phase 3: Store and Browse

Expected files:

- `app/src/ui_v2/views/store_front.rs`
- `app/src/ui_v2/views/browse_games.rs`
- `app/src/ui_v2/views/marketplace_loader.rs` only if view-facing state needs extension
- `app/src/ui_v2/components/game_card.rs`

Work:

- Apply the hero, promotion banner, category, and catalog visual structure.
- Keep marketplace streaming, cache fallback, pagination, and replaceable-event ordering unchanged.
- Add search, genre, price/access, and sorting over loaded real listings.
- Preserve host-platform filtering and incompatible-platform indicators.
- Omit live activity and fake support totals until real data exists.

Validation:

```bash
cargo fmt --all --check
cargo check -p arcadestr-app
cargo test -p arcadestr-app
cargo check -p arcadestr-desktop
cargo check -p arcadestr-web --target wasm32-unknown-unknown
git diff --check
```

Desktop flow: cached load, relay refresh, sparse platform filter, load more, search, empty filter, and detail navigation.

## Phase 4: Game Detail and Acquisition

Expected files:

- `app/src/ui_v2/views/game_detail.rs`
- `app/src/ui_v2/components/game_card.rs`
- `app/src/models.rs` only if a presentation helper is needed

Work:

- Recompose detail into hero, content, acquisition, campaign, technical, and seller panels.
- Preserve ownership refresh, invoice generation, copy/open-wallet, NWC, preimage confirmation, entitlement claims, and installation.
- Retain loading, disabled, error, confirmation, and success states.
- Omit ratings, currently-playing users, fake notes, and unsupported verification claims.

Validation:

```bash
cargo fmt --all --check
cargo check -p arcadestr-app
cargo test -p arcadestr-app
cargo check -p arcadestr-desktop
cargo test -p arcadestr-desktop
git diff --check
```

Desktop flow: paid unowned, paid owned, active claim campaign, public access, timed access, gated listing, incompatible platform, installed game, and failed payment/install.

## Phase 5: Authentication, Accounts, Profile, and Settings

Expected files:

- `app/src/ui_v2/views/login.rs`
- `app/src/ui_v2/views/profile.rs`
- `app/src/ui_v2/shell.rs`
- New `app/src/ui_v2/views/settings.rs`
- `app/src/ui_v2/views/mod.rs`
- `app/src/components/nip49_modal.rs`
- `app/src/components/backup_manager.rs` only after command availability is confirmed

Work:

- Apply account-card visuals to real saved accounts.
- Retain bunker, NIP-07, QR, nsec, restore, switch, and delete flows.
- Move inline settings to a dedicated view.
- Surface only settings backed by real state.
- Wire NIP-49 export to the existing typed bridge.
- Keep unsupported backup and network controls absent or explicitly unavailable.

Validation:

```bash
cargo fmt --all --check
cargo check -p arcadestr-app
cargo test -p arcadestr-app
cargo check -p arcadestr-desktop
cargo test -p arcadestr-desktop
cargo check -p arcadestr-web --target wasm32-unknown-unknown
cargo check -p arcadestr-web --target wasm32-unknown-unknown --features web
git diff --check
```

## Phase 6: Library

Expected files:

- `app/src/ui_v2/views/library.rs`
- `app/src/ui_v2/components/game_card.rs`

Work:

- Adopt the reference library card and summary language.
- Continue using only the real installed-game registry until durable credentials can be listed.
- Do not add fake Owned, Updates, Play, storage totals, verification, or access-problem results.
- Link installed entries to detail where real listing metadata is available.

Validation:

```bash
cargo fmt --all --check
cargo check -p arcadestr-app
cargo test -p arcadestr-app
cargo check -p arcadestr-desktop
git diff --check
```

## Phase 7: Publishing and Campaign Management

Expected files:

- `app/src/ui_v2/views/publish.rs`
- `app/src/components/publish.rs`
- `app/src/campaign_management.rs`
- `app/src/ui_v2/theme.rs`

Work:

- Present existing publication inputs as a staged wizard.
- Keep listing IDs, descriptions, images, tags, platform tags, pricing, acquisition policy, timed windows, fulfillment, operator, distribution server, build hashing, validation, and progress.
- Preserve published-game management, editing, campaign creation, cancellation, and listing-pointer updates.
- Do not add fake drafts or screenshot uploads.

Validation:

```bash
cargo fmt --all --check
cargo check -p arcadestr-app
cargo test -p arcadestr-app
cargo check -p arcadestr-desktop
cargo test -p arcadestr-desktop
git diff --check
```

## Phase 8: Achievements and Community

Expected files:

- `app/src/ui_v2/views/achievements.rs`
- `app/src/ui_v2/views/social.rs`
- `app/src/components/badge_showcase.rs`
- `app/src/components/badge_earned_modal.rs`

Work:

- Restyle achievements and badge presentation.
- Preserve cached-first relay refresh and stale-request rejection.
- Apply community visual structure only to honest empty/unavailable states.
- Do not ship fabricated feed data or nonfunctional controls.

Validation:

```bash
cargo fmt --all --check
cargo check -p arcadestr-app
cargo test -p arcadestr-app
cargo check -p arcadestr-desktop
cargo check -p arcadestr-web --target wasm32-unknown-unknown
git diff --check
```

## Phase 9: Durable Purchase and Access Records

This phase must not begin until product scope approves a new backend query.

Expected files:

- `core/src/purchases.rs`
- `core/src/ownership.rs`
- `core/src/entitlements.rs`
- `desktop/src/command_contracts.rs`
- `desktop/src/main.rs`
- `app/src/models.rs`
- `app/src/tauri_bridge.rs`
- New `app/src/ui_v2/views/purchases.rs`
- `app/src/ui_v2/views/mod.rs`
- `app/src/ui_v2/shell.rs`

Work:

- Add a read-only typed query for validated NIP-102 purchase receipts and Entitlement Grants belonging to the active account.
- Do not synthesize records from listings.
- Distinguish purchases, claim-and-keep grants, status, amount, timestamp, listing coordinate, and validation failures.
- Explicitly exclude public access and timed access: neither creates a durable credential or durable ownership.

Validation:

```bash
cargo fmt --all --check
cargo test -p arcadestr-core --features native -- --test-threads=1
cargo check -p arcadestr-app
cargo test -p arcadestr-app
cargo check -p arcadestr-desktop
cargo test -p arcadestr-desktop
git diff --check
```

## Final Cross-Target Validation

```bash
cargo fmt --all --check
cargo check -p arcadestr-app
cargo test -p arcadestr-app
cargo check -p arcadestr-desktop
cargo test -p arcadestr-desktop
cargo check -p arcadestr-web --target wasm32-unknown-unknown
cargo check -p arcadestr-web --target wasm32-unknown-unknown --features web
git diff --check
```

From `web/`:

```bash
trunk build
trunk build --features web
```

Use a finite `cargo tauri build` from the repository root only when full desktop packaging validation is warranted. Do not use a long-running development server as an automated validation command.
