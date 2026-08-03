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

### 1. Store Page eight-tab runtime geometry

Carried since Phase 9. The Store Page editor was migrated onto the canonical
token system as a whole surface, but its eight tab panels have never been
compared against the handoff individually:

Basic Info, Description, Media, Feature Sections, Requirements, Languages,
Accessibility, Links.

Verify per tab: field spacing, control grouping, label geometry, control
height, explanatory text, validation placement, the tab-specific media
layouts, responsive stacking, footer and readiness-panel alignment, and tab
overflow behavior. Remove any remaining visual approximations where the
handoff gives an exact value.

### 2. Dialog runtime verification

Carried since Phase 12. Five dialogs have never been seen rendered:

- Blossom upload
- media expansion
- install flow
- campaign confirmation
- Store Page discard

Each is covered by close-policy contract tests, and all five route through the
canonical `Dialog` primitive — so the scroll-lock, focus, and Escape behavior
verified in Phase 13 applies to them by construction. What is missing is
visual confirmation of their geometry and content.

## Accepted deviations

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

### 5. Legacy Tailwind utilities in `date_time_picker.rs`

`app/src/components/date_time_picker.rs` still styles itself with legacy
Tailwind utility clusters (`text-on-surface-variant`,
`bg-surface-container-high`, `rounded-lg`) instead of the canonical `--arc-*`
tokens. It is the last meaningful holdout. Phase 13 corrected its
accessibility defects (explicit `aria-expanded`, hidden decorative icons) but
did not restyle it, because a blind restyle could not be verified at runtime —
the picker is only reachable from campaign authoring, which needs a listing.

Fold this into the work for item 1 or 2, when that state is reachable.

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
