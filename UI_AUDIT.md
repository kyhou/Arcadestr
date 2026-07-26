# Arcadestr UI Audit

Authoritative visual reference: `../arcadestr-9b2941d6`

Target: the current Rust, Leptos 0.8, Tailwind CSS, and Tauri Arcadestr workspace.

## Reference Application

### Routes

| Route | Page | Source |
|---|---|---|
| `/` | Storefront | `src/routes/index.tsx` |
| `/browse` | Searchable and filterable catalog | `src/routes/browse.tsx` |
| `/game/$id` | Game detail and acquisition | `src/routes/game.$id.tsx` |
| `/library` | Owned and installed games | `src/routes/library.tsx` |
| `/community` | Community feed and composer | `src/routes/community.tsx` |
| `/publish` | Four-step publishing wizard | `src/routes/publish.tsx` |
| `/accounts` | Account selection | `src/routes/accounts.tsx` |
| `/purchases` | Purchase and entitlement history | `src/routes/purchases.tsx` |
| `/settings` | Account, network, security, backup, appearance | `src/routes/settings.tsx` |
| `/sitemap.xml` | Generated sitemap | `src/routes/sitemap[.]xml.ts` |
| Unmatched route | Global 404 | `src/routes/__root.tsx` |
| Route failure | Global retry/error page | `src/routes/__root.tsx` |

### Shared Layout and Navigation

`src/components/AppShell.tsx` provides:

- A sticky translucent top bar.
- A desktop sidebar at the `md` breakpoint and above.
- A maximum content width of 1600px.
- An identity card with signing-app connection status.
- Primary navigation for Store, Browse, Library, Community, Publish, and Account.
- Secondary actions for connecting an account, support, and sign out.
- Top-bar Discover and Browse links, network status, search, notifications, purchases, and accounts.
- A reusable `PageHeader` with eyebrow, title, description, and action content.

The reference has an important responsive gap: the desktop sidebar disappears below `md`, but no mobile navigation replaces it. Search and direct account access also disappear. Arcadestr must not copy this regression; its existing mobile navigation must remain reachable.

### Reusable Visual Components

Components used by application routes:

- `AppShell`
- `PageHeader`
- `GameCard`
- Route-local helpers: `Field`, `SectionHead`, `ModelCard`, `Check`, `Toggle`, `SummaryRow`, `FeedActions`, and `Row`

The generated `src/components/ui/` Radix/shadcn catalog is not imported by application routes. It is runtime scaffolding, not an authoritative set of components, and must not be ported.

### Navigation and Interaction

- Cards and hero calls to action navigate to game detail.
- Categories navigate to Browse but do not carry the selected category into the filter state.
- Browse search, price, genre, platform, and sort controls operate only on local mock data.
- Library, purchases, community, settings, publishing, and account controls mostly change local presentation state or have no behavior.
- Detail Technical details is the only active disclosure panel.
- No application route uses a dialog, drawer, context menu, alert dialog, or sheet.
- The reference uses URL routing and browser history; current Arcadestr uses an in-memory `UiV2View` enum.

### Forms and Fields

#### Search and Browse

- Top-bar search: `Search curated games...`; visual only.
- Browse search: title, developer, and genre matching.
- Price: All, Free, Paid.
- Genre filter.
- Platform filter.
- Sort: Recommended, Newest, Price ascending, Price descending.

#### Community

- Two-row note textarea.
- Image attachment button.
- Post note button.
- No Nostr publication behavior.

#### Publishing

Step 1, Basic info:

- Title.
- Short summary.
- Full description.
- Genre.
- Tags.
- Hero banner upload.
- Four screenshot upload slots.

Step 2, Pricing and access:

- Paid purchase or no-paid-purchase model.
- Price in sats.
- Lightning payment address.
- Gated, public, or timed access.
- Timed-access start and end.

Step 3, Builds and compatibility:

- Release version.
- Distribution provider.
- Supported platforms.
- Game archive.

Step 4 is a read-only publication checklist. Save Draft, Publish Game, image upload, screenshot upload, and archive upload are not implemented in the reference.

#### Settings

- Display name.
- About.
- Lightning address.
- Network server toggles and Add Server.
- Theme.
- Larger text.
- Reduce motion.
- Reconnect/disconnect signer.
- Encrypted key import/export.
- Application backup/restore.
- Clear cache.
- Diagnostics actions.

Only server, theme, text-size, and motion toggles change temporary local React state.

### Tabs and Filters

- Library: Owned, Installed, Not installed, Updates, Access problems.
- Purchases: All, Purchases, Promotion claims, Refunded.
- Community: All Notes, Long form.
- Publish: four wizard steps.
- Browse: search, price, genre, platform, and sort.
- No tab state is persisted or represented in the URL.

### Card and Panel Inventory

- Game card with cover, acquisition badge, zap count, ownership state, genres, developer, price, and action.
- Store hero and community-support glass panel.
- Category tiles.
- Claim/timed-access promotion banner.
- Live activity note cards.
- Library installation cards and account, storage, and reminder panels.
- Community editorial and note cards.
- Detail acquisition, technical-details, notes, and currently-playing cards.
- Account selector cards.
- Purchase-record rows.
- Settings section cards.
- Publishing model cards, preview, and checklist.

### Loading, Error, Empty, Disabled, Success, and Confirmation States

Explicit reference states:

- Global 404.
- Global route error with retry.
- Detail not found.
- Detail load failure with retry.
- Browse empty results.
- Library empty tabs.
- Library No access problems success state.
- Purchases empty filter.
- Connected signer and network badges.
- Publishing checklist complete/incomplete indicators.
- Gated/unavailable acquisition presentation.

Missing reference states:

- No route pending/loading UI.
- No catalog loading or stale-cache state.
- No asynchronous form-submission states.
- No invoice/payment states.
- No download progress.
- No account connection progress or errors.
- No publication progress or partial-success state.
- No confirmation dialogs.
- No persisted success notifications.
- No disabled application actions beyond styles in unused generic components.

Arcadestr's existing real loading, error, cached-data, payment, campaign, installation, confirmation, and publishing states remain authoritative and must be restyled rather than removed.

### Responsive Behavior

The reference uses Tailwind's standard breakpoints: `sm` 640px, `md` 768px, `lg` 1024px, and `xl` 1280px.

- Catalog: one, two, three, then four columns.
- Store: single-column activity layout until `lg`.
- Detail and Publish: main content plus a 360px sidebar at `lg`.
- Library and Community: main content plus a 320px sidebar at `lg`.
- Settings: one column, then two at `lg`.
- Forms: one column, then two or three at `md`.
- Purchases: stacked rows below `md`, five-column table at `md`.
- Hero padding, type, and height increase at `md`.
- Sidebar and top-bar search disappear below `md`.
- No reference mobile navigation exists.

### Visual System

The visual system is defined in `src/styles.css`.

Typography:

- Inter for body text.
- Space Grotesk for headings and display text.
- Google Fonts loading with UI/system fallbacks.

Palette:

- Near-black blue/purple OKLCH background.
- Purple primary.
- Cyan secondary.
- Pink tertiary.
- Emerald status/success.
- Red destructive.
- Semi-transparent white borders.
- Low, high, and bright surface elevation levels.

Shape and depth:

- Base radius of 0.75rem.
- Cards generally use 1.25rem.
- Major containers use large 3xl radii.
- Ambient shadow: `0 20px 40px rgba(0, 0, 0, 0.4)`.
- Primary glow: `0 0 32px` with translucent purple.
- Glass panels use translucent elevated surfaces and 24px blur.
- Cards rise 2px on hover.
- Images scale over 500ms.
- Buttons rise 1px on hover.
- Online indicators use pulse/ping animations.

The icon set is Lucide. Major icons include LayoutGrid, Gamepad2, MessagesSquare, Settings, Upload, Search, Bell, Receipt, LifeBuoy, LogOut, ShieldCheck, Users, Zap, ShoppingCart, Gift, Clock, SlidersHorizontal, Play, Download, AlertTriangle, RefreshCw, Image, MessageSquare, Repeat2, Share2, Copy, Star, ChevronDown, Plus, Check, Server, Palette, User, HardDrive, Info, FileEdit, and Coins.

The reference includes eight generated cover/hero JPEGs under `src/assets/`. They represent mock listings and must not replace images from real Arcadestr listing events.

### Mock Data and Placeholder Behavior

All marketplace, ownership, pricing, access, account, social, purchase, and settings data in the reference are local constants:

- Games: `src/lib/games.ts`.
- Store categories and activity: `src/routes/index.tsx`.
- Community notes, trends, and suggestions: `src/routes/community.tsx`.
- Accounts: `src/routes/accounts.tsx`.
- Purchases synthesized from owned mock games: `src/routes/purchases.tsx`.
- Settings servers, identity, latency, version, and health: `src/routes/settings.tsx`.

The acquisition presentation usefully distinguishes four supported acquisition categories. Arcadestr must preserve their protocol semantics:

1. **Paid purchase:** durable ownership through a NIP-102 receipt.
2. **Claim-and-keep campaign:** durable ownership through an Entitlement Grant.
3. **Public access:** temporary access based on the current listing policy; it creates no durable ownership.
4. **Timed access:** temporary access during the configured interval; it creates no durable ownership.

A zero or absent price must never imply public access. Acquisition policy and durable credentials remain separate concepts.

### Visually Represented but Unimplemented

- Top-bar search and notifications.
- Support and sign out.
- Category-specific filtering.
- Social note creation, image attachment, reactions, reposting, sharing, and zapping.
- Ratings, supporter counts, currently-playing users, and technical verification claims.
- Game launching.
- Updates, file verification, storage totals, and access-problem checks.
- Account switching and account creation.
- Purchase/refund history retrieval.
- Profile editing.
- Relay/server add, remove, and enable/disable.
- Theme, larger-text, and reduced-motion persistence.
- Backup, restore, cache clearing, and diagnostics.
- Publishing draft persistence and reference publication controls.

## Current Arcadestr Implementation

Primary sources:

- Navigation and shell: `app/src/ui_v2/shell.rs`.
- Top bar: `app/src/ui_v2/components/topbar.rs`.
- Store: `app/src/ui_v2/views/store_front.rs`.
- Browse: `app/src/ui_v2/views/browse_games.rs`.
- Detail and acquisition: `app/src/ui_v2/views/game_detail.rs`.
- Library: `app/src/ui_v2/views/library.rs`.
- Social: `app/src/ui_v2/views/social.rs`.
- Publishing and campaigns: `app/src/ui_v2/views/publish.rs`.
- Profile: `app/src/ui_v2/views/profile.rs`.
- Achievements: `app/src/ui_v2/views/achievements.rs`.
- Authentication and accounts: `app/src/ui_v2/views/login.rs`.
- Marketplace streaming: `app/src/ui_v2/views/marketplace_loader.rs`.
- Models: `app/src/models.rs`.
- Typed desktop bridge: `app/src/tauri_bridge.rs`.
- Root state and authentication: `app/src/lib.rs`.
- Tailwind entry: `web/style/tailwind.css`.
- Tailwind configuration: `web/tailwind.config.js`.

Existing behavior that must remain authoritative:

- Authentication and saved-account switching through `AuthContext`.
- Cache-first marketplace streaming, pagination, and replaceable-event ordering.
- Desktop host-platform detection and sparse-filter pagination.
- Purchase-receipt and entitlement-grant ownership.
- LNURL, NWC, and manual-preimage purchase flows.
- Campaign discovery and free entitlement claims.
- Installation and the installed-game registry.
- Real ADP/NIP-99 publishing and campaign management.
- Profile caching, NIP-05 verification, and badges.
- Achievement cache/relay refresh.
- Live relay snapshot/event merging.

Important current gaps:

- Navigation uses an in-memory enum rather than URL routing.
- Search and sorting are visual-only.
- Social content is static.
- Library only lists local installations.
- No typed command lists durable purchase/access credentials.
- Desktop emits download progress, but the typed bridge only exposes download completion.
- The profile NIP-49 export UI is not wired to the existing bridge.
- Standalone web lacks marketplace, profile, relay, purchase, campaign, publishing, installation, and achievement backends.

No real backend integration may be replaced by reference mock data during migration.
