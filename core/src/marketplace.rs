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
fn deserialize_optional_vec_shipping<'de, D>(deserializer: D) -> Result<Vec<ProductShipping>, D::Error>
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

#[derive(Debug, Deserialize)]
struct StallContent {
    id: String,
    name: String,
    description: Option<String>,
    currency: String,
    #[serde(default)]
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
    pub cost: f64,
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

// ── MarketplaceFilter ─────────────────────────────────────────────────────────

/// Describes every dimension along which the marketplace can be filtered.
///
/// Every field is `Option<_>` — `None` means "no restriction on this
/// dimension". `MarketplaceFilter::default()` therefore passes every product
/// through unmodified, making it safe to wire up now and restrict later.
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

    // ── Stall ─────────────────────────────────────────────────────────────────
    /// Restrict results to products from these stall UUIDs.
    /// Empty `Vec` = no restriction.
    pub stall_ids: Option<Vec<String>>,
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

    // ── Stall filter ──────────────────────────────────────────────────────────
    if let Some(ref stall_ids) = f.stall_ids {
        if !stall_ids.is_empty() && !stall_ids.contains(&p.stall_id) {
            return false;
        }
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
    let filter = build_filter(Kind::Custom(30017), limit, since_days);

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
    let filter = build_filter(Kind::Custom(30018), limit, since_days);

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
    
    let filter = build_filter(Kind::Custom(30018), limit, since_days);

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
    let result = manager.fetch_events_streaming(
        filter,
        5,  // 5s timeout per relay
        5,  // 5s inactivity timeout
        move |_relay_url, events| {
            // Parse events and emit products
            for event in events {
                match parse_product(event) {
                    Ok(product) => {
                        // Deduplicate by ID
                        let mut seen = seen_ids_clone.blocking_lock();
                        if !seen.contains(&product.id) {
                            seen.insert(product.id.clone());
                            drop(seen);  // Explicitly drop to release lock before callback
                            
                            // Update count
                            let mut count = product_count_clone.blocking_lock();
                            *count += 1;
                            drop(count);  // Explicitly drop to release lock before callback
                            
                            // Emit product
                            on_product(product);
                        }
                    }
                    Err(e) => {
                        tracing::debug!("Skipping malformed product event: {}", e);
                    }
                }
            }
        }
    ).await;

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

fn build_filter(kind: Kind, limit: usize, since_days: Option<u64>) -> Filter {
    let mut f = Filter::new().kind(kind).limit(limit);
    if let Some(days) = since_days {
        // Saturating sub guards against underflow on very large values.
        let since_unix = Timestamp::now().as_secs().saturating_sub(days * 86_400);
        f = f.since(Timestamp::from(since_unix));
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

/// Convert a `PublicKey` to bech32 `npub`, falling back to hex on error.
fn npub_of(pubkey: &PublicKey) -> String {
    pubkey.to_bech32().unwrap_or_else(|_| pubkey.to_string())
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

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
}
