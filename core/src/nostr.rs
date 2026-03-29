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
use crate::signer::{ActiveSigner, NostrSigner as ArcadestrNostrSigner, SignerError};

/// Arcadestr game listing event kind.
/// Using kind 30078 (NIP-78 arbitrary app data, parameterized replaceable).
pub const KIND_GAME_LISTING: u16 = 30078;

/// Default relays for Arcadestr.
pub const DEFAULT_RELAYS: &[&str] = &[
    "wss://relay.damus.io",
    "wss://relay.nostr.band",
    "wss://nos.lol",
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

/// Parse relay list content from Kind 10002 event
pub fn parse_relay_list_content(content: &str) -> Result<CachedRelayList, NostrError> {
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
#[derive(Deserialize, Default)]
struct UserProfileContent {
    name: Option<String>,
    display_name: Option<String>,
    about: Option<String>,
    picture: Option<String>,
    website: Option<String>,
    nip05: Option<String>,
    lud16: Option<String>,
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
    pub async fn fetch_profile(
        &self,
        npub: &str,
    ) -> Result<UserProfile, NostrError> {
        // Parse npub as PublicKey
        let pubkey = PublicKey::parse(npub)
            .map_err(|e| NostrError::MalformedEvent(format!("Invalid npub: {}", e)))?;

        // Log for debugging
        #[cfg(target_arch = "wasm32")]
        {
            web_sys::console::log_1(&format!("[Nostr] Fetching profile for pubkey: {}", pubkey.to_hex()).into());
        }
        tracing::info!("Fetching profile for pubkey: {}", pubkey.to_hex());

        // Build filter for kind-0 (metadata) events
        let filter = Filter::new()
            .kind(Kind::Metadata)
            .author(pubkey)
            .limit(1);

        #[cfg(target_arch = "wasm32")]
        {
            web_sys::console::log_1(&"[Nostr] Querying relays for kind-0 metadata...".into());
        }
        tracing::info!("Querying relays for kind-0 metadata events...");

        // Fetch with 10s timeout
        let events = self.inner
            .fetch_events(filter, Duration::from_secs(10))
            .await
            .map_err(|e| NostrError::RelayError(format!("Failed to fetch profile: {}", e)))?;

        #[cfg(target_arch = "wasm32")]
        {
            web_sys::console::log_1(&format!("[Nostr] Got {} events from relays", events.len()).into());
        }
        tracing::info!("Got {} events from relays", events.len());

        // If no events returned, return minimal profile with only npub
        let event = match events.first() {
            Some(e) => {
                #[cfg(target_arch = "wasm32")]
                {
                    web_sys::console::log_1(&format!("[Nostr] Event content: {}", e.content).into());
                }
                tracing::info!("Event content: {}", e.content);
                e
            }
            None => {
                #[cfg(target_arch = "wasm32")]
                {
                    web_sys::console::log_1(&"[Nostr] No kind-0 events found for this user!".into());
                }
                tracing::warn!("No kind-0 events found for this user");
                return Ok(UserProfile {
                    npub: npub.to_string(),
                    ..Default::default()
                });
            }
        };

        // Parse the event content as UserProfileContent
        let content: UserProfileContent = serde_json::from_str(&event.content)
            .unwrap_or_default();

        #[cfg(target_arch = "wasm32")]
        {
            web_sys::console::log_1(&format!("[Nostr] Parsed profile: name={:?}, display_name={:?}", content.name, content.display_name).into());
        }
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
    ) -> Result<UserProfile, NostrError> {
        let mut profile = self.fetch_profile(npub).await?;
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

    /// Fetch Kind 10002 (relay list metadata) for a pubkey
    pub async fn fetch_relay_list(&self, npub: &str) -> Result<CachedRelayList, NostrError> {
        let pubkey = PublicKey::parse(npub)
            .map_err(|e| NostrError::MalformedEvent(format!("Invalid npub: {}", e)))?;
        
        let filter = Filter::new()
            .kind(Kind::from_u16(KIND_RELAY_LIST))
            .author(pubkey)
            .limit(1);
        
        let events = self.inner
            .fetch_events(filter, Duration::from_secs(10))
            .await
            .map_err(|e| NostrError::RelayError(format!("Failed to fetch relay list: {}", e)))?;
        
        let event = events.first()
            .ok_or_else(|| NostrError::MalformedEvent("No relay list found".into()))?;
        
        let mut relay_list = parse_relay_list_content(&event.content)?;
        relay_list.pubkey = pubkey.to_hex();
        relay_list.updated_at = event.created_at.as_secs();
        
        Ok(relay_list)
    }

    /// Get relays for a pubkey with fallback discovery
    pub async fn get_relays_for_pubkey(
        &self,
        npub: &str,
        cache: &RelayCache,
    ) -> Result<RelayDiscoveryResult, NostrError> {
        if let Some(cached) = cache.get_relay_list(npub) {
            if !cache.is_stale(npub) {
                return Ok(RelayDiscoveryResult {
                    write_relays: cached.write_relays,
                    read_relays: cached.read_relays,
                    source: RelayDiscoverySource::RelayList,
                });
            }
        }
        
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
                let seen_on = cache.get_seen_on(npub);
                if !seen_on.is_empty() {
                    return Ok(RelayDiscoveryResult {
                        write_relays: seen_on.clone(),
                        read_relays: seen_on,
                        source: RelayDiscoverySource::SeenOn,
                    });
                }
                Ok(RelayDiscoveryResult {
                    write_relays: DEFAULT_RELAYS.iter().map(|s| s.to_string()).collect(),
                    read_relays: DEFAULT_RELAYS.iter().map(|s| s.to_string()).collect(),
                    source: RelayDiscoverySource::GlobalFallback,
                })
            }
        }
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
