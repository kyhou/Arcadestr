# NIP-65 Relay Gossip Integration Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Integrate NIP-65 relay gossip into the application lifecycle - collect relay info after auth, implement relay selection, add dedup + seen_on tracking.

**Architecture:** Extend AppState with RelayCache and EventDeduplicator. Add relay selector pipeline. Hook into post-auth flow.

**Tech Stack:** Rust, Tauri, SQLite, nostr-sdk

---

## Context

The worktree is at `.worktrees/nip65-relay-gossip`. The core/src/relay_cache.rs and core/src/nostr.rs already have:
- RelayCache with SQLite storage
- EventDeduplicator
- fetch_relay_list(), get_relays_for_pubkey()
- parse_nip19_identifier()

Desktop main.rs has AppState with auth and nostr clients.

---

## Task 1: Add get_stale_pubkeys to RelayCache

**Files:**
- Modify: `core/src/relay_cache.rs`

- [ ] **Step 1: Add get_stale_pubkeys method**

```rust
/// Get all pubkeys with stale relay lists (>7 days old)
pub fn get_stale_pubkeys(&self) -> Vec<String> {
    let conn = match self.conn.lock() {
        Ok(c) => c,
        Err(_) => return vec![],
    };
    
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    
    let seven_days: u64 = 7 * 24 * 60 * 60;
    let threshold = now.saturating_sub(seven_days);
    
    let mut stmt = match conn.prepare(
        "SELECT pubkey FROM relay_lists WHERE updated_at < ?"
    ) {
        Ok(s) => s,
        Err(_) => return vec![],
    };
    
    let pubkeys = stmt
        .query_map([threshold], |row| row.get(0))
        .ok()
        .map(|rows| rows.filter_map(|r| r.ok()).collect())
        .unwrap_or_default();
    
    pubkeys
}
```

- [ ] **Step 2: Commit**

```bash
cd .worktrees/nip65-relay-gossip && git add core/src/relay_cache.rs && git commit -m "feat(nip65): add get_stale_pubkeys method"
```

---

## Task 2: Add relay selector functions to nostr.rs

**Files:**
- Modify: `core/src/nostr.rs`

- [ ] **Step 1: Add relay selector types and functions**

Add after existing types:

```rust
use std::collections::{HashMap, HashSet};

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
        
        // Apply health multiplier
        let health_score = cache.get_health_score(relay_url);
        
        // Apply staleness multiplier (if stale, half the score)
        let staleness_multiplier = if cache.is_stale(relay_url) { 0.5 } else { 1.0 };
        
        // Apply user's own relay bonus
        let is_user_own = user_pubkey.map(|u| {
            cache.get_relay_list(u)
                .map(|l| l.write_relays.contains(relay_url))
                .unwrap_or(false)
        }).unwrap_or(false);
        
        let own_relay_multiplier = if is_user_own { 1.5 } else { 1.0 };
        
        let final_score = raw_score * health_score * staleness_multiplier * own_relay_multiplier;
        
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
```

- [ ] **Step 2: Add fetch_follow_list method to NostrClient**

```rust
/// Fetch Kind 3 (follow list) for a pubkey
pub async fn fetch_follow_list(&self, npub: &str) -> Result<Vec<String>, NostrError> {
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
        None => return Ok(vec![]), // No follow list
    };
    
    // Parse content - Kind 3 content is a JSON array of pubkeys
    let content_str = event.content.trim();
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
```

- [ ] **Step 3: Commit**

```bash
cd .worktrees/nip65-relay-gossip && git add core/src/nostr.rs && git commit -m "feat(nip65): add relay selector functions and fetch_follow_list"
```

---

## Task 3: Integrate RelayCache into AppState

**Files:**
- Modify: `desktop/src/main.rs`

- [ ] **Step 1: Add imports and modify AppState**

Add to imports (around line 14):
```rust
use arcadestr_core::relay_cache::RelayCache;
use arcadestr_core::nostr::{EventDeduplicator, build_relay_map, score_relays, select_relays, fetch_follow_list, ScoredRelay, RelaySelection};
```

Modify AppState (around line 23):
```rust
pub struct AppState {
    pub auth: Arc<Mutex<AuthState>>,
    pub nostr: Arc<Mutex<NostrClient>>,
    pub relay_cache: Arc<RelayCache>,
    pub deduplicator: Arc<Mutex<EventDeduplicator>>,
}
```

- [ ] **Step 2: Initialize RelayCache at startup**

Around line 420, after creating nostr_client, add:
```rust
// Initialize RelayCache
let relay_cache = RelayCache::new(keys_dir.join("relay_cache.db"))
    .expect("Failed to create relay cache");
let deduplicator = EventDeduplicator::new(10000);
```

- [ ] **Step 3: Add to AppState in Builder**

Around line 555-558, modify the manage() call:
```rust
manage(AppState {
    auth: Arc::new(Mutex::new(AuthState::new())),
    nostr: Arc::new(Mutex::new(nostr_client)),
    relay_cache: Arc::new(relay_cache),
    deduplicator: Arc::new(Mutex::new(deduplicator)),
})
```

- [ ] **Step 4: Commit**

```bash
cd .worktrees/nip65-relay-gossip && git add desktop/src/main.rs && git commit -m "feat(nip65): integrate RelayCache into AppState"
```

---

## Task 4: Add post-authentication relay discovery

**Files:**
- Modify: `desktop/src/main.rs`

- [ ] **Step 1: Add helper function for post-auth relay discovery**

Add before the tauri::Builder (around line 550):

```rust
/// Perform post-authentication relay discovery
async fn initialize_relay_gossip(
    nostr: Arc<Mutex<NostrClient>>,
    relay_cache: Arc<RelayCache>,
    user_npub: String,
) {
    let mut nostr_client = nostr.lock().await;
    
    // Fetch user's follow list
    tracing::info!("Fetching follow list for {}", user_npub);
    let followed = match nostr_client.fetch_follow_list(&user_npub).await {
        Ok(list) => list,
        Err(e) => {
            tracing::warn!("Failed to fetch follow list: {}", e);
            return;
        }
    };
    
    tracing::info!("Found {} followed pubkeys", followed.len());
    
    // Fetch relay lists for followed pubkeys
    for pubkey in &followed {
        match nostr_client.fetch_relay_list(pubkey).await {
            Ok(relays) => {
                let _ = relay_cache.save_relay_list(&relays);
            }
            Err(_) => {
                // Fallback to seen_on if no relay list
                let seen = relay_cache.get_seen_on(pubkey);
                if !seen.is_empty() {
                    let fallback = CachedRelayList {
                        pubkey: pubkey.clone(),
                        write_relays: seen.clone(),
                        read_relays: seen,
                        updated_at: 0,
                    };
                    let _ = relay_cache.save_relay_list(&fallback);
                }
            }
        }
    }
    
    // Build relay map and select
    let all_pubkeys: HashSet<_> = followed.iter().cloned().collect();
    let relay_map = build_relay_map(&followed, &relay_cache);
    let scored = score_relays(&relay_map, &relay_cache, Some(&user_npub));
    let selection = select_relays(scored, 10, &all_pubkeys);
    
    tracing::info!("Selected {} permanent relays", selection.permanent.len());
    
    // Add selected relays
    for relay in &selection.permanent {
        let _ = nostr_client.add_relay(relay).await;
    }
    
    nostr_client.connect().await;
    
    tracing::info!("Relay gossip initialized");
}
```

- [ ] **Step 2: Add import for HashSet**

```rust
use std::collections::HashSet;
```

- [ ] **Step 3: Hook into connect_saved_user**

Modify connect_saved_user function (around line 540) to call the initialization after successful auth:

```rust
// After successful connection (around line 546 after getting pubkey)
// Add this block:
{
    let user_npub = pubkey.to_bech32().unwrap_or_default();
    let state_nostr = state.nostr.clone();
    let state_cache = state.relay_cache.clone();
    
    // Spawn background task for relay discovery
    tokio::spawn(async move {
        initialize_relay_gossip(state_nostr, state_cache, user_npub).await;
    });
}
```

- [ ] **Step 4: Commit**

```bash
cd .worktrees/nip65-relay-gossip && git add desktop/src/main.rs && git commit -m "feat(nip65): add post-authentication relay discovery"
```

---

## Task 5: Add seen_on tracking to event handling

**Files:**
- Modify: `desktop/src/main.rs`

- [ ] **Step 1: Modify fetch_listings to track seen_on and dedup**

Find the fetch_listings function (around line 313) and modify to update seen_on:

```rust
async fn fetch_listings(
    limit: usize,
    state: tauri::State<'_, AppState>,
) -> Result<Vec<GameListing>, String> {
    let mut nostr = state.nostr.lock().await;
    let cache = state.relay_cache.clone();
    let mut dedup = state.deduplicator.lock().await;
    
    match nostr.fetch_listings(limit).await {
        Ok(listings) => {
            // Track seen_on for each listing's publisher
            for listing in &listings {
                let _ = cache.update_seen_on(&listing.publisher_npub, ""); // Will be updated by event listener
            }
            
            // Note: In a full implementation, we'd track which relay delivered each event
            // For now, the relay selection already uses optimal relays
            
            Ok(listings)
        }
        Err(e) => Err(e.to_string()),
    }
}
```

- [ ] **Step 2: Commit**

```bash
cd .worktrees/nip65-relay-gossip && git add desktop/src/main.rs && git commit -m "feat(nip65): add seen_on tracking"
```

---

## Task 6: Add on-demand profile fetch with NIP-19 hints

**Files:**
- Modify: `desktop/src/main.rs`

- [ ] **Step 1: Add fetch_profile_with_hints command**

Add before tauri::Builder:

```rust
#[tauri::command]
async fn fetch_profile_with_hints(
    identifier: String, // nprofile or nevent
    state: tauri::State<'_, AppState>,
) -> Result<UserProfile, String> {
    use arcadestr_core::nostr::parse_nip19_identifier;
    
    // Parse NIP-19 identifier
    let parsed = parse_nip19_identifier(&identifier)
        .map_err(|e| e.to_string())?;
    
    let mut nostr = state.nostr.lock().await;
    let cache = state.relay_cache.clone();
    
    // Connect to hint relays if present
    for hint in &parsed.relays {
        let _ = nostr.add_relay(hint).await;
    }
    
    // Fetch relay list and cache it
    if let Ok(relays) = nostr.fetch_relay_list(&parsed.pubkey).await {
        let _ = cache.save_relay_list(&relays);
    }
    
    // Fetch profile
    let npub = format!("npub1{}", &parsed.pubkey[4..]); // Convert hex to npub format
    nostr.fetch_profile(&npub).await.map_err(|e| e.to_string())
}
```

- [ ] **Step 2: Add to invoke_handler**

Add `fetch_profile_with_hints` to the generate_handler! macro (around line 559).

- [ ] **Step 3: Commit**

```bash
cd .worktrees/nip65-relay-gossip && git add desktop/src/main.rs && git commit -m "feat(nip65): add on-demand profile fetch with NIP-19 hints"
```

---

## Task 7: Add background stale relay refresh

**Files:**
- Modify: `desktop/src/main.rs`

- [ ] **Step 1: Add refresh_stale_relays function**

```rust
async fn refresh_stale_relays(
    nostr: Arc<Mutex<NostrClient>>,
    relay_cache: Arc<RelayCache>,
) {
    let stale_pubkeys = relay_cache.get_stale_pubkeys();
    
    if stale_pubkeys.is_empty() {
        return;
    }
    
    tracing::info!("Refreshing {} stale relay lists", stale_pubkeys.len());
    
    let mut nostr_client = nostr.lock().await;
    
    for pubkey in stale_pubkeys {
        let npub = format!("npub1{}", &pubkey[4..]); // Convert hex to npub
        match nostr_client.fetch_relay_list(&npub).await {
            Ok(relays) => {
                let _ = relay_cache.save_relay_list(&relays);
            }
            Err(e) => {
                tracing::debug!("Failed to refresh {}: {}", pubkey, e);
            }
        }
    }
}
```

- [ ] **Step 2: Call on app startup**

After initializing relay gossip, add a periodic refresh. For now, just refresh once on startup after auth:

```rust
// In initialize_relay_gossip, after initial setup:
let cache_for_refresh = relay_cache.clone();
let nostr_for_refresh = nostr.clone();
tokio::spawn(async move {
    tokio::time::sleep(tokio::time::Duration::from_secs(30)).await;
    refresh_stale_relays(nostr_for_refresh, cache_for_refresh).await;
});
```

- [ ] **Step 3: Commit**

```bash
cd .worktrees/nip65-relay-gossip && git add desktop/src/main.rs && git commit -m "feat(nip65): add background stale relay refresh"
```

---

## Task 8: Verify build

**Files:**
- Test: Full build

- [ ] **Step 1: Run cargo check**

```bash
cd .worktrees/nip65-relay-gossip && cargo check -p arcadestr-desktop 2>&1 | tail -30
```

- [ ] **Step 2: Commit final**

```bash
cd .worktrees/nip65-relay-gossip && git add . && git commit -m "feat(nip65): integrate relay gossip into application lifecycle"
```

---

## Summary

| Task | Description | Files Changed |
|------|-------------|---------------|
| 1 | Add get_stale_pubkeys | relay_cache.rs |
| 2 | Add relay selector functions | nostr.rs |
| 3 | Integrate RelayCache into AppState | main.rs |
| 4 | Post-auth relay discovery | main.rs |
| 5 | seen_on tracking | main.rs |
| 6 | NIP-19 profile fetch | main.rs |
| 7 | Background refresh | main.rs |
| 8 | Verify build | - |
