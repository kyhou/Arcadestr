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

    // Match Amethyst's dedicated NIP-46 rendezvous defaults, not feed relays.
    let relays = crate::relay_defaults::DEFAULT_NIP46_RELAYS;

    let mut uri = Url::parse(&format!(
        "nostrconnect://{}",
        app_keys.public_key().to_hex()
    ))?;
    {
        let mut query = uri.query_pairs_mut();
        query.append_pair("name", "Arcadestr");
        // Keep the older SDK metadata field alongside the current NIP-46 name.
        query.append_pair("metadata", r#"{"name":"Arcadestr"}"#);
        for relay in relays {
            query.append_pair("relay", relay);
        }
        query.append_pair("secret", &secret);
        if let Some(permissions) = permissions.filter(|permissions| !permissions.is_empty()) {
            query.append_pair("perms", &permissions.join(","));
        }
    }
    info!("Generated QR login URI (pairing secret omitted from logs)");
    Ok((uri.to_string(), app_keys, secret))
}

/// After the QR is scanned, wait for the signer to initiate the connection.
/// Call this immediately after generate_login_qr() returns.
///
/// This function:
/// 1. Connects to the relay(s) in the QR URI
/// 2. Subscribes to kind 24133 events where p tag = app_keys.public_key()
/// 3. Waits for a "connect" response from the signer
/// 4. Validates the secret nonce
/// 5. Requests the account public key over the already-authorized transport
/// 6. Returns the saved profile and connected signer client without re-pairing
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
) -> anyhow::Result<(SavedProfile, nostr_sdk::Client)> {
    super::QrLoginSession::start(qr_uri_string, app_keys, expected_secret, timeout_secs)
        .await?
        .wait()
        .await
}

/// Handle a NIP-46 connect event
/// Decrypts the event content and validates the secret
/// Accepts both request format (method+params) and response format (id+result)
/// Returns the remote transport key and an optional legacy request id to acknowledge.
pub(super) async fn handle_nip46_connect_event(
    event: &nostr::Event,
    app_keys: &Keys,
    expected_secret: &str,
) -> anyhow::Result<(PublicKey, Option<String>)> {
    use nostr::nips::nip44;

    anyhow::ensure!(
        event.kind == nostr::Kind::NostrConnect && event.verify().is_ok(),
        "Invalid QR approval event"
    );
    let recipient = app_keys.public_key().to_hex();
    anyhow::ensure!(
        event.tags.iter().any(|tag| {
            let fields = tag.as_slice();
            fields.len() >= 2 && fields[0] == "p" && fields[1] == recipient
        }),
        "QR approval recipient mismatch"
    );

    // Try to decrypt the event content using NIP-44
    // nip44::decrypt takes (secret_key, public_key, ciphertext)
    let decrypted = match nip44::decrypt(app_keys.secret_key(), &event.pubkey, &event.content) {
        Ok(content) => content,
        Err(e) => {
            anyhow::bail!("Failed to decrypt NIP-46 event: {}", e);
        }
    };

    // Parse the JSON-RPC message
    let message: serde_json::Value = serde_json::from_str(&decrypted)
        .map_err(|e| anyhow::anyhow!("Failed to parse JSON-RPC message: {}", e))?;

    // Extract the request id for the response (JSON-RPC requires id matching)
    let request_id = message
        .get("id")
        .and_then(|id| id.as_str())
        .filter(|id| !id.is_empty())
        .ok_or_else(|| anyhow::anyhow!("QR approval requires a request id"))?;
    anyhow::ensure!(
        message.get("error").is_none_or(serde_json::Value::is_null),
        "Signer rejected QR approval"
    );

    // Try to extract the secret from various formats
    let received_secret = if let Some(method) = message.get("method") {
        let method = method
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Invalid QR request method"))?;
        anyhow::ensure!(
            message.get("result").is_none(),
            "QR approval cannot be both request and response"
        );
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
    let acknowledgement = message
        .get("method")
        .is_some()
        .then(|| request_id.to_string());
    Ok((event.pubkey, acknowledgement))
}

/// Send a connect acknowledgment response
/// Uses the same id as the request for JSON-RPC correlation
pub(super) async fn send_connect_ack(
    client: &nostr_sdk::Client,
    app_keys: &Keys,
    signer_pubkey: PublicKey,
    _relays: &[RelayUrl],
    request_id: &str,
) -> anyhow::Result<()> {
    use nostr::nips::nip44;
    use nostr::{EventBuilder, Tag};

    info!("Sending legacy QR connect acknowledgement");

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

    info!("Legacy QR connect acknowledgement sent");
    Ok(())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::generate_login_qr;
    use super::handle_nip46_connect_event;
    use crate::signers::{load_or_create_client_keys, reset_client_keys, set_keys_dir};
    use crate::test_helpers::nip46_mocks::MockNip46Relay;
    use nostr::{Event, EventBuilder, Keys, Kind, SecretKey};

    fn fixed_keys(hex_secret: &str) -> Keys {
        let secret = SecretKey::from_hex(hex_secret).expect("test secret key hex must be valid");
        Keys::new(secret)
    }

    fn app_keys() -> Keys {
        fixed_keys("0000000000000000000000000000000000000000000000000000000000000001")
    }

    fn signer_keys() -> Keys {
        fixed_keys("0000000000000000000000000000000000000000000000000000000000000002")
    }

    fn user_keys() -> Keys {
        fixed_keys("0000000000000000000000000000000000000000000000000000000000000003")
    }

    fn extract_client_pubkey_hex(uri: &str) -> Option<&str> {
        let rest = uri.strip_prefix("nostrconnect://")?;
        let end = rest.find('?').unwrap_or(rest.len());
        Some(&rest[..end])
    }

    #[tokio::test]
    async fn nostrconnect_client_pubkey_not_derived_from_stored_key_material() {
        let temp_dir = tempfile::tempdir().expect("tempdir creation must succeed");
        set_keys_dir(temp_dir.path().to_path_buf());

        reset_client_keys().expect("reset_client_keys must succeed");
        let (stored_client_keys, _) =
            load_or_create_client_keys().expect("load_or_create_client_keys must succeed");

        let (uri, _app_keys, _secret) = generate_login_qr(None)
            .await
            .expect("generate_login_qr must succeed");

        let uri_pubkey_hex =
            extract_client_pubkey_hex(&uri).expect("nostrconnect URI must include client pubkey");

        assert_ne!(
            uri_pubkey_hex,
            stored_client_keys.public_key().to_hex(),
            "QR nostrconnect client pubkey must not be derived from persisted key material"
        );
    }

    #[test]
    fn request_event_uses_kind_24133() {
        let app = app_keys();
        let signer = signer_keys();

        let request = MockNip46Relay::build_client_request_event(
            &app,
            signer.public_key(),
            "connect",
            json!(["secret-123"]),
            "req-06",
        );

        assert_eq!(request.kind, Kind::NostrConnect);
    }

    #[test]
    fn request_payload_is_nip44_encrypted() {
        let app = app_keys();
        let signer = signer_keys();

        let request = MockNip46Relay::build_client_request_event(
            &app,
            signer.public_key(),
            "connect",
            json!(["secret-123"]),
            "req-07",
        );

        assert!(!request.content.contains("\"method\""));
        assert!(!request.content.contains("connect"));

        let payload = MockNip46Relay::decrypt_client_request(&signer, &request)
            .expect("mock relay must decrypt request payload");
        assert_eq!(payload["method"], "connect");
    }

    #[test]
    fn request_payload_json_rpc_structure() {
        let app = app_keys();
        let signer = signer_keys();

        let request = MockNip46Relay::build_client_request_event(
            &app,
            signer.public_key(),
            "connect",
            json!(["secret-123"]),
            "req-08",
        );

        let payload = MockNip46Relay::decrypt_client_request(&signer, &request)
            .expect("mock relay must decrypt request payload");
        assert!(payload.get("id").is_some());
        assert!(payload.get("method").is_some());
        assert!(payload.get("params").is_some());
    }

    #[test]
    fn response_payload_json_rpc_structure() {
        let app = app_keys();
        let signer = signer_keys();
        let user = user_keys();
        let mut relay = MockNip46Relay::new(signer.clone(), user.clone());
        relay.set_expected_secret("secret-123");

        let request = MockNip46Relay::build_client_request_event(
            &app,
            signer.public_key(),
            "connect",
            json!(["secret-123"]),
            "req-09",
        );

        let response = relay
            .process_client_event(&request)
            .expect("mock relay must process request");
        let payload = MockNip46Relay::decrypt_relay_response(&app, &response)
            .expect("app must decrypt relay response");

        assert!(payload.get("id").is_some());
        assert!(payload.get("result").is_some());
        assert!(payload.get("error").is_some());
    }

    #[test]
    fn connect_method_returns_ack() {
        let app = app_keys();
        let signer = signer_keys();
        let user = user_keys();
        let mut relay = MockNip46Relay::new(signer.clone(), user.clone());
        relay.set_expected_secret("secret-123");

        let request = MockNip46Relay::build_client_request_event(
            &app,
            signer.public_key(),
            "connect",
            json!(["secret-123"]),
            "req-10",
        );

        let response = relay
            .process_client_event(&request)
            .expect("connect request must succeed");
        let payload = MockNip46Relay::decrypt_relay_response(&app, &response)
            .expect("app must decrypt connect response");
        assert_eq!(payload["result"], "secret-123");
    }

    #[test]
    fn get_public_key_after_connect() {
        let app = app_keys();
        let signer = signer_keys();
        let user = user_keys();
        let mut relay = MockNip46Relay::new(signer.clone(), user.clone());
        relay.set_expected_secret("secret-123");

        let connect = MockNip46Relay::build_client_request_event(
            &app,
            signer.public_key(),
            "connect",
            json!(["secret-123"]),
            "req-11a",
        );
        relay
            .process_client_event(&connect)
            .expect("connect request must succeed");

        let get_pubkey = MockNip46Relay::build_client_request_event(
            &app,
            signer.public_key(),
            "get_public_key",
            json!([]),
            "req-11b",
        );
        let response = relay
            .process_client_event(&get_pubkey)
            .expect("get_public_key request must succeed");
        let payload = MockNip46Relay::decrypt_relay_response(&app, &response)
            .expect("app must decrypt get_public_key response");

        let pubkey_hex = payload["result"]
            .as_str()
            .expect("result must be pubkey hex string");
        assert_eq!(pubkey_hex, user.public_key().to_hex());
        assert_eq!(pubkey_hex.len(), 64);
    }

    #[test]
    fn sign_event_returns_valid_signature() {
        let app = app_keys();
        let signer = signer_keys();
        let user = user_keys();
        let mut relay = MockNip46Relay::new(signer.clone(), user.clone());
        relay.set_expected_secret("secret-123");

        let connect = MockNip46Relay::build_client_request_event(
            &app,
            signer.public_key(),
            "connect",
            json!(["secret-123"]),
            "req-12a",
        );
        relay
            .process_client_event(&connect)
            .expect("connect request must succeed");

        let unsigned = EventBuilder::new(Kind::TextNote, "hello-sign").build(user.public_key());
        let sign_request = MockNip46Relay::build_client_request_event(
            &app,
            signer.public_key(),
            "sign_event",
            json!([unsigned]),
            "req-12b",
        );

        let response = relay
            .process_client_event(&sign_request)
            .expect("sign_event request must succeed");
        let payload = MockNip46Relay::decrypt_relay_response(&app, &response)
            .expect("app must decrypt sign_event response");

        let signed: Event = serde_json::from_value(payload["result"].clone())
            .expect("result must be signed event JSON");
        assert!(signed.verify().is_ok());
    }

    #[tokio::test]
    async fn wrong_secret_rejected() {
        let app = app_keys();
        let signer = signer_keys();

        let connect_event = MockNip46Relay::build_signer_connect_event(
            &signer,
            app.public_key(),
            "wrong-secret",
            "req-15",
        );

        let err = handle_nip46_connect_event(&connect_event, &app, "expected-secret")
            .await
            .expect_err("mismatched secret must be rejected");

        assert!(err.to_string().contains("Secret mismatch"));
    }

    fn qr_approval(payload: serde_json::Value, recipient: nostr::PublicKey) -> Event {
        let signer = signer_keys();
        let encrypted = nostr::nips::nip44::encrypt(
            signer.secret_key(),
            &app_keys().public_key(),
            payload.to_string(),
            nostr::nips::nip44::Version::V2,
        )
        .expect("encrypt approval");
        EventBuilder::new(Kind::NostrConnect, encrypted)
            .tags([nostr::Tag::public_key(recipient)])
            .sign_with_keys(&signer)
            .expect("signed approval")
    }

    #[tokio::test]
    async fn standard_qr_response_does_not_require_an_acknowledgement() {
        let app = app_keys();
        let event = qr_approval(
            json!({"id":"approval-id","result":"test-secret","error":null}),
            app.public_key(),
        );
        let (remote, acknowledgement) = handle_nip46_connect_event(&event, &app, "test-secret")
            .await
            .expect("valid approval");
        assert_eq!(remote, signer_keys().public_key());
        assert!(acknowledgement.is_none());
    }

    #[tokio::test]
    async fn malformed_or_untrusted_qr_approvals_are_rejected() {
        let app = app_keys();
        for payload in [
            json!({"id":"approval","result":"wrong-secret"}),
            json!({"id":"approval","result":"test-secret","error":"denied"}),
            json!({"result":"test-secret"}),
            json!({"id":42,"result":"test-secret"}),
            json!({"id":"approval","method":42,"result":"test-secret"}),
            json!({"id":"approval","method":"connect","params":["test-secret"],"result":"test-secret"}),
        ] {
            let event = qr_approval(payload, app.public_key());
            assert!(handle_nip46_connect_event(&event, &app, "test-secret")
                .await
                .is_err());
        }
        let payload = json!({"id":"approval","result":"test-secret"});
        let wrong_recipient = qr_approval(payload.clone(), user_keys().public_key());
        assert!(
            handle_nip46_connect_event(&wrong_recipient, &app, "test-secret")
                .await
                .is_err()
        );
        let mut forged = qr_approval(payload, app.public_key());
        forged.content.push('x');
        assert!(handle_nip46_connect_event(&forged, &app, "test-secret")
            .await
            .is_err());
    }

    #[tokio::test]
    async fn qr_uri_encodes_metadata_permissions_and_secret_as_query_parameters() {
        let (uri, keys, secret) =
            generate_login_qr(Some(vec!["sign_event:1".into(), "nip44_encrypt".into()]))
                .await
                .expect("QR URI");
        let parsed = url::Url::parse(&uri).expect("encoded URI");
        let values: std::collections::HashMap<_, _> = parsed.query_pairs().into_owned().collect();
        assert_eq!(parsed.host_str(), Some(keys.public_key().to_hex().as_str()));
        assert_eq!(values["name"], "Arcadestr");
        assert_eq!(values["secret"], secret);
        assert_eq!(values["perms"], "sign_event:1,nip44_encrypt");
        let relays = parsed
            .query_pairs()
            .filter(|(key, _)| key == "relay")
            .map(|(_, value)| value.into_owned())
            .collect::<Vec<_>>();
        assert_eq!(relays, crate::relay_defaults::DEFAULT_NIP46_RELAYS);
        assert!(uri.contains("relay=wss%3A%2F%2F"));
    }

    #[test]
    fn ping_returns_pong() {
        let app = app_keys();
        let signer = signer_keys();
        let user = user_keys();
        let mut relay = MockNip46Relay::new(signer.clone(), user.clone());

        let ping = MockNip46Relay::build_client_request_event(
            &app,
            signer.public_key(),
            "ping",
            json!([]),
            "req-16",
        );

        let response = relay
            .process_client_event(&ping)
            .expect("ping request must succeed");
        let payload = MockNip46Relay::decrypt_relay_response(&app, &response)
            .expect("app must decrypt ping response");
        assert_eq!(payload["result"], "pong");
    }
}
