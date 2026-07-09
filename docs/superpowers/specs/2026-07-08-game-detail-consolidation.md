# Game Detail Page — v2 Consolidation

## Goal

Replace hardcoded placeholders in the v2 `GameDetailView` with real data from
`GameListing`, absorb the old `DetailView` component's buy flow and seller
profile into the v2 layout, and remove the `DetailView` call.

## Template References (Stitch)

Two detail page templates exist in the Stitch project
(`Arcadestr Game Storefront`):

- **Connected** — Full hero (716px) with glass-panel buy panel inside hero,
  12-column content grid below. Uses some hardcoded colors.
- **Standardized** — Smaller hero (600px), tighter spacing, uses Tailwind
  design tokens exclusively. Reference for token usage.

Layout common to both: hero background image → 8+4 grid (gallery/description/
feed left, purchase card/currently-playing/specs right).

Our implementation keeps the **hero+buy-panel-aside** layout (better UX — price
visible immediately) and adopts the 8+4 content grid below.

## Changes

### 1. Hero Section (`v2-detail-hero`)

| Element | Source | Status |
|---|---|---|
| Hero background image | `listing.images.first()` | **Wire** |
| Kicker badge | `listing.stall_name` or `listing.tags.first()` | **Wire** |
| Title | `listing.title` | Already wired |
| Rating / Stars | No Nostr data source | Keep placeholder |
| Zap count | No Nostr data source | Keep placeholder |
| Description | `listing.description` | Already wired |
| Tags | `listing.tags` | Already wired |

### 2. Hero Buy Panel (right aside in hero)

| Element | Source | Status |
|---|---|---|
| Price | `listing.price_sats` > 0 → formatted sats; 0 → "Free" | **Wire** |
| Strikethrough price | No data source | Remove |
| Buy button | Invoice flow from DetailView | **Migrate** |
| Add to Library | No backend yet | Keep as disabled |
| Developer | Seller Nostr profile name (via ProfileStore) | **Migrate** |
| Publisher | `listing.stall_name` or `listing.tags` | **Wire** |
| Release Date | `listing.created_at` → formatted date | **Wire** |
| Protocol / Source | `listing.source` → "NIP-15 Product" / "NIP-99 Listing" | **Wire** |
| Currently Playing | No subscription data | Keep placeholder |

### 3. Content Grid (replaces flat sections)

Current sections are stacked vertically. Restructure as a 2-column grid
(same `v2-detail-grid` class, already used for the feed+specs section).

**Left column:**
- **Gallery** — `listing.images` (skip first), rendered as bento grid
  matching template (2-col, first image larger)
- **Description block** — Keep, serves as expanded description
- **Nostr Feed** — Keep placeholder cards (same structure)

**Right column:**
- **Specs** — `listing.specs` key-value pairs (currently hardcoded OS/GPU/Storage)
- **Seller Profile** — Migrated from `DetailView` (ProfileRow + NIP-05 + LUD16 + website)

### 4. Buy Flow (migrated from DetailView)

Invoice request → loading state → invoice display → copy clipboard / open
wallet. All states rendered with v2 CSS classes instead of inline styles.

State machine:
1. Initial → "Buy with Lightning" button
2. Loading → disabled button "Requesting invoice..."
3. Invoice ready → bolt11 (truncated) + Copy / Open in Wallet buttons
4. Error → error message display

### 5. Seller Profile (migrated from DetailView)

- Fetch seller profile from `listing.publisher_npub` on mount
- Check `ProfileStore` cache first, fall back to network
- Display in right column below specs:
  - ProfileRow (avatar + name + truncated npub)
  - NIP-05 identifier with verification badge
  - LUD16 lightning address
  - Website link

### 6. Cleanup

- Remove `<DetailView listing={...} />` call from `game_detail.rs`
- Remove imports of `DetailView` and `BadgeEarnedModal` (modal stays)
- Remove duplicate `listing.description` and tags from content area (shown in hero)
- Remove `listing.title` from content area (shown in hero)

## Files to Modify

| File | Changes |
|---|---|
| `app/src/ui_v2/views/game_detail.rs` | Main rewrite — wire data, migrate buy flow + profile, restructure layout |
| `app/src/ui_v2/theme.rs` | Possibly add `.v2-detail-seller` section class if needed |

## Files to Remove Imports From

- `app/src/ui_v2/views/game_detail.rs` — Remove `DetailView` import

## What Stays as Placeholder

- Star rating (no Nostr kind-7 subscription)
- Zap count on hero (no Nostr zap subscription)
- "Currently Playing" section (no Nostr presence data)
- Nostr Feed cards (no Nostr note subscription)

These require real Nostr event subscriptions, which is a separate feature.
