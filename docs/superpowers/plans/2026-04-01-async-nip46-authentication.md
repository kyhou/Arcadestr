# Async NIP-46 Authentication Optimization - Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Optimize Arcadestr's NIP-46 authentication to return immediately after user approval (like Yakihonne), deferring the full handshake until first signing request.

**Architecture:** Convert from blocking synchronous handshake to async deferred connection pattern. Store user keys immediately, establish WebSocket connections in background, perform NIP-46 handshake lazily on first sign request.

**Tech Stack:** Rust, nostr-sdk, nostr-connect crate, Tauri, Leptos

---

## Problem Analysis

**Current Flow (Slow):**
1. User pastes bunker URI → `init_signer_session()` called
2. Creates NostrConnect signer with 60s timeout
3. **BLOCKS** on `get_public_key()` - waits for full NIP-46 handshake
4. Returns success only after complete handshake

**Target Flow (Fast - like Yakihonne):**
1. User pastes bunker URI → `init_signer_session()` called
2. Creates NostrConnect signer
3. **Returns immediately** with user pubkey from stored profile
4. Handshake happens asynchronously on first signing request

## Files to Modify

1. `core/src/nip46/auth.rs` - Modify `init_signer_session()` to skip blocking handshake
2. `core/src/nip46/session.rs` - Add deferred handshake logic to `activate_profile()`
3. `desktop/src/nip46_commands.rs` - Update `connect_bunker()` to return immediately
4. `core/src/nip46/types.rs` - Add connection state enum
5. `core/src/signers/nip46.rs` - Add lazy connection wrapper for Nip46Signer
6. `app/src/lib.rs` - Update UI to show connection status

---

## Task 1: Add Connection State Tracking

**Files:**
- Modify: `core/src/nip46/types.rs`

**Purpose:** Track whether the NIP-46 connection is pending, connected, or failed.

- [ ] **Step 1: Add ConnectionState enum**

```rust
/// Connection state for NIP-46 signer
#[derive(Debug, Clone, PartialEq)]
pub enum ConnectionState {
    /// Initial state, no connection attempted
    Disconnected,
    /// Connection in progress (async handshake happening)
    Connecting,
    /// Successfully connected and ready
    Connected,
    /// Connection failed
    Failed(String),
}
```

- [ ] **Step 2: Add connection_state field to AppSignerState**

Add to `AppSignerState` struct (around line 37):
```rust
/// Current connection state for the active signer
pub connection_state: ConnectionState,
```

Update `AppSignerState::new()` to initialize:
```rust
connection_state: ConnectionState::Disconnected,
```

- [ ] **Step 3: Verify compilation**

Run: `cargo check -p arcadestr-core`
Expected: No errors

- [ ] **Step 4: Commit**

```bash
git add core/src/nip46/types.rs
git commit -m "feat: add ConnectionState enum for tracking NIP-46 connection status"
```

---

## Task 2: Create LazyNip46Signer Wrapper

**Files:**
- Create: `core/src/signers/lazy_nip46.rs`
- Modify: `core/src/signers/mod.rs`
- Modify: `core/src/signers/nip46.rs` (add re-export if needed)

**Purpose:** Wrapper that defers the NIP-46 handshake until first signing request.

- [ ] **Step 1: Create lazy_nip46.rs file**

```rust
//! Lazy NIP-46 Signer
//! 
//! This wrapper defers the NIP-46 handshake until the first signing request.
//! It stores the connection parameters and establishes the connection lazily.

use std::sync::Arc;
use tokio::sync::{Mutex, RwLock};
use nostr::{PublicKey, Event, Keys};
use nostr::nips::nip46::NostrConnectURI;
use nostr::signer::{NostrSigner, SignerError};
use tracing::{info, error, warn};

use crate::nip46::types::ConnectionState;

/// Lazy wrapper around NIP-46 signer that defers handshake
#[derive(Clone)]
pub struct LazyNip46Signer {
    /// The bunker URI for connection
    bunker_uri: NostrConnectURI,
    /// Ephemeral app keys for this session
    app_keys: Keys,
    /// Inner signer (initialized lazily)
    inner: Arc<RwLock<Option<Arc<dyn NostrSigner>>>>,
    /// Connection state
    state: Arc<RwLock<ConnectionState>>,
    /// User's public key (known from initial connection)
    user_pubkey: PublicKey,
}

impl LazyNip46Signer {
    /// Create a new lazy signer without establishing connection
    pub fn new(
        bunker_uri: NostrConnectURI,
        app_keys: Keys,
        user_pubkey: PublicKey,
    ) -> Self {
        info!("Creating LazyNip46Signer (deferred connection)");
        Self {
            bunker_uri,
            app_keys,
            inner: Arc::new(RwLock::new(None)),
            state: Arc::new(RwLock::new(ConnectionState::Disconnected)),
            user_pubkey,
        }
    }

    /// Get current connection state
    pub async fn connection_state(&self) -> ConnectionState {
        self.state.read().await.clone()
    }

    /// Ensure connection is established (called before signing)
    async fn ensure_connected(&self) -> Result<Arc<dyn NostrSigner>, SignerError> {
        // Check if already connected
        {
            let inner = self.inner.read().await;
            if let Some(signer) = inner.clone() {
                return Ok(signer);
            }
        }

        // Need to connect
        let mut state = self.state.write().await;
        
        // Double-check after acquiring write lock
        {
            let inner = self.inner.read().await;
            if let Some(signer) = inner.clone() {
                return Ok(signer);
            }
        }

        // Set connecting state
        *state = ConnectionState::Connecting;
        drop(state);

        info!("Establishing deferred NIP-46 connection...");

        // Create the actual signer and perform handshake
        match self.connect_and_handshake().await {
            Ok(signer) => {
                let mut inner = self.inner.write().await;
                *inner = Some(signer.clone());
                
                let mut state = self.state.write().await;
                *state = ConnectionState::Connected;
                
                info!("NIP-46 connection established successfully");
                Ok(signer)
            }
            Err(e) => {
                let mut state = self.state.write().await;
                *state = ConnectionState::Failed(e.to_string());
                
                error!("Failed to establish NIP-46 connection: {}", e);
                Err(e)
            }
        }
    }

    /// Perform the actual connection and handshake
    async fn connect_and_handshake(&self) -> Result<Arc<dyn NostrSigner>, SignerError> {
        use nostr_connect::client::NostrConnect;
        use std::time::Duration;

        let signer = NostrConnect::new(
            self.bunker_uri.clone(),
            self.app_keys.clone(),
            Duration::from_secs(30), // Shorter timeout for deferred connection
            None,
        ).map_err(|e| SignerError::Nip46Error(format!("Failed to create signer: {}", e)))?;

        // Perform handshake
        signer.get_public_key().await
            .map_err(|e| SignerError::Nip46Error(format!("Handshake failed: {}", e)))?;

        Ok(Arc::new(signer) as Arc<dyn NostrSigner>)
    }
}

#[async_trait::async_trait]
impl NostrSigner for LazyNip46Signer {
    async fn get_public_key(&self) -> Result<PublicKey, SignerError> {
        // Return the known public key immediately
        Ok(self.user_pubkey)
    }

    async fn sign_event(&self, event: Event) -> Result<Event, SignerError> {
        // Ensure connection before signing
        let signer = self.ensure_connected().await?;
        signer.sign_event(event).await
    }
}

impl std::fmt::Debug for LazyNip46Signer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LazyNip46Signer")
            .field("user_pubkey", &self.user_pubkey)
            .field("state", &"<async>")
            .finish()
    }
}
```

- [ ] **Step 2: Add async_trait dependency if needed**

Check `core/Cargo.toml` for `async-trait` dependency. If not present, add:
```toml
async-trait = "0.1"
```

- [ ] **Step 3: Update mod.rs to include lazy_nip46**

Add to `core/src/signers/mod.rs`:
```rust
#[cfg(not(target_arch = "wasm32"))]
pub mod lazy_nip46;

#[cfg(not(target_arch = "wasm32"))]
pub use lazy_nip46::LazyNip46Signer;
```

- [ ] **Step 4: Verify compilation**

Run: `cargo check -p arcadestr-core`
Expected: No errors

- [ ] **Step 5: Commit**

```bash
git add core/src/signers/lazy_nip46.rs core/src/signers/mod.rs
git commit -m "feat: add LazyNip46Signer for deferred connection"
```

---

## Task 3: Modify init_signer_session for Async Flow

**Files:**
- Modify: `core/src/nip46/auth.rs`

**Purpose:** Change `init_signer_session()` to return immediately without blocking on handshake.

- [ ] **Step 1: Update imports**

Add to imports in `core/src/nip46/auth.rs`:
```rust
use crate::signers::LazyNip46Signer;
```

- [ ] **Step 2: Modify init_signer_session function**

Replace the function (around line 55-145) with:

```rust
/// Entry point for Flow A: user provides either a "bunker://..." URI
/// or a NIP-05 identifier like "bob@nsec.app".
///
/// Steps performed:
///   1. Resolve identifier → NostrConnectURI
///   2. Generate ephemeral Keys for this session
///   3. Create LazyNip46Signer (deferred connection)
///   4. Build nostr-sdk Client with lazy signer
///   5. Return immediately with SavedProfile (handshake happens on first sign)
///
/// # Arguments
/// * `identifier` - bunker:// URI or NIP-05 identifier (user@domain)
/// * `user_pubkey` - The user's public key (from initial connection or known)
///
/// # Returns
/// Tuple of (SavedProfile, Client) containing the connection details and active client
pub async fn init_signer_session_fast(
    identifier: &str,
    user_pubkey: PublicKey,
) -> anyhow::Result<(SavedProfile, Client)> {
    info!("init_signer_session_fast called with identifier: {}", identifier);

    // STEP 1 — URI Resolution (NIP-05 discovery or direct parse)
    let uri = if identifier.contains('@') {
        info!("Resolving NIP-05 identifier: {}", identifier);
        resolve_nip05_to_uri(identifier).await?
    } else {
        info!("Parsing bunker URI directly");
        NostrConnectURI::parse(identifier)?
    };

    // STEP 2 — Ephemeral key generation
    let app_keys = Keys::generate();
    info!("Generated ephemeral app_keys: pubkey={}", app_keys.public_key().to_hex());

    // STEP 3 — Create LazyNip46Signer (deferred connection)
    info!("Creating LazyNip46Signer with deferred connection...");
    let lazy_signer = LazyNip46Signer::new(
        uri.clone(),
        app_keys.clone(),
        user_pubkey,
    );

    // STEP 4 — Build nostr-sdk Client with lazy signer
    info!("Building nostr-sdk Client with LazyNip46Signer...");
    let client = Client::new(Arc::new(lazy_signer) as Arc<dyn nostr::NostrSigner>);
    info!("Client built successfully (deferred connection)");

    // STEP 5 — Generate profile ID and return
    let profile_id = uuid::Uuid::new_v4().to_string();

    let profile = SavedProfile {
        id: profile_id,
        name: identifier.to_string(),
        user_pubkey,
        bunker_uri: uri,
        app_keys,
    };

    Ok((profile, client))
}
```

- [ ] **Step 3: Keep old function for backward compatibility**

Rename old function to `init_signer_session_blocking` and keep it available:

```rust
/// Legacy blocking version - kept for compatibility
/// Use init_signer_session_fast for new code
pub async fn init_signer_session<F>(
    identifier: &str,
    on_auth: Option<F>,
) -> anyhow::Result<(SavedProfile, Client)>
where
    F: Fn(Url) + Send + Sync + 'static,
{
    // ... existing implementation ...
}
```

- [ ] **Step 4: Verify compilation**

Run: `cargo check -p arcadestr-core`
Expected: No errors

- [ ] **Step 5: Commit**

```bash
git add core/src/nip46/auth.rs
git commit -m "feat: add init_signer_session_fast for async NIP-46 connection"
```

---

## Task 4: Update connect_bunker Command

**Files:**
- Modify: `desktop/src/nip46_commands.rs`

**Purpose:** Update the Tauri command to use the fast async flow.

- [ ] **Step 1: Modify connect_bunker function**

Replace the function (around line 38-109) with:

```rust
/// Called by Leptos when the user submits a bunker URI or NIP-05 address.
/// Flow A entry point - FAST VERSION (async connection).
/// On success: saves profile to keyring and activates it immediately.
/// Returns the new profile's ID and display name to the frontend.
#[tauri::command]
pub async fn connect_bunker(
    identifier: String,
    display_name: String,
    state: State<'_, Arc<Mutex<AppSignerState>>>,
    app_handle: AppHandle,
) -> Result<serde_json::Value, String> {
    use arcadestr_core::nip46::init_signer_session_fast;
    use nostr::nips::nip19::ToBech32;

    info!("connect_bunker called (fast mode): identifier={}, display_name={}", identifier, display_name);

    // For fast mode, we need the user_pubkey. 
    // If it's a bunker URI with known pubkey, extract it.
    // Otherwise, we'll need to do a quick NIP-05 resolution or use a placeholder.
    
    // Try to extract pubkey from bunker URI or resolve NIP-05
    let user_pubkey = if identifier.contains('@') {
        // NIP-05 identifier - resolve to get pubkey
        match resolve_nip05_to_pubkey(&identifier).await {
            Ok(pk) => pk,
            Err(e) => {
                error!("Failed to resolve NIP-05: {}", e);
                return Err(format!("Failed to resolve NIP-05: {}", e));
            }
        }
    } else {
        // Try to parse bunker URI and extract remote signer pubkey
        match extract_pubkey_from_bunker(&identifier) {
            Ok(pk) => pk,
            Err(e) => {
                error!("Failed to extract pubkey from bunker URI: {}", e);
                return Err(format!("Invalid bunker URI: {}", e));
            }
        }
    };

    // Initialize signer session with fast async flow
    let (mut profile, client) = init_signer_session_fast(&identifier, user_pubkey)
        .await
        .map_err(|e| {
            error!("init_signer_session_fast failed: {}", e);
            e.to_string()
        })?;

    // Allow the user to override the auto-generated name
    profile.name = display_name.clone();

    // Save to keyring
    save_profile_to_keyring(&profile)
        .map_err(|e| {
            error!("save_profile_to_keyring failed: {}", e);
            e.to_string()
        })?;

    info!("Profile saved to keyring: id={}", profile.id);

    // Get bunker pubkey for state management
    let bunker_pubkey = profile.bunker_uri.remote_signer_public_key()
        .ok_or("No remote signer public key in URI")?
        .to_hex();

    // Update state with the client (connection happens in background)
    {
        let mut state_guard = state.lock().await;
        state_guard.active_client = Some(client);
        state_guard.active_profile_id = Some(bunker_pubkey.clone());
        state_guard.connection_state = ConnectionState::Connecting; // Will transition on first sign
    }

    // Set as last active profile for auto-restore on next startup
    if let Err(e) = set_last_active_profile_id(&profile.id) {
        warn!("Failed to set last active profile ID: {}", e);
    }

    // Emit auth success event immediately (fast!)
    let user_npub = profile.user_pubkey.to_bech32()
        .map_err(|e| e.to_string())?;
    let _ = app_handle.emit("auth_success", user_npub.clone());

    info!("Fast authentication complete! user_npub={}", user_npub);

    // Return profile info immediately
    Ok(serde_json::json!({
        "id": profile.id,
        "name": profile.name,
        "pubkey": user_npub,
        "pubkey_hex": profile.user_pubkey.to_hex(),
        "connection_state": "connecting", // Frontend can poll for updates
    }))
}
```

- [ ] **Step 2: Add helper functions for pubkey extraction**

Add these helper functions to the file:

```rust
/// Extract public key from bunker URI
fn extract_pubkey_from_bunker(uri: &str) -> Result<PublicKey, String> {
    use nostr::nips::nip46::NostrConnectURI;
    
    let parsed = NostrConnectURI::parse(uri)
        .map_err(|e| format!("Failed to parse bunker URI: {}", e))?;
    
    parsed.remote_signer_public_key()
        .ok_or_else(|| "No remote signer public key in bunker URI".to_string())
}

/// Resolve NIP-05 identifier to public key
async fn resolve_nip05_to_pubkey(identifier: &str) -> Result<PublicKey, String> {
    use nostr::nips::nip05;
    
    let (name, domain) = identifier.split_once('@')
        .ok_or("Invalid NIP-05 format")?;
    
    let profile = nip05::get_profile(name, domain, None)
        .await
        .map_err(|e| format!("NIP-05 resolution failed: {}", e))?;
    
    Ok(profile.public_key)
}
```

- [ ] **Step 3: Add ConnectionState import**

Add to imports:
```rust
use arcadestr_core::nip46::types::ConnectionState;
```

- [ ] **Step 4: Verify compilation**

Run: `cargo check -p arcadestr-desktop`
Expected: No errors

- [ ] **Step 5: Commit**

```bash
git add desktop/src/nip46_commands.rs
git commit -m "feat: update connect_bunker to use fast async flow"
```

---

## Task 5: Update activate_profile for Async Flow

**Files:**
- Modify: `core/src/nip46/session.rs`

**Purpose:** Update profile activation to use deferred connection.

- [ ] **Step 1: Modify activate_profile function**

Replace the function (around line 18-96) with:

```rust
/// Activate a previously saved profile by bunker pubkey.
/// This is called when the user selects a profile from the saved list.
/// FAST VERSION: Uses deferred connection (handshake on first sign).
///
/// Steps:
///   1. Load the SavedProfile from the keyring by bunker pubkey
///   2. Drop + disconnect the current active_client (if any)
///   3. Create LazyNip46Signer with deferred connection
///   4. Build nostr-sdk Client with lazy signer
///   5. Update AppSignerState immediately (connection happens in background)
///
/// # Arguments
/// * `state` - The application state containing active session
/// * `bunker_pubkey` - The bunker pubkey (hex) of the profile to activate
pub async fn activate_profile_fast(
    state: &Arc<Mutex<AppSignerState>>,
    bunker_pubkey: &str,
) -> anyhow::Result<()> {
    use crate::signers::LazyNip46Signer;

    info!("Activating profile (fast mode) for bunker pubkey: {}", bunker_pubkey);

    // STEP 1: Load profile from keyring using bunker pubkey
    let profile = load_profile_from_keyring(bunker_pubkey)
        .ok_or_else(|| anyhow::anyhow!("Profile with bunker pubkey {} not found in keyring", bunker_pubkey))?;

    // STEP 2: Drop old active_client by setting it to None
    {
        let mut state_guard = state.lock().await;
        if state_guard.active_client.is_some() {
            info!("Dropping previous active client");
            state_guard.active_client = None;
            state_guard.active_profile_id = None;
        }
    }

    // STEP 3: Create LazyNip46Signer (deferred connection)
    info!("Creating LazyNip46Signer for bunker pubkey {}...", bunker_pubkey);
    let lazy_signer = LazyNip46Signer::new(
        profile.bunker_uri.clone(),
        profile.app_keys.clone(),
        profile.user_pubkey,
    );

    // STEP 4: Build nostr-sdk Client with lazy signer
    info!("Building nostr-sdk Client with LazyNip46Signer...");
    let client = Client::new(Arc::new(lazy_signer) as Arc<dyn nostr::NostrSigner>);
    info!("Client built successfully (deferred connection)");

    // STEP 5: Update AppSignerState immediately
    {
        let mut state_guard = state.lock().await;
        state_guard.active_client = Some(client);
        state_guard.active_profile_id = Some(bunker_pubkey.to_string());
        state_guard.connection_state = ConnectionState::Connecting;
    }

    info!("Profile {} activated successfully (fast mode): user_pubkey={}", 
        bunker_pubkey, profile.user_pubkey.to_hex());

    Ok(())
}
```

- [ ] **Step 2: Keep old function for backward compatibility**

Rename old `activate_profile` to `activate_profile_blocking`.

- [ ] **Step 3: Update restore_session_on_startup**

Modify `restore_session_on_startup` (around line 111-210) to use the fast flow:

```rust
// STEP 4: Create LazyNip46Signer (deferred connection)
info!("Creating LazyNip46Signer for auto-restore (deferred connection)...");

let lazy_signer = LazyNip46Signer::new(
    profile.bunker_uri.clone(),
    profile.app_keys.clone(),
    profile.user_pubkey,
);

// STEP 5: Build Client immediately without waiting for handshake
info!("Building nostr-sdk Client with LazyNip46Signer...");
let client = Client::new(Arc::new(lazy_signer) as Arc<dyn nostr::NostrSigner>);
info!("Client built successfully for bunker pubkey: {}", key_to_use);

// Update state
{
    let mut state_guard = state.lock().await;
    state_guard.active_client = Some(client);
    state_guard.active_profile_id = Some(profile_id);
    state_guard.is_offline_mode = false;
    state_guard.connection_state = ConnectionState::Connecting;
}
```

- [ ] **Step 4: Verify compilation**

Run: `cargo check -p arcadestr-core`
Expected: No errors

- [ ] **Step 5: Commit**

```bash
git add core/src/nip46/session.rs
git commit -m "feat: update activate_profile and restore_session for async flow"
```

---

## Task 6: Add Connection Status Command

**Files:**
- Modify: `desktop/src/nip46_commands.rs`

**Purpose:** Allow frontend to check connection status.

- [ ] **Step 1: Add get_connection_status command**

Add new command function:

```rust
/// Get the current NIP-46 connection status
#[tauri::command]
pub async fn get_connection_status(
    state: State<'_, Arc<Mutex<AppSignerState>>>,
) -> Result<serde_json::Value, String> {
    let state_guard = state.lock().await;
    
    let status = match &state_guard.connection_state {
        ConnectionState::Disconnected => "disconnected",
        ConnectionState::Connecting => "connecting",
        ConnectionState::Connected => "connected",
        ConnectionState::Failed(_) => "failed",
    };
    
    let error = match &state_guard.connection_state {
        ConnectionState::Failed(e) => Some(e.clone()),
        _ => None,
    };
    
    Ok(serde_json::json!({
        "status": status,
        "error": error,
        "has_active_client": state_guard.active_client.is_some(),
    }))
}
```

- [ ] **Step 2: Register the command in main.rs**

In `desktop/src/main.rs`, add `get_connection_status` to the command list.

- [ ] **Step 3: Verify compilation**

Run: `cargo check -p arcadestr-desktop`
Expected: No errors

- [ ] **Step 4: Commit**

```bash
git add desktop/src/nip46_commands.rs desktop/src/main.rs
git commit -m "feat: add get_connection_status command for frontend polling"
```

---

## Task 7: Update Frontend for Connection Status

**Files:**
- Modify: `app/src/lib.rs`

**Purpose:** Show connection status in UI and poll for updates.

- [ ] **Step 1: Add connection status to AuthContext**

Add to AuthContext struct (around line 373):
```rust
/// Current NIP-46 connection status
pub connection_status: RwSignal<String>,
```

Initialize in AuthContext::new():
```rust
connection_status: RwSignal::new("disconnected".to_string()),
```

- [ ] **Step 2: Add invoke function for get_connection_status**

Add around line 100:
```rust
/// Get NIP-46 connection status
#[cfg(any(target_arch = "wasm32", not(feature = "web")))]
async fn invoke_get_connection_status() -> Result<serde_json::Value, String> {
    use crate::tauri_invoke::invoke;
    invoke("get_connection_status", serde_json::json!({})).await
}

#[cfg(not(any(target_arch = "wasm32", not(feature = "web"))))]
async fn invoke_get_connection_status() -> Result<serde_json::Value, String> {
    Err("Tauri not available in web mode".to_string())
}
```

- [ ] **Step 3: Add connection status polling in LoginView**

In the LoginView component, add polling after successful auth:

```rust
// Add to LoginView component state
let connection_status = RwSignal::new("connecting".to_string());

// Add polling effect after successful login
Effect::new(move |_| {
    if auth.npub.get().is_some() {
        // Start polling connection status
        spawn_local(async move {
            loop {
                match invoke_get_connection_status().await {
                    Ok(status) => {
                        let status_str = status.get("status")
                            .and_then(|s| s.as_str())
                            .unwrap_or("unknown");
                        connection_status.set(status_str.to_string());
                        
                        if status_str == "connected" || status_str == "failed" {
                            break; // Stop polling when final state reached
                        }
                    }
                    Err(_) => break,
                }
                // Poll every 2 seconds
                gloo_timers::future::TimeoutFuture::new(2000).await;
            }
        });
    }
});
```

- [ ] **Step 4: Display connection status in UI**

Add visual indicator in the account selector or main view:

```rust
// In the UI where you show logged-in state
view! {
    <div class="connection-status">
        {move || match connection_status.get().as_str() {
            "connecting" => view! { <span class="status-connecting">"🟡 Connecting..."</span> },
            "connected" => view! { <span class="status-connected">"🟢 Connected"</span> },
            "failed" => view! { <span class="status-failed">"🔴 Connection Failed"</span> },
            _ => view! { <span>"⚪ Unknown"</span> },
        }}
    </div>
}
```

- [ ] **Step 5: Verify compilation**

Run: `cargo check -p arcadestr-app`
Expected: No errors

- [ ] **Step 6: Commit**

```bash
git add app/src/lib.rs
git commit -m "feat: add connection status polling and UI indicator"
```

---

## Task 8: Testing

**Files:**
- Test manually with desktop app

**Purpose:** Verify the async flow works correctly.

- [ ] **Step 1: Build and run desktop app**

```bash
cd /home/joel/Sync/Projetos/Arcadestr/desktop
cargo tauri dev
```

- [ ] **Step 2: Test Flow A (bunker URI)**

1. Click "Connect with Bunker"
2. Paste a bunker:// URI
3. **Verify:** UI returns immediately (within 1-2 seconds)
4. **Verify:** Connection status shows "Connecting..."
5. Approve in signer app
6. **Verify:** Status changes to "Connected"
7. Try to publish a listing
8. **Verify:** First signing request triggers actual handshake

- [ ] **Step 3: Test Flow B (QR code)**

1. Click "Login with QR"
2. Scan QR with mobile signer
3. **Verify:** UI returns immediately after approval
4. **Verify:** Connection status shows "Connecting..."
5. **Verify:** Status changes to "Connected"

- [ ] **Step 4: Test session restore**

1. Close app while connected
2. Reopen app
3. **Verify:** Previous session restores immediately
4. **Verify:** Connection status shows "Connecting..." then "Connected"

- [ ] **Step 5: Test error handling**

1. Enter invalid bunker URI
2. **Verify:** Error shown immediately
3. Disconnect internet
4. Try to sign event
5. **Verify:** Connection fails gracefully with error message

- [ ] **Step 6: Commit test results**

```bash
git add docs/superpowers/tests/nip46-async-test-results.md
git commit -m "test: add async NIP-46 authentication test results"
```

---

## Summary

After completing all tasks, Arcadestr will have:

1. **Immediate authentication response** - UI returns in <2 seconds instead of waiting for full handshake
2. **Deferred connection** - WebSocket connection and NIP-46 handshake happen in background
3. **Connection status tracking** - UI shows real-time connection state
4. **Lazy signing** - First signing request triggers actual handshake
5. **Backward compatibility** - Old blocking functions kept for reference

**Expected performance improvement:**
- Before: 5-30+ seconds (depending on relay latency and user response time)
- After: 1-2 seconds (immediate UI feedback, background connection)

---

## Rollback Plan

If issues occur:

1. Revert to blocking functions by changing `connect_bunker` to call `init_signer_session` instead of `init_signer_session_fast`
2. Revert `activate_profile` calls to use `activate_profile_blocking`
3. Remove connection status UI if causing issues

All changes are additive - old functions remain available.
