//! Lazy NIP-46 Signer
//!
//! This wrapper defers the NIP-46 handshake until the first signing request.
//! It stores the connection parameters and establishes the connection lazily.

use std::sync::Arc;
use tokio::sync::RwLock;
use nostr::{PublicKey, Event, UnsignedEvent, Keys};
use nostr::nips::nip46::NostrConnectURI;
use tracing::{info, error};

use crate::signers::{Nip46Signer, SignerError, NostrSigner};

/// Connection state for lazy signer
#[derive(Debug, Clone, PartialEq)]
pub enum LazyConnectionState {
    /// Initial state, no connection attempted
    Disconnected,
    /// Connection in progress (async handshake happening)
    Connecting,
    /// Successfully connected and ready
    Connected,
    /// Connection failed
    Failed(String),
}

/// Lazy wrapper around NIP-46 signer that defers handshake
#[derive(Clone)]
pub struct LazyNip46Signer {
    /// The bunker URI for connection
    bunker_uri: NostrConnectURI,
    /// Ephemeral app keys for this session
    app_keys: Keys,
    /// Inner signer (initialized lazily)
    inner: Arc<RwLock<Option<Nip46Signer>>>,
    /// Connection state
    state: Arc<RwLock<LazyConnectionState>>,
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
            state: Arc::new(RwLock::new(LazyConnectionState::Disconnected)),
            user_pubkey,
        }
    }

    /// Get current connection state
    pub async fn connection_state(&self) -> LazyConnectionState {
        let state = self.state.read().await;
        (*state).clone()
    }

    /// Ensure connection is established (called before signing)
    async fn ensure_connected(&self) -> Result<Nip46Signer, SignerError> {
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
        *state = LazyConnectionState::Connecting;
        drop(state);

        info!("Establishing deferred NIP-46 connection...");

        // Create the actual signer and perform handshake
        match self.connect_and_handshake().await {
            Ok(signer) => {
                let mut inner = self.inner.write().await;
                *inner = Some(signer.clone());
                
                let mut state = self.state.write().await;
                *state = LazyConnectionState::Connected;
                
                info!("NIP-46 connection established successfully");
                Ok(signer)
            }
            Err(e) => {
                let mut state = self.state.write().await;
                *state = LazyConnectionState::Failed(e.to_string());
                
                error!("Failed to establish NIP-46 connection: {}", e);
                Err(e)
            }
        }
    }

    /// Perform the actual connection and handshake
    async fn connect_and_handshake(&self) -> Result<Nip46Signer, SignerError> {
        use std::time::Duration;

        // Create the NostrConnect client using Nip46Signer's method
        let signer = Nip46Signer::connect_with_keys(
            self.bunker_uri.clone(),
            self.app_keys.clone(),
            Duration::from_secs(30), // Shorter timeout for deferred connection
        ).await?;

        // Perform handshake by calling get_public_key
        signer.get_public_key().await?;

        Ok(signer)
    }
}

#[cfg(not(target_arch = "wasm32"))]
#[async_trait::async_trait]
impl NostrSigner for LazyNip46Signer {
    async fn get_public_key(&self) -> Result<PublicKey, SignerError> {
        // Return the known public key immediately
        Ok(self.user_pubkey)
    }

    async fn sign_event(&self, unsigned: UnsignedEvent) -> Result<Event, SignerError> {
        // Ensure connection before signing
        let signer = self.ensure_connected().await?;
        signer.sign_event(unsigned).await
    }
}

#[cfg(target_arch = "wasm32")]
#[async_trait::async_trait(?Send)]
impl NostrSigner for LazyNip46Signer {
    async fn get_public_key(&self) -> Result<PublicKey, SignerError> {
        // Return the known public key immediately
        Ok(self.user_pubkey)
    }

    async fn sign_event(&self, unsigned: UnsignedEvent) -> Result<Event, SignerError> {
        // Ensure connection before signing
        let signer = self.ensure_connected().await?;
        signer.sign_event(unsigned).await
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
