// Lightning Network integration: NIP-57 Zap payments for game purchases.

#![cfg(not(target_arch = "wasm32"))]

use nostr::prelude::*;
use nostr::RelayUrl;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tracing::{debug, info};

use crate::auth::AuthState;
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
    pub bolt11: String,           // the Lightning invoice string
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

    Ok(format!(
        "https://{}/.well-known/lnurlp/{}",
        domain, user
    ))
}

/// Signs an event using the Arcadestr ActiveSigner.
async fn sign_event_with_signer(
    builder: EventBuilder,
    signer: &ActiveSigner,
) -> Result<Event, LightningError> {
    // Get the public key from the signer
    let pubkey = signer
        .get_public_key()
        .await
        .map_err(|e| LightningError::ZapRequestSigning(format!("Failed to get public key: {}", e)))?;

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
    // Check authentication
    if !auth.is_authenticated() {
        return Err(LightningError::NotAuthenticated);
    }

    // Get the signer
    let signer = auth.signer().ok_or(LightningError::NotAuthenticated)?;

    // Step 1: Resolve LNURL-pay metadata
    let lnurl_url = lud16_to_lnurl_pay_url(&zap_req.seller_lud16)?;
    debug!("Resolving LNURL-pay URL: {}", lnurl_url);

    let client = reqwest::Client::new();
    let metadata: LnurlPayMetadata = client
        .get(&lnurl_url)
        .send()
        .await
        .map_err(|e| LightningError::LnurlResolution(e.to_string()))?
        .json()
        .await
        .map_err(|e| LightningError::LnurlResolution(format!("Failed to parse metadata: {}", e)))?;

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

    // Step 2: Build and sign the NIP-57 zap request event (kind 9734)
    // Parse seller's public key from npub
    let seller_pubkey = PublicKey::parse(&zap_req.seller_npub)
        .map_err(|e| LightningError::InvalidLud16(format!("Invalid seller npub: {}", e)))?;

    // Parse listing event ID from hex
    let listing_event_id = EventId::from_hex(&zap_req.listing_event_id)
        .map_err(|e| LightningError::Serialization(format!("Invalid listing event ID: {}", e)))?;

    // Build tags for zap request
    let mut tags: Vec<Tag> = vec![
        Tag::public_key(seller_pubkey),
        Tag::event(listing_event_id),
        Tag::custom(
            TagKind::Custom("amount".into()),
            [amount_msats.to_string()],
        ),
    ];

    // Add relays tag
    let relay_urls: Vec<RelayUrl> = zap_req
        .relays
        .iter()
        .filter_map(|r| RelayUrl::parse(r).ok())
        .collect();
    if !relay_urls.is_empty() {
        tags.push(Tag::relays(relay_urls));
    }

    // Build the zap request event
    let builder = EventBuilder::new(Kind::ZapRequest, "").tags(tags);

    // Sign the event
    let zap_event = sign_event_with_signer(builder, signer).await?;
    let zap_event_id = zap_event.id.to_hex();

    info!("Zap request event signed: {}", zap_event_id);

    // Step 3: Request the invoice from the LNURL callback
    // Serialize the zap event to JSON
    let zap_event_json = serde_json::to_string(&zap_event)?;
    let encoded_event = urlencoding::encode(&zap_event_json);

    // Build callback URL with query params
    let callback_url = format!(
        "{}?amount={}&nostr={}",
        metadata.callback, amount_msats, encoded_event
    );

    debug!("Requesting invoice from callback: {}", callback_url);

    let callback_response: CallbackResponse = client
        .get(&callback_url)
        .send()
        .await
        .map_err(|e| LightningError::InvoiceRequest(format!("HTTP request failed: {}", e)))?
        .json()
        .await
        .map_err(|e| LightningError::InvoiceRequest(format!("Failed to parse response: {}", e)))?;

    info!("Invoice received from LNURL callback");

    // Return the ZapInvoice
    Ok(ZapInvoice {
        bolt11: callback_response.pr,
        amount_sats: zap_req.amount_sats,
        seller_npub: zap_req.seller_npub.clone(),
        listing_event_id: zap_req.listing_event_id.clone(),
        zap_request_event_id: zap_event_id,
    })
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
