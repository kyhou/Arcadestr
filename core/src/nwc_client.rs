//! NIP-47 wallet connection helpers for ADP purchases.

use serde::{Deserialize, Serialize};
use thiserror::Error;
use url::Url;

use nostr::nips::nip44;
use nostr::{Event, EventBuilder, EventId, Filter, Keys, Kind, PublicKey, SecretKey, Tag, TagKind};
use nostr_sdk::{Client, RelayPoolNotification};
use std::time::Duration;

const NWC_KEYRING_SERVICE: &str = "arcadestr-nwc";

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
    #[error("no NWC wallet configured")]
    NotConfigured,
    #[error("invalid or insecure NWC relay URL")]
    InvalidRelay,
    #[error("wallet verification is unsupported")]
    VerificationUnsupported,
    #[error("wallet lookup returned an invalid or mismatched outgoing payment")]
    InvalidLookup,
}

/// Parsed Nostr Wallet Connect URI without exposing the secret in Debug output.
#[derive(Clone, Eq, PartialEq)]
pub struct NwcConnection {
    wallet_pubkey_hex: String,
    relay_urls: Vec<String>,
    secret_hex: String,
    lud16: Option<String>,
}

/// Missing configuration is normal; other keyring failures remain errors.
pub fn get_nwc_connection(account: &PublicKey) -> Result<Option<NwcConnection>, NwcClientError> {
    let entry = account_nwc_entry(account)?;
    load_nwc_entry(&entry)
}

pub fn remove_nwc_connection(account: &PublicKey) -> Result<(), NwcClientError> {
    remove_nwc_entry(&account_nwc_entry(account)?)
}

fn account_nwc_entry_name(account: &PublicKey) -> String {
    format!("wallet-connect-uri:{}", account.to_hex())
}

fn account_nwc_entry(account: &PublicKey) -> Result<keyring::Entry, NwcClientError> {
    keyring::Entry::new(NWC_KEYRING_SERVICE, &account_nwc_entry_name(account))
        .map_err(|_| NwcClientError::Keyring("credential store unavailable".into()))
}

fn load_nwc_entry(entry: &keyring::Entry) -> Result<Option<NwcConnection>, NwcClientError> {
    match entry.get_password() {
        Ok(uri) => NwcConnection::parse(&uri).map(Some),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(_) => Err(NwcClientError::Keyring(
            "credential could not be read".into(),
        )),
    }
}

fn save_nwc_entry(entry: &keyring::Entry, uri: &str) -> Result<NwcConnection, NwcClientError> {
    let connection = NwcConnection::parse(uri)?;
    entry
        .set_password(uri)
        .map_err(|_| NwcClientError::Keyring("credential could not be saved".into()))?;
    Ok(connection)
}

fn remove_nwc_entry(entry: &keyring::Entry) -> Result<(), NwcClientError> {
    match entry.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(_) => Err(NwcClientError::Keyring(
            "credential could not be removed".into(),
        )),
    }
}

/// Validate structurally, never by string prefix. Marketplace policy is unrelated.
fn validate_relay(value: &str) -> Result<(), NwcClientError> {
    let url = Url::parse(value).map_err(|_| NwcClientError::InvalidRelay)?;
    let loopback = match url.host() {
        Some(url::Host::Domain(host)) => host.eq_ignore_ascii_case("localhost"),
        Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
        Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
        None => false,
    };
    if url.host().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
        || !(url.scheme() == "wss" || (cfg!(debug_assertions) && loopback && url.scheme() == "ws"))
    {
        return Err(NwcClientError::InvalidRelay);
    }
    Ok(())
}

/// Stores this account's NWC connection URI in the OS keyring. No legacy fallback.
///
/// # Errors
/// Returns an error if the URI is invalid or keyring storage fails.
pub fn save_nwc_connection(
    account: &PublicKey,
    uri: &str,
) -> Result<NwcConnection, NwcClientError> {
    save_nwc_entry(&account_nwc_entry(account)?, uri)
}

/// Loads this account's NWC connection URI from the OS keyring.
///
/// # Errors
/// Returns an error if keyring access fails or the stored URI is invalid.
pub fn load_nwc_connection(account: &PublicKey) -> Result<NwcConnection, NwcClientError> {
    get_nwc_connection(account)?.ok_or(NwcClientError::NotConfigured)
}

impl std::fmt::Debug for NwcConnection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NwcConnection")
            .field("wallet_pubkey_hex", &self.wallet_pubkey_hex)
            .field("relay_urls", &self.safe_relay_metadata())
            .field("secret_hex", &"<redacted>")
            .field("lud16", &self.lud16())
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
        if url.scheme() != NWC_SCHEME
            || !url.username().is_empty()
            || url.password().is_some()
            || url.port().is_some()
            || !matches!(url.path(), "" | "/")
            || url.fragment().is_some()
        {
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

        for relay in &relay_urls {
            validate_relay(relay)?;
        }
        let secret_hex = secret_hex.ok_or(NwcClientError::MissingSecret)?;
        validate_hex_32(&secret_hex).map_err(|_| NwcClientError::InvalidSecret)?;
        SecretKey::from_hex(&secret_hex).map_err(|_| NwcClientError::InvalidSecret)?;
        PublicKey::from_hex(&wallet_pubkey_hex).map_err(|_| NwcClientError::InvalidWalletPubkey)?;

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

    /// Origins only: relay paths and query strings may contain authentication tokens.
    pub fn safe_relay_metadata(&self) -> Vec<String> {
        self.relay_urls
            .iter()
            .filter_map(|value| Url::parse(value).ok())
            .map(|url| url.origin().ascii_serialization())
            .filter(|origin| {
                !origin
                    .to_ascii_lowercase()
                    .contains(&self.secret_hex.to_ascii_lowercase())
            })
            .collect()
    }

    /// Returns relay URLs declared by the wallet connection.
    pub fn relay_urls(&self) -> &[String] {
        &self.relay_urls
    }

    /// Returns the optional wallet Lightning address.
    pub fn lud16(&self) -> Option<&str> {
        self.lud16.as_deref().filter(|value| {
            !value
                .to_ascii_lowercase()
                .contains(&self.secret_hex.to_ascii_lowercase())
                && !value.contains("://")
                && value.contains('@')
                && value.len() <= 320
                && !value.chars().any(char::is_control)
        })
    }
}

/// Thin NIP-47 client used to pay ADP purchase invoices.
#[derive(Debug, Clone)]
pub struct NwcClient {
    connection: NwcConnection,
    timeout: Duration,
}

/// A debug-build laboratory transport, never a replacement wallet connection.
/// Release builds do not read the override. Invalid overrides fail closed.
#[cfg(debug_assertions)]
fn regtest_transport_relays(
    connection: &NwcConnection,
    proxy: Option<&str>,
    opted_in: bool,
) -> Result<Vec<String>, NwcClientError> {
    let Some(proxy) = proxy else {
        return Ok(connection.relay_urls().to_vec());
    };
    let local = |value: &str| {
        Url::parse(value).ok().is_some_and(|url| {
            url.scheme() == "ws"
                && matches!(url.host_str(), Some("127.0.0.1" | "localhost" | "[::1]"))
                && url.username().is_empty()
                && url.password().is_none()
                && url.query().is_none()
                && url.fragment().is_none()
        })
    };
    if !opted_in || !local(proxy) || !connection.relay_urls().iter().all(|url| local(url)) {
        return Err(NwcClientError::InvalidRelay);
    }
    Ok(vec![proxy.to_owned()])
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

    /// Public wallet/application identity. Never includes the NWC secret or URI.
    pub fn payment_identity(&self) -> Result<String, NwcClientError> {
        let secret = SecretKey::from_hex(&self.connection.secret_hex)
            .map_err(|_| NwcClientError::InvalidSecret)?;
        Ok(format!(
            "{}:{}",
            self.connection.wallet_pubkey_hex(),
            Keys::new(secret).public_key()
        ))
    }

    /// Reconcile an outgoing payment without ever dispatching `pay_invoice`.
    pub async fn lookup_payment(
        &self,
        payment_hash: &str,
        amount_msat: u64,
    ) -> Result<Option<PayInvoiceResult>, NwcClientError> {
        validate_hex_32(payment_hash).map_err(|_| NwcClientError::InvalidLookup)?;
        let response = self
            .request(
                serde_json::json!({"method":"lookup_invoice",
            "params":{"payment_hash":payment_hash}})
                .to_string(),
            )
            .await?;
        parse_lookup_payment_response(&response, payment_hash, amount_msat)
    }

    /// Sends a NIP-47 `pay_invoice` request and waits for the matching response.
    ///
    /// # Errors
    /// Returns relay, timeout, wallet, encryption, or malformed response errors.
    pub async fn pay_invoice(&self, invoice: &str) -> Result<PayInvoiceResult, NwcClientError> {
        let json = self
            .request(build_pay_invoice_request_json(invoice)?)
            .await?;
        parse_pay_invoice_response_json(&json)
    }

    /// Standard read-only NIP-47 capability discovery; no invented fee fields.
    pub async fn capabilities(
        &self,
    ) -> Result<crate::purchase_contract::WalletCapabilities, NwcClientError> {
        let json = self
            .request(r#"{"method":"get_info","params":{}}"#.into())
            .await?;
        parse_wallet_capabilities(&json)
    }

    /// Read-only connection verification never grants purchase authority.
    pub async fn verify(&self) -> Result<(), NwcClientError> {
        self.capabilities().await?;
        Ok(())
    }

    async fn request(&self, plaintext: String) -> Result<String, NwcClientError> {
        let secret_key = SecretKey::from_hex(&self.connection.secret_hex)
            .map_err(|_| NwcClientError::InvalidSecret)?;
        let keys = Keys::new(secret_key);
        let request = build_request_event(&self.connection, plaintext)?;
        let wallet_pubkey = PublicKey::from_hex(self.connection.wallet_pubkey_hex())
            .map_err(|_| NwcClientError::InvalidWalletPubkey)?;
        let client = Client::new(keys.clone());
        // Subscribe to notifications before sending: fast local wallets can reply immediately.
        let mut notifications = client.notifications();
        #[cfg(debug_assertions)]
        let transport_relays = regtest_transport_relays(
            &self.connection,
            std::env::var("ARCADESTR_REGTEST_NWC_RELAY_PROXY")
                .ok()
                .as_deref(),
            std::env::var("ARCADESTR_REGTEST_ALLOW_LOOPBACK_HTTP").as_deref() == Ok("true"),
        )?;
        #[cfg(not(debug_assertions))]
        let transport_relays = self.connection.relay_urls().to_vec();
        let exchange = async {
            for relay_url in &transport_relays {
                client
                    .add_relay(relay_url)
                    .await
                    .map_err(|_| NwcClientError::Relay("relay unavailable".into()))?;
            }
            client.connect().await;
            let filter = Filter::new()
                .kind(Kind::WalletConnectResponse)
                .author(wallet_pubkey)
                .pubkey(keys.public_key())
                .event(request.id);
            client
                .subscribe(filter, None)
                .await
                .map_err(|_| NwcClientError::Relay("subscription failed".into()))?;
            client
                .send_event(&request)
                .await
                .map_err(|_| NwcClientError::Relay("request could not be sent".into()))?;
            loop {
                match notifications.recv().await {
                    Ok(RelayPoolNotification::Event { event, .. }) => {
                        match decrypt_response_event(&self.connection, &event, request.id) {
                            Err(NwcClientError::UnexpectedResponseEvent) => continue,
                            result => return result,
                        }
                    }
                    Ok(_) => continue,
                    Err(_) => return Err(NwcClientError::Relay("relay disconnected".into())),
                }
            }
        };
        let result = tokio::time::timeout(self.timeout, exchange)
            .await
            .unwrap_or(Err(NwcClientError::WalletTimeout));
        client.disconnect().await;
        result
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
    build_request_event(connection, build_pay_invoice_request_json(invoice)?)
}

fn build_request_event(
    connection: &NwcConnection,
    plaintext: String,
) -> Result<Event, NwcClientError> {
    let wallet_pubkey = PublicKey::from_hex(connection.wallet_pubkey_hex())
        .map_err(|err| NwcClientError::Nostr(err.to_string()))?;
    let secret_key = SecretKey::from_hex(&connection.secret_hex)
        .map_err(|err| NwcClientError::Nostr(err.to_string()))?;
    let keys = Keys::new(secret_key);
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
#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct PayInvoiceResult {
    pub preimage: String,
    pub fees_paid_msat: Option<u64>,
}

impl std::fmt::Debug for PayInvoiceResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PayInvoiceResult")
            .field("preimage", &"<redacted>")
            .field("fees_paid_msat", &self.fees_paid_msat)
            .finish()
    }
}

/// NOT_FOUND, pending, failed and expired are all unresolved, never permission to pay again.
pub fn parse_lookup_payment_response(
    json: &str,
    payment_hash: &str,
    amount_msat: u64,
) -> Result<Option<PayInvoiceResult>, NwcClientError> {
    let value: serde_json::Value = serde_json::from_str(json)?;
    if value["result_type"] != "lookup_invoice" {
        return Err(NwcClientError::InvalidLookup);
    }
    if !value["error"].is_null() {
        return if value["error"]["code"] == "NOT_FOUND" {
            Ok(None)
        } else {
            Err(NwcClientError::InvalidLookup)
        };
    }
    let result = &value["result"];
    if result["type"] != "outgoing"
        || result["payment_hash"].as_str() != Some(payment_hash)
        || result["amount"].as_u64() != Some(amount_msat)
    {
        return Err(NwcClientError::InvalidLookup);
    }
    let settled = match result["state"].as_str() {
        Some("settled") => true,
        Some("pending" | "accepted" | "failed" | "expired") => false,
        None if result["state"].is_null() => result["settled_at"].as_u64().is_some_and(|v| v > 0),
        _ => return Err(NwcClientError::InvalidLookup),
    };
    if !settled {
        return Ok(None);
    }
    let preimage = result["preimage"]
        .as_str()
        .ok_or(NwcClientError::InvalidLookup)?;
    validate_hex_32(preimage).map_err(|_| NwcClientError::InvalidLookup)?;
    let fees_paid_msat = if result["fees_paid"].is_null() {
        None
    } else {
        Some(
            result["fees_paid"]
                .as_u64()
                .ok_or(NwcClientError::InvalidLookup)?,
        )
    };
    Ok(Some(PayInvoiceResult {
        preimage: preimage.into(),
        fees_paid_msat,
    }))
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
    parse_pay_invoice_response_json(&decrypt_response_event(connection, event, request_id)?)
}

fn decrypt_response_event(
    connection: &NwcConnection,
    event: &Event,
    request_id: EventId,
) -> Result<String, NwcClientError> {
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
    Ok(decrypted)
}

pub fn parse_wallet_capabilities(
    json: &str,
) -> Result<crate::purchase_contract::WalletCapabilities, NwcClientError> {
    let value: serde_json::Value = serde_json::from_str(json)?;
    if matches!(
        value["error"]["code"].as_str(),
        Some("NOT_IMPLEMENTED" | "UNSUPPORTED_METHOD" | "METHOD_NOT_FOUND")
    ) {
        return Err(NwcClientError::VerificationUnsupported);
    }
    if value["result_type"] != "get_info"
        || !value["error"].is_null()
        || !value["result"].is_object()
    {
        return Err(NwcClientError::MissingResult);
    }
    let result = &value["result"];
    let methods = match result.get("methods") {
        None => Vec::new(),
        Some(serde_json::Value::Array(methods)) => methods
            .iter()
            .map(|v| {
                v.as_str()
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned)
                    .ok_or(NwcClientError::MissingResult)
            })
            .collect::<Result<Vec<_>, _>>()?,
        _ => return Err(NwcClientError::MissingResult),
    };
    let network = result
        .get("network")
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned);
    Ok(crate::purchase_contract::WalletCapabilities { methods, network })
}

fn validate_hex_32(value: &str) -> Result<(), ()> {
    if value.len() != SECRET_HEX_LEN || !value.chars().all(|ch| ch.is_ascii_hexdigit()) {
        return Err(());
    }
    Ok(())
}

#[cfg(test)]
#[path = "nwc_client_tests.rs"]
mod tests;
