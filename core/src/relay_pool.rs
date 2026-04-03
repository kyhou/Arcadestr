//! Relay Pool - Maintains unified set of relays for a profile

use std::collections::HashSet;
use std::sync::Arc;
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

/// Information about a relay in the pool
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RelayInfo {
    pub url: String,
    pub added_at: u64, // Unix timestamp
    pub source: RelaySource,
    pub last_connected: Option<u64>,
    pub failure_count: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RelaySource {
    Default,    // From DEFAULT_RELAYS
    Indexer,    // From INDEXER_RELAYS
    Discovered, // From NIP-65 or hints
}

/// Thread-safe relay pool state
pub struct RelayPool {
    relays: Arc<RwLock<HashSet<String>>>,
    relay_info: Arc<RwLock<Vec<RelayInfo>>>,
    profile_id: String,
}

impl RelayPool {
    pub fn new(profile_id: String) -> Self {
        Self {
            relays: Arc::new(RwLock::new(HashSet::new())),
            relay_info: Arc::new(RwLock::new(Vec::new())),
            profile_id,
        }
    }

    /// Add a relay to the pool
    pub async fn add_relay(&self, url: String, source: RelaySource) -> bool {
        let mut relays = self.relays.write().await;
        if relays.contains(&url) {
            return false; // Already exists
        }
        relays.insert(url.clone());

        let mut info = self.relay_info.write().await;
        let added_at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or_else(|_| {
                tracing::warn!("System time is before UNIX epoch, using 0");
                0
            });
        info.push(RelayInfo {
            url,
            added_at,
            source,
            last_connected: None,
            failure_count: 0,
        });

        true
    }

    /// Get all relay URLs
    pub async fn get_relays(&self) -> Vec<String> {
        let relays = self.relays.read().await;
        relays.iter().cloned().collect()
    }

    /// Get profile ID
    pub fn profile_id(&self) -> &str {
        &self.profile_id
    }

    /// Check if relay exists
    pub async fn has_relay(&self, url: &str) -> bool {
        let relays = self.relays.read().await;
        relays.contains(url)
    }
}

// Assert Send and Sync for thread safety
#[allow(dead_code)]
fn _assert_send_sync() {
    fn assert_send<T: Send>() {}
    fn assert_sync<T: Sync>() {}
    assert_send::<RelayPool>();
    assert_sync::<RelayPool>();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_relay_pool_initialization() {
        let pool = RelayPool::new("test_profile".to_string());
        assert_eq!(pool.profile_id(), "test_profile");
        assert!(pool.get_relays().await.is_empty());
    }

    #[tokio::test]
    async fn test_add_relay() {
        let pool = RelayPool::new("test_profile".to_string());
        let url = "wss://relay.example.com".to_string();

        // Add relay for the first time
        let added = pool.add_relay(url.clone(), RelaySource::Default).await;
        assert!(added);

        // Try to add the same relay again
        let added_again = pool.add_relay(url.clone(), RelaySource::Default).await;
        assert!(!added_again);

        // Verify relay is in the pool
        let relays = pool.get_relays().await;
        assert_eq!(relays.len(), 1);
        assert!(relays.contains(&url));
    }

    #[tokio::test]
    async fn test_has_relay() {
        let pool = RelayPool::new("test_profile".to_string());
        let url = "wss://relay.example.com".to_string();

        assert!(!pool.has_relay(&url).await);

        pool.add_relay(url.clone(), RelaySource::Discovered).await;

        assert!(pool.has_relay(&url).await);
    }

    #[tokio::test]
    async fn test_multiple_relays() {
        let pool = RelayPool::new("test_profile".to_string());
        let url1 = "wss://relay1.example.com".to_string();
        let url2 = "wss://relay2.example.com".to_string();

        pool.add_relay(url1.clone(), RelaySource::Default).await;
        pool.add_relay(url2.clone(), RelaySource::Indexer).await;

        let relays = pool.get_relays().await;
        assert_eq!(relays.len(), 2);
        assert!(relays.contains(&url1));
        assert!(relays.contains(&url2));
    }
}
