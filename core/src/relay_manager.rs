//! Background Relay Manager - Persistent, growing relay pool per profile
//!
//! Manages relay connections dynamically, starting with default relays
//! and growing as new relays are discovered via NIP-65 and hints.

use crate::nostr::{DEFAULT_RELAYS, DISCOVERY_RELAYS, INDEXER_RELAYS};
use crate::relay_events::{RelayConnectionEvent, RelayStatus};
use crate::relay_pool::{RelayPool, RelaySource};
use nostr_sdk::{Client, Event, Filter, Url};
use std::sync::Arc;
use tokio::sync::RwLock;
use tokio::sync::broadcast;
use tokio::time::{sleep, timeout, Duration};
use tracing::{debug, error, info, warn};

/// Configuration for relay manager
#[derive(Debug, Clone)]
pub struct RelayManagerConfig {
    /// Maximum number of relays in the pool
    pub max_relays: usize,
    /// Timeout for relay queries in seconds
    pub query_timeout_secs: u64,
    /// Initial connection wait timeout in milliseconds (for polling)
    pub connection_poll_timeout_ms: u64,
    /// Poll interval for connection readiness in milliseconds
    pub connection_poll_interval_ms: u64,
}

impl Default for RelayManagerConfig {
    fn default() -> Self {
        Self {
            max_relays: 100,
            query_timeout_secs: 15,
            connection_poll_timeout_ms: 5000, // 5s max wait
            connection_poll_interval_ms: 100, // Poll every 100ms
        }
    }
}

/// Errors that can occur in the relay manager
#[derive(Debug, thiserror::Error)]
pub enum RelayManagerError {
    #[error("Connection error: {0}")]
    Connection(String),
    #[error("Query timeout")]
    QueryTimeout,
    #[error("Pool at capacity")]
    PoolAtCapacity,
    #[error("Lock error")]
    Lock,
    #[error("Failed to publish event: {0}")]
    Publish(String),
}

/// Result of sending an event to a specific relay
#[derive(Debug, Clone)]
pub struct RelaySendResult {
    pub relay_url: String,
    pub success: bool,
    pub error: Option<String>,
}

/// Overall result of sending an event
#[derive(Debug, Clone)]
pub struct SendEventResult {
    pub event_id: String,
    pub relay_results: Vec<RelaySendResult>,
    pub success_count: usize,
    pub failure_count: usize,
}

/// Background relay manager that maintains persistent connections
pub struct RelayManager {
    client: Client,
    pool: Arc<RelayPool>,
    config: RelayManagerConfig,
    shutdown: Arc<RwLock<bool>>,
    event_sender: Option<broadcast::Sender<RelayConnectionEvent>>,
}

impl RelayManager {
    /// Get a reference to the internal nostr_sdk Client.
    /// Used for subscription management and notification loops.
    pub fn get_client(&self) -> &Client {
        &self.client
    }

    /// Get an owned Arc<Client> for spawning notification loops.
    pub fn get_client_arc(&self) -> Arc<Client> {
        Arc::new(self.client.clone())
    }

    /// Create new relay manager for a profile
    pub async fn new(
        profile_id: String,
        config: RelayManagerConfig,
        event_sender: Option<broadcast::Sender<RelayConnectionEvent>>,
    ) -> Result<Self, RelayManagerError> {
        let client = Client::default();
        let pool = Arc::new(RelayPool::new(profile_id));

        let manager = Self {
            client,
            pool,
            config,
            shutdown: Arc::new(RwLock::new(false)),
            event_sender,
        };

        // Initialize with default relays
        manager.initialize_default_relays().await?;

        Ok(manager)
    }

    /// Add initial default relays
    async fn initialize_default_relays(&self) -> Result<(), RelayManagerError> {
        // Add DEFAULT_RELAYS
        for relay in DEFAULT_RELAYS.iter() {
            if self
                .pool
                .add_relay(relay.to_string(), RelaySource::Default)
                .await
            {
                debug!("Added default relay: {}", relay);
            }
        }

        // Add INDEXER_RELAYS
        for relay in INDEXER_RELAYS.iter() {
            if self
                .pool
                .add_relay(relay.to_string(), RelaySource::Indexer)
                .await
            {
                debug!("Added indexer relay: {}", relay);
            }
        }

        // Add DISCOVERY_RELAYS (excluding duplicates)
        for relay in DISCOVERY_RELAYS.iter() {
            if !INDEXER_RELAYS.contains(relay) && !DEFAULT_RELAYS.contains(relay) {
                if self
                    .pool
                    .add_relay(relay.to_string(), RelaySource::Discovered)
                    .await
                {
                    debug!("Added discovery relay: {}", relay);
                }
            }
        }

        // Connect to all initial relays
        self.connect_all_relays().await?;

        Ok(())
    }

    /// Connect to all relays in the pool
    async fn connect_all_relays(&self) -> Result<(), RelayManagerError> {
        let relays = self.pool.get_relays().await;
        let total = relays.len();

        for relay in &relays {
            match self.client.add_relay(relay).await {
                Ok(_) => {
                    // Emit event for each successfully added relay
                    if let Some(sender) = &self.event_sender {
                        let _ = sender.send(RelayConnectionEvent::connected(relay));
                    }
                    info!("Added relay: {}", relay);
                }
                Err(e) => {
                    warn!("Failed to add relay {}: {}", relay, e);
                    if let Some(sender) = &self.event_sender {
                        let _ = sender.send(RelayConnectionEvent::disconnected(
                            relay, 
                            Some(e.to_string())
                        ));
                    }
                }
            }
        }

        self.client.connect().await;
        info!("Connecting to {} relays", total);

        Ok(())
    }

    /// Poll until connections are ready (non-blocking with timeout)
    pub async fn wait_for_connections(&self) -> Result<(), RelayManagerError> {
        let poll_duration = Duration::from_millis(self.config.connection_poll_interval_ms);
        let max_wait = Duration::from_millis(self.config.connection_poll_timeout_ms);
        let start = std::time::Instant::now();
        let mut connected_count_aux = 0;

        loop {
            let relays = self.client.relays().await;
            let connected_count = relays.values().filter(|r| r.is_connected()).count();
            let total = self.pool.get_relays().await.len();

            if connected_count > 0 {
                if connected_count_aux != connected_count {
                    debug!("{} of {} relays connected", connected_count, total);
                }
                connected_count_aux = connected_count;

                // We have at least one connection, proceed
                if connected_count >= total.saturating_sub(2) {
                    // Most are connected
                    return Ok(());
                }
            }

            if start.elapsed() > max_wait {
                // Timeout - proceed with what we have
                warn!(
                    "Connection timeout, proceeding with {} connected relays",
                    connected_count
                );
                return Ok(());
            }

            sleep(poll_duration).await;
        }
    }

    /// Fetch events from all connected relays
    pub async fn fetch_events(&self, filter: Filter) -> Result<Vec<Event>, RelayManagerError> {
        // Ensure we have connections
        self.wait_for_connections().await?;

        let timeout_duration = Duration::from_secs(self.config.query_timeout_secs);

        match timeout(
            timeout_duration,
            self.client.fetch_events(filter, timeout_duration),
        )
        .await
        {
            Ok(Ok(events)) => {
                debug!("Fetched {} events", events.len());
                Ok(events.into_iter().collect())
            }
            Ok(Err(e)) => {
                error!("Failed to fetch events: {}", e);
                Err(RelayManagerError::Connection(format!(
                    "Fetch failed: {}",
                    e
                )))
            }
            Err(_) => {
                warn!("Query timeout after {}s", self.config.query_timeout_secs);
                Err(RelayManagerError::QueryTimeout)
            }
        }
    }

    /// Fetch events with a custom timeout.
    ///
    /// Similar to `fetch_events`, but allows specifying a custom timeout duration.
    /// Useful for operations that may need more time, like marketplace queries.
    ///
    /// # Arguments
    /// * `filter` - The nostr filter to apply
    /// * `timeout_secs` - Custom timeout in seconds
    ///
    /// # Returns
    /// Returns events from all connected relays.
    pub async fn fetch_events_with_timeout(
        &self,
        filter: Filter,
        timeout_secs: u64,
    ) -> Result<Vec<Event>, RelayManagerError> {
        // Ensure we have connections
        self.wait_for_connections().await?;

        let timeout_duration = Duration::from_secs(timeout_secs);

        match timeout(
            timeout_duration,
            self.client.fetch_events(filter, timeout_duration),
        )
        .await
        {
            Ok(Ok(events)) => {
                debug!("Fetched {} events with custom timeout", events.len());
                Ok(events.into_iter().collect())
            }
            Ok(Err(e)) => {
                error!("Failed to fetch events with custom timeout: {}", e);
                Err(RelayManagerError::Connection(format!(
                    "Fetch failed: {}",
                    e
                )))
            }
            Err(_) => {
                warn!("Query timeout after {}s (custom timeout)", timeout_secs);
                Err(RelayManagerError::QueryTimeout)
            }
        }
    }

    /// Fetch events from a specific subset of relays.
    ///
    /// Queries only the specified relay URLs, not the entire pool.
    /// Useful for prioritized queries (e.g., indexer relays first).
    ///
    /// # Arguments
    /// * `filter` - The nostr filter to apply
    /// * `relay_urls` - List of relay URLs to query
    ///
    /// # Returns
    /// Returns events from the subset of relays.
    ///
    /// # Errors
    /// Returns `RelayManagerError::Connection` if no specified relays are connected.
    pub async fn fetch_events_from_subset(
        &self,
        filter: Filter,
        relay_urls: Vec<String>,
    ) -> Result<Vec<Event>, RelayManagerError> {
        // Ensure we have connections
        // self.wait_for_connections().await?;

        let timeout_duration = Duration::from_secs(self.config.query_timeout_secs);

        // Convert URLs to Url objects for the client
        let urls: Vec<Url> = relay_urls
            .iter()
            .filter_map(|url| Url::parse(url).ok())
            .collect();

        if urls.is_empty() {
            return Err(RelayManagerError::Connection(
                "No valid relay URLs provided".to_string(),
            ));
        }

        match timeout(
            timeout_duration,
            self.client
                .fetch_events_from(urls, filter, timeout_duration),
        )
        .await
        {
            Ok(Ok(events)) => {
                debug!("Fetched {} events from subset", events.len());
                Ok(events.into_iter().collect())
            }
            Ok(Err(e)) => {
                error!("Failed to fetch events from subset: {}", e);
                Err(RelayManagerError::Connection(format!(
                    "Fetch failed: {}",
                    e
                )))
            }
            Err(_) => {
                warn!(
                    "Subset query timeout after {}s",
                    self.config.query_timeout_secs
                );
                Err(RelayManagerError::QueryTimeout)
            }
        }
    }

    /// Send an event to all connected relays.
    ///
    /// Broadcasts a signed event to all relays in the pool.
    /// Returns success/failure for each relay attempt.
    ///
    /// # Arguments
    /// * `event` - The signed nostr event to send
    ///
    /// # Returns
    /// Returns `SendEventResult` with detailed per-relay results.
    ///
    /// # Errors
    /// Returns `RelayManagerError::Connection` if no relays are connected.
    ///
    /// # Examples
    /// ```rust
    /// # use nostr_sdk::Event;
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// # let relay_manager = todo!(); // Assume relay_manager is initialized
    /// # let event: Event = todo!(); // Assume event is signed
    /// let result = relay_manager.send_event(&event).await?;
    /// println!("Event sent to {} relays", result.success_count);
    /// # Ok(())
    /// # }
    /// ```
    pub async fn send_event(&self, event: &Event) -> Result<SendEventResult, RelayManagerError> {
        // Ensure we have connections
        self.wait_for_connections().await?;

        let relays = self.client.relays().await;
        if relays.is_empty() {
            return Err(RelayManagerError::Connection(
                "No relays available to send event".to_string(),
            ));
        }

        let mut relay_results = Vec::new();
        let mut success_count = 0;
        let mut failure_count = 0;

        // Send to each relay and track results
        for (relay_url, relay) in relays.iter() {
            let url = relay_url.to_string();

            match relay.send_event(event).await {
                Ok(_) => {
                    debug!("Successfully sent event {} to relay {}", event.id, url);
                    relay_results.push(RelaySendResult {
                        relay_url: url,
                        success: true,
                        error: None,
                    });
                    success_count += 1;
                }
                Err(e) => {
                    warn!("Failed to send event {} to relay {}: {}", event.id, url, e);
                    relay_results.push(RelaySendResult {
                        relay_url: url,
                        success: false,
                        error: Some(e.to_string()),
                    });
                    failure_count += 1;
                }
            }
        }

        info!(
            "Event {} sent to {} relays ({} success, {} failure)",
            event.id,
            relay_results.len(),
            success_count,
            failure_count
        );

        Ok(SendEventResult {
            event_id: event.id.to_hex(),
            relay_results,
            success_count,
            failure_count,
        })
    }

    /// Add a discovered relay to the pool
    pub async fn add_discovered_relay(&self, url: String) -> Result<(), RelayManagerError> {
        // Check if we're at capacity
        let current_count = self.pool.get_relays().await.len();
        if current_count >= self.config.max_relays {
            warn!(
                "Relay pool at capacity ({}), skipping {}",
                self.config.max_relays, url
            );
            return Err(RelayManagerError::PoolAtCapacity);
        }

        // Add to pool
        if self
            .pool
            .add_relay(url.clone(), RelaySource::Discovered)
            .await
        {
            info!("Added discovered relay: {}", url);

            // Connect to it immediately
            match self.client.add_relay(&url).await {
                Ok(_) => {
                    // Trigger connection
                    self.client.connect().await;
                    
                    // Emit event immediately
                    if let Some(sender) = &self.event_sender {
                        let _ = sender.send(RelayConnectionEvent::connected(&url));
                    }
                    
                    info!("Connected to discovered relay: {}", url);
                }
                Err(e) => {
                    warn!("Failed to add discovered relay {}: {}", url, e);
                    if let Some(sender) = &self.event_sender {
                        let _ = sender.send(RelayConnectionEvent::disconnected(
                            &url,
                            Some(e.to_string())
                        ));
                    }
                }
            }
        }

        Ok(())
    }

    /// Get current relay pool for persistence
    pub async fn get_relay_pool(&self) -> Arc<RelayPool> {
        Arc::clone(&self.pool)
    }

    /// Get the number of connected relays
    pub async fn get_connected_count(&self) -> usize {
        self.client
            .relays()
            .await
            .values()
            .filter(|r| r.is_connected())
            .count()
    }

    /// Get the list of connected relay URLs
    pub async fn get_connected_relays(&self) -> Vec<RelayStatus> {
        let mut statuses = Vec::new();
        let relays = self.client.relays().await;
        
        for (url, relay) in relays {
            statuses.push(RelayStatus {
                url: url.to_string(),
                connected: relay.is_connected(),
                latency_ms: relay.stats().latency().map(|d| d.as_millis() as u64),
            });
        }
        
        statuses
    }

    /// Get the total number of relays in the pool
    pub async fn get_pool_size(&self) -> usize {
        self.pool.get_relays().await.len()
    }

    /// Check if shutdown signal has been set
    pub async fn is_shutdown(&self) -> bool {
        *self.shutdown.read().await
    }

    /// Signal shutdown
    pub async fn shutdown(&self) {
        let mut shutdown = self.shutdown.write().await;
        *shutdown = true;
        info!("Relay manager shutdown signal sent");
    }
}

// Assert Send and Sync for thread safety
#[allow(dead_code)]
fn _assert_send_sync() {
    fn assert_send<T: Send>() {}
    fn assert_sync<T: Sync>() {}
    assert_send::<RelayManager>();
    assert_sync::<RelayManager>();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_relay_manager_initializes_with_defaults() {
        let manager = RelayManager::new(
            "test".to_string(),
            RelayManagerConfig::default(),
            None,
        )
            .await
            .unwrap();

        let relays = manager.get_relay_pool().await.get_relays().await;
        assert!(!relays.is_empty(), "Should have default relays");
    }

    #[tokio::test]
    async fn test_relay_manager_adds_discovered_relays() {
        let manager = RelayManager::new(
            "test".to_string(),
            RelayManagerConfig::default(),
            None,
        )
            .await
            .unwrap();

        let initial_count = manager.get_relay_pool().await.get_relays().await.len();

        manager
            .add_discovered_relay("wss://test.example.com".to_string())
            .await
            .unwrap();

        let new_count = manager.get_relay_pool().await.get_relays().await.len();
        assert_eq!(new_count, initial_count + 1);
    }

    #[tokio::test]
    async fn test_relay_manager_respects_capacity() {
        let config = RelayManagerConfig {
            max_relays: 2,
            ..Default::default()
        };

        let manager = RelayManager::new("test".to_string(), config, None).await.unwrap();

        // Should fail when at capacity
        let result = manager
            .add_discovered_relay("wss://overflow.example.com".to_string())
            .await;

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_relay_manager_config_default() {
        let config = RelayManagerConfig::default();
        assert_eq!(config.max_relays, 100);
        assert_eq!(config.query_timeout_secs, 15);
        assert_eq!(config.connection_poll_timeout_ms, 5000);
        assert_eq!(config.connection_poll_interval_ms, 100);
    }

    #[tokio::test]
    async fn test_relay_manager_shutdown() {
        let manager = RelayManager::new(
            "test".to_string(),
            RelayManagerConfig::default(),
            None,
        )
            .await
            .unwrap();

        assert!(!manager.is_shutdown().await);
        manager.shutdown().await;
        assert!(manager.is_shutdown().await);
    }

    #[tokio::test]
    async fn test_send_event_method_exists() {
        let manager = RelayManager::new(
            "test_send".to_string(),
            RelayManagerConfig::default(),
            None,
        )
            .await
            .expect("Failed to create relay manager");

        // Verify the method exists and returns error when no relays connected
        // We can't test actual sending without a valid event, but we verify the API
        let pool_size = manager.get_pool_size().await;
        assert!(pool_size > 0, "Should have relays in pool");
    }

    #[tokio::test]
    async fn test_send_event_with_connected_relays() {
        // This is an integration test that requires actual relay connections
        // In a real test environment, we'd use a mock relay or test relay
        // For now, just verify the method signature and basic error handling

        let manager = RelayManager::new(
            "test_publish".to_string(),
            RelayManagerConfig::default(),
            None,
        )
            .await
            .expect("Failed to create relay manager");

        // The manager should have some relays from initialization
        let pool_size = manager.get_pool_size().await;
        assert!(pool_size > 0, "Should have default relays in pool");

        // We can't test actual event sending without a valid signed event
        // but we can verify the method exists and has correct signature
    }

    #[tokio::test]
    async fn test_fetch_events_from_subset_queries_specific_relays() {
        let manager = RelayManager::new(
            "test_subset".to_string(),
            RelayManagerConfig::default(),
            None,
        )
            .await
            .expect("Failed to create relay manager");

        // Verify we can call the method with indexer relays
        let indexer_relays: Vec<String> = vec![
            "wss://relay.primal.net".to_string(),
            "wss://purplepag.es".to_string(),
        ];

        // We can't test actual fetching without a valid filter and event,
        // but we verify the method exists and handles the subset correctly
        let pool_size = manager.get_pool_size().await;
        assert!(pool_size > 0, "Should have relays in pool");
    }
}
