// Signer integration: NIP-46 remote signer and NIP-07 browser extension support.

use nostr::{Event, Keys, PublicKey, SecretKey, UnsignedEvent};
use std::fs;
use std::path::PathBuf;
use thiserror::Error;
use tracing::{info, error, debug};

#[cfg(not(target_arch = "wasm32"))]
use nostr::nips::nip46::NostrConnectURI;

/// Directory for storing NIP-46 client keys.
/// This is set dynamically at runtime via `set_keys_dir()`.
static KEYS_DIR: std::sync::OnceLock<Option<PathBuf>> = std::sync::OnceLock::new();

/// Sets the directory where NIP-46 client keys will be stored.
/// Must be called before using persistent key functions.
pub fn set_keys_dir(path: PathBuf) {
    let _ = KEYS_DIR.set(Some(path));
}

/// Gets the path to the client keys file.
fn get_keys_path() -> Option<PathBuf> {
    KEYS_DIR.get().and_then(|dir| {
        dir.as_ref().map(|p| p.join(".nostr_client_key"))
    })
}

/// Loads client keys from disk, or generates and saves a new keypair.
///
/// This is essential for NIP-46 connections: signers associate approval with
/// the *specific* client public key. If a new keypair is generated on every
/// invocation, the signer won't recognize the client and will reject connections.
///
/// # Returns
/// - `Ok((Keys, true))` - if new keys were generated
/// - `Ok((Keys, false))` - if existing keys were loaded
pub fn load_or_create_client_keys() -> Result<(Keys, bool), SignerError> {
    let path = match get_keys_path() {
        Some(p) => p,
        None => {
            error!("Keys directory not set! KEYS_DIR.get() returned None");
            return Err(SignerError::Nip46Error("Keys directory not set. Call set_keys_dir() first.".to_string()));
        }
    };

    // Check if file exists
    if path.exists() {
        let hex = fs::read_to_string(&path)
            .map_err(|e| SignerError::Nip46Error(format!("Failed to read keys file: {}", e)))?;
        
        let secret = SecretKey::from_hex(hex.trim())
            .map_err(|e| SignerError::Nip46Error(format!("Failed to parse saved secret key: {}", e)))?;
        
        let keys = Keys::new(secret);
        debug!("Loaded existing client keys from {}", path.display());
        Ok((keys, false)) // false = not newly generated
    } else {
        // Generate new keys
        let keys = Keys::generate();

        // Ensure parent directory exists
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| SignerError::Nip46Error(format!("Failed to create keys directory: {}", e)))?;
        }
        
        // Save to file
        fs::write(&path, keys.secret_key().to_secret_hex())
            .map_err(|e| SignerError::Nip46Error(format!("Failed to save keys: {}", e)))?;
        
        info!("Generated and saved new client keys to {}", path.display());
        Ok((keys, true)) // true = freshly generated
    }
}

/// Resets (deletes) the saved client keys.
/// Use this when you want to generate a fresh keypair.
pub fn reset_client_keys() -> Result<(), SignerError> {
    let path = get_keys_path()
        .ok_or_else(|| SignerError::Nip46Error("Keys directory not set".to_string()))?;

    if path.exists() {
        fs::remove_file(&path)
            .map_err(|e| SignerError::Nip46Error(format!("Failed to delete keys: {}", e)))?;
        info!("Deleted saved client keys from {}", path.display());
    }
    Ok(())
}

/// Errors that can occur when interacting with a NOSTR signer.
#[derive(Error, Debug, Clone)]
pub enum SignerError {
    /// The signer is not connected.
    #[error("Signer not connected")]
    NotConnected,

    /// Signing operation failed.
    #[error("Signing failed: {0}")]
    SigningFailed(String),

    /// Public key is unavailable.
    #[error("Public key unavailable")]
    PublicKeyUnavailable,

    /// NIP-46 specific error.
    #[error("NIP-46 error: {0}")]
    Nip46Error(String),

    /// Serialization error.
    #[error("Serialization error: {0}")]
    Serialization(String),

    /// Connection timeout.
    #[error("Connection timeout")]
    ConnectionTimeout,

    /// No private key available (for local signing).
    #[error("No private key available")]
    NoPrivateKey,

    /// Remote signer not implemented.
    #[error("Remote signer not yet implemented")]
    RemoteNotImplemented,

    /// Read-only account cannot sign.
    #[error("Read-only account cannot sign")]
    ReadOnlyAccount,

    /// JavaScript/WASM error.
    #[cfg(target_arch = "wasm32")]
    #[error("JavaScript error: {0}")]
    JsError(String),
}

impl From<nostr::signer::SignerError> for SignerError {
    fn from(e: nostr::signer::SignerError) -> Self {
        SignerError::SigningFailed(e.to_string())
    }
}

// Native: keep Send + Sync for Tauri compatibility
#[cfg(not(target_arch = "wasm32"))]
#[async_trait::async_trait]
pub trait NostrSigner: Send + Sync {
    /// Returns the signer's public key.
    async fn get_public_key(&self) -> Result<PublicKey, SignerError>;

    /// Signs an unsigned event and returns the signed event.
    async fn sign_event(&self, unsigned: UnsignedEvent) -> Result<Event, SignerError>;
}

// WASM: drop Send + Sync — JS futures are !Send
#[cfg(target_arch = "wasm32")]
#[async_trait::async_trait(?Send)]
pub trait NostrSigner {
    /// Returns the signer's public key.
    async fn get_public_key(&self) -> Result<PublicKey, SignerError>;

    /// Signs an unsigned event and returns the signed event.
    async fn sign_event(&self, unsigned: UnsignedEvent) -> Result<Event, SignerError>;
}

/// NIP-46 remote signer implementation using nostr-connect.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Clone, Debug)]
pub struct Nip46Signer {
    inner: std::sync::Arc<nostr_connect::client::NostrConnect>,
}

#[cfg(not(target_arch = "wasm32"))]
impl Nip46Signer {
    /// Generates a nostrconnect:// URI for client-initiated connections.
    ///
    /// This creates a URI that the user can paste into their signer app (Nsec.app, Amber, etc.)
    /// to establish a connection. IMPORTANT: This reuses persisted client keys if available,
    /// which is required for the signer to recognize the client on subsequent connections.
    ///
    /// # Arguments
    /// * `relay` - The relay URL where the client will listen for responses
    /// * `secret` - A random secret string that the signer must return (ignored, kept for API compat)
    /// * `perms` - Optional comma-separated list of requested permissions (e.g., "sign_event:1,sign_event:30078")
    /// * `name` - Optional client application name
    ///
    /// # Returns
    /// A tuple of (nostrconnect_uri, client_keys) where client_keys must be preserved for the connection
    #[cfg(not(target_arch = "wasm32"))]
    pub fn generate_nostrconnect_uri(
        relay: &str,
        _secret: &str,
        _perms: Option<&str>,
        name: Option<&str>,
    ) -> Result<(String, Keys), SignerError> {
        use nostr::types::url::RelayUrl;

        // Load or create persistent client keys
        // IMPORTANT: This reuses existing keys if available!
        let (client_keys, is_new) = load_or_create_client_keys()?;
        
        if is_new {
            info!("Generated new client keys for NIP-46 connection");
        } else {
            info!("Reusing existing client keys (pubkey: {})", client_keys.public_key().to_hex());
        }

        // Parse relay URL
        let relay_url: RelayUrl = relay.parse()
            .map_err(|e| SignerError::Nip46Error(format!("Invalid relay URL: {}", e)))?;

        // Generate URI using the library's built-in method (like the working implementation)
        // This creates a proper nostrconnect:// URI with automatic secret generation
        let app_name = name.unwrap_or("Arcadestr");
        let uri = NostrConnectURI::client(
            client_keys.public_key(),
            vec![relay_url],
            app_name,
        );

        Ok((uri.to_string(), client_keys))
    }

    /// Waits for a nostrconnect:// signer to connect.
    ///
    /// This matches the working implementation from nostr-connect-tester.
    /// The URI should be generated using NostrConnectUri::client() method.
    ///
    /// # Arguments
    /// * `uri` - The parsed NostrConnectUri (use generate_nostrconnect_uri to create)
    /// * `client_keys` - The client keys used to generate the nostrconnect:// URI (MUST be preserved)
    /// * `timeout_secs` - How long to wait for the signer to connect
    ///
    /// # Returns
    /// A tuple of (connected Nip46Signer instance, signer public key)
    #[cfg(not(target_arch = "wasm32"))]
    pub async fn wait_for_nostrconnect_signer(
        uri: nostr::nips::nip46::NostrConnectURI,
        client_keys: nostr::Keys,
        timeout_secs: u64,
    ) -> Result<(Self, PublicKey), SignerError> {
        use std::time::Duration;
        use nostr::signer::NostrSigner as _;

        info!("Starting nostrconnect signer wait...");
        info!("Client pubkey: {}", client_keys.public_key().to_hex());
        info!("URI: {}", uri);

        // Create the NostrConnect client (matching working implementation)
        info!("Creating NostrConnect client...");
        let inner = match nostr_connect::client::NostrConnect::new(
            uri,
            client_keys,
            Duration::from_secs(timeout_secs),
            None, // Default options
        ) {
            Ok(client) => {
                info!("NostrConnect client created");
                client
            }
            Err(e) => {
                error!("Failed to create NostrConnect client: {:?}", e);
                return Err(SignerError::Nip46Error(format!("Failed to create NostrConnect: {}", e)));
            }
        };

        info!("Waiting for signer...");
        
        // Fetch public key to verify connection (matching working implementation)
        // Call this BEFORE wrapping in our type, just like the MVP does
        info!("About to call get_public_key() on NostrConnect client...");
        match inner.get_public_key().await {
            Ok(pubkey) => {
                info!("Signer connected! Public key: {}", pubkey.to_hex());
                
                // Create signer wrapper AFTER successful connection
                let signer = Self { 
                    inner: std::sync::Arc::new(inner) 
                };
                
                Ok((signer, pubkey))
            }
            Err(e) => {
                error!("FAILED: get_public_key() returned error");
                error!("Error type: {:?}", std::any::type_name_of_val(&e));
                error!("Error message: {}", e);
                error!("Error debug: {:?}", e);
                error!("Possible causes:");
                error!("  - Signer did not approve the connection");
                error!("  - Connection timeout");
                error!("  - Relay is unreachable");
                let converted = SignerError::from(e);
                error!("Converted to SignerError: {:?}", converted);
                Err(converted)
            }
        }
    }

    /// Connects to a NIP-46 signer using a nostrconnect:// or bunker:// URI.
    ///
    /// # Arguments
    /// * `uri` - The NIP-46 URI (nostrconnect:// or bunker://)
    /// * `relay` - The relay URL to use for communication (currently unused, kept for API compatibility)
    pub async fn connect(uri: &str, _relay: &str) -> Result<Self, SignerError> {
        use nostr::prelude::NostrConnectURI;
        use std::time::Duration;


        info!("Parsing NIP-46 URI: {}", uri);

        // Parse the NIP-46 URI
        let uri = NostrConnectURI::parse(uri)
            .map_err(|e| {
                error!("Failed to parse NIP-46 URI: {}", e);
                SignerError::Nip46Error(format!("Invalid NIP-46 URI: {}", e))
            })?;

        info!("URI parsed successfully");

        // Load or create persistent client keys
        // IMPORTANT: This reuses existing keys if available!
        let (client_keys, is_new) = load_or_create_client_keys()?;
        
        if is_new {
            info!("Generated new client keys for connection");
        } else {
            info!("Reusing existing client keys (pubkey: {})", client_keys.public_key().to_hex());
        }

        // Create the NostrConnect client
        // Parameters: uri, client_keys, timeout, opts (None = default)
        info!("Creating NostrConnect client...");
        let inner = match nostr_connect::client::NostrConnect::new(
            uri,
            client_keys,
            Duration::from_secs(60), // 60 second timeout
            None, // Default options
        ) {
            Ok(client) => {
                info!("NostrConnect client created successfully");
                client
            }
            Err(e) => {
                error!("Failed to create NostrConnect client: {:?}", e);
                // Check if this is a JSON parse error
                let err_str = format!("{:?}", e);
                if err_str.contains("parse") || err_str.contains("json") || err_str.contains("expected ident") {
                    error!("JSON parse error detected. This usually means:");
                    error!("1. The relay returned an HTML error page instead of a WebSocket response");
                    error!("2. The signer app returned an error in non-JSON format");
                    error!("3. There's a protocol version mismatch between nostr-connect and the signer");
                    return Err(SignerError::Nip46Error(
                        "Protocol error: Received invalid response from signer/relay. \
                        This usually means the relay returned HTML instead of JSON, or there's a version mismatch. \
                        Try: 1) Use a different relay 2) Use nostrconnect:// flow 3) Check Amber is updated".to_string()
                    ));
                }
                return Err(SignerError::Nip46Error(format!("Failed to create NostrConnect: {}", e)));
            }
        };

        Ok(Self { inner: std::sync::Arc::new(inner) })
    }

    /// Connects to a NIP-46 signer using a pre-existing URI and client keys.
    /// This is used by LazyNip46Signer for deferred connections.
    ///
    /// # Arguments
    /// * `uri` - The parsed NostrConnectURI
    /// * `client_keys` - The client keys to use for the connection
    /// * `timeout` - Connection timeout
    pub async fn connect_with_keys(
        uri: NostrConnectURI,
        client_keys: Keys,
        timeout: std::time::Duration,
    ) -> Result<Self, SignerError> {
        info!("Creating NostrConnect client with provided keys...");
        
        let inner = match nostr_connect::client::NostrConnect::new(
            uri,
            client_keys,
            timeout,
            None, // Default options
        ) {
            Ok(client) => {
                info!("NostrConnect client created successfully");
                client
            }
            Err(e) => {
                error!("Failed to create NostrConnect client: {:?}", e);
                return Err(SignerError::Nip46Error(format!("Failed to create NostrConnect: {}", e)));
            }
        };

        Ok(Self { inner: std::sync::Arc::new(inner) })
    }

    /// Returns the underlying NostrConnect client for advanced usage.
    pub fn inner(&self) -> &nostr_connect::client::NostrConnect {
        &self.inner
    }
}

#[cfg(not(target_arch = "wasm32"))]
#[async_trait::async_trait]
impl NostrSigner for Nip46Signer {
    async fn get_public_key(&self) -> Result<PublicKey, SignerError> {
        use nostr::signer::NostrSigner as _;

        self.inner
            .get_public_key()
            .await
            .map_err(SignerError::from)
    }

    async fn sign_event(&self, unsigned: UnsignedEvent) -> Result<Event, SignerError> {
        use nostr::signer::NostrSigner as _;

        self.inner
            .sign_event(unsigned)
            .await
            .map_err(SignerError::from)
    }
}

/// NIP-07 browser extension signer implementation for WASM target.
#[cfg(target_arch = "wasm32")]
#[derive(Clone, Debug)]
pub struct Nip07Signer;

#[cfg(target_arch = "wasm32")]
impl Nip07Signer {
    /// Creates a new NIP-07 signer instance.
    pub fn new() -> Self {
        Self
    }

    /// Checks if window.nostr is available.
    fn is_available() -> bool {
        use wasm_bindgen::JsCast;
        use web_sys::window;

        if let Some(win) = window() {
            if let Ok(nostr) = js_sys::Reflect::get(&win, &"nostr".into()) {
                return !nostr.is_undefined() && !nostr.is_null();
            }
        }
        false
    }
}

#[cfg(target_arch = "wasm32")]
impl Default for Nip07Signer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(target_arch = "wasm32")]
#[async_trait::async_trait(?Send)]
impl NostrSigner for Nip07Signer {
    async fn get_public_key(&self) -> Result<PublicKey, SignerError> {
        use wasm_bindgen::JsCast;
        use wasm_bindgen_futures::JsFuture;
        use web_sys::window;

        if !Self::is_available() {
            return Err(SignerError::NotConnected);
        }

        let win = window().ok_or(SignerError::NotConnected)?;
        let nostr = js_sys::Reflect::get(&win, &"nostr".into())
            .map_err(|_| SignerError::NotConnected)?;

        let get_public_key = js_sys::Reflect::get(&nostr, &"getPublicKey".into())
            .map_err(|_| SignerError::NotConnected)?;

        let promise: js_sys::Promise = get_public_key
            .dyn_into()
            .map_err(|_| SignerError::JsError("getPublicKey is not a function".into()))?;

        let result = JsFuture::from(promise)
            .await
            .map_err(|e| SignerError::JsError(format!("{:?}", e)))?;

        let hex_pubkey: String = result
            .as_string()
            .ok_or(SignerError::JsError("Expected string public key".into()))?;

        PublicKey::from_hex(&hex_pubkey)
            .map_err(|_| SignerError::PublicKeyUnavailable)
    }

    async fn sign_event(&self, unsigned: UnsignedEvent) -> Result<Event, SignerError> {
        use wasm_bindgen::JsCast;
        use wasm_bindgen_futures::JsFuture;
        use web_sys::window;

        if !Self::is_available() {
            return Err(SignerError::NotConnected);
        }

        let win = window().ok_or(SignerError::NotConnected)?;
        let nostr = js_sys::Reflect::get(&win, &"nostr".into())
            .map_err(|_| SignerError::NotConnected)?;

        let sign_event = js_sys::Reflect::get(&nostr, &"signEvent".into())
            .map_err(|_| SignerError::NotConnected)?;

        // Serialize the unsigned event to JSON
        let event_json = serde_json::to_string(&unsigned)
            .map_err(|e| SignerError::Serialization(e.to_string()))?;

        let event_js = js_sys::JSON::parse(&event_json)
            .map_err(|e| SignerError::JsError(format!("Failed to parse event JSON: {:?}", e)))?;

        let promise: js_sys::Promise = sign_event
            .dyn_into()
            .map_err(|_| SignerError::JsError("signEvent is not a function".into()))?;

        let result = JsFuture::from(promise)
            .await
            .map_err(|e| SignerError::JsError(format!("{:?}", e)))?;

        let signed_json = js_sys::JSON::stringify(&result)
            .map_err(|e| SignerError::JsError(format!("Failed to stringify result: {:?}", e)))?
            .as_string()
            .ok_or(SignerError::JsError("Expected string result".into()))?;

        serde_json::from_str(&signed_json)
            .map_err(|e| SignerError::Serialization(e.to_string()))
    }
}

/// Direct key signer for testing - uses a raw private key (nsec or hex).
/// 
/// ⚠️ WARNING: This is for testing only! In production, use NIP-46 or NIP-07
/// to avoid exposing private keys to the application.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Clone, Debug)]
pub struct DirectKeySigner {
    keys: nostr::Keys,
}

#[cfg(not(target_arch = "wasm32"))]
impl DirectKeySigner {
    /// Creates a new direct key signer from a nsec (bech32) or hex private key string.
    ///
    /// # Arguments
    /// * `key` - The private key as nsec1... string or hex string
    pub fn from_key(key: &str) -> Result<Self, SignerError> {
        let keys = if key.starts_with("nsec1") {
            // Parse as bech32 nsec
            nostr::Keys::parse(key)
                .map_err(|e| SignerError::SigningFailed(format!("Invalid nsec key: {}", e)))?
        } else {
            // Parse as hex
            nostr::Keys::parse(key)
                .map_err(|e| SignerError::SigningFailed(format!("Invalid hex key: {}", e)))?
        };

        Ok(Self { keys })
    }

    /// Returns the keys (for advanced usage).
    pub fn keys(&self) -> &nostr::Keys {
        &self.keys
    }
}

#[cfg(not(target_arch = "wasm32"))]
#[async_trait::async_trait]
impl NostrSigner for DirectKeySigner {
    async fn get_public_key(&self) -> Result<PublicKey, SignerError> {
        Ok(self.keys.public_key())
    }

    async fn sign_event(&self, unsigned: UnsignedEvent) -> Result<Event, SignerError> {
        unsigned.sign(&self.keys)
            .await
            .map_err(|e| SignerError::SigningFailed(format!("Failed to sign event: {}", e)))
    }
}

/// Active signer enum that wraps the different signer implementations.
#[non_exhaustive]
#[derive(Clone, Debug)]
pub enum ActiveSigner {
    /// NIP-46 remote signer (desktop).
    #[cfg(not(target_arch = "wasm32"))]
    Nip46(Nip46Signer),
    /// Direct key signer for testing (desktop).
    #[cfg(not(target_arch = "wasm32"))]
    DirectKey(DirectKeySigner),
    /// NIP-07 browser extension signer (web/WASM).
    #[cfg(target_arch = "wasm32")]
    Nip07(Nip07Signer),
}

#[cfg(not(target_arch = "wasm32"))]
#[async_trait::async_trait]
impl NostrSigner for ActiveSigner {
    async fn get_public_key(&self) -> Result<PublicKey, SignerError> {
        match self {
            ActiveSigner::Nip46(signer) => signer.get_public_key().await,
            ActiveSigner::DirectKey(signer) => signer.get_public_key().await,
        }
    }

    async fn sign_event(&self, unsigned: UnsignedEvent) -> Result<Event, SignerError> {
        match self {
            ActiveSigner::Nip46(signer) => signer.sign_event(unsigned).await,
            ActiveSigner::DirectKey(signer) => signer.sign_event(unsigned).await,
        }
    }
}

#[cfg(target_arch = "wasm32")]
#[async_trait::async_trait(?Send)]
impl NostrSigner for ActiveSigner {
    async fn get_public_key(&self) -> Result<PublicKey, SignerError> {
        match self {
            ActiveSigner::Nip07(signer) => signer.get_public_key().await,
        }
    }

    async fn sign_event(&self, unsigned: UnsignedEvent) -> Result<Event, SignerError> {
        match self {
            ActiveSigner::Nip07(signer) => signer.sign_event(unsigned).await,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_nostrconnect_uri_basic() {
        let relay = "wss://relay.damus.io";
        let secret = "test_secret_123";
        
        let result = Nip46Signer::generate_nostrconnect_uri(
            relay,
            secret,
            None,
            None,
        );
        
        assert!(result.is_ok());
        let (uri, _pubkey) = result.unwrap();
        
        // Check URI starts with nostrconnect://
        assert!(uri.starts_with("nostrconnect://"));
        
        // Check URI contains the relay
        assert!(uri.contains("relay="));
        
        // Check URI contains metadata (generated by the library)
        assert!(uri.contains("metadata="));
        
        // Verify the URI format matches what the library generates
        // Format: nostrconnect://<pubkey>?metadata={"name":"..."}&relay=wss://...
        assert!(uri.starts_with("nostrconnect://"));
    }

    #[test]
    fn test_generate_nostrconnect_uri_returns_keys() {
        let relay = "wss://relay.nostr.band";
        
        let result = Nip46Signer::generate_nostrconnect_uri(
            relay,
            "",
            None,
            None,
        );
        
        assert!(result.is_ok());
        let (uri, client_keys) = result.unwrap();
        
        // Check we got valid keys back
        assert!(!client_keys.public_key().to_hex().is_empty());
        
        // Check URI contains the pubkey
        let expected_prefix = format!("nostrconnect://{}", client_keys.public_key().to_hex());
        assert!(uri.starts_with(&expected_prefix));
    }

    #[test]
    fn test_generate_nostrconnect_uri_with_name() {
        let relay = "wss://relay.example.com";
        let secret = "secret789";
        let name = "TestApp";
        
        let result = Nip46Signer::generate_nostrconnect_uri(
            relay,
            secret,
            None,
            Some(name),
        );
        
        assert!(result.is_ok());
        let (uri, _) = result.unwrap();
        
        // Check URI contains metadata parameter (which includes the app name)
        assert!(uri.contains("metadata="));
        // The name is URL-encoded inside the metadata JSON
        // URL-encoded "TestApp" is still "TestApp" since it has no special chars
        assert!(uri.contains("TestApp"));
    }

    #[test]
    fn test_generate_nostrconnect_uri_url_encoding() {
        let relay = "wss://relay.test.com/path";
        let secret = "secret";
        
        let result = Nip46Signer::generate_nostrconnect_uri(
            relay,
            secret,
            None,
            None,
        );
        
        assert!(result.is_ok());
        let (uri, _) = result.unwrap();
        
        // Special characters should be encoded
        assert!(!uri.contains(" "));
    }

    #[test]
    fn test_generate_nostrconnect_uri_unique() {
        let relay = "wss://relay.damus.io";
        
        // Generate multiple URIs and ensure they're different
        let mut uris = Vec::new();
        for i in 0..5 {
            let secret = format!("secret_{}", i);
            let result = Nip46Signer::generate_nostrconnect_uri(
                relay,
                &secret,
                None,
                None,
            ).unwrap();
            uris.push(result.0);
        }
        
        // Check all URIs are unique
        let unique_uris: std::collections::HashSet<_> = uris.iter().collect();
        assert_eq!(uris.len(), unique_uris.len());
    }
}
