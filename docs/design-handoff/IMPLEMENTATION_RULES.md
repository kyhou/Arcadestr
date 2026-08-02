# Arcadestr UI Handoff — Implementation Rules

These rules apply to every UI migration phase under `docs/design-handoff/`.

Phase prompts should contain only phase-specific scope, state mappings, tests, captures, and validation. Do not repeat this file unless a phase explicitly overrides one of its rules.

## 1. Sources of truth

Use this priority order:

1. `docs/design-handoff/screenshots/` for visible appearance.
2. `docs/design-handoff/Arcadestr.dc.html` for exact dimensions, structure, spacing, typography, colors, borders, shadows, and clipped geometry.
3. `docs/design-handoff/README.md` for design tokens and intended interactions.
4. `docs/design-handoff/arcadestr-design-spec-updated.md` for required screens and UI states.
5. `docs/design-handoff/arcadestr-architecture-handoff-updated.md` for architectural constraints.
6. `docs/design-handoff/IMPLEMENTATION_PLAN.md` for migration order and file mappings.
7. Existing Arcadestr code for all behavioral semantics.

The handoff controls presentation. Existing application code controls functionality.

## 2. Product boundary

Arcadestr is a desktop game store and publisher console built with Rust, Tauri, and Leptos.

Do not reinterpret it as a generic classifieds marketplace.

Preserve existing Store and Browse discovery, game details, acquisition policies, ownership and entitlements, campaigns and free claims, Library and installation state, Store Page authoring, Blossom uploads, releases, publisher workflows, purchases and receipts, accounts and settings, relay behavior, signer behavior, protocol rules, and security rules.

## 3. Production architecture

Implement UI changes with the existing Rust, Leptos, Tauri, CSS/Tailwind, routing, state management, backend commands, and protocol models.

Do not introduce React, another frontend framework, prototype JavaScript, `support.js` as a production dependency, duplicated client-side business logic, or a second design-token system.

The prototype runtime may remain in the handoff directory only for local visual reference.

## 4. Behavioral preservation

A UI phase must not change backend or protocol behavior unless the phase explicitly authorizes it.

Preserve account binding, stale-response rejection, signer requirements, relay loading and partial results, listing validation, acquisition resolution, ownership and entitlement rules, campaign validation and cancellation semantics, payment and receipt behavior, download and installation behavior, exact-byte verification, SSRF and unsafe-URL protections, upload cleanup, local persistence, event formats, and tag semantics.

Do not simplify real behavior to match the prototype.

## 5. State integrity

Keep independent state axes separate, including acquisition policy, ownership or entitlement, local installation, compatibility, campaign state, download state, publication state, listing validity, Store Page completeness, release state, account and signer state, and relay completeness.

Do not collapse unrelated state into one generic status.

Presentation components must not infer business state. Feature code must map authoritative state into typed visual props.

## 6. No fabricated data

Do not fabricate games, listings, artwork, prices, campaigns, claims, ownership, installations, reviews, ratings, sales, revenue, views, popularity, players, playtime, followers, social activity, analytics, progress, sizes, time estimates, version information, or relay completeness.

Unsupported handoff content must be omitted, disabled with an explicit explanation, or represented as unavailable.

Do not use zero as a substitute for unknown or unavailable data.

## 7. Honest asynchronous states

Loading, partial, empty, error, unavailable, and signed-out states are distinct.

- Do not show empty state before loading completes.
- Keep already loaded data usable during partial relay failure.
- Do not label unavailable functionality as an empty result.
- Do not replace independently usable sections with one blocking spinner.
- Do not claim relay publication succeeded solely because a local command completed.
- Do not present optimistic completion before authoritative results arrive.
- Retry actions must use existing retry behavior.
- Busy actions must reject duplicate activation.

## 8. Visual implementation

Reproduce the handoff as exactly as practical: layout, dimensions, spacing, typography, colors, borders, shadows, clipped shapes, control sizes, responsive desktop behavior, feedback states, and focus/hover states.

Use the canonical token system created in Phase 1. Do not introduce repeated raw values when a semantic token exists.

At narrower desktop widths, preserve functionality, prevent overlap and horizontal overflow, stack or wrap where necessary, and do not create separate mobile navigation or a separate mobile design.

## 9. Shared components

Reuse and extend existing shared primitives before creating page-local duplicates.

Current shared primitives include, where applicable: `PageContainer`, `ClippedPanel`, `IconButton`, shared buttons and inputs, page and publisher tabs, status chips, artwork states, game cards, and loading/empty/partial/error feedback.

Avoid speculative abstraction. Add only components needed by the current phase or clearly required by mapped later phases.

Do not delete compatibility wrappers until all consumers are migrated.

## 10. Accessibility

Every phase must preserve or improve semantic headings, visible keyboard focus, keyboard navigation, labels for icon-only controls, logical reading order, reduced-motion behavior, non-color-only status communication, accessible loading/progress semantics, dialog focus trapping and restoration, scroll-safe dialogs at 924×540, and contextual disabled-state explanations.

Do not create conflicting nested interactive controls.

## 11. Security and privacy

Do not expose private keys, signer secrets, tokens, NIP-98 headers, payment preimages, decrypted private content, encrypted receipt content, private buyer or seller data, full sensitive filesystem paths, or unsafe raw URLs.

Use safe abbreviated identifiers and deliberate disclosure controls where needed.

Do not weaken existing media, URL, protocol, or host validation to match the design.

## 12. Integration boundaries

Normally allowed:

- restructuring markup for the phase’s pages;
- adding phase-specific presentation components;
- adapting existing state into typed view models;
- adding pure state-selection helpers;
- refining shared components for genuine cross-page defects;
- adding focused tests;
- using temporary devtools-only visual sheets.

Normally forbidden:

- protocol changes;
- backend command changes;
- persistence changes;
- broad refactors;
- unrelated page redesigns;
- mock production data;
- unsupported controls;
- new product behavior;
- speculative features.

Any exception must be explicit in the phase prompt.

## 13. Runtime visual verification

Code inspection is insufficient.

For each phase:

1. Run the Tauri application.
2. Capture migrated surfaces at the handoff reference viewport where possible.
3. Capture at the effective 924×540 constrained viewport.
4. Capture narrower desktop states when new breakpoints are introduced.
5. Capture loading, partial, empty, error, fallback, and important active states.
6. Use temporary devtools-only sheets only when real runtime state cannot produce an implemented state.
7. Remove all temporary fixture code before finishing.

Compare hierarchy, dimensions, spacing, typography, geometry, card or row density, controls, feedback placement, focus behavior, and responsive stacking.

Do not claim exact fidelity from CSS values alone.

## 14. Validation baseline

Unless the phase requires more, run:

```bash
cargo test -p arcadestr-app
cargo check -p arcadestr-app
cargo check -p arcadestr-desktop
cargo check -p arcadestr-web --target wasm32-unknown-unknown
cargo fmt --all -- --check
git diff --check
```

Use focused tests for changed components and state selection. Do not run broad unrelated protocol suites unless implementation boundaries were crossed.

Existing warnings may remain only when unchanged and unrelated.

## 15. Worktree preservation

Before and after runtime builds:

- preserve unrelated worktree changes;
- do not stage or commit unless explicitly instructed;
- keep `web/dist/index.html` byte-identical to its pre-task state;
- verify no temporary capability, fixture, route, or permission diff remains;
- verify `support.js` is not referenced by production code;
- inspect the complete diff before finishing.

## 16. Final review checklist

Every phase must verify:

1. Scope remained limited to the requested phase.
2. Existing routes remain reachable.
3. No mock production data was added.
4. No business logic moved into visual components.
5. No unsupported feature was simulated.
6. State axes remain independent.
7. Loading, partial, empty, error, and unavailable states remain distinct.
8. Account switching cannot display stale data.
9. Keyboard focus and accessible labels are present.
10. Temporary visual fixtures were removed.
11. No prototype runtime entered production.
12. `web/dist/index.html` was restored exactly.
13. Unrelated worktree changes were preserved.
14. All actionable phase findings were fixed.

## 17. Phase deliverable format

Report:

- files changed;
- surfaces completed;
- real-state mappings used;
- unsupported controls omitted or disabled;
- runtime captures;
- focused tests and results;
- validation results;
- remaining fidelity differences;
- unrelated worktree changes preserved;
- recommended next-phase scope.

Do not commit unless explicitly instructed.
