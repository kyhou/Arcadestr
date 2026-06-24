# VS Code Debugging Guide for Arcadestr

This guide explains how to debug the Arcadestr application using VS Code on Arch Linux.

## 🚀 Quick Start

### 1. Install Required Dependencies

```bash
# System packages
sudo pacman -S lldb rust rust-src

# VS Code extensions
code --install-extension vadimcn.vscode-lldb
code --install-extension rust-lang.rust-analyzer
code --install-extension serayuzgur.crates
code --install-extension tamasfe.even-better-toml
```

### 2. Open Project in VS Code

```bash
cd /home/joel/Sync/Projetos/Arcadestr
code .
```

### 3. Build the Project First

Before debugging, build the project at least once:

```bash
# Option A: Use VS Code task (Ctrl+Shift+P → "Tasks: Run Build Task")
# Option B: Command line
cargo build -p arcadestr-desktop
```

## 🐛 Debugging Workflows

### Debug the Desktop App

1. Open `core/src/nostr.rs`
2. Set a breakpoint at line 84 (NostrClient::new)
3. Press `F5` or go to Run → Start Debugging
4. Select "Debug Desktop App"
5. The app will launch with debugger attached
6. Execution will pause at your breakpoint

### Debug Core Tests

1. Open `core/tests/nostrconnect_tests.rs`
2. Set a breakpoint in the test function
3. Press `F5`
4. Select "Debug Core Tests"

### Debug a Specific Test

1. Press `F5`
2. Select "Debug Specific Test"
3. Enter the test name (e.g., `test_generate_nostrconnect_uri_basic`)

## 🎯 Common Debug Scenarios

### Scenario 1: Relay Connection Error

**File**: `core/src/nostr.rs`  
**Line**: 84 (in `NostrClient::new`)  
**What to watch**: The `relays` parameter and relay connection results

```rust
// Set breakpoint here:
pub async fn new(relays: Vec<String>) -> Result<Self, NostrError> {
```

### Scenario 2: NIP-46 Connection Issue

**File**: `core/src/signer.rs`  
**Line**: 139 (in `wait_for_nostrconnect_signer`)  
**What to watch**: `nostrconnect_uri` construction and parsing

```rust
// Set breakpoint here:
let nostrconnect_uri = format!(
    "nostrconnect://{}?relay={}&secret={}&metadata={}",
    ...
);
```

### Scenario 3: Direct Key Authentication

**File**: `core/src/auth.rs`  
**Line**: 124 (in `connect_with_key`)  
**What to watch**: Key parsing and signer creation

```rust
// Set breakpoint here:
let signer = match DirectKeySigner::from_key(key) {
```

## 📊 Environment Variables

The debug configurations set these environment variables:

- `RUST_LOG=debug,arcadestr_core=trace` - Maximum logging
- `RUST_BACKTRACE=1` - Show backtraces on errors

## 🔧 Troubleshooting

### "Unable to find executable"

Build the project first:
```bash
cargo build -p arcadestr-desktop
```

### "LLDB exited with code 1"

Make sure lldb is installed:
```bash
sudo pacman -S lldb
```

### Breakpoints not hitting

1. Make sure you've built the project in debug mode (not `--release`)
2. Try running "Clean Build" task first, then rebuild
3. Check that the source file paths match exactly

## 📁 Debug Configurations

All debug configurations are in `.vscode/launch.json`:

1. **Debug Desktop App** - Debug the full Tauri application
2. **Debug Desktop App (Release)** - Debug optimized build
3. **Debug Core Tests** - Debug unit tests in core library
4. **Debug NostrConnect Tests** - Debug integration tests
5. **Debug Specific Test** - Debug a single test by name

## ⌨️ Useful Shortcuts

- `F5` - Start debugging
- `Shift+F5` - Stop debugging
- `F9` - Toggle breakpoint
- `F10` - Step over
- `F11` - Step into
- `Shift+F11` - Step out
- `Ctrl+Shift+Y` - Show debug console

## 📝 Debug Console Commands

In the Debug Console panel, you can evaluate Rust expressions:

```rust
// View variable values
uri
secret.len()
client_pubkey.to_hex()

// Call methods
signer.keys().public_key()
```

## 🎓 Tips

1. **Use the Variables panel** to inspect structs and enums
2. **Set conditional breakpoints** by right-clicking a breakpoint
3. **Use Watch expressions** for values you want to monitor
4. **Check the Call Stack** to trace execution flow
5. **Use the Debug Console** to evaluate expressions during debugging

## 🆘 Getting Help

If you encounter issues:
1. Check the Debug Console for error messages
2. Verify `RUST_LOG` output in the terminal
3. Try building from command line: `cargo build -p arcadestr-desktop`
4. Check that all extensions are installed and enabled
