//! Lazy NIP-46 Signer
//!
//! This wrapper defers the NIP-46 handshake until the first signing request.
//! It stores the connection parameters and establishes the connection lazily.

use nostr::nips::nip46::NostrConnectURI;
use nostr::signer::{NostrSigner, SignerBackend, SignerError};
use nostr::util::BoxedFuture;
use nostr::{Event, Keys, PublicKey, UnsignedEvent};
use std::any::Any;
use std::fmt;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{error, info};

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
    inner: Arc<RwLock<Option<Arc<dyn NostrSigner>>>>,
    /// Connection state
    state: Arc<RwLock<LazyConnectionState>>,
    /// User's public key (known from initial connection)
    user_pubkey: PublicKey,
    connect_gate: Arc<tokio::sync::Semaphore>,
}

impl LazyNip46Signer {
    /// Create a new lazy signer without establishing connection
    pub fn new(bunker_uri: NostrConnectURI, app_keys: Keys, user_pubkey: PublicKey) -> Self {
        info!("Creating LazyNip46Signer (deferred connection)");
        Self {
            bunker_uri,
            app_keys,
            inner: Arc::new(RwLock::new(None)),
            state: Arc::new(RwLock::new(LazyConnectionState::Disconnected)),
            user_pubkey,
            connect_gate: Arc::new(tokio::sync::Semaphore::new(1)),
        }
    }

    /// Get current connection state
    pub async fn connection_state(&self) -> LazyConnectionState {
        let state = self.state.read().await;
        (*state).clone()
    }

    /// Ensure connection is established (called before signing)
    async fn ensure_connected(&self) -> Result<Arc<dyn NostrSigner>, SignerError> {
        let _permit = self
            .connect_gate
            .acquire()
            .await
            .map_err(SignerError::backend)?;
        // Check if already connected
        {
            let inner = self.inner.read().await;
            if let Some(signer) = inner.clone() {
                return Ok(signer);
            }
        }

        // The semaphore coalesces connection initialization without retaining a
        // state lock across network operations or another asynchronous lock.
        *self.state.write().await = LazyConnectionState::Connecting;

        info!("Establishing deferred NIP-46 connection...");

        // Create the actual signer and perform handshake
        match self.connect_and_handshake().await {
            Ok(signer) => {
                let mut inner = self.inner.write().await;
                *inner = Some(signer.clone());
                drop(inner);

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
    async fn connect_and_handshake(&self) -> Result<Arc<dyn NostrSigner>, SignerError> {
        use std::time::Duration;
        let client = super::restored_nip46::RestoredNip46Signer::connect(
            &self.bunker_uri,
            self.app_keys.clone(),
            self.user_pubkey,
            Duration::from_secs(30),
        )
        .await?;
        Ok(Arc::new(client))
    }

    /// Verify saved authorization with the remote signer, never a pairing token.
    pub async fn verify_saved_session(&self) -> Result<(), SignerError> {
        self.ensure_connected().await?;
        Ok(())
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
            Err(SignerError::backend(LazySignerError {
                message: "NIP-04 not supported by LazyNip46Signer".to_string(),
            }))
        })
    }

    fn nip04_decrypt<'a>(
        &'a self,
        _public_key: &'a PublicKey,
        _encrypted_content: &'a str,
    ) -> BoxedFuture<'a, Result<String, SignerError>> {
        Box::pin(async move {
            Err(SignerError::backend(LazySignerError {
                message: "NIP-04 not supported by LazyNip46Signer".to_string(),
            }))
        })
    }

    fn nip44_encrypt<'a>(
        &'a self,
        public_key: &'a PublicKey,
        content: &'a str,
    ) -> BoxedFuture<'a, Result<String, SignerError>> {
        Box::pin(async move {
            let signer = self.ensure_connected().await?;
            signer.nip44_encrypt(public_key, content).await
        })
    }

    fn nip44_decrypt<'a>(
        &'a self,
        public_key: &'a PublicKey,
        payload: &'a str,
    ) -> BoxedFuture<'a, Result<String, SignerError>> {
        Box::pin(async move {
            let signer = self.ensure_connected().await?;
            signer.nip44_decrypt(public_key, payload).await
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

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn restored_session_uses_authorization_not_consumed_pairing_secret(
    ) -> Result<(), Box<dyn std::error::Error>> {
        use futures_util::{SinkExt, StreamExt};
        use nostr::{EventBuilder, JsonUtil, Kind};
        use serde_json::{json, Value};
        let user = Keys::generate();
        let app = Keys::generate();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let uri = format!(
            "bunker://{}?relay=ws%3A%2F%2F127.0.0.1%3A{}&secret=already-consumed",
            user.public_key(),
            listener.local_addr()?.port()
        )
        .parse()?;
        let signer = LazyNip46Signer::new(uri, app.clone(), user.public_key());
        let remote = user.clone();
        let methods = Arc::new(std::sync::Mutex::new(Vec::new()));
        let recorded = methods.clone();
        let task = tokio::spawn(async move {
            let (tcp, _) = listener.accept().await.expect("test listener");
            let mut ws = tokio_tungstenite::accept_async(tcp)
                .await
                .expect("test websocket");
            let mut subscription = String::new();
            while let Some(Ok(frame)) = ws.next().await {
                let Ok(text) = frame.to_text() else { continue };
                let Ok(value) = serde_json::from_str::<Value>(text) else {
                    continue;
                };
                match value[0].as_str() {
                    Some("REQ") => {
                        subscription = value[1].as_str().expect("subscription").into();
                        ws.send(json!(["EOSE", subscription]).to_string().into())
                            .await
                            .expect("EOSE");
                    }
                    Some("EVENT") => {
                        let event: Event = serde_json::from_value(value[1].clone()).expect("event");
                        event.verify().expect("authentic app request");
                        let text = nostr::nips::nip44::decrypt(
                            remote.secret_key(),
                            &event.pubkey,
                            &event.content,
                        )
                        .expect("request decryption");
                        let request: Value = serde_json::from_str(&text).expect("request");
                        let method = request["method"].as_str().expect("method");
                        recorded.lock().expect("methods").push(method.to_owned());
                        let (result, error) = match method {
                            "connect" => (
                                Value::Null,
                                json!("denied: no matching pairing in progress"),
                            ),
                            "get_public_key" => (json!(remote.public_key().to_hex()), Value::Null),
                            "sign_event" => {
                                let unsigned: UnsignedEvent = serde_json::from_str(
                                    request["params"][0].as_str().expect("unsigned"),
                                )
                                .expect("unsigned event");
                                (
                                    json!(unsigned
                                        .sign_with_keys(&remote)
                                        .expect("signature")
                                        .as_json()),
                                    Value::Null,
                                )
                            }
                            "nip44_decrypt" => {
                                let peer = PublicKey::from_hex(
                                    request["params"][0].as_str().expect("peer"),
                                )
                                .expect("peer key");
                                (
                                    json!(nostr::nips::nip44::decrypt(
                                        remote.secret_key(),
                                        &peer,
                                        request["params"][1].as_str().expect("payload")
                                    )
                                    .expect("payload")),
                                    Value::Null,
                                )
                            }
                            _ => (Value::Null, json!("unsupported test request")),
                        };
                        let response: nostr::nips::nip46::NostrConnectMessage =
                            serde_json::from_value(
                                json!({"id":request["id"],"result":result,"error":error}),
                            )
                            .expect("response");
                        let response = EventBuilder::nostr_connect(&remote, event.pubkey, response)
                            .expect("encrypted response")
                            .sign_with_keys(&remote)
                            .expect("signed response");
                        ws.send(json!(["OK", event.id, true, ""]).to_string().into())
                            .await
                            .expect("OK");
                        ws.send(json!(["EVENT", subscription, response]).to_string().into())
                            .await
                            .expect("response event");
                    }
                    _ => {}
                }
            }
        });
        let unsigned = EventBuilder::new(Kind::Custom(27235), "").build(user.public_key());
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            signer.sign_event(unsigned),
        )
        .await?;
        if result.is_err() {
            task.abort();
        }
        let signed = result?;
        signed.verify()?;
        let peer = Keys::generate();
        let encrypted = nostr::nips::nip44::encrypt(
            peer.secret_key(),
            &user.public_key(),
            "original checkpoint",
            nostr::nips::nip44::Version::V2,
        )?;
        assert_eq!(
            signer.nip44_decrypt(&peer.public_key(), &encrypted).await?,
            "original checkpoint"
        );
        assert_eq!(
            *methods.lock().expect("recorded methods"),
            vec!["get_public_key", "sign_event", "nip44_decrypt"]
        );
        task.abort();
        Ok(())
    }

    #[tokio::test]
    async fn nip44_operations_forward_to_the_connected_backend(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let backend = Keys::generate();
        let peer = Keys::generate();
        let uri = format!(
            "bunker://{}?relay=ws%3A%2F%2F127.0.0.1%3A1",
            backend.public_key()
        )
        .parse()?;
        let signer = LazyNip46Signer::new(uri, Keys::generate(), backend.public_key());
        *signer.inner.write().await = Some(Arc::new(backend.clone()));
        *signer.state.write().await = LazyConnectionState::Connected;
        let encrypted = signer
            .nip44_encrypt(&peer.public_key(), "checkpoint")
            .await?;
        assert_eq!(
            peer.nip44_decrypt(&backend.public_key(), &encrypted)
                .await?,
            "checkpoint"
        );
        let incoming = peer
            .nip44_encrypt(&backend.public_key(), "original proof")
            .await?;
        assert_eq!(
            signer.nip44_decrypt(&peer.public_key(), &incoming).await?,
            "original proof"
        );
        assert_eq!(
            signer.connection_state().await,
            LazyConnectionState::Connected
        );
        let expected = backend
            .nip44_decrypt(&peer.public_key(), "invalid")
            .await
            .err();
        let actual = signer
            .nip44_decrypt(&peer.public_key(), "invalid")
            .await
            .err();
        assert!(expected.is_some());
        assert_eq!(
            actual.map(|e| e.to_string()),
            expected.map(|e| e.to_string())
        );
        Ok(())
    }
}
