# Arcadestr

A decentralized game marketplace built on NOSTR with Lightning payments.

## What is Arcadestr?

Arcadestr is a decentralized marketplace for indie games that runs entirely on the NOSTR protocol. Unlike traditional game stores, Arcadestr has no central server, no custodial payments, and no platform fees. Publishers maintain full custody of their games and earnings, while buyers pay sellers directly via Lightning Network zaps.

What makes Arcadestr different is its commitment to decentralization at every layer. Game listings are NOSTR events (kind 30078) broadcast to public relays, making them censorship-resistant and permanently available. Payments flow directly from buyer to seller through Lightning Network invoices generated via LNURL-pay, with no intermediary holding funds. The entire application logic runs client-side, with your private key never leaving your signer app.

## Architecture

Arcadestr is organized as a Rust workspace with four crates:

- **`core`** — Business logic: NOSTR event handling, Lightning/NIP-57 zap payments, signer abstraction (NIP-46/NIP-07), and relay communication.

- **`app`** — Leptos UI components shared between desktop and web targets. Contains the marketplace interface, publish form, detail view with buy flow, and all styling.

- **`desktop`** — Tauri v2 shell that exposes native Tauri commands for NOSTR operations. Targets Windows, macOS, and Linux with native system integration.

- **`web`** — WASM entry point for browser deployment via Trunk. Uses the same `app` components but runs entirely in the browser.

## NOSTR Protocol Usage

Arcadestr implements several NIPs to provide a complete decentralized marketplace experience:

- **NIP-07** — Browser extension signer support for the web target. Users authenticate via extensions like Alby or nos2x.

- **NIP-46** — Remote signer / Nostr Connect support for the desktop target. Connects to signer apps like Nsec.app or Amber via `nostrconnect://` URIs.

- **NIP-57** — Zap payments for purchases. Generates Lightning invoices via LNURL-pay and publishes zap receipt events (kind 9735) to confirm payment.

- **NIP-78** — Parameterized replaceable events for game listings. Uses kind 30078 with a `d` tag for the listing ID, enabling updates and preventing duplicates.

## Prerequisites

Before building Arcadestr, ensure you have the following installed:

- **Rust** (stable, 2021 edition) — Install via [rustup](https://rustup.rs/)
- **Trunk** — For web builds: `cargo install trunk`
- **Tauri CLI v2** — For desktop builds: `cargo install tauri-cli --version "^2"`
- **`wasm32-unknown-unknown` target** — `rustup target add wasm32-unknown-unknown`
- **A NOSTR signer** — For web: Alby browser extension (NIP-07). For desktop: Nsec.app, Amber, or any NIP-46 compatible signer app.
- **A Lightning wallet** — With LNURL-pay support for receiving payments (sellers only).

## Building and Running

### Desktop

```bash
# Development (hot reload)
cargo tauri dev

# Production build
cargo tauri build
```

The desktop app opens a native window with the Arcadestr interface.

### Web

```bash
cd web
trunk serve              # Development server at http://localhost:8080
trunk build --release    # Production build to web/dist/
```

The web target requires a NIP-07 browser extension for authentication.

### Core Tests

```bash
cargo test -p arcadestr-core
```

## Authentication

Arcadestr supports two authentication flows depending on your target:

### Web (NIP-07)

1. Install the Alby or nos2x browser extension
2. Open Arcadestr in your browser
3. Click "Connect" — the extension will prompt for approval
4. Approve the connection in the extension popup

### Desktop (NIP-46)

1. Open a NIP-46 compatible signer app (Nsec.app, Amber, etc.)
2. In Arcadestr desktop, click "Connect with NIP-46"
3. Copy the `nostrconnect://` URI from your signer app
4. Paste it into Arcadestr and click Connect
5. Approve the connection in your signer app

## Publishing a Game

1. Log in using either NIP-07 (web) or NIP-46 (desktop)
2. Click "Publish" in the sidebar
3. Fill in the listing details:
   - **Listing ID** — A unique slug (e.g., `my-game-v1`)
   - **Title** — Display name for your game
   - **Description** — What your game is about
   - **Price** — In satoshis (0 for free)
   - **Download URL** — Direct HTTPS link to your game files
   - **Tags** — Comma-separated categories
   - **Lightning Address** — Your lud16 address for receiving payments (required for paid games)
4. Click "Publish Listing" — the listing is broadcast to NOSTR relays as a kind-30078 event

## Buying a Game

1. Browse listings on the main page
2. Click a listing to open the detail view
3. For paid games, click "Buy for N sats"
4. Arcadestr generates a Lightning invoice via the seller's LNURL-pay address
5. Copy the invoice or click "Open in Wallet"
6. Pay with any Lightning wallet
7. The seller's wallet publishes a NIP-57 zap receipt confirming payment
8. Download the game using the download link

Free games can be downloaded immediately without payment.

## Known Limitations

- **NIP-46 compatibility** — Tested with Nsec.app and Amber. Other signer apps may have compatibility issues.

- **Event IDs** — For fetched listings, the event ID falls back to the listing slug (d-tag) when the actual event ID is not available. This will be fixed in a future update to properly track event IDs.

- **Clipboard API** — Requires HTTPS in browser environments. Local development may not support clipboard operations.

- **Payment verification** — Arcadestr generates Lightning invoices but does not verify on-chain payment status. Payment confirmation relies on the seller's wallet publishing zap receipts.

- **Web target limitations** — NIP-46 is not supported in the web target. Use NIP-07 browser extensions for web authentication.

- **Search and filtering** — Not yet implemented. The browse view shows the 20 most recent listings without search or category filtering.

## License

MIT
