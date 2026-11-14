# Post-Migration Follow-Ups

The `docs/design-handoff/` UI migration (Phases 1–13) shipped at commit
`3df8e79`. This file records the items that were reviewed during the Phase 13
final pass and deliberately left open.

**None of these blocks release.** Each is either gated on state the development
environment could not reach safely, or an accepted deviation with a stated
reason. Items 1 and 2 are the two that still need real verification work; the
rest are decisions, recorded so they are not rediscovered as bugs.

## Blocked on a real authored listing

During Phase 13 verification the configured relays returned no listings. Every
publisher sub-route is gated behind "Select a managed game first", so the
surfaces below were unreachable. Producing a listing requires outward-facing
relay publication, which the UI phases forbid, so no fixture was introduced.

Both items need a session with a genuine authored listing on a reachable relay.

### 1. Store Page eight-tab runtime geometry — verified

Verified 2026-11-14 with a publisher account, by computed-style audit of every
tab (Basic Info, Description, Media, Feature Sections, Requirements,
Languages, Accessibility, Links): palette, text size >= 10.5px, monospace,
no horizontal overflow, labelled controls, tab bar without overflow.

Fixed during verification:

- Basic Info controls measured 47, 54 and 46px against 36px elsewhere. Field
  labels are grids and were stretched by taller grid-row siblings; they now
  use `align-content: start`. The date picker's `min-h-12` and its icons'
  28px `text-lg` line height also forced the height.
- Field labels now follow the handoff (700, 11px, dim); panel titles are amber
  uppercase kickers.

### 2. Dialog runtime verification — three of five verified

Verified: Blossom upload, Store Page discard (unsaved changes), and campaign
confirmation (discard). All are native modal dialogs named by their title,
move focus inside (to the safe action where one exists), close on Escape,
and pass the palette and text-size audit.

Fixed during verification:

- Editor-scoped rules (`.v2-store-page-editor h2`, `p`) leaked into dialogs
  opened from the editor, shrinking titles to 12.5px and descriptions to
  10.5px. Dialog text rules are now `.arc-dialog .x`, so page rules cannot
  reach them.
- Dialog kickers are amber 800 and titles 800, per the skill.
- The campaign confirmation's destructive actions (Cancel Promotion, Discard
  changes) used the amber primary button. The accept button now follows the
  dialog tone (`CampaignConfirmation::accept_class`).

Still unverified:

- **Install flow.** Checked 2026-11-14 with a buyer account that owns three
  games, but all three are regtest listings without a `platform` tag. Core
  refuses to plan an install for those (`resolve_platform_eligibility`,
  `NoLaunchTierReason::MissingPlatform`), so the dialog cannot open. It
  needs an owned game whose listing declares the host platform, which means
  buying or claiming one (e.g. `linux-x86_64` listings in the Store).
- **Media expansion.** No Store listing visible on 2026-11-14 has Store Page
  media; it needs a published Store Page with media.

Fixed while checking the install flow: a listing with no `platform` tag was
reported as "Compatibility unknown" / "Checking this device" / "Platform
detection must complete", implying it would become installable. Game Detail
and Library now have a distinct `NoPlatformDeclared` state ("Not
installable", "No platform declared · Not installable on any device"), and
Library's Needs attention filter includes it. Purchase and install remain
blocked as before.

Observation, not changed: **Publish Promotion** publishes immediately, with no
confirmation step, unlike Cancel Promotion. Decide deliberately whether
publication should be confirmed.

### 12. Skill drift from the handoff HTML — resolved

`Arcadestr.dc.html` is the source of truth; the `arcadestr-design` skill
summarises it. The skill's palette omitted violet, which led to replacing the
handoff's `VIOLET` with blue (tertiary/info tokens, timed, pending and draft
chips) and with amber ("Needs attention"). Restored to the HTML: tertiary/info
`oklch(0.68 0.15 300)`, timed chip fill `/ .16`, attention panel border
`oklch(0.5 0.13 300 / .5)`, fill `/ .08`, text 600 12px `oklch(0.75 0.13 300)`.
Dialog titles are 800 16px, as in the HTML modals. Blue stays only where the
HTML uses it: tag chips and outlined details actions. The skill now lists violet
and points to the HTML as its reference.

## Accepted deviations

The handoff HTML is a reference, not the final design. Deviations are fine
when deliberate and recorded here.

### 13. Richer empty states than the handoff

The HTML's Library empty state is one muted line ("Nothing here yet.") with
no action. The app's empty states have a headline, an explanation of what
makes content appear, and a next step where one exists (Library and
Purchases: **Browse the store**). Accepted 2026-11-14.

### 3. Profile has no full-public-key copy control

The Phase 13 carry-over list assumed a copy-full-key control existed on the
Profile surface and asked that it be preserved. It does not exist in
`app/src/ui_v2/views/profile.rs` and appears never to have.

The public key renders once, abbreviated, via the shared `npub_fallback_label`.
Adding a copy control is new UI, so Phase 13 left it alone rather than
introduce a control under an accessibility mandate. Worth deciding
deliberately: there is currently no way to obtain the full key from Profile.

### 4. Blossom server rows crowd at 924 x 540

In the Blossom settings section at the constrained viewport, server names and
URLs wrap mid-string and the row action buttons stack awkwardly against the
enabled/preferred controls. The layout remains usable and does not overflow
horizontally; it is dense rather than broken.

### 5. Legacy Tailwind utilities in `date_time_picker.rs` — partly addressed

Verified at runtime from campaign authoring. Fixed: control height (removed
`min-h-12`), icon line height (`leading-none`), and STARTS/ENDS labels raised
from 10px to 10.5px. The picker still styles itself with legacy Tailwind
utilities rather than `--arc-*` tokens; it renders on-palette, so a full
restyle remains optional.

### 6. Low-contrast decorative borders

Panel and control borders measure between 1.2:1 and 2.2:1 against their
surfaces, below the WCAG 1.4.11 3:1 threshold for non-text UI components:

| token | ratio on `--arc-surface` |
|---|---|
| `--arc-border-subtle` | 1.30:1 |
| `--arc-border-card` | 1.37:1 |
| `--arc-border-control` | 1.47:1 |
| `--arc-border-default` | 1.72:1 |
| `--arc-border-strong` | 2.23:1 |
| `--arc-separator` | 1.18:1 |

**Retained intentionally.** These borders are the handoff's dark-theme look,
and raising them would amount to redesigning the surface treatment rather than
fixing a defect. The indicator that carries the accessibility weight is the
focus ring, which measures **8.94:1**, and every text and status-chip
foreground was brought to WCAG AA during Phase 13.

Revisit only as a deliberate design decision, not as a contrast bug.

## Measurement notes

Two traps in this codebase cost real time during the migration. Both are
documented in the code that resolves them; repeated here so they are not hit
again.

**Contrast must not be measured inside the webview.** The desktop WebKitGTK
webview's canvas `fillStyle` does not resolve `oklch()` and returns stale
values silently rather than failing. Phase 10 corrected chip contrast by eye
for this reason and recorded no trustworthy ratio. Measure by converting OKLCH
to sRGB directly, compositing alpha against the real backdrop, then applying
the WCAG 2.1 formula. Compact status chips render at 10.5px, so the 4.5:1
normal-text threshold applies, not the large-text one.

**`overflow: hidden` does not lock the viewport in WebKitGTK.** Measured on the
Settings route (page 2397px, viewport 752px), the background still scrolled
with the rule applied to `html`, and to `html` and `body` together.
`html, body { height: 100%; overflow: hidden }` locks but snaps the page to the
top and does not restore. Only a scroll-compensating pinned body holds and
preserves position; see `app/src/ui_v2/components/modal_background.rs`.

Related: `inert` cannot be applied to `.arc-app-shell`. Dialogs render inside
that subtree, and HTML inertness carves out no exception for a modal dialog
under an explicitly inert ancestor, so it would make the dialog unreachable.

## Non-UI findings from the Signal Terminal UI/UX pass

Recorded during the 2026-11 UI/UX pass against the `arcadestr-design` skill.
Items 7–10 were fixed in a follow-up pass. Item 11 is an environment
precondition rather than a code defect.

### 7. Partial relay fetches are reported as errors — resolved

On Nostr anyone can run a relay, so some relay is always down, slow or
refusing. Partial coverage is the normal outcome of a refresh, not a failure.
`arcadestr_core::marketplace_fetch::classify_relay_fetch` applies that policy
to the relay manager's raw outcome:

- any relay answered (EOSE), or any listing arrived: successful refresh, and
  the completion event fires; coverage is logged at info level;
- no relay answered and nothing arrived:
  `MarketplaceFetchError::Unreachable { failed, timed_out }`;
- the fetch could not run: `MarketplaceFetchError::Failed { message }`.

The error crosses the command bridge as a `kind`-tagged object and is decoded
once in `invoke_fetch_marketplace_stream`; callers match on the variant, never
on error text. `RelayManagerError::FetchIncomplete` is unchanged and still
reports every non-answering relay, for callers that need full coverage.
Covered by `core/tests/relay_fetch_outcomes.rs` against local relays (partial
coverage, answered-empty, events-then-timeout, nothing answering).

### 8. Generic invoke errors are debug-formatted — resolved

`app/src/tauri_invoke.rs` now passes backend error strings through as
written, sends structured errors as their JSON, and falls back to a readable
sentence for empty rejections (`invoke_error_text` / `bridge_error_text`).
`JS error: JsValue(...)` no longer reaches views. The sanitized allowlist for
NWC and receipt commands is unchanged.

### 9. Icon font loads from Google Fonts at runtime — resolved

Material Symbols Outlined (variable, Apache-2.0) is bundled at
`web/assets/fonts/` with its license and copied to `dist/fonts/` by Trunk.
`web/style/tailwind.css` declares the `@font-face` (`font-display: block`) and
the full `.material-symbols-outlined` rule that Google's stylesheet used to
supply. The Google Fonts links are removed. Verified in Chromium with all
external requests blocked: the font loads and icons render as glyphs.

### 10. Trunk does not watch the crates the UI lives in — resolved

`web/Trunk.toml` now has `[watch] watch = [".", "../app", "../core"]`, so UI
edits under `app/` rebuild the frontend during `cargo tauri dev`. Restart the
dev command once for Trunk to pick up the new config.

### 11. Store Page editor and dialogs still need a publisher account — done

Items 1 and 2 remain blocked, but now on the account rather than on relays.
The relays return listings, but none is authored by the active account, so
"Select a managed game first" still gates every publisher sub-route. Verifying
them needs a session signed in as a listing's publisher.

## Measurement notes (continued)

**Linux MCP-bridge screenshots are html2canvas renders, not native captures.**
`webview_screenshot` falls back to html2canvas on Linux. That renderer ignores
`clip-path` (notched cards appear square), drops `repeating-linear-gradient`
(hatched artwork appears flat), draws input text too low (placeholders look
clipped), renders closed `<details>` content, and shows `<summary>` markers as
`1.`/`2.`. Confirm any such defect with computed styles or
`document.elementFromPoint` before treating it as real.
