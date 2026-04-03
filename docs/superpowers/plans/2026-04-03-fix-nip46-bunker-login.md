# Fix NIP-46 Bunker Login Flow Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fix the NIP-46 bunker:// login flow to properly complete authentication before redirecting and display the correct user npub

**Architecture:** Replace the async "fast" connection flow with a synchronous blocking handshake that waits for user approval before returning. The bunker URI parsing needs to not extract a fake user pubkey, but wait for the actual handshake to return the real user pubkey.

**Tech Stack:** Rust, Tauri v2, Leptos 0.8, nostr-sdk 0.44

---

## Problem Analysis

### Current Broken Flow:
1. User enters bunker:// URI
2. `parse_bunker_uri()` extracts the bunker's pubkey (wrong! this is not the user pubkey)
3. `connect_bunker()` calls `init_signer_session_fast()` which returns immediately
4. Frontend immediately sets `auth.npub` → view switches to MainView
5. Background connection attempts to connect but user never sees approval prompt
6. Wrong npub is displayed (it's the bunker's pubkey, not the user's)

### Required Fixed Flow:
1. User enters bunker:// URI  
2. `connect_bunker()` performs blocking NIP-46 handshake
3. User approves connection in Amber/nsec.app
4. Backend receives actual user pubkey from handshake
5. Profile saved with correct user pubkey
6. Backend returns success with correct npub
7. Frontend sets `auth.npub` → view switches to MainView

---

## File Structure

**Files to Modify:**
- `desktop/src/nip46_commands.rs:20-141` - `connect_bunker()` command - switch to blocking handshake
- `desktop/src/nip46_commands.rs:143-163` - `parse_bunker_uri()` - remove wrong pubkey extraction
- `core/src/nip46/auth.rs:77-161` - `init_signer_session()` - ensure it works for bunker:// URIs
- `app/src/lib.rs:3666-3692` - `on_connect_bunker` - add proper loading state during handshake

---

## Task 1: Fix parse_bunker_uri to not return fake user pubkey

**Files:**
- Modify: `desktop/src/nip46_commands.rs:143-163`

**Context:** The current `parse_bunker_uri()` extracts the bunker's pubkey and returns it as the user_pubkey. This is wrong - the bunker's pubkey is the signer's pubkey, not the user's pubkey. The user pubkey should come from the NIP-46 handshake via `get_public_key()`.

- [ ] **Step 1: Read the current parse_bunker_uri function**

Read file: `desktop/src/nip46_commands.rs` lines 143-163 to understand current implementation.

- [ ] **Step 2: Modify parse_bunker_uri to return Option for user_pubkey**

Change the function signature and implementation to indicate that user_pubkey is not available from bunker URI parsing:

```rust
/// Parse a bunker:// URI and extract the NostrConnectURI
/// Note: The user_pubkey is NOT available from the bunker URI itself - it must be
/// obtained from the NIP-46 handshake via get_public_key().
fn parse_bunker_uri(
    uri_str: &str,
) -> Result<NostrConnectURI, String> {
    use nostr::nips::nip46::NostrConnectURI;

    let uri = NostrConnectURI::parse(uri_str)
        .map_err(|e| format!("Failed to parse bunker URI: {}", e))?;

    // For bunker URIs, we cannot determine the user pubkey from the URI alone.
    // The user pubkey will be obtained during the NIP-46 handshake.
    Ok(uri)
}
```

- [ ] **Step 3: Update the caller in connect_bunker to handle the new return type**

The caller around line 48 needs to be updated from:
```rust
// Parse bunker URI directly
match parse_bunker_uri(&identifier) {
    Ok((uri, pubkey)) => (uri, pubkey),
    Err(e) => {
        error!("Failed to parse bunker URI: {}", e);
        return Err(format!("Invalid bunker URI: {}", e));
    }
}
```

To just get the URI:
```rust
// Parse bunker URI directly
let bunker_uri = parse_bunker_uri(&identifier)
    .map_err(|e| {
        error!("Failed to parse bunker URI: {}", e);
        format!("Invalid bunker URI: {}", e)
    })?;
```

- [ ] **Step 4: Test compilation**

Run: `cargo check -p arcadestr-desktop`
Expected: Compilation errors in connect_bunker (expected - we need to fix that next)

---

## Task 2: Rewrite connect_bunker to use blocking handshake

**Files:**
- Modify: `desktop/src/nip46_commands.rs:20-141`
- Uses: `core/src/nip46/auth.rs:77-161` (`init_signer_session`)

**Context:** The current implementation uses `init_signer_session_fast()` which creates a LazyNip46Signer and returns immediately. The connection happens in the background. This prevents users from approving the connection in their signer app because the UI has already moved on.

We need to use `init_signer_session()` which performs the blocking NIP-46 handshake, waiting for the user to approve in their signer app before returning.

- [ ] **Step 1: Read the full connect_bunker function**

Read file: `desktop/src/nip46_commands.rs` lines 20-141 to understand current implementation.

- [ ] **Step 2: Rewrite connect_bunker with blocking handshake**

Replace the entire function with this implementation:

```rust
/// Called by Leptos when the user submits a bunker URI or NIP-05 address.
/// Flow A entry point - BLOCKING VERSION (waits for handshake).
/// On success: saves profile to keyring and returns the user npub.
/// 
/// IMPORTANT: This function BLOCKS until the user approves the connection
/// in their signer app (Amber, nsec.app, etc.). The frontend will show
/// "Connecting..." during this time.
#[tauri::command]
pub async fn connect_bunker(
    identifier: String,
    display_name: String,
    state: State<'_, Arc<Mutex<AppSignerState>>>,
    app_handle: AppHandle,
) -> Result<serde_json::Value, String> {
    info!(
        "connect_bunker called (blocking handshake): identifier={}, display_name={}",
        identifier, display_name
    );

    // STEP 1: Parse the identifier to get bunker_uri and optional user_pubkey hint
    let (bunker_uri, _user_pubkey_hint) = if identifier.contains('@') {
        // NIP-05 identifier - resolve to get bunker URI
        match resolve_nip05_to_uri_and_pubkey(&identifier).await {
            Ok((uri, pubkey)) => (uri, Some(pubkey)),
            Err(e) => {
                error!("Failed to resolve NIP-05: {}", e);
                return Err(format!("Failed to resolve NIP-05: {}", e));
            }
        }
    } else {
        // Parse bunker URI directly
        match parse_bunker_uri(&identifier) {
            Ok(uri) => (uri, None),
            Err(e) => {
                error!("Failed to parse bunker URI: {}", e);
                return Err(format!("Invalid bunker URI: {}", e));
            }
        }
    };

    // STEP 2: Set up auth URL handler for bunkers that need browser approval (e.g., nsec.app)
    let auth_url_handler = |auth_url: Url| {
        info!("Auth URL received from bunker: {}", auth_url);
        // Emit event to frontend to open browser
        let _ = app_handle.emit("bunker-auth-challenge", auth_url.to_string());
    };

    // STEP 3: Perform BLOCKING NIP-46 handshake
    // This waits for the user to approve the connection in their signer app
    info!("Starting blocking NIP-46 handshake...");
    let bunker_uri_string = bunker_uri.to_string();
    let (mut profile, client) = init_signer_session(
        &bunker_uri_string,
        Some(auth_url_handler),
    )
    .await
    .map_err(|e| {
        error!("NIP-46 handshake failed: {}", e);
        format!("Failed to connect to bunker: {}", e)
    })?;

    info!(
        "NIP-46 handshake successful! user_pubkey={}",
        profile.user_pubkey.to_hex()
    );

    // STEP 4: Allow the user to override the auto-generated name
    if !display_name.is_empty() {
        profile.name = display_name.clone();
    }

    // STEP 5: Save to keyring
    save_profile_to_keyring(&profile).map_err(|e| {
        error!("save_profile_to_keyring failed: {}", e);
        e.to_string()
    })?;

    info!("Profile saved to keyring: id={}", profile.id);

    // STEP 6: Get bunker pubkey for state management
    let bunker_pubkey = profile
        .bunker_uri
        .remote_signer_public_key()
        .ok_or("No remote signer public key in URI")?
        .to_hex();

    // STEP 7: Update state with the client
    {
        let mut state_guard = state.lock().await;
        state_guard.active_client = Some(client.clone());
        state_guard.active_profile_id = Some(bunker_pubkey.clone());
        state_guard.connection_state = ConnectionState::Connected; // Already connected!
    }

    // STEP 8: Set as last active profile for auto-restore on next startup
    if let Err(e) = set_last_active_profile_id(&profile.id) {
        warn!("Failed to set last active profile ID: {}", e);
    }

    // STEP 9: Return success with correct user npub
    let user_npub = profile
        .user_pubkey
        .to_bech32()
        .map_err(|e| format!("Failed to encode pubkey: {}", e))?;

    info!("Authentication complete! user_npub={}", user_npub);

    // Return profile info with connected state
    Ok(serde_json::json!({
        "id": profile.id,
        "name": profile.name,
        "pubkey": user_npub,
        "pubkey_hex": profile.user_pubkey.to_hex(),
        "connection_state": "connected",
    }))
}
```

- [ ] **Step 3: Add required import for Url type**

Add at the top of the file:
```rust
use url::Url;
```

- [ ] **Step 4: Test compilation**

Run: `cargo check -p arcadestr-desktop`
Expected: Should compile successfully

- [ ] **Step 5: Test full workspace compilation**

Run: `cargo check --workspace`
Expected: All crates should compile

---

## Task 3: Update frontend loading state

**Files:**
- Modify: `app/src/lib.rs:3666-3692`

**Context:** The frontend currently shows "Connecting..." briefly but doesn't handle long-duration connections well. Since the handshake can take 30-60 seconds (waiting for user approval), we should ensure the UI properly shows the loading state and doesn't allow multiple attempts.

- [ ] **Step 1: Read current on_connect_bunker handler**

Read file: `app/src/lib.rs` around lines 3649-3693 to understand current implementation.

- [ ] **Step 2: Enhance loading state and error handling**

The current implementation is mostly correct, but let's ensure it properly handles the longer connection time. Update the `on_connect_bunker` callback:

```rust
// Handle bunker:// connect button click (updated for blocking NIP-46 API)
// Now uses connect_bunker which waits for user approval in signer app
let on_connect_bunker = move |_| {
    let auth = auth_stored.get_value();
    let uri_val = bunker_uri.get();
    let display_name_val = bunker_display_name.get();

    if uri_val.is_empty() {
        auth.error.set(Some(
            "Please enter a bunker:// URI or NIP-05 identifier".to_string(),
        ));
        return;
    }

    // Check if already connecting
    if auth.is_loading.get() {
        return; // Prevent multiple simultaneous attempts
    }

    auth.is_loading.set(true);
    auth.error.set(None);

    spawn_local(async move {
        // Use the connect_bunker API which performs blocking handshake
        // This can take 30-60 seconds while waiting for user approval
        match invoke_connect_bunker(uri_val, display_name_val).await {
            Ok(result) => {
                // Extract npub from result
                if let Some(npub) = result.get("pubkey").and_then(|v| v.as_str()) {
                    // Reload profiles list to show the new profile
                    let _ = auth.load_profiles_list().await;
                    auth.npub.set(Some(npub.to_string()));
                    auth.has_secure_accounts.set(true);
                    auth.is_loading.set(false);

                    // Start connection status polling for NIP-46 accounts
                    auth.start_connection_status_polling().await;
                    
                    // Clear the input field
                    bunker_uri.set(String::new());
                    bunker_display_name.set(String::new());
                } else {
                    auth.error.set(Some(
                        "Connected but failed to get pubkey from response".to_string(),
                    ));
                    auth.is_loading.set(false);
                }
            }
            Err(e) => {
                auth.error.set(Some(format!("Failed to connect: {}", e)));
                auth.is_loading.set(false);
            }
        }
    });
};
```

- [ ] **Step 3: Add better loading message**

Update the button text to be more descriptive about what's happening:

```rust
<button on:click=on_connect_bunker disabled=move || auth_stored.get_value().is_loading.get()>
    {move || if auth_stored.get_value().is_loading.get() { 
        "Waiting for signer approval..." 
    } else { 
        "Connect with Bunker" 
    }}
</button>
```

- [ ] **Step 4: Add helpful text explaining the flow**

Add explanatory text below the button:

```rust
<p class="bunker-hint">
    "After clicking connect, approve the connection in your signer app (Amber, nsec.app, etc.)"
</p>
```

- [ ] **Step 5: Test compilation**

Run: `cargo check -p arcadestr-app`
Expected: Should compile successfully

---

## Task 4: Test the complete flow

**Files:**
- All modified files
- Test with actual signer app

- [ ] **Step 1: Build the desktop app**

Run: `cd /home/joel/Sync/Projetos/Arcadestr/desktop && cargo build`
Expected: Successful build

- [ ] **Step 2: Run the desktop app and test bunker login**

Run: `cd /home/joel/Sync/Projetos/Arcadestr/desktop && timeout 60 cargo tauri dev 2>&1`

Test steps:
1. Open the app
2. Go to "Add Account"
3. Select "Remote Signer (NIP-46)"
4. Enter a valid bunker:// URI (e.g., from nsec.app or Amber)
5. Click "Connect with Bunker"
6. **Expected**: See "Waiting for signer approval..." message
7. Approve the connection in your signer app
8. **Expected**: After approval, redirected to MainView with CORRECT npub displayed

- [ ] **Step 3: Verify correct npub is shown**

Check that the npub shown in the UI matches your actual account, not the bunker's pubkey.

- [ ] **Step 4: Test error handling**

Test with an invalid bunker URI:
1. Enter an invalid bunker:// URI
2. Click connect
3. **Expected**: Error message shown, remains on login page

- [ ] **Step 5: Test cancellation**

1. Start a connection
2. Cancel/deny in your signer app
3. **Expected**: Error message shown after timeout, remains on login page

---

## Verification Checklist

After all tasks are complete, verify:

- [ ] Bunker login waits for user approval before redirecting
- [ ] Correct user npub is displayed (not the bunker's pubkey)
- [ ] Error handling works for denied/cancelled connections
- [ ] UI shows appropriate "waiting" state during handshake
- [ ] All existing functionality still works (nsec login, QR login, etc.)
- [ ] No compilation warnings or errors

---

## Summary of Changes

**Key architectural changes:**
1. `parse_bunker_uri()` no longer returns a fake user_pubkey - it returns just the URI
2. `connect_bunker()` now uses blocking `init_signer_session()` instead of `init_signer_session_fast()`
3. The NIP-46 handshake completes BEFORE returning to frontend
4. Frontend shows "Waiting for signer approval..." during the handshake
5. User sees the correct npub (from the handshake, not the bunker's pubkey)

**Files changed:**
- `desktop/src/nip46_commands.rs` - Fixed bunker flow to use blocking handshake
- `app/src/lib.rs` - Enhanced loading states and user messaging
