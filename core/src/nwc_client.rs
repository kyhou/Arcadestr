//! NIP-47 wallet connection helpers for ADP purchases.

use serde::{Deserialize, Serialize};
use thiserror::Error;
use url::Url;

use nostr::nips::nip44;
use nostr::{Event, EventBuilder, EventId, Filter, Keys, Kind, PublicKey, SecretKey, Tag, TagKind};
use nostr_sdk::{Client, RelayPoolNotification};
use std::time::{Duration, Instant};

const NWC_KEYRING_SERVICE: &str = "arcadestr-nwc";
const DEFAULT_NWC_KEYRING_ENTRY: &str = "default-wallet-connect-uri";

const NWC_SCHEME: &str = "nostr+walletconnect";
const SECRET_HEX_LEN: usize = 64;

/// Errors returned by NWC connection and request helpers.
#[derive(Debug, Error)]
pub enum NwcClientError {
    #[error("invalid NWC URI")]
    InvalidUri,
    #[error("NWC URI is missing at least one relay")]
    MissingRelay,
    #[error("NWC URI is missing secret")]
    MissingSecret,
    #[error("NWC wallet pubkey must be 32-byte hex")]
    InvalidWalletPubkey,
    #[error("NWC secret must be 32-byte hex")]
    InvalidSecret,
    #[error("failed to serialize NWC request: {0}")]
    Json(#[from] serde_json::Error),
    #[error("NWC wallet error {code}: {message}")]
    WalletError { code: String, message: String },
    #[error("unexpected NWC response type: {0}")]
    UnexpectedResponseType(String),
    #[error("NWC response missing pay_invoice result")]
    MissingResult,
    #[error("NWC crypto or event error: {0}")]
    Nostr(String),
    #[error("unexpected NWC response event")]
    UnexpectedResponseEvent,
    #[error("NWC wallet did not respond before timeout")]
    WalletTimeout,
    #[error("NWC relay error: {0}")]
    Relay(String),
    #[error("NWC keyring error: {0}")]
    Keyring(String),
}

/// Parsed Nostr Wallet Connect URI without exposing the secret in Debug output.
#[derive(Clone, Eq, PartialEq)]
pub struct NwcConnection {
    wallet_pubkey_hex: String,
    relay_urls: Vec<String>,
    secret_hex: String,
    lud16: Option<String>,
}

/// Stores the default NWC connection URI in the OS keyring.
///
/// # Errors
/// Returns an error if the URI is invalid or keyring storage fails.
pub fn save_default_nwc_connection(uri: &str) -> Result<NwcConnection, NwcClientError> {
    let connection = NwcConnection::parse(uri)?;
    let entry = keyring::Entry::new(NWC_KEYRING_SERVICE, DEFAULT_NWC_KEYRING_ENTRY)
        .map_err(|err| NwcClientError::Keyring(err.to_string()))?;
    entry
        .set_password(uri)
        .map_err(|err| NwcClientError::Keyring(err.to_string()))?;
    Ok(connection)
}

/// Loads the default NWC connection URI from the OS keyring.
///
/// # Errors
/// Returns an error if keyring access fails or the stored URI is invalid.
pub fn load_default_nwc_connection() -> Result<NwcConnection, NwcClientError> {
    let entry = keyring::Entry::new(NWC_KEYRING_SERVICE, DEFAULT_NWC_KEYRING_ENTRY)
        .map_err(|err| NwcClientError::Keyring(err.to_string()))?;
    let uri = entry
        .get_password()
        .map_err(|err| NwcClientError::Keyring(err.to_string()))?;
    NwcConnection::parse(&uri)
}

impl std::fmt::Debug for NwcConnection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NwcConnection")
            .field("wallet_pubkey_hex", &self.wallet_pubkey_hex)
            .field("relay_urls", &self.relay_urls)
            .field("secret_hex", &"<redacted>")
            .field("lud16", &self.lud16)
            .finish()
    }
}

impl NwcConnection {
    /// Parses a `nostr+walletconnect://` URI.
    ///
    /// # Errors
    /// Returns [`NwcClientError`] when the URI is malformed or misses NIP-47 fields.
    pub fn parse(uri: &str) -> Result<Self, NwcClientError> {
        let url = Url::parse(uri).map_err(|_| NwcClientError::InvalidUri)?;
        if url.scheme() != NWC_SCHEME {
            return Err(NwcClientError::InvalidUri);
        }

        let wallet_pubkey_hex = url
            .host_str()
            .ok_or(NwcClientError::InvalidWalletPubkey)?
            .to_string();
        validate_hex_32(&wallet_pubkey_hex).map_err(|_| NwcClientError::InvalidWalletPubkey)?;

        let mut relay_urls = Vec::new();
        let mut secret_hex = None;
        let mut lud16 = None;

        for (key, value) in url.query_pairs() {
            match key.as_ref() {
                "relay" => relay_urls.push(value.to_string()),
                "secret" => secret_hex = Some(value.to_string()),
                "lud16" => lud16 = Some(value.to_string()),
                _ => {}
            }
        }

        if relay_urls.is_empty() {
            return Err(NwcClientError::MissingRelay);
        }

        let secret_hex = secret_hex.ok_or(NwcClientError::MissingSecret)?;
        validate_hex_32(&secret_hex).map_err(|_| NwcClientError::InvalidSecret)?;

        Ok(Self {
            wallet_pubkey_hex,
            relay_urls,
            secret_hex,
            lud16,
        })
    }

    /// Returns the wallet service public key hex.
    pub fn wallet_pubkey_hex(&self) -> &str {
        &self.wallet_pubkey_hex
    }

    /// Returns relay URLs declared by the wallet connection.
    pub fn relay_urls(&self) -> &[String] {
        &self.relay_urls
    }

    /// Returns the optional wallet Lightning address.
    pub fn lud16(&self) -> Option<&str> {
        self.lud16.as_deref()
    }
}

/// Thin NIP-47 client used to pay ADP purchase invoices.
#[derive(Debug, Clone)]
pub struct NwcClient {
    connection: NwcConnection,
    timeout: Duration,
}

impl NwcClient {
    /// Creates a client with a default wallet response timeout.
    pub fn new(connection: NwcConnection) -> Self {
        Self {
            connection,
            timeout: Duration::from_secs(60),
        }
    }

    /// Overrides the wallet response timeout.
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Sends a NIP-47 `pay_invoice` request and waits for the matching response.
    ///
    /// # Errors
    /// Returns relay, timeout, wallet, encryption, or malformed response errors.
    pub async fn pay_invoice(&self, invoice: &str) -> Result<PayInvoiceResult, NwcClientError> {
        let secret_key = SecretKey::from_hex(&self.connection.secret_hex)
            .map_err(|err| NwcClientError::Nostr(err.to_string()))?;
        let keys = Keys::new(secret_key);
        let client = Client::new(keys.clone());

        for relay_url in self.connection.relay_urls() {
            client
                .add_relay(relay_url)
                .await
                .map_err(|err| NwcClientError::Relay(err.to_string()))?;
        }
        client.connect().await;

        let request = build_pay_invoice_request_event(&self.connection, invoice)?;
        let wallet_pubkey = PublicKey::from_hex(self.connection.wallet_pubkey_hex())
            .map_err(|err| NwcClientError::Nostr(err.to_string()))?;
        let filter = Filter::new()
            .kind(Kind::WalletConnectResponse)
            .author(wallet_pubkey)
            .pubkey(keys.public_key())
            .event(request.id);
        client
            .subscribe(filter, None)
            .await
            .map_err(|err| NwcClientError::Relay(err.to_string()))?;
        client
            .send_event(&request)
            .await
            .map_err(|err| NwcClientError::Relay(err.to_string()))?;

        let mut notifications = client.notifications();
        let start = Instant::now();
        loop {
            if start.elapsed() > self.timeout {
                client.disconnect().await;
                return Err(NwcClientError::WalletTimeout);
            }

            match tokio::time::timeout(Duration::from_millis(500), notifications.recv()).await {
                Ok(Ok(RelayPoolNotification::Event { event, .. })) => {
                    match parse_pay_invoice_response_event(&self.connection, &event, request.id) {
                        Ok(result) => {
                            client.disconnect().await;
                            return Ok(result);
                        }
                        Err(NwcClientError::UnexpectedResponseEvent) => continue,
                        Err(err) => {
                            client.disconnect().await;
                            return Err(err);
                        }
                    }
                }
                Ok(Ok(_)) => continue,
                Ok(Err(err)) => {
                    client.disconnect().await;
                    return Err(NwcClientError::Relay(err.to_string()));
                }
                Err(_) => continue,
            }
        }
    }
}

#[derive(Serialize)]
struct PayInvoiceRequest<'a> {
    method: &'static str,
    params: PayInvoiceParams<'a>,
}

#[derive(Serialize)]
struct PayInvoiceParams<'a> {
    invoice: &'a str,
}

/// Builds the decrypted NIP-47 `pay_invoice` request payload.
///
/// The `amount` override is intentionally omitted because LNURL fixes the
/// amount when requesting the BOLT-11 invoice for ADP purchases.
///
/// # Errors
/// Returns an error when JSON serialization fails.
pub fn build_pay_invoice_request_json(invoice: &str) -> Result<String, NwcClientError> {
    let request = PayInvoiceRequest {
        method: "pay_invoice",
        params: PayInvoiceParams { invoice },
    };
    serde_json::to_string(&request).map_err(NwcClientError::from)
}

/// Builds and signs a NIP-47 `pay_invoice` request event.
///
/// # Errors
/// Returns an error if URI keys are invalid, encryption fails, or signing fails.
pub fn build_pay_invoice_request_event(
    connection: &NwcConnection,
    invoice: &str,
) -> Result<Event, NwcClientError> {
    let wallet_pubkey = PublicKey::from_hex(connection.wallet_pubkey_hex())
        .map_err(|err| NwcClientError::Nostr(err.to_string()))?;
    let secret_key = SecretKey::from_hex(&connection.secret_hex)
        .map_err(|err| NwcClientError::Nostr(err.to_string()))?;
    let keys = Keys::new(secret_key);
    let plaintext = build_pay_invoice_request_json(invoice)?;
    let encrypted = nip44::encrypt(
        keys.secret_key(),
        &wallet_pubkey,
        plaintext,
        nip44::Version::V2,
    )
    .map_err(|err| NwcClientError::Nostr(err.to_string()))?;

    EventBuilder::new(Kind::WalletConnectRequest, encrypted)
        .tags([
            Tag::custom(TagKind::custom("encryption"), ["nip44_v2"]),
            Tag::public_key(wallet_pubkey),
        ])
        .sign_with_keys(&keys)
        .map_err(|err| NwcClientError::Nostr(err.to_string()))
}

/// Successful NIP-47 `pay_invoice` result.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PayInvoiceResult {
    pub preimage: String,
    pub fees_paid_msat: Option<u64>,
}

#[derive(Deserialize)]
struct PayInvoiceResponseEnvelope {
    result_type: String,
    error: Option<WalletErrorJson>,
    result: Option<PayInvoiceResponseResult>,
}

#[derive(Deserialize)]
struct WalletErrorJson {
    code: String,
    message: String,
}

#[derive(Deserialize)]
struct PayInvoiceResponseResult {
    preimage: String,
    fees_paid: Option<u64>,
}

/// Parses decrypted NIP-47 `pay_invoice` response JSON.
///
/// # Errors
/// Returns wallet errors and malformed/unexpected response errors.
pub fn parse_pay_invoice_response_json(json: &str) -> Result<PayInvoiceResult, NwcClientError> {
    let envelope: PayInvoiceResponseEnvelope = serde_json::from_str(json)?;
    if envelope.result_type != "pay_invoice" {
        return Err(NwcClientError::UnexpectedResponseType(envelope.result_type));
    }

    if let Some(error) = envelope.error {
        return Err(NwcClientError::WalletError {
            code: error.code,
            message: error.message,
        });
    }

    let result = envelope.result.ok_or(NwcClientError::MissingResult)?;
    Ok(PayInvoiceResult {
        preimage: result.preimage,
        fees_paid_msat: result.fees_paid,
    })
}

/// Validates, decrypts, and parses a NIP-47 `pay_invoice` response event.
///
/// # Errors
/// Returns an error for invalid signatures, non-correlated responses, or decrypt failures.
pub fn parse_pay_invoice_response_event(
    connection: &NwcConnection,
    event: &Event,
    request_id: EventId,
) -> Result<PayInvoiceResult, NwcClientError> {
    event
        .verify()
        .map_err(|err| NwcClientError::Nostr(err.to_string()))?;

    let wallet_pubkey = PublicKey::from_hex(connection.wallet_pubkey_hex())
        .map_err(|err| NwcClientError::Nostr(err.to_string()))?;
    let client_secret = SecretKey::from_hex(&connection.secret_hex)
        .map_err(|err| NwcClientError::Nostr(err.to_string()))?;
    let client_keys = Keys::new(client_secret);

    if event.kind != Kind::WalletConnectResponse || event.pubkey != wallet_pubkey {
        return Err(NwcClientError::UnexpectedResponseEvent);
    }

    let expected_client_pubkey = client_keys.public_key().to_hex();
    let expected_request_id = request_id.to_hex();
    let has_client_p = event.tags.iter().any(|tag| {
        let values = tag.as_slice();
        values.len() >= 2 && values[0] == "p" && values[1] == expected_client_pubkey
    });
    let has_request_e = event.tags.iter().any(|tag| {
        let values = tag.as_slice();
        values.len() >= 2 && values[0] == "e" && values[1] == expected_request_id
    });
    if !has_client_p || !has_request_e {
        return Err(NwcClientError::UnexpectedResponseEvent);
    }

    let decrypted = nip44::decrypt(client_keys.secret_key(), &wallet_pubkey, &event.content)
        .map_err(|err| NwcClientError::Nostr(err.to_string()))?;
    parse_pay_invoice_response_json(&decrypted)
}

fn validate_hex_32(value: &str) -> Result<(), ()> {
    if value.len() != SECRET_HEX_LEN || !value.chars().all(|ch| ch.is_ascii_hexdigit()) {
        return Err(());
    }
    Ok(())
}
