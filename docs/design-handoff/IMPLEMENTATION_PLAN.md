# Arcadestr UI Handoff Implementation Plan

This plan maps the authoritative desktop handoff to the current Arcadestr
Leptos/Tauri application. It is an implementation audit, not an approval to
copy prototype fixtures or `support.js` into production.

Source priority used here is: screenshots, `Arcadestr.dc.html`, handoff
`README.md`, updated design specification, then existing application behavior.
The six supplied screenshots are 924 x 540 images. Production behavior remains
authoritative wherever the prototype only simulates an interaction.

## Audit summary

- The active UI has no URL router. `app/src/ui_v2/shell.rs` uses `UiV2View` as
  an in-memory route enum. Paths in the handoff are desired information
  architecture, not current routes.
- `app/src/lib.rs` mounts `LoginV2View` or `UiV2Root`; legacy `LoginView` and
  `MainView` remain compiled but are not the active UI.
- Styling is split across Noir variables in `web/style/tailwind.css`, Tailwind
  aliases in `web/tailwind.config.js`, the large `UI_V2_STYLES` block in
  `app/src/ui_v2/theme.rs`, legacy `STYLES` in `app/src/lib.rs`, and global CSS
  in `web/index.html`. Phase 1 must establish one token source with temporary
  compatibility aliases.
- The production UI already has real marketplace, account, payment, claim,
  purchase-history, publishing, campaign, Store Page, Blossom, and verified
  download behavior. Those state machines are to be recomposed, not replaced.
- Dedicated Releases and publisher Activity data models do not exist. Play,
  update, repair, uninstall, install-folder, review, and community-feed actions
  also lack production implementations. They must not be simulated.
- Current “installation” downloads an artifact, verifies its SHA-256, and
  records it. It does not extract, execute, launch, repair, or uninstall games.
- Production Store Page capabilities exceed the prototype: associations,
  reciprocal pointers, feature sections, languages, accessibility, canonical
  preview, clone/link, readiness diagnostics, and partial-publication recovery
  must remain reachable.
- The current UI contains extensive mobile behavior. The new shell should be
  desktop-first at the Tauri minimum width, while retaining safe overflow,
  reduced-motion support, and compact resizable-window behavior.

## 1. Screen mapping

There are currently no pathname routes. “Current destination” below means the
active `UiV2View` or nested `PublishViewState` in-memory destination.

| Handoff surface | Current destination | Current module/component | Migration decision |
|---|---|---|---|
| Login | Unauthenticated `App` fallback | `app/src/ui_v2/views/login.rs` — `LoginV2View` | Restyle. Preserve saved accounts, reconnect/remove, NIP-46 bunker/QR, NIP-07, encrypted local key, NIP-49 validation, target capability differences, cancellation, and stale-response rejection. Do not use the prototype’s instant-success login. |
| Store Home | `UiV2View::Store` | `app/src/ui_v2/views/store_front.rs` — `StoreFrontView` | Recompose into the split hero and four-card section. Keep real cache refresh, relay failure, campaign, Store Page, and marketplace data. A carousel is enabled only when multiple real featured items exist. |
| Browse | `UiV2View::Browse(BrowseRequest)` | `app/src/ui_v2/views/browse_games.rs` — `BrowseGamesView` | Restyle controls/grid. Preserve query/category input, paid/claim/public/timed/owned distinctions, exact platform filtering, real sorting, pagination, sparse-platform fetch, cache, and partial loading. |
| Game details | `UiV2View::Detail(GameListing, DetailOrigin)` | `app/src/ui_v2/views/game_detail.rs` — `GameDetailView`; `app/src/ui_v2/components/store_page_detail.rs` — `StorePageRichDetail` | Recompose into handoff hero/content/access panel. Preserve listing-authoritative commerce, Store Page enrichment, compatibility, seller identity, payment, claim, add-to-library, and verified download flows. |
| Library | `UiV2View::Library` | `app/src/ui_v2/views/library.rs` — `LibraryView` | Restyle as rows/tabs only where each tab has real derivable state. Keep account-scoped saved games separate from device-scoped installs and durable ownership. Unsupported Play/Update/Repair/Uninstall controls stay absent or disabled with an honest explanation. |
| Community | `UiV2View::Social` | `app/src/ui_v2/views/social.rs` — `SocialView` | Retain the honest unavailable state. Do not render prototype social fixtures. |
| Profile | `UiV2View::Profile` | `app/src/ui_v2/views/profile.rs` — `ProfileV2View` | Restyle the real relay profile, NIP-05 result, publisher listings, and badge showcase. Profile editing remains unavailable until a real publish command exists. |
| Publisher Dashboard | `UiV2View::Publish(PublishViewState::Games)` | `app/src/ui_v2/views/publish.rs` — `PublishedGamesView` | Restyle/extend into the Dashboard hierarchy using real listings and campaign summaries. Do not invent draft counts, revenue, warnings, or analytics. |
| Create Game | `PublishViewState::NewPublication` | `app/src/components/publish.rs` — `PublishView` | Restyle the existing four-stage publication workflow. A visual provider subsection may be separated without splitting validation/state. Do not add prototype Save Draft; `SUPPORTS_DRAFTS` is false. |
| Manage Game | `PublishViewState::Game(GameListing)` | `app/src/ui_v2/views/publish.rs` — `GameManagementView` | Restyle. Preserve network listing facts, fulfillment/operator data, Store Page entry, promotions, and diagnostics. |
| Store Page editor | `PublishViewState::StorePage(GameListing)` | `app/src/ui_v2/views/store_page_publish.rs` — `StorePageEditorView`; `app/src/ui_v2/components/blossom_media_upload.rs` | Restyle and regroup, not replace. Keep all production-only tabs/features, typed validation, in-memory account-scoped draft, media integrity, association review, and retry recovery. |
| Releases | No destination or model | Version/build fields in `app/src/components/publish.rs`; installed version in `core/src/adp_storage.rs` | Add only an honest unavailable/summary surface backed by the current listing version. Do not portray replaceable listing revisions as immutable releases. A functional release history requires separate backend/protocol work outside this visual migration. |
| Promotions | Per-game `PublishViewState::Campaign` | `app/src/ui_v2/views/publish.rs` — `CampaignEditorView`, `CampaignConfirmationDialog`, `campaign_row`; `app/src/campaign_management.rs` | Reuse and restyle. A top-level tab may aggregate real discovered campaigns, while game-scoped management remains available. Only claim-and-keep campaigns are currently valid; discount/timed-promotion controls remain disabled. |
| Activity | No destination or query | Publication/campaign status is local to publisher components | Render an unavailable state unless a real persisted activity query is separately implemented. Do not copy prototype purchase/refund rows. |
| Purchases | `UiV2View::Purchases` | `app/src/ui_v2/views/purchases.rs` — `PurchasesView` | Restyle real durable purchases and promotion claims. Keep active/disputed/refunded/revoked/unverified statuses and exclude public/timed access. |
| Accounts | Login panels and Settings account section | `app/src/ui_v2/views/login.rs`; `app/src/ui_v2/views/settings.rs`; auth state in `app/src/lib.rs` | Keep both pre-login and authenticated account access. An account-switch dialog may reuse these operations; do not equate a selected identity with an available signer. |
| Settings | `UiV2View::Settings` | `app/src/ui_v2/views/settings.rs` — `SettingsView` | Restyle into handoff side navigation. Preserve real account, signer, relay, Blossom, diagnostics, and NIP-49 behavior. Backup and Appearance remain honestly unavailable where unsupported. |

### Modal mapping

| Handoff modal | Current implementation | Plan |
|---|---|---|
| Payment | Inline in `GameDetailView` | Extract presentation into a shared dialog while keeping invoice, NWC, wallet opening, manual preimage recovery, account checks, and command order. No fake QR, countdown, or automatic payment detection. |
| Claim | Inline confirmation in `GameDetailView` | Extract presentation; retain campaign discovery/validation, signer wait, backend grant validation, persistence, and stale-account checks. |
| Download/install | Coarse `DetailOperation::Installing`; completion listener in `app/src/tauri_bridge.rs` | Add a dialog over the existing command. Wire real `download-progress` before showing percentages. Label phases accurately as download, integrity verification, and registration; do not claim extraction/native installation. |
| Receipt | Purchases rows only | Add read-only details/timeline for a selected real `DurableAcquisitionRecord`. Unknown timeline stages must not be synthesized. |
| Account switcher | Saved-account rows in Login/Settings | Consolidate UI over existing switch/reconnect operations and preserve flow-generation invalidation. |
| Revoke | No publisher revoke command | Do not enable. Campaign cancellation is not entitlement revocation. Show unavailable only if the handoff surface must remain visible. |
| Unsaved changes | Local Store Page and campaign `<dialog>` guards | Consolidate the shell/publisher navigation guard. Preserve Keep editing/Discard and add Save-and-leave only if it invokes a real persistence/publication action; current Store Page autosave is memory-only. |

### Existing surfaces absent or lighter in the prototype

These remain accessible from the shell, account menu, Settings, or publisher
management even if they are not primary top-navigation items:

- Achievements (`AchievementsView`) with NIP-58 loading, empty, error, cached,
  refresh, issuer, and selection states.
- Public Profile as a distinct destination, including NIP-05 verification and
  badges.
- Purchases and durable promotion claims.
- Advanced authentication: bunker identifier, Nostr Connect QR, NIP-07,
  encrypted local key, NIP-49 validation/export, reconnect, and account removal.
- Relay URLs/status/reconnect and insecure-public-relay policy.
- Blossom server preference, enablement, health, upload authentication,
  payment/error states, cancellation, and retry.
- Publisher build hashing, fulfillment mode, operator/distribution servers,
  relay publication progress, partial success, and recovery.
- Store Page associations, feature sections, languages, accessibility,
  canonical preview, clone/link, readiness, and reciprocal-pointer recovery.
- Technical diagnostics, malformed-data states, cache-refresh failures, and
  standalone-web unsupported states.

The prototype-only Design Notes page and `support.js` are not product surfaces
and will not be copied into the application.

## 2. Component mapping

| Required component | Current basis | Decision |
|---|---|---|
| Application shell | `UiV2Root` in `app/src/ui_v2/shell.rs` | Reuse state/view dispatch and relay reconciliation; replace visual composition with the 64px desktop shell. Retain compact-window overflow and focus behavior. |
| Top navigation | `TopBar` in `app/src/ui_v2/components/topbar.rs`; `NavItem` | Restyle/extend. Keep real search, relay state, profile identity, sign-in/out, and all secondary destinations. |
| Logo | Repeated text/mark in Login and TopBar | Extract a shared clipped `ArcadestrLogo`; no external runtime asset required. |
| Search field | Functional form in `TopBar` | Reuse behavior; restyle to 260 x 32px. Unlike the prototype, submission continues to create a real `BrowseRequest`. |
| Page tabs | Ad hoc controls in Browse, Library, Community, Purchases | Add shared `PageTabs`/`TabButton` primitive; migrate without flattening each screen’s state. |
| Publisher tabs | Nested `PublishViewState` controls | Extend with a shared publisher tab strip. Releases/Activity use honest unavailable states until backed. |
| Settings navigation | Settings card sections | Replace visual composition with a 220px side navigation; reuse section contents and operations. |
| Game card | `GameCard` in `app/src/ui_v2/components/game_card.rs` | Reuse decision model; restyle markup to exact clipped geometry and density. |
| Game artwork fallback | Existing fallback branches in `GameCard`, Store, Detail, Library | Consolidate into a reusable fallback that uses listing title/context and never fabricates art. |
| Favorite control | Current card presentation supports favorite state but no durable account persistence | Extend only when wired to real existing persistence; otherwise keep hidden/disabled rather than in-memory prototype state. |
| Hero carousel | Single featured listing in `StoreFrontView` | Extend to a carousel only from real listing data; use loading/fallback states and hide controls for fewer than two slides. |
| Status chip | Multiple local badge/status class branches | Extract shared semantic variants for acquisition, ownership, local install, relay, campaign, publication, and error states. Do not use one enum for all axes. |
| Action buttons | `Button`/`ButtonVariant` | Restyle and extend with amber primary, cyan action, neutral, ghost, and destructive variants plus busy/disabled/focus states. |
| Form fields | Repeated input/select/textarea markup | Add shared classes/components incrementally; retain native semantics, labels, validation, and date-picker behavior. |
| Panels | Repeated `v2-*panel/card` markup | Add shared panel classes for base, recessed, outlined, access, and warning panels; prefer styling reuse over a prop-heavy Rust abstraction. |
| Dialog shell | No common primitive; several native `<dialog>` implementations | Add a shared native-dialog shell with title/body/actions, focus restoration, Escape/backdrop policy, busy close blocking, and destructive/default focus options. |
| Progress indicators | Publish/upload progress plus desktop download events | Reuse real progress sources and common styling. Never use the prototype’s fixed 35/70/90 values. |
| Receipt timeline | No component | Add over real durable records and known chain statuses; unavailable details remain explicit. |
| Library row | Current installed/saved cards in `LibraryView` | Replace presentation, reuse reconciliation and semantic labels. |
| Empty state | Repeated local markup | Extract style/component while retaining screen-specific truthful messages/actions. |
| Relay partial-results state | Marketplace cache/refresh notices | Extend into a shared state banner. Backend relay failures currently can be swallowed, so do not promise complete results. |
| Error state | Repeated alerts | Restyle shared variants; preserve retry only where a real retry exists. |
| Loading skeleton | Mostly text/spinner loading states | Add skeleton primitives for cards, rows, hero, and panels, respecting reduced motion and retaining accessible status text. |

The canonical buyer-facing Store Page renderer remains
`StorePageRichDetail`; the editor preview must continue to use the same
sanitized typed data rather than a separate prototype renderer.

## 3. Token mapping

### Canonical target tokens

Phase 1 should define the handoff values once in `web/style/tailwind.css`, map
Tailwind aliases to them, and temporarily alias `--v2-*`/`--noir-*` consumers
while `app/src/ui_v2/theme.rs` is reduced. Values below are exact where the HTML
provides them.

| Semantic token | Handoff value | Current Noir value | Migration |
|---|---|---|---|
| Background | `#0b0b0d` | `oklch(0.16 0.02 270)` | Replace `--noir-background`. |
| Top bar / inset dialog | `#0d0d10` | No exact token | Add `--arc-surface-lowest`; map relevant lowest surface. |
| Card/panel | `#121215` | `oklch(0.215 0.025 273)` | Replace base surface. |
| Input/recessed | `#141417` | Current surface hierarchy varies | Add exact recessed surface. |
| Hero art base | `#141419` | No exact token | Add exact artwork surface. |
| Progress track | `#1a1a1d` | No exact token | Add progress-track token. |
| Primary text | `#eae6df` | `oklch(0.96 0.01 270)` | Replace foreground. |
| Heading/overlay | `#fff` | No separate heading token | Add on-image/heading token. |
| Muted text | Exact HTML levels from `oklch(0.4 0.01 60)` through `oklch(0.75 0.01 60)` | `oklch(0.68 0.03 270)` | Define named subdued/muted/secondary levels used by component context. |
| Amber accent | `oklch(0.78 0.16 60)` | Primary `oklch(0.78 0.16 300)` | Replace primary hue/chroma; dark on-accent text is `#0b0b0d`. |
| Cyan action | `oklch(0.8 0.14 195)` | Secondary `oklch(0.82 0.16 220)` | Replace secondary. Supporting exact variants: `oklch(0.55 0.13 195)`, `0.6`, `0.7`, `0.75`; hue-210 variants `oklch(0.78 0.13 210)` and `oklch(0.8 0.13 210)`. |
| Green success | `oklch(0.72 0.16 145)` | Success `oklch(0.75 0.15 160)` | Replace success. Supporting values: `oklch(0.6 0.13 145)`, `oklch(0.75 0.16 145)`. |
| Violet informational | `oklch(0.68 0.15 300)` | Primary currently violet | Retain as a non-primary semantic token. Supporting values: `oklch(0.6 0.13 300)`, `0.72 0.15 300`, `0.75 0.13 300`, `0.8 0.15 300`. |
| Destructive | `oklch(0.6 0.18 25)` | `oklch(0.66 0.22 20)` | Replace; text variants are `oklch(0.65/0.7/0.75 0.18 25)`. |
| Neutral borders | `rgba(255,255,255,.1/.12/.14/.15/.18/.2/.22/.25/.3)` | Generic 8% white plus Noir outline | Preserve exact opacity by component role. |
| Card border | `1px solid oklch(0.3 0.01 60)` | Generic outline | Add card-border token. |
| Search border | `1px solid oklch(0.32 0.01 60)` | Generic outline | Add control-border token. |
| Chrome separators | `oklch(0.4 0.03 60 / .3/.35)` and `oklch(0.32 0.02 60 / .5)` | No exact equivalents | Add exact separator variants. |
| Backdrop | `rgba(0,0,0,.6)` | Local dialog values | Centralize in dialog primitive. |
| Selection | `oklch(0.78 0.16 60 / .35)` | No exact token | Add global selection color. |
| Link | `oklch(0.8 0.14 195)`; hover `oklch(0.85 0.14 195)` | Current secondary aliases | Map link states to cyan. |

Other exact effects:

- Dot grid: `radial-gradient(circle, rgba(255,255,255,.02) 1px,
  transparent 1px)` at `24px 24px`.
- Account dropdown shadow: `0 8px 24px rgba(0,0,0,.5)`.
- Relay glow: `0 0 6px oklch(0.8 0.14 195)`.
- Favorite background: `rgba(0,0,0,.55)`.
- Hero controls: `rgba(10,10,12,.7)`.
- Disabled action: background `#2a2a2e`; generic unavailable
  background/text `#333`/`#888`.

### Typography

- Global family: `ui-monospace, 'SFMono-Regular', 'Consolas', monospace`.
  Replace current Inter/Space Grotesk mappings in both CSS variables and
  Tailwind families.
- Exact size set used by the prototype: `10`, `10.5`, `11`, `11.5`, `12`,
  `12.5`, `13`, `14`, `15`, `16`, `17`, `18`, `19`, `20`, `22`, `24`, `32`,
  and `34px`.
- Weights: `400`, `500`, `600`, `700`, `800`.
- Explicit line heights: `1`, `1.08`, `1.15`, `1.55`, `1.6`, `1.7`, `1.8`.
- Tracking: `.5px`, `1px`, `1.5px`, `2px`.
- Key roles: wordmark `800 19px/1` with `2px` tracking; hero title
  `800 32px/1.08` with `.5px` tracking; detail title `800 34px`; page title
  `800 20–22px`; modal title `800 16px`; eyebrow `800 11px` with `1.5px`
  tracking; body `400 11.5–13px` with `1.55–1.8` line height.

### Geometry and dimensions

- Radii in use: `3`, `4`, `5`, `6`, `8`, `10px`, plus circular `50%`.
  Current `0.75rem` and `1.25rem` Noir radii are not retained as defaults.
- Logo clip:
  `polygon(0 30%,70% 0,100% 30%,100% 100%,30% 100%,0 70%)`.
- Game-card clip:
  `polygon(0 0,calc(100% - 12px) 0,100% 12px,100% 100%,12px 100%,0 calc(100% - 12px))`.
- Exact spacing set observed: `1`, `2`, `4`, `5`, `6`, `7`, `8`, `9`, `10`,
  `11`, `12`, `13`, `14`, `16`, `18`, `20`, `22`, `24`, `26`, `28`, `30`,
  `60`, `70px`.
- Top bar: `64px` high with `0 28px` padding.
- Standard page inset: `26px 28px`; hero text inset: `26px 30px`.
- Search: `260 x 32px`, `0 10px` padding.
- Main/footer maximum width: `1440px`; main bottom padding `60px`.
- Hero: `38% / 62%`, minimum height `380px`; bottom controls `52px`;
  arrow controls `34 x 34px`; indicator `24 x 5px` active and `14 x 5px`
  inactive.
- Home and Browse: four columns, `18px` gap; artwork `160px` Home and
  `150px` Browse.
- Detail: `1fr 380px`, `26px` gap; hero art `340px`.
- Library artwork: `56 x 56px`; row padding `14px 16px`.
- Profile avatar: `70 x 70px`; profile grid three columns with `14px` gap.
- Publisher row artwork: `44 x 44px`.
- Store Page editor: `190px 1fr`, `22px` gap; Settings: `220px 1fr`,
  `26px` gap.
- Content maxima used by the handoff: editor sections `480/520/560/640px`,
  releases `700px`, manage `760px`, activity `820px`, design notes `920px`.
- Login wrapper: `440px`; dialog: `440px`, `max-height:86vh`, `26px`
  padding, `10px` radius.
- Payment QR area: `120 x 120px`; upload progress `7px`; install progress
  `8px`; upload thumbnail `70 x 44px`; media slot `80px`; preview hero
  `180px`.

At 924 x 540, content may scroll but must not switch to phone navigation. The
Tauri default remains 1280 x 800; visual comparison must test both dimensions.

## 4. State mapping

### Cross-cutting state axes

These remain separate in components, labels, and tests:

1. **Acquisition policy:** `Gated`, `Public`, or `TimedAccess` from
   `core/src/marketplace.rs` and `app/src/models.rs`.
2. **Durable acquisition/ownership:** purchase receipt or entitlement grant,
   including active/disputed/refunded/revoked/unverified status, from
   `core/src/ownership.rs`, `core/src/entitlements.rs`, and app models.
3. **Local device state:** saved-to-library, downloaded/registered artifact,
   compatibility, and any future update state. Installation never proves
   ownership.

Timed access uses `starts_at <= now < ends_at`. Zero price does not imply public
access. Campaign cancellation is prospective and does not revoke prior grants.

### Screen states

| Screen | Real state mapped to visuals | Missing/disabled behavior |
|---|---|---|
| Login/Accounts | Signed out; saved identity; connecting/connected/disconnected/failed signer; QR pending; validation; remove confirmation; stale/cancelled flow; target-specific methods. | Prototype instant success and generic imported-key activation are not used. NIP-49 import currently validates/decrypts but does not activate an account. |
| Store Home | Initial loading; cached results refreshing; partial listings; relay/command error with cached data; empty; featured listing; campaign enrichment; Store Page enrichment; artwork fallback. | Decorative fixed carousel count and fixture promotions are removed. |
| Browse | Initial/loading-more/refreshing; cached partial/error; real result count; no match; access/platform/query/sort filters; compatible/incompatible; campaign states; ownership and installation enrichment. | No fabricated recommendation rank. Backend relay-query failures can currently be swallowed, so “complete” must be phrased conservatively. |
| Game details | Ownership checking; paid invoice request/NWC/manual confirmation; claim campaign discovery and grant; public/timed/gated policy; timed upcoming/active/expired; add-to-library; compatibility; install busy/completion/failure; missing payment address; unsupported web; signed-out state. | No invoice countdown or passive payment detection exists. Do not expose fake Play, reviews, rating, repair, update, uninstall, or trust verification. |
| Library | Loading saved and installed records; account required; malformed coordinate; metadata available/missing; ownership confirmed/unconfirmed; public/timed/gated policy; compatibility; installed/not installed; empty. | Update availability is not currently modeled. Play/Update/Repair/Verify/Uninstall/Open Folder stay unavailable until backed. |
| Community | Honest unavailable/empty explanation. | Feed, posts, reactions, follows, players, and activity are not simulated. |
| Profile/Achievements | Profile loading/cache/error; NIP-05 verified/unverified; listings empty/ready; badge cached/loading/empty/error/unavailable/ready; showcase selection. | Profile edit/save and hard-coded achievements are not enabled. |
| Publisher Dashboard | Listings initial/cache/refresh/error/empty; active publisher filtering; campaign summary; publication entry. | Draft counts, revenue, sales, warnings, and activity are shown only if derived from real data. |
| Create Game | Four real stages; field validation; unsupported drafts; image selection/upload; build selected/hash progress/hash failure; provider reachability; signing; listing publication; relay progress; file upload; complete/partial/failed; stale account. | Prototype five-stage fixtures, one-click hash/provider, forced success/failure, and fixture review data are removed. |
| Manage Game | Current signed listing; listing coordinate/version/policy; fulfillment/operator/server; Store Page association; real promotions; diagnostics. | Unlist, disable access, individual revoke, and other danger actions remain absent unless their real commands exist. |
| Store Page editor | Load/new/link/clone; baseline clean/dirty; memory autosave; validation blockers/warnings/recommendations; media idle/selecting/ready/uploading/failed/cancelled; manual URL integrity clearing; association/pointer review; publishing/signing; per-event and per-pointer relay results; complete/partial/retry recovery; stale account/revision. | “Save draft” must be described as local in-memory draft retention, not durable persistence. Shell navigation must gain the same guard as editor Back. Remote Blossom deletion is not promised. |
| Releases | Current listing version/hash and installed version where available. | No release history, rollback, staged rollout, immutable release record, or changelog model; show unavailable rather than fixtures. |
| Promotions | Campaign upcoming/active/ended/cancelled/invalid; create/edit allowed by chain rules; publication result; pointer linked/unlinked/failure/retry; prospective cancellation. | Discounted and timed-promotion types are disabled. A single relay acceptance must not be described as the stronger two-relay confirmation used elsewhere. |
| Activity | Only concrete publication/campaign operation outcomes available in current component state. | No persisted publisher activity feed or analytics query; render unavailable. |
| Purchases/Receipt | Loading/account required/empty/error/web unavailable/ready; purchase versus promotion claim; active/disputed/refunded/revoked/unverified; amount and real record identifiers. | Public and timed access do not create records. Unknown receipt timeline events are not inferred. |
| Settings | Account/signer state; relay snapshot/reconnect; insecure relay policy; Blossom server enabled/preferred/health/error; NIP-49 export; diagnostics; copy result; explicitly unsupported backup/appearance. | Add/reset relay, backup/restore, reduce motion preference, or export actions are not made functional without commands. System reduced-motion CSS remains honored. |

### Modal states

- **Payment:** requesting invoice, invoice ready, copying/opening wallet,
  connecting NWC, paying, confirming purchase, success, recoverable failure,
  cancellation, and stale-account rejection. No countdown/expiration state until
  the backend supplies expiry.
- **Claim:** ready confirmation, discovering/validating campaign, waiting for
  signer/server, validating/persisting grant, success, campaign unavailable,
  signer rejection, backend failure, cancellation, stale account.
- **Download/install:** authorization resolution, download progress (only after
  typed listener wiring), SHA-256 verification, registry write, complete,
  cancellation/failure, hash mismatch/quarantine. It must not claim archive
  extraction or runnable installation.
- **Receipt:** real durable record, known chain status, amount, coordinate,
  order/record identifier, and unavailable technical fields.
- **Account switcher:** current/saved identity, signer connection state,
  reconnect/switch/add/remove, pending cancellation, flow-generation change,
  and late-result rejection.
- **Revoke:** unavailable because no individual-revocation command is exposed.
  Campaign cancellation is routed to the campaign confirmation dialog instead.
- **Unsaved changes:** clean/dirty, operation in progress, incomplete
  publication recovery, Keep editing, Discard, and only a truthful persistence
  action. Busy dialogs block unsafe backdrop/Escape closure.

### Safety and consistency requirements

- Continue validating relay-derived events before persistence or token use.
- Keep replaceable-event ordering and chain validation centralized.
- Preserve account-generation and request-correlation checks for auth,
  marketplace, payment, claim, library, purchases, publishing, Store Page, and
  Blossom operations.
- Preserve Blossom DNS resolution, address filtering, pinned destinations,
  disabled redirects/proxies, MIME/size/hash checks, mutation detection, and
  cancellation cleanup.
- Preserve fresh signed listing retrieval, server authorization, SHA-256
  verification, temporary-file cleanup, and quarantine behavior for downloads.
- The code audit found that native install currently resolves a signer before
  checking public/timed policy. The handoff requires anonymous public downloads;
  the UI migration must not worsen this or fake anonymity. Restoring anonymous
  installation safely is separate backend work and must retain fresh listing,
  policy-window, authorized-server, and hash validation.
- The ADP URL path does not currently match Blossom’s private-address/DNS
  protections. Do not broaden URL trust as part of UI work; SSRF hardening is a
  separate security task.

## 5. File-level implementation plan

Only listed files should change unless a phase discovers a required dependency;
such expansion should be reviewed before editing. New modules are named
explicitly to keep each patch narrow.

### Phase 1 — Design tokens and application shell

- `web/style/tailwind.css` — canonical handoff tokens, typography, selection,
  dot grid, reduced-motion preservation.
- `web/tailwind.config.js` — map colors, fonts, radii, shadows, and widths to the
  canonical variables.
- `app/src/ui_v2/theme.rs` — compatibility aliases and shell geometry; begin
  removing duplicate token declarations without rewriting unrelated rules.
- `app/src/ui_v2/shell.rs` — desktop shell structure and content boundary.
- `app/src/ui_v2/components/topbar.rs` — 64px desktop top bar.
- `web/index.html` — remove only conflicting global style declarations after
  confirming ownership; retain font/resource setup required by production.
- `app/src/lib.rs` — narrow legacy-style isolation only if reachable UI proves
  it necessary; do not remove legacy components in this phase.

### Phase 2 — Shared primitives and navigation

- `app/src/ui_v2/components/button.rs`
- `app/src/ui_v2/components/nav_item.rs`
- `app/src/ui_v2/components/page_header.rs`
- `app/src/ui_v2/components/game_card.rs`
- `app/src/ui_v2/components/topbar.rs`
- `app/src/ui_v2/components/mod.rs`
- New: `app/src/ui_v2/components/logo.rs`
- New: `app/src/ui_v2/components/tabs.rs`
- New: `app/src/ui_v2/components/status.rs`
- New: `app/src/ui_v2/components/feedback.rs`
- `app/src/ui_v2/theme.rs` — primitive styles only.

### Phase 3 — Store Home and Browse

- `app/src/ui_v2/views/store_front.rs`
- `app/src/ui_v2/views/browse_games.rs`
- `app/src/ui_v2/views/marketplace_loader.rs` only if a shared partial-result
  presentation needs a typed state already available there.
- `app/src/ui_v2/components/game_card.rs`
- `app/src/ui_v2/theme.rs` — page-specific styles.

### Phase 4 — Game details

- `app/src/ui_v2/views/game_detail.rs`
- `app/src/ui_v2/components/store_page_detail.rs`
- `app/src/ui_v2/theme.rs`
- Acquisition dialogs are deferred to Phase 12; Phase 4 preserves current
  inline flows while establishing the handoff detail layout.

### Phase 5 — Library and installation UI

- `app/src/ui_v2/views/library.rs`
- `app/src/ui_v2/views/game_detail.rs` — accurate install phase labels.
- `app/src/tauri_bridge.rs` — typed `download-progress` listener/model.
- `desktop/src/main.rs` only if the existing emitted payload requires a narrow,
  backward-compatible contract correction.
- `desktop/src/command_contracts.rs` — event contract test if payload changes.
- `app/src/ui_v2/theme.rs`

No extraction, launcher, updater, repair, or uninstall backend is added in this
visual phase.

### Phase 6 — Community and profile

- `app/src/ui_v2/views/social.rs`
- `app/src/ui_v2/views/profile.rs`
- `app/src/ui_v2/views/achievements.rs`
- `app/src/components/nip05_badge.rs`
- `app/src/components/badge_showcase.rs`
- `app/src/ui_v2/theme.rs`

### Phase 7 — Publisher Dashboard and game management

- `app/src/ui_v2/views/publish.rs`
- `app/src/ui_v2/views/mod.rs` if new unavailable subviews are split out.
- `app/src/ui_v2/shell.rs` — publisher destination wiring only.
- `app/src/ui_v2/theme.rs`
- New if separation improves reviewability:
  `app/src/ui_v2/views/publisher_releases.rs` and
  `app/src/ui_v2/views/publisher_activity.rs`; both remain real-data summaries
  or unavailable states, not fixtures.

### Phase 8 — Create Game workflow

- `app/src/components/publish.rs`
- `app/src/ui_v2/views/publish.rs` — wrapper/navigation only.
- `app/src/ui_v2/components/blossom_media_upload.rs` only for shared visual
  primitives; behavior changes wait for Phase 9.
- `app/src/ui_v2/theme.rs`

### Phase 9 — Store Page editor and Blossom media

- `app/src/ui_v2/views/store_page_publish.rs`
- `app/src/ui_v2/components/blossom_media_upload.rs`
- `app/src/ui_v2/components/store_page_detail.rs`
- `app/src/ui_v2/views/publish.rs` — editor entry/exit guard wiring.
- `app/src/ui_v2/theme.rs`

No expected core/desktop change: existing Store Page and Blossom commands are
reused. If behavior defects are discovered, handle them in separate commits in
`desktop/src/store_page_commands.rs`, `desktop/src/blossom_commands.rs`, or
`desktop/src/blossom_upload.rs`, not mixed with the visual patch.

### Phase 10 — Releases, promotions, campaigns, and activity

- `app/src/ui_v2/views/publish.rs`
- `app/src/campaign_management.rs` only if new visual controls require existing
  policy helpers to be exposed, not to broaden supported campaign types.
- `app/src/ui_v2/views/publisher_releases.rs` if created in Phase 7.
- `app/src/ui_v2/views/publisher_activity.rs` if created in Phase 7.
- `app/src/ui_v2/theme.rs`

No release/activity backend is expected in this migration. Any proposal to add
one requires a separate architecture and protocol plan.

### Phase 11 — Purchases, receipts, accounts, and settings

- `app/src/ui_v2/views/purchases.rs`
- `app/src/ui_v2/views/login.rs`
- `app/src/ui_v2/views/settings.rs`
- `app/src/lib.rs` — existing auth context wiring only if needed by the shared
  account switcher.
- `app/src/components/nip49_modal.rs`
- `app/src/ui_v2/theme.rs`
- New: `app/src/ui_v2/components/receipt_detail.rs`
- New: `app/src/ui_v2/components/account_switcher.rs`

### Phase 12 — Modal and transient-state consolidation

- New: `app/src/ui_v2/components/dialog.rs`
- New: `app/src/ui_v2/components/payment_dialog.rs`
- New: `app/src/ui_v2/components/claim_dialog.rs`
- New: `app/src/ui_v2/components/install_dialog.rs`
- New: `app/src/ui_v2/components/unsaved_changes_dialog.rs`
- `app/src/ui_v2/components/mod.rs`
- `app/src/ui_v2/views/game_detail.rs`
- `app/src/ui_v2/views/login.rs`
- `app/src/ui_v2/views/settings.rs`
- `app/src/ui_v2/views/publish.rs`
- `app/src/ui_v2/views/store_page_publish.rs`
- `app/src/ui_v2/components/blossom_media_upload.rs`
- `app/src/components/nip49_modal.rs`
- `app/src/ui_v2/shell.rs` — shell-wide dirty-navigation interception.
- `app/src/ui_v2/theme.rs`

There is no new revoke dialog until a real command and scope exist.

### Phase 13 — Accessibility and fidelity pass

- All touched `app/src/ui_v2/**` components, limited to semantics, focus order,
  labels, keyboard behavior, reduced motion, overflow, and screenshot deltas.
- `web/style/tailwind.css`
- `app/src/ui_v2/theme.rs`
- `desktop/tauri.conf.json` only if review explicitly approves a desktop minimum
  window size; do not silently make the current resizable window fixed.

Carried over from Phase 7: the WebKitGTK webview used by the desktop shell
renders the contents of a **closed** `<details>` element, so collapsed
disclosures leak their body outside the surrounding panel. Phase 7 added a
workaround scoped to publisher surfaces only:

```css
.v2-publisher-studio details:not([open]) > *:not(summary) { display: none; }
```

The defect is application-wide and affects every `<details>` in `ui_v2`
(Store Page editor, Library, game detail diagnostics). Phase 13 should decide
whether to generalize the rule into the canonical stylesheet and remove the
publisher-scoped copy, after confirming no surface relies on always-visible
disclosure content.

### Phase 14 — Final regression review

- Test-only changes beside affected modules when missing coverage is found.
- No generated `web/dist/**` files are committed.
- No protocol or backend format changes are expected.

## 6. Migration phases

Each phase is independently reviewable and commit-ready. Do not combine a
visual phase with unrelated security, protocol, or persistence work.

1. **Design tokens and application shell** — canonical amber/cyan/green
   terminal tokens, monospace typography, background, 64px shell, 1440px
   content boundary, and safe desktop overflow. No screen redesign yet.
2. **Shared primitives and navigation** — logo, buttons, tabs, status chips,
   feedback states, clipped game card, functional search, relay/account menus,
   and retained access to all production destinations.
3. **Store Home and Browse** — real-data hero, cards, filters, cache/partial/
   empty/error/loading states, and exact 924 x 540 visual comparison.
4. **Game details** — handoff hierarchy and access panel while preserving the
   existing inline acquisition flows and all three state axes.
5. **Library and installation UI** — row presentation, real saved/installed
   reconciliation, accurate verified-download wording, and typed progress.
6. **Community and profile** — truthful Community unavailable state, real
   profile, NIP-05, listings, achievements, and badge states.
7. **Publisher Dashboard and game management** — real listing/campaign summary,
   management hierarchy, and honest Releases/Activity placeholders.
8. **Create Game workflow** — restyle the four production stages, hashing,
   provider, readiness, relay publication, partial, success, and failure states.
9. **Store Page editor and Blossom media** — handoff editor chrome around all
   production fields, upload states, typed preview, associations, dirtiness,
   relay confirmation, and partial recovery.
10. **Releases, promotions, campaigns, and activity** — top-level publisher
    hierarchy using real campaign data and explicit unavailable states where no
    backend exists.
11. **Purchases, receipts, accounts, and settings** — real durable history,
    receipt detail, signer-aware account controls, relay/Blossom settings, and
    honest unavailable settings.
12. **Modal and transient-state consolidation** — shared native-dialog shell,
    payment/claim/install extraction, account switcher, receipt details, local
    destructive confirmations, and global unsaved navigation guard.
13. **Accessibility, keyboard navigation, focus behavior, and visual fidelity**
    — focus trap/restore, Escape/backdrop policy, tab/arrow behavior, accessible
    status announcements, contrast, reduced motion, 924 x 540 screenshot pass,
    and 1280 x 800 desktop pass.
14. **Final regression review** — acquisition/account/publishing/upload/install
    regression matrix, desktop/web compile gates, focused tests, diff hygiene,
    and generated-output exclusion.

## 7. Validation plan

Run focused validation first. CSS/markup-only phases do not justify broad core
tests; bridge or desktop contract changes do.

| Phase | Smallest command validation | Manual/visual validation |
|---|---|---|
| 1 | `cargo fmt --all -- --check`; `cargo check -p arcadestr-app`; `git diff --check` | Run Tauri and compare shell at 924 x 540 and 1280 x 800; verify no phone navigation at 924px and no clipped essential controls. |
| 2 | `cargo test -p arcadestr-app ui_v2`; `cargo check -p arcadestr-app`; `git diff --check` | Keyboard through top nav, search, account and relay menus; verify every current destination remains reachable. |
| 3 | `cargo test -p arcadestr-app marketplace`; `cargo check -p arcadestr-app`; `git diff --check` | Compare Home screenshot and Browse prototype; test initial, cached-refresh, partial, empty, artwork fallback, incompatible, and error states using real state/test seams. |
| 4 | `cargo test -p arcadestr-app game_detail`; `cargo check -p arcadestr-app`; `git diff --check` | Compare details screenshot; exercise paid, claim, public, timed upcoming/active/expired, owned, installed, incompatible, signed-out, and failure states. |
| 5 | `cargo test -p arcadestr-app library`; `cargo check -p arcadestr-app`; if bridge/desktop changes, `cargo check -p arcadestr-desktop` and focused command-contract tests; `git diff --check` | Compare Library screenshot; verify account library/device install distinction, real progress, cancellation, error, and hash-failure wording. |
| 6 | `cargo test -p arcadestr-app achievements`; `cargo check -p arcadestr-app`; `git diff --check` | Compare profile screenshot; test profile loading/error/NIP-05 and badge loading/empty/error; confirm Community remains truthful. |
| 7 | `cargo test -p arcadestr-app publish`; `cargo check -p arcadestr-app`; `git diff --check` | Compare dashboard screenshot; verify empty/error/partial listing states and that Releases/Activity contain no fixtures. |
| 8 | Focused tests in `app/src/components/publish.rs` via `cargo test -p arcadestr-app publish`; `cargo check -p arcadestr-app`; `git diff --check` | Exercise every stage, validation, hash progress/error, provider failure, signing, relay partial, upload failure, success, and account switch. |
| 9 | `cargo test -p arcadestr-app store_page`; `cargo test -p arcadestr-app blossom`; `cargo check -p arcadestr-app`; `git diff --check` | Compare editor screenshot; exercise clean/dirty, upload success/failure/cancel/stale, manual URL integrity clearing, readiness, preview, partial publication, retry, and navigation guard. |
| 10 | `cargo test -p arcadestr-app campaign`; `cargo check -p arcadestr-app`; `git diff --check` | Test upcoming/active/ended/cancelled/invalid, pointer failure/retry, and prospective cancellation; confirm unsupported promotion types/actions are disabled. |
| 11 | `cargo test -p arcadestr-app purchases`; focused auth/settings tests; `cargo check -p arcadestr-app`; `git diff --check` | Exercise no account, signer offline, switch/reconnect/remove, purchase statuses, receipt details, relay failure, Blossom health, and unavailable settings. |
| 12 | Focused dialog/component tests plus `cargo test -p arcadestr-app`; `cargo check -p arcadestr-app`; if progress contracts changed, `cargo check -p arcadestr-desktop`; `git diff --check` | Keyboard-only modal pass: initial focus, Tab trap, Escape, backdrop, busy blocking, focus restore, stale account, and dirty navigation across every shell destination. |
| 13 | `cargo fmt --all -- --check`; `cargo test -p arcadestr-app`; `cargo check -p arcadestr-app`; `cargo check -p arcadestr-web --target wasm32-unknown-unknown`; `git diff --check` | Full screenshot matrix at 924 x 540 and 1280 x 800 in the running Tauri app; inspect focus, zoom, long text, overflow, reduced motion, loading, empty, partial, success, failure, and pending states. |
| 14 | Run the final gate below. | Repeat critical real flows on Tauri: account switch during requests/uploads, purchase/claim, public/timed acquisition, verified download, Store Page partial retry, campaign cancellation, and settings. |

### Final gate

Workspace packages are confirmed as `arcadestr-app`, `arcadestr-core`,
`arcadestr-desktop`, and `arcadestr-web`.

```bash
cargo fmt --all -- --check
cargo check -p arcadestr-app
cargo test -p arcadestr-app
cargo check -p arcadestr-desktop
cargo check -p arcadestr-web --target wasm32-unknown-unknown
git diff --check
```

If standalone-browser behavior was touched, also run:

```bash
cargo check -p arcadestr-web --target wasm32-unknown-unknown --features web
```

For visual validation, run the configured Tauri application from the repository
root with `cargo tauri dev`, resize the main window to **924 x 540**, and compare
against all six handoff screenshots. Repeat at the configured default
**1280 x 800**. Use equivalent real records or empty/error test seams; never add
fixture game, payment, entitlement, profile, achievement, or activity data to
production for screenshot matching.

`trunk build` may update `web/dist`; generated `web/dist/**` output must remain
uncommitted. Preserve any pre-existing working-tree changes while validating.

## Recommended first implementation phase

Start with **Phase 1: Design tokens and application shell** only. The exact
scope is:

1. define the handoff palette, monospace typography, spacing, radii, shadows,
   dot grid, and selection values in `web/style/tailwind.css`;
2. remap `web/tailwind.config.js` aliases without changing component behavior;
3. add temporary compatibility aliases and shell-only rules in
   `app/src/ui_v2/theme.rs` rather than rewriting the full stylesheet;
4. recompose `UiV2Root` and `TopBar` to the 64px desktop chrome, 1440px content
   boundary, and safe compact-window overflow;
5. retain all existing destinations, functional search, relay status, account
   state, focus behavior, and reduced-motion handling;
6. do not restyle individual pages, add modals, change routing, or alter backend
   behavior in the first commit.

This creates a stable visual foundation while keeping the first review narrow,
reversible, and independent of application-state changes.
