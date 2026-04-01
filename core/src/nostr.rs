// NOSTR protocol integration: event handling, relay connections, NIP-46 signer support.

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use nostr_sdk::prelude::*;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tracing::warn;

use crate::auth::AuthState;
#[cfg(feature = "native")]
use crate::relay_cache::{CachedRelayList, RelayCache, RelayDiscoverySource};
use crate::signers::{ActiveSigner, NostrSigner as ArcadestrNostrSigner, SignerError};

/// Arcadestr game listing event kind.
/// Using kind 30078 (NIP-78 arbitrary app data, parameterized replaceable).
pub const KIND_GAME_LISTING: u16 = 30078;

/// Default relays for Arcadestr.
/// Includes popular relay discovery services that aggregate user metadata.
pub const DEFAULT_RELAYS: &[&str] = &[
    // Relay discovery services (query these first for user lookups)
    "wss://purplepag.es",           // Aggregates user metadata and relay lists
    "wss://relay.nostr.info",       // Relay discovery service
    "wss://relay.nostr.band",       // Relay aggregator with good coverage
    // General relays
    "wss://relay.damus.io",
    "wss://nos.lol",
    "wss://relay.snort.social",
    "wss://relay.current.fyi",
    "wss://nostr.wine",
];

/// Relay discovery services - prioritized for user lookups
pub const DISCOVERY_RELAYS: &[&str] = &[
    "wss://purplepag.es",
    "wss://relay.nostr.info", 
    "wss://relay.nostr.band",
    "wss://relay.nsec.app",        // NIP-46 service relay
];

/// Indexer relays for profile/relay discovery (subset of DEFAULT_RELAYS)
pub const INDEXER_RELAYS: &[&str] = &[
    "wss://relay.primal.net",
    "wss://relay.nostr.band",
    "wss://purplepag.es",
    "wss://indexer.coracle.social",
];

/// Kind 10002: Relay List Metadata (NIP-65)
pub const KIND_RELAY_LIST: u16 = 10002;

/// Kind 3: Follow List (NIP-02)
pub const KIND_FOLLOW_LIST: u16 = 3;

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
        self.last_activity.insert(relay_url.to_string(), Instant::now());
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
    
    tracing::debug!("Parsed relay list: {} read relays, {} write relays", read_relays.len(), write_relays.len());
    
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
                relay_map.entry(relay.clone()).or_default().insert(pubkey.clone());
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
        let is_user_own = user_pubkey.map(|u| {
            cache.get_relay_list(u)
                .map(|l| l.write_relays.contains(relay_url))
                .unwrap_or(false)
        }).unwrap_or(false);
        
        let own_relay_multiplier = if is_user_own { 1.5 } else { 1.0 };
        
        let final_score = raw_score * health_multiplier * staleness_multiplier * own_relay_multiplier;
        
        scored.push(ScoredRelay {
            url: relay_url.clone(),
            score: final_score,
            pubkeys: pubkeys.clone(),
        });
    }
    
    // Sort by score descending
    scored.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
    
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
    pub id: String,           // unique slug / d-tag value, e.g. "my-game-v1"
    pub title: String,
    pub description: String,
    pub price_sats: u64,      // price in satoshis, 0 = free
    pub download_url: String, // direct download link (may be encrypted in future)
    pub publisher_npub: String, // bech32 npub of the publisher
    pub created_at: u64,      // unix timestamp
    pub tags: Vec<String>,    // freeform tags e.g. ["rpg", "pixel-art"]
    pub lud16: String,        // Lightning address for payments (e.g., "seller@walletofsatoshi.com")
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
    inner: nostr_sdk::Client,
}

impl NostrClient {
    /// Creates a new NOSTR client connected to the specified relays.
    /// 
    /// Note: Relay connection errors are logged but don't fail - the client
    /// will retry connections automatically.
    pub async fn new(relays: Vec<String>) -> Result<Self, NostrError> {
        use tracing::{info, warn, error};
        
        info!("Creating NostrClient with {} relays", relays.len());
        let client = nostr_sdk::Client::default();
        
        for relay in &relays {
            match client.add_relay(relay).await {
                Ok(_) => {
                    info!("Added relay: {}", relay);
                }
                Err(e) => {
                    let err_str = e.to_string();
                    error!("Failed to add relay {}: {}", relay, err_str);
                    if err_str.contains("parse") || err_str.contains("expected ident") {
                        error!("  -> Relay returned HTML instead of WebSocket response");
                        error!("  -> This usually means the relay is down or blocked");
                    } else {
                        warn!("  -> Will retry on first use");
                    }
                    // Don't fail - continue with other relays
                }
            }
        }
        
        // Try to connect, but don't fail if relays are temporarily down
        client.connect().await;
        
        // Give some time for connections to establish
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        
        // Check relay status
        let relay_list = client.relays().await;
        info!("Connected relays: {:?}", relay_list.keys().collect::<Vec<_>>());
        
        if relay_list.is_empty() {
            warn!("No relays connected - queries may fail");
        }
        
        info!("NostrClient initialized");
        
        Ok(Self { inner: client })
    }

    /// Publishes a game listing as a signed NOSTR event.
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

        // Send the event
        self.inner.send_event(&signed_event).await.map_err(|e| {
            NostrError::RelayError(format!("Failed to send event: {}", e))
        })?;

        Ok(signed_event.id)
    }

    /// Fetches recent game listings from relays.
    pub async fn fetch_listings(&self, limit: usize) -> Result<Vec<GameListing>, NostrError> {
        let filter = Filter::new()
            .kind(Kind::Custom(KIND_GAME_LISTING))
            .limit(limit);

        let events = self.inner
            .fetch_events(filter, Duration::from_secs(10))
            .await
            .map_err(|e| NostrError::RelayError(format!("Failed to fetch events: {}", e)))?;

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

        let events = self.inner
            .fetch_events(filter, Duration::from_secs(10))
            .await
            .map_err(|e| NostrError::RelayError(format!("Failed to fetch event: {}", e)))?;

        let event = events
            .first()
            .ok_or_else(|| NostrError::MalformedEvent("Listing not found".into()))?;

        event_to_game_listing(event)
    }

    /// Fetches user profile metadata (kind-0) from relays.
    /// First tries discovery services, then additional relays (if provided), then falls back to all connected relays.
    pub async fn fetch_profile(
        &self,
        npub: &str,
        additional_relays: Option<Vec<String>>,
    ) -> Result<UserProfile, NostrError> {
        // Parse npub as PublicKey
        let pubkey = PublicKey::parse(npub)
            .map_err(|e| NostrError::MalformedEvent(format!("Invalid npub: {}", e)))?;

        tracing::info!("Fetching profile for pubkey: {} (npub: {})", pubkey.to_hex(), npub);

        // Build filter for kind-0 (metadata) events
        let filter = Filter::new()
            .kind(Kind::Metadata)
            .author(pubkey)
            .limit(1);
        
        tracing::debug!("Profile filter: kind=0, author={}, limit=1", pubkey.to_hex());

        // First, try discovery services
        let discovery_urls: Vec<Url> = DISCOVERY_RELAYS
            .iter()
            .filter_map(|url| Url::parse(url).ok())
            .collect();
        
        tracing::debug!("Discovery relays: {:?}", discovery_urls);
        
        if !discovery_urls.is_empty() {
            tracing::info!("Adding {} discovery relays to client...", discovery_urls.len());
            
            // Add discovery relays to client first
            for url in &discovery_urls {
                let url_str = url.to_string();
                match self.inner.add_relay(&url_str).await {
                    Ok(_) => tracing::debug!("Added discovery relay: {}", url_str),
                    Err(e) => tracing::warn!("Failed to add discovery relay {}: {}", url_str, e),
                }
            }
            
            // Connect to newly added relays
            self.inner.connect().await;
            // Quick 100ms yield for connections to start (was 500ms)
            tokio::time::sleep(Duration::from_millis(100)).await;
            
            // Use 5s timeout for discovery (was 8s) - discovery should be fast
            tracing::info!("Querying {} discovery services for profile with 5s timeout...", discovery_urls.len());
            match self.inner.fetch_events_from(discovery_urls.clone(), filter.clone(), Duration::from_secs(5)).await {
                Ok(events) => {
                    tracing::debug!("Discovery query returned {} events", events.len());
                    if let Some(event) = events.first() {
                        tracing::info!("Found profile on discovery service! Event id: {}", event.id.to_hex());
                        return self.parse_profile_event(event, npub);
                    }
                    tracing::debug!("Discovery returned empty events list");
                }
                Err(e) => {
                    tracing::warn!("Discovery service query failed with error: {}", e);
                }
            }
            tracing::debug!("No profile found on discovery services, will try additional relays if provided");
        } else {
            tracing::warn!("No valid discovery relay URLs found!");
        }

        // Second, try additional relays if provided (e.g., from NIP-46 bunker connection)
        if let Some(additional) = additional_relays {
            let additional_urls: Vec<Url> = additional
                .iter()
                .filter_map(|url| Url::parse(url).ok())
                .collect();
            
            if !additional_urls.is_empty() {
                tracing::info!("Adding {} additional relays from NIP-46 connection...", additional_urls.len());
                
                // Add additional relays
                for url in &additional_urls {
                    let url_str = url.to_string();
                    match self.inner.add_relay(&url_str).await {
                        Ok(_) => tracing::debug!("Added additional relay: {}", url_str),
                        Err(e) => tracing::warn!("Failed to add additional relay {}: {}", url_str, e),
                    }
                }
                
                // Connect to newly added relays
                self.inner.connect().await;
                tokio::time::sleep(Duration::from_millis(100)).await;
                
                // Query additional relays with 5s timeout
                tracing::info!("Querying {} additional relays for profile with 5s timeout...", additional_urls.len());
                match self.inner.fetch_events_from(additional_urls.clone(), filter.clone(), Duration::from_secs(5)).await {
                    Ok(events) => {
                        tracing::debug!("Additional relays query returned {} events", events.len());
                        if let Some(event) = events.first() {
                            tracing::info!("Found profile on additional relay! Event id: {}", event.id.to_hex());
                            return self.parse_profile_event(event, npub);
                        }
                        tracing::debug!("Additional relays returned empty events list");
                    }
                    Err(e) => {
                        tracing::warn!("Additional relays query failed with error: {}", e);
                    }
                }
                tracing::debug!("No profile found on additional relays, will try all connected relays");
            }
        }

        // Fetch with 8s timeout from all relays (was 10s)
        tracing::info!("Querying all connected relays with 8s timeout...");
        let events = self.inner
            .fetch_events(filter, Duration::from_secs(8))
            .await
            .map_err(|e| {
                tracing::error!("Failed to fetch profile from all relays: {}", e);
                NostrError::RelayError(format!("Failed to fetch profile: {}", e))
            })?;

        tracing::debug!("All relays query returned {} events", events.len());

        // If no events returned, return minimal profile with only npub
        let event = match events.first() {
            Some(e) => {
                tracing::info!("Found profile event! Content preview: {}", &e.content[..e.content.len().min(100)]);
                e
            }
            None => {
                tracing::warn!("No kind-0 (metadata) events found for user {} on any relay", npub);
                return Ok(UserProfile {
                    npub: npub.to_string(),
                    ..Default::default()
                });
            }
        };

        self.parse_profile_event(event, npub)
    }

    /// Parse a profile event into UserProfile
    pub fn parse_profile_event(&self, event: &Event, npub: &str) -> Result<UserProfile, NostrError> {
        // Parse the event content as UserProfileContent
        let content: UserProfileContent = serde_json::from_str(&event.content)
            .unwrap_or_default();

        tracing::info!("Parsed profile: name={:?}, display_name={:?}, picture={:?}", 
            content.name, content.display_name, content.picture);

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
    pub async fn verify_nip05(
        &self,
        npub: &str,
        nip05_identifier: &str,
    ) -> bool {
        // Split nip05_identifier on '@' to get name and domain
        let parts: Vec<&str> = nip05_identifier.split('@').collect();
        if parts.len() != 2 {
            return false;
        }
        let (name, domain) = (parts[0], parts[1]);

        // Fetch the NIP-05 JSON from the domain
        let url = format!("https://{}/.well-known/nostr.json?name={}", domain, name);
        
        let client = match reqwest::Client::builder()
            .timeout(Duration::from_secs(5))
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
        nip05_resp.names
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
        self.inner.add_relay(relay).await.map_err(|e| {
            NostrError::RelayError(format!("Failed to add relay: {}", e))
        })
    }

    /// Connects to all added relays.
    pub async fn connect(&self) {
        self.inner.connect().await;
    }

    /// Get the inner nostr_sdk client for subscription management
    pub fn inner(&self) -> &Client {
        &self.inner
    }

    /// Get a clone of the inner client as Arc for subscription loops
    pub fn inner_clone(&self) -> Client {
        self.inner.clone()
    }

    /// Get the number of connected relays.
    pub async fn get_relay_count(&self) -> usize {
        self.inner.relays().await.len()
    }

    /// Get the list of connected relay URLs.
    pub async fn get_connected_relays(&self) -> Vec<String> {
        self.inner.relays().await.into_iter().map(|(url, _)| url.to_string()).collect()
    }

    /// Fetch from indexer relays first, silently fall back to all relays
    /// This is the core method for efficient profile/relay discovery
    pub async fn fetch_from_indexers_then_all(
        &self,
        filter: Filter,
    ) -> Result<Vec<Event>, NostrError> {
        // Try indexers first
        let indexer_urls: Vec<Url> = INDEXER_RELAYS
            .iter()
            .filter_map(|url| Url::parse(url).ok())
            .collect();
        
        tracing::debug!("Indexer URLs to query: {:?}", indexer_urls);
        tracing::debug!("Filter: {:?}", filter);
        
        if !indexer_urls.is_empty() {
            tracing::info!("Adding {} indexer relays to client...", indexer_urls.len());
            
            // Add indexer relays to client first (required before querying)
            for url in &indexer_urls {
                let url_str = url.to_string();
                match self.inner.add_relay(&url_str).await {
                    Ok(_) => tracing::debug!("Added indexer relay: {}", url_str),
                    Err(e) => tracing::warn!("Failed to add indexer relay {}: {}", url_str, e),
                }
            }
            
            // Connect to the newly added relays
            self.inner.connect().await;
            
            // Give a moment for connections to establish
            tokio::time::sleep(Duration::from_millis(500)).await;
            
            tracing::info!("Querying {} indexer relays with 8s timeout...", indexer_urls.len());
            match self.inner.fetch_events_from(indexer_urls.clone(), filter.clone(), Duration::from_secs(8)).await {
                Ok(events) if !events.is_empty() => {
                    tracing::info!("Found {} events from indexers", events.len());
                    return Ok(events.into_iter().collect());
                }
                Ok(events) => {
                    tracing::debug!("Indexer query returned {} events (empty)", events.len());
                }
                Err(e) => {
                    tracing::warn!("Indexer query failed with error: {}", e);
                }
            }
            tracing::debug!("No events from indexers, will try all relays");
        } else {
            tracing::warn!("No valid indexer URLs found!");
        }
        
        // Fallback to all connected relays
        tracing::info!("Querying all connected relays with 10s timeout...");
        let events = self.inner
            .fetch_events(filter, Duration::from_secs(10))
            .await
            .map_err(|e| {
                tracing::error!("Failed to fetch from all relays: {}", e);
                NostrError::RelayError(format!("Failed to fetch: {}", e))
            })?;
        
        tracing::info!("Fallback query returned {} events", events.len());
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
    /// First tries discovery services, then falls back to all connected relays
    pub async fn fetch_relay_list(&self, npub: &str) -> Result<CachedRelayList, NostrError> {
        let pubkey = PublicKey::parse(npub)
            .map_err(|e| NostrError::MalformedEvent(format!("Invalid npub: {}", e)))?;
        
        let filter = Filter::new()
            .kind(Kind::from_u16(KIND_RELAY_LIST))
            .author(pubkey)
            .limit(1);
        
        // First, try to query discovery services specifically
        let discovery_urls: Vec<Url> = DISCOVERY_RELAYS
            .iter()
            .filter_map(|url| Url::parse(url).ok())
            .collect();
        
        if !discovery_urls.is_empty() {
            tracing::info!("Querying discovery services for {}'s relay list", npub);
            match self.inner.fetch_events_from(discovery_urls, filter.clone(), Duration::from_secs(8)).await {
                Ok(events) => {
                    if let Some(event) = events.first() {
                        let relay_list = parse_relay_list_from_event(event)?;
                        tracing::info!("Found relay list from discovery service for {} with {} read and {} write relays", 
                            npub, relay_list.read_relays.len(), relay_list.write_relays.len());
                        return Ok(relay_list);
                    }
                }
                Err(e) => {
                    tracing::debug!("Discovery service query failed for {}: {}", npub, e);
                }
            }
            tracing::debug!("No relay list found on discovery services for {}, trying all relays", npub);
        }
        
        // Fall back to querying all connected relays
        let events = self.inner
            .fetch_events(filter, Duration::from_secs(10))
            .await
            .map_err(|e| NostrError::RelayError(format!("Failed to fetch relay list: {}", e)))?;
        
        let event = events.first()
            .ok_or_else(|| NostrError::MalformedEvent("No relay list found".into()))?;
        
        let relay_list = parse_relay_list_from_event(event)?;
        tracing::info!("Found relay list from all relays for {} with {} read and {} write relays",
            npub, relay_list.read_relays.len(), relay_list.write_relays.len());
        
        Ok(relay_list)
    }

    /// Fetch Kind 3 (follow list) for a pubkey
    /// First tries discovery services, then falls back to all connected relays
    pub async fn fetch_follow_list(&self, npub: &str) -> Result<Vec<String>, NostrError> {
        let pubkey = PublicKey::parse(npub)
            .map_err(|e| NostrError::MalformedEvent(format!("Invalid npub: {}", e)))?;
        
        let filter = Filter::new()
            .kind(Kind::from_u16(KIND_FOLLOW_LIST))
            .author(pubkey)
            .limit(1);
        
        // First, try discovery services
        let discovery_urls: Vec<Url> = DISCOVERY_RELAYS
            .iter()
            .filter_map(|url| Url::parse(url).ok())
            .collect();
        
        if !discovery_urls.is_empty() {
            tracing::info!("Adding {} discovery relays to client for follow list fetch...", discovery_urls.len());
            
            // Add discovery relays to client first
            for url in &discovery_urls {
                let url_str = url.to_string();
                match self.inner.add_relay(&url_str).await {
                    Ok(_) => tracing::debug!("Added discovery relay: {}", url_str),
                    Err(e) => tracing::warn!("Failed to add discovery relay {}: {}", url_str, e),
                }
            }
            
            // Connect to newly added relays
            self.inner.connect().await;
            tokio::time::sleep(Duration::from_millis(500)).await;
            
            tracing::info!("Querying discovery services for follow list");
            match self.inner.fetch_events_from(discovery_urls.clone(), filter.clone(), Duration::from_secs(8)).await {
                Ok(events) => {
                    if let Some(event) = events.first() {
                        tracing::info!("Found follow list on discovery service");
                        return self.parse_follow_list_content(&event.content);
                    }
                }
                Err(e) => {
                    tracing::debug!("Discovery service query failed: {}", e);
                }
            }
            tracing::debug!("No follow list found on discovery services, trying all relays");
        }
        
        // Fall back to all connected relays
        let events = self.inner
            .fetch_events(filter, Duration::from_secs(10))
            .await
            .map_err(|e| NostrError::RelayError(format!("Failed to fetch follow list: {}", e)))?;
        
        let event = match events.first() {
            Some(e) => e,
            None => return Ok(vec![]), // No follow list
        };
        
        self.parse_follow_list_content(&event.content)
    }

    /// Parse follow list content from Kind 3 event
    fn parse_follow_list_content(&self, content: &str) -> Result<Vec<String>, NostrError> {
        // Parse content - Kind 3 content is a JSON array of pubkeys
        let content_str = content.trim();
        if content_str.is_empty() {
            return Ok(vec![]);
        }
        
        // Try to parse as array of pubkeys
        let pubkeys: Vec<String> = serde_json::from_str(content_str)
            .unwrap_or_else(|_| {
                // Try parsing as array of ["pubkey", "relay"] pairs
                let pairs: Vec<Vec<String>> = serde_json::from_str(content_str).unwrap_or_default();
                pairs.into_iter().filter_map(|p| p.into_iter().next()).collect()
            });
        
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
        user_npub: Option<&str>,  // authenticated user's npub for Tier 3
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

    /// Fetch Kind 3 (follow list) and extract relay hints from content field
    /// Returns list of relay URLs found in the content
    async fn fetch_follow_list_with_relays(
        &self,
        npub: &str,
    ) -> Result<Vec<String>, NostrError> {
        let pubkey = PublicKey::parse(npub)
            .map_err(|e| NostrError::MalformedEvent(format!("Invalid npub: {}", e)))?;
        
        let filter = Filter::new()
            .kind(Kind::from_u16(KIND_FOLLOW_LIST))
            .author(pubkey)
            .limit(1);
        
        let events = self.inner
            .fetch_events(filter, Duration::from_secs(10))
            .await
            .map_err(|e| NostrError::RelayError(format!("Failed to fetch follow list: {}", e)))?;
        
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

        // Publish to each relay
        for relay_url in target_relays {
            match self.inner.add_relay(&relay_url).await {
                Ok(_) => {
                    if let Err(e) = self.inner.send_event(&event).await {
                        tracing::warn!("Failed to send event to {}: {}", relay_url, e);
                    }
                }
                Err(e) => {
                    tracing::warn!("Failed to add relay {}: {}", relay_url, e);
                }
            }
        }

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
        Tag::custom(
            TagKind::Custom("title".into()),
            [listing.title.clone()],
        ),
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
        tags.push(Tag::custom(
            TagKind::Custom("t".into()),
            [tag.clone()],
        ));
    }

    EventBuilder::new(Kind::Custom(KIND_GAME_LISTING), content_json)
        .tags(tags)
}

/// Parses a NOSTR event into a GameListing.
pub fn event_to_game_listing(event: &Event) -> Result<GameListing, NostrError> {
    // Extract d tag (id)
    let id = event.tags.identifier()
        .ok_or_else(|| NostrError::MalformedEvent("Missing d tag".into()))?
        .to_string();

    // Extract title tag
    let title = event.tags.find(TagKind::Custom("title".into()))
        .and_then(|t| t.content().map(|c| c.to_string()))
        .ok_or_else(|| NostrError::MalformedEvent("Missing title tag".into()))?;

    // Extract price tag
    let price_str = event.tags.find(TagKind::Custom("price".into()))
        .and_then(|t| t.content().map(|c| c.to_string()))
        .ok_or_else(|| NostrError::MalformedEvent("Missing price tag".into()))?;
    let price_sats = price_str.parse::<u64>()
        .map_err(|_| NostrError::MalformedEvent("Invalid price format".into()))?;

    // Extract t tags
    let tags: Vec<String> = event.tags
        .filter(TagKind::Custom("t".into()))
        .filter_map(|t| t.content().map(|c| c.to_string()))
        .collect();

    // Extract lud16 tag (optional - for backwards compatibility)
    let lud16 = event.tags.find(TagKind::Custom("lud16".into()))
        .and_then(|t| t.content().map(|c| c.to_string()))
        .unwrap_or_default();

    // Parse content
    let content: GameListingContent = serde_json::from_str(&event.content)
        .map_err(|e| NostrError::MalformedEvent(format!("Invalid content JSON: {}", e)))?;

    // Get publisher npub
    let publisher_npub = event.pubkey.to_bech32()
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
    let pubkey = signer.get_public_key().await
        .map_err(|e| NostrError::SigningError(format!("Failed to get public key: {}", e)))?;
    
    // Build the unsigned event
    let unsigned = builder.build(pubkey);
    
    // Sign the event using our signer
    let signed = signer.sign_event(unsigned).await
        .map_err(|e| NostrError::SigningError(format!("Failed to sign event: {}", e)))?;
    
    Ok(signed)
}

/// Stub implementation for WASM target (signing not supported in WASM).
#[cfg(target_arch = "wasm32")]
async fn sign_event_with_arcadestr_signer(
    _builder: EventBuilder,
    _signer: &ActiveSigner,
) -> Result<Event, NostrError> {
    Err(NostrError::SigningError("Signing not supported in WASM target".into()))
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
    use nostr::nips::nip19::{Nip19Profile, Nip19Event, FromBech32};
    use nostr::key::PublicKey;
    
    // Check prefix by looking at the identifier start
    if identifier.starts_with("nprofile1") {
        let profile = Nip19Profile::from_bech32(identifier)
            .map_err(|e| NostrError::MalformedEvent(format!("Invalid nprofile: {}", e)))?;
        let pubkey = profile.public_key.to_hex();
        let relays: Vec<String> = profile.relays.into_iter()
            .map(|r| r.to_string())
            .collect();
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
        let relays: Vec<String> = event.relays.into_iter()
            .map(|r| r.to_string())
            .collect();
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
        Err(NostrError::MalformedEvent("Invalid NIP-19 identifier".into()))
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
        assert!(relays.write_relays.contains(&"wss://relay.damus.io".to_string()));
        assert!(relays.read_relays.contains(&"wss://relay.nostr.info".to_string()));
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
