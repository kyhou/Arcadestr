// Lightning Network integration: NIP-57 Zap payments for game purchases.

#![cfg(not(target_arch = "wasm32"))]

use nostr::prelude::*;
use nostr::RelayUrl;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;
use tracing::{debug, info};
use ::url::Url;

use crate::auth::AuthState;
use crate::http_client::HttpClient;
use crate::http_client::ReqwestHttpClient;
use crate::signers::{ActiveSigner, NostrSigner, SignerError};

/// Errors that can occur during Lightning operations.
#[derive(Debug, Error)]
pub enum LightningError {
    #[error("LNURL resolution failed: {0}")]
    LnurlResolution(String),
    #[error("Invoice request failed: {0}")]
    InvoiceRequest(String),
    #[error("Zap request signing failed: {0}")]
    ZapRequestSigning(String),
    #[error("Invalid lud16 address: {0}")]
    InvalidLud16(String),
    #[error("HTTP error: {0}")]
    Http(String),
    #[error("Serialization error: {0}")]
    Serialization(String),
    #[error("Not authenticated")]
    NotAuthenticated,
    #[error("Amount out of range: {0}")]
    AmountOutOfRange(String),
}

impl From<SignerError> for LightningError {
    fn from(e: SignerError) -> Self {
        LightningError::ZapRequestSigning(e.to_string())
    }
}

impl From<reqwest::Error> for LightningError {
    fn from(e: reqwest::Error) -> Self {
        LightningError::Http(e.to_string())
    }
}

impl From<serde_json::Error> for LightningError {
    fn from(e: serde_json::Error) -> Self {
        LightningError::Serialization(e.to_string())
    }
}

/// Zap request parameters for requesting a Lightning invoice.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ZapRequest {
    pub seller_npub: String,      // bech32 npub of seller
    pub seller_lud16: String,     // e.g. "seller@walletofsatoshi.com"
    pub listing_event_id: String, // hex event ID of the game listing
    pub amount_sats: u64,         // amount to pay
    pub buyer_npub: String,       // bech32 npub of buyer (from AuthState)
    pub relays: Vec<String>,      // relays to include in zap request event
}

/// Lightning invoice returned to the frontend.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ZapInvoice {
    pub bolt11: String, // the Lightning invoice string
    pub amount_sats: u64,
    pub seller_npub: String,
    pub listing_event_id: String,
    pub zap_request_event_id: String, // the signed kind-9734 event ID
}

/// LNURL-pay metadata response.
#[derive(Deserialize)]
struct LnurlPayMetadata {
    callback: String,
    #[serde(rename = "minSendable")]
    min_sendable: u64, // in millisatoshis
    #[serde(rename = "maxSendable")]
    max_sendable: u64, // in millisatoshis
    #[serde(rename = "allowsNostr")]
    allows_nostr: Option<bool>,
    #[serde(rename = "nostrPubkey")]
    nostr_pubkey: Option<String>,
}

/// LNURL-pay callback response containing the invoice.
#[derive(Deserialize)]
struct CallbackResponse {
    pr: String, // the bolt11 invoice
}

/// Converts a lud16 address to an LNURL-pay URL.
fn lud16_to_lnurl_pay_url(lud16: &str) -> Result<String, LightningError> {
    let parts: Vec<&str> = lud16.split('@').collect();
    if parts.len() != 2 {
        return Err(LightningError::InvalidLud16(
            "Invalid format, expected user@domain.com".to_string(),
        ));
    }

    let user = parts[0];
    let domain = parts[1];

    if user.is_empty() || domain.is_empty() {
        return Err(LightningError::InvalidLud16(
            "Empty user or domain".to_string(),
        ));
    }

    Ok(format!("https://{}/.well-known/lnurlp/{}", domain, user))
}

/// Signs an event using the Arcadestr ActiveSigner.
async fn sign_event_with_signer(
    builder: EventBuilder,
    signer: &ActiveSigner,
) -> Result<Event, LightningError> {
    // Get the public key from the signer
    let pubkey = signer.get_public_key().await.map_err(|e| {
        LightningError::ZapRequestSigning(format!("Failed to get public key: {}", e))
    })?;

    // Build the unsigned event
    let unsigned = builder.build(pubkey);

    // Sign the event using our signer
    let signed = signer
        .sign_event(unsigned)
        .await
        .map_err(|e| LightningError::ZapRequestSigning(format!("Failed to sign event: {}", e)))?;

    Ok(signed)
}

/// Requests a Lightning invoice for a zap payment.
pub async fn request_zap_invoice(
    zap_req: &ZapRequest,
    auth: &AuthState,
) -> Result<ZapInvoice, LightningError> {
    let http_client = ReqwestHttpClient::new(std::time::Duration::from_secs(8))
        .map_err(|e| LightningError::LnurlResolution(e.to_string()))?;

    request_zap_invoice_with_http(zap_req, auth, &http_client).await
}

pub(crate) async fn request_zap_invoice_with_http(
    zap_req: &ZapRequest,
    auth: &AuthState,
    http_client: &dyn HttpClient,
) -> Result<ZapInvoice, LightningError> {
    // Check authentication
    if !auth.is_authenticated() {
        return Err(LightningError::NotAuthenticated);
    }

    // Get the signer
    let signer = auth.signer().ok_or(LightningError::NotAuthenticated)?;

    // Step 1: Resolve LNURL-pay metadata
    let lnurl_url = lud16_to_lnurl_pay_url(&zap_req.seller_lud16)?;
    debug!("Resolving LNURL-pay URL: {}", lnurl_url);

    let metadata_value = http_client
        .get_json(&lnurl_url)
        .await
        .map_err(|e| LightningError::LnurlResolution(e.to_string()))?;

    let metadata: LnurlPayMetadata = serde_json::from_value(metadata_value).map_err(|e| {
        LightningError::LnurlResolution(format!("Failed to parse metadata: {}", e))
    })?;

    // Validate amount is within range
    let amount_msats = zap_req.amount_sats * 1000;
    if amount_msats < metadata.min_sendable || amount_msats > metadata.max_sendable {
        return Err(LightningError::AmountOutOfRange(format!(
            "Amount {} msats is outside range [{}, {}]",
            amount_msats, metadata.min_sendable, metadata.max_sendable
        )));
    }

    info!(
        "LNURL-pay metadata resolved: callback={}, min={}, max={}",
        metadata.callback, metadata.min_sendable, metadata.max_sendable
    );

    // Step 2: Build and sign NIP-57 zap request only when receiver supports it.
    let (zap_request_event_id, callback_url) = if supports_nostr_zaps(&metadata) {
        let zap_event = build_signed_zap_request_event(zap_req, amount_msats, signer).await?;
        let zap_event_id = zap_event.id.to_hex();
        info!("Zap request event signed: {}", zap_event_id);

        let zap_event_json = serde_json::to_string(&zap_event)?;
        let callback_url = build_callback_url(&metadata.callback, amount_msats, Some(&zap_event_json))?;

        (zap_event_id, callback_url)
    } else {
        (
            String::new(),
            build_callback_url(&metadata.callback, amount_msats, None)?,
        )
    };

    debug!("Requesting invoice from callback: {}", callback_url);

    let callback_value = http_client
        .get_json(&callback_url)
        .await
        .map_err(|e| LightningError::InvoiceRequest(format!("HTTP request failed: {}", e)))?;

    let callback_response: CallbackResponse = serde_json::from_value(callback_value).map_err(|e| {
        LightningError::InvoiceRequest(format!("Failed to parse response: {}", e))
    })?;

    info!("Invoice received from LNURL callback");

    // Return the ZapInvoice
    Ok(ZapInvoice {
        bolt11: callback_response.pr,
        amount_sats: zap_req.amount_sats,
        seller_npub: zap_req.seller_npub.clone(),
        listing_event_id: zap_req.listing_event_id.clone(),
        zap_request_event_id,
    })
}

fn supports_nostr_zaps(metadata: &LnurlPayMetadata) -> bool {
    let allows_nostr = metadata.allows_nostr.unwrap_or(false);
    let has_valid_nostr_pubkey = metadata
        .nostr_pubkey
        .as_ref()
        .map(|pubkey| is_hex_pubkey(pubkey) && PublicKey::from_hex(pubkey).is_ok())
        .unwrap_or(false);

    allows_nostr && has_valid_nostr_pubkey
}

fn is_hex_pubkey(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|b| b.is_ascii_hexdigit())
}

fn build_callback_url(
    callback: &str,
    amount_msats: u64,
    nostr_json: Option<&str>,
) -> Result<String, LightningError> {
    let mut url = Url::parse(callback)
        .map_err(|e| LightningError::InvoiceRequest(format!("Invalid callback URL: {}", e)))?;
    {
        let mut pairs = url.query_pairs_mut();
        pairs.append_pair("amount", &amount_msats.to_string());

        if let Some(nostr_json) = nostr_json {
            pairs.append_pair("nostr", nostr_json);
        }
    }

    Ok(url.to_string())
}

async fn build_signed_zap_request_event(
    zap_req: &ZapRequest,
    amount_msats: u64,
    signer: &ActiveSigner,
) -> Result<Event, LightningError> {
    let seller_pubkey = PublicKey::parse(&zap_req.seller_npub)
        .map_err(|e| LightningError::InvalidLud16(format!("Invalid seller npub: {}", e)))?;

    let listing_event_id = EventId::from_hex(&zap_req.listing_event_id)
        .map_err(|e| LightningError::Serialization(format!("Invalid listing event ID: {}", e)))?;

    let mut tags: Vec<Tag> = vec![
        Tag::public_key(seller_pubkey),
        Tag::event(listing_event_id),
        Tag::custom(TagKind::Custom("amount".into()), [amount_msats.to_string()]),
    ];

    let relay_urls: Vec<RelayUrl> = zap_req
        .relays
        .iter()
        .filter_map(|relay| RelayUrl::parse(relay).ok())
        .collect();
    if !relay_urls.is_empty() {
        tags.push(Tag::relays(relay_urls));
    }

    let builder = EventBuilder::new(Kind::ZapRequest, "").tags(tags);
    sign_event_with_signer(builder, signer).await
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::test_helpers::http_mocks::MockHttpClient;

    const TEST_SELLER_PUBKEY_HEX: &str =
        "d94a3f0b5b907fda6c1d2716af34e4d533ddf8f6f6f0f8f1f4a3f605f6c9a3b4";
    const TEST_LISTING_EVENT_ID: &str =
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const TEST_BOLT11: &str =
        "lnbc10n1p0testpp5qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqq";

    fn test_auth_state() -> AuthState {
        let mut auth = AuthState::new();
        let keys = Keys::generate();
        auth.connect_with_key(&keys.secret_key().to_secret_hex())
            .expect("test key should authenticate auth state");
        auth
    }

    fn test_zap_request() -> ZapRequest {
        ZapRequest {
            seller_npub: TEST_SELLER_PUBKEY_HEX.to_string(),
            seller_lud16: "seller@example.com".to_string(),
            listing_event_id: TEST_LISTING_EVENT_ID.to_string(),
            amount_sats: 21,
            buyer_npub: "npub1buyerunusedinthispath".to_string(),
            relays: vec![
                "wss://relay1.example.com".to_string(),
                "wss://relay2.example.com".to_string(),
            ],
        }
    }

    fn lnurl_url() -> String {
        "https://example.com/.well-known/lnurlp/seller".to_string()
    }

    #[test]
    fn lud16_parsing() {
        let parsed = lud16_to_lnurl_pay_url("seller@example.com")
            .expect("lud16 parsing should succeed");
        assert_eq!(parsed, lnurl_url());

        let invalid = lud16_to_lnurl_pay_url("seller-example.com");
        assert!(invalid.is_err());
    }

    #[tokio::test]
    async fn zap_request_uses_kind_9734_and_expected_tags() {
        let auth = test_auth_state();
        let request = test_zap_request();
        let callback_base = "https://ln.example.com/callback";

        let mock = MockHttpClient::new()
            .with_json_response(
                &lnurl_url(),
                serde_json::json!({
                    "callback": callback_base,
                    "minSendable": 1000,
                    "maxSendable": 1000000,
                    "allowsNostr": true,
                    "nostrPubkey": TEST_SELLER_PUBKEY_HEX
                }),
            )
            .with_prefix_json_response(callback_base, serde_json::json!({ "pr": TEST_BOLT11 }));

        let invoice = request_zap_invoice_with_http(&request, &auth, &mock)
            .await
            .expect("zap invoice request should succeed");

        assert_eq!(invoice.amount_sats, request.amount_sats);
        assert!(!invoice.zap_request_event_id.is_empty());

        let callback_url = mock
            .last_requested_url()
            .expect("callback request URL should be captured");
        let parsed = Url::parse(&callback_url).expect("callback URL should parse");
        let query: std::collections::HashMap<String, String> = parsed.query_pairs().into_owned().collect();

        let amount_msats = request.amount_sats * 1000;
        assert_eq!(query.get("amount"), Some(&amount_msats.to_string()));

        let nostr_encoded = query.get("nostr").expect("nostr query should exist");
        let nostr_event_json = urlencoding::decode(nostr_encoded)
            .expect("nostr payload should decode")
            .into_owned();
        let zap_event: Event =
            serde_json::from_str(&nostr_event_json).expect("decoded nostr payload should parse as Event");

        assert_eq!(zap_event.kind, Kind::ZapRequest);

        let serialized_tags = serde_json::to_value(&zap_event)
            .expect("event should serialize")
            .get("tags")
            .cloned()
            .expect("event should contain tags");
        let tags = serialized_tags
            .as_array()
            .expect("tags should be array");

        assert!(tags.iter().any(|tag| {
            tag.as_array()
                .map(|parts| {
                    parts.len() >= 2
                        && parts[0] == Value::String("p".to_string())
                        && parts[1] == Value::String(TEST_SELLER_PUBKEY_HEX.to_string())
                })
                .unwrap_or(false)
        }));

        assert!(tags.iter().any(|tag| {
            tag.as_array()
                .map(|parts| {
                    parts.len() >= 2
                        && parts[0] == Value::String("amount".to_string())
                        && parts[1] == Value::String(amount_msats.to_string())
                })
                .unwrap_or(false)
        }));

        assert!(tags.iter().any(|tag| {
            tag.as_array()
                .map(|parts| {
                    parts.len() >= 3
                        && parts[0] == Value::String("relays".to_string())
                        && parts[1] == Value::String("wss://relay1.example.com".to_string())
                        && parts[2] == Value::String("wss://relay2.example.com".to_string())
                })
                .unwrap_or(false)
        }));
    }

    #[tokio::test]
    async fn lnurl_response_without_allows_nostr_skips_nostr_query() {
        let auth = test_auth_state();
        let request = test_zap_request();
        let callback_base = "https://ln.example.com/callback";

        let mock = MockHttpClient::new()
            .with_json_response(
                &lnurl_url(),
                serde_json::json!({
                    "callback": callback_base,
                    "minSendable": 1000,
                    "maxSendable": 1000000
                }),
            )
            .with_prefix_json_response(callback_base, serde_json::json!({ "pr": TEST_BOLT11 }));

        let invoice = request_zap_invoice_with_http(&request, &auth, &mock)
            .await
            .expect("invoice request should succeed without nostr mode");

        assert!(invoice.zap_request_event_id.is_empty());

        let callback_url = mock
            .last_requested_url()
            .expect("callback request URL should be captured");
        let parsed = Url::parse(&callback_url).expect("callback URL should parse");
        let query: std::collections::HashMap<String, String> = parsed.query_pairs().into_owned().collect();

        assert!(query.contains_key("amount"));
        assert!(!query.contains_key("nostr"));
    }

    #[tokio::test]
    async fn max_sendable_respected() {
        let auth = test_auth_state();
        let mut request = test_zap_request();
        request.amount_sats = 2_000;

        let mock = MockHttpClient::new().with_json_response(
            &lnurl_url(),
            serde_json::json!({
                "callback": "https://ln.example.com/callback",
                "minSendable": 1000,
                "maxSendable": 1_000_000,
                "allowsNostr": true,
                "nostrPubkey": TEST_SELLER_PUBKEY_HEX
            }),
        );

        let result = request_zap_invoice_with_http(&request, &auth, &mock).await;
        assert!(matches!(result, Err(LightningError::AmountOutOfRange(_))));
    }

    #[tokio::test]
    async fn not_authenticated_rejected() {
        let auth = AuthState::new();
        let request = test_zap_request();
        let mock = MockHttpClient::new();

        let result = request_zap_invoice_with_http(&request, &auth, &mock).await;
        assert!(matches!(result, Err(LightningError::NotAuthenticated)));
    }
}

/// Placeholder Lightning client for WASM target (empty implementation).
#[cfg(target_arch = "wasm32")]
pub struct LightningClient;

#[cfg(target_arch = "wasm32")]
impl LightningClient {
    /// Creates a new Lightning client instance.
    pub fn new() -> Self {
        Self
    }
}

#[cfg(target_arch = "wasm32")]
impl Default for LightningClient {
    fn default() -> Self {
        Self::new()
    }
}
