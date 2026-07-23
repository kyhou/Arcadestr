//! NIP-15 Marketplace — stalls, products, and client-side filtering.
//!
//! Reference: <https://github.com/nostr-protocol/nips/blob/master/15.md>
//!
//! # Module layout
//!
//! | Item                    | Role                                                    |
//! |-------------------------|---------------------------------------------------------|
//! | [`Nip15Stall`]          | Parsed kind-30017 event (stall)                         |
//! | [`Nip15Product`]        | Parsed kind-30018 event (product)                       |
//! | [`ShippingZone`]        | Shipping zone defined at stall level                    |
//! | [`ProductShipping`]     | Extra cost override at product level                    |
//! | [`MarketplaceFilter`]   | All filter dimensions; `None` on each = no restriction  |
//! | [`apply_filter`]        | Pure function: apply a filter to a product list         |
//! | `fetch_nip15_stalls_impl`   | `pub(crate)` relay query for kind-30017            |
//! | `fetch_nip15_products_impl` | `pub(crate)` relay query for kind-30018            |
//! | `fetch_nip15_products_streaming` | `pub` streaming relay query for kind-30018      |
//!
//! Callers outside this crate should go through the `NostrClient` wrapper
//! methods in `nostr.rs`, which keep the inner `nostr_sdk::Client` private.

use nostr_sdk::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

pub use crate::is_replaceable_event_newer;

// ── Internal deserialization helpers ─────────────────────────────────────────
// These mirror the raw NIP-15 JSON structures and are private to this module.
// Downstream code always works with the richer domain types below.

/// Deserialize a field that may be either a float or a string representation of a float.
fn deserialize_f64_or_string<'de, D>(deserializer: D) -> Result<f64, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::Error;
    use serde_json::Value;

    let value = Value::deserialize(deserializer)?;
    match value {
        Value::Number(n) => n
            .as_f64()
            .ok_or_else(|| D::Error::custom("invalid float number")),
        Value::String(s) => s
            .parse::<f64>()
            .map_err(|e| D::Error::custom(format!("invalid float string: {}", e))),
        _ => Err(D::Error::custom("expected float or string")),
    }
}

/// Deserialize an optional u64 that may be an integer, string, or null.
fn deserialize_optional_u64_or_string<'de, D>(deserializer: D) -> Result<Option<u64>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::Error;
    use serde_json::Value;

    let value = Value::deserialize(deserializer)?;
    match value {
        Value::Null => Ok(None),
        Value::Number(n) => n
            .as_u64()
            .map(Some)
            .ok_or_else(|| D::Error::custom("invalid unsigned integer")),
        Value::String(s) => s
            .parse::<u64>()
            .map(Some)
            .map_err(|e| D::Error::custom(format!("invalid unsigned integer string: {}", e))),
        _ => Err(D::Error::custom("expected integer, string, or null")),
    }
}

/// Deserialize an optional vector that may be an array or null.
fn deserialize_optional_vec_shipping<'de, D>(
    deserializer: D,
) -> Result<Vec<ProductShipping>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::Error;
    use serde_json::Value;

    let value = Value::deserialize(deserializer)?;
    match value {
        Value::Null => Ok(Vec::new()),
        Value::Array(arr) => {
            let mut result = Vec::with_capacity(arr.len());
            for item in arr {
                let shipping: ProductShipping = serde_json::from_value(item)
                    .map_err(|e| D::Error::custom(format!("invalid shipping entry: {}", e)))?;
                result.push(shipping);
            }
            Ok(result)
        }
        _ => Err(D::Error::custom("expected array or null")),
    }
}

fn deserialize_optional_vec_shipping_zone<'de, D>(
    deserializer: D,
) -> Result<Vec<ShippingZone>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    struct OptionalVecShippingZoneVisitor;

    impl<'de> serde::de::Visitor<'de> for OptionalVecShippingZoneVisitor {
        type Value = Vec<ShippingZone>;

        fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
            formatter.write_str("an array of shipping zones or null")
        }

        fn visit_none<E>(self) -> Result<Self::Value, E>
        where
            E: serde::de::Error,
        {
            Ok(Vec::new())
        }

        fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
        where
            A: serde::de::SeqAccess<'de>,
        {
            let mut vec = Vec::new();
            while let Some(item) = seq.next_element()? {
                vec.push(item);
            }
            Ok(vec)
        }
    }

    deserializer.deserialize_option(OptionalVecShippingZoneVisitor)
}

#[derive(Debug, Deserialize)]
struct StallContent {
    id: String,
    name: String,
    description: Option<String>,
    currency: String,
    #[serde(default, deserialize_with = "deserialize_optional_vec_shipping_zone")]
    shipping: Vec<ShippingZone>,
}

#[derive(Debug, Deserialize)]
struct ProductContent {
    id: String,
    stall_id: String,
    name: String,
    description: Option<String>,
    #[serde(default)]
    images: Vec<String>,
    currency: String,
    #[serde(deserialize_with = "deserialize_f64_or_string")]
    price: f64,
    /// `null` in JSON becomes `None` here (unlimited / digital goods).
    #[serde(default, deserialize_with = "deserialize_optional_u64_or_string")]
    quantity: Option<u64>,
    /// NIP-15 encodes specs as `[[key, value], ...]` arrays.
    #[serde(default)]
    specs: Vec<[String; 2]>,
    #[serde(default, deserialize_with = "deserialize_optional_vec_shipping")]
    shipping: Vec<ProductShipping>,
}

// ── Public domain types ───────────────────────────────────────────────────────

/// A shipping zone as defined inside a stall's `shipping` array.
///
/// The stall `currency` applies to the `cost` field here.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShippingZone {
    pub id: String,
    pub name: Option<String>,
    /// Base shipping cost for orders to this zone (in the stall's currency).
    #[serde(deserialize_with = "deserialize_f64_or_string")]
    pub cost: f64,
    #[serde(default)]
    pub regions: Vec<String>,
}

/// A per-product shipping cost override.
///
/// `id` must match a zone defined in the parent stall.
/// Total shipping = stall base cost for the zone
///               + (order quantity × `cost` here, if present).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProductShipping {
    pub id: String,
    #[serde(deserialize_with = "deserialize_f64_or_string")]
    pub cost: f64,
}

/// A NIP-15 stall (kind 30017) enriched with event-level metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Nip15Stall {
    /// Stall UUID — matches both the `d` tag and the `id` field in content.
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub currency: String,
    pub shipping: Vec<ShippingZone>,
    /// Bech32-encoded `npub` of the merchant who published this event.
    pub merchant_npub: String,
    pub created_at: u64,
}

/// A NIP-15 product (kind 30018) enriched with event-level metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Nip15Product {
    /// Product UUID — matches both the `d` tag and the `id` field in content.
    pub id: String,
    /// References the parent [`Nip15Stall::id`].
    pub stall_id: String,
    pub name: String,
    pub description: Option<String>,
    pub images: Vec<String>,
    pub currency: String,
    pub price: f64,
    /// `None` = unlimited availability (typical for digital goods).
    pub quantity: Option<u64>,
    /// Key→value specification pairs (e.g. `("os", "Linux")`).
    pub specs: Vec<(String, String)>,
    pub shipping: Vec<ProductShipping>,
    /// Categories derived from the event's `t` tags.
    pub categories: Vec<String>,
    /// Bech32-encoded `npub` of the merchant who published this event.
    pub merchant_npub: String,
    pub created_at: u64,
}

/// Current listing policy for callers without durable ownership.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum AcquisitionPolicy {
    #[default]
    Gated,
    Public,
    TimedAccess {
        starts_at: u64,
        ends_at: u64,
    },
}

impl AcquisitionPolicy {
    pub fn allows_access_at(&self, now: u64) -> bool {
        match self {
            Self::Gated => false,
            Self::Public => true,
            Self::TimedAccess { starts_at, ends_at } => *starts_at <= now && now < *ends_at,
        }
    }
}

/// Advisory pointer to an immutable campaign root event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CampaignPointer {
    pub root_event_id: nostr::EventId,
    pub relay_hint: Option<String>,
}

/// One independently valid listing reference to a fulfillment authorization root.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FulfillmentAuthorizationReference {
    pub root_event_id: nostr::EventId,
    pub fulfillment_pubkey: nostr::PublicKey,
    pub relay_hint: Option<String>,
}

/// A NIP-99 listing (kind 30402/30403) enriched with event-level metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Nip99Listing {
    /// Signed source event ID.
    #[serde(default)]
    pub event_id: String,
    /// Listing UUID from the `d` tag.
    pub id: String,
    pub title: String,
    /// Markdown description from `event.content`.
    pub content: String,
    pub summary: Option<String>,
    pub published_at: Option<i64>,
    pub location: Option<String>,
    pub price_amount: Option<String>,
    pub price_currency: Option<String>,
    pub price_frequency: Option<String>,
    pub images: Vec<String>,
    /// Geohash from the `g` tag.
    pub geohash: Option<String>,
    /// Categories/keywords derived from `t` tags.
    pub tags: Vec<String>,
    /// Supported delivery platforms from `platform` tags.
    #[serde(default)]
    pub platforms: Vec<String>,
    /// First NIP-94 metadata event id from `nip94` tags.
    #[serde(default)]
    pub nip94_event_id: Option<String>,
    /// ADP distribution server URLs.
    #[serde(default)]
    pub servers: Vec<String>,
    /// SHA-256 artifact hash advertised by ADP fulfillment.
    #[serde(default)]
    pub file_hash: Option<String>,
    /// Published artifact version.
    #[serde(default)]
    pub version: Option<String>,
    /// Independently parsed valid authorization references.
    #[serde(default)]
    pub fulfillment_authorizations: Vec<FulfillmentAuthorizationReference>,
    /// Malformed raw references retained for diagnostics without poisoning siblings.
    #[serde(default)]
    pub malformed_fulfillment_authorization_tags: Vec<Vec<String>>,
    /// Current explicit access policy. Missing or malformed tags fail closed.
    #[serde(default)]
    pub acquisition: AcquisitionPolicy,
    /// Advisory campaign discovery pointers.
    #[serde(default)]
    pub campaigns: Vec<CampaignPointer>,
    pub status: Option<String>,
    /// Original Nostr event JSON for debug-only inspection.
    #[cfg(debug_assertions)]
    #[serde(default)]
    pub raw_event_json: Option<String>,
    /// Bech32-encoded `npub` of the merchant who published this event.
    pub merchant_npub: String,
    pub created_at: u64,
}

// ── MarketplaceFilter ─────────────────────────────────────────────────────────

/// Describes every dimension along which the marketplace can be filtered.
///
/// Most fields are `Option<_>` — `None` means "no restriction on this
/// dimension". Empty vectors also mean "no restriction".
/// `MarketplaceFilter::default()` therefore passes every product through
/// unmodified, making it safe to wire up now and restrict later.
///
/// ### Adding a new filter type
///
/// 1. Add an `Option<T>` field here with a doc-comment.
/// 2. Add the corresponding check in [`passes_filter`].
/// 3. Both user-defined preferences and hardcoded business rules should be
///    expressed as a `MarketplaceFilter` and composed before calling
///    [`apply_filter`] — keeping all filtering logic in one place.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MarketplaceFilter {
    // ── Price ─────────────────────────────────────────────────────────────────
    /// Inclusive lower bound, in the product's own currency.
    pub min_price: Option<f64>,
    /// Inclusive upper bound, in the product's own currency.
    pub max_price: Option<f64>,
    /// When `true`, only products with `price == 0` pass through.
    pub free_only: Option<bool>,

    // ── Currency ──────────────────────────────────────────────────────────────
    /// Allowed currencies (case-insensitive). An empty `Vec` = no restriction.
    pub currencies: Option<Vec<String>>,

    // ── Taxonomy ──────────────────────────────────────────────────────────────
    /// Required categories (from `t` tags). Product must match ≥1.
    /// An empty `Vec` = no restriction.
    pub categories: Option<Vec<String>>,

    // ── Merchant access control ───────────────────────────────────────────────
    /// Exclusive allowlist of merchant `npub`s. Empty `Vec` = allow all.
    pub merchant_whitelist: Option<Vec<String>>,
    /// Blocked merchant `npub`s. Always enforced, even when a whitelist
    /// is also set. Empty `Vec` = block nobody.
    pub merchant_blacklist: Option<Vec<String>>,

    // ── Stall filtering ───────────────────────────────────────────────────────
    /// Exclusive allowlist of stall IDs. Empty `Vec` = allow all.
    pub stall_ids: Option<Vec<String>>,

    /// If non-empty, only return listings whose `platforms` field contains
    /// at least one of these values. An empty vec disables platform filtering.
    /// Listings that declare no `["platform", ...]` tags are treated as
    /// unrestricted and always pass platform filters regardless of active value.
    #[serde(default)]
    pub platforms: Vec<String>,
}

// ── Filtering ─────────────────────────────────────────────────────────────────

/// Apply `filter` to `products`, returning only those that pass every
/// active dimension.
///
/// This is the **single entry point** for all filtering — both user-defined
/// preferences and hardcoded rules should funnel through here.
/// Call with `&MarketplaceFilter::default()` to skip all filtering.
pub fn apply_filter(products: Vec<Nip15Product>, filter: &MarketplaceFilter) -> Vec<Nip15Product> {
    products
        .into_iter()
        .filter(|p| passes_filter(p, filter))
        .collect()
}

pub fn apply_filter_nip99(
    listings: Vec<Nip99Listing>,
    filter: &MarketplaceFilter,
) -> Vec<Nip99Listing> {
    listings
        .into_iter()
        .filter(|p| passes_filter_nip99(p, filter))
        .collect()
}

fn passes_filter(p: &Nip15Product, f: &MarketplaceFilter) -> bool {
    // Free-only check first — short-circuits before the price-range checks.
    if f.free_only.unwrap_or(false) && p.price > 0.0 {
        return false;
    }

    // ── Price range ───────────────────────────────────────────────────────────
    if let Some(min) = f.min_price {
        if p.price < min {
            return false;
        }
    }
    if let Some(max) = f.max_price {
        if p.price > max {
            return false;
        }
    }

    // ── Currency (case-insensitive) ───────────────────────────────────────────
    if let Some(ref currencies) = f.currencies {
        if !currencies.is_empty()
            && !currencies
                .iter()
                .any(|c| c.eq_ignore_ascii_case(&p.currency))
        {
            return false;
        }
    }

    // ── Categories: product must carry at least one matching t-tag ────────────
    if let Some(ref cats) = f.categories {
        if !cats.is_empty() {
            let hit = cats
                .iter()
                .any(|c| p.categories.iter().any(|pc| pc.eq_ignore_ascii_case(c)));
            if !hit {
                return false;
            }
        }
    }

    // ── Merchant whitelist ────────────────────────────────────────────────────
    if let Some(ref wl) = f.merchant_whitelist {
        if !wl.is_empty() && !wl.contains(&p.merchant_npub) {
            return false;
        }
    }

    // ── Merchant blacklist (always enforced) ──────────────────────────────────
    if let Some(ref bl) = f.merchant_blacklist {
        if bl.contains(&p.merchant_npub) {
            return false;
        }
    }

    // ── Stall filter ───────────────────────────────────────────────────────────
    if let Some(ref stall_ids) = f.stall_ids {
        if !stall_ids.is_empty() && !stall_ids.contains(&p.stall_id) {
            return false;
        }
    }

    true
}

fn passes_filter_nip99(p: &Nip99Listing, f: &MarketplaceFilter) -> bool {
    let price = p
        .price_amount
        .as_deref()
        .unwrap_or("0")
        .parse::<f64>()
        .unwrap_or(0.0);

    if f.free_only.unwrap_or(false) && price > 0.0 {
        return false;
    }

    if let Some(min) = f.min_price {
        if price < min {
            return false;
        }
    }
    if let Some(max) = f.max_price {
        if price > max {
            return false;
        }
    }

    if let Some(ref currencies) = f.currencies {
        if !currencies.is_empty()
            && !currencies
                .iter()
                .any(|c| c.eq_ignore_ascii_case(p.price_currency.as_deref().unwrap_or("")))
        {
            return false;
        }
    }

    if let Some(ref cats) = f.categories {
        if !cats.is_empty() {
            let hit = cats
                .iter()
                .any(|c| p.tags.iter().any(|pc| pc.eq_ignore_ascii_case(c)));
            if !hit {
                return false;
            }
        }
    }

    if let Some(ref wl) = f.merchant_whitelist {
        if !wl.is_empty() && !wl.contains(&p.merchant_npub) {
            return false;
        }
    }

    if let Some(ref bl) = f.merchant_blacklist {
        if bl.contains(&p.merchant_npub) {
            return false;
        }
    }

    // stall_ids filter not applicable in NIP-99

    if !f.platforms.is_empty()
        && !p.platforms.is_empty()
        && !p.platforms.iter().any(|listing_platform| {
            f.platforms
                .iter()
                .any(|requested| listing_platform == requested)
        })
    {
        return false;
    }

    true
}

// ── Relay fetch functions ─────────────────────────────────────────────────────
// `pub(crate)` so only `NostrClient` wrappers in `nostr.rs` can call these.
// This keeps `nostr_sdk::Client` encapsulated and lets NostrClient control
// relay connection lifecycle.

use crate::relay_manager::RelayManager;
use std::sync::Arc;
use tokio::sync::Mutex;

/// Fetch NIP-15 stalls (kind 30017) from connected relays.
///
/// * `relay_manager` — the relay manager for relay communication.
/// * `limit`      — maximum number of events to return.
/// * `since_days` — if `Some(n)`, restrict to events published in the last
///                  `n` days (open marketplace, no pubkey filter).
///
/// Events that fail to parse are silently skipped with a `tracing::warn`.
/// If the query times out, returns an empty list instead of failing.
pub(crate) async fn fetch_nip15_stalls_impl(
    relay_manager: &Arc<Mutex<RelayManager>>,
    limit: usize,
    since_days: Option<u64>,
) -> Result<Vec<Nip15Stall>, String> {
    let filter = build_filter(Kind::Custom(30017), limit, since_days, None, &[]);

    let manager = relay_manager.lock().await;

    // Log the filter we're using
    tracing::info!(
        "Fetching NIP-15 stalls with filter: kind=30017, limit={}, since_days={:?}",
        limit,
        since_days
    );

    // Use a longer timeout (30s) for marketplace queries and handle timeout gracefully
    let events = match manager.fetch_events_with_timeout(filter, 30).await {
        Ok(events) => {
            tracing::info!(
                "fetch_nip15_stalls: successfully received {} events",
                events.len()
            );
            events
        }
        Err(e) => {
            tracing::warn!(
                "fetch_nip15_stalls relay error: {} - returning empty list",
                e
            );
            // Return empty list on timeout instead of failing
            return Ok(Vec::new());
        }
    };

    let stalls: Vec<Nip15Stall> = events
        .into_iter()
        .filter_map(|ev| {
            parse_stall(ev)
                .map_err(|e| tracing::warn!("Skipping malformed stall event: {e}"))
                .ok()
        })
        .collect();

    tracing::info!("fetch_nip15_stalls: parsed {} valid stalls", stalls.len());
    Ok(stalls)
}

/// Fetch NIP-15 products (kind 30018) from connected relays.
///
/// * `relay_manager` — the relay manager for relay communication.
/// * `limit`      — maximum number of events to return.
/// * `since_days` — if `Some(n)`, restrict to events published in the last
///                  `n` days (open marketplace, no pubkey filter).
///
/// Events that fail to parse are silently skipped with a `tracing::warn`.
/// If the query times out, returns an empty list instead of failing.
pub(crate) async fn fetch_nip15_products_impl(
    relay_manager: &Arc<Mutex<RelayManager>>,
    limit: usize,
    since_days: Option<u64>,
) -> Result<Vec<Nip15Product>, String> {
    let filter = build_filter(Kind::Custom(30018), limit, since_days, None, &[]);

    let manager = relay_manager.lock().await;

    // Log the filter we're using
    tracing::info!(
        "Fetching NIP-15 products with filter: kind=30018, limit={}, since_days={:?}",
        limit,
        since_days
    );

    // Use a longer timeout (30s) for marketplace queries and handle timeout gracefully
    let events = match manager.fetch_events_with_timeout(filter, 30).await {
        Ok(events) => {
            tracing::info!(
                "fetch_nip15_products: successfully received {} events",
                events.len()
            );
            events
        }
        Err(e) => {
            tracing::warn!(
                "fetch_nip15_products relay error: {} - returning empty list",
                e
            );
            // Return empty list on timeout instead of failing
            return Ok(Vec::new());
        }
    };

    let products: Vec<Nip15Product> = events
        .into_iter()
        .filter_map(|ev| {
            parse_product(ev)
                .map_err(|e| tracing::warn!("Skipping malformed product event: {e}"))
                .ok()
        })
        .collect();

    tracing::info!(
        "fetch_nip15_products: parsed {} valid products",
        products.len()
    );
    Ok(products)
}

/// Fetch NIP-15 products (kind 30018) with streaming results from each relay.
///
/// * `relay_manager` — the relay manager for relay communication.
/// * `limit`      — maximum number of events to return.
/// * `since_days` — if `Some(n)`, restrict to events published in the last
///                  `n` days (open marketplace, no pubkey filter).
/// * `on_product` — callback invoked for each unique product as it arrives.
///
/// Returns the total count of unique products found.
///
/// Products are deduplicated by ID (first occurrence wins).
/// Events that fail to parse are silently skipped.
pub async fn fetch_nip15_products_streaming<F>(
    relay_manager: &Arc<tokio::sync::Mutex<RelayManager>>,
    limit: usize,
    since_days: Option<u64>,
    mut on_product: F,
) -> Result<u32, String>
where
    F: FnMut(Nip15Product) + Send + 'static,
{
    use std::collections::HashSet;
    use tokio::sync::Mutex;

    let filter = build_filter(Kind::Custom(30018), limit, since_days, None, &[]);

    tracing::info!(
        "Streaming NIP-15 products: kind=30018, limit={}, since_days={:?}",
        limit,
        since_days
    );

    let manager = relay_manager.lock().await;

    // Track seen product IDs for deduplication
    let seen_ids: Arc<Mutex<HashSet<String>>> = Arc::new(Mutex::new(HashSet::new()));
    let product_count: Arc<Mutex<u32>> = Arc::new(Mutex::new(0));

    let seen_ids_clone = Arc::clone(&seen_ids);
    let product_count_clone = Arc::clone(&product_count);

    // Stream events from relays
    let result = manager
        .fetch_events_streaming(
            filter,
            5, // 5s timeout per relay
            5, // 5s inactivity timeout
            move |_relay_url, events| {
                // Parse events and emit products
                for event in events {
                    match parse_product(event) {
                        Ok(product) => {
                            // Deduplicate by ID - use try_lock to avoid blocking in async context
                            if let Ok(mut seen) = seen_ids_clone.try_lock() {
                                if !seen.contains(&product.id) {
                                    seen.insert(product.id.clone());
                                    drop(seen); // Explicitly drop to release lock

                                    // Update count
                                    if let Ok(mut count) = product_count_clone.try_lock() {
                                        *count += 1;
                                        drop(count); // Explicitly drop to release lock
                                    }

                                    // Emit product
                                    on_product(product);
                                }
                            }
                        }
                        Err(e) => {
                            tracing::debug!("Skipping malformed product event: {}", e);
                        }
                    }
                }
            },
        )
        .await;

    let count = match result {
        Ok(_) => {
            let count = *product_count.lock().await;
            tracing::info!("Streaming fetch complete: {} unique products", count);
            count
        }
        Err(e) => {
            tracing::warn!("Streaming fetch ended with error: {}", e);
            *product_count.lock().await
        }
    };

    Ok(count)
}

// ── Internal parsing helpers ──────────────────────────────────────────────────

fn build_filter(
    kind: Kind,
    limit: usize,
    since_days: Option<u64>,
    until_secs: Option<u64>,
    tag_filter: &[&str],
) -> Filter {
    let since_secs = since_days.map(|days| {
        // Saturating sub guards against underflow on very large values.
        Timestamp::now().as_secs().saturating_sub(days * 86_400)
    });
    build_filter_since_secs(kind, limit, since_secs, until_secs, tag_filter)
}

fn build_filter_since_secs(
    kind: Kind,
    limit: usize,
    since_secs: Option<u64>,
    until_secs: Option<u64>,
    tag_filter: &[&str],
) -> Filter {
    let mut f = Filter::new().kind(kind).limit(limit);
    if let Some(since) = since_secs {
        f = f.since(Timestamp::from(since));
    }
    if let Some(until) = until_secs {
        f = f.until(Timestamp::from(until));
    }
    if !tag_filter.is_empty() {
        f = f.custom_tags(
            SingleLetterTag::lowercase(Alphabet::T),
            tag_filter
                .iter()
                .map(|tag| (*tag).to_string())
                .collect::<Vec<_>>(),
        );
    }
    f
}

fn parse_stall(event: Event) -> Result<Nip15Stall, serde_json::Error> {
    let c: StallContent = serde_json::from_str(&event.content)?;
    Ok(Nip15Stall {
        id: c.id,
        name: c.name,
        description: c.description,
        currency: c.currency,
        shipping: c.shipping,
        merchant_npub: npub_of(&event.pubkey),
        created_at: event.created_at.as_secs(),
    })
}

fn parse_product(event: Event) -> Result<Nip15Product, serde_json::Error> {
    let c: ProductContent = serde_json::from_str(&event.content)?;

    // t-tags carry the product categories. We parse them in a version-agnostic
    // way (inspecting the raw tag name string) to avoid nostr-sdk API churn.
    let categories: Vec<String> = event
        .tags
        .iter()
        .filter_map(|tag| {
            let v = tag.clone().to_vec();
            if v.first().map(String::as_str) == Some("t") {
                v.get(1).cloned()
            } else {
                None
            }
        })
        .collect();

    Ok(Nip15Product {
        id: c.id,
        stall_id: c.stall_id,
        name: c.name,
        description: c.description,
        images: c.images,
        currency: c.currency,
        price: c.price,
        quantity: c.quantity,
        // Convert [[key, value], ...] arrays to (String, String) tuples.
        specs: c.specs.into_iter().map(|[k, v]| (k, v)).collect(),
        shipping: c.shipping,
        categories,
        merchant_npub: npub_of(&event.pubkey),
        created_at: event.created_at.as_secs(),
    })
}

fn parse_listing(event: Event) -> Result<Nip99Listing, String> {
    #[cfg(debug_assertions)]
    let raw_event_json = serde_json::to_string(&event).ok();

    let mut id: Option<String> = None;
    let mut title: Option<String> = None;
    let mut summary: Option<String> = None;
    let mut published_at: Option<i64> = None;
    let mut location: Option<String> = None;
    let mut geohash: Option<String> = None;
    let mut status: Option<String> = None;
    let mut price_amount: Option<String> = None;
    let mut price_currency: Option<String> = None;
    let mut price_frequency: Option<String> = None;
    let mut images: Vec<String> = Vec::new();
    let mut tags: Vec<String> = Vec::new();
    let mut platforms: Vec<String> = Vec::new();
    let mut nip94_event_id: Option<String> = None;
    let mut servers = Vec::new();
    let mut file_hash = None;
    let mut version = None;
    let mut fulfillment_authorizations = Vec::new();
    let mut malformed_fulfillment_authorization_tags = Vec::new();
    let mut seen_fulfillment_authorization_tags = HashSet::new();
    let mut acquisition = AcquisitionPolicy::Gated;
    let mut acquisition_seen = false;
    let mut campaigns = Vec::new();

    for tag in event.tags.iter() {
        let v = tag.clone().to_vec();
        match v.first().map(String::as_str) {
            Some("d") => {
                if id.is_none() {
                    id = v.get(1).cloned();
                }
            }
            Some("title") => {
                if title.is_none() {
                    title = v.get(1).cloned();
                }
            }
            Some("summary") => {
                if summary.is_none() {
                    summary = v.get(1).cloned();
                }
            }
            Some("published_at") => {
                if published_at.is_none() {
                    published_at = v.get(1).and_then(|s| s.parse::<i64>().ok());
                }
            }
            Some("location") => {
                if location.is_none() {
                    location = v.get(1).cloned();
                }
            }
            Some("g") => {
                if geohash.is_none() {
                    geohash = v.get(1).cloned();
                }
            }
            Some("status") => {
                if status.is_none() {
                    status = v.get(1).cloned();
                }
            }
            Some("price") => {
                if price_amount.is_none() {
                    price_amount = v.get(1).cloned();
                }
                if price_currency.is_none() {
                    price_currency = v.get(2).cloned();
                }
                if price_frequency.is_none() {
                    price_frequency = v.get(3).cloned();
                }
            }
            Some("image") => {
                if let Some(url) = v.get(1).cloned() {
                    images.push(url);
                }
            }
            Some("t") => {
                if let Some(tag_value) = v.get(1).cloned() {
                    tags.push(tag_value);
                }
            }
            Some("platform") => {
                if let Some(platform) = v.get(1).cloned() {
                    platforms.push(platform);
                }
            }
            Some("nip94") => {
                if nip94_event_id.is_none() {
                    nip94_event_id = v.get(1).cloned();
                }
            }
            Some("server") => {
                if let Some(server) = v.get(1).filter(|server| !server.is_empty()) {
                    servers.push(server.clone());
                }
            }
            Some("file_hash") => {
                if file_hash.is_none() {
                    file_hash = v.get(1).filter(|hash| !hash.is_empty()).cloned();
                }
            }
            Some("version") => {
                if version.is_none() {
                    version = v.get(1).filter(|value| !value.is_empty()).cloned();
                }
            }
            Some("fulfillment_authorization") => {
                if !seen_fulfillment_authorization_tags.insert(v.clone()) {
                    continue;
                }
                let parsed = match v.as_slice() {
                    [_, root, pubkey] => nostr::EventId::from_hex(root)
                        .ok()
                        .zip(nostr::PublicKey::from_hex(pubkey).ok())
                        .map(|(root_event_id, fulfillment_pubkey)| {
                            FulfillmentAuthorizationReference {
                                root_event_id,
                                fulfillment_pubkey,
                                relay_hint: None,
                            }
                        }),
                    [_, root, pubkey, relay] if !relay.is_empty() => nostr::EventId::from_hex(root)
                        .ok()
                        .zip(nostr::PublicKey::from_hex(pubkey).ok())
                        .zip(nostr::RelayUrl::parse(relay).ok())
                        .map(|((root_event_id, fulfillment_pubkey), _)| {
                            FulfillmentAuthorizationReference {
                                root_event_id,
                                fulfillment_pubkey,
                                relay_hint: Some(relay.clone()),
                            }
                        }),
                    _ => None,
                };
                if let Some(reference) = parsed {
                    fulfillment_authorizations.push(reference);
                } else {
                    malformed_fulfillment_authorization_tags.push(v);
                }
            }
            Some("acquisition") if !acquisition_seen => {
                acquisition_seen = true;
                acquisition = match v.as_slice() {
                    [_, mode] if mode == "public" => AcquisitionPolicy::Public,
                    [_, mode, starts, ends] if mode == "timed-access" => {
                        match (starts.parse::<u64>(), ends.parse::<u64>()) {
                            (Ok(starts_at), Ok(ends_at)) if starts_at < ends_at => {
                                AcquisitionPolicy::TimedAccess { starts_at, ends_at }
                            }
                            _ => AcquisitionPolicy::Gated,
                        }
                    }
                    _ => AcquisitionPolicy::Gated,
                };
            }
            Some("campaign") => {
                let relay_hint = match v.as_slice() {
                    [_, _] => Some(None),
                    [_, _, relay] if nostr::RelayUrl::parse(relay).is_ok() => {
                        Some(Some(relay.clone()))
                    }
                    _ => None,
                };
                if let (Some(root), Some(relay_hint)) = (v.get(1), relay_hint) {
                    if let Ok(root_event_id) = nostr::EventId::from_hex(root) {
                        campaigns.push(CampaignPointer {
                            root_event_id,
                            relay_hint,
                        });
                    }
                }
            }
            _ => {}
        }
    }

    let id = id.ok_or_else(|| "Missing required d tag".to_string())?;
    let title = title.ok_or_else(|| "Missing required title tag".to_string())?;

    Ok(Nip99Listing {
        event_id: event.id.to_hex(),
        id,
        title,
        content: event.content,
        summary,
        published_at,
        location,
        price_amount,
        price_currency,
        price_frequency,
        images,
        geohash,
        tags,
        platforms,
        nip94_event_id,
        servers,
        file_hash,
        version,
        fulfillment_authorizations,
        malformed_fulfillment_authorization_tags,
        acquisition,
        campaigns,
        status,
        #[cfg(debug_assertions)]
        raw_event_json,
        merchant_npub: npub_of(&event.pubkey),
        created_at: event.created_at.as_secs(),
    })
}

/// Returns true when `target_coordinate` appears from at least `min_relays` distinct relays.
pub(crate) fn listing_seen_on_min_relays(
    relay_events: &[(String, Vec<Event>)],
    target_coordinate: &str,
    min_relays: usize,
) -> bool {
    use std::collections::HashSet;

    let mut relays = HashSet::new();
    for (relay_url, events) in relay_events {
        if events
            .iter()
            .any(|event| event_matches_coordinate(event, target_coordinate))
        {
            relays.insert(relay_url.clone());
        }
    }
    relays.len() >= min_relays
}

type RelayEventsByUrl = Vec<(String, Vec<Event>)>;

/// Confirms a NIP-99 listing is visible from at least `min_relays` distinct relays.
pub async fn confirm_nip99_listing_propagated(
    relay_manager: &Arc<Mutex<RelayManager>>,
    target_coordinate: &str,
    min_relays: usize,
) -> Result<bool, String> {
    use std::sync::{Arc as StdArc, Mutex as StdMutex};

    let filter = build_filter_since_secs(Kind::Custom(30402), 50, None, None, &["game"]);
    let relay_events: StdArc<StdMutex<RelayEventsByUrl>> = StdArc::new(StdMutex::new(Vec::new()));
    let relay_events_clone = StdArc::clone(&relay_events);

    let manager = relay_manager.lock().await;
    manager
        .fetch_events_streaming(filter, 5, 5, move |relay_url, events| {
            if let Ok(mut collected) = relay_events_clone.lock() {
                collected.push((relay_url.to_string(), events));
            }
        })
        .await
        .map_err(|err| err.to_string())?;
    drop(manager);

    let collected = relay_events
        .lock()
        .map_err(|_| "relay propagation collection mutex poisoned".to_string())?;
    Ok(listing_seen_on_min_relays(
        &collected,
        target_coordinate,
        min_relays,
    ))
}

fn event_matches_coordinate(event: &Event, target_coordinate: &str) -> bool {
    parse_listing(event.clone())
        .map(|listing| {
            format!("30402:{}:{}", event.pubkey.to_hex(), listing.id) == target_coordinate
        })
        .unwrap_or(false)
}

pub(crate) async fn fetch_nip99_listings_impl(
    relay_manager: &Arc<Mutex<RelayManager>>,
    limit: usize,
    since_days: Option<u64>,
) -> Result<Vec<Nip99Listing>, String> {
    let since_secs =
        since_days.map(|days| Timestamp::now().as_secs().saturating_sub(days * 86_400));
    fetch_nip99_listings_since_impl(relay_manager, limit, since_secs).await
}

pub(crate) async fn fetch_nip99_listings_since_impl(
    relay_manager: &Arc<Mutex<RelayManager>>,
    limit: usize,
    since_secs: Option<u64>,
) -> Result<Vec<Nip99Listing>, String> {
    let filter = build_filter_since_secs(Kind::Custom(30402), limit, since_secs, None, &["game"]);

    let manager = relay_manager.lock().await;

    tracing::info!(
        "Fetching NIP-99 listings with filter: kind=30402, limit={}, since_secs={:?}",
        limit,
        since_secs
    );

    let events = match manager.fetch_events_with_timeout(filter, 30).await {
        Ok(events) => {
            tracing::info!(
                "fetch_nip99_listings: successfully received {} events",
                events.len()
            );
            events
        }
        Err(e) => {
            tracing::warn!(
                "fetch_nip99_listings relay error: {} - returning empty list",
                e
            );
            return Ok(Vec::new());
        }
    };

    let listings: Vec<Nip99Listing> = events
        .into_iter()
        .filter_map(|ev| {
            parse_listing(ev)
                .map_err(|e| tracing::warn!("Skipping malformed listing event: {e}"))
                .ok()
        })
        .collect();

    tracing::info!(
        "fetch_nip99_listings: parsed {} valid listings",
        listings.len()
    );
    Ok(listings)
}

pub async fn fetch_nip99_listings_streaming<F>(
    relay_manager: &Arc<tokio::sync::Mutex<RelayManager>>,
    limit: usize,
    since_days: Option<u64>,
    until_secs: Option<u64>,
    on_product: F,
) -> Result<u32, String>
where
    F: FnMut(Nip99Listing) + Send + 'static,
{
    let since_secs =
        since_days.map(|days| Timestamp::now().as_secs().saturating_sub(days * 86_400));
    fetch_nip99_listings_streaming_since(relay_manager, limit, since_secs, until_secs, on_product)
        .await
}

fn record_replaceable_event(
    seen: &mut std::collections::HashMap<String, (u64, String)>,
    coordinate: String,
    candidate_created_at: u64,
    candidate_event_id: String,
) -> (bool, bool) {
    use std::collections::hash_map::Entry;

    match seen.entry(coordinate) {
        Entry::Vacant(entry) => {
            entry.insert((candidate_created_at, candidate_event_id));
            (true, true)
        }
        Entry::Occupied(mut entry) => {
            let current = entry.get();
            if is_replaceable_event_newer(
                candidate_created_at,
                Some(candidate_event_id.as_str()),
                current.0,
                Some(current.1.as_str()),
            ) {
                entry.insert((candidate_created_at, candidate_event_id));
                (true, false)
            } else {
                (false, false)
            }
        }
    }
}

pub async fn fetch_nip99_listings_streaming_since<F>(
    relay_manager: &Arc<tokio::sync::Mutex<RelayManager>>,
    limit: usize,
    since_secs: Option<u64>,
    until_secs: Option<u64>,
    mut on_product: F,
) -> Result<u32, String>
where
    F: FnMut(Nip99Listing) + Send + 'static,
{
    use std::collections::HashMap;
    use tokio::sync::Mutex;

    let filter = build_filter_since_secs(
        Kind::Custom(30402),
        limit,
        since_secs,
        until_secs,
        &["game"],
    );

    tracing::info!(
        "Streaming NIP-99 listings: kind=30402, limit={}, since_secs={:?}, until_secs={:?}",
        limit,
        since_secs,
        until_secs
    );

    let manager = relay_manager.lock().await;

    let seen_coordinates: Arc<Mutex<HashMap<String, (u64, String)>>> =
        Arc::new(Mutex::new(HashMap::new()));
    let product_count: Arc<Mutex<u32>> = Arc::new(Mutex::new(0));

    let seen_coordinates_clone = Arc::clone(&seen_coordinates);
    let product_count_clone = Arc::clone(&product_count);

    let result = manager
        .fetch_events_streaming(filter, 5, 5, move |_relay_url, events| {
            for event in events {
                let coordinate_pubkey = event.pubkey.to_hex();
                match parse_listing(event) {
                    Ok(product) => {
                        if let Ok(mut seen) = seen_coordinates_clone.try_lock() {
                            let coordinate = format!("30402:{coordinate_pubkey}:{}", product.id);
                            let (accepted, is_first_coordinate) = record_replaceable_event(
                                &mut seen,
                                coordinate,
                                product.created_at,
                                product.event_id.clone(),
                            );
                            if accepted {
                                drop(seen);

                                if is_first_coordinate {
                                    if let Ok(mut count) = product_count_clone.try_lock() {
                                        *count += 1;
                                        drop(count);
                                    }
                                }

                                on_product(product);
                            }
                        }
                    }
                    Err(e) => {
                        tracing::debug!("Skipping malformed listing event: {}", e);
                    }
                }
            }
        })
        .await;

    let count = match result {
        Ok(_) => {
            let count = *product_count.lock().await;
            tracing::info!("Streaming fetch complete: {} unique listings", count);
            count
        }
        Err(e) => {
            tracing::warn!("Streaming fetch ended with error: {}", e);
            *product_count.lock().await
        }
    };

    Ok(count)
}

/// Convert a `PublicKey` to bech32 `npub`, falling back to hex on error.
fn npub_of(pubkey: &PublicKey) -> String {
    pubkey.to_bech32().unwrap_or_else(|_| pubkey.to_string())
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use nostr_sdk::{Event, EventBuilder, Keys, Kind, Tag, TagKind};

    fn make_product(price: f64, currency: &str, categories: &[&str], npub: &str) -> Nip15Product {
        Nip15Product {
            id: "prod-1".into(),
            stall_id: "stall-1".into(),
            name: "Test Game".into(),
            description: None,
            images: vec![],
            currency: currency.into(),
            price,
            quantity: None,
            specs: vec![],
            shipping: vec![],
            categories: categories.iter().map(|s| s.to_string()).collect(),
            merchant_npub: npub.into(),
            created_at: 0,
        }
    }

    fn make_nip99_listing(id: &str, platforms: &[&str]) -> Nip99Listing {
        Nip99Listing {
            event_id: String::new(),
            id: id.into(),
            title: format!("Listing {id}"),
            content: "Markdown body".into(),
            summary: None,
            published_at: None,
            location: None,
            price_amount: Some("1000".into()),
            price_currency: Some("SATS".into()),
            price_frequency: None,
            images: vec![],
            geohash: None,
            tags: vec![],
            status: Some("active".into()),
            merchant_npub: "npub1merchant".into(),
            created_at: 0,
            platforms: platforms
                .iter()
                .map(|platform| (*platform).into())
                .collect(),
            nip94_event_id: None,
            servers: Vec::new(),
            file_hash: None,
            version: None,
            fulfillment_authorizations: Vec::new(),
            malformed_fulfillment_authorization_tags: Vec::new(),
            acquisition: AcquisitionPolicy::Gated,
            campaigns: Vec::new(),
            raw_event_json: None,
        }
    }

    fn make_nip99_event(kind: u16, content: &str, tags: Vec<Vec<&str>>) -> Event {
        let keys = Keys::generate();
        let mut builder = EventBuilder::new(Kind::Custom(kind), content);

        for tag in tags {
            let tag_name = tag.first().expect("tag must have a name").to_string();
            let values = tag
                .into_iter()
                .skip(1)
                .map(std::string::ToString::to_string)
                .collect::<Vec<String>>();
            builder = builder.tag(Tag::custom(TagKind::Custom(tag_name.into()), values));
        }

        builder
            .sign_with_keys(&keys)
            .expect("NIP-99 event should sign")
    }

    #[test]
    fn replaceable_event_ordering_covers_timestamp_and_event_id_cases() {
        let cases = [
            (11, Some("z"), 10, Some("a"), true),
            (9, Some("a"), 10, Some("z"), false),
            (10, Some("a"), 10, Some("b"), true),
            (10, Some("b"), 10, Some("a"), false),
            (10, Some("a"), 10, Some("a"), false),
            (10, Some("a"), 10, None, true),
            (10, None, 10, Some("a"), false),
            (10, None, 10, None, false),
        ];

        for (candidate_created_at, candidate_id, current_created_at, current_id, expected) in cases
        {
            assert_eq!(
                is_replaceable_event_newer(
                    candidate_created_at,
                    candidate_id,
                    current_created_at,
                    current_id,
                ),
                expected,
                "candidate=({candidate_created_at}, {candidate_id:?}), current=({current_created_at}, {current_id:?})"
            );
        }
    }

    #[test]
    fn recording_replaceable_events_counts_only_the_first_coordinate() {
        let mut seen = std::collections::HashMap::new();
        let coordinate = "30402:publisher:game".to_string();
        let mut count = 0;

        let first = record_replaceable_event(&mut seen, coordinate.clone(), 10, "bbb".to_string());
        count += u32::from(first.1);
        assert_eq!(count, 1);

        let replacement =
            record_replaceable_event(&mut seen, coordinate.clone(), 20, "ccc".to_string());
        count += u32::from(replacement.1);
        assert_eq!(count, 1);

        let stale = record_replaceable_event(&mut seen, coordinate, 15, "aaa".to_string());
        count += u32::from(stale.1);

        assert_eq!(first, (true, true));
        assert_eq!(replacement, (true, false));
        assert_eq!(stale, (false, false));
        assert_eq!(count, 1);
        assert_eq!(seen.len(), 1);
        assert_eq!(seen.values().next(), Some(&(20, "ccc".to_string())));
    }

    #[test]
    fn nip99_listing_preserves_raw_event_json_for_debug_panel() {
        // Arrange
        let event = make_nip99_event(
            30402,
            "# Debug Listing\nRaw content",
            vec![
                vec!["d", "listing-debug-raw"],
                vec!["title", "Debug Raw Listing"],
                vec!["price", "2100", "SATS"],
                vec!["image", "https://example.com/image.png"],
            ],
        );
        let expected_id = event.id.to_string();

        // Act
        let listing = parse_listing(event).expect("listing should parse");
        let raw_event_json = listing
            .raw_event_json
            .as_deref()
            .expect("raw event JSON should be retained in debug builds");
        let raw_event: serde_json::Value =
            serde_json::from_str(raw_event_json).expect("raw event JSON should parse");

        // Assert
        assert_eq!(raw_event["id"].as_str(), Some(expected_id.as_str()));
        assert_eq!(raw_event["kind"].as_u64(), Some(30_402));
        assert_eq!(
            raw_event["content"].as_str(),
            Some("# Debug Listing\nRaw content")
        );
    }

    #[test]
    fn listing_event_uses_kind_30402() {
        let event = make_nip99_event(
            30402,
            "# Listing\nActive listing in markdown",
            vec![vec!["d", "listing-01"], vec!["title", "Test Listing"]],
        );

        let listing = parse_listing(event).expect("active listing should parse");
        assert_eq!(listing.id, "listing-01");
    }

    #[test]
    fn adp_listing_preserves_publisher_management_fields() {
        let event = make_nip99_event(
            30402,
            "ADP listing",
            vec![
                vec!["d", "managed-game"],
                vec!["title", "Managed Game"],
                vec!["server", "http://localhost:9099"],
                vec!["file_hash", "abc123"],
                vec!["version", "1.4.2"],
                vec!["fulfillment_pubkey", "delegate-key", "1700000000", ""],
            ],
        );
        let event_id = event.id.to_hex();

        let listing = parse_listing(event).expect("ADP listing should parse");

        assert_eq!(listing.event_id, event_id);
        assert_eq!(listing.servers, vec!["http://localhost:9099"]);
        assert_eq!(listing.file_hash.as_deref(), Some("abc123"));
        assert_eq!(listing.version.as_deref(), Some("1.4.2"));
        assert!(listing.fulfillment_authorizations.is_empty());
        assert!(listing.malformed_fulfillment_authorization_tags.is_empty());
    }

    #[test]
    fn valid_authorization_siblings_survive_malformed_entries_and_same_key_roots() {
        let fulfillment = Keys::generate().public_key().to_hex();
        let first_root = EventId::from_hex(&"11".repeat(32)).expect("root");
        let second_root = EventId::from_hex(&"22".repeat(32)).expect("root");
        let event = make_nip99_event(
            30402,
            "ADP listing",
            vec![
                vec!["d", "managed-game"],
                vec!["title", "Managed Game"],
                vec![
                    "fulfillment_authorization",
                    &first_root.to_hex(),
                    &fulfillment,
                ],
                vec!["fulfillment_authorization", "bad", "bad"],
                vec![
                    "fulfillment_authorization",
                    &second_root.to_hex(),
                    &fulfillment,
                ],
                vec![
                    "fulfillment_authorization",
                    &first_root.to_hex(),
                    &fulfillment,
                ],
            ],
        );
        let listing = parse_listing(event).expect("listing");
        assert_eq!(listing.fulfillment_authorizations.len(), 2);
        assert_eq!(listing.malformed_fulfillment_authorization_tags.len(), 1);
        assert_eq!(
            listing.fulfillment_authorizations[0].root_event_id,
            first_root
        );
        assert_eq!(
            listing.fulfillment_authorizations[1].root_event_id,
            second_root
        );
    }

    #[test]
    fn draft_listing_uses_kind_30403() {
        let event = make_nip99_event(
            30403,
            "# Draft\nDraft listing in markdown",
            vec![vec!["d", "listing-draft"], vec!["title", "Draft Listing"]],
        );

        let listing = parse_listing(event).expect("draft listing should parse");
        assert_eq!(listing.id, "listing-draft");
    }

    #[test]
    fn content_is_markdown_string() {
        let markdown = "## Indie Adventure\nA cozy marketplace listing";
        let event = make_nip99_event(
            30402,
            markdown,
            vec![vec!["d", "listing-md"], vec!["title", "Markdown Listing"]],
        );

        let listing = parse_listing(event).expect("listing should parse");
        assert!(!listing.content.trim().is_empty());
        assert!(listing.content.contains("##"));
    }

    #[test]
    fn title_tag_is_present() {
        let event = make_nip99_event(
            30402,
            "content",
            vec![vec!["d", "listing-title"], vec!["title", "The Game Title"]],
        );

        let listing = parse_listing(event).expect("listing should parse");
        assert_eq!(listing.title, "The Game Title");
    }

    #[test]
    fn published_at_tag_is_unix_timestamp() {
        let timestamp = "1700000000";
        let event = make_nip99_event(
            30402,
            "content",
            vec![
                vec!["d", "listing-published"],
                vec!["title", "Published Listing"],
                vec!["published_at", timestamp],
            ],
        );

        let listing = parse_listing(event).expect("listing should parse");
        let parsed_timestamp = listing
            .published_at
            .expect("published_at should be present");
        assert_eq!(parsed_timestamp, 1_700_000_000_i64);
    }

    #[test]
    fn price_tag_currency_codes() {
        let event = make_nip99_event(
            30402,
            "content",
            vec![
                vec!["d", "listing-price"],
                vec!["title", "Priced Listing"],
                vec!["price", "100", "SATS"],
            ],
        );

        let listing = parse_listing(event).expect("listing should parse");
        assert_eq!(listing.price_amount.as_deref(), Some("100"));
        let currency = listing
            .price_currency
            .as_deref()
            .expect("price currency should be present");
        assert_eq!(currency, "SATS");
        assert!(currency
            .chars()
            .all(|ch| !ch.is_ascii_alphabetic() || ch.is_ascii_uppercase()));
    }

    #[test]
    fn summary_tag_when_summary_present() {
        let event = make_nip99_event(
            30402,
            "content",
            vec![
                vec!["d", "listing-summary"],
                vec!["title", "Summary Listing"],
                vec!["summary", "Short summary text"],
            ],
        );

        let listing = parse_listing(event).expect("listing should parse");
        assert_eq!(listing.summary.as_deref(), Some("Short summary text"));
    }

    #[test]
    fn location_tag_when_location_present() {
        let event = make_nip99_event(
            30402,
            "content",
            vec![
                vec!["d", "listing-location"],
                vec!["title", "Location Listing"],
                vec!["location", "Remote"],
            ],
        );

        let listing = parse_listing(event).expect("listing should parse");
        assert_eq!(listing.location.as_deref(), Some("Remote"));
    }

    #[test]
    fn platform_tags_are_captured() {
        let event = make_nip99_event(
            30402,
            "content",
            vec![
                vec!["d", "listing-platforms"],
                vec!["title", "Platform Listing"],
                vec!["platform", "linux-x86_64"],
                vec!["platform", "windows-x86_64"],
            ],
        );

        let listing = parse_listing(event).expect("listing should parse");
        assert_eq!(listing.platforms, vec!["linux-x86_64", "windows-x86_64"]);
    }

    #[test]
    fn nip94_tag_is_captured() {
        let event = make_nip99_event(
            30402,
            "content",
            vec![
                vec!["d", "listing-nip94"],
                vec!["title", "NIP-94 Listing"],
                vec!["nip94", "event-id-01"],
                vec!["nip94", "event-id-02"],
            ],
        );

        let listing = parse_listing(event).expect("listing should parse");
        assert_eq!(listing.nip94_event_id.as_deref(), Some("event-id-01"));
    }

    #[test]
    fn legacy_listing_json_defaults_delivery_metadata() {
        let json = r#"
        {
            "id": "legacy-listing",
            "title": "Legacy Listing",
            "content": "Markdown body",
            "summary": null,
            "published_at": null,
            "location": null,
            "price_amount": "1000",
            "price_currency": "SATS",
            "price_frequency": null,
            "images": [],
            "geohash": null,
            "tags": [],
            "status": "active",
            "merchant_npub": "npub1merchant",
            "created_at": 0
        }
        "#;

        let listing: Nip99Listing =
            serde_json::from_str(json).expect("legacy listing should deserialize");

        assert!(listing.platforms.is_empty());
        assert_eq!(listing.nip94_event_id, None);
    }

    #[test]
    fn build_filter_includes_game_tag_filter() {
        // Arrange / Act
        let filter = build_filter(Kind::Custom(30402), 50, None, None, &["game"]);
        let value = serde_json::to_value(&filter).expect("filter should serialize");

        // Assert
        assert_eq!(value.get("#t"), Some(&serde_json::json!(["game"])));
    }

    #[test]
    fn build_filter_empty_tag_filter_omits_t_tag() {
        // Arrange / Act
        let filter = build_filter(Kind::Custom(30018), 50, None, None, &[]);
        let value = serde_json::to_value(&filter).expect("filter should serialize");

        // Assert
        assert!(value.get("#t").is_none());
    }

    #[test]
    fn build_filter_since_secs_uses_absolute_timestamp() {
        // Arrange / Act
        let filter =
            build_filter_since_secs(Kind::Custom(30402), 50, Some(1_710_030_000), None, &[]);
        let value = serde_json::to_value(&filter).expect("filter should serialize");

        // Assert
        assert_eq!(value.get("since"), Some(&serde_json::json!(1_710_030_000)));
    }

    #[test]
    fn default_filter_passes_everything() {
        let products = vec![
            make_product(0.0, "SATS", &["game"], "npub1alice"),
            make_product(1000.0, "USD", &["software"], "npub1bob"),
        ];
        let result = apply_filter(products.clone(), &MarketplaceFilter::default());
        assert_eq!(result.len(), 2);
    }

    #[test]
    fn free_only_filter() {
        let products = vec![
            make_product(0.0, "SATS", &[], "npub1alice"),
            make_product(500.0, "SATS", &[], "npub1bob"),
        ];
        let filter = MarketplaceFilter {
            free_only: Some(true),
            ..Default::default()
        };
        let result = apply_filter(products, &filter);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].price, 0.0);
    }

    #[test]
    fn price_range_filter() {
        let products = vec![
            make_product(100.0, "SATS", &[], "npub1a"),
            make_product(500.0, "SATS", &[], "npub1b"),
            make_product(2000.0, "SATS", &[], "npub1c"),
        ];
        let filter = MarketplaceFilter {
            min_price: Some(200.0),
            max_price: Some(1000.0),
            ..Default::default()
        };
        let result = apply_filter(products, &filter);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].price, 500.0);
    }

    #[test]
    fn currency_filter_case_insensitive() {
        let products = vec![
            make_product(10.0, "SATS", &[], "npub1a"),
            make_product(10.0, "usd", &[], "npub1b"),
            make_product(10.0, "EUR", &[], "npub1c"),
        ];
        let filter = MarketplaceFilter {
            currencies: Some(vec!["sats".into(), "USD".into()]),
            ..Default::default()
        };
        let result = apply_filter(products, &filter);
        assert_eq!(result.len(), 2);
    }

    #[test]
    fn category_filter() {
        let products = vec![
            make_product(0.0, "SATS", &["rpg", "indie"], "npub1a"),
            make_product(0.0, "SATS", &["shooter"], "npub1b"),
        ];
        let filter = MarketplaceFilter {
            categories: Some(vec!["rpg".into()]),
            ..Default::default()
        };
        let result = apply_filter(products, &filter);
        assert_eq!(result.len(), 1);
    }

    #[test]
    fn merchant_whitelist_and_blacklist() {
        let products = vec![
            make_product(0.0, "SATS", &[], "npub1alice"),
            make_product(0.0, "SATS", &[], "npub1bob"),
            make_product(0.0, "SATS", &[], "npub1carol"),
        ];
        // whitelist includes alice and bob, blacklist removes bob
        let filter = MarketplaceFilter {
            merchant_whitelist: Some(vec!["npub1alice".into(), "npub1bob".into()]),
            merchant_blacklist: Some(vec!["npub1bob".into()]),
            ..Default::default()
        };
        let result = apply_filter(products, &filter);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].merchant_npub, "npub1alice");
    }

    #[test]
    fn stall_filter() {
        let mut p2 = make_product(0.0, "SATS", &[], "npub1a");
        p2.stall_id = "stall-2".into();

        let products = vec![make_product(0.0, "SATS", &[], "npub1a"), p2];
        let filter = MarketplaceFilter {
            stall_ids: Some(vec!["stall-1".into()]),
            ..Default::default()
        };
        let result = apply_filter(products, &filter);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].stall_id, "stall-1");
    }

    #[test]
    fn matching_platform_filter_passes_listing() {
        let listings = vec![make_nip99_listing("linux-game", &["linux-x86_64"])];
        let filter = MarketplaceFilter {
            platforms: vec!["linux-x86_64".into()],
            ..Default::default()
        };

        let result = apply_filter_nip99(listings, &filter);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].id, "linux-game");
    }

    #[test]
    fn nonmatching_platform_filter_removes_listing() {
        let listings = vec![make_nip99_listing("windows-game", &["windows-x86_64"])];
        let filter = MarketplaceFilter {
            platforms: vec!["linux-x86_64".into()],
            ..Default::default()
        };

        let result = apply_filter_nip99(listings, &filter);
        assert!(result.is_empty());
    }

    #[test]
    fn propagation_count_requires_target_listing_from_two_relays() {
        let event = make_nip99_event(
            30402,
            "Title",
            vec![vec!["d", "game"], vec!["title", "Title"], vec!["t", "game"]],
        );
        let target = format!("30402:{}:game", event.pubkey.to_hex());
        let relays = vec![
            ("wss://relay-one.example".to_string(), vec![event.clone()]),
            ("wss://relay-two.example".to_string(), vec![event]),
        ];

        assert!(listing_seen_on_min_relays(&relays, &target, 2));
    }

    #[test]
    fn propagation_count_rejects_single_relay_visibility() {
        let event = make_nip99_event(
            30402,
            "Title",
            vec![vec!["d", "game"], vec!["title", "Title"], vec!["t", "game"]],
        );
        let target = format!("30402:{}:game", event.pubkey.to_hex());
        let relays = vec![("wss://relay-one.example".to_string(), vec![event])];

        assert!(!listing_seen_on_min_relays(&relays, &target, 2));
    }

    #[test]
    fn propagation_count_ignores_wrong_coordinates() {
        let event = make_nip99_event(
            30402,
            "Title",
            vec![
                vec!["d", "other-game"],
                vec!["title", "Title"],
                vec!["t", "game"],
            ],
        );
        let relays = vec![
            ("wss://relay-one.example".to_string(), vec![event.clone()]),
            ("wss://relay-two.example".to_string(), vec![event]),
        ];

        assert!(!listing_seen_on_min_relays(&relays, "30402:pubkey:game", 2));
    }

    #[test]
    fn unrestricted_listing_passes_platform_filter() {
        let listings = vec![make_nip99_listing("any-platform-game", &[])];
        let filter = MarketplaceFilter {
            platforms: vec!["linux-x86_64".into()],
            ..Default::default()
        };

        let result = apply_filter_nip99(listings, &filter);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].id, "any-platform-game");
    }

    #[test]
    fn explicit_public_acquisition_is_parsed() {
        let event = make_nip99_event(
            30402,
            "Description",
            vec![
                vec!["d", "game"],
                vec!["title", "Game"],
                vec!["acquisition", "public"],
            ],
        );

        let listing = parse_listing(event).expect("listing parses");
        assert_eq!(listing.acquisition, AcquisitionPolicy::Public);
        assert!(listing.acquisition.allows_access_at(1));
    }

    #[test]
    fn timed_access_obeys_half_open_interval() {
        let event = make_nip99_event(
            30402,
            "Description",
            vec![
                vec!["d", "game"],
                vec!["title", "Game"],
                vec!["acquisition", "timed-access", "100", "200"],
            ],
        );

        let listing = parse_listing(event).expect("listing parses");
        assert!(listing.acquisition.allows_access_at(100));
        assert!(listing.acquisition.allows_access_at(199));
        assert!(!listing.acquisition.allows_access_at(200));
    }

    #[test]
    fn malformed_timed_access_fails_closed() {
        for acquisition in [
            vec!["acquisition", "timed-access", "bad", "200"],
            vec!["acquisition", "timed-access", "200", "100"],
            vec!["acquisition", "timed-access", "100"],
        ] {
            let event = make_nip99_event(
                30402,
                "Description",
                vec![vec!["d", "game"], vec!["title", "Game"], acquisition],
            );

            let listing = parse_listing(event).expect("listing remains parseable");
            assert_eq!(listing.acquisition, AcquisitionPolicy::Gated);
        }
    }

    #[test]
    fn zero_price_without_acquisition_remains_gated() {
        let event = make_nip99_event(
            30402,
            "Description",
            vec![
                vec!["d", "game"],
                vec!["title", "Game"],
                vec!["price", "0", "SATS"],
            ],
        );

        let listing = parse_listing(event).expect("listing parses");
        assert_eq!(listing.acquisition, AcquisitionPolicy::Gated);
    }

    #[test]
    fn campaign_pointer_preserves_optional_relay_hint() {
        let event_id = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let event = make_nip99_event(
            30402,
            "Description",
            vec![
                vec!["d", "game"],
                vec!["title", "Game"],
                vec!["campaign", event_id, "wss://relay.example.com"],
            ],
        );

        let listing = parse_listing(event).expect("listing parses");
        assert_eq!(listing.campaigns.len(), 1);
        assert_eq!(listing.campaigns[0].root_event_id.to_hex(), event_id);
        assert_eq!(
            listing.campaigns[0].relay_hint.as_deref(),
            Some("wss://relay.example.com")
        );
    }

    #[test]
    fn malformed_campaign_pointer_is_ignored() {
        let event = make_nip99_event(
            30402,
            "Description",
            vec![
                vec!["d", "game"],
                vec!["title", "Game"],
                vec!["campaign", "not-an-event-id", "not-a-relay"],
            ],
        );

        let listing = parse_listing(event).expect("listing parses");
        assert!(listing.campaigns.is_empty());
    }
}
