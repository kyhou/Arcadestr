//! Global marketplace store for managing game listings across navigation.
//!
//! This store persists listings across view transitions (e.g., Browse → Detail → Browse)
//! to prevent data loss and reduce redundant network fetches.

use crate::models::GameListing;
use leptos::prelude::*;
use std::collections::HashMap;
use std::time::{Duration, Instant};

/// Default TTL for cached listings (5 minutes)
pub const DEFAULT_LISTING_TTL_SECS: u64 = 300;

/// Global marketplace store - reactive HashMap keyed by listing ID
#[derive(Clone, Debug)]
pub struct MarketplaceStore {
    listings: RwSignal<HashMap<String, GameListing>>,
    last_fetch_time: RwSignal<Option<Instant>>,
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
        self.listings.get().get(id).cloned()
    }

    /// Get all listings as a vector
    pub fn get_all(&self) -> Vec<GameListing> {
        self.listings.get().values().cloned().collect()
    }

    /// Add or update a single listing
    pub fn put(&self, listing: GameListing) {
        self.listings.update(|map| {
            map.insert(listing.id.clone(), listing);
        });
    }

    /// Add or update multiple listings
    pub fn put_many(&self, listings: Vec<GameListing>) {
        self.listings.update(|map| {
            for listing in listings {
                map.insert(listing.id.clone(), listing);
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
            // Deduplicate: only insert if not already present
            if !map.contains_key(&listing.id) {
                map.insert(listing.id.clone(), listing);
            }
        });
    }

    /// Check if a listing exists in the store
    pub fn has(&self, id: &str) -> bool {
        self.listings.get().contains_key(id)
    }

    /// Get the number of cached listings
    pub fn len(&self) -> usize {
        self.listings.get().len()
    }

    /// Check if the store is empty
    pub fn is_empty(&self) -> bool {
        self.listings.get().is_empty()
    }

    /// Check if the cache needs refresh based on TTL
    /// Returns true if cache is empty or last fetch was longer than ttl_secs ago
    pub fn needs_refresh(&self, ttl_secs: u64) -> bool {
        match self.last_fetch_time.get() {
            None => true,
            Some(last_fetch) => {
                let elapsed = last_fetch.elapsed();
                elapsed > Duration::from_secs(ttl_secs)
            }
        }
    }

    /// Update the last fetch timestamp to now
    pub fn mark_fresh(&self) {
        self.last_fetch_time.set(Some(Instant::now()));
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
            .get()
            .values()
            .filter(|l| l.publisher_npub == publisher_npub)
            .cloned()
            .collect()
    }
}

impl Default for MarketplaceStore {
    fn default() -> Self {
        Self::new()
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
