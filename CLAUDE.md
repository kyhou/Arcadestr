# Arcadestr Development Guide

## Project Overview

Arcadestr is a Nostr-powered indie game marketplace built with Rust, Tauri, and Leptos.

## Development Commands

### Run Development Server

**Standard command (60s timeout, full output):**
```bash
cd /home/joel/Sync/Projetos/Arcadestr/desktop && timeout 60 cargo tauri dev 2>&1
```

This command:
- Runs the Tauri development server
- Uses a 60-second timeout to prevent hanging
- Shows full compilation output without tailing
- Captures both stdout and stderr

### Build Commands

**Check core crate:**
```bash
cargo check -p arcadestr-core
```

**Check desktop crate:**
```bash
cargo check -p arcadestr-desktop
```

**Check app crate:**
```bash
cargo check -p arcadestr-app
```

**Build desktop app:**
```bash
cargo build -p arcadestr-desktop
```

### Testing

**Run core tests:**
```bash
cargo test -p arcadestr-core
```

## Project Structure

- `/core` - Core business logic (Nostr, NIP-46, storage)
- `/desktop` - Tauri desktop application
- `/app` - Leptos web frontend
- `/docs` - Documentation

## Key Features Implemented

### Async NIP-46 Authentication

The app now uses deferred connection for NIP-46 signers (like Yakihonne):
- Returns immediately after user approval
- Connection establishes in background
- Shows real-time connection status (🟡 Connecting / 🟢 Connected / 🔴 Failed)

### Connection Status Flow

1. User pastes bunker URI
2. App returns in <2 seconds with "Connecting..." status
3. Connection establishes in background via LazyNip46Signer
4. First signing request triggers actual NIP-46 handshake
5. UI updates to show "Connected" status

## Troubleshooting

**If build hangs:**
- Use `timeout 60` prefix to limit execution time
- Check for file locks: `lsof +D target/`
- Clean build: `cargo clean -p arcadestr-desktop`

**Common warnings:**
- Unused imports (safe to ignore during development)
- Deprecated base64 functions (pre-existing, not critical)
- Lifetime syntax warnings (cosmetic only)

## Git Workflow

**Commit changes:**
```bash
rtk git add <files>
rtk git commit -m "type: description"
```

**Check status:**
```bash
rtk git status
```
