# Nostrconnect URI Generation - Implementation & Testing

## Summary

I've implemented the `nostrconnect://` URI generation feature and created comprehensive tests for it. The issue with the button hanging was due to the Tauri command returning a `serde_json::Value` which wasn't being handled correctly in the WASM bridge. I fixed this by changing the return type to `String`.

## Changes Made

### 1. Core Library (`core/src/signer.rs`)
- Added `generate_nostrconnect_uri()` method to `Nip46Signer`
- Generates proper nostrconnect:// URIs with:
  - Client pubkey (hex)
  - URL-encoded relay URL
  - URL-encoded secret
  - Optional permissions
  - Optional app name
- Added 5 unit tests

### 2. Desktop App (`desktop/src/main.rs`)
- Added `generate_nostrconnect_uri` Tauri command
- Changed return type from `serde_json::Value` to `String` to fix WASM bridge issue
- Returns JSON string with: `uri`, `client_pubkey`, `relay`

### 3. UI (`app/src/lib.rs`)
- Added "Option 1: Generate Connection URI" section
- Added "Generate nostrconnect:// URI" button
- Added text area to display generated URI
- Added "Copy to Clipboard" button
- Added proper error handling

### 4. Tests
- **Unit tests** (`core/src/signer.rs`): 5 tests covering basic generation, permissions, name, URL encoding, and uniqueness
- **Integration tests** (`core/tests/nostrconnect_tests.rs`): 3 tests covering full URI generation, parsing, and uniqueness
- **Test script** (`test_nostrconnect.sh`): Automated test runner

## Test Results

```
✓ All tests passed!
- 5 unit tests
- 3 integration tests
- Core library builds successfully
- Desktop app builds successfully
```

## How to Test Manually

### 1. Run the tests:
```bash
cd /home/joel/Sync/Projetos/Arcadestr
./test_nostrconnect.sh
```

### 2. Run the desktop app:
```bash
cargo tauri dev
```

### 3. Test the feature:
1. You should see the login screen with two options:
   - **Option 1: Generate Connection URI** (NEW)
   - **Option 2: Paste Signer URI** (existing)

2. Click **"Generate nostrconnect:// URI"**

3. A text area should appear with a URI like:
   ```
   nostrconnect://<64-char-hex-pubkey>?relay=wss%3A%2F%2Frelay.damus.io&secret=<random-secret>&perms=sign_event%3A1%2Csign_event%3A30078&name=Arcadestr
   ```

4. Click **"Copy to Clipboard"** to copy the URI

5. Open your signer app (Nsec.app, Amber, etc.)

6. Paste the URI into the signer app

7. The signer should connect back to Arcadestr

## Technical Details

### URI Format
```
nostrconnect://<client-pubkey-hex>?relay=<encoded>&secret=<encoded>&perms=<encoded>&name=<encoded>
```

### Example
```
nostrconnect://a1b2c3d4...?relay=wss%3A%2F%2Frelay.damus.io&secret=AbCdEf123456&perms=sign_event%3A1%2Csign_event%3A30078&name=Arcadestr
```

### Permissions Requested
- `sign_event:1` - For signing kind 1 events (notes)
- `sign_event:30078` - For signing game listings

## Troubleshooting

If the button still hangs:
1. Check the browser console for errors
2. Verify the Tauri command is registered in `desktop/src/main.rs`
3. Check that the WASM bridge is handling the response correctly
4. Look for errors in the terminal running `cargo tauri dev`

## Files Modified

1. `core/src/signer.rs` - Added URI generation and unit tests
2. `core/tests/nostrconnect_tests.rs` - Integration tests
3. `desktop/src/main.rs` - Added Tauri command
4. `app/src/lib.rs` - Added UI components
5. `desktop/Cargo.toml` - Added `rand` dependency
6. `test_nostrconnect.sh` - Test runner script
