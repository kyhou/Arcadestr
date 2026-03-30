// Profile fetching with batching and caching
// Based on Wisp's MetadataFetcher pattern

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};
use std::num::NonZeroUsize;

use lru::LruCache;
use nostr_sdk::prelude::*;

use crate::nostr::{NostrClient, NostrError, UserProfile, UserProfileContent};

/// Configuration constants
pub const BATCH_SIZE: usize = 200;
pub const MAX_PROFILE_ATTEMPTS: u32 = 2;
pub const PROFILE_CACHE_SIZE: usize = 5000;
pub const CACHE_TTL_SECONDS: u64 = 86400; // 24 hours

/// Trait for cache backends - allows swapping implementations later
pub trait ProfileCache: Send + Sync {
    fn get(&self, npub: &str) -> Option<UserProfile>;
    fn put(&self, npub: String, profile: UserProfile);
    fn contains(&self, npub: &str) -> bool;
}

/// In-memory LRU cache implementation
pub struct LruProfileCache {
    inner: Arc<Mutex<LruCache<String, CachedProfile>>>,
    ttl_seconds: u64,
}

struct CachedProfile {
    profile: UserProfile,
    timestamp: u64,
}

impl LruProfileCache {
    pub fn new(capacity: usize, ttl_seconds: u64) -> Self {
        let non_zero_capacity = NonZeroUsize::new(capacity).unwrap_or(NonZeroUsize::new(100).unwrap());
        Self {
            inner: Arc::new(Mutex::new(LruCache::new(non_zero_capacity))),
            ttl_seconds,
        }
    }

    fn now() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0)
    }
}

impl ProfileCache for LruProfileCache {
    fn get(&self, npub: &str) -> Option<UserProfile> {
        let mut cache = self.inner.lock().ok()?;
        let cached = cache.get(npub)?;
        
        // Check if expired
        let now = Self::now();
        if now - cached.timestamp > self.ttl_seconds {
            cache.pop(npub);
            return None;
        }
        
        Some(cached.profile.clone())
    }

    fn put(&self, npub: String, profile: UserProfile) {
        if let Ok(mut cache) = self.inner.lock() {
            cache.put(npub, CachedProfile {
                profile,
                timestamp: Self::now(),
            });
        }
    }

    fn contains(&self, npub: &str) -> bool {
        self.get(npub).is_some()
    }
}

/// Batched profile fetcher with queue management
pub struct ProfileFetcher {
    /// Pending profiles to fetch
    pending: Arc<Mutex<VecDeque<String>>>,
    /// Currently in-flight fetches (prevents duplicates)
    in_flight: Arc<Mutex<HashSet<String>>>,
    /// Failed profiles with attempt count
    failed_attempts: Arc<Mutex<HashMap<String, u32>>>,
    /// Cache backend (swappable)
    cache: Arc<dyn ProfileCache>,
    /// Maximum retry attempts
    max_attempts: u32,
    /// Batch size for fetching
    batch_size: usize,
}

impl ProfileFetcher {
    /// Create with default in-memory LRU cache
    pub fn new() -> Self {
        Self::with_cache(Arc::new(LruProfileCache::new(
            PROFILE_CACHE_SIZE,
            CACHE_TTL_SECONDS,
        )))
    }

    /// Create with custom cache backend
    pub fn with_cache(cache: Arc<dyn ProfileCache>) -> Self {
        Self {
            pending: Arc::new(Mutex::new(VecDeque::new())),
            in_flight: Arc::new(Mutex::new(HashSet::new())),
            failed_attempts: Arc::new(Mutex::new(HashMap::new())),
            cache,
            max_attempts: MAX_PROFILE_ATTEMPTS,
            batch_size: BATCH_SIZE,
        }
    }

    /// Queue a profile for fetching
    pub fn enqueue(&self, npub: String) {
        // Skip if already cached
        if self.cache.contains(&npub) {
            return;
        }

        let mut pending = self.pending.lock().unwrap();
        let in_flight = self.in_flight.lock().unwrap();
        let failed = self.failed_attempts.lock().unwrap();
        
        // Skip if already queued, in flight, or exhausted
        if pending.contains(&npub) 
            || in_flight.contains(&npub)
            || failed.get(&npub).map(|&c| c >= self.max_attempts).unwrap_or(false) {
            return;
        }
        
        pending.push_back(npub);
    }

    /// Queue multiple profiles at once
    pub fn enqueue_many(&self, npubs: Vec<String>) {
        for npub in npubs {
            self.enqueue(npub);
        }
    }

    /// Fetch all pending profiles in batches
    /// Returns (fetched_profiles, remaining_count)
    pub async fn fetch_batch(&self, client: &NostrClient) -> (Vec<UserProfile>, usize) {
        let mut results = Vec::new();
        
        // Collect batch of pending pubkeys
        let batch: Vec<String> = {
            let mut pending = self.pending.lock().unwrap();
            let mut in_flight = self.in_flight.lock().unwrap();
            let count = pending.len().min(self.batch_size);
            let batch: Vec<String> = pending.drain(..count).collect();
            for npub in &batch {
                in_flight.insert(npub.clone());
            }
            batch
        };
        
        if batch.is_empty() {
            return (results, 0);
        }
        
        tracing::info!("Fetching batch of {} profiles", batch.len());
        
        // Fetch from indexers first, then all relays
        match self.fetch_profiles_batch(client, &batch).await {
            Ok(profiles) => {
                for (npub, profile) in profiles {
                    results.push(profile.clone());
                    self.cache.put(npub, profile);
                }
            }
            Err(e) => {
                tracing::warn!("Failed to fetch profile batch: {}", e);
                // Mark as failed
                let mut failed = self.failed_attempts.lock().unwrap();
                for npub in &batch {
                    *failed.entry(npub.clone()).or_insert(0) += 1;
                }
            }
        }
        
        // Remove from in-flight
        let mut in_flight = self.in_flight.lock().unwrap();
        for npub in &batch {
            in_flight.remove(npub);
        }
        
        let remaining = self.pending.lock().unwrap().len();
        (results, remaining)
    }

    /// Fast path: fetch single profile immediately (for logged-in user or feed users)
    pub async fn fetch_single(&self, client: &NostrClient, npub: &str) -> Option<UserProfile> {
        tracing::info!("ProfileFetcher::fetch_single called for {}", npub);
        
        // Check cache first
        if let Some(cached) = self.cache.get(npub) {
            tracing::info!("Profile cache HIT for {}", npub);
            return Some(cached);
        }
        tracing::debug!("Profile cache MISS for {}", npub);
        
        // Check if already being fetched
        {
            let in_flight = self.in_flight.lock().unwrap();
            if in_flight.contains(npub) {
                tracing::debug!("Profile {} already in flight, skipping", npub);
                return None;
            }
        }
        
        // Parse pubkey
        let pubkey = match PublicKey::parse(npub) {
            Ok(pk) => pk,
            Err(e) => {
                tracing::error!("Failed to parse npub {}: {}", npub, e);
                return None;
            }
        };
        
        // Fetch immediately with priority
        let filter = Filter::new()
            .author(pubkey)
            .kind(Kind::Metadata)
            .limit(1);
        
        tracing::info!("Fetching profile from indexers for {}...", npub);
        match client.fetch_from_indexers_then_all(filter).await {
            Ok(events) => {
                tracing::debug!("fetch_single got {} events for {}", events.len(), npub);
                if let Some(event) = events.first() {
                    tracing::info!("Found profile event for {}, parsing...", npub);
                    match Self::parse_profile_event(event, npub) {
                        Ok(profile) => {
                            tracing::info!("Successfully parsed profile for {}: name={:?}", npub, profile.name);
                            self.cache.put(npub.to_string(), profile.clone());
                            return Some(profile);
                        }
                        Err(e) => {
                            tracing::error!("Failed to parse profile event for {}: {}", npub, e);
                        }
                    }
                } else {
                    tracing::warn!("No events returned for {} - profile not found", npub);
                }
            }
            Err(e) => tracing::error!("Fast path fetch FAILED for {}: {}", npub, e),
        }
        
        tracing::warn!("ProfileFetcher::fetch_single returning None for {}", npub);
        None
    }

    /// Check if profile is cached
    pub fn is_cached(&self, npub: &str) -> bool {
        self.cache.contains(npub)
    }

    /// Get cached profile
    pub fn get_cached(&self, npub: &str) -> Option<UserProfile> {
        self.cache.get(npub)
    }

    /// Get pending count
    pub fn pending_count(&self) -> usize {
        self.pending.lock().unwrap().len()
    }

    /// Get in-flight count
    pub fn in_flight_count(&self) -> usize {
        self.in_flight.lock().unwrap().len()
    }

    /// Clear all pending and failed state (for account switch)
    pub fn clear(&self) {
        self.pending.lock().unwrap().clear();
        self.in_flight.lock().unwrap().clear();
        self.failed_attempts.lock().unwrap().clear();
    }

    /// Internal: fetch a batch of profiles
    async fn fetch_profiles_batch(
        &self,
        client: &NostrClient,
        npubs: &[String],
    ) -> Result<Vec<(String, UserProfile)>, NostrError> {
        let mut results = Vec::new();
        
        // Build filter for all pubkeys
        let authors: Vec<PublicKey> = npubs
            .iter()
            .filter_map(|npub| PublicKey::parse(npub).ok())
            .collect();
        
        if authors.is_empty() {
            return Ok(results);
        }
        
        let filter = Filter::new()
            .authors(authors)
            .kind(Kind::Metadata);
        
        let events = client.fetch_from_indexers_then_all(filter).await?;
        
        // Parse events and match to requested pubkeys
        for event in events {
            let npub = event.pubkey.to_hex();
            if let Ok(profile) = Self::parse_profile_event(&event, &npub) {
                results.push((npub, profile));
            }
        }
        
        Ok(results)
    }

    /// Parse a profile event into UserProfile
    fn parse_profile_event(event: &Event, npub: &str) -> Result<UserProfile, NostrError> {
        // Parse the event content as UserProfileContent
        let content: UserProfileContent = serde_json::from_str(&event.content)
            .unwrap_or_default();

        Ok(UserProfile {
            npub: npub.to_string(),
            name: content.name,
            display_name: content.display_name,
            about: content.about,
            picture: content.picture,
            website: content.website,
            nip05: content.nip05,
            lud16: content.lud16,
            nip05_verified: false,
        })
    }
}

impl Default for ProfileFetcher {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_lru_cache_basic() {
        let cache = LruProfileCache::new(100, 3600);
        
        let profile = UserProfile {
            npub: "test".to_string(),
            name: Some("Test User".to_string()),
            ..Default::default()
        };
        
        cache.put("test".to_string(), profile.clone());
        assert!(cache.contains("test"));
        
        let retrieved = cache.get("test").unwrap();
        assert_eq!(retrieved.name, Some("Test User".to_string()));
    }

    #[test]
    fn test_profile_fetcher_enqueue() {
        let fetcher = ProfileFetcher::new();
        
        fetcher.enqueue("npub1test".to_string());
        fetcher.enqueue("npub1test".to_string()); // Duplicate, should be ignored
        
        assert_eq!(fetcher.pending_count(), 1);
    }
}
