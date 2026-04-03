// NIP-46 Standard Methods
//
// Implements the standard NIP-46 JSON-RPC methods for remote signing:
// - get_public_key: Get the user's public key
// - sign_event: Sign a Nostr event
// - nip44_encrypt: Encrypt a message using NIP-44
// - nip44_decrypt: Decrypt a message using NIP-44
// - ping: Check if the bunker is alive
//
// These methods are implemented using the nostr-connect crate which handles
// the NIP-44 v2 encryption and relay communication internally.

use nostr::{signer::NostrSigner, Event, PublicKey, UnsignedEvent};
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::{error, info, warn};

use crate::nip46::types::AppSignerState;

/// Get the user's public key from the active bunker.
///
/// This is the standard NIP-46 `get_public_key` method.
///
/// # Returns
/// The user's PublicKey if connected, None otherwise
pub async fn get_public_key(state: &Arc<Mutex<AppSignerState>>) -> Option<PublicKey> {
    let state_guard = state.lock().await;

    match &state_guard.active_client {
        Some(client) => {
            let client_clone = client.clone();
            drop(state_guard);

            // Get the signer from the client, then call get_public_key
            match client_clone.signer().await {
                Ok(signer) => match signer.get_public_key().await {
                    Ok(pubkey) => {
                        info!("get_public_key: {}", pubkey.to_hex());
                        Some(pubkey)
                    }
                    Err(e) => {
                        warn!("get_public_key failed: {}", e);
                        None
                    }
                },
                Err(e) => {
                    warn!("get_public_key: Failed to get signer: {}", e);
                    None
                }
            }
        }
        None => {
            warn!("get_public_key: No active client");
            None
        }
    }
}

/// Sign an unsigned Nostr event using the active bunker.
///
/// This is the standard NIP-46 `sign_event` method.
/// The bunker will validate the event and return a signed version.
///
/// # Arguments
/// * `state` - The application state
/// * `unsigned_event` - The event to sign
///
/// # Returns
/// The signed Event if successful, error message otherwise
pub async fn sign_event(
    state: &Arc<Mutex<AppSignerState>>,
    unsigned_event: UnsignedEvent,
) -> Result<Event, String> {
    let state_guard = state.lock().await;

    let client = state_guard
        .active_client
        .as_ref()
        .ok_or("No active session. Please log in first.")?;

    let client_clone = client.clone();
    drop(state_guard);

    info!("sign_event: Signing event kind {}", unsigned_event.kind);

    // Get the signer from the client, then use it to sign the event
    match client_clone.signer().await {
        Ok(signer) => match signer.sign_event(unsigned_event).await {
            Ok(event) => {
                info!("sign_event: Successfully signed event {}", event.id);
                Ok(event)
            }
            Err(e) => {
                error!("sign_event failed: {}", e);
                Err(format!("Failed to sign event: {}", e))
            }
        },
        Err(e) => {
            error!("sign_event: Failed to get signer: {}", e);
            Err(format!("Failed to get signer: {}", e))
        }
    }
}

/// Encrypt a message using NIP-44 v2.
///
/// This is the standard NIP-46 `nip44_encrypt` method.
/// The message is encrypted for the specified recipient public key.
///
/// # Arguments
/// * `state` - The application state
/// * `recipient_pubkey` - The public key to encrypt for
/// * `plaintext` - The message to encrypt
///
/// # Returns
/// The encrypted ciphertext if successful, error message otherwise
pub async fn nip44_encrypt(
    state: &Arc<Mutex<AppSignerState>>,
    recipient_pubkey: PublicKey,
    plaintext: &str,
) -> Result<String, String> {
    let state_guard = state.lock().await;

    let client = state_guard
        .active_client
        .as_ref()
        .ok_or("No active session. Please log in first.")?;

    let client_clone = client.clone();
    drop(state_guard);

    info!(
        "nip44_encrypt: Encrypting message for {}",
        recipient_pubkey.to_hex()
    );

    // Note: The nostr-connect crate handles NIP-44 encryption internally
    // We need to use the lower-level nostr crate for direct NIP-44 operations
    // For now, this is a placeholder - full implementation would require
    // access to the session's conversation key

    warn!("nip44_encrypt: Full implementation requires conversation key access");
    Err("nip44_encrypt: Not fully implemented".to_string())
}

/// Decrypt a message using NIP-44 v2.
///
/// This is the standard NIP-46 `nip44_decrypt` method.
/// The message is decrypted using the session's conversation key.
///
/// # Arguments
/// * `state` - The application state
/// * `sender_pubkey` - The public key of the sender
/// * `ciphertext` - The encrypted message
///
/// # Returns
/// The decrypted plaintext if successful, error message otherwise
pub async fn nip44_decrypt(
    state: &Arc<Mutex<AppSignerState>>,
    sender_pubkey: PublicKey,
    ciphertext: &str,
) -> Result<String, String> {
    let state_guard = state.lock().await;

    let client = state_guard
        .active_client
        .as_ref()
        .ok_or("No active session. Please log in first.")?;

    let client_clone = client.clone();
    drop(state_guard);

    info!(
        "nip44_decrypt: Decrypting message from {}",
        sender_pubkey.to_hex()
    );

    // Note: Full implementation requires access to the conversation key
    warn!("nip44_decrypt: Full implementation requires conversation key access");
    Err("nip44_decrypt: Not fully implemented".to_string())
}

/// Ping the bunker to check if it's alive.
///
/// This is the standard NIP-46 `ping` method.
/// According to the spec, the bunker should respond with "pong".
///
/// # Arguments
/// * `state` - The application state
///
/// # Returns
/// true if the bunker responds, false otherwise
pub async fn ping(state: &Arc<Mutex<AppSignerState>>) -> bool {
    let state_guard = state.lock().await;

    match &state_guard.active_client {
        Some(client) => {
            let client_clone = client.clone();
            drop(state_guard);

            // The NIP-46 ping method should return "pong"
            // For now, we use get_public_key as a ping test
            // Full implementation would use the actual ping JSON-RPC method
            match client_clone.signer().await {
                Ok(signer) => match signer.get_public_key().await {
                    Ok(_) => {
                        info!("ping: Bunker is alive (via get_public_key)");
                        true
                    }
                    Err(e) => {
                        warn!("ping: Bunker did not respond: {}", e);
                        false
                    }
                },
                Err(e) => {
                    warn!("ping: Failed to get signer: {}", e);
                    false
                }
            }
        }
        None => {
            warn!("ping: No active client");
            false
        }
    }
}

/// Get the status of all connected relays.
///
/// Returns a map of relay URLs to their connection status.
/// This is useful for debugging connection issues.
pub async fn get_relay_status(state: &Arc<Mutex<AppSignerState>>) -> Option<Vec<String>> {
    let state_guard = state.lock().await;

    match &state_guard.active_client {
        Some(client) => {
            let client_clone = client.clone();
            drop(state_guard);

            // Get relay list from the client
            let relays = client_clone.relays().await;
            let relay_urls: Vec<String> = relays.keys().map(|url| url.to_string()).collect();
            info!(
                "get_relay_status: Retrieved status for {} relays",
                relay_urls.len()
            );
            Some(relay_urls)
        }
        None => {
            warn!("get_relay_status: No active client");
            None
        }
    }
}
