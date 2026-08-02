# Handoff: Arcadestr Storefront + Publisher Console

## Overview
Arcadestr is a desktop game store and developer-publishing app built on Nostr (Tauri + Leptos in production). This bundle is an interactive HTML/React click-through prototype covering the player-facing storefront (home, browse, game details, library) and the early Publisher Console (developer dashboard, game creation wizard, Store Page editor, campaign modal). It demonstrates layout, visual language ("Signal Terminal" aesthetic), state transitions, and the specific semantic distinctions the product requires (acquisition method vs. ownership state vs. local install state).

## About the Design Files
The bundled HTML file is a **design reference** built with inline-styled React-like markup (Design Component format) — it is not production code to copy directly. Recreate these designs in the target codebase's actual environment (Rust/Leptos + Tailwind, per `arcadestr_CODEBASE.md`), using its existing components, routing, and the Noir OKLCH token system already in the app. Where no equivalent component exists yet, follow this prototype's structure and states as the spec.

## Fidelity
**High-fidelity.** Colors, type, spacing, and states below are exact values pulled from the prototype's inline styles — implement them precisely, not as loose inspiration. Layout grids/breakpoints are desktop-only (no responsive/mobile requirement).

## Design tokens

**Background / surface**
- App background: `#0b0b0d`, with a faint radial dot-grid overlay (`rgba(255,255,255,.02)` 1px dots)
- Panel / card surface: `#121215`
- Recessed surface (search box, inputs, art placeholder): `#141417` / `#141419`
- Top bar background: `#0d0d10`
- Borders (default): `rgba(255,255,255,.1)` to `rgba(255,255,255,.18)`; `oklch(0.3 0.01 60)` on game cards

**Text**
- Primary text: `#eae6df` (warm off-white), headings `#fff`
- Dim/secondary text: `oklch(0.6 0.01 60)` (labelled `DIM` in code), muted metadata `oklch(0.52–0.62 0.01 60)`

**Accent colors**
- Amber (primary accent — CTAs, active tab indicator, selection): `oklch(0.78 0.16 60)` (labelled `AMBER` in code) — used at full opacity for buttons/active state, and at `/.14` alpha for active-tab background tint
- Cyan/teal (secondary action, e.g. "View details"): `oklch(0.8 0.13–0.14 195–210)`, border `oklch(0.5 0.13 210)`
- Green (success/verified — payment confirmed, ownership proof): `oklch(0.72 0.16 145)`
- Link default: `oklch(0.8 0.14 195)`

**Typography**
- Font stack (entire app): `ui-monospace, 'SFMono-Regular', 'Consolas', monospace` — deliberate terminal/monospace aesthetic ("Signal Terminal")
- Weight/size patterns: headings `800 weight`, 32–34px for hero/detail titles, 19–22px for section/page titles, 15–17px card titles; body copy 11–13px; eyebrow/section labels `800 weight, 11px, letter-spacing 1.5px`, uppercase (e.g. "ABOUT", "ACCESS", "OWNERSHIP STATUS")
- Buttons: `800 weight, 12–13px`

**Shape**
- Standard border-radius: 4–8px on panels, inputs, buttons
- Game cards use a notched/clipped-corner `clip-path` (polygon cutting the top-right and bottom-left corners ~12px) — a signature card shape, not a plain rounded rect
- Circular icon buttons (favorite toggle, carousel arrows): 28–34px diameter

## Screens / Views

### Login
- Centered card (max-width ~360px) on the dark background, logo mark (amber notched-square glyph + "ARCADESTR" wordmark) above.
- Card: "Connect your account" heading, explanatory copy about signature-based auth (no private key required), three stacked buttons: primary amber "Continue" style action + two secondary outlined actions ("Use browser extension", "Scan QR").

### Store Home (`nav:'store', storeScreen:'home'`)
- Persistent top bar: logo/wordmark (click → home), nav links, search field (decorative, non-functional in prototype), account menu / sign-in button.
- Hero: full clickable media region for the featured game — left column has kicker rule, title (34px), developer/platform line, tag chips, description, primary action button (color driven by acquisition state) + secondary "View details" outlined button. Right side is the artwork region with layered radial gradients as placeholder art, a fallback "NO ART" hint state, and a bottom control bar (carousel index "01/04", two 34px square prev/next arrow buttons) anchored at the bottom, z-indexed above the caption/gradient.
- "More on the store" grid: 4-column grid of notched game cards, each with 160px art region (or striped no-art fallback), title overlaid bottom-left, circular favorite toggle top-left (never in a clipped corner), and an expiration badge only when a real deadline exists (not shown by default).

### Browse (`storeScreen:'browse'`)
- Page title "Browse games", filter row: three `<select>` dropdowns (Acquisition: All/Paid/Free/Owned, Platform: All/Windows/Mac/Linux/Web, Sort: Recommended/Newest).
- 4-column grid of the same notched game card component (150px art height), same favorite-toggle and no-art fallback treatment as Home.

### Game details (`storeScreen:'details'`)
- Large 340px hero art banner with bottom-gradient and overlaid title (34px).
- Left column: "ABOUT" section (label + description copy).
- Right column: "ACCESS" panel — acquisition-state chips/copy — plus a conditional "OWNERSHIP STATUS" panel (shown only when the game is owned) using green success-colored text such as "PERMANENT OWNERSHIP" (never "You own it", to avoid falsely implying ownership when the actual state may be timed/incompatible).
- Primary action button is contextual and independent of ownership: **Play / Install / Update / Incompatible** — this is a third semantic axis, distinct from acquisition method (paid/claim/public/timed) and from ownership (owned/unowned).

### Library (`nav:'library'`)
- List rows (not a grid): 56×56 art thumbnail, title + status metadata, action button per row. Supports tabs (`libraryTab: 'owned'` etc., though only "owned" is wired in the prototype).

### Community / Profile (`nav:'community'`)
Sub-screens via `communityScreen`: `profile`, `editProfile`, `achievements`, `feed` — tab row at top (amber active-tab underline/color, dim inactive).
- **Profile**: identity display, badges, "My games" grid (developer's own published/draft games) with status pill (PUBLISHED/DRAFT).
- **Edit profile**: form with labeled inputs (Display name, About) — label text is `700 weight, 11px, dim`, stacked above each field.
- **Achievements / Feed**: tab-selectable, minimal placeholder content (kept honest/empty rather than fabricated per spec — no fake activity feed).

### Publisher Console (`nav:'publish'`)
Tab row (Dashboard / Create / Store Page / Releases / Promotions / Activity — `publishTabs`), amber active state.
- **Dashboard** (`publishScreen:'dashboard'`): "My games" management grid, each card opens to Manage.
- **Create game wizard** (`publishScreen: createBasic → createPricing → createBuilds → createProvider → createReview`): multi-step linear flow. Builds step includes a simulated hash-verification action; Provider step includes a simulated fulfillment-provider connection; Review step has the publish action, followed by a "just published" success state.
- **Manage** (`publishScreen:'manage'`): single published/draft game's management view, entry point to Store Page editor.
- **Store Page editor** (`publishScreen:'storePage'`, `storePageTab`): 8 tabs — **Basic, Description, Media, Video, Tags, Links, Requirements, Preview** — amber active-tab indicator with a colored dot per tab. Media tab models a full Blossom upload lifecycle (`mediaUploadPhase`: idle → selected → uploading → success, with retry/remove). Editing any field sets `unsavedChanges:true`; navigating away triggers an **unsaved-changes confirmation modal** (Discard / Save & leave / Cancel) rather than silently discarding drafts.
- **Releases / Promotions / Activity**: placeholder tabs reserved for build/release management, campaign management, and publish-activity log respectively — not yet fleshed out in the prototype; treat as required scope per the design spec, not as cut features.

### Settings (`nav:'account'`)
Left nav (`settingsNav`, amber active bg tint) with sections: Home, Security, Network, Backup, Appearance, About. Content area swaps per section.

### Modals (overlay, dim backdrop, click-outside-to-close except during in-progress states)
- **Payment** (`modal:'payment'`): Lightning invoice states — waiting → confirmed (simulated in prototype), "Waiting for payment" language (never "processing order").
- **Claim** (`modal:'claim'`): free-campaign claim flow — ready → waiting on signer approval → claimed.
- **Download** (`modal:'download'`): install pipeline steps — Downloading… → Verifying file integrity… → Installing… → Complete, each with a percentage (35/70/90/100%).
- **Receipt** (`modal:'receipt'`): shows "Purchase receipt" for paid acquisitions vs. "Permanent access record" for claimed/free ones — type and amount fields adapt accordingly (amount shows "—" for non-paid).
- **Revoke**, **Account switcher** (`modal:'revoke'`, `'accountSwitcher'`): present but minimal in current build.
- **Unsaved-changes confirm** (`modal:'unsavedConfirm'`): guards navigation away from a dirty Store Page draft.

## Interactions & Behavior
- All navigation across major sections (`goHome`, `goStore`, `goLibrary`, `goCommunity`, `goPublish`, `goSettings`, `goProfile`) is routed through `guardedNav`, which intercepts navigation when the Store Page editor has unsaved changes and opens the unsaved-changes modal instead of navigating directly.
- Tabs and sub-screens are plain state swaps (no transition/animation modeled) — instant content swap on click.
- Hero carousel arrows and index counter are decorative in this prototype (state not wired to advance) — implement as a real carousel in production.
- Card favorite toggle is a separate circular hit target from the card's own click-through (which opens the game) — do not let it bubble into card navigation.
- Media upload, payment, claim, and download flows are modeled as explicit multi-phase state machines (see phase names above) — each phase is a distinct visual state, not a single spinner.

## State Management
Key state fields (see `Component.state` in the prototype) map directly to production concerns:
- `nav`, `storeScreen`, `communityScreen`, `publishScreen`, `settingsSection`, `storePageTab`, `libraryTab` — screen/tab routing
- `modal` + phase fields (`paymentPhase`, `claimPhase`, `downloadPhase`, `mediaUploadPhase`) — one active modal at a time, each with its own phase enum
- `unsavedChanges`, `pendingNavAction` — Store Page draft-guard pair; `pendingNavAction` stores the navigation the user attempted so it can be replayed after "Save & leave"
- `browseAcquisition`, `browsePlatform`, `browseSort` — Browse filter state
- `selectedGameId` — drives Game Details and Manage screens

## Assets
No real artwork is used — all "art" is CSS gradient placeholders (`artBg` fields) or an explicit "NO ART" striped fallback state. Do not treat gradient placeholders as the intended final visual; production should use real capsule/hero art and gallery media per the Store Page editor spec, with the same fallback treatment when art is missing.

## Screenshots
`screenshots/` contains reference captures: 01-home, 02-library, 03-community-profile, 04-publish-dashboard, 05-store-page-editor, 06-game-details.

## Files
- `Arcadestr.dc.html` — the current prototype (all screens above), exactly as run in the live preview
- `support.js` — the runtime this prototype's preview loads it with (included as-is so the file opens/runs standalone; not part of the target codebase)
- `arcadestr-architecture-handoff-updated.md` — production architecture context (Tauri/Leptos/Rust stack, protocol model, acquisition/entitlement rules)
- `arcadestr-design-spec-updated.md` — full functional design spec this prototype implements against
