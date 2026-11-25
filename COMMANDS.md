# Arcadestr Development Commands

Run commands from the repository root unless a command explicitly changes directory.
Prerequisites and initial setup are in [README.md](README.md).

## Desktop and Web

| Task | Command |
|---|---|
| Desktop development with hot reload | `(cd desktop && cargo tauri dev)` |
| Production desktop bundle | `(cd desktop && cargo tauri build)` |
| Build desktop Rust binary | `cargo build -p arcadestr-desktop` |
| Build frontend with its build stamp | `cargo run -p arcadestr-xtask -- frontend-build` |
| Build release frontend with its build stamp | `cargo run -p arcadestr-xtask -- frontend-build --release` |
| Standalone browser development | `(cd web && trunk serve --features web)` |
| Run an already-built desktop binary | `./target/debug/arcadestr-desktop` |

The desktop binary requires matching frontend assets. Prefer `cargo tauri dev`
for development and `cargo tauri build` for distribution; building the Rust
binary alone does not rebuild the frontend. Tauri configuration is under
`desktop/`, so invoke its CLI there. Stop your development process with `Ctrl+C`.

## Checks and Tests

| Task | Command |
|---|---|
| Check one crate | `cargo check -p <crate>` |
| Core tests | `cargo test -p arcadestr-core --features native` |
| Desktop tests | `cargo test -p arcadestr-desktop` |
| App tests | `cargo test -p arcadestr-app` |
| Desktop WebView WASM check | `cargo check -p arcadestr-web --target wasm32-unknown-unknown` |
| Standalone browser WASM check | `cargo check -p arcadestr-web --target wasm32-unknown-unknown --features web` |
| Formatting | `cargo fmt --all -- --check` |
| Whitespace validation | `git diff --check` |
| Frontend target checks | `./scripts/check-frontend-targets.sh` |

Run focused checks first. None of the commands above need the `adp-server`
repository.

## Cross-implementation (adp-server) Tests

| Task | Command |
|---|---|
| All interop suites at the pinned adp-server revision | `scripts/interop-test` |
| Real adp-server process over HTTP/Nostr only | `scripts/interop-test black-box` |
| Gate 7 acceptance flows | `scripts/interop-test crossflow` |
| Compile regtest drivers and run their unit tests | `scripts/interop-test regtest-harness` |

These suites use independent Cargo workspaces and need adp-server checked out
beside Arcadestr at the revision in `tests/interop/adp-server.rev`
(`--fetch` clones it). Follow [tests/interop/README.md](tests/interop/README.md);
for Lightning regtest scenarios, see [scripts/README.md](scripts/README.md).

## Debugging and Analysis

- Backend debugger setup: [.vscode/BACKEND_DEBUG.md](.vscode/BACKEND_DEBUG.md).
- Development logs: `(cd desktop && RUST_LOG=debug cargo tauri dev)`.
- Available analysis patterns: `./scripts/analyze-patterns --list-patterns`.
- Example analysis: `./scripts/analyze-patterns --pattern=mutex-patterns --output=markdown`.
- Local data reset: read `./scripts/clear_arcadestr_data.sh` before using it;
  it removes local application data and is not a routine build step.

For a stale or blank frontend, stop your own development process and rebuild
with `cargo run -p arcadestr-xtask -- frontend-build` before restarting it.
Check for another process on port 5173 before starting another Trunk server.
