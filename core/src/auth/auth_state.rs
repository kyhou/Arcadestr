//! Authentication state management
//!
//! AuthState holds the current authentication state including the active signer
//! and public key. It provides methods for connecting with different methods
//! (direct key, NIP-46) and managing the authentication lifecycle.

use nostr::{Keys, PublicKey};

use crate::signers::{ActiveSigner, DirectKeySigner, Nip46Signer, NostrSigner, SignerError};

/// State for pending NostrConnect handshake
#[derive(Clone, Debug)]
pub struct PendingNostrConnectState {
    /// Client keys for the NIP-46 connection
    pub client_keys: Keys,
    /// Relay URL for the connection
    pub relay: String,
    /// Secret for the connection (if any)
    pub secret: String,
}

/// Authentication state that holds the current signer and public key
#[derive(Clone, Debug)]
pub struct AuthState {
    /// The currently active signer (if authenticated)
    signer: Option<ActiveSigner>,
    /// The public key of the authenticated user
    public_key: Option<PublicKey>,
    /// Pending NostrConnect state (during handshake)
    pending_nostrconnect: Option<PendingNostrConnectState>,
}

impl AuthState {
    /// Creates a new empty authentication state
    pub fn new() -> Self {
        Self {
            signer: None,
            public_key: None,
            pending_nostrconnect: None,
        }
    }

    /// Connect with a direct private key (nsec or hex)
    ///
    /// # Arguments
    /// * `key` - The private key as nsec1... string or hex string
    ///
    /// # Returns
    /// * `Ok(())` - Connection successful
    /// * `Err(SignerError)` - Connection failed
    #[cfg(not(target_arch = "wasm32"))]
    pub fn connect_with_key(&mut self, key: &str) -> Result<(), SignerError> {
        let signer = DirectKeySigner::from_key(key)?;
        // Access public key directly from the keys field
        let public_key = signer.keys().public_key();

        self.signer = Some(ActiveSigner::DirectKey(signer));
        self.public_key = Some(public_key);

        Ok(())
    }

    /// Connect with a NIP-46 signer using a URI
    ///
    /// # Arguments
    /// * `uri` - The NIP-46 URI (nostrconnect:// or bunker://)
    /// * `relay` - The relay URL to use for the connection
    ///
    /// # Returns
    /// * `Ok(())` - Connection successful
    /// * `Err(SignerError)` - Connection failed
    #[cfg(not(target_arch = "wasm32"))]
    pub async fn connect_nip46(&mut self, uri: &str, relay: &str) -> Result<(), SignerError> {
        let signer = Nip46Signer::connect(uri, relay).await?;
        let public_key = signer.get_public_key().await?;

        self.signer = Some(ActiveSigner::Nip46(signer));
        self.public_key = Some(public_key);

        Ok(())
    }

    /// Set the active signer directly
    pub fn set_signer(&mut self, signer: ActiveSigner) {
        self.signer = Some(signer);
    }

    /// Set the NIP-46 signer specifically
    #[cfg(not(target_arch = "wasm32"))]
    pub fn set_nip46_signer(&mut self, signer: Nip46Signer) {
        self.signer = Some(ActiveSigner::Nip46(signer));
    }

    /// Set the public key directly
    pub fn set_public_key(&mut self, public_key: PublicKey) {
        self.public_key = Some(public_key);
    }

    /// Set pending NostrConnect state for handshake
    pub fn set_pending_nostrconnect(&mut self, client_keys: Keys, relay: String, secret: String) {
        self.pending_nostrconnect = Some(PendingNostrConnectState {
            client_keys,
            relay,
            secret,
        });
    }

    /// Get the pending NostrConnect state (if any)
    pub fn pending_nostrconnect(&self) -> Option<&PendingNostrConnectState> {
        self.pending_nostrconnect.as_ref()
    }

    /// Take the pending NostrConnect state (removes it from self)
    pub fn take_pending_nostrconnect(&mut self) -> Option<PendingNostrConnectState> {
        self.pending_nostrconnect.take()
    }

    /// Clear the pending NostrConnect state
    pub fn clear_pending_nostrconnect(&mut self) {
        self.pending_nostrconnect = None;
    }

    /// Get the public key (if authenticated)
    pub fn public_key(&self) -> Option<PublicKey> {
        self.public_key
    }

    /// Check if currently authenticated
    pub fn is_authenticated(&self) -> bool {
        self.signer.is_some() && self.public_key.is_some()
    }

    /// Get the active signer (if authenticated)
    pub fn signer(&self) -> Option<&ActiveSigner> {
        self.signer.as_ref()
    }

    /// Disconnect and clear all authentication state
    pub fn disconnect(&mut self) {
        self.signer = None;
        self.public_key = None;
        self.pending_nostrconnect = None;
    }
}

impl Default for AuthState {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_auth_state_new() {
        let auth = AuthState::new();
        assert!(!auth.is_authenticated());
        assert!(auth.public_key().is_none());
        assert!(auth.signer().is_none());
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn test_auth_state_disconnect() {
        let mut auth = AuthState::new();

        // Create a test key and authenticate with both signer and public key
        let keys = Keys::generate();
        auth.connect_with_key(&keys.secret_key().to_secret_hex())
            .expect("test key should authenticate auth state");

        assert!(auth.is_authenticated());

        auth.disconnect();

        assert!(!auth.is_authenticated());
        assert!(auth.public_key().is_none());
    }
}
