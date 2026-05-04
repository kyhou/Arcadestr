// Extended Network Discovery - 2nd-degree follows with relay optimization
// Discovers friends-of-friends, filters by threshold, and computes optimal relay coverage
// Uses async streaming approach - relays are connected incrementally as they're discovered

use crate::nostr::{parse_relay_list_from_event, NostrClient, KIND_FOLLOW_LIST, KIND_RELAY_LIST};
use crate::relay_cache::RelayCache;
use crate::social_graph::SocialGraphDb;
use futures::stream::{FuturesUnordered, StreamExt};
use nostr_sdk::{Event, Filter, Kind};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use thiserror::Error;
use tokio::time::timeout;
use tracing::{debug, info, warn};

#[derive(Debug, Error)]
pub enum ExtendedNetworkError {
    #[error("Nostr client error: {0}")]
    Nostr(String),
    #[error("Timeout: {0}")]
    Timeout(String),
    #[error("Database error: {0}")]
    Database(String),
}

/// Threshold for qualifying as extended network member (must be followed by >= 10 1st-degree)
pub const QUALIFYING_THRESHOLD: usize = 10;

/// Maximum relays to add for extended network
const MAX_EXTENDED_RELAYS: usize = 100;

/// Maximum authors per relay cap
const MAX_AUTHORS_PER_RELAY: usize = 300;

/// Cache TTL: 24 hours
const CACHE_TTL_HOURS: u64 = 24;

/// Discovery timeout: 30 seconds
const DISCOVERY_TIMEOUT_SECS: u64 = 30;

/// Follow list fetch timeout: 10 seconds (conservative faster path)
const FOLLOW_LIST_TIMEOUT_SECS: u64 = 10;

/// Max first-degree authors queried per follow-list batch
const FOLLOW_LIST_AUTHORS_PER_BATCH: usize = 20;

/// Maximum concurrent follow-list batch requests.
const MAX_CONCURRENT_FOLLOW_LIST_BATCHES: usize = 3;

/// Relay list fetch timeout: 10 seconds (conservative faster path)
const RELAY_LIST_TIMEOUT_SECS: u64 = 10;

/// Extended network discovery state
#[derive(Debug, Clone, PartialEq)]
pub enum DiscoveryState {
    Idle,
    FetchingFollowLists {
        fetched: usize,
        total: usize,
        coverage_percent: u8,
    },
    BuildingGraph {
        processed: usize,
        total: usize,
    },
    ComputingNetwork {
        unique_users: usize,
    },
    Filtering {
        qualified: usize,
    },
    FetchingRelayLists {
        fetched: usize,
        total: usize,
    },
    Complete {
        stats: NetworkStats,
    },
    Failed {
        reason: String,
    },
}

/// Statistics about the extended network
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NetworkStats {
    pub first_degree_count: usize,
    pub total_second_degree: usize,
    pub qualified_count: usize,
    pub relays_covered: usize,
    pub computed_at: u64,
}

/// Cached extended network data
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtendedNetworkCache {
    pub qualified_pubkeys: HashSet<String>,
    pub first_degree_pubkeys: HashSet<String>,
    pub relay_urls: Vec<String>,
    pub relay_hints: HashMap<String, Vec<String>>, // pubkey -> relay hints
    pub stats: NetworkStats,
}

/// Extended network discovery manager
pub struct ExtendedNetworkRepository {
    my_pubkey: Option<String>,
    social_graph: Arc<SocialGraphDb>,
    discovery_state: Arc<Mutex<DiscoveryState>>,
    cached_network: Arc<Mutex<Option<ExtendedNetworkCache>>>,
    discovery_in_progress: Arc<Mutex<bool>>,
    allow_insecure_public_ws: Arc<Mutex<bool>>,
}

impl ExtendedNetworkRepository {
    pub fn new(social_graph: Arc<SocialGraphDb>) -> Self {
        Self {
            my_pubkey: None,
            social_graph,
            discovery_state: Arc::new(Mutex::new(DiscoveryState::Idle)),
            cached_network: Arc::new(Mutex::new(None)),
            discovery_in_progress: Arc::new(Mutex::new(false)),
            allow_insecure_public_ws: Arc::new(Mutex::new(false)),
        }
    }

    pub fn set_pubkey(&mut self, pubkey: String) {
        self.my_pubkey = Some(pubkey);
    }

    pub fn clear(&self) -> Result<(), ExtendedNetworkError> {
        self.social_graph
            .clear_all()
            .map_err(|e| ExtendedNetworkError::Database(e.to_string()))?;
        *self
            .discovery_state
            .lock()
            .expect("discovery_state mutex poisoned") = DiscoveryState::Idle;
        *self
            .cached_network
            .lock()
            .expect("cached_network mutex poisoned") = None;
        *self
            .discovery_in_progress
            .lock()
            .expect("discovery_in_progress mutex poisoned") = false;
        Ok(())
    }

    pub fn set_allow_insecure_public_ws(&self, allow: bool) {
        *self
            .allow_insecure_public_ws
            .lock()
            .expect("allow_insecure_public_ws mutex poisoned") = allow;
    }

    pub fn get_allow_insecure_public_ws(&self) -> bool {
        *self
            .allow_insecure_public_ws
            .lock()
            .expect("allow_insecure_public_ws mutex poisoned")
    }

    pub fn get_state(&self) -> DiscoveryState {
        self.discovery_state
            .lock()
            .expect("discovery_state mutex poisoned")
            .clone()
    }

    pub fn get_cached_network(&self) -> Option<ExtendedNetworkCache> {
        self.cached_network
            .lock()
            .expect("cached_network mutex poisoned")
            .clone()
    }

    pub fn is_cache_stale(&self) -> bool {
        let cache = self
            .cached_network
            .lock()
            .expect("cached_network mutex poisoned");
        match cache.as_ref() {
            None => true,
            Some(c) => {
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs())
                    .unwrap_or(0);
                let age_hours = (now - c.stats.computed_at) / 3600;
                age_hours >= CACHE_TTL_HOURS
            }
        }
    }

    /// Main discovery workflow - uses async streaming approach
    pub async fn discover_network(
        &self,
        nostr_client: &NostrClient,
        relay_cache: &RelayCache,
        first_degree_follows: Vec<String>,
    ) -> Result<NetworkStats, ExtendedNetworkError> {
        // Prevent concurrent discoveries
        {
            let mut in_progress = self
                .discovery_in_progress
                .lock()
                .expect("discovery_in_progress mutex poisoned");
            if *in_progress {
                return Err(ExtendedNetworkError::Nostr(
                    "Discovery already in progress".to_string(),
                ));
            }
            *in_progress = true;
        }

        // Clear previous data
        let _ = self.social_graph.clear_all();

        let result = self
            ._discover_network(nostr_client, relay_cache, first_degree_follows)
            .await;

        *self
            .discovery_in_progress
            .lock()
            .expect("discovery_in_progress mutex poisoned") = false;

        result
    }

    /// Streaming discovery: fetches relay lists and follow lists in parallel
    /// Connects to relays incrementally as they're discovered (Amethyst-style)
    async fn _discover_network(
        &self,
        nostr_client: &NostrClient,
        relay_cache: &RelayCache,
        first_degree_follows: Vec<String>,
    ) -> Result<NetworkStats, ExtendedNetworkError> {
        let my_pubkey = self
            .my_pubkey
            .clone()
            .ok_or_else(|| ExtendedNetworkError::Nostr("No pubkey set".to_string()))?;

        if first_degree_follows.is_empty() {
            return Err(ExtendedNetworkError::Nostr(
                "Follow list is empty".to_string(),
            ));
        }

        let first_degree_set: HashSet<String> = first_degree_follows.iter().cloned().collect();
        let total_first = first_degree_follows.len();
        let allow_insecure_public_ws = self.get_allow_insecure_public_ws();

        info!(
            "Starting streaming extended network discovery with {} first-degree follows",
            total_first
        );

        // Initialize state
        *self
            .discovery_state
            .lock()
            .expect("discovery_state mutex poisoned") = DiscoveryState::FetchingFollowLists {
            fetched: 0,
            total: total_first,
            coverage_percent: 0,
        };

        // Convert pubkeys to nostr_sdk PublicKey for filtering
        let authors: Vec<nostr_sdk::PublicKey> = first_degree_follows
            .iter()
            .filter_map(|p| nostr_sdk::PublicKey::from_hex(p).ok())
            .collect();

        if authors.is_empty() {
            return Err(ExtendedNetworkError::Nostr(
                "No valid pubkeys to query".to_string(),
            ));
        }

        // Step 1: Fetch NIP-65 relay lists and connect to them
        info!("Step 1: Discovering and connecting to user relays...");
        let relay_list_filter = Filter::new()
            .kind(Kind::Custom(KIND_RELAY_LIST))
            .authors(authors.clone());

        let timeout_duration = Duration::from_secs(RELAY_LIST_TIMEOUT_SECS);
        let mut connected_relays = 0;

        match timeout(
            timeout_duration,
            nostr_client.fetch_from_connected_best_effort(relay_list_filter),
        )
        .await
        {
            Ok(Ok(events)) => {
                let mut unique_relays: HashSet<String> = HashSet::new();

                for event in events {
                    if let Ok(relay_list) = parse_relay_list_from_event(&event) {
                        // Cache the relay list
                        let _ = relay_cache.save_relay_list(&relay_list);

                        // Connect to this user's write relays (max 3 per user)
                        for relay_url in relay_list.write_relays.iter().take(3) {
                            if let Some(normalized_relay) = Self::normalize_and_admit_relay_url(
                                relay_url,
                                allow_insecure_public_ws,
                            ) {
                                if unique_relays.insert(normalized_relay.clone()) {
                                    match nostr_client.add_relay(&normalized_relay).await {
                                        Ok(true) => {
                                            debug!("Connected to user relay: {}", normalized_relay);
                                            connected_relays += 1;
                                        }
                                        Ok(false) => {
                                            debug!("Relay already exists: {}", normalized_relay)
                                        }
                                        Err(e) => {
                                            debug!(
                                                "Failed to add relay {}: {}",
                                                normalized_relay, e
                                            )
                                        }
                                    }
                                }
                            } else {
                                debug!("Skipping relay not admitted by policy: {}", relay_url);
                            }
                        }
                    }
                }

                info!(
                    "Connected to {} new relays from NIP-65 lists",
                    connected_relays
                );
            }
            Ok(Err(e)) => warn!("Error fetching relay lists: {}", e),
            Err(_) => warn!("Timeout fetching relay lists"),
        }

        // Step 2: Fetch follow lists from all connected relays
        info!("Step 2: Fetching follow lists from all connected relays...");

        let mut follow_lists: HashMap<String, Event> = HashMap::new();
        let timeout_duration = Duration::from_secs(FOLLOW_LIST_TIMEOUT_SECS);

        let author_batches: Vec<Vec<nostr_sdk::PublicKey>> = authors
            .chunks(FOLLOW_LIST_AUTHORS_PER_BATCH)
            .map(|chunk| chunk.to_vec())
            .collect();
        let total_batches = author_batches.len();

        info!(
            "Fetching follow lists in {} batch(es) ({} authors per batch)",
            total_batches, FOLLOW_LIST_AUTHORS_PER_BATCH
        );

        let mut pending_batches = author_batches.into_iter().enumerate();
        let mut in_flight = FuturesUnordered::new();

        for _ in 0..MAX_CONCURRENT_FOLLOW_LIST_BATCHES {
            if let Some((batch_idx, author_batch)) = pending_batches.next() {
                let follow_list_filter = Filter::new()
                    .kind(Kind::Custom(KIND_FOLLOW_LIST))
                    .authors(author_batch);

                in_flight.push(Self::fetch_follow_list_batch(
                    nostr_client,
                    timeout_duration,
                    batch_idx,
                    follow_list_filter,
                ));
            }
        }

        while let Some((batch_idx, fetch_result)) = in_flight.next().await {
            match fetch_result {
                Ok(Ok(events)) => {
                    debug!(
                        "Follow-list batch {}/{}: {} events",
                        batch_idx + 1,
                        total_batches,
                        events.len()
                    );

                    for event in events {
                        let author = event.pubkey.to_hex();
                        match follow_lists.get(&author) {
                            Some(existing) => {
                                if Self::should_replace_follow_list(existing, &event) {
                                    follow_lists.insert(author, event);
                                }
                            }
                            None => {
                                follow_lists.insert(author, event);
                            }
                        }

                        // Update progress from unique authors fetched so far.
                        let fetched = follow_lists.len();
                        let coverage_percent = ((fetched * 100) / total_first) as u8;

                        let mut state = self
                            .discovery_state
                            .lock()
                            .expect("discovery_state mutex poisoned");
                        *state = DiscoveryState::FetchingFollowLists {
                            fetched,
                            total: total_first,
                            coverage_percent,
                        };

                        if fetched % 10 == 0 || fetched == total_first {
                            info!(
                                "Follow list progress: {}/{} ({}%)",
                                fetched, total_first, coverage_percent
                            );
                        }
                    }
                }
                Ok(Err(e)) => warn!(
                    "Error fetching follow lists in batch {}/{}: {}",
                    batch_idx + 1,
                    total_batches,
                    e
                ),
                Err(_) => warn!(
                    "Timeout fetching follow lists in batch {}/{}",
                    batch_idx + 1,
                    total_batches
                ),
            }

            if let Some((next_batch_idx, next_author_batch)) = pending_batches.next() {
                let follow_list_filter = Filter::new()
                    .kind(Kind::Custom(KIND_FOLLOW_LIST))
                    .authors(next_author_batch);

                in_flight.push(Self::fetch_follow_list_batch(
                    nostr_client,
                    timeout_duration,
                    next_batch_idx,
                    follow_list_filter,
                ));
            }
        }

        info!(
            "Follow list fetching complete: {}/{} authors ({:.1}%)",
            follow_lists.len(),
            total_first,
            (follow_lists.len() as f64 / total_first as f64) * 100.0
        );

        // Build social graph from fetched follow lists
        let (second_degree_counts, relay_hints) = self
            .build_social_graph(
                &follow_lists,
                &my_pubkey,
                &first_degree_set,
                allow_insecure_public_ws,
            )
            .await?;

        info!(
            "Built social graph: {} unique 2nd-degree follows",
            second_degree_counts.len()
        );

        // Filter to qualified pubkeys
        let qualified: HashSet<String> = second_degree_counts
            .iter()
            .filter(|(pubkey, count)| {
                **count >= QUALIFYING_THRESHOLD
                    && **pubkey != my_pubkey
                    && !first_degree_set.contains(*pubkey)
            })
            .map(|(pubkey, _)| pubkey.clone())
            .collect();

        info!(
            "Qualified {} pubkeys (threshold >= {})",
            qualified.len(),
            QUALIFYING_THRESHOLD
        );

        if qualified.is_empty() {
            let stats = NetworkStats {
                first_degree_count: total_first,
                total_second_degree: second_degree_counts.len(),
                qualified_count: 0,
                relays_covered: 0,
                computed_at: std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs())
                    .unwrap_or(0),
            };

            *self
                .discovery_state
                .lock()
                .expect("discovery_state mutex poisoned") = DiscoveryState::Complete {
                stats: stats.clone(),
            };

            return Ok(stats);
        }

        // Fetch relay lists for qualified pubkeys
        let qualified_list: Vec<String> = qualified.iter().cloned().collect();
        self.fetch_relay_lists_for_pubkeys(nostr_client, relay_cache, &qualified_list)
            .await?;

        // Compute optimal relay set
        let qualified_hints: HashMap<String, Vec<String>> = relay_hints
            .into_iter()
            .filter(|(k, _)| qualified.contains(k))
            .collect();

        let relay_urls = self.compute_relay_set_cover(&qualified, &qualified_hints, relay_cache);

        info!(
            "Computed {} relays for extended network coverage",
            relay_urls.len()
        );

        let stats = NetworkStats {
            first_degree_count: total_first,
            total_second_degree: second_degree_counts.len(),
            qualified_count: qualified.len(),
            relays_covered: relay_urls.len(),
            computed_at: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0),
        };

        // Save to cache
        let cache = ExtendedNetworkCache {
            qualified_pubkeys: qualified,
            first_degree_pubkeys: first_degree_set,
            relay_urls: relay_urls.clone(),
            relay_hints: qualified_hints,
            stats: stats.clone(),
        };
        *self
            .cached_network
            .lock()
            .expect("cached_network mutex poisoned") = Some(cache);

        *self
            .discovery_state
            .lock()
            .expect("discovery_state mutex poisoned") = DiscoveryState::Complete {
            stats: stats.clone(),
        };

        info!(
            "Discovery complete: {}/{} follow lists, {} qualified, {} relays",
            follow_lists.len(),
            total_first,
            stats.qualified_count,
            relay_urls.len()
        );

        Ok(stats)
    }

    async fn build_social_graph(
        &self,
        follow_lists: &HashMap<String, Event>,
        my_pubkey: &str,
        first_degree_set: &HashSet<String>,
        allow_insecure_public_ws: bool,
    ) -> Result<(HashMap<String, usize>, HashMap<String, Vec<String>>), ExtendedNetworkError> {
        let mut second_degree_counts: HashMap<String, usize> = HashMap::new();
        let mut relay_hints: HashMap<String, Vec<String>> = HashMap::new();
        let mut batch: Vec<(String, String)> = Vec::new();

        let total = follow_lists.len();
        let mut processed = 0;

        for (follower_pubkey, event) in follow_lists {
            // Parse follow list entries
            for tag in event.tags.iter() {
                let tag_vec: Vec<String> =
                    tag.clone().to_vec().iter().map(|s| s.to_string()).collect();

                if tag_vec.len() >= 2 && tag_vec[0] == "p" {
                    let target_pubkey = &tag_vec[1];

                    // Record followed-by relationship
                    batch.push((target_pubkey.clone(), follower_pubkey.clone()));

                    // Count 2nd-degree (exclude self and 1st-degree)
                    if target_pubkey != my_pubkey && !first_degree_set.contains(target_pubkey) {
                        *second_degree_counts
                            .entry(target_pubkey.clone())
                            .or_insert(0) += 1;

                        // Extract relay hint from p-tag if present
                        if tag_vec.len() >= 3 {
                            let hint = &tag_vec[2];
                            if let Some(normalized_hint) =
                                Self::normalize_and_admit_relay_url(hint, allow_insecure_public_ws)
                            {
                                relay_hints
                                    .entry(target_pubkey.clone())
                                    .or_default()
                                    .push(normalized_hint);
                            }
                        }
                    }
                }
            }

            processed += 1;
            if batch.len() >= 5000 {
                self.social_graph
                    .insert_batch(&batch)
                    .map_err(|e| ExtendedNetworkError::Database(e.to_string()))?;
                batch.clear();
            }

            // Update state periodically
            if processed % 100 == 0 {
                let mut state = self
                    .discovery_state
                    .lock()
                    .expect("discovery_state mutex poisoned");
                *state = DiscoveryState::BuildingGraph { processed, total };
            }
        }

        // Insert remaining batch
        if !batch.is_empty() {
            self.social_graph
                .insert_batch(&batch)
                .map_err(|e| ExtendedNetworkError::Database(e.to_string()))?;
        }

        *self
            .discovery_state
            .lock()
            .expect("discovery_state mutex poisoned") = DiscoveryState::BuildingGraph {
            processed: total,
            total,
        };

        Ok((second_degree_counts, relay_hints))
    }

    async fn fetch_follow_list_batch(
        nostr_client: &NostrClient,
        timeout_duration: Duration,
        batch_idx: usize,
        follow_list_filter: Filter,
    ) -> (
        usize,
        Result<Result<Vec<Event>, crate::nostr::NostrError>, tokio::time::error::Elapsed>,
    ) {
        (
            batch_idx,
            timeout(
                timeout_duration,
                nostr_client.fetch_from_connected_best_effort(follow_list_filter),
            )
            .await,
        )
    }

    fn should_replace_follow_list(existing: &Event, candidate: &Event) -> bool {
        let existing_ts = existing.created_at.as_secs();
        let candidate_ts = candidate.created_at.as_secs();

        if candidate_ts != existing_ts {
            return candidate_ts > existing_ts;
        }

        // Deterministic tie-breaker for equal timestamps.
        candidate.id.to_hex() > existing.id.to_hex()
    }

    async fn fetch_relay_lists_for_pubkeys(
        &self,
        nostr_client: &NostrClient,
        relay_cache: &RelayCache,
        pubkeys: &[String],
    ) -> Result<(), ExtendedNetworkError> {
        // Find pubkeys missing from cache
        let missing: Vec<String> = pubkeys
            .iter()
            .filter(|p| relay_cache.get_relay_list(p).is_none())
            .cloned()
            .collect();

        if missing.is_empty() {
            return Ok(());
        }

        // Fetch in chunks of 500
        let chunk_size = 500;
        let chunks: Vec<Vec<String>> = missing.chunks(chunk_size).map(|c| c.to_vec()).collect();

        let mut total_fetched = 0;
        let total = missing.len();

        for (i, chunk) in chunks.iter().enumerate() {
            let authors: Vec<nostr_sdk::PublicKey> = chunk
                .iter()
                .filter_map(|p| nostr_sdk::PublicKey::from_hex(p).ok())
                .collect();

            if authors.is_empty() {
                continue;
            }

            let filter = Filter::new()
                .kind(Kind::Custom(KIND_RELAY_LIST))
                .authors(authors);

            let timeout_duration = Duration::from_secs(RELAY_LIST_TIMEOUT_SECS);

            match timeout(
                timeout_duration,
                nostr_client.fetch_from_connected_best_effort(filter),
            )
            .await
            {
                Ok(Ok(events)) => {
                    for event in events {
                        // Parse and cache relay list
                        if let Ok(relay_list) = parse_relay_list_from_event(&event) {
                            let _ = relay_cache.save_relay_list(&relay_list);
                            total_fetched += 1;
                        }
                    }
                }
                Ok(Err(e)) => {
                    warn!("Error fetching relay list chunk {}: {}", i, e);
                }
                Err(_) => {
                    warn!("Timeout fetching relay list chunk {}", i);
                }
            }

            *self
                .discovery_state
                .lock()
                .expect("discovery_state mutex poisoned") = DiscoveryState::FetchingRelayLists {
                fetched: total_fetched,
                total,
            };
        }

        Ok(())
    }

    /// Greedy set-cover algorithm: pick relay covering most uncovered qualified pubkeys, repeat
    fn compute_relay_set_cover(
        &self,
        qualified: &HashSet<String>,
        relay_hints: &HashMap<String, Vec<String>>,
        relay_cache: &RelayCache,
    ) -> Vec<String> {
        let allow_insecure_public_ws = self.get_allow_insecure_public_ws();
        let mut relay_to_authors: HashMap<String, HashSet<String>> = HashMap::new();
        let mut from_relay_lists = 0usize;
        let mut from_hints = 0usize;

        // Build relay -> authors mapping
        for pubkey in qualified {
            if let Some(cached) = relay_cache.get_relay_list(pubkey) {
                // From NIP-65 relay list
                from_relay_lists += 1;
                for relay in &cached.write_relays {
                    if let Some(normalized_relay) =
                        Self::normalize_and_admit_relay_url(relay, allow_insecure_public_ws)
                    {
                        relay_to_authors
                            .entry(normalized_relay)
                            .or_default()
                            .insert(pubkey.clone());
                    }
                }
            } else if let Some(hints) = relay_hints.get(pubkey) {
                // From relay hints
                from_hints += 1;
                for hint in hints {
                    if let Some(normalized_hint) =
                        Self::normalize_and_admit_relay_url(hint, allow_insecure_public_ws)
                    {
                        relay_to_authors
                            .entry(normalized_hint)
                            .or_default()
                            .insert(pubkey.clone());
                    }
                }
            }
        }

        debug!(
            "Set-cover input: {} from relay lists, {} from hints, {} unique relays available",
            from_relay_lists,
            from_hints,
            relay_to_authors.len()
        );

        if relay_to_authors.is_empty() {
            debug!("Set-cover: No relay data available, returning empty set");
            return Vec::new();
        }

        let mut uncovered = qualified.clone();
        let mut selected: Vec<String> = Vec::new();
        let mut remaining = relay_to_authors.clone();

        debug!(
            "Set-cover: Starting with {} qualified pubkeys to cover",
            uncovered.len()
        );

        while !uncovered.is_empty() && selected.len() < MAX_EXTENDED_RELAYS && !remaining.is_empty()
        {
            // Find relay covering most uncovered pubkeys
            let mut best_url: Option<String> = None;
            let mut best_cover_size = 0usize;

            for (url, authors) in &remaining {
                let cover_size = authors.iter().filter(|a| uncovered.contains(*a)).count();
                if cover_size > best_cover_size {
                    best_url = Some(url.clone());
                    best_cover_size = cover_size;
                }
            }

            if best_url.is_none() || best_cover_size == 0 {
                break;
            }

            let url = best_url.unwrap();
            selected.push(url.clone());

            // Remove covered pubkeys (with cap per relay)
            let covered: Vec<String> = remaining[&url]
                .iter()
                .filter(|a| uncovered.contains(*a))
                .take(MAX_AUTHORS_PER_RELAY)
                .cloned()
                .collect();

            for pubkey in covered {
                uncovered.remove(&pubkey);
            }

            remaining.remove(&url);
        }

        let covered_count = qualified.len() - uncovered.len();
        debug!(
            "Set-cover complete: {} relays selected, covering {}/{} pubkeys ({}% coverage)",
            selected.len(),
            covered_count,
            qualified.len(),
            if !qualified.is_empty() {
                (covered_count * 100) / qualified.len()
            } else {
                0
            }
        );

        info!(
            "Set-cover: {} relays cover {}/{} pubkeys",
            selected.len(),
            covered_count,
            qualified.len()
        );

        selected
    }

    fn normalize_and_admit_relay_url(url: &str, allow_insecure_public_ws: bool) -> Option<String> {
        let normalized = Self::normalize_relay_url(url)?;
        let is_ws = normalized.starts_with("ws://");
        let host = Self::extract_relay_host(&normalized)?;

        if !is_ws {
            return Some(normalized);
        }

        if Self::is_local_or_private_host(&host) || allow_insecure_public_ws {
            return Some(normalized);
        }

        None
    }

    fn normalize_relay_url(url: &str) -> Option<String> {
        let trimmed = url.trim().trim_end_matches('/');
        if trimmed.is_empty() {
            return None;
        }

        let lowered = trimmed.to_ascii_lowercase();
        if lowered.starts_with("ws://") || lowered.starts_with("wss://") {
            return Some(lowered);
        }

        None
    }

    fn extract_relay_host(normalized_url: &str) -> Option<String> {
        let rest = normalized_url
            .strip_prefix("wss://")
            .or_else(|| normalized_url.strip_prefix("ws://"))?;

        let authority_end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
        let authority = &rest[..authority_end];
        if authority.is_empty() {
            return None;
        }

        if let Some(stripped) = authority.strip_prefix('[') {
            let end = stripped.find(']')?;
            return Some(stripped[..end].to_string());
        }

        Some(authority.split(':').next().unwrap_or_default().to_string())
    }

    fn is_local_or_private_host(host: &str) -> bool {
        if host == "localhost" || host == "::1" || host == "127.0.0.1" || host == "0.0.0.0" {
            return true;
        }

        if host.ends_with(".local") {
            return true;
        }

        Self::is_private_ipv4(host)
    }

    fn is_private_ipv4(host: &str) -> bool {
        let parts: Vec<&str> = host.split('.').collect();
        if parts.len() != 4 {
            return false;
        }

        let octets: Vec<u8> = parts
            .iter()
            .filter_map(|part| part.parse::<u8>().ok())
            .collect();

        if octets.len() != 4 {
            return false;
        }

        match octets.as_slice() {
            [10, _, _, _] => true,
            [127, _, _, _] => true,
            [192, 168, _, _] => true,
            [169, 254, _, _] => true,
            [172, second, _, _] if (16..=31).contains(second) => true,
            _ => false,
        }
    }

    /// Get relay configurations for extended network (read-only)
    pub fn get_relay_configs(&self) -> Vec<String> {
        self.cached_network
            .lock()
            .expect("cached_network mutex poisoned")
            .as_ref()
            .map(|c| c.relay_urls.clone())
            .unwrap_or_default()
    }

    /// Get followers who follow a specific pubkey (from social graph)
    pub fn get_followed_by(&self, pubkey: &str) -> Vec<String> {
        self.social_graph.get_followers(pubkey).unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use tempfile::TempDir;

    fn create_test_repo() -> (ExtendedNetworkRepository, Arc<SocialGraphDb>, PathBuf) {
        let temp = TempDir::new().unwrap();
        let db_path = temp.path().join("social.db");
        let social_graph = Arc::new(SocialGraphDb::new(&db_path).unwrap());
        let temp_path = temp.keep();
        let repo = ExtendedNetworkRepository::new(social_graph.clone());
        (repo, social_graph, temp_path)
    }

    #[test]
    fn test_compute_set_cover_basic() {
        let (mut repo, _, _tmp_path) = create_test_repo();
        repo.set_pubkey("me".to_string());

        let mut qualified: HashSet<String> = HashSet::new();
        qualified.insert("a".to_string());
        qualified.insert("b".to_string());
        qualified.insert("c".to_string());

        let mut hints: HashMap<String, Vec<String>> = HashMap::new();
        hints.insert(
            "a".to_string(),
            vec![
                "wss://relay1.com".to_string(),
                "wss://relay2.com".to_string(),
            ],
        );
        hints.insert("b".to_string(), vec!["wss://relay1.com".to_string()]);
        hints.insert("c".to_string(), vec!["wss://relay2.com".to_string()]);

        // Create mock relay_cache that returns None for all
        // For this test, we rely on hints only
        let temp = TempDir::new().unwrap();
        let relay_cache = RelayCache::new(temp.path().join("relay.db")).unwrap();

        let result = repo.compute_relay_set_cover(&qualified, &hints, &relay_cache);

        // relay1 covers a and b, relay2 covers a and c
        // Greedy picks relay1 first (covers 2), then relay2 (covers c)
        assert_eq!(result.len(), 2);
        assert!(result.contains(&"wss://relay1.com".to_string()));
        assert!(result.contains(&"wss://relay2.com".to_string()));
    }

    #[test]
    fn test_qualifying_threshold() {
        let (mut repo, social_graph, _tmp_path) = create_test_repo();
        repo.set_pubkey("me".to_string());

        // Create follow relationships
        // pubkey_x is followed by 15 first-degree follows (qualifies)
        // pubkey_y is followed by 5 first-degree follows (doesn't qualify)
        let mut pairs = Vec::new();
        for i in 0..15 {
            pairs.push(("pubkey_x".to_string(), format!("follower_{}", i)));
        }
        for i in 0..5 {
            pairs.push(("pubkey_y".to_string(), format!("follower_{}", i)));
        }

        social_graph.insert_batch(&pairs).unwrap();

        let counts = social_graph
            .count_followers(&["pubkey_x".to_string(), "pubkey_y".to_string()])
            .unwrap();

        assert_eq!(counts.get("pubkey_x"), Some(&15));
        assert_eq!(counts.get("pubkey_y"), Some(&5));

        // Only pubkey_x qualifies
        let qualified: Vec<String> = counts
            .iter()
            .filter(|(_, count)| **count >= QUALIFYING_THRESHOLD as i32)
            .map(|(k, _)| k.clone())
            .collect();

        assert_eq!(qualified.len(), 1);
        assert!(qualified.contains(&"pubkey_x".to_string()));
    }

    #[test]
    fn test_relay_url_normalization_and_dedupe_shape() {
        let a = ExtendedNetworkRepository::normalize_and_admit_relay_url(
            "  WSS://Relay.Example.COM/  ",
            false,
        );
        let b = ExtendedNetworkRepository::normalize_and_admit_relay_url(
            "wss://relay.example.com",
            false,
        );

        assert_eq!(a, Some("wss://relay.example.com".to_string()));
        assert_eq!(a, b);
    }

    #[test]
    fn test_public_ws_relay_policy_and_local_override() {
        let public_ws = "ws://relay.example.com";
        let local_ws = "ws://localhost:4848";

        // Public insecure relay is blocked by default.
        assert!(
            ExtendedNetworkRepository::normalize_and_admit_relay_url(public_ws, false).is_none()
        );

        // Public insecure relay allowed when policy enabled.
        assert_eq!(
            ExtendedNetworkRepository::normalize_and_admit_relay_url(public_ws, true),
            Some(public_ws.to_string())
        );

        // Local/dev insecure relay is always allowed.
        assert_eq!(
            ExtendedNetworkRepository::normalize_and_admit_relay_url(local_ws, false),
            Some(local_ws.to_string())
        );
    }
}
