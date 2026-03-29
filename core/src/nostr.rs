// NOSTR protocol integration: event handling, relay connections, NIP-46 signer support.

use std::time::Duration;

use nostr_sdk::prelude::*;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tracing::warn;

use crate::auth::AuthState;
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
        
        for relay in relays {
            match client.add_relay(&relay).await {
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
