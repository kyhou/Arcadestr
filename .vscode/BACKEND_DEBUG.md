# VS Code Backend Debugging Guide for Arcadestr

This guide explains how to debug the Rust backend code using VS Code on Arch Linux.

## 🚀 Quick Start (Backend Debugging)

### 1. Required Setup

```bash
# Install dependencies (if not already done)
sudo pacman -S lldb rust rust-src

# VS Code extensions
code --install-extension vadimcn.vscode-lldb
code --install-extension rust-lang.rust-analyzer
```

### 2. Open Project

```bash
cd /home/joel/Sync/Projetos/Arcadestr
code .
```

### 3. Start Debugging

**Option A: Full Build and Debug**
1. Open `desktop/src/main.rs`
2. Go to line 227 (inside `connect_with_key` function)
3. Press `F9` to set breakpoint (red dot appears)
4. Press `F5` to start debugging
5. Select **"Debug Backend (Desktop App)"**
6. The app will build and launch
7. Use the UI to trigger the breakpoint

**Option B: Quick Debug (No Rebuild)**
- Use **"Debug Backend (Quick - No Rebuild)"** configuration
- Faster if you've already built recently
- Uses existing binary in `target/debug/`

## 🎯 Where to Set Breakpoints

### Backend Code (These will work ✅)

**File: `desktop/src/main.rs:227`**
```rust
async fn connect_with_key(key: String, state: tauri::State<'_, AppState>) -> Result<String, String> {
    // Set breakpoint here to see the nsec key input
    // Watch variable: 'key'
```

**File: `core/src/auth.rs:124`**
```rust
pub fn connect_with_key(&mut self, key: &str) -> Result<(), SignerError> {
    // Set breakpoint here to debug key parsing
    // Watch variable: 'key'
```

**File: `core/src/signer.rs:445`**
```rust
pub fn from_key(key: &str) -> Result<Self, SignerError> {
    // Set breakpoint here to debug DirectKeySigner creation
    // Watch variables: 'key', 'keys'
```

### Frontend Code (Won't work with this setup ❌)

**File: `app/src/lib.rs:1180`**
```rust
let on_connect_direct_key = move |_| {
    // This is Leptos frontend code - runs in WebView
    // Use console.log or println! instead of breakpoints
```

## 🐛 Debugging Your Specific Issue

### Debug the nsec Connection Flow

1. **Set breakpoint** in `desktop/src/main.rs:227` (connect_with_key command)
2. **Start debugging** with F5
3. **In the app:** Enter your nsec key in "Option 3: Test with Private Key"
4. **Click:** "Connect with Private Key"
5. **Debugger will pause** at your breakpoint
6. **Inspect:**
   - Variable `key` - The nsec string you entered
   - Variable `state` - AppState with auth and nostr clients

7. **Step into** (F11) the function calls:
   - Goes into `auth.connect_with_key(&key)` 
   - Then into `DirectKeySigner::from_key(key)`
   - You can see exactly where it fails

### Debug the Relay Connection Error

1. **Set breakpoint** in `core/src/nostr.rs:84` (NostrClient::new)
2. **Start debugging** with F5
3. **App launches** and pauses at relay initialization
4. **Inspect:**
   - Variable `relays` - Array of relay URLs
   - Step through the loop to see which relay causes the error

## ⌨️ Useful Shortcuts

| Key | Action |
|-----|--------|
| `F5` | Start debugging |
| `Shift+F5` | Stop debugging |
| `F9` | Toggle breakpoint |
| `F10` | Step over |
| `F11` | Step into |
| `Shift+F11` | Step out |
| `Ctrl+Shift+Y` | Show debug console |

## 📝 Debug Console Tips

In the Debug Console panel, you can evaluate expressions:

```rust
// View variable values
key
key.len()
key.starts_with("nsec1")

// Call methods
state.auth.lock().await.is_authenticated()
```

## 🔧 Troubleshooting

### "Breakpoint not resolved"

**Solution:**
1. Make sure you've built the project first:
   ```bash
   cargo build -p arcadestr-desktop
   ```
2. Reload VS Code window: `Ctrl+Shift+P` → "Developer: Reload Window"
3. Try again

### "No such file or directory" for binary

**Solution:**
Use the **"Debug Backend (Desktop App)"** configuration (not "Quick" version)
- This uses cargo to build and find the correct binary path

### "Cannot find bounds of current function"

**Solution:**
This happens in optimized code. Make sure you're:
- Building without `--release` flag
- Using the debug configuration (not release)

### Variables not showing values

**Solution:**
1. Make sure you've compiled with debug symbols (default for debug builds)
2. Try rebuilding: `cargo clean && cargo build -p arcadestr-desktop`
3. Check that you're stopped at a breakpoint, not just paused

## 🎓 Debug Configurations Explained

### "Debug Backend (Desktop App)"
- Builds fresh binary before debugging
- Uses cargo to locate binary automatically
- Best for first debug session or after code changes

### "Debug Backend (Quick - No Rebuild)"
- Uses existing binary in `target/debug/`
- Faster startup
- Use when you've already built and just want to debug again

### Test Configurations
- Debug Core Tests: Unit tests in `core/src/`
- Debug NostrConnect Tests: Integration tests
- Debug Specific Test: Debug a single test by name

## 📊 Environment Variables

These are automatically set during debugging:

```bash
RUST_LOG=debug,arcadestr_core=trace  # Maximum logging
RUST_BACKTRACE=1                      # Show backtraces
```

View output in the **Debug Console** panel.

## ✅ Success Checklist

After setting up, verify:
- [ ] Can set breakpoint in `desktop/src/main.rs`
- [ ] Debugger stops at breakpoint
- [ ] Can see variable values
- [ ] Can step into `core/src/` functions
- [ ] Debug console shows RUST_LOG output

## 🆘 Still Having Issues?

1. **Check extensions are installed:**
   - Open Extensions panel (Ctrl+Shift+X)
   - Verify: rust-analyzer, CodeLLDB

2. **Verify lldb works:**
   ```bash
   lldb --version
   ```

3. **Try command line debugging:**
   ```bash
   cargo build -p arcadestr-desktop
   lldb target/debug/arcadestr-desktop
   (lldb) breakpoint set --file main.rs --line 227
   (lldb) run
   ```

4. **Check VS Code settings:**
   - `Ctrl+,` to open settings
   - Search "rust-analyzer"
   - Ensure it's enabled

## 🎯 Next Steps

Once backend debugging works, you can:
1. Debug the connection flow step-by-step
2. Inspect variable values at each step
3. Identify exactly where the error occurs
4. Fix the issue with full context

**Ready to debug!** Open VS Code and press F5. 🎉
