//! Global marketplace store for managing game listings across navigation.
//!
//! This store persists listings across view transitions (e.g., Browse → Detail → Browse)
//! to prevent data loss and reduce redundant network fetches.

use std::collections::HashMap;
#[cfg(not(target_arch = "wasm32"))]
use std::time::{SystemTime, UNIX_EPOCH};

use arcadestr_core::is_replaceable_event_newer;
use leptos::prelude::*;

use crate::models::GameListing;

/// Default TTL for cached listings (5 minutes)
pub const DEFAULT_LISTING_TTL_SECS: u64 = 300;

/// Global marketplace store - reactive HashMap keyed by full listing coordinate.
#[derive(Clone, Debug)]
pub struct MarketplaceStore {
    listings: RwSignal<HashMap<String, GameListing>>,
    last_fetch_time: RwSignal<Option<u64>>,
}

impl MarketplaceStore {
    /// Create a new empty marketplace store
    pub fn new() -> Self {
        Self {
            listings: RwSignal::new(HashMap::new()),
            last_fetch_time: RwSignal::new(None),
        }
    }

    /// Get a listing by ID
    pub fn get(&self, id: &str) -> Option<GameListing> {
        self.listings
            .get_untracked()
            .values()
            .find(|listing| listing.id == id)
            .cloned()
    }

    /// Get a listing by its publisher-scoped NIP-99 coordinate.
    pub fn get_by_coordinate(&self, publisher_npub: &str, id: &str) -> Option<GameListing> {
        self.listings
            .get_untracked()
            .get(&listing_coordinate(publisher_npub, id))
            .cloned()
    }

    /// Get all listings as a vector
    pub fn get_all(&self) -> Vec<GameListing> {
        self.listings.get_untracked().values().cloned().collect()
    }

    /// Add or update a single listing
    pub fn put(&self, listing: GameListing) {
        self.listings.update(|map| {
            map.insert(
                listing_coordinate(&listing.publisher_npub, &listing.id),
                listing,
            );
        });
    }

    /// Add or update multiple listings
    pub fn put_many(&self, listings: Vec<GameListing>) {
        self.listings.update(|map| {
            for listing in listings {
                map.insert(
                    listing_coordinate(&listing.publisher_npub, &listing.id),
                    listing,
                );
            }
        });
    }

    /// Add or update a single listing incrementally.
    ///
    /// Similar to `put()` but silently skips duplicates without logging.
    /// Use this for streaming updates where the same product may arrive
    /// from multiple relays.
    pub fn put_streaming(&self, listing: GameListing) {
        self.listings.update(|map| {
            let coordinate = listing_coordinate(&listing.publisher_npub, &listing.id);
            if map.get(&coordinate).map_or(true, |current| {
                is_replaceable_event_newer(
                    listing.created_at,
                    listing.event_id.as_deref(),
                    current.created_at,
                    current.event_id.as_deref(),
                )
            }) {
                map.insert(coordinate, listing);
            }
        });
    }

    /// Check if a listing exists in the store
    pub fn has(&self, id: &str) -> bool {
        self.listings
            .get_untracked()
            .values()
            .any(|listing| listing.id == id)
    }

    /// Get the number of cached listings
    pub fn len(&self) -> usize {
        self.listings.get_untracked().len()
    }

    /// Check if the store is empty
    pub fn is_empty(&self) -> bool {
        self.listings.get_untracked().is_empty()
    }

    /// Check if the cache needs refresh based on TTL
    /// Returns true if cache is empty or last fetch was longer than ttl_secs ago
    pub fn needs_refresh(&self, ttl_secs: u64) -> bool {
        match self.last_fetch_time.get_untracked() {
            None => true,
            Some(last_fetch) => {
                current_time_millis().saturating_sub(last_fetch) > ttl_secs.saturating_mul(1000)
            }
        }
    }

    /// Update the last fetch timestamp to now
    pub fn mark_fresh(&self) {
        self.last_fetch_time.set(Some(current_time_millis()));
    }

    /// Clear all listings and reset fetch time
    pub fn clear(&self) {
        self.listings.set(HashMap::new());
        self.last_fetch_time.set(None);
    }

    /// Get the raw signal for reactive access
    pub fn signal(&self) -> RwSignal<HashMap<String, GameListing>> {
        self.listings
    }

    /// Get listings by publisher npub
    pub fn get_by_publisher(&self, publisher_npub: &str) -> Vec<GameListing> {
        self.listings
            .get_untracked()
            .values()
            .filter(|l| l.publisher_npub == publisher_npub)
            .cloned()
            .collect()
    }
}

fn listing_coordinate(publisher_npub: &str, id: &str) -> String {
    format!("30402:{publisher_npub}:{id}")
}

#[cfg(target_arch = "wasm32")]
fn current_time_millis() -> u64 {
    js_sys::Date::now().max(0.0) as u64
}

#[cfg(not(target_arch = "wasm32"))]
fn current_time_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or_default()
}

impl Default for MarketplaceStore {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn listing(publisher_npub: &str, id: &str) -> GameListing {
        GameListing {
            id: id.into(),
            source: crate::models::ListingSource::Nip99Listing,
            title: id.into(),
            description: String::new(),
            images: Vec::new(),
            download_url: String::new(),
            price: 0.0,
            currency: "SATS".into(),
            price_sats: 0,
            quantity: None,
            tags: Vec::new(),
            specs: Vec::new(),
            publisher_npub: publisher_npub.into(),
            stall_id: String::new(),
            stall_name: None,
            lud16: String::new(),
            event_id: None,
            created_at: 0,
            platforms: Vec::new(),
            nip94_event_id: None,
            acquisition: crate::models::AcquisitionPolicy::Gated,
            campaigns: Vec::new(),
            is_owned: false,
            #[cfg(debug_assertions)]
            nip99_raw_event_json: None,
        }
    }

    #[test]
    fn listings_with_same_d_from_different_publishers_are_kept_separately() {
        let store = MarketplaceStore::new();
        store.put(listing("npub1publishera", "same-game"));
        store.put(listing("npub1publisherb", "same-game"));

        assert_eq!(store.len(), 2);
        assert_eq!(store.get_by_publisher("npub1publishera").len(), 1);
        assert_eq!(store.get_by_publisher("npub1publisherb").len(), 1);
    }

    #[test]
    fn publisher_management_filter_excludes_other_publishers_and_non_nip99_listings() {
        let own = listing("npub1publishera", "own-game");
        let other = listing("npub1publisherb", "other-game");
        let mut legacy = listing("npub1publishera", "legacy-game");
        legacy.source = crate::models::ListingSource::Legacy;

        let filtered = crate::campaign_management::current_user_listings(
            [other, legacy, own.clone()],
            "npub1publishera",
        );

        assert_eq!(filtered, vec![own.clone()]);
        assert_eq!(
            crate::campaign_management::listing_coordinate(&own),
            "30402:npub1publishera:own-game"
        );
    }

    #[test]
    fn fulfillment_signer_cannot_access_publisher_campaign_management() {
        let publisher_listing = listing("npub1publisher", "publisher-game");
        let visible = crate::campaign_management::current_user_listings(
            [publisher_listing],
            "npub1fulfillment",
        );
        assert!(visible.is_empty());
    }

    #[test]
    fn publisher_management_keeps_latest_replaceable_listing() {
        let mut old = listing("npub1publisher", "same-game");
        old.title = "Old title".into();
        old.created_at = 10;
        let mut updated = old.clone();
        updated.title = "Updated title".into();
        updated.created_at = 20;

        let visible = crate::campaign_management::current_user_listings(
            [old, updated.clone()],
            "npub1publisher",
        );

        assert_eq!(visible, vec![updated]);
    }

    #[test]
    fn streaming_store_rejects_stale_listing_after_newer_listing() {
        let store = MarketplaceStore::new();
        let mut old = listing("npub1publisher", "same-game");
        old.title = "Old title".into();
        old.created_at = 10;
        let mut updated = old.clone();
        updated.title = "Updated title".into();
        updated.created_at = 20;

        store.put_streaming(updated.clone());
        store.put_streaming(old);

        assert_eq!(
            store
                .get_by_coordinate("npub1publisher", "same-game")
                .as_ref(),
            Some(&updated)
        );
    }

    #[test]
    fn streaming_store_uses_lower_event_id_for_equal_timestamps_in_both_arrival_orders() {
        let mut lower_id = listing("npub1publisher", "same-game");
        lower_id.created_at = 20;
        lower_id.event_id = Some("aaa".into());
        let mut higher_id = lower_id.clone();
        higher_id.event_id = Some("bbb".into());

        for arrivals in [
            [higher_id.clone(), lower_id.clone()],
            [lower_id.clone(), higher_id.clone()],
        ] {
            let store = MarketplaceStore::new();
            for listing in arrivals {
                store.put_streaming(listing);
            }

            assert_eq!(
                store
                    .get_by_coordinate("npub1publisher", "same-game")
                    .as_ref(),
                Some(&lower_id)
            );
        }
    }
}

/// Provide the marketplace store as a context
pub fn provide_marketplace_store() {
    provide_context(MarketplaceStore::new());
}

/// Hook to access the marketplace store from any component
/// Panics if not provided - use only when you're sure the store is available
pub fn use_marketplace_store() -> MarketplaceStore {
    use_context::<MarketplaceStore>().expect("MarketplaceStore not provided")
}

/// Try to get the marketplace store without panicking
/// Returns None if the store hasn't been provided yet
pub fn try_use_marketplace_store() -> Option<MarketplaceStore> {
    use_context::<MarketplaceStore>()
}
