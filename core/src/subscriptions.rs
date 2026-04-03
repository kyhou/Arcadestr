// Subscription manager for persistent WebSocket connections
// Replaces polling with real-time event streaming

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use nostr_sdk::prelude::*;
use serde::Serialize;

use crate::relay_cache::RelayCache;
use crate::relay_hints::RelayHints;

/// Connection type for lifecycle management
#[derive(Clone, PartialEq, Debug)]
pub enum ConnectionKind {
    Permanent,
    EphemeralRead,
    EphemeralWrite,
}

/// Subscription registry to track connection types by ID
pub struct SubscriptionRegistry {
    /// subscription_id → ConnectionKind
    pub entries: Arc<Mutex<HashMap<String, ConnectionKind>>>,
}

impl SubscriptionRegistry {
    /// Create a new empty registry
    pub fn new() -> Self {
        Self {
            entries: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Register a subscription with its connection type
    pub fn register(&self, id: String, kind: ConnectionKind) {
        if let Ok(mut entries) = self.entries.lock() {
            entries.insert(id, kind);
        }
    }

    /// Get the connection kind for a subscription ID
    pub fn get_kind(&self, id: &str) -> Option<ConnectionKind> {
        self.entries.lock().ok()?.get(id).cloned()
    }

    /// Remove a subscription from the registry
    pub fn remove(&self, id: &str) {
        if let Ok(mut entries) = self.entries.lock() {
            entries.remove(id);
        }
    }
}

impl Default for SubscriptionRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Serializable event for frontend consumption
#[derive(Serialize, Clone, Debug)]
pub struct SerializableEvent {
    pub id: String,
    pub pubkey: String,
    pub created_at: u64,
    pub kind: u16,
    pub content: String,
    pub tags: Vec<Vec<String>>,
    pub sig: String,
    pub relay_url: String,
}

impl From<(Event, String)> for SerializableEvent {
    fn from((event, relay_url): (Event, String)) -> Self {
        Self {
            id: event.id.to_hex(),
            pubkey: event.pubkey.to_hex(),
            created_at: event.created_at.as_secs(),
            kind: event.kind.as_u16(),
            content: event.content.clone(),
            tags: event.tags.iter().map(|t| t.clone().to_vec()).collect(),
            sig: event.sig.to_string(),
            relay_url,
        }
    }
}

/// Event callback type for pushing events to the frontend
type EventCallback = Box<dyn Fn(SerializableEvent) + Send + Sync>;

/// The central notification loop - runs for the lifetime of the app
/// Handles all incoming relay messages and dispatches to frontend via callback
pub async fn run_notification_loop(
    client: Arc<Client>,
    relay_cache: Arc<RelayCache>,
    registry: Arc<SubscriptionRegistry>,
    relay_hints: Option<Arc<RelayHints>>,
    event_callback: EventCallback,
) {
    let mut notifications = client.notifications();

    loop {
        match notifications.recv().await {
            Ok(RelayPoolNotification::Event {
                relay_url,
                subscription_id,
                event,
            }) => {
                // Fix 3: update seen_on unconditionally
                let _ = relay_cache.update_seen_on(&event.pubkey.to_hex(), &relay_url.to_string());

                // Extract relay hints from events
                if let Some(ref hints) = relay_hints {
                    if let Err(e) = hints.extract_hints_from_event(&event) {
                        tracing::debug!("Failed to extract hints from event: {}", e);
                    }
                }

                // De-duplication check
                let event_id = event.id.to_hex();
                if relay_cache.is_seen_event(&event_id) {
                    continue;
                }
                relay_cache.mark_event_seen(&event_id);

                // Push to frontend via callback
                let serializable =
                    SerializableEvent::from(((*event).clone(), relay_url.to_string()));
                event_callback(serializable);
            }

            Ok(RelayPoolNotification::Message { relay_url, message }) => {
                match message {
                    // Fix 9: close ephemeral read connections on EOSE
                    RelayMessage::EndOfStoredEvents(sub_id) => {
                        if let Some(ConnectionKind::EphemeralRead) =
                            registry.get_kind(&sub_id.to_string())
                        {
                            let _ = client.unsubscribe(&sub_id).await;
                            registry.remove(&sub_id.to_string());
                        }
                    }

                    // Fix 10: close ephemeral write connections on OK
                    RelayMessage::Ok {
                        event_id, status, ..
                    } => {
                        let id = event_id.to_hex();
                        if let Some(ConnectionKind::EphemeralWrite) = registry.get_kind(&id) {
                            // Disconnect the relay after receiving OK
                            let _ = client.disconnect_relay(&relay_url).await;
                            registry.remove(&id);
                        }
                    }

                    _ => {}
                }
            }

            Ok(RelayPoolNotification::Shutdown) => break,
            Err(_) => {
                // Channel closed or error, exit loop
                break;
            }
        }
    }
}

/// Dispatch targeted subscriptions to permanent relays (Fix 4)
/// Each relay gets a subscription with only its assigned pubkeys
pub async fn dispatch_permanent_subscriptions(
    client: &Client,
    relay_map: &HashMap<String, HashSet<String>>, // relay_url → pubkeys
    registry: &Arc<SubscriptionRegistry>,
    relay_cache: &Arc<RelayCache>,
) {
    for (relay_url, pubkeys) in relay_map {
        // Check connection limit before subscribing
        if !relay_cache.can_open_permanent_connection() {
            tracing::warn!("Max permanent connections reached, skipping {}", relay_url);
            continue;
        }

        let sub_id = SubscriptionId::new(&format!("perm_{}", sanitize_relay_url(relay_url)));

        // Build filter with assigned pubkeys only
        let authors: Vec<PublicKey> = pubkeys
            .iter()
            .filter_map(|p| PublicKey::from_hex(p).ok())
            .collect();

        if authors.is_empty() {
            continue;
        }

        let filter = Filter::new()
            .authors(authors)
            .kinds(vec![Kind::TextNote, Kind::Repost, Kind::Reaction])
            .limit(50);

        // Add relay if not already connected
        let _ = client.add_relay(relay_url).await;

        // Subscribe to this specific relay only
        match Url::parse(relay_url) {
            Ok(url) => {
                // nostr_sdk 0.44 subscribe_to takes: relays, filter, opts
                let _ = client.subscribe_to(vec![url], filter, None).await;
                registry.register(sub_id.to_string(), ConnectionKind::Permanent);

                // Increment permanent connection counter
                relay_cache.increment_permanent_connection();

                tracing::info!("Subscribed to permanent relay: {}", relay_url);
            }
            Err(e) => {
                tracing::warn!("Invalid relay URL {}: {}", relay_url, e);
            }
        }
    }
}

/// Dispatch ephemeral read connection for uncovered pubkey (Fix 5)
pub async fn dispatch_ephemeral_read(
    client: &Client,
    pubkey: &str,
    relay_url: &str,
    registry: &Arc<SubscriptionRegistry>,
) {
    let sub_id = format!("eph_read_{}_{}", pubkey, sanitize_relay_url(relay_url));

    let filter = match PublicKey::from_hex(pubkey) {
        Ok(pk) => Filter::new()
            .author(pk)
            .kinds(vec![Kind::TextNote, Kind::Repost, Kind::Reaction])
            .limit(50),
        Err(_) => {
            tracing::warn!("Invalid pubkey for ephemeral read: {}", pubkey);
            return;
        }
    };

    // Add and connect to relay
    let _ = client.add_relay(relay_url).await;
    let _ = client.connect_relay(relay_url).await;

    // Subscribe
    match Url::parse(relay_url) {
        Ok(url) => {
            // nostr_sdk 0.44 subscribe_to takes: relays, filter, opts
            let _ = client.subscribe_to(vec![url], filter, None).await;
            // Tag it — the notification loop will close it on EOSE (Fix 9)
            registry.register(sub_id, ConnectionKind::EphemeralRead);
            tracing::info!("Started ephemeral read for {} on {}", pubkey, relay_url);
        }
        Err(e) => {
            tracing::warn!("Invalid relay URL {}: {}", relay_url, e);
        }
    }
}

/// Publish a note with ephemeral write connection tagging (Fix 10)
pub async fn publish_note(
    client: &Client,
    event: Event,
    relays: Vec<String>,
    registry: &Arc<SubscriptionRegistry>,
) {
    // Tag with event ID so the OK handler can close the write connection
    registry.register(event.id.to_hex(), ConnectionKind::EphemeralWrite);

    for relay_url in &relays {
        let _ = client.add_relay(relay_url).await;
        let _ = client.connect_relay(relay_url).await;
    }

    // Send the event - notification loop will handle OK responses
    match client.send_event(&event).await {
        Ok(_) => tracing::info!("Event sent successfully"),
        Err(e) => tracing::warn!("Failed to send event: {}", e),
    }
    // The notification loop closes the relay after receiving OK (Fix 10)
}

/// Helper to sanitize relay URL for use in subscription IDs
fn sanitize_relay_url(url: &str) -> String {
    url.replace("wss://", "")
        .replace("ws://", "")
        .replace("/", "_")
        .replace(":", "_")
}
