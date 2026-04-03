// NIP-46 Core Authentication Logic
//
// This is the most critical file. It handles both Flow A (user provides URI or NIP-05)
// and Flow B (app generates QR code).
//
// ## Implementation Notes
//
// This implementation uses the `nostr-connect` crate which provides NIP-46 client functionality.
// The crate internally handles:
// - NIP-44 v2 encryption for all kind 24133 events
// - WebSocket connections to relays
// - JSON-RPC request/response matching
//
// ## Auth URL Flow
//
// Some bunkers (like nsec.app) require user approval via a web URL. When this happens,
// the `auth_url_handler` callback is triggered, which emits a Tauri event to the frontend
// so the user can open the URL in their browser.

use nostr::types::url::RelayUrl;
use nostr::{
    nips::nip05::Nip05Address, nips::nip46::NostrConnectURI, signer::NostrSigner, Keys, PublicKey,
};
use nostr_connect::client::{AuthUrlHandler, NostrConnect};
use nostr_sdk::Client;
use std::fmt;
use std::sync::Arc;
use std::time::Duration;
use tracing::{info, warn};
use url::Url;

use crate::nip46::types::SavedProfile;

/// Auth URL handler that emits Tauri events for web approval
#[derive(Clone)]
pub struct TauriAuthUrlHandler {
    /// Callback function that receives the auth URL and emits it to the frontend
    pub on_auth: Arc<dyn Fn(Url) + Send + Sync>,
}

impl fmt::Debug for TauriAuthUrlHandler {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TauriAuthUrlHandler").finish()
    }
}

impl AuthUrlHandler for TauriAuthUrlHandler {
    fn on_auth_url(
        &self,
        auth_url: Url,
    ) -> nostr_connect::prelude::BoxedFuture<nostr_connect::prelude::Result<()>> {
        let handler = self.on_auth.clone();
        Box::pin(async move {
            info!("Auth URL received from bunker: {}", auth_url);
            handler(auth_url);
            Ok(())
        })
    }
}

/// Entry point for Flow A: user provides either a "bunker://..." URI
/// or a NIP-05 identifier like "bob@nsec.app".
///
/// Steps performed:
///   1. Resolve identifier → NostrConnectURI
///   2. Generate ephemeral Keys for this session
///   3. Create NostrConnect signer with auth URL handler
///   4. Build nostr-sdk Client with Arc-wrapped signer
///   5. Perform NIP-46 handshake via get_public_key()
///   6. Return a SavedProfile and the Client (caller is responsible for persisting it)
///
/// # Arguments
/// * `identifier` - bunker:// URI or NIP-05 identifier (user@domain)
/// * `on_auth` - Optional callback for auth URL handling (for web approval)
///
/// # Returns
/// Tuple of (SavedProfile, Client) containing the connection details and active client
pub async fn init_signer_session<F>(
    identifier: &str,
    on_auth: Option<F>,
) -> anyhow::Result<(SavedProfile, Client)>
where
    F: Fn(Url) + Send + Sync + 'static,
{
    info!("init_signer_session called with identifier: {}", identifier);

    // STEP 1 — URI Resolution (NIP-05 discovery or direct parse)
    let uri = if identifier.contains('@') {
        info!("Resolving NIP-05 identifier: {}", identifier);
        resolve_nip05_to_uri(identifier).await?
    } else {
        info!("Parsing bunker URI directly");
        NostrConnectURI::parse(identifier)?
    };

    // STEP 2 — Ephemeral key generation (one unique keypair per profile/bunker link)
    let app_keys = Keys::generate();
    info!(
        "Generated ephemeral app_keys: pubkey={}",
        app_keys.public_key().to_hex()
    );

    // STEP 3 — Create NostrConnect signer with 60s timeout
    // nostr-connect 0.44 internally uses NIP-44 v2 for kind 24133 events.
    info!("Creating NostrConnect signer with 60s timeout...");
    let mut signer = NostrConnect::new(
        uri.clone(),
        app_keys.clone(),
        Duration::from_secs(60),
        None, // Default relay options
    )?;

    // STEP 4 — Set up auth URL handler if provided
    // This is critical for bunkers like nsec.app that require browser approval
    if let Some(handler) = on_auth {
        let auth_handler = TauriAuthUrlHandler {
            on_auth: Arc::new(handler),
        };
        signer.auth_url_handler(auth_handler);
        info!("Auth URL handler registered");
    }

    // STEP 5 — Verify all relays from URI are available
    // The NostrConnect signer automatically manages relays from the URI,
    // but we log them for debugging (anySync pattern)
    let available_relays: Vec<RelayUrl> = uri.relays().iter().cloned().collect();
    info!(
        "NostrConnect will use {} relays: {:?}",
        available_relays.len(),
        available_relays
    );

    // STEP 6 — NIP-46 handshake
    // Calling get_public_key() triggers the connect handshake
    // It sends a "connect" JSON-RPC command (kind 24133) to the bunker
    // and waits for the bunker's response with the user's public key
    info!("Performing NIP-46 handshake...");
    let user_pubkey = signer.get_public_key().await?;
    info!(
        "NIP-46 handshake successful! user_pubkey={}",
        user_pubkey.to_hex()
    );

    // STEP 7 — Build nostr-sdk Client with Arc-wrapped signer
    info!("Building nostr-sdk Client with Nip46Signer...");
    let client = Client::new(Arc::new(signer) as Arc<dyn nostr::NostrSigner>);
    info!("Client built successfully with Nip46Signer");

    // STEP 8 — Generate profile ID and return
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

/// Fast async version of init_signer_session - returns immediately without blocking on handshake.
///
/// This is the optimized version that matches Yakihonne's behavior:
/// - Returns immediately after creating the signer (no blocking handshake)
/// - Connection is established lazily on first signing request
/// - User sees instant UI feedback
///
/// Steps performed:
///   1. Parse the bunker URI
///   2. Generate ephemeral Keys for this session
///   3. Create LazyNip46Signer (deferred connection)
///   4. Build nostr-sdk Client with lazy signer
///   5. Return immediately with SavedProfile
///
/// # Arguments
/// * `bunker_uri` - The bunker:// URI (already parsed)
/// * `user_pubkey` - The user's public key (known from initial connection)
///
/// # Returns
/// Tuple of (SavedProfile, Client) containing the connection details and active client
pub async fn init_signer_session_fast(
    bunker_uri: NostrConnectURI,
    user_pubkey: PublicKey,
) -> anyhow::Result<(SavedProfile, Client)> {
    use crate::signers::LazyNip46Signer;

    info!("init_signer_session_fast called (deferred connection)");

    // STEP 1 — Ephemeral key generation
    let app_keys = Keys::generate();
    info!(
        "Generated ephemeral app_keys: pubkey={}",
        app_keys.public_key().to_hex()
    );

    // STEP 2 — Create LazyNip46Signer (deferred connection - no blocking handshake)
    info!("Creating LazyNip46Signer with deferred connection...");
    let lazy_signer = LazyNip46Signer::new(bunker_uri.clone(), app_keys.clone(), user_pubkey);

    // STEP 3 — Build nostr-sdk Client with lazy signer
    info!("Building nostr-sdk Client with LazyNip46Signer...");
    let client = Client::new(Arc::new(lazy_signer) as Arc<dyn nostr::NostrSigner>);
    info!("Client built successfully (deferred connection)");

    // STEP 4 — Generate profile ID and return immediately
    let profile_id = uuid::Uuid::new_v4().to_string();

    let profile = SavedProfile {
        id: profile_id,
        name: format!("Bunker {}", user_pubkey.to_hex()[..8].to_string()),
        user_pubkey,
        bunker_uri,
        app_keys,
    };

    info!("Fast authentication complete - returning immediately!");
    Ok((profile, client))
}

/// Resolves a NIP-05 identifier to a NostrConnectURI.
///
/// Fetches https://{domain}/.well-known/nostr.json?name={local}
/// and extracts the pubkey and NIP-46 relays.
///
/// Hard constraint: If nip46 field is missing or empty, returns an error.
/// No fallback to other relay sources.
async fn resolve_nip05_to_uri(identifier: &str) -> anyhow::Result<NostrConnectURI> {
    let address = Nip05Address::parse(identifier)
        .map_err(|e| anyhow::anyhow!("Invalid NIP-05 format: {}", e))?;

    let client = reqwest::Client::new();
    let url = format!(
        "https://{}/.well-known/nostr.json?name={}",
        address.domain(),
        address.name()
    );

    info!("Fetching NIP-05 from: {}", url);

    let response = client
        .get(&url)
        .send()
        .await
        .map_err(|e| anyhow::anyhow!("Failed to fetch NIP-05: {}", e))?;

    let json: serde_json::Value = response
        .json()
        .await
        .map_err(|e| anyhow::anyhow!("Failed to parse NIP-05 JSON: {}", e))?;

    // Extract the public key from the response
    let names = json
        .get("names")
        .and_then(|n| n.as_object())
        .ok_or_else(|| anyhow::anyhow!("No 'names' field in NIP-05 response"))?;

    let pubkey_hex = names
        .get(address.name())
        .and_then(|v| v.as_str())
        .ok_or_else(|| {
            anyhow::anyhow!(
                "No pubkey found for '{}' in NIP-05 response",
                address.name()
            )
        })?;

    let pubkey = PublicKey::from_hex(pubkey_hex)
        .map_err(|e| anyhow::anyhow!("Invalid pubkey in NIP-05 response: {}", e))?;

    // Get NIP-46 relays from the response - HARD REQUIREMENT
    let nip46_relays = json
        .get("nip46")
        .and_then(|n| n.as_object())
        .and_then(|n| n.get(pubkey_hex))
        .and_then(|v| v.as_array());

    let relays: Vec<RelayUrl> = match nip46_relays {
        Some(arr) if !arr.is_empty() => arr
            .iter()
            .filter_map(|v| v.as_str())
            .filter_map(|s| s.parse().ok())
            .collect(),
        _ => {
            return Err(anyhow::anyhow!(
                "NIP-05 found, but no NIP-46 bunker relays defined for this user."
            ));
        }
    };

    if relays.is_empty() {
        return Err(anyhow::anyhow!(
            "NIP-05 found, but no NIP-46 bunker relays defined for this user."
        ));
    }

    info!(
        "NIP-05 resolved: pubkey={}, relays={:?}",
        pubkey.to_hex(),
        relays
    );

    // Build the bunker URI
    let relay_params: Vec<String> = relays.iter().map(|r| format!("relay={}", r)).collect();
    let bunker_uri_str = format!("bunker://{}?{}", pubkey.to_hex(), relay_params.join("&"));

    Ok(NostrConnectURI::parse(&bunker_uri_str)?)
}

/// Entry point for Flow B: Arcadestr generates a nostrconnect:// URI
/// displayed as a QR code. The user scans it with Amethyst or Amber on mobile.
///
/// Returns: (uri_string_for_qr, app_keys, secret_nonce)
///
/// IMPORTANT: The QR URI uses nostrconnect:// format (client-initiated),
/// NOT bunker:// (signer-initiated). This is the inverse of Flow A.
/// The app_keys and secret returned here MUST be kept alive and later passed to
/// `wait_for_qr_connection()` to complete the handshake.
///
/// # Arguments
/// * `permissions` - Optional permissions to request (e.g., ["sign_event:1", "nip44_encrypt"])
pub async fn generate_login_qr(
    permissions: Option<Vec<String>>,
) -> anyhow::Result<(String, Keys, String)> {
    info!("Generating login QR code (Flow B)...");

    // Generate a fresh ephemeral keypair for this QR session
    let app_keys = Keys::generate();
    info!(
        "Generated ephemeral keys for QR: pubkey={}",
        app_keys.public_key().to_hex()
    );

    // Generate cryptographically secure random secret nonce
    // This prevents replay attacks - each QR code is unique
    let mut secret_bytes = [0u8; 32];
    rand::Rng::fill_bytes(&mut rand::rng(), &mut secret_bytes);
    let secret = hex::encode(secret_bytes);
    info!("Generated secure nonce for QR session");

    // Use well-known reliable relays as the rendezvous point
    let relays = vec![
        "wss://relay.damus.io".to_string(),
        "wss://nos.lol".to_string(),
    ];

    // Build the nostrconnect:// URI using the proper constructor
    let relay_urls: Vec<RelayUrl> = relays.iter().filter_map(|r| r.parse().ok()).collect();

    // Use NostrConnectURI::client() to create a properly formatted URI
    let base_uri = NostrConnectURI::client(app_keys.public_key(), relay_urls, "Arcadestr");

    // Convert to string and add the secret parameter
    // The base_uri looks like: nostrconnect://<pubkey>?relay=...&name=...
    let base_uri_str = base_uri.to_string();
    let uri = if base_uri_str.contains('?') {
        format!("{}&secret={}", base_uri_str, secret)
    } else {
        format!("{}?secret={}", base_uri_str, secret)
    };

    info!("Generated nostrconnect URI for QR display: {}", uri);

    Ok((uri, app_keys, secret))
}

/// After the QR is scanned, wait for the signer to initiate the connection.
/// Call this immediately after generate_login_qr() returns.
///
/// This function:
/// 1. Connects to the relay(s) in the QR URI
/// 2. Subscribes to kind 24133 events where p tag = app_keys.public_key()
/// 3. Waits for a "connect" request from the signer
/// 4. Validates the secret nonce
/// 5. Sends back "ack" response
/// 6. Returns SavedProfile on successful connection
///
/// # Arguments
/// * `qr_uri_string` - The nostrconnect:// URI that was displayed as QR
/// * `app_keys` - The ephemeral keys generated by generate_login_qr
/// * `expected_secret` - The secret nonce to validate against
/// * `timeout_secs` - How long to wait for connection (default: 300 seconds)
pub async fn wait_for_qr_connection(
    qr_uri_string: &str,
    app_keys: Keys,
    expected_secret: String,
    timeout_secs: u64,
) -> anyhow::Result<SavedProfile> {
    use nostr::{Filter, Kind, Timestamp};
    use nostr_sdk::Client;
    use std::time::Duration;

    info!("Waiting for QR connection... timeout={}s", timeout_secs);

    // Parse the QR URI manually to extract relays and secret
    // The URI format is: nostrconnect://<pubkey>?relay=<url>&relay=<url>&name=<name>&secret=<secret>
    let uri_url =
        url::Url::parse(qr_uri_string).map_err(|e| anyhow::anyhow!("Invalid QR URI: {}", e))?;

    // Extract relays from query parameters
    let mut relays: Vec<RelayUrl> = Vec::new();
    for (key, value) in uri_url.query_pairs() {
        if key == "relay" {
            if let Ok(relay_url) = value.parse::<RelayUrl>() {
                relays.push(relay_url);
            }
        }
    }

    if relays.is_empty() {
        // Fallback to default relays if none found
        relays.push("wss://relay.damus.io".parse().unwrap());
        relays.push("wss://nos.lol".parse().unwrap());
    }

    info!("Using {} relays for QR connection", relays.len());

    // Create a temporary client to listen for the signer's connect request
    let client = Client::new(app_keys.clone());

    // Add all relays from the URI
    for relay in &relays {
        info!("Adding relay: {}", relay);
        client.add_relay(relay).await?;
    }

    // Connect to relays
    client.connect().await;
    info!("Connected to relays, waiting for signer...");

    // Create subscription filter for kind 24133 events where p tag = our pubkey
    let filter = Filter::new()
        .kind(Kind::NostrConnect)
        .pubkey(app_keys.public_key())
        .since(Timestamp::now());

    // Subscribe to events
    let _ = client.subscribe(filter, None).await;
    info!("Subscribed to NIP-46 connect events");

    // Wait for connection with timeout
    let timeout = Duration::from_secs(timeout_secs);
    let start_time = std::time::Instant::now();

    // Get notifications from client
    let mut notifications = client.notifications();

    loop {
        // Check for timeout
        if start_time.elapsed() > timeout {
            client.disconnect().await;
            anyhow::bail!(
                "QR connection timeout - no signer connected within {} seconds",
                timeout_secs
            );
        }

        // Try to get next notification with a short timeout
        match tokio::time::timeout(Duration::from_millis(500), notifications.recv()).await {
            Ok(Ok(notification)) => {
                info!("Received notification");

                // Check if this is a NIP-46 event
                if let nostr_sdk::RelayPoolNotification::Event { event, .. } = notification {
                    if event.kind == Kind::NostrConnect {
                        info!("Received NIP-46 event from {}", event.pubkey);

                        // Try to decrypt and validate the connect request
                        match handle_nip46_connect_event(&event, &app_keys, &expected_secret).await
                        {
                            Ok((bunker_pubkey, request_id)) => {
                                info!(
                                    "Successfully validated connect request from bunker {} with id: {}",
                                    bunker_pubkey, request_id
                                );

                                // Send ack response with matching request id
                                if let Err(e) =
                                    send_connect_ack(&client, &app_keys, event.pubkey, &relays, &request_id)
                                        .await
                                {
                                    warn!("Failed to send ack: {}", e);
                                }

                                // IMPORTANT: event.pubkey is the BUNKER's ephemeral key, NOT the user's identity key
                                // We need to perform NIP-46 handshake to get the actual user's pubkey
                                info!("Performing NIP-46 handshake to get user's identity pubkey...");
                                
                                // Create bunker URI with the bunker's pubkey
                                let relay_params: Vec<String> = relays
                                    .iter()
                                    .map(|r| format!("relay={}", r))
                                    .collect();
                                let bunker_uri_str = format!(
                                    "bunker://{}?{}",
                                    bunker_pubkey.to_hex(),
                                    relay_params.join("&")
                                );
                                let bunker_uri = NostrConnectURI::parse(&bunker_uri_str)?;
                                
                                // Create NostrConnect client to perform handshake
                                let signer = NostrConnect::new(
                                    bunker_uri.clone(),
                                    app_keys.clone(),
                                    Duration::from_secs(30),
                                    None,
                                )?;
                                
                                // Get the actual user's identity pubkey (not the bunker's key)
                                let user_pubkey = signer.get_public_key().await?;
                                info!(
                                    "NIP-46 handshake complete! User identity pubkey: {}",
                                    user_pubkey.to_hex()
                                );

                                let profile = SavedProfile {
                                    id: uuid::Uuid::new_v4().to_string(),
                                    name: "QR Connected Account".to_string(),
                                    user_pubkey,
                                    bunker_uri,
                                    app_keys,
                                };

                                client.disconnect().await;
                                return Ok(profile);
                            }
                            Err(e) => {
                                warn!("Failed to handle connect event: {}", e);
                            }
                        }
                    }
                }
            }
            Ok(Err(_)) => {
                // Channel closed
                client.disconnect().await;
                anyhow::bail!("Notification channel closed");
            }
            Err(_) => {
                // Timeout - no events yet, continue polling
                continue;
            }
        }
    }
}

/// Handle a NIP-46 connect event
/// Decrypts the event content and validates the secret
/// Accepts both request format (method+params) and response format (id+result)
/// Returns the signer's public key and the request id (for JSON-RPC response correlation)
async fn handle_nip46_connect_event(
    event: &nostr::Event,
    app_keys: &Keys,
    expected_secret: &str,
) -> anyhow::Result<(PublicKey, String)> {
    use nostr::nips::nip44;

    info!("Handling NIP-46 connect event from {}", event.pubkey);

    // Try to decrypt the event content using NIP-44
    // nip44::decrypt takes (secret_key, public_key, ciphertext)
    let decrypted = match nip44::decrypt(app_keys.secret_key(), &event.pubkey, &event.content) {
        Ok(content) => content,
        Err(e) => {
            anyhow::bail!("Failed to decrypt NIP-46 event: {}", e);
        }
    };

    info!("Decrypted NIP-46 content: {}", decrypted);

    // Parse the JSON-RPC message
    let message: serde_json::Value = serde_json::from_str(&decrypted)
        .map_err(|e| anyhow::anyhow!("Failed to parse JSON-RPC message: {}", e))?;

    // Extract the request id for the response (JSON-RPC requires id matching)
    let request_id = message
        .get("id")
        .and_then(|id| id.as_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| {
            // If no id in request, generate one (shouldn't happen with compliant signers)
            warn!("No id field in NIP-46 connect request, generating random id");
            uuid::Uuid::new_v4().to_string()
        });
    info!("Extracted request id: {}", request_id);

    // Try to extract the secret from various formats
    let received_secret = if let Some(method) = message.get("method").and_then(|m| m.as_str()) {
        // Request format: {"id":"...","method":"connect","params":[secret]}
        if method == "connect" {
            let params = message
                .get("params")
                .and_then(|p| p.as_array())
                .ok_or_else(|| anyhow::anyhow!("Missing params in connect request"))?;

            if params.is_empty() {
                anyhow::bail!("Empty params in connect request");
            }

            params[0]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("Invalid secret format in params"))?
        } else {
            anyhow::bail!("Expected 'connect' method, got '{}'", method);
        }
    } else if let Some(result) = message.get("result") {
        // Response format: {"id":"...","result":secret}
        // Some signers send the secret in the result field
        result
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Invalid secret format in result"))?
    } else {
        anyhow::bail!("Missing method or result in NIP-46 message");
    };

    // Validate the secret
    if received_secret != expected_secret {
        anyhow::bail!("Secret mismatch - possible replay attack");
    }

    info!("Secret validated successfully");

    // Return the signer's public key and the request id
    Ok((event.pubkey, request_id))
}

/// Send a connect acknowledgment response
/// Uses the same id as the request for JSON-RPC correlation
async fn send_connect_ack(
    client: &nostr_sdk::Client,
    app_keys: &Keys,
    signer_pubkey: PublicKey,
    _relays: &[RelayUrl],
    request_id: &str,
) -> anyhow::Result<()> {
    use nostr::nips::nip44;
    use nostr::{EventBuilder, Tag};

    info!("Sending connect ack to {} with request id: {}", signer_pubkey, request_id);

    // Build the JSON-RPC response using the SAME id as the request
    // This is required by JSON-RPC 2.0 spec for proper correlation
    let response = serde_json::json!({
        "id": request_id,
        "result": "ack",
    });

    // Encrypt the response using NIP-44
    // nip44::encrypt takes (secret_key, public_key, plaintext, version)
    let encrypted = nip44::encrypt(
        app_keys.secret_key(),
        &signer_pubkey,
        &response.to_string(),
        nip44::Version::V2,
    )
    .map_err(|e| anyhow::anyhow!("Failed to encrypt response: {}", e))?;

    // Build the event
    let tags = vec![Tag::public_key(signer_pubkey)];

    let builder = EventBuilder::new(nostr::Kind::NostrConnect, encrypted).tags(tags);

    // Send to all relays
    client
        .send_event_builder(builder)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to send ack: {}", e))?;

    info!("Connect ack sent successfully with id: {}", request_id);
    Ok(())
}
