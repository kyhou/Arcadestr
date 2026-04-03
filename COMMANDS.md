# Arcadestr Development Commands Reference

## Quick Start (One Command)

```bash
cd /home/joel/Sync/Projetos/Arcadestr/desktop && cargo tauri dev
```

**IMPORTANT:** Must be run from the `desktop/` directory, not the workspace root!

This will:
1. Build the web assets (WASM)
2. Build the desktop app
3. Start the Tauri dev server
4. Open the app window
5. Watch for changes and auto-reload

---

## Individual Build Commands

### Build Web (WASM) Only
```bash
cd /home/joel/Sync/Projetos/Arcadestr/web
trunk build
```

### Build Desktop Only
```bash
cd /home/joel/Sync/Projetos/Arcadestr
cargo build -p arcadestr-desktop
```

### Build Core Library
```bash
cd /home/joel/Sync/Projetos/Arcadestr
cargo build -p arcadestr-core
```

### Build Everything
```bash
cd /home/joel/Sync/Projetos/Arcadestr
cargo build --workspace
```

---

## Running the App

### Development Mode (Recommended)
```bash
cd /home/joel/Sync/Projetos/Arcadestr
cargo tauri dev
```

### Run Without Rebuilding
```bash
cd /home/joel/Sync/Projetos/Arcadestr
./target/debug/arcadestr-desktop
```

### Production Build
```bash
cd /home/joel/Sync/Projetos/Arcadestr
cargo tauri build
```

---

## Kill All Processes (Clean Restart)

### Quick Kill Script
```bash
#!/bin/bash
echo "Killing all Arcadestr processes..."
pkill -f "arcadestr-desktop" 2>/dev/null
pkill -f "cargo-tauri" 2>/dev/null
pkill -f "WebKitWebProcess" 2>/dev/null
pkill -f "WebKitNetworkProcess" 2>/dev/null
pkill -f "trunk" 2>/dev/null
sleep 2
echo "All processes killed"
```

Save this as `kill_arcadestr.sh` and run:
```bash
chmod +x kill_arcadestr.sh
./kill_arcadestr.sh
```

### Manual Kill Commands
```bash
# Kill desktop app
pkill -f "arcadestr-desktop"

# Kill Tauri CLI
pkill -f "cargo-tauri"

# Kill WebKit processes (browser engine)
pkill -f "WebKitWebProcess"
pkill -f "WebKitNetworkProcess"

# Kill trunk dev server
pkill -f "trunk"

# Wait for cleanup
sleep 2
```

---

## Clean Build (Nuclear Option)

If things are really broken:

```bash
cd /home/joel/Sync/Projetos/Arcadestr

# Kill everything
pkill -f "arcadestr-desktop"
pkill -f "cargo-tauri"
pkill -f "WebKit"
pkill -f "trunk"
sleep 2

# Clean build artifacts
cargo clean

# Remove trunk dist
rm -rf web/dist/*

# Rebuild everything
cargo tauri dev
```

---

## Check What's Running

```bash
# See Arcadestr processes
ps aux | grep -E "arcadestr|WebKit" | grep -v grep

# See if app is running
ps aux | grep "arcadestr-desktop" | grep -v grep

# See WebKit processes
ps aux | grep "WebKit" | grep -v grep

# Check ports
netstat -tlnp 2>/dev/null | grep -E "(5173|1420)" || ss -tlnp | grep -E "(5173|1420)"
```

---

## Testing Commands

### Run Tests
```bash
cd /home/joel/Sync/Projetos/Arcadestr

# Run all tests
cargo test

# Run core tests only
cargo test -p arcadestr-core

# Run specific test
cargo test -p arcadestr-core test_generate_nostrconnect
```

### Check Compilation (Without Running)
```bash
cd /home/joel/Sync/Projetos/Arcadestr
cargo check --workspace
```

---

## Troubleshooting Commands

### Check for Errors
```bash
# Build and show all errors
cargo build 2>&1 | grep -E "(error|warning:)"

# Check specific crate
cargo check -p arcadestr-app 2>&1 | grep -E "(error|warning:)"
```

### View Logs
```bash
# Run with output
cargo tauri dev 2>&1 | tee tauri.log

# Run desktop with output
./target/debug/arcadestr-desktop 2>&1 | tee app.log
```

### Check Dependencies
```bash
# Check if all deps are installed
cargo tree | head -20

# Check for outdated deps
cargo outdated  # requires: cargo install cargo-outdated
```

---

## File Locations

```
/home/joel/Sync/Projetos/Arcadestr/
├── core/           # Core library (NOSTR, Lightning, Auth)
│   └── src/
├── app/            # Leptos UI components
│   └── src/
├── desktop/        # Tauri desktop app
│   └── src/
├── web/            # WASM entry point
│   ├── src/
│   └── dist/       # Built web assets
└── target/         # Build artifacts
    └── debug/
```

---

## Common Issues & Fixes

### White Screen
```bash
# 1. Kill everything
pkill -f "arcadestr-desktop"
pkill -f "WebKit"
sleep 2

# 2. Rebuild web
cd web && trunk build

# 3. Run desktop
cd .. && cargo run -p arcadestr-desktop
```

### Port Already in Use
```bash
# Find and kill process on port 5173
lsof -ti:5173 | xargs kill -9 2>/dev/null
```

### WASM Build Fails
```bash
# Clean and rebuild
cd /home/joel/Sync/Projetos/Arcadestr/web
cargo clean
trunk build
```

### Tauri Config Error
```bash
# Regenerate Tauri config
cd /home/joel/Sync/Projetos/Arcadestr/desktop
cargo tauri init --force
```

---

## Environment Setup Check

```bash
# Verify Rust
cargo --version
rustc --version

# Verify Tauri CLI
cargo tauri --version

# Verify Trunk
trunk --version

# Verify WASM target
rustup target list --installed | grep wasm32

# Should show: wasm32-unknown-unknown (installed)
```

---

## Quick Reference Card

| Task | Command |
|------|---------|
| **Start dev server** | `cargo tauri dev` |
| **Build everything** | `cargo build --workspace` |
| **Run tests** | `cargo test` |
| **Kill all** | `pkill -f "arcadestr-desktop" && pkill -f "WebKit"` |
| **Clean build** | `cargo clean && rm -rf web/dist/*` |
| **Check errors** | `cargo check 2>&1 \| grep error` |
| **Rebuild web** | `cd web && trunk build` |
| **Run desktop only** | `cargo run -p arcadestr-desktop` |

---

## Your Workflow

**Normal development:**
```bash
cd /home/joel/Sync/Projetos/Arcadestr
cargo tauri dev
# Edit files, changes auto-reload
# Ctrl+C to stop
```

**If white screen appears:**
```bash
# Terminal 1: Kill everything
pkill -f "arcadestr-desktop" && pkill -f "WebKit" && sleep 2

# Terminal 2: Rebuild and run
cd /home/joel/Sync/Projetos/Arcadestr
cargo tauri dev
```

**If completely broken:**
```bash
cd /home/joel/Sync/Projetos/Arcadestr
pkill -f "arcadestr" && pkill -f "WebKit" && pkill -f "trunk"
sleep 2
cargo clean
rm -rf web/dist/*
cargo tauri dev
```

---

## Code Analysis Commands

### Analyze Patterns

Search the codebase for recurring patterns, similar implementations, and refactoring opportunities.

```bash
# List all available patterns
./scripts/analyze-patterns --list-patterns

# Analyze error handling patterns
./scripts/analyze-patterns --pattern=error-handling --language=rust

# Analyze async patterns with JSON output
./scripts/analyze-patterns --pattern=async-patterns --output=json

# Analyze mutex usage with markdown output
./scripts/analyze-patterns --pattern=mutex-patterns --output=markdown

# Custom pattern search
./scripts/analyze-patterns --pattern="Arc<" --depth=deep
```

**Available Patterns:**
- `error-handling` - Result, Error types, thiserror, anyhow
- `async-patterns` - async/await, tokio, futures
- `singleton` - lazy_static, once_cell, Arc<Mutex>
- `factory` - Factory pattern implementations
- `builder` - Builder pattern implementations
- `trait-patterns` - trait definitions and implementations
- `mutex-patterns` - Mutex, RwLock usage patterns
- `serialization` - serde, Serialize, Deserialize
- `logging` - tracing, log macros
- `testing` - Test modules and attributes
- `unsafe` - unsafe code blocks
- `todo` - TODO, FIXME, XXX comments
- `documentation` - Documentation comments

**Output Formats:**
- `text` (default) - Human-readable report
- `json` - Structured JSON data
- `markdown` - Formatted markdown for documentation

**Search Depth:**
- `shallow` - Current directory only
- `medium` - Source directories (core/src, desktop/src, app/src, web/src)
- `deep` - Entire repository

---

## ⚠️ IMPORTANT: Run from desktop/ directory!

The `cargo tauri dev` command **MUST** be run from the `desktop/` directory:

```bash
cd /home/joel/Sync/Projetos/Arcadestr/desktop
cargo tauri dev
```

NOT from the workspace root:

```bash
# WRONG - This will fail!
cd /home/joel/Sync/Projetos/Arcadestr
cargo tauri dev

# CORRECT - Run from desktop directory
cd /home/joel/Sync/Projetos/Arcadestr/desktop
cargo tauri dev
```

The `tauri.conf.json` file is located in `desktop/`, so Tauri CLI needs to be run from there.
