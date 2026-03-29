// Authentication: NOSTR-based identity, session management, permission handling.

pub use nostr::{Event, PublicKey, UnsignedEvent};
use crate::signer::{ActiveSigner, NostrSigner, SignerError};

#[cfg(not(target_arch = "wasm32"))]
use crate::signer::Nip46Signer;

#[cfg(target_arch = "wasm32")]
use crate::signer::Nip07Signer;

/// Pending nostrconnect connection state.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Clone)]
pub struct PendingNostrConnect {
    /// Client keys for the nostrconnect session.
    pub client_keys: nostr::Keys,
    /// Relay URL to listen on.
    pub relay: String,
    /// Secret for connection validation (NIP-46 requirement).
    pub secret: String,
}

/// Authentication state that holds the active signer and public key.
#[derive(Clone)]
pub struct AuthState {
    /// The active signer, if authenticated.
    signer: Option<ActiveSigner>,
    /// The cached public key, if available.
    public_key: Option<PublicKey>,
    /// Pending nostrconnect connection, if waiting for signer.
    #[cfg(not(target_arch = "wasm32"))]
    pending_nostrconnect: Option<PendingNostrConnect>,
}

impl AuthState {
    /// Creates a new unauthenticated auth state.
    pub fn new() -> Self {
        Self {
            signer: None,
            public_key: None,
            #[cfg(not(target_arch = "wasm32"))]
            pending_nostrconnect: None,
        }
    }

    /// Connects via NIP-46, stores the signer, and fetches the public key.
    #[cfg(not(target_arch = "wasm32"))]
    pub async fn connect_nip46(&mut self, uri: &str, relay: &str) -> Result<(), SignerError> {
        use tracing::{info, error};

        let is_bunker = uri.starts_with("bunker://");
        info!("Auth: Starting NIP-46 connection...");
        info!("Auth: URI type: {}", if is_bunker { "bunker" } else if uri.starts_with("nostrconnect://") { "nostrconnect" } else { "unknown" });

        // Step 1: Create signer connection
        info!("Auth: Creating signer connection...");
        let signer = match Nip46Signer::connect(uri, relay).await {
            Ok(s) => {
                info!("Auth: Signer connection created");
                s
            }
            Err(e) => {
                error!("Auth: Failed to create signer connection: {}", e);
                return Err(e);
            }
        };

        // Step 2: Get public key (this triggers the actual connection handshake)
        info!("Auth: Requesting public key from signer...");
        let public_key = match signer.get_public_key().await {
            Ok(pk) => {
                info!("Auth: Got public key: {}", pk.to_hex());
                pk
            }
            Err(e) => {
                error!("Auth: Failed to get public key: {}", e);
                return Err(e);
            }
        };

        // Step 3: Store the signer and public key
        info!("Auth: Storing signer and public key");
        self.signer = Some(ActiveSigner::Nip46(signer));
        self.public_key = Some(public_key);

        info!("Auth: NIP-46 connection completed successfully");
        Ok(())
    }

    /// Connects via NIP-07, stores the signer, and fetches the public key.
    #[cfg(target_arch = "wasm32")]
    pub async fn connect_nip07(&mut self) -> Result<(), SignerError> {
        let signer = Nip07Signer::new();
        let public_key = signer.get_public_key().await?;

        self.signer = Some(ActiveSigner::Nip07(signer));
        self.public_key = Some(public_key);

        Ok(())
    }

    /// Connects with a raw private key for testing purposes.
    /// 
    /// ⚠️ WARNING: This is for testing only! Use NIP-46 or NIP-07 in production.
    /// 
    /// # Arguments
    /// * `key` - The private key as nsec1... string or hex string
    #[cfg(not(target_arch = "wasm32"))]
    pub fn connect_with_key(&mut self, key: &str) -> Result<(), SignerError> {
        use tracing::{info, error};
        use crate::signer::DirectKeySigner;

        info!("Auth: Connecting with direct key...");

        let signer = match DirectKeySigner::from_key(key) {
            Ok(s) => {
                info!("Auth: Direct key signer created");
                s
            }
            Err(e) => {
                error!("Auth: Failed to create direct key signer: {}", e);
                return Err(e);
            }
        };

        let public_key = signer.keys().public_key();
        info!("Auth: Got public key: {}", public_key.to_hex());

        self.signer = Some(ActiveSigner::DirectKey(signer));
        self.public_key = Some(public_key);

        info!("Auth: Direct key authentication successful");
        Ok(())
    }

    /// Returns true if the user is authenticated (has a signer).
    pub fn is_authenticated(&self) -> bool {
        self.signer.is_some()
    }

    /// Returns the public key if available.
    pub fn public_key(&self) -> Option<&PublicKey> {
        self.public_key.as_ref()
    }

    /// Returns the active signer if available.
    pub fn signer(&self) -> Option<&ActiveSigner> {
        self.signer.as_ref()
    }

    /// Signs an event using the active signer.
    pub async fn sign_event(&self, unsigned: UnsignedEvent) -> Result<Event, SignerError> {
        match &self.signer {
            Some(signer) => signer.sign_event(unsigned).await,
            None => Err(SignerError::NotConnected),
        }
    }

    /// Disconnects the signer and clears the auth state.
    pub fn disconnect(&mut self) {
        self.signer = None;
        self.public_key = None;
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.pending_nostrconnect = None;
        }
    }
}

impl Default for AuthState {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl AuthState {
    /// Sets pending nostrconnect credentials.
    pub fn set_pending_nostrconnect(&mut self, client_keys: nostr::Keys, relay: String, secret: String) {
        self.pending_nostrconnect = Some(PendingNostrConnect {
            client_keys,
            relay,
            secret,
        });
    }

    /// Takes the pending nostrconnect credentials, if any.
    pub fn take_pending_nostrconnect(&mut self) -> Option<(nostr::Keys, String, String)> {
        self.pending_nostrconnect.take().map(|p| (p.client_keys, p.relay, p.secret))
    }

    /// Sets the NIP-46 signer directly.
    pub fn set_nip46_signer(&mut self, signer: Nip46Signer) {
        self.signer = Some(ActiveSigner::Nip46(signer));
    }

    /// Sets the public key directly.
    pub fn set_public_key(&mut self, public_key: PublicKey) {
        self.public_key = Some(public_key);
    }
}
