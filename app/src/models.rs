// Models for the Arcadestr marketplace UI.
// These mirror the core crate structs for serialization/deserialization.

use serde::{Deserialize, Serialize};

/// Game listing data structure.
/// Mirrors arcadestr_core::nostr::GameListing exactly.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GameListing {
    pub id: String, // unique slug / d-tag value, e.g. "my-game-v1"
    pub title: String,
    pub description: String,
    pub price_sats: u64,          // price in satoshis, 0 = free
    pub download_url: String,     // direct download link
    pub publisher_npub: String,   // bech32 npub of the publisher
    pub created_at: u64,          // unix timestamp
    pub tags: Vec<String>,        // freeform tags e.g. ["rpg", "pixel-art"]
    pub event_id: Option<String>, // hex event ID (optional, for zap requests)
    pub lud16: String, // Lightning address for payments (e.g., "seller@walletofsatoshi.com")
}

/// Zap request parameters for Lightning payment.
/// Mirrors arcadestr_core::lightning::ZapRequest.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ZapRequest {
    pub seller_npub: String,      // bech32 npub of seller
    pub seller_lud16: String,     // e.g. "seller@walletofsatoshi.com"
    pub listing_event_id: String, // hex event ID of the game listing
    pub amount_sats: u64,         // amount to pay
    pub buyer_npub: String,       // bech32 npub of buyer (from AuthState)
    pub relays: Vec<String>,      // relays to include in zap request event
}

/// Lightning invoice returned from zap request.
/// Mirrors arcadestr_core::lightning::ZapInvoice.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ZapInvoice {
    pub bolt11: String, // the Lightning invoice string
    pub amount_sats: u64,
    pub seller_npub: String,
    pub listing_event_id: String,
    pub zap_request_event_id: String, // the signed kind-9734 event ID
}

/// Marketplace view state for navigation.
#[derive(Clone, PartialEq)]
pub enum MarketplaceView {
    Browse,
    Publish,
    Detail(GameListing),
}
