# Arcadestr

A decentralized indie game marketplace and desktop launcher built on
[Nostr](https://nostr.com/) with Lightning payments.

Publishers sign their listings, choose where to host their builds, and sell
directly to players. Players can discover games, collect verifiable access
credentials, and install and launch games from one desktop app.

**Status:** active development, with desktop pre-releases available. Arcadestr
and its distribution protocol have not entered production. **Buyer and refund
payment dispatch are disabled in default desktop builds**; activation requires
separately reviewed release features. See the
[payment release policy](docs/PRODUCTION_PAYMENTS.md).

## For Players and Publishers

### Players

- Browse games, search currently loaded listings and Store Page text, and filter
  by platform.
- Explore publisher profiles and rich Store Pages with screenshots,
  descriptions, requirements, and links.
- Acquire games through Lightning purchases, claim-and-keep promotions, or
  explicitly public and timed access.
- Keep account access records separate from device installations. Install
  verified packages, launch games, uninstall, and optionally keep offline copies
  for reinstalling without another download.
- Sign in with a remote signer on desktop, a browser extension on web, or an
  encrypted local Nostr key.

### Publishers

- Manage games through a per-game publishing hub, with drafts and listing updates.
- Publish Store Pages and upload media to configurable Blossom servers.
- Inspect build archives, declare supported platforms, and define signed launch
  profiles and defaults.
- Provision distribution through ADP servers, upload builds, and retry partial
  publications.
- Manage promotional campaigns, fulfillment authorizations, and access
  credentials.

Zero price alone does **not** grant download access. Public access, timed access,
and claim-and-keep promotions are explicit policies; public or timed access does
not create durable ownership.

## How It Works

Arcadestr separates discovery, payments, and file delivery rather than relying
on one central storefront backend:

1. **Discovery:** publishers sign NIP-99 game listings (`kind:30402`) and
   Arcadestr Store Pages (`kind:30407`) and publish them to Nostr relays.
2. **Acquisition:** paid acquisitions use verifiable marketplace receipts;
   non-payment acquisitions can use entitlement grants. Lightning checkout
   supports managed payment plans, including experimental split settlement and
   recovery for partial payments.
3. **Delivery:** publisher-selected servers implement the
   [Arcadestr Distribution Protocol (ADP-01)](ADP-01.md). The desktop app checks
   signed listing metadata and package hashes before extracting and installing
   builds, then starts the selected game profile without a shell.

ADP operators may charge disclosed distribution fees. Lightning wallets may
also charge routing fees; standard NWC does not provide an enforced fee ceiling.
A partial payment without a validated receipt does not establish ownership.
See [payment guarantees and recovery](docs/PRODUCTION_PAYMENTS.md).

### Protocols

| Protocol | Role |
|---|---|
| NIP-07 / NIP-46 | Browser-extension signing / desktop remote signing |
| NIP-47 (NWC) | Desktop Lightning wallet connection |
| NIP-57 | Zap payment proofs |
| NIP-65 | Publisher write-relay discovery |
| NIP-98 | Authenticated HTTP requests to ADP services |
| NIP-99 | Classified listings (`kind:30402`) |
| NIP-58 | Profile badges and achievements |
| [NIP-102 draft](NIP-102.md) | Marketplace receipts (`kind:1020`) |
| [NIP-103 draft](NIP-103.md) | Non-monetary entitlement grants |
| [ADP-01 draft](ADP-01.md) | Distribution, fulfillment authorization, access policies, packages, and launch metadata |
| Blossom | Store Page media hosting |

Store Pages, entitlements, and split-settlement extensions use experimental or
provisional event kinds, not established interoperable standards. The
consolidated `ADP-01.md` is the canonical specification; former amendments are
historical records, not additional requirements.

## Desktop and Web

Both targets share a Rust/Leptos UI, but the browser is not a replacement for
the native client.

| Capability | Desktop (Tauri v2) | Standalone web (WASM) |
|---|---|---|
| Marketplace browsing and Store Pages | Yes | Yes |
| Sign-in | NIP-46 or encrypted local key | NIP-07 or browser-encrypted local key |
| Native build inspection and ADP publishing | Yes | Desktop required |
| Managed NWC checkout and settlement recovery | Subject to build payment policy | Desktop required |
| Install, launch, uninstall, and offline copies | Yes | Desktop required |

## Downloads

Prebuilt desktop installers for Linux (DEB, RPM), Windows (setup
EXE, MSI) and macOS (Intel and Apple Silicon DMG) are published on
[GitHub Releases](https://github.com/kyhou/Arcadestr/releases) with a
`SHA256SUMS` file. Future releases replace AppImage with a native Arch Linux
package; `v0.1.0` is unchanged. Installers are **not code-signed**. See
[docs/releasing.md](docs/releasing.md) for installation, checksum
verification, the expected Windows and macOS warnings, and how releases are
published.

## Prerequisites

- **Rust 1.89 or newer** — Install a current stable toolchain via
  [rustup](https://rustup.rs/).
- **WASM target:** `rustup target add wasm32-unknown-unknown`.
- **Trunk:** `cargo install trunk --locked` (also used for the desktop frontend).
- **Tauri CLI v2:** `cargo install tauri-cli --version "^2" --locked`.
- **Native desktop dependencies:** follow the
  [Tauri prerequisites](https://v2.tauri.app/start/prerequisites/) for your OS.
  Linux needs WebKitGTK 4.1 and system development libraries.

Browsing does not require a signer. For account features, use a compatible
signer or a local key. Desktop wallet integration uses a NWC connection;
publishers receiving zap payments need an LNURL-pay-enabled Lightning address.

## Quick Start

Run these commands from the repository root:

```sh
# Desktop development (starts the frontend and native app)
(cd desktop && cargo tauri dev)

# Desktop release bundle (rebuilds and stamps the frontend)
(cd desktop && cargo tauri build)

# Standalone browser development at http://localhost:5173
(cd web && trunk serve --features web)
```

The desktop binary needs matching frontend assets; `cargo build` alone does not
rebuild them. Prefer the Tauri commands above. A release build does not
automatically enable payment dispatch.

On Linux/Wayland, run `bash scripts/install-linux-dev-icon.sh` once to show the
Arcadestr icon for the development build.

### Checks and Tests

```sh
cargo test -p arcadestr-core --features native --no-fail-fast
cargo test -p arcadestr-app
cargo test -p arcadestr-desktop
cargo check -p arcadestr-web --target wasm32-unknown-unknown --features web
cargo fmt --all -- --check
git diff --check
```

See [COMMANDS.md](COMMANDS.md) for focused checks, frontend build commands, and
debugging guidance.

Arcadestr builds and tests on its own. The [adp-server](https://github.com/kyhou/adp-server)
reference implementation is needed only for the explicitly invoked
cross-implementation suites (`scripts/interop-test`). These run in
independent Cargo workspaces against a pinned server revision; see
[tests/interop/README.md](tests/interop/README.md).

## Workspace

| Crate | Responsibility |
|---|---|
| `core` | Domain and protocol logic, validation, Nostr, ADP, payments, and storage |
| `app` | Shared Leptos UI and desktop/browser bridges |
| `desktop` | Tauri shell, native services, publishing, installation, and process launch |
| `web` | WASM entry point for the desktop WebView and standalone browser |
| `xtask` | Frontend builds and asset fingerprints used to reject stale release assets |

## Security and Current Boundaries

- **Keys:** remote signing keeps your identity key in the signer. Local-key
  accounts instead entrust it to Arcadestr's encrypted storage. Desktop local
  accounts use a protected app-data master-key file, not the OS keyring.
- **Availability:** relay publication does not guarantee permanent storage.
  Downloads depend on available distribution servers unless you retain an
  offline copy or already have the game installed.
- **Games are executable software:** package verification checks integrity and
  signed metadata, not whether a publisher's game is safe to run.
- **Pre-releases:** installers are unsigned, there are no automatic updates, and
  browser capabilities remain limited as shown above.

Read [desktop security boundaries](docs/SECURITY_BOUNDARIES.md) before using real
keys, and [the payment release policy](docs/PRODUCTION_PAYMENTS.md) before
building a payment-enabled artifact.

## Documentation

- [Development commands](COMMANDS.md) — build, test, and debug.
- [Codebase guide](CODEBASE.md) — architecture and implementation map.
- [Documentation index](docs/README.md) — current references versus historical plans.
- [Releases](docs/releasing.md) — installation, checksums, and publishing.
- [ADP-01](ADP-01.md), [NIP-102](NIP-102.md), and [NIP-103](NIP-103.md) — protocol drafts.
- [Client conformance tracking](docs/protocol-implementation-gaps.md).
- [Interop tests](tests/interop/README.md) and [regression scripts](scripts/README.md).

## License

Licensed under either the [MIT License](LICENSE-MIT) or the
[Apache License, Version 2.0](LICENSE-APACHE), at your option
(`MIT OR Apache-2.0`).

Third-party dependencies and assets retain their own licenses.
