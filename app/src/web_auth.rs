// Web authentication module for NIP-07 browser extension support.
// This module is only compiled for the WASM web target.

#![cfg(all(target_arch = "wasm32", feature = "web"))]

use arcadestr_core::auth::AuthState;
use nostr::nips::nip19::ToBech32;
use std::cell::RefCell;

// Thread-local AuthState for web (no Tauri state management available)
thread_local! {
    static WEB_AUTH: RefCell<AuthState> = RefCell::new(AuthState::new());
}

/// Connect via NIP-07 browser extension.
/// Returns the user's npub bech32 string on success.
pub async fn web_connect_nip07() -> Result<String, String> {
    // Create a temporary AuthState, connect, then store result
    let mut auth = AuthState::new();
    auth.connect_nip07()
        .await
        .map_err(|e: arcadestr_core::signer::SignerError| e.to_string())?;

    let npub = auth
        .public_key()
        .ok_or("No public key after connect".to_string())?
        .to_bech32()
        .map_err(|e| format!("Failed to encode npub: {}", e))?;

    // Store the connected auth state
    WEB_AUTH.with(|cell| {
        *cell.borrow_mut() = auth;
    });

    Ok(npub)
}

/// Get the current public key if authenticated.
pub fn web_get_public_key() -> Option<String> {
    WEB_AUTH.with(|cell| {
        cell.borrow()
            .public_key()
            .and_then(|pk| pk.to_bech32().ok())
    })
}

/// Check if the user is currently authenticated.
pub fn web_is_authenticated() -> bool {
    WEB_AUTH.with(|cell| cell.borrow().is_authenticated())
}

/// Disconnect the current user.
pub fn web_disconnect() {
    WEB_AUTH.with(|cell| {
        *cell.borrow_mut() = AuthState::new();
    })
}
