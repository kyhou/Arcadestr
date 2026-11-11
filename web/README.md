# Frontend build modes

The default WASM frontend is the supported **desktop bridge** UI. It calls the
native Tauri application's commands and listens for its events. The `web` feature
instead compiles the **standalone browser** UI with explicit desktop-only
fallbacks; it is not a browser implementation of the desktop services.

From the repository root:

```sh
# Standalone browser (the WASM target is required).
cargo check -p arcadestr-web --target wasm32-unknown-unknown --features web --locked
# Default desktop-bridge frontend.
cargo check -p arcadestr-web --target wasm32-unknown-unknown --locked
# Both frontend modes and native desktop, without launching an application.
bash scripts/check-frontend-targets.sh
```

From `web/`, `trunk build --features web` builds standalone mode; `trunk build`
builds desktop-bridge mode. Use `--dist /tmp/opencode/<new-directory>` for isolated
validation rather than overwriting an existing desktop frontend checkpoint.
`cargo check --features web` on a native target is not the standalone browser
build: browser authentication modules are intentionally WASM-specific.

## Standalone capability boundary

Existing browser support includes rendering the shared UI, NIP-07 extension
identity connection/disconnection, and browser-local account metadata. The
extension remains responsible for user permission; connecting an identity does
not create a browser payment or publishing executor.

Native relay marketplace discovery/streaming, native profile enrichment and
reconnection are unavailable. The shell can render supplied/debug preview data;
that is not live marketplace discovery. In debug standalone builds only,
`localStorage['arcadestr.debug.storefront']='1'` exposes storefront inspection
without authentication; it does not enable native operations.

Desktop-only capabilities remain unavailable: purchases and wallet execution,
settlement/refund execution and recovery, receipt import/synchronization, known
grant reconciliation, publisher signing/publication/uploads, native archive
selection, installation/download management and launch, and the debug NIP-44
signer probe. Their existing bridge fallbacks remain errors; affected receipt,
refund and signer-probe controls explicitly explain the desktop requirement and
are disabled in standalone mode. No Tauri event listeners are installed for those
services in standalone builds, even if a page supplies `window.__TAURI__`.

Successful compilation does not establish live relay coverage, durable browser
ownership recovery, wallet compatibility, production packaging or release
authorization. Standalone mode does not bypass ADP Server authorization.

The shared UI currently references an external icon font. When that font is
blocked or unavailable, icon ligature names can render as wide fallback text and
affect narrow-screen layout. Offline font bundling and full-shell mobile layout
acceptance are not provided by this compilation fix.
