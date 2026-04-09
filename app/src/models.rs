// ── app/src/models.rs ────────────────────────────────────────────────────────
//
// Replace the existing `GameListing` struct with this version.
// `UserProfile`, `ZapRequest`, and `ZapInvoice` are unchanged.
//
// What changed vs the old kind-30078 struct:
//   • `source_kind` field added (always 30018 for new events)
//   • `stall_id` / `stall_name` link the product back to its stall
//   • `images` replaces the single implicit download URL for media
//   • `currency` + `price` carry the raw NIP-15 pricing; `price_sats` is
//     kept as a best-effort display value (0 when currency ≠ SATS/SAT)
//   • `quantity` reflects NIP-15 stock info (None = unlimited / digital)
//   • `specs` exposes arbitrary key→value product attributes
//   • `lud16` is retained but always empty on initial fetch; callers fill it
//     from the merchant profile once that profile is loaded
//   • All new fields have `#[serde(default)]` so stale cached JSON from
//     old 30078 events still deserialises without error during the migration.

use serde::{Deserialize, Serialize};

// ── GameListing ───────────────────────────────────────────────────────────────

/// Identifies which NOSTR event kind was the source of this listing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ListingSource {
    /// NIP-15 product event (kind 30018). Current standard.
    Nip15Product,
    /// NIP-99 listing event (kind 30402/30403).
    Nip99Listing,
    /// Legacy game listing (kind 30078). Deprecated — no longer published.
    Legacy,
}

impl Default for ListingSource {
    fn default() -> Self {
        Self::Legacy
    }
}

/// A game (or any digital product) available in the marketplace.
///
/// This type is the shared currency between the Tauri backend and the Leptos
/// frontend. It is always serialised/deserialised as JSON across the IPC
/// bridge, so every field must be `serde`-compatible.
///
/// Fields that have no equivalent in the source event are left at their
/// `Default` values and may be enriched by later lookups (e.g. `lud16`
/// comes from the merchant's NIP-01 profile, not the product event itself).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GameListing {
    // ── Identity ──────────────────────────────────────────────────────────────
    /// Product UUID — the `d` tag value; uniquely identifies this listing
    /// within the publisher's event stream.
    pub id: String,

    /// Which event kind produced this listing.
    #[serde(default)]
    pub source: ListingSource,

    // ── Core metadata ─────────────────────────────────────────────────────────
    pub title: String,
    pub description: String,

    /// Image URLs for screenshots / cover art (from NIP-15 `images` array).
    #[serde(default)]
    pub images: Vec<String>,

    /// The first `images` entry, or a URL found in `specs["download_url"]`.
    /// Kept for backwards compatibility with UI components that expect a
    /// single `download_url` field. May be empty.
    #[serde(default)]
    pub download_url: String,

    // ── Pricing ───────────────────────────────────────────────────────────────
    /// Raw price in the stall's declared currency (e.g. `9.99` USD or
    /// `21000` SATS). Use this + `currency` for display.
    #[serde(default)]
    pub price: f64,

    /// Currency code as declared by the merchant (e.g. `"SATS"`, `"USD"`).
    #[serde(default)]
    pub currency: String,

    /// Best-effort satoshi equivalent, used by the existing buy/zap flow.
    /// Set to `price as u64` when `currency` is `"SATS"` or `"SAT"`;
    /// otherwise `0` until a conversion rate is available.
    #[serde(default)]
    pub price_sats: u64,

    // ── Stock ─────────────────────────────────────────────────────────────────
    /// Available units. `None` = unlimited (typical for digital downloads).
    #[serde(default)]
    pub quantity: Option<u64>,

    // ── Taxonomy ──────────────────────────────────────────────────────────────
    /// Categories from the product's `t` tags.
    #[serde(default)]
    pub tags: Vec<String>,

    // ── Specs ─────────────────────────────────────────────────────────────────
    /// Arbitrary key→value product attributes (NIP-15 `specs` array).
    /// Example: `[("os", "Linux"), ("download_url", "https://...")]`.
    #[serde(default)]
    pub specs: Vec<(String, String)>,

    // ── Publisher / stall ─────────────────────────────────────────────────────
    /// Bech32 `npub` of the merchant who published this product.
    pub publisher_npub: String,

    /// UUID of the stall this product belongs to.
    #[serde(default)]
    pub stall_id: String,

    /// Human-readable stall name, populated when the stall event is
    /// fetched alongside the product.
    #[serde(default)]
    pub stall_name: Option<String>,

    /// Lightning address for the buy flow. Empty on initial fetch;
    /// filled in from the merchant's NIP-01 kind-0 profile.
    #[serde(default)]
    pub lud16: String,

    /// Event ID (hex) - set by backend after publishing
    #[serde(default)]
    pub event_id: Option<String>,

    // ── Timestamps ────────────────────────────────────────────────────────────
    pub created_at: u64,
}

impl GameListing {
    /// Construct a `GameListing` from a NIP-15 product, optionally enriched
    /// with its parent stall.
    ///
    /// `lud16` is left empty here — callers should fill it once the
    /// merchant's NIP-01 profile has been fetched.
    ///
    /// This constructor lives in `app/src/models.rs` so the frontend can
    /// perform the mapping without an extra IPC round-trip if needed.
    /// On the backend (`desktop/src/main.rs`) the Tauri command calls an
    /// equivalent mapping directly on the `core` types.
    #[cfg(all(not(target_arch = "wasm32"), feature = "native"))]
    pub fn from_nip15(
        product: arcadestr_core::marketplace::Nip15Product,
        stall: Option<&arcadestr_core::marketplace::Nip15Stall>,
    ) -> Self {
        // Prefer an explicit "download_url" spec entry, then fall back to
        // the first image.
        let download_url = product
            .specs
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case("download_url"))
            .map(|(_, v): &(String, String)| v.clone())
            .or_else(|| product.images.first().cloned())
            .unwrap_or_default();

        // Best-effort sats conversion for the legacy buy/zap flow.
        let price_sats = if product.currency.eq_ignore_ascii_case("SATS")
            || product.currency.eq_ignore_ascii_case("SAT")
        {
            product.price as u64
        } else {
            0 // UI should use price + currency when price_sats == 0
        };

        GameListing {
            id: product.id,
            source: ListingSource::Nip15Product,
            title: product.name,
            description: product.description.unwrap_or_default(),
            images: product.images,
            download_url,
            price: product.price,
            currency: product.currency,
            price_sats,
            quantity: product.quantity,
            tags: product.categories,
            specs: product.specs,
            publisher_npub: product.merchant_npub,
            stall_id: product.stall_id,
            stall_name: stall.map(|s| s.name.clone()),
            lud16: String::new(),
            event_id: None,
            created_at: product.created_at,
        }
    }

    #[cfg(all(not(target_arch = "wasm32"), feature = "native"))]
    pub fn from_listing(listing: arcadestr_core::marketplace::Nip99Listing) -> Self {
        let download_url = listing.images.first().cloned().unwrap_or_default();

        let parsed_amount = listing
            .price_amount
            .as_deref()
            .and_then(|amount| amount.parse::<f64>().ok())
            .unwrap_or(0.0);

        let currency = listing.price_currency.clone().unwrap_or_default();
        let price_sats =
            if currency.eq_ignore_ascii_case("SATS") || currency.eq_ignore_ascii_case("SAT") {
                parsed_amount as u64
            } else {
                0
            };

        GameListing {
            id: listing.id,
            source: ListingSource::Nip99Listing,
            title: listing.title,
            description: listing.content,
            images: listing.images,
            download_url,
            price: parsed_amount,
            currency,
            price_sats,
            quantity: None,
            tags: listing.tags,
            specs: Vec::new(),
            publisher_npub: listing.merchant_npub,
            stall_id: String::new(),
            stall_name: None,
            lud16: String::new(),
            event_id: None,
            created_at: listing.created_at,
        }
    }
}

// ── UserProfile ───────────────────────────────────────────────────────────────
// (unchanged from original)

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct UserProfile {
    pub npub: String,
    pub name: Option<String>,
    pub display_name: Option<String>,
    pub about: Option<String>,
    pub picture: Option<String>,
    pub website: Option<String>,
    pub nip05: Option<String>,
    pub lud16: Option<String>,
    pub nip05_verified: bool,
}

impl UserProfile {
    /// Returns the best available display name, falling back to truncated npub.
    pub fn display(&self) -> String {
        self.display_name
            .clone()
            .or_else(|| self.name.clone())
            .unwrap_or_else(|| {
                if self.npub.len() > 16 {
                    format!("{}...", &self.npub[..16])
                } else {
                    self.npub.clone()
                }
            })
    }
}

// ── ZapRequest / ZapInvoice ───────────────────────────────────────────────────
// (unchanged from original — shown here for context)

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ZapRequest {
    /// Backwards compatibility: maps to recipient_npub
    #[serde(alias = "recipient_npub")]
    pub seller_npub: String,
    /// Backwards compatibility: maps to lud16
    #[serde(alias = "lud16")]
    pub seller_lud16: String,
    /// Backwards compatibility: maps to listing_id
    #[serde(alias = "listing_id")]
    pub listing_event_id: String,
    pub amount_sats: u64,
    /// The buyer's npub (from AuthState)
    pub buyer_npub: String,
    /// Relays to include in zap request event
    pub relays: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ZapInvoice {
    pub bolt11: String,
    pub amount_sats: u64,
    pub seller_npub: String,
    pub listing_event_id: String,
    pub zap_request_event_id: String,
}

/// Marketplace view state for navigation.
#[derive(Clone, PartialEq)]
pub enum MarketplaceView {
    Browse,
    Publish,
    Detail(GameListing),
    Profile,
}
