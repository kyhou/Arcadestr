# Arcadestr Instructions

## Architecture

- `core`: protocol, storage, Nostr, and domain logic.
- `app`: Leptos UI shared by desktop/web.
- `desktop`: Tauri integration and native services.
- `web`: browser-specific entry points.
- Keep domain and protocol behavior out of UI crates.

## Commands

- Focused check: `cargo check -p <crate>`
- Core tests: `cargo test -p arcadestr-core`
- Desktop tests: `cargo test -p arcadestr-desktop`
- App tests: `cargo test -p arcadestr-app`
- WASM check: `<exact command>`
- Format: `cargo fmt --all -- --check`
- Diff validation: `git diff --check`

Run focused tests first. Run workspace-wide verification only after multi-crate or protocol changes.

## Repository Rules

- Never use `unwrap()` in production code.
- Do not hold mutex guards across `await`.
- Preserve unrelated working-tree changes.
- Do not commit unless explicitly requested.
- Prefer narrow patches over complete-file rewrites.
- Follow existing error and storage patterns in the affected module.

## Protocol Work

- Treat replaceable-event ordering and chain validation as centralized invariants.
- Preserve event IDs, signatures, authors, tags, and timestamps.
- Validate relay-derived data before persistence or token issuance.
- Add adversarial tests for malformed, stale, forked, or mismatched events.

## Verification

- Small localized change: affected crate check and focused tests.
- Cross-crate change: checks and tests for every affected crate.
- Protocol/storage change: include malformed-input and persistence tests.
- UI flow change: verify the relevant desktop or web flow.