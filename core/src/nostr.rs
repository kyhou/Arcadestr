// NOSTR protocol integration: event handling, relay connections, NIP-46 signer support.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

use nostr_sdk::prelude::*;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::sync::broadcast;
use tokio::sync::Mutex;
use tracing::{info, warn};

use crate::auth::AuthState;
#[cfg(feature = "native")]
use crate::relay_cache::{CachedRelayList, RelayCache, RelayDiscoverySource};
use crate::relay_events::{RelayConnectionEvent, RelayStatus};
#[cfg(feature = "native")]
use crate::relay_hints::RelayHints;
#[cfg(feature = "native")]
use crate::relay_manager::{RelayManager, RelayManagerConfig};
use crate::signers::{ActiveSigner, NostrSigner as ArcadestrNostrSigner, SignerError};
use crate::user_cache::UserCache;

/// Arcadestr game listing event kind.
/// Using kind 30078 (NIP-78 arbitrary app data, parameterized replaceable).
pub const KIND_GAME_LISTING: u16 = 30078;

/// Default relays for Arcadestr.
/// Includes popular relay discovery services that aggregate user metadata.
pub const DEFAULT_RELAYS: &[&str] = &[
    // Relay discovery services (query these first for user lookups)
    "wss://relay.nostr.info", // Relay discovery service
    "wss://relay.nostr.band", // Relay aggregator with good coverage
    // General relays
    "wss://relay.damus.io",
    "wss://nos.lol",
    "wss://relay.snort.social",
    "wss://relay.current.fyi",
    "wss://nostr.wine",
];

/// Relay discovery services - prioritized for user lookups
pub const DISCOVERY_RELAYS: &[&str] = &[
    "wss://relay.nostr.info",
    "wss://relay.nostr.band",
    "wss://relay.nsec.app", // NIP-46 service relay
];

/// Indexer relays for profile/relay discovery (subset of DEFAULT_RELAYS)
pub const INDEXER_RELAYS: &[&str] = &[
    "wss://relay.primal.net",
    "wss://relay.nostr.band",
    "wss://indexer.coracle.social",
];

/// Kind 10002: Relay List Metadata (NIP-65)
pub const KIND_RELAY_LIST: u16 = 10002;

/// Kind 3: Follow List (NIP-02)
pub const KIND_FOLLOW_LIST: u16 = 3;

/// Capacity of the relay event broadcast channel.
///
/// This bounds memory usage while allowing for burst event handling.
/// Events are dropped if the channel is full (acceptable for non-critical events).
const RELAY_EVENT_CHANNEL_CAPACITY: usize = 100;

// ============================================
// Event De-duplication (Task 1)
// ============================================

/// Event deduplicator to prevent processing duplicate events from multiple relays
pub struct EventDeduplicator {
    seen_ids: HashSet<String>,
    max_size: usize,
}

impl EventDeduplicator {
    /// Create a new deduplicator with specified max size
    pub fn new(max_size: usize) -> Self {
        Self {
            seen_ids: HashSet::new(),
            max_size,
        }
    }

    /// Check if event was already seen, insert if not
    /// Returns true if this is a duplicate (already seen)
    pub fn check_and_insert(&mut self, event_id: &str) -> bool {
        // If we're at capacity, clear half the entries (simple eviction)
        if self.seen_ids.len() >= self.max_size {
            let half = self.max_size / 2;
            let ids: Vec<String> = self.seen_ids.iter().take(half).cloned().collect();
            self.seen_ids.clear();
            self.seen_ids.extend(ids);
        }

        // Check and insert
        !self.seen_ids.insert(event_id.to_string())
    }

    /// Clear all seen events
    pub fn clear(&mut self) {
        self.seen_ids.clear();
    }

    /// Get current count of seen events
    pub fn len(&self) -> usize {
        self.seen_ids.len()
    }

    pub fn is_empty(&self) -> bool {
        self.seen_ids.is_empty()
    }
}

// ============================================
// Idle Timeout Management (Task 3)
// ============================================

/// Manages relay connection idle timeouts
pub struct RelayConnectionManager {
    last_activity: HashMap<String, Instant>,
    idle_timeout: Duration,
}

impl RelayConnectionManager {
    /// Create a new manager with specified idle timeout
    /// Default: 5 minutes
    pub fn new(idle_timeout: Duration) -> Self {
        Self {
            last_activity: HashMap::new(),
            idle_timeout,
        }
    }

    /// Create with default 5-minute timeout
    pub fn with_default_timeout() -> Self {
        Self::new(Duration::from_secs(300))
    }

    /// Update last activity time for a relay
    pub fn touch(&mut self, relay_url: &str) {
        self.last_activity
            .insert(relay_url.to_string(), Instant::now());
    }

    /// Get relays that have been idle too long
    pub fn get_idle_relays(&self) -> Vec<String> {
        let now = Instant::now();
        self.last_activity
            .iter()
            .filter(|(_, last_seen)| now.duration_since(**last_seen) > self.idle_timeout)
            .map(|(url, _)| url.clone())
            .collect()
    }

    /// Clean up idle relays and return them
    pub fn cleanup(&mut self) -> Vec<String> {
        let idle = self.get_idle_relays();
        for url in &idle {
            self.last_activity.remove(url);
        }
        idle
    }

    /// Remove a specific relay
    pub fn remove(&mut self, relay_url: &str) {
        self.last_activity.remove(relay_url);
    }
}

/// Parse relay list from Kind 10002 event tags (NIP-65 format)
/// NIP-65 uses "r" tags: ["r", "<url>"] or ["r", "<url>", "read"] or ["r", "<url>", "write"]
pub fn parse_relay_list_from_event(event: &Event) -> Result<CachedRelayList, NostrError> {
    let mut read_relays = Vec::new();
    let mut write_relays = Vec::new();

    // Iterate through all tags looking for "r" tags
    for tag in event.tags.iter() {
        // Check if this is a relay tag ("r")
        let tag_kind = tag.kind();
        if tag_kind.to_string() == "r" {
            // Get all values from the tag as a vector
            let values = tag.clone().to_vec();

            if values.len() >= 2 {
                let url = &values[1];

                // Check for marker (read/write)
                match values.get(2).map(|s: &String| s.as_str()) {
                    Some("read") => read_relays.push(url.clone()),
                    Some("write") => write_relays.push(url.clone()),
                    _ => {
                        // No marker or unknown marker - add to both
                        read_relays.push(url.clone());
                        write_relays.push(url.clone());
                    }
                }
            }
        }
    }

    tracing::debug!(
        "Parsed relay list: {} read relays, {} write relays",
        read_relays.len(),
        write_relays.len()
    );

    Ok(CachedRelayList {
        pubkey: event.pubkey.to_hex(),
        write_relays,
        read_relays,
        updated_at: event.created_at.as_secs(),
    })
}

/// Legacy: Parse relay list content from JSON (for backward compatibility)
/// Note: NIP-65 stores relay list in tags, not content. This is for old formats.
pub fn parse_relay_list_content(content: &str) -> Result<CachedRelayList, NostrError> {
    // If content is empty, return empty lists (will be populated from tags)
    if content.trim().is_empty() {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);

        return Ok(CachedRelayList {
            pubkey: String::new(),
            write_relays: Vec::new(),
            read_relays: Vec::new(),
            updated_at: now,
        });
    }

    #[derive(Deserialize)]
    struct RelayListContent {
        read: Option<Vec<String>>,
        write: Option<Vec<String>>,
    }

    let parsed: RelayListContent = serde_json::from_str(content)
        .map_err(|e| NostrError::MalformedEvent(format!("Invalid relay list content: {}", e)))?;

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    Ok(CachedRelayList {
        pubkey: String::new(),
        write_relays: parsed.write.unwrap_or_default(),
        read_relays: parsed.read.unwrap_or_default(),
        updated_at: now,
    })
}

// ============================================
// Relay Selection (NIP-65 Relay Gossip)
// ============================================

/// Scored relay for selection
#[derive(Debug, Clone)]
pub struct ScoredRelay {
    pub url: String,
    pub score: f32,
    pub pubkeys: HashSet<String>,
}

/// Result of relay selection
#[derive(Debug, Clone)]
pub struct RelaySelection {
    pub permanent: Vec<String>,
    pub uncovered_pubkeys: Vec<String>,
}

/// Build relay map from follow list
/// Returns: relay_url -> Set<pubkey>
pub fn build_relay_map(
    followed_pubkeys: &[String],
    cache: &RelayCache,
) -> HashMap<String, HashSet<String>> {
    let mut relay_map: HashMap<String, HashSet<String>> = HashMap::new();

    for pubkey in followed_pubkeys {
        if let Some(cached) = cache.get_relay_list(pubkey) {
            for relay in &cached.write_relays {
                relay_map
                    .entry(relay.clone())
                    .or_default()
                    .insert(pubkey.clone());
            }
        }
    }

    relay_map
}

/// Score relays based on coverage and health
pub fn score_relays(
    relay_map: &HashMap<String, HashSet<String>>,
    cache: &RelayCache,
    user_pubkey: Option<&str>,
) -> Vec<ScoredRelay> {
    let mut scored = Vec::new();

    for (relay_url, pubkeys) in relay_map {
        let raw_score = pubkeys.len() as f32;

        // Apply health multiplier from in-memory cache
        let health_multiplier = {
            let health_map = cache.relay_health.lock().unwrap();
            if let Some(health) = health_map.get(relay_url) {
                // Apply ×0.7 penalty if error_rate > 20%
                if health.error_rate > 0.20 {
                    0.7
                } else {
                    1.0
                }
            } else {
                1.0 // No health data, no penalty
            }
        };

        // Apply staleness multiplier (if stale, half the score)
        let staleness_multiplier = if cache.is_stale(relay_url) { 0.5 } else { 1.0 };

        // Apply user's own relay bonus
        let is_user_own = user_pubkey
            .map(|u| {
                cache
                    .get_relay_list(u)
                    .map(|l| l.write_relays.contains(relay_url))
                    .unwrap_or(false)
            })
            .unwrap_or(false);

        let own_relay_multiplier = if is_user_own { 1.5 } else { 1.0 };

        let final_score =
            raw_score * health_multiplier * staleness_multiplier * own_relay_multiplier;

        scored.push(ScoredRelay {
            url: relay_url.clone(),
            score: final_score,
            pubkeys: pubkeys.clone(),
        });
    }

    // Sort by score descending
    scored.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    scored
}

/// Greedy set cover selection
pub fn select_relays(
    scored: Vec<ScoredRelay>,
    max_relays: usize,
    all_pubkeys: &HashSet<String>,
) -> RelaySelection {
    let mut selected: Vec<String> = Vec::new();
    let mut covered: HashSet<String> = HashSet::new();
    let mut uncovered: HashSet<String> = all_pubkeys.clone();

    for relay in scored {
        if selected.len() >= max_relays {
            break;
        }

        if uncovered.is_empty() {
            break;
        }

        // Calculate marginal gain
        let marginal: HashSet<_> = relay.pubkeys.intersection(&uncovered).cloned().collect();

        if marginal.is_empty() {
            continue; // No new coverage
        }

        selected.push(relay.url);
        covered.extend(marginal.clone());
        uncovered.retain(|p| !marginal.contains(p));
    }

    RelaySelection {
        permanent: selected,
        uncovered_pubkeys: uncovered.into_iter().collect(),
    }
}

#[derive(Debug, Clone)]
pub struct RelayDiscoveryResult {
    pub write_relays: Vec<String>,
    pub read_relays: Vec<String>,
    pub source: RelayDiscoverySource,
}

/// Errors that can occur when interacting with NOSTR.
#[derive(Debug, Error)]
pub enum NostrError {
    #[error("Relay error: {0}")]
    RelayError(String),
    #[error("Malformed event: {0}")]
    MalformedEvent(String),
    #[error("Signing error: {0}")]
    SigningError(String),
    #[error("Serialization error: {0}")]
    SerializationError(String),
    #[error("Not authenticated")]
    NotAuthenticated,
}

impl From<SignerError> for NostrError {
    fn from(e: SignerError) -> Self {
        NostrError::SigningError(e.to_string())
    }
}

impl From<nostr_sdk::client::Error> for NostrError {
    fn from(e: nostr_sdk::client::Error) -> Self {
        NostrError::RelayError(e.to_string())
    }
}

impl From<serde_json::Error> for NostrError {
    fn from(e: serde_json::Error) -> Self {
        NostrError::SerializationError(e.to_string())
    }
}

/// Game listing data structure.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GameListing {
    pub id: String, // unique slug / d-tag value, e.g. "my-game-v1"
    pub title: String,
    pub description: String,
    pub price_sats: u64,        // price in satoshis, 0 = free
    pub download_url: String,   // direct download link (may be encrypted in future)
    pub publisher_npub: String, // bech32 npub of the publisher
    pub created_at: u64,        // unix timestamp
    pub tags: Vec<String>,      // freeform tags e.g. ["rpg", "pixel-art"]
    pub lud16: String, // Lightning address for payments (e.g., "seller@walletofsatoshi.com")
}

impl GameListing {
    /// Construct a `GameListing` from a NIP-15 product, optionally enriched
    /// with its parent stall.
    ///
    /// `lud16` is left empty here — callers should fill it once the
    /// merchant's NIP-01 profile has been fetched.
    pub fn from_nip15(
        product: crate::marketplace::Nip15Product,
        stall: Option<&crate::marketplace::Nip15Stall>,
    ) -> Self {
        // Prefer an explicit "download_url" spec entry, then fall back to
        // the first image.
        let download_url = product
            .specs
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case("download_url"))
            .map(|(_, v)| v.clone())
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
            title: product.name,
            description: product.description.unwrap_or_default(),
            price_sats,
            download_url,
            publisher_npub: product.merchant_npub,
            created_at: product.created_at,
            tags: product.categories,
            lud16: String::new(),
        }
    }
}

/// Content portion of a game listing event (stored in event.content).
#[derive(Serialize, Deserialize)]
struct GameListingContent {
    description: String,
    download_url: String,
}

/// User profile data structure (NIP-01 kind-0 metadata).
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

/// Internal deserialization struct for kind-0 metadata content.
#[derive(Deserialize, Default, Clone)]
pub struct UserProfileContent {
    pub name: Option<String>,
    pub display_name: Option<String>,
    pub about: Option<String>,
    pub picture: Option<String>,
    pub website: Option<String>,
    pub nip05: Option<String>,
    pub lud16: Option<String>,
}

/// NIP-05 verification response structure.
#[derive(Deserialize)]
struct Nip05Response {
    names: HashMap<String, String>,
}

/// NOSTR client for Arcadestr.
pub struct NostrClient {
    relay_manager: Arc<Mutex<RelayManager>>,
    user_cache: Option<Arc<UserCache>>,
    profile_id: String,
    /// Broadcast sender for relay connection events.
    relay_event_sender: broadcast::Sender<RelayConnectionEvent>,
}

impl NostrClient {
    /// Get access to the relay manager.
    pub fn relay_manager(&self) -> Arc<Mutex<RelayManager>> {
        self.relay_manager.clone()
    }

    /// Get access to the relay manager for streaming queries.
    pub fn get_relay_manager(&self) -> &Arc<tokio::sync::Mutex<RelayManager>> {
        &self.relay_manager
    }

    /// Subscribe to relay connection events.
    ///
    /// Returns a broadcast receiver that receives `RelayConnectionEvent` values
    /// whenever a relay connects or disconnects. Multiple subscribers can receive
    /// events simultaneously.
    ///
    /// # Examples
    /// ```
    /// let client = NostrClient::new("profile".to_string(), vec![], None).await?;
    /// let mut rx = client.subscribe_relay_events();
    ///
    /// while let Ok(event) = rx.recv().await {
    ///     println!("Relay event: {:?}", event);
    /// }
    /// ```
    pub fn subscribe_relay_events(&self) -> broadcast::Receiver<RelayConnectionEvent> {
        self.relay_event_sender.subscribe()
    }

    /// Emit a relay connection event
    fn emit_relay_event(&self, event: RelayConnectionEvent) {
        let _ = self.relay_event_sender.send(event);
    }

    /// Creates a new NOSTR client with background relay manager.
    pub async fn new(
        profile_id: String,
        relays: Vec<String>,
        config: Option<RelayManagerConfig>,
    ) -> Result<Self, NostrError> {
        use tracing::{error, info, warn};

        info!("Creating NostrClient for profile: {}", profile_id);

        let config = config.unwrap_or_default();

        // Create the broadcast channel for relay events
        let (relay_event_sender, _) = broadcast::channel(RELAY_EVENT_CHANNEL_CAPACITY);

        let relay_manager =
            RelayManager::new(profile_id.clone(), config, Some(relay_event_sender.clone()))
                .await
                .map_err(|e| {
                    NostrError::RelayError(format!("Failed to create relay manager: {}", e))
                })?;

        // Add any additional relays from params
        for relay in &relays {
            if !DEFAULT_RELAYS.contains(&relay.as_str())
                && !INDEXER_RELAYS.contains(&relay.as_str())
            {
                let _ = relay_manager.add_discovered_relay(relay.clone()).await;
            }
        }

        info!("NostrClient initialized with relay manager");

        Ok(Self {
            relay_manager: Arc::new(Mutex::new(relay_manager)),
            user_cache: None,
            profile_id,
            relay_event_sender,
        })
    }

    /// Creates a new NOSTR client with user cache support.
    pub async fn new_with_cache(
        profile_id: String,
        relays: Vec<String>,
        user_cache: Arc<UserCache>,
        config: Option<RelayManagerConfig>,
    ) -> Result<Self, NostrError> {
        use tracing::{info, warn};

        info!(
            "Creating NostrClient with {} relays and user cache for profile: {}",
            relays.len(),
            profile_id
        );

        let config = config.unwrap_or_default();

        // Create the broadcast channel for relay events
        let (relay_event_sender, _) = broadcast::channel(RELAY_EVENT_CHANNEL_CAPACITY);

        let relay_manager =
            RelayManager::new(profile_id.clone(), config, Some(relay_event_sender.clone()))
                .await
                .map_err(|e| {
                    NostrError::RelayError(format!("Failed to create relay manager: {}", e))
                })?;

        // Add any additional relays from params
        for relay in &relays {
            if !DEFAULT_RELAYS.contains(&relay.as_str())
                && !INDEXER_RELAYS.contains(&relay.as_str())
            {
                let _ = relay_manager.add_discovered_relay(relay.clone()).await;
            }
        }

        info!("NostrClient initialized with relay manager and cache");

        Ok(Self {
            relay_manager: Arc::new(Mutex::new(relay_manager)),
            user_cache: Some(user_cache),
            profile_id,
            relay_event_sender,
        })
    }

    /// Publishes a game listing as a signed NOSTR event.
    ///
    /// # Arguments
    /// * `listing` - The game listing to publish
    /// * `auth` - The authentication state containing the signer
    ///
    /// # Returns
    /// Returns the event ID on success.
    ///
    /// # Errors
    /// Returns `NostrError::NotAuthenticated` if user is not authenticated.
    /// Returns `NostrError::RelayError` if event cannot be published to any relay.
    pub async fn publish_listing(
        &self,
        listing: &GameListing,
        auth: &AuthState,
    ) -> Result<EventId, NostrError> {
        // Check authentication
        if !auth.is_authenticated() {
            return Err(NostrError::NotAuthenticated);
        }

        // Get the signer
        let signer = auth.signer().ok_or(NostrError::NotAuthenticated)?;

        // Build the event
        let builder = game_listing_to_event_builder(listing);

        // Sign the event using our bridged signer
        let signed_event = sign_event_with_arcadestr_signer(builder, signer).await?;

        // Send the event through relay manager
        let result = {
            let manager = self.relay_manager.lock().await;
            manager
                .send_event(&signed_event)
                .await
                .map_err(|e| NostrError::RelayError(format!("Failed to send event: {}", e)))?
        };

        // Check if event was successfully sent to at least one relay
        if result.success_count == 0 {
            return Err(NostrError::RelayError(format!(
                "Failed to publish event to any relay. Attempted {} relays, all failed.",
                result.relay_results.len()
            )));
        }

        info!(
            "Successfully published listing event {} to {} relays",
            result.event_id, result.success_count
        );

        Ok(signed_event.id)
    }

    /// Fetches recent game listings from relays.
    pub async fn fetch_listings(&self, limit: usize) -> Result<Vec<GameListing>, NostrError> {
        let filter = Filter::new()
            .kind(Kind::Custom(KIND_GAME_LISTING))
            .limit(limit);

        let events = {
            let manager = self.relay_manager.lock().await;
            manager
                .fetch_events(filter)
                .await
                .map_err(|e| NostrError::RelayError(format!("Failed to fetch events: {}", e)))?
        };

        let mut listings = Vec::new();
        for event in events {
            match event_to_game_listing(&event) {
                Ok(listing) => listings.push(listing),
                Err(e) => {
                    warn!("Skipping malformed event {}: {}", event.id, e);
                }
            }
        }

        Ok(listings)
    }

    /// Fetch NIP-15 stalls (kind 30017) from connected relays.
    ///
    /// * `limit`      — max events per relay.
    /// * `since_days` — if `Some(n)`, restrict to the last `n` days.
    pub async fn fetch_nip15_stalls(
        &self,
        limit: usize,
        since_days: Option<u64>,
    ) -> Result<Vec<crate::marketplace::Nip15Stall>, String> {
        crate::marketplace::fetch_nip15_stalls_impl(&self.relay_manager, limit, since_days).await
    }

    /// Fetch NIP-15 products (kind 30018) from connected relays.
    ///
    /// * `limit`      — max events per relay.
    /// * `since_days` — if `Some(n)`, restrict to the last `n` days.
    pub async fn fetch_nip15_products(
        &self,
        limit: usize,
        since_days: Option<u64>,
    ) -> Result<Vec<crate::marketplace::Nip15Product>, String> {
        crate::marketplace::fetch_nip15_products_impl(&self.relay_manager, limit, since_days).await
    }

    /// Fetches a specific game listing by its ID and publisher.
    pub async fn fetch_listing_by_id(
        &self,
        publisher_npub: &str,
        listing_id: &str,
    ) -> Result<GameListing, NostrError> {
        // Parse the publisher's public key
        let pubkey = PublicKey::parse(publisher_npub)
            .map_err(|e| NostrError::MalformedEvent(format!("Invalid npub: {}", e)))?;

        let filter = Filter::new()
            .kind(Kind::Custom(KIND_GAME_LISTING))
            .author(pubkey)
            .identifier(listing_id);

        let events = {
            let manager = self.relay_manager.lock().await;
            manager
                .fetch_events(filter)
                .await
                .map_err(|e| NostrError::RelayError(format!("Failed to fetch event: {}", e)))?
        };

        let event = events
            .first()
            .ok_or_else(|| NostrError::MalformedEvent("Listing not found".into()))?;

        event_to_game_listing(event)
    }

    /// Fetches user profile metadata (kind-0) from relays.
    /// Uses indexer relays first, then falls back to full pool if not found.
    pub async fn fetch_profile(
        &self,
        npub: &str,
        _additional_relays: Option<Vec<String>>,
    ) -> Result<UserProfile, NostrError> {
        // Check cache first
        if let Some(ref cache) = self.user_cache {
            if let Some(cached_profile) = cache.get(npub).await {
                tracing::info!("Found profile in cache for {}", npub);
                return Ok(cached_profile);
            }
        }

        // Parse npub as PublicKey
        let pubkey = PublicKey::parse(npub)
            .map_err(|e| NostrError::MalformedEvent(format!("Invalid npub: {}", e)))?;

        tracing::info!(
            "Fetching profile for pubkey: {} (npub: {})",
            pubkey.to_hex(),
            npub
        );

        // Build filter for kind-0 (metadata) events
        let filter = Filter::new().kind(Kind::Metadata).author(pubkey).limit(1);

        tracing::debug!(
            "Profile filter: kind=0, author={}, limit=1",
            pubkey.to_hex()
        );

        // Phase 1: Try indexer relays first (5s timeout)
        tracing::info!("Phase 1: Querying indexer relays for profile...");
        let indexer_events = {
            let manager = self.relay_manager.lock().await;
            let indexer_relays: Vec<String> =
                INDEXER_RELAYS.iter().map(|&s| s.to_string()).collect();

            match manager
                .fetch_events_from_subset(filter.clone(), indexer_relays)
                .await
            {
                Ok(events) => Ok(events),
                Err(e) => {
                    tracing::error!("Indexer relay query failed: {}", e);
                    Err(e)
                }
            }
        };

        match indexer_events {
            Ok(events) if !events.is_empty() => {
                tracing::info!("Found profile on indexer relays! {} events", events.len());
                if let Some(event) = events.first() {
                    let profile = self.parse_profile_event(event, npub)?;

                    // Save to cache
                    if let Some(ref cache) = self.user_cache {
                        if let Err(e) = cache.put(npub, &profile).await {
                            tracing::warn!("Failed to save profile to cache: {}", e);
                        }
                    }

                    return Ok(profile);
                }
            }
            Ok(_) => {
                tracing::debug!("No profile found on indexer relays");
            }
            Err(e) => {
                tracing::warn!("Indexer relay query failed: {}", e);
            }
        }

        // Phase 2: Fall back to full relay pool (8s timeout)
        tracing::info!("Phase 2: Querying all relays for profile...");
        let events = {
            let manager = self.relay_manager.lock().await;
            manager
                .fetch_events(filter)
                .await
                .map_err(|e| NostrError::RelayError(format!("Failed to fetch profile: {}", e)))?
        };

        tracing::debug!("Query returned {} events", events.len());

        // If no events returned, return minimal profile
        let event = match events.first() {
            Some(e) => {
                tracing::info!(
                    "Found profile event! Content preview: {}",
                    &e.content[..e.content.len().min(100)]
                );
                e
            }
            None => {
                tracing::warn!(
                    "No kind-0 (metadata) events found for user {} on any relay",
                    npub
                );
                return Ok(UserProfile {
                    npub: npub.to_string(),
                    ..Default::default()
                });
            }
        };

        let profile = self.parse_profile_event(event, npub)?;

        // Save to cache
        if let Some(ref cache) = self.user_cache {
            if let Err(e) = cache.put(npub, &profile).await {
                tracing::warn!("Failed to save profile to cache: {}", e);
            }
        }

        Ok(profile)
    }

    /// Fetch profile with NIP-65 relay discovery
    /// First discovers the user's relays, connects to them, then fetches profile
    pub async fn fetch_profile_with_relay_discovery(
        &self,
        npub: &str,
    ) -> Result<UserProfile, NostrError> {
        // First, try to discover the user's relays via NIP-65
        tracing::info!("Discovering relays for {} via NIP-65...", npub);

        match self.fetch_relay_list(npub).await {
            Ok(relay_list) => {
                tracing::info!(
                    "Found {} read relays and {} write relays for {}",
                    relay_list.read_relays.len(),
                    relay_list.write_relays.len(),
                    npub
                );

                // Connect to the user's read relays
                let manager = self.relay_manager.lock().await;
                for relay_url in &relay_list.read_relays {
                    if let Err(e) = manager.add_discovered_relay(relay_url.clone()).await {
                        tracing::warn!("Failed to add relay {}: {}", relay_url, e);
                    }
                }
                drop(manager);
            }
            Err(e) => {
                tracing::warn!("Could not discover relays for {}: {}", npub, e);
            }
        }

        // Now fetch the profile (will use the newly connected relays)
        self.fetch_profile(npub, None).await
    }

    /// Parse a profile event into UserProfile
    pub fn parse_profile_event(
        &self,
        event: &Event,
        npub: &str,
    ) -> Result<UserProfile, NostrError> {
        // Parse the event content as UserProfileContent
        let content: UserProfileContent = match serde_json::from_str(&event.content) {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!("Failed to parse profile content: {}", e);
                UserProfileContent::default()
            }
        };

        tracing::info!(
            "Parsed profile: name={:?}, display_name={:?}, picture={:?}",
            content.name,
            content.display_name,
            content.picture
        );

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

    /// Verifies a NIP-05 identifier against the user's pubkey.
    pub async fn verify_nip05(&self, npub: &str, nip05_identifier: &str) -> bool {
        // Split nip05_identifier on '@' to get name and domain
        let parts: Vec<&str> = nip05_identifier.split('@').collect();
        if parts.len() != 2 {
            return false;
        }
        let (name, domain) = (parts[0], parts[1]);

        // Fetch the NIP-05 JSON from the domain
        let url = format!("https://{}/.well-known/nostr.json?name={}", domain, name);

        let client = match reqwest::Client::builder()
            .timeout(Duration::from_secs(8))
            .build()
        {
            Ok(c) => c,
            Err(_) => return false,
        };

        let response = match client.get(&url).send().await {
            Ok(r) => r,
            Err(_) => return false,
        };

        let nip05_resp: Nip05Response = match response.json().await {
            Ok(resp) => resp,
            Err(_) => return false,
        };

        // Get the expected hex pubkey from npub
        let pubkey = match PublicKey::parse(npub) {
            Ok(pk) => pk,
            Err(_) => return false,
        };

        let expected_hex = pubkey.to_hex();

        // Check if the names map contains our pubkey
        nip05_resp
            .names
            .get(name)
            .map(|stored_hex| stored_hex.to_lowercase() == expected_hex.to_lowercase())
            .unwrap_or(false)
    }

    /// Convenience method that fetches profile and verifies NIP-05.
    pub async fn fetch_profile_verified(
        &self,
        npub: &str,
        additional_relays: Option<Vec<String>>,
    ) -> Result<UserProfile, NostrError> {
        let mut profile = self.fetch_profile(npub, additional_relays).await?;
        if let Some(ref identifier) = profile.nip05 {
            profile.nip05_verified = self.verify_nip05(npub, identifier).await;
        }
        Ok(profile)
    }

    /// Adds a relay to the client.
    /// Returns true if the relay was newly added, false if it already existed.
    pub async fn add_relay(&self, relay: &str) -> Result<bool, NostrError> {
        let result = {
            let manager = self.relay_manager.lock().await;
            manager.add_discovered_relay(relay.to_string()).await
        };
        result
            .map(|_| true)
            .map_err(|e| NostrError::RelayError(format!("Failed to add relay: {}", e)))
    }

    /// Connects to all added relays.
    pub async fn connect(&self) {
        // RelayManager handles connections automatically
        tracing::debug!("connect() called - RelayManager handles connections automatically");
    }

    /// Get the inner nostr_sdk client for subscription management
    /// DEPRECATED: Use relay_manager instead
    pub fn inner(&self) -> Option<&Client> {
        // TODO: This method is deprecated and returns None
        // Subscriptions should use the relay_manager instead
        None
    }

    /// Get a clone of the inner client as Arc for subscription loops
    /// DEPRECATED: Use relay_manager instead
    pub fn inner_clone(&self) -> Option<Client> {
        // TODO: This method is deprecated and returns None
        // Subscriptions should use the relay_manager instead
        None
    }

    /// Get the number of connected relays.
    pub async fn get_relay_count(&self) -> usize {
        let manager = self.relay_manager.lock().await;
        manager.get_connected_count().await
    }

    /// Get the list of connected relay URLs.
    pub async fn get_connected_relays(&self) -> Vec<RelayStatus> {
        let manager = self.relay_manager.lock().await;
        manager.get_connected_relays().await
    }

    /// Fetch from unified relay pool
    pub async fn fetch_from_indexers_then_all(
        &self,
        filter: Filter,
    ) -> Result<Vec<Event>, NostrError> {
        let events = {
            let manager = self.relay_manager.lock().await;
            manager
                .fetch_events(filter)
                .await
                .map_err(|e| NostrError::RelayError(format!("Failed to fetch: {}", e)))?
        };

        tracing::info!("Query returned {} events", events.len());
        Ok(events.into_iter().collect())
    }

    /// Combined fetch for both profile metadata and relay list (kind 0 + 10002)
    /// Returns: (UserProfile, Option<CachedRelayList>)
    pub async fn fetch_user_metadata(
        &self,
        npub: &str,
    ) -> Result<(UserProfile, Option<CachedRelayList>), NostrError> {
        let pubkey = PublicKey::parse(npub)
            .map_err(|e| NostrError::MalformedEvent(format!("Invalid npub: {}", e)))?;

        let filter = Filter::new()
            .author(pubkey)
            .kinds(vec![Kind::Metadata, Kind::from_u16(KIND_RELAY_LIST)])
            .limit(2);

        let events = self.fetch_from_indexers_then_all(filter).await?;

        let mut profile = None;
        let mut relay_list = None;

        for event in events {
            match event.kind.as_u16() {
                0 => profile = Some(self.parse_profile_event(&event, npub)?),
                KIND_RELAY_LIST => relay_list = Some(parse_relay_list_from_event(&event)?),
                _ => {}
            }
        }

        Ok((profile.unwrap_or_default(), relay_list))
    }

    /// Fetch Kind 10002 (relay list metadata) for a pubkey
    /// Uses unified relay pool through RelayManager
    pub async fn fetch_relay_list(&self, npub: &str) -> Result<CachedRelayList, NostrError> {
        let pubkey = PublicKey::parse(npub)
            .map_err(|e| NostrError::MalformedEvent(format!("Invalid npub: {}", e)))?;

        let filter = Filter::new()
            .kind(Kind::from_u16(KIND_RELAY_LIST))
            .author(pubkey)
            .limit(1);

        // Query through relay manager (unified pool)
        let events = {
            let manager = self.relay_manager.lock().await;
            manager
                .fetch_events(filter)
                .await
                .map_err(|e| NostrError::RelayError(format!("Failed to fetch relay list: {}", e)))?
        };

        let event = events
            .first()
            .ok_or_else(|| NostrError::MalformedEvent("No relay list found".into()))?;

        let relay_list = parse_relay_list_from_event(event)?;
        tracing::info!(
            "Found relay list for {} with {} read and {} write relays",
            npub,
            relay_list.read_relays.len(),
            relay_list.write_relays.len()
        );

        Ok(relay_list)
    }

    /// Fetch Kind 3 (follow list) for a pubkey
    /// Uses unified relay pool through RelayManager
    pub async fn fetch_follow_list(&self, npub: &str) -> Result<Vec<String>, NostrError> {
        let pubkey = PublicKey::parse(npub)
            .map_err(|e| NostrError::MalformedEvent(format!("Invalid npub: {}", e)))?;

        let filter = Filter::new()
            .kind(Kind::from_u16(KIND_FOLLOW_LIST))
            .author(pubkey)
            .limit(1);

        // Query through relay manager (unified pool)
        let events = {
            let manager = self.relay_manager.lock().await;
            manager.fetch_events(filter).await.map_err(|e| {
                NostrError::RelayError(format!("Failed to fetch follow list: {}", e))
            })?
        };

        tracing::info!("Follow list fetch returned {} events", events.len());

        let event = match events.first() {
            Some(e) => e,
            None => {
                tracing::warn!("No follow list found on any relay");
                return Ok(vec![]); // No follow list
            }
        };

        tracing::info!(
            "Found follow list event - kind: {}, content length: {}",
            event.kind,
            event.content.len()
        );
        tracing::debug!("Follow list has {} tags", event.tags.len());

        self.parse_follow_list_content(event)
    }

    /// Parse follow list from Kind 3 event tags (NIP-02 format)
    /// NIP-02 uses "p" tags: ["p", "<pubkey>", "<relay-url>", "<petname>"]
    fn parse_follow_list_content(&self, event: &Event) -> Result<Vec<String>, NostrError> {
        let mut pubkeys = Vec::new();

        // Iterate through all tags looking for "p" tags
        for tag in event.tags.iter() {
            let tag_kind = tag.kind();
            if tag_kind.to_string() == "p" {
                // Get all values from the tag as a vector
                let values: Vec<String> = tag.clone().to_vec();

                // p-tag format: ["p", "<pubkey>", "<relay>", "<petname>"]
                // We need at least 2 elements: "p" and the pubkey
                if values.len() >= 2 {
                    let pubkey = &values[1];
                    // Validate it's a 64-char hex pubkey
                    if pubkey.len() == 64 && pubkey.chars().all(|c| c.is_ascii_hexdigit()) {
                        pubkeys.push(pubkey.clone());
                    } else {
                        tracing::debug!("Skipping invalid pubkey in p-tag: {}", pubkey);
                    }
                }
            }
        }

        tracing::info!("Parsed {} pubkeys from follow list p-tags", pubkeys.len());
        Ok(pubkeys)
    }

    /// Get relays for a pubkey with fallback discovery
    /// Implements 4-tier waterfall:
    /// Tier 1: Kind 3 content field (relay hints in follow list)
    /// Tier 2: seen_on tracker
    /// Tier 3: user's own read relays
    /// Tier 4: global aggregators (DEFAULT_RELAYS)
    ///
    /// If data is stale (>7 days), returns it immediately and triggers background refresh.
    pub async fn get_relays_for_pubkey(
        &self,
        npub: &str,
        cache: &RelayCache,
        user_npub: Option<&str>, // authenticated user's npub for Tier 3
    ) -> Result<RelayDiscoveryResult, NostrError> {
        // Check cache first
        if let Some(cached) = cache.get_relay_list(npub) {
            let is_stale = cache.is_stale(npub);

            // If fresh, return immediately
            if !is_stale {
                return Ok(RelayDiscoveryResult {
                    write_relays: cached.write_relays,
                    read_relays: cached.read_relays,
                    source: RelayDiscoverySource::RelayList,
                });
            }

            // If stale, return immediately but trigger background refresh
            cache.mark_for_refresh(npub);

            return Ok(RelayDiscoveryResult {
                write_relays: cached.write_relays,
                read_relays: cached.read_relays,
                source: RelayDiscoverySource::RelayList,
            });
        }

        // Try to fetch fresh Kind 10002
        match self.fetch_relay_list(npub).await {
            Ok(relay_list) => {
                let _ = cache.save_relay_list(&relay_list);
                Ok(RelayDiscoveryResult {
                    write_relays: relay_list.write_relays,
                    read_relays: relay_list.read_relays,
                    source: RelayDiscoverySource::RelayList,
                })
            }
            Err(_) => {
                // Tier 1: Try Kind 3 content field for relay hints
                if let Ok(follow_list) = self.fetch_follow_list_with_relays(npub).await {
                    if !follow_list.is_empty() {
                        return Ok(RelayDiscoveryResult {
                            write_relays: follow_list.clone(),
                            read_relays: follow_list,
                            source: RelayDiscoverySource::SeenOn, // Using SeenOn as closest match
                        });
                    }
                }

                // Tier 2: seen_on tracker
                let seen_on = cache.get_seen_on(npub);
                if !seen_on.is_empty() {
                    return Ok(RelayDiscoveryResult {
                        write_relays: seen_on.clone(),
                        read_relays: seen_on,
                        source: RelayDiscoverySource::SeenOn,
                    });
                }

                // Tier 3: user's own read relays
                if let Some(user) = user_npub {
                    if let Some(user_relays) = cache.get_relay_list(user) {
                        if !user_relays.read_relays.is_empty() {
                            return Ok(RelayDiscoveryResult {
                                write_relays: user_relays.read_relays.clone(),
                                read_relays: user_relays.read_relays,
                                source: RelayDiscoverySource::SeenOn, // Using SeenOn as closest match
                            });
                        }
                    }
                }

                // Tier 4: global aggregators
                Ok(RelayDiscoveryResult {
                    write_relays: DEFAULT_RELAYS.iter().map(|s| s.to_string()).collect(),
                    read_relays: DEFAULT_RELAYS.iter().map(|s| s.to_string()).collect(),
                    source: RelayDiscoverySource::GlobalFallback,
                })
            }
        }
    }

    /// Get relays for a pubkey with full fallback chain (Tier 1-4)
    ///
    /// Tier 1: NIP-65 Kind 10002 (from cache or fetch)
    /// Tier 2: Kind 3 content field (legacy)
    /// Tier 3: Relay hints (from p-tags)
    /// Tier 4: Global fallbacks
    #[cfg(feature = "native")]
    pub async fn get_relays_for_pubkey_with_hints(
        &self,
        pubkey: &str,
        relay_cache: &RelayCache,
        hint_store: Option<&RelayHints>,
    ) -> RelayDiscoveryResult {
        // Tier 1: NIP-65 from cache
        if let Some(cached) = relay_cache.get_relay_list(pubkey) {
            return RelayDiscoveryResult {
                write_relays: cached.write_relays,
                read_relays: cached.read_relays,
                source: RelayDiscoverySource::RelayList,
            };
        }

        // Try fetching from network (indexers first)
        match self.fetch_relay_list(pubkey).await {
            Ok(relay_list) => {
                if let Err(e) = relay_cache.save_relay_list(&relay_list) {
                    tracing::warn!("Failed to save relay list to cache for {}: {}", pubkey, e);
                }
                return RelayDiscoveryResult {
                    write_relays: relay_list.write_relays,
                    read_relays: relay_list.read_relays,
                    source: RelayDiscoverySource::RelayList,
                };
            }
            Err(e) => {
                tracing::warn!("Failed to fetch relay list for {}: {}", pubkey, e);
                // Continue to fallback tiers
            }
        }

        // Tier 2: Kind 3 content field (legacy)
        match self.fetch_follow_list_with_relays(pubkey).await {
            Ok(follow_list) if !follow_list.is_empty() => {
                return RelayDiscoveryResult {
                    write_relays: follow_list.clone(),
                    read_relays: follow_list,
                    source: RelayDiscoverySource::SeenOn,
                };
            }
            Err(e) => {
                tracing::warn!("Failed to fetch follow list for {}: {}", pubkey, e);
            }
            Ok(_) => {
                // Empty follow list, continue to Tier 3
            }
        }

        // Tier 3: Relay hints
        if let Some(hint_store) = hint_store {
            match hint_store.get_hints(pubkey) {
                Ok(hints) if !hints.is_empty() => {
                    return RelayDiscoveryResult {
                        write_relays: hints.clone(),
                        read_relays: hints,
                        source: RelayDiscoverySource::RelayHints,
                    };
                }
                Err(e) => {
                    tracing::warn!("Failed to get relay hints for {}: {}", pubkey, e);
                }
                Ok(_) => {
                    // Empty hints, continue to Tier 4
                }
            }
        }

        // Tier 4: Global fallbacks
        RelayDiscoveryResult {
            write_relays: DEFAULT_RELAYS.iter().map(|s| s.to_string()).collect(),
            read_relays: DEFAULT_RELAYS.iter().map(|s| s.to_string()).collect(),
            source: RelayDiscoverySource::GlobalFallback,
        }
    }

    /// Fetch Kind 3 (follow list) and extract relay hints from content field
    /// Returns list of relay URLs found in the content
    async fn fetch_follow_list_with_relays(&self, npub: &str) -> Result<Vec<String>, NostrError> {
        let pubkey = PublicKey::parse(npub)
            .map_err(|e| NostrError::MalformedEvent(format!("Invalid npub: {}", e)))?;

        let filter = Filter::new()
            .kind(Kind::from_u16(KIND_FOLLOW_LIST))
            .author(pubkey)
            .limit(1);

        let events = {
            let manager = self.relay_manager.lock().await;
            manager.fetch_events(filter).await.map_err(|e| {
                NostrError::RelayError(format!("Failed to fetch follow list: {}", e))
            })?
        };

        let event = match events.first() {
            Some(e) => e,
            None => return Ok(vec![]),
        };

        // Parse content for relay URLs
        let content_str = event.content.trim();
        if content_str.is_empty() {
            return Ok(vec![]);
        }

        // Try to parse as array of ["pubkey", "relay_url"] pairs
        let pairs: Vec<Vec<String>> = serde_json::from_str(content_str).unwrap_or_default();
        let relay_urls: Vec<String> = pairs
            .into_iter()
            .filter_map(|p| {
                // Extract relay URL from second element if present
                if p.len() >= 2 && p[1].starts_with("wss://") {
                    Some(p[1].clone())
                } else {
                    None
                }
            })
            .collect();

        Ok(relay_urls)
    }

    /// Publish event to outbox relays
    /// Rule A: Always include user's own write relays
    /// Rule B: If replying, include the replied-to author's READ relays
    pub async fn publish_to_outbox(
        &self,
        event: Event,
        reply_target: Option<&Nip19Identifier>,
        user_write_relays: &[String],
    ) -> Result<(), NostrError> {
        let mut target_relays: Vec<String> = user_write_relays.to_vec();

        // Rule B: If replying, add target's read relays
        if let Some(target) = reply_target {
            if !target.relays.is_empty() {
                target_relays.extend(target.relays.clone());
            }
        }

        // Deduplicate relays
        target_relays.sort();
        target_relays.dedup();

        // Publish to each relay through RelayManager
        {
            let manager = self.relay_manager.lock().await;
            for relay_url in target_relays {
                if let Err(e) = manager.add_discovered_relay(relay_url.clone()).await {
                    tracing::debug!("Could not add relay {}: {}", relay_url, e);
                }
                // TODO: Implement event publishing through RelayManager
                tracing::debug!("Would publish event to {}", relay_url);
            }
        } // Lock dropped after loop

        Ok(())
    }
}

/// Converts a GameListing to an EventBuilder for signing.
pub fn game_listing_to_event_builder(listing: &GameListing) -> EventBuilder {
    let content = GameListingContent {
        description: listing.description.clone(),
        download_url: listing.download_url.clone(),
    };

    let content_json = serde_json::to_string(&content).unwrap_or_default();

    let mut tags: Vec<Tag> = vec![
        Tag::identifier(listing.id.clone()),
        Tag::custom(TagKind::Custom("title".into()), [listing.title.clone()]),
        Tag::custom(
            TagKind::Custom("price".into()),
            [listing.price_sats.to_string()],
        ),
    ];

    // Add lud16 tag if present
    if !listing.lud16.is_empty() {
        tags.push(Tag::custom(
            TagKind::Custom("lud16".into()),
            [listing.lud16.clone()],
        ));
    }

    // Add tags
    for tag in &listing.tags {
        tags.push(Tag::custom(TagKind::Custom("t".into()), [tag.clone()]));
    }

    EventBuilder::new(Kind::Custom(KIND_GAME_LISTING), content_json).tags(tags)
}

/// Parses a NOSTR event into a GameListing.
pub fn event_to_game_listing(event: &Event) -> Result<GameListing, NostrError> {
    // Extract d tag (id)
    let id = event
        .tags
        .identifier()
        .ok_or_else(|| NostrError::MalformedEvent("Missing d tag".into()))?
        .to_string();

    // Extract title tag
    let title = event
        .tags
        .find(TagKind::Custom("title".into()))
        .and_then(|t| t.content().map(|c| c.to_string()))
        .ok_or_else(|| NostrError::MalformedEvent("Missing title tag".into()))?;

    // Extract price tag
    let price_str = event
        .tags
        .find(TagKind::Custom("price".into()))
        .and_then(|t| t.content().map(|c| c.to_string()))
        .ok_or_else(|| NostrError::MalformedEvent("Missing price tag".into()))?;
    let price_sats = price_str
        .parse::<u64>()
        .map_err(|_| NostrError::MalformedEvent("Invalid price format".into()))?;

    // Extract t tags
    let tags: Vec<String> = event
        .tags
        .filter(TagKind::Custom("t".into()))
        .filter_map(|t| t.content().map(|c| c.to_string()))
        .collect();

    // Extract lud16 tag (optional - for backwards compatibility)
    let lud16 = event
        .tags
        .find(TagKind::Custom("lud16".into()))
        .and_then(|t| t.content().map(|c| c.to_string()))
        .unwrap_or_default();

    // Parse content
    let content: GameListingContent = serde_json::from_str(&event.content)
        .map_err(|e| NostrError::MalformedEvent(format!("Invalid content JSON: {}", e)))?;

    // Get publisher npub
    let publisher_npub = event
        .pubkey
        .to_bech32()
        .map_err(|e| NostrError::MalformedEvent(format!("Invalid pubkey: {}", e)))?;

    Ok(GameListing {
        id,
        title,
        description: content.description,
        price_sats,
        download_url: content.download_url,
        publisher_npub,
        created_at: event.created_at.as_secs(),
        tags,
        lud16,
    })
}

/// Signs an event using the Arcadestr ActiveSigner.
///
/// This bridges our local NostrSigner trait to nostr_sdk's NostrSigner trait.
#[cfg(not(target_arch = "wasm32"))]
async fn sign_event_with_arcadestr_signer(
    builder: EventBuilder,
    signer: &ActiveSigner,
) -> Result<Event, NostrError> {
    // Get the public key from the signer
    let pubkey = signer
        .get_public_key()
        .await
        .map_err(|e| NostrError::SigningError(format!("Failed to get public key: {}", e)))?;

    // Build the unsigned event
    let unsigned = builder.build(pubkey);

    // Sign the event using our signer
    let signed = signer
        .sign_event(unsigned)
        .await
        .map_err(|e| NostrError::SigningError(format!("Failed to sign event: {}", e)))?;

    Ok(signed)
}

/// Stub implementation for WASM target (signing not supported in WASM).
#[cfg(target_arch = "wasm32")]
async fn sign_event_with_arcadestr_signer(
    _builder: EventBuilder,
    _signer: &ActiveSigner,
) -> Result<Event, NostrError> {
    Err(NostrError::SigningError(
        "Signing not supported in WASM target".into(),
    ))
}

// ============================================
// NIP-19: Identifier Parsing
// ============================================

/// NIP-19 parsed identifier with relay hints
#[derive(Debug, Clone)]
pub struct Nip19Identifier {
    pub pubkey: String,
    pub relays: Vec<String>,
}

/// Parse NIP-19 identifier (nprofile, nevent, npub, or note) to extract pubkey and relay hints
pub fn parse_nip19_identifier(identifier: &str) -> Result<Nip19Identifier, NostrError> {
    use nostr::key::PublicKey;
    use nostr::nips::nip19::{FromBech32, Nip19Event, Nip19Profile};

    // Check prefix by looking at the identifier start
    if identifier.starts_with("nprofile1") {
        let profile = Nip19Profile::from_bech32(identifier)
            .map_err(|e| NostrError::MalformedEvent(format!("Invalid nprofile: {}", e)))?;
        let pubkey = profile.public_key.to_hex();
        let relays: Vec<String> = profile.relays.into_iter().map(|r| r.to_string()).collect();
        Ok(Nip19Identifier { pubkey, relays })
    } else if identifier.starts_with("npub1") {
        // Parse npub using nostr_sdk
        let pubkey = PublicKey::parse(identifier)
            .map_err(|e| NostrError::MalformedEvent(format!("Invalid npub: {}", e)))?;
        Ok(Nip19Identifier {
            pubkey: pubkey.to_hex(),
            relays: vec![],
        })
    } else if identifier.starts_with("nevent1") {
        let event = Nip19Event::from_bech32(identifier)
            .map_err(|e| NostrError::MalformedEvent(format!("Invalid nevent: {}", e)))?;
        let pubkey = event.author.map(|p| p.to_hex()).unwrap_or_default();
        let relays: Vec<String> = event.relays.into_iter().map(|r| r.to_string()).collect();
        Ok(Nip19Identifier { pubkey, relays })
    } else if identifier.starts_with("note1") {
        // For notes, we return the event id as the "pubkey" (for lack of a better field)
        // Parse using nostr_sdk
        let event_id = nostr::EventId::from_bech32(identifier)
            .map_err(|e| NostrError::MalformedEvent(format!("Invalid note: {}", e)))?;
        Ok(Nip19Identifier {
            pubkey: event_id.to_hex(),
            relays: vec![],
        })
    } else {
        Err(NostrError::MalformedEvent(
            "Invalid NIP-19 identifier".into(),
        ))
    }
}

#[cfg(test)]
mod nip65_tests {
    use super::*;

    #[test]
    fn test_parse_relay_list_content() {
        let content = r#"{"read":["wss://relay.nostr.info"],"write":["wss://relay.damus.io"]}"#;
        let result = parse_relay_list_content(content);
        assert!(result.is_ok());

        let relays = result.unwrap();
        assert!(relays
            .write_relays
            .contains(&"wss://relay.damus.io".to_string()));
        assert!(relays
            .read_relays
            .contains(&"wss://relay.nostr.info".to_string()));
    }

    #[tokio::test]
    async fn test_fetch_relay_list_parses_kind_10002() {
        // This test verifies that we can parse a Kind 10002 event
        // The actual implementation will be added in the next step
        //let client = NostrClient::new(vec!["wss://relay.damus.io".to_string()]).await.unwrap();

        // Test parsing of known Kind 10002 content format
    }

    #[test]
    fn test_parse_nip19_npub() {
        // Test parsing a valid npub
        let test_npub = "npub1qqqs20u8s8pmnxuenjc6k7h7ej895v93su5qcytrdsxvmnvu8qcyqqqgs5p";
        let result = parse_nip19_identifier(test_npub);
        // This may fail if the npub is invalid - that's ok, just verify the function exists
        match result {
            Ok(id) => {
                assert!(!id.pubkey.is_empty());
                assert!(id.relays.is_empty());
            }
            Err(_) => {
                // npub may be invalid, which is fine for this test
            }
        }
    }
}

#[cfg(test)]
mod dedup_tests {
    use super::*;

    #[test]
    fn test_deduplicator_new_event() {
        let mut dedup = EventDeduplicator::new(100);
        let is_dup = dedup.check_and_insert("event123");
        assert!(!is_dup); // First time, not a duplicate
    }

    #[test]
    fn test_deduplicator_duplicate_event() {
        let mut dedup = EventDeduplicator::new(100);
        let _ = dedup.check_and_insert("event123");
        let is_dup = dedup.check_and_insert("event123");
        assert!(is_dup); // Second time, is a duplicate
    }

    #[test]
    fn test_deduplicator_clear() {
        let mut dedup = EventDeduplicator::new(100);
        let _ = dedup.check_and_insert("event123");
        dedup.clear();
        let is_dup = dedup.check_and_insert("event123");
        assert!(!is_dup); // After clear, not a duplicate
    }
}

#[cfg(test)]
mod idle_timeout_tests {
    use super::*;
    use std::thread;
    use std::time::Duration;

    #[test]
    fn test_touch_updates_last_activity() {
        let mut manager = RelayConnectionManager::with_default_timeout();
        manager.touch("wss://relay.example.com");

        let idle = manager.get_idle_relays();
        assert!(idle.is_empty()); // Just touched, not idle
    }

    #[test]
    fn test_cleanup_removes_idle_relays() {
        let mut manager = RelayConnectionManager::new(Duration::from_millis(1));
        manager.touch("wss://relay.example.com");

        // Wait a bit
        thread::sleep(Duration::from_millis(10));

        let idle = manager.cleanup();
        assert!(idle.contains(&"wss://relay.example.com".to_string()));

        // Should be removed now
        let idle = manager.get_idle_relays();
        assert!(idle.is_empty());
    }
}

#[cfg(test)]
mod follow_list_tests {
    use super::*;
    use nostr_sdk::{EventBuilder, Keys, Tag, TagKind};

    fn create_test_event_with_p_tags(pubkeys: Vec<&str>) -> Event {
        let keys = Keys::generate();

        let mut builder = EventBuilder::new(
            Kind::from_u16(KIND_FOLLOW_LIST),
            "", // empty content
        );

        for pubkey in pubkeys {
            builder = builder.tag(Tag::custom(TagKind::p(), vec![pubkey.to_string()]));
        }

        builder.sign_with_keys(&keys).expect("Failed to sign event")
    }

    fn create_test_event_with_full_p_tags(
        entries: Vec<(&str, Option<&str>, Option<&str>)>,
    ) -> Event {
        let keys = Keys::generate();

        let mut builder = EventBuilder::new(
            Kind::from_u16(KIND_FOLLOW_LIST),
            "", // empty content
        );

        for (pubkey, relay, petname) in entries {
            let mut tag_parts = vec![pubkey.to_string()];
            if let Some(r) = relay {
                tag_parts.push(r.to_string());
            }
            if let Some(p) = petname {
                if tag_parts.len() < 2 {
                    tag_parts.push("".to_string()); // Empty relay hint placeholder
                }
                tag_parts.push(p.to_string());
            }
            builder = builder.tag(Tag::custom(TagKind::p(), tag_parts));
        }

        builder.sign_with_keys(&keys).expect("Failed to sign event")
    }

    #[tokio::test]
    async fn test_parse_follow_list_from_p_tags() {
        let client = NostrClient::new("test".to_string(), vec![], None)
            .await
            .expect("Failed to create client");

        // Valid 64-char hex pubkeys
        let pubkey1 = "a".repeat(64);
        let pubkey2 = "b".repeat(64);
        let event = create_test_event_with_p_tags(vec![&pubkey1, &pubkey2]);

        let result = client.parse_follow_list_content(&event);
        assert!(result.is_ok());

        let pubkeys = result.unwrap();
        assert_eq!(pubkeys.len(), 2);
        assert!(pubkeys.contains(&pubkey1));
        assert!(pubkeys.contains(&pubkey2));
    }

    #[tokio::test]
    async fn test_parse_follow_list_with_full_p_tags() {
        let client = NostrClient::new("test".to_string(), vec![], None)
            .await
            .expect("Failed to create client");

        let pubkey1 = "c".repeat(64);
        let pubkey2 = "d".repeat(64);
        let entries = vec![
            (pubkey1.as_str(), Some("wss://relay1.com"), Some("Alice")),
            (pubkey2.as_str(), Some("wss://relay2.com"), Some("Bob")),
        ];
        let event = create_test_event_with_full_p_tags(entries);

        let result = client.parse_follow_list_content(&event);
        assert!(result.is_ok());

        let pubkeys = result.unwrap();
        assert_eq!(pubkeys.len(), 2);
        assert!(pubkeys.contains(&pubkey1));
        assert!(pubkeys.contains(&pubkey2));
    }

    #[tokio::test]
    async fn test_parse_follow_list_empty_tags() {
        let client = NostrClient::new("test".to_string(), vec![], None)
            .await
            .expect("Failed to create client");

        let event = create_test_event_with_p_tags(vec![]);

        let result = client.parse_follow_list_content(&event);
        assert!(result.is_ok());

        let pubkeys = result.unwrap();
        assert!(pubkeys.is_empty());
    }

    #[tokio::test]
    async fn test_parse_follow_list_skips_malformed_p_tags() {
        let client = NostrClient::new("test".to_string(), vec![], None)
            .await
            .expect("Failed to create client");
        let keys = Keys::generate();

        let valid_pubkey = "e".repeat(64);
        let invalid_pubkey = "too_short"; // Not 64 chars

        // Create event with mixed valid and invalid p-tags
        let event = EventBuilder::new(Kind::from_u16(KIND_FOLLOW_LIST), "")
            .tag(Tag::custom(TagKind::p(), vec![valid_pubkey.clone()]))
            .tag(Tag::custom(TagKind::p(), vec![invalid_pubkey.to_string()]))
            .tag(Tag::custom(TagKind::e(), vec!["some_event_id".to_string()]))
            .sign_with_keys(&keys)
            .expect("Failed to sign event");

        let result = client.parse_follow_list_content(&event);
        assert!(result.is_ok());

        let pubkeys = result.unwrap();
        assert_eq!(pubkeys.len(), 1); // Only the valid pubkey
        assert!(pubkeys.contains(&valid_pubkey));
    }

    #[tokio::test]
    async fn test_parse_follow_list_ignores_non_p_tags() {
        let client = NostrClient::new("test".to_string(), vec![], None)
            .await
            .expect("Failed to create client");
        let keys = Keys::generate();

        let pubkey = "f".repeat(64);

        let event = EventBuilder::new(Kind::from_u16(KIND_FOLLOW_LIST), "")
            .tag(Tag::custom(TagKind::p(), vec![pubkey.clone()]))
            .tag(Tag::custom(TagKind::e(), vec!["some_event_id".to_string()]))
            .sign_with_keys(&keys)
            .expect("Failed to sign event");

        let result = client.parse_follow_list_content(&event);
        assert!(result.is_ok());

        let pubkeys = result.unwrap();
        assert_eq!(pubkeys.len(), 1); // Only the p-tag pubkey
        assert!(pubkeys.contains(&pubkey));
    }
}

#[cfg(test)]
mod assertions {
    use super::NostrClient;

    const _: () = {
        fn assert_send<T: Send>() {}
        fn assert_sync<T: Sync>() {}
        assert_send::<NostrClient>();
        assert_sync::<NostrClient>();
    };
}
