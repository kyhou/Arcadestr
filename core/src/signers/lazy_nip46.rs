//! Lazy NIP-46 Signer
//!
//! This wrapper defers the NIP-46 handshake until the first signing request.
//! It stores the connection parameters and establishes the connection lazily.

use std::sync::Arc;
use std::any::Any;
use std::fmt;
use tokio::sync::RwLock;
use nostr::{PublicKey, Event, UnsignedEvent, Keys};
use nostr::nips::nip46::NostrConnectURI;
use nostr::signer::{NostrSigner, SignerError, SignerBackend};
use nostr::util::BoxedFuture;
use tracing::{info, error};

/// Error type for LazyNip46Signer operations
#[derive(Debug)]
struct LazySignerError {
    message: String,
}

impl fmt::Display for LazySignerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for LazySignerError {}

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
    /// Inner signer (initialized lazily) - stores the nostr_connect client directly
    inner: Arc<RwLock<Option<Arc<nostr_connect::client::NostrConnect>>>>,
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
    async fn ensure_connected(&self) -> Result<Arc<nostr_connect::client::NostrConnect>, SignerError> {
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
    async fn connect_and_handshake(&self) -> Result<Arc<nostr_connect::client::NostrConnect>, SignerError> {
        use std::time::Duration;
        use nostr::signer::NostrSigner as _;

        // Create the NostrConnect client directly
        let client = nostr_connect::client::NostrConnect::new(
            self.bunker_uri.clone(),
            self.app_keys.clone(),
            Duration::from_secs(30), // Shorter timeout for deferred connection
            None, // Default options
        ).map_err(|e| SignerError::backend(e))?;

        // Perform handshake by calling get_public_key
        client.get_public_key().await?;

        Ok(Arc::new(client))
    }
}

impl NostrSigner for LazyNip46Signer {
    fn backend(&self) -> SignerBackend {
        SignerBackend::Custom(std::borrow::Cow::Borrowed("lazy-nip46"))
    }

    fn get_public_key(&self) -> BoxedFuture<Result<PublicKey, SignerError>> {
        // Return the known public key immediately
        let pubkey = self.user_pubkey;
        Box::pin(async move { Ok(pubkey) })
    }

    fn sign_event(&self, unsigned: UnsignedEvent) -> BoxedFuture<Result<Event, SignerError>> {
        let this = self.clone();
        Box::pin(async move {
            // Ensure connection before signing
            let signer = this.ensure_connected().await?;
            signer.sign_event(unsigned).await
        })
    }

    fn nip04_encrypt<'a>(
        &'a self,
        _public_key: &'a PublicKey,
        _content: &'a str,
    ) -> BoxedFuture<'a, Result<String, SignerError>> {
        Box::pin(async move {
            Err(SignerError::backend(LazySignerError { message: "NIP-04 not supported by LazyNip46Signer".to_string() }))
        })
    }

    fn nip04_decrypt<'a>(
        &'a self,
        _public_key: &'a PublicKey,
        _encrypted_content: &'a str,
    ) -> BoxedFuture<'a, Result<String, SignerError>> {
        Box::pin(async move {
            Err(SignerError::backend(LazySignerError { message: "NIP-04 not supported by LazyNip46Signer".to_string() }))
        })
    }

    fn nip44_encrypt<'a>(
        &'a self,
        _public_key: &'a PublicKey,
        _content: &'a str,
    ) -> BoxedFuture<'a, Result<String, SignerError>> {
        Box::pin(async move {
            Err(SignerError::backend(LazySignerError { message: "NIP-44 not supported by LazyNip46Signer".to_string() }))
        })
    }

    fn nip44_decrypt<'a>(
        &'a self,
        _public_key: &'a PublicKey,
        _payload: &'a str,
    ) -> BoxedFuture<'a, Result<String, SignerError>> {
        Box::pin(async move {
            Err(SignerError::backend(LazySignerError { message: "NIP-44 not supported by LazyNip46Signer".to_string() }))
        })
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

impl AsRef<dyn Any> for LazyNip46Signer {
    fn as_ref(&self) -> &dyn Any {
        self
    }
}
