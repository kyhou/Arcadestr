# Profile System Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fix Arcadestr's broken profile system by implementing YakiHonne's proven multi-layer caching strategy

**Architecture:** 
1. Add SQLite `users` table for persistent storage of ALL user profiles
2. Integrate `ProfileFetcher` batch results with SQLite persistence
3. Add global `NostrAuthors` signal to app layer for reactive profile access
4. Connect Tauri events to frontend profile store
5. Update UI components to display profile info instead of raw npubs

**Tech Stack:** Rust (core), Leptos (app), Tauri (desktop), SQLite, SQLx, tokio

---

## Phase 1: Persistent User Cache (Core Layer)

### Task 1: Create UserCache Module

**Files:**
- Create: `core/src/user_cache.rs`
- Modify: `core/src/storage/db.rs` - Add users table migration
- Modify: `core/src/lib.rs` - Export UserCache

**Objective:** Create a new module for persisting user profiles to SQLite, mirroring YakiHonne's Dexie `users` table functionality.

- [ ] **Step 1: Add users table to database migrations**

Run command to verify current migrations:
```bash
cd /home/joel/Sync/Projetos/Arcadestr/core && grep -n "CREATE TABLE" src/storage/db.rs
```

Add migration in `core/src/storage/db.rs` after existing tables:

```rust
// Migration 4: Add users table for profile caching
const MIGRATION_4_USERS_TABLE: &str = r#"
CREATE TABLE IF NOT EXISTS users (
    id INTEGER PRIMARY KEY,
    npub TEXT NOT NULL UNIQUE,
    name TEXT,
    display_name TEXT,
    picture TEXT,
    about TEXT,
    nip05 TEXT,
    lud16 TEXT,
    website TEXT,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    expires_at INTEGER NOT NULL
);

CREATE INDEX idx_users_npub ON users(npub);
CREATE INDEX idx_users_expires ON users(expires_at);
"#;
```

Update migrations list:
```rust
const MIGRATIONS: &[&str] = &[
    MIGRATION_1_INITIAL,
    MIGRATION_2_GAMES_TABLE,
    MIGRATION_3_RELAYS_TABLE,
    MIGRATION_4_USERS_TABLE, // Add this
];
```

- [ ] **Step 2: Create UserCache struct**

Create `core/src/user_cache.rs`:

```rust
//! User profile cache for persistent storage of fetched profiles.
//! Mirrors YakiHonne's Dexie users table functionality.

use std::time::{SystemTime, UNIX_EPOCH};
use sqlx::{Pool, Sqlite, Row};
use crate::nostr::UserProfile;

const DEFAULT_CACHE_TTL_SECONDS: i64 = 86400; // 24 hours, matching YakiHonne

pub struct UserCache {
    db: Pool<Sqlite>,
    ttl_seconds: i64,
}

impl UserCache {
    pub fn new(db: Pool<Sqlite>) -> Self {
        Self {
            db,
            ttl_seconds: DEFAULT_CACHE_TTL_SECONDS,
        }
    }

    /// Get a user profile from cache
    pub async fn get(&self, npub: &str) -> Option<UserProfile> {
        let now = Self::now();
        
        let row = sqlx::query(
            r#"
            SELECT npub, name, display_name, picture, about, 
                   nip05, lud16, website, created_at
            FROM users 
            WHERE npub = ? AND expires_at > ?
            "#
        )
        .bind(npub)
        .bind(now)
        .fetch_optional(&self.db)
        .await
        .ok()?;
        
        row.map(|r| UserProfile {
            npub: r.get("npub"),
            name: r.get("name"),
            display_name: r.get("display_name"),
            picture: r.get("picture"),
            about: r.get("about"),
            nip05: r.get("nip05"),
            lud16: r.get("lud16"),
            website: r.get("website"),
            nip05_verified: false, // Will be verified on fetch
        })
    }

    /// Save or update a user profile
    pub async fn put(&self, npub: &str, profile: &UserProfile) -> Result<(), sqlx::Error> {
        let now = Self::now();
        let expires = now + self.ttl_seconds;
        
        sqlx::query(
            r#"
            INSERT INTO users (npub, name, display_name, picture, about, 
                             nip05, lud16, website, created_at, updated_at, expires_at)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            ON CONFLICT(npub) DO UPDATE SET
                name = excluded.name,
                display_name = excluded.display_name,
                picture = excluded.picture,
                about = excluded.about,
                nip05 = excluded.nip05,
                lud16 = excluded.lud16,
                website = excluded.website,
                updated_at = excluded.updated_at,
                expires_at = excluded.expires_at
            "#
        )
        .bind(npub)
        .bind(&profile.name)
        .bind(&profile.display_name)
        .bind(&profile.picture)
        .bind(&profile.about)
        .bind(&profile.nip05)
        .bind(&profile.lud16)
        .bind(&profile.website)
        .bind(profile.created_at.unwrap_or(now))
        .bind(now)
        .bind(expires)
        .execute(&self.db)
        .await?;
        
        Ok(())
    }

    /// Save multiple profiles in a batch transaction
    pub async fn put_many(&self, profiles: &[(String, UserProfile)]) -> Result<(), sqlx::Error> {
        let mut tx = self.db.begin().await?;
        
        for (npub, profile) in profiles {
            let now = Self::now();
            let expires = now + self.ttl_seconds;
            
            sqlx::query(
                r#"
                INSERT INTO users (npub, name, display_name, picture, about, 
                                 nip05, lud16, website, created_at, updated_at, expires_at)
                VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
                ON CONFLICT(npub) DO UPDATE SET
                    name = excluded.name,
                    display_name = excluded.display_name,
                    picture = excluded.picture,
                    about = excluded.about,
                    nip05 = excluded.nip05,
                    lud16 = excluded.lud16,
                    website = excluded.website,
                    updated_at = excluded.updated_at,
                    expires_at = excluded.expires_at
                "#
            )
            .bind(npub)
            .bind(&profile.name)
            .bind(&profile.display_name)
            .bind(&profile.picture)
            .bind(&profile.about)
            .bind(&profile.nip05)
            .bind(&profile.lud16)
            .bind(&profile.website)
            .bind(profile.created_at.unwrap_or(now))
            .bind(now)
            .bind(expires)
            .execute(&mut *tx)
            .await?;
        }
        
        tx.commit().await?;
        Ok(())
    }

    /// Get all cached users
    pub async fn get_all(&self) -> Result<Vec<UserProfile>, sqlx::Error> {
        let now = Self::now();
        
        let rows = sqlx::query(
            r#"
            SELECT npub, name, display_name, picture, about, 
                   nip05, lud16, website, created_at
            FROM users 
            WHERE expires_at > ?
            ORDER BY updated_at DESC
            "#
        )
        .bind(now)
        .fetch_all(&self.db)
        .await?;
        
        Ok(rows.into_iter().map(|r| UserProfile {
            npub: r.get("npub"),
            name: r.get("name"),
            display_name: r.get("display_name"),
            picture: r.get("picture"),
            about: r.get("about"),
            nip05: r.get("nip05"),
            lud16: r.get("lud16"),
            website: r.get("website"),
            nip05_verified: false,
        }).collect())
    }

    /// Check if profile exists and is fresh
    pub async fn is_fresh(&self, npub: &str) -> bool {
        self.get(npub).await.is_some()
    }

    /// Delete expired profiles
    pub async fn cleanup_expired(&self) -> Result<u64, sqlx::Error> {
        let now = Self::now();
        
        let result = sqlx::query(
            "DELETE FROM users WHERE expires_at <= ?"
        )
        .bind(now)
        .execute(&self.db)
        .await?;
        
        Ok(result.rows_affected())
    }

    fn now() -> i64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    fn create_test_profile() -> UserProfile {
        UserProfile {
            npub: "npub1test123".to_string(),
            name: Some("testuser".to_string()),
            display_name: Some("Test User".to_string()),
            picture: Some("https://example.com/pic.jpg".to_string()),
            about: Some("Test bio".to_string()),
            nip05: Some("test@example.com".to_string()),
            lud16: None,
            website: Some("https://example.com".to_string()),
            nip05_verified: false,
            created_at: Some(1234567890),
        }
    }
    
    // Tests would need actual DB - integration tests in tests/ dir
}
```

- [ ] **Step 3: Export UserCache from lib.rs**

Modify `core/src/lib.rs`:

```rust
#[cfg(feature = "native")]
pub mod user_cache;

#[cfg(feature = "native")]
pub use user_cache::UserCache;
```

- [ ] **Step 4: Integrate UserCache with ProfileFetcher**

Modify `core/src/profile_fetcher.rs` to accept and use UserCache:

Add to imports:
```rust
use crate::user_cache::UserCache;
```

Modify ProfileFetcher struct:
```rust
pub struct ProfileFetcher {
    pending: Arc<Mutex<VecDeque<String>>>,
    in_flight: Arc<Mutex<HashSet<String>>>,
    failed_attempts: Arc<Mutex<HashMap<String, u32>>>,
    cache: Arc<dyn ProfileCache>,
    persistent_cache: Option<Arc<UserCache>>, // NEW
    max_attempts: u32,
    batch_size: usize,
}
```

Add constructor with UserCache:
```rust
impl ProfileFetcher {
    /// Create with persistent SQLite cache
    pub fn with_persistent_cache(user_cache: Arc<UserCache>) -> Self {
        let mut fetcher = Self::new();
        fetcher.persistent_cache = Some(user_cache);
        fetcher
    }
    
    /// Load cached profiles on startup
    pub async fn load_cached_profiles(&self) -> Vec<UserProfile> {
        if let Some(ref cache) = self.persistent_cache {
            match cache.get_all().await {
                Ok(profiles) => {
                    // Populate in-memory cache
                    for profile in &profiles {
                        self.cache.put(profile.npub.clone(), profile.clone());
                    }
                    return profiles;
                }
                Err(e) => {
                    tracing::warn!("Failed to load cached profiles: {}", e);
                }
            }
        }
        vec![]
    }
}
```

Modify `fetch_batch` to persist results:
```rust
pub async fn fetch_batch(&self, client: &NostrClient) -> (Vec<UserProfile>, usize) {
    // ... existing batch collection code ...
    
    match self.fetch_profiles_batch(client, &batch).await {
        Ok(profiles) => {
            // Save to both caches
            for (npub, profile) in &profiles {
                results.push(profile.clone());
                self.cache.put(npub.clone(), profile.clone());
            }
            
            // Persist to SQLite (NEW)
            if let Some(ref user_cache) = self.persistent_cache {
                if let Err(e) = user_cache.put_many(&profiles).await {
                    tracing::warn!("Failed to persist profiles: {}", e);
                }
            }
        }
        Err(e) => {
            // ... error handling ...
        }
    }
    
    // ... rest of method ...
}
```

Modify `fetch_single` to persist:
```rust
pub async fn fetch_single(&self, client: &NostrClient, npub: &str) -> Option<UserProfile> {
    // ... existing cache check ...
    
    // Fetch from relays ...
    
    if let Some(event) = events.first() {
        match Self::parse_profile_event(event, npub) {
            Ok(profile) => {
                self.cache.put(npub.to_string(), profile.clone());
                
                // Persist to SQLite (NEW)
                if let Some(ref user_cache) = self.persistent_cache {
                    if let Err(e) = user_cache.put(npub, &profile).await {
                        tracing::warn!("Failed to persist profile: {}", e);
                    }
                }
                
                return Some(profile);
            }
            // ... error handling ...
        }
    }
    
    None
}
```

- [ ] **Step 5: Update AppState to include UserCache**

Modify `desktop/src/main.rs` AppState struct:

```rust
struct AppState {
    nostr: Arc<Mutex<NostrClient>>,
    relay_cache: Arc<RelayCache>,
    auth: Arc<Mutex<AuthState>>,
    subscription_registry: Arc<SubscriptionRegistry>,
    profile_fetcher: Arc<ProfileFetcher>,
    user_cache: Arc<UserCache>, // NEW
}
```

Initialize UserCache in main():
```rust
let db_pool = sqlx::SqlitePool::connect(&format!("sqlite:{}", db_path)).await
    .expect("Failed to connect to database");

let user_cache = Arc::new(UserCache::new(db_pool));
let profile_fetcher = Arc::new(ProfileFetcher::with_persistent_cache(user_cache.clone()));

// Load cached profiles on startup
let _ = profile_fetcher.load_cached_profiles().await;
```

- [ ] **Step 6: Run tests and verify**

```bash
cd /home/joel/Sync/Projetos/Arcadestr
cargo test -p arcadestr-core --lib user_cache
cargo check -p arcadestr-core
cargo check -p arcadestr-desktop
```

---

## Phase 2: Frontend Profile Store (App Layer)

### Task 2: Create Global Profile Store

**Files:**
- Create: `app/src/store/profiles.rs`
- Modify: `app/src/lib.rs` - Add profile store initialization
- Modify: `app/src/lib.rs` - Add event listeners for `profile_fetched`

**Objective:** Create a reactive global store for user profiles, similar to YakiHonne's Redux `nostrAuthors`.

- [ ] **Step 1: Create profiles store module**

Create `app/src/store/profiles.rs`:

```rust
//! Global profile store for managing Nostr user profiles.
//! Mirrors YakiHonne's nostrAuthors Redux slice.

use leptos::prelude::*;
use std::collections::HashMap;
use crate::models::UserProfile;

/// Global profile store - reactive HashMap keyed by npub
#[derive(Clone, Debug)]
pub struct ProfileStore {
    profiles: RwSignal<HashMap<String, UserProfile>>,
}

impl ProfileStore {
    pub fn new() -> Self {
        Self {
            profiles: RwSignal::new(HashMap::new()),
        }
    }
    
    /// Get a profile by npub
    pub fn get(&self, npub: &str) -> Option<UserProfile> {
        self.profiles.get().get(npub).cloned()
    }
    
    /// Add or update a profile
    pub fn put(&self, profile: UserProfile) {
        self.profiles.update(|map| {
            map.insert(profile.npub.clone(), profile);
        });
    }
    
    /// Add multiple profiles
    pub fn put_many(&self, profiles: Vec<UserProfile>) {
        self.profiles.update(|map| {
            for profile in profiles {
                map.insert(profile.npub.clone(), profile);
            }
        });
    }
    
    /// Check if profile exists
    pub fn has(&self, npub: &str) -> bool {
        self.profiles.get().contains_key(npub)
    }
    
    /// Get all profiles as vec
    pub fn get_all(&self) -> Vec<UserProfile> {
        self.profiles.get().values().cloned().collect()
    }
    
    /// Get the signal for reactive access
    pub fn signal(&self) -> RwSignal<HashMap<String, UserProfile>> {
        self.profiles
    }
}

impl Default for ProfileStore {
    fn default() -> Self {
        Self::new()
    }
}

/// Context provider for ProfileStore
pub fn provide_profile_store() {
    provide_context(ProfileStore::new());
}

/// Hook to use the profile store
pub fn use_profile_store() -> ProfileStore {
    use_context::<ProfileStore>().expect("ProfileStore not provided")
}

/// Hook to get a specific profile - fetches if missing
pub fn use_profile(npub: String) -> Signal<Option<UserProfile>> {
    let store = use_profile_store();
    
    Signal::derive(move || {
        store.get(&npub)
    })
}
```

- [ ] **Step 2: Add event listener for profile_fetched events**

In `app/src/lib.rs`, add event listener setup:

```rust
/// Setup Tauri event listeners for profile updates
#[cfg(any(target_arch = "wasm32", not(feature = "web")))]
pub fn setup_profile_listeners(profile_store: ProfileStore) {
    use tauri::event::listen;
    use wasm_bindgen_futures::spawn_local;
    
    spawn_local(async move {
        // Listen for individual profile fetched events
        let _ = listen::<UserProfile>("profile_fetched", move |event| {
            let profile = event.payload;
            profile_store.put(profile);
        });
        
        // Listen for batch progress (optional - for UI progress bars)
        let _ = listen::<ProfileFetchProgress>("profile_fetch_progress", move |event| {
            let progress = event.payload;
            tracing::info!("Profile fetch progress: {}/{}", progress.completed, progress.total);
        });
    });
}

/// Profile fetch progress event
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ProfileFetchProgress {
    pub completed: usize,
    pub total: usize,
}
```

- [ ] **Step 3: Initialize store in main App component**

Modify `app/src/lib.rs` App component:

```rust
#[component]
pub fn App() -> impl IntoView {
    // ... existing initialization ...
    
    // Initialize profile store
    provide_profile_store();
    let profile_store = use_profile_store();
    
    // Setup event listeners
    #[cfg(any(target_arch = "wasm32", not(feature = "web")))]
    setup_profile_listeners(profile_store.clone());
    
    // ... rest of App component ...
}
```

- [ ] **Step 4: Create profile fetch helper**

In `app/src/lib.rs`, add:

```rust
/// Fetch a profile and store it
pub async fn fetch_and_store_profile(npub: String) -> Result<UserProfile, String> {
    let profile = invoke_fetch_profile(npub.clone(), None).await?;
    
    // Store in global cache
    if let Ok(store) = try_use_context::<ProfileStore>() {
        store.put(profile.clone());
    }
    
    Ok(profile)
}

/// Batch fetch profiles for multiple npubs
pub async fn fetch_missing_profiles(npubs: Vec<String>) -> Result<Vec<UserProfile>, String> {
    let store = try_use_context::<ProfileStore>();
    
    // Filter out already cached profiles
    let missing: Vec<String> = npubs.into_iter()
        .filter(|npub| {
            if let Some(ref s) = store {
                !s.has(npub)
            } else {
                true
            }
        })
        .collect();
    
    if missing.is_empty() {
        return Ok(vec![]);
    }
    
    // Use the batch fetch command (need to add this)
    let profiles = invoke_fetch_profiles_batch(missing).await?;
    
    // Store results
    if let Some(ref s) = store {
        s.put_many(profiles.clone());
    }
    
    Ok(profiles)
}
```

---

## Phase 3: Profile Integration (App Components)

### Task 3: Update ListingCard to Display Profile Info

**Files:**
- Modify: `app/src/components/browse.rs` - ListingCard component
- Modify: `app/src/components/detail.rs` - Detail view

**Objective:** Replace raw npub display with profile display name and avatar, like YakiHonne does everywhere.

- [ ] **Step 1: Create ProfileAvatar component**

Create `app/src/components/profile_avatar.rs`:

```rust
//! Profile avatar component - displays profile picture or fallback

use leptos::prelude::*;
use crate::store::profiles::{use_profile_store, use_profile};

#[component]
pub fn ProfileAvatar(
    npub: String,
    size: &'static str,
) -> impl IntoView {
    let profile = use_profile(npub.clone());
    
    view! {
        <div class="profile-avatar-container" style:width=size style:height=size>
            {move || {
                match profile.get() {
                    Some(p) => {
                        if let Some(pic) = p.picture {
                            view! {
                                <img 
                                    src=pic 
                                    class="profile-avatar-img" 
                                    alt="Profile"
                                    style:width="100%"
                                    style:height="100%"
                                    style:border-radius="50%"
                                    style:object-fit="cover"
                                />
                            }.into_any()
                        } else {
                            // Fallback: first letter of display name
                            let letter = p.display()
                                .chars()
                                .next()
                                .map(|c| c.to_uppercase().to_string())
                                .unwrap_or_else(|| "?".to_string());
                            
                            view! {
                                <div 
                                    class="profile-avatar-fallback"
                                    style:width="100%"
                                    style:height="100%"
                                    style:border-radius="50%"
                                    style:background="#444"
                                    style:display="flex"
                                    style:align-items="center"
                                    style:justify-content="center"
                                    style:font-size="1.2em"
                                    style:color="white"
                                >
                                    {letter}
                                </div>
                            }.into_any()
                        }
                    }
                    None => {
                        // Loading state
                        view! {
                            <div 
                                class="profile-avatar-placeholder"
                                style:width="100%"
                                style:height="100%"
                                style:border-radius="50%"
                                style:background="#222"
                            />
                        }.into_any()
                    }
                }
            }}
        </div>
    }
}
```

- [ ] **Step 2: Create ProfileDisplayName component**

```rust
//! Profile display name component - shows display_name or name or npub fallback

use leptos::prelude::*;
use crate::store::profiles::use_profile;

#[component]
pub fn ProfileDisplayName(
    npub: String,
    #[prop(optional)] truncate_npub: Option<usize>,
) -> impl IntoView {
    let profile = use_profile(npub.clone());
    
    view! {
        <span class="profile-display-name">
            {move || {
                match profile.get() {
                    Some(p) => {
                        // Prefer display_name, fallback to name, then npub
                        let display = p.display();
                        view! { <span>{display}</span> }.into_any()
                    }
                    None => {
                        // Show truncated npub
                        let display = if let Some(len) = truncate_npub {
                            if npub.len() > len {
                                format!("{}...", &npub[..len])
                            } else {
                                npub.clone()
                            }
                        } else {
                            npub.clone()
                        };
                        view! { <span class="npub-fallback">{display}</span> }.into_any()
                    }
                }
            }}
        </span>
    }
}
```

- [ ] **Step 3: Update ListingCard component**

Modify `app/src/components/browse.rs`:

```rust
use crate::components::profile_avatar::ProfileAvatar;
use crate::components::profile_display::ProfileDisplayName;
use crate::store::profiles::fetch_and_store_profile;
use wasm_bindgen_futures::spawn_local;

#[component]
pub fn ListingCard(
    listing: GameListing,
    on_select: Callback<GameListing>,
) -> impl IntoView {
    let listing_clone = listing.clone();
    let listing_for_click = listing.clone();
    
    // Fetch profile if not cached
    Effect::new(move |_| {
        let npub = listing.publisher_npub.clone();
        spawn_local(async move {
            // This will use cached version if available
            let _ = fetch_and_store_profile(npub).await;
        });
    });
    
    let on_click = move |_| {
        on_select.run(listing_for_click.clone());
    };
    
    view! {
        <div class="listing-card">
            <div class="listing-header">
                <h3 class="listing-title">{listing.title.clone()}</h3>
                
                // NEW: Profile row with avatar + name
                <div class="listing-publisher-row">
                    <ProfileAvatar 
                        npub={listing.publisher_npub.clone()} 
                        size="24px"
                    />
                    <ProfileDisplayName 
                        npub={listing.publisher_npub.clone()}
                        truncate_npub=16
                    />
                </div>
            </div>
            
            // ... rest of component ...
        </div>
    }
}
```

Add CSS:
```css
.listing-publisher-row {
    display: flex;
    align-items: center;
    gap: 8px;
    margin-top: 4px;
}

.npub-fallback {
    color: #666;
    font-family: monospace;
    font-size: 0.9em;
}
```

- [ ] **Step 4: Update detail view**

Modify `app/src/components/detail.rs` to use profile components:

```rust
use crate::components::{ProfileAvatar, ProfileDisplayName};

// In the view! block, replace npub display with:
<div class="seller-profile">
    <ProfileAvatar npub={listing.publisher_npub.clone()} size="48px" />
    <div class="seller-info">
        <ProfileDisplayName npub={listing.publisher_npub.clone()} />
        // ... nip05, about, etc.
    </div>
</div>
```

---

## Phase 4: Background Refresh & Batch Preloading

### Task 4: Implement Browse View Profile Preloading

**Files:**
- Modify: `app/src/components/browse.rs` - Batch fetch profiles when listings load
- Create: `desktop/src/commands/profiles.rs` - Batch fetch command
- Modify: `desktop/src/main.rs` - Register batch command

**Objective:** When listings are fetched, batch-fetch all publisher profiles in one operation.

- [ ] **Step 1: Add batch profile fetch command**

Create `desktop/src/commands/profiles.rs`:

```rust
//! Profile-related Tauri commands

use arcadestr_core::nostr::UserProfile;
use tauri::State;
use std::sync::Arc;
use tokio::sync::Mutex;

use crate::AppState;

/// Fetch multiple profiles in batch
#[tauri::command]
pub async fn fetch_profiles_batch(
    npubs: Vec<String>,
    state: State<'_, AppState>,
) -> Result<Vec<UserProfile>, String> {
    let nostr = state.nostr.lock().await;
    let profile_fetcher = state.profile_fetcher.clone();
    
    // Queue all requested profiles
    profile_fetcher.enqueue_many(npubs);
    
    // Fetch immediately in batches
    let mut all_profiles = Vec::new();
    loop {
        let (batch, remaining) = profile_fetcher.fetch_batch(&nostr).await;
        if batch.is_empty() {
            break;
        }
        all_profiles.extend(batch);
        if remaining == 0 {
            break;
        }
    }
    
    Ok(all_profiles)
}

/// Get all cached profiles from SQLite
#[tauri::command]
pub async fn get_cached_profiles(
    state: State<'_, AppState>,
) -> Result<Vec<UserProfile>, String> {
    let cache = state.user_cache.clone();
    
    cache.get_all()
        .await
        .map_err(|e| e.to_string())
}

/// Preload profiles for a list of npubs (non-blocking)
#[tauri::command]
pub async fn preload_profiles(
    npubs: Vec<String>,
    state: State<'_, AppState>,
    app_handle: tauri::AppHandle,
) -> Result<(), String> {
    let nostr = state.nostr.lock().await;
    let profile_fetcher = state.profile_fetcher.clone();
    
    // Queue all profiles
    profile_fetcher.enqueue_many(npubs);
    
    // Spawn background task to fetch
    tokio::spawn(async move {
        loop {
            let (batch, remaining) = profile_fetcher.fetch_batch(&nostr).await;
            
            // Emit each profile
            for profile in batch {
                let _ = app_handle.emit("profile_fetched", profile);
            }
            
            if remaining == 0 || batch.is_empty() {
                break;
            }
            
            tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
        }
    });
    
    Ok(())
}
```

- [ ] **Step 2: Register commands in main.rs**

Modify `desktop/src/main.rs`:

```rust
mod commands {
    pub mod profiles;
}

use commands::profiles::{fetch_profiles_batch, get_cached_profiles, preload_profiles};

// In generate_handler! macro:
generate_handler![
    // ... existing commands ...
    fetch_profiles_batch,
    get_cached_profiles,
    preload_profiles,
]
```

- [ ] **Step 3: Add invoke helper for batch fetch**

In `app/src/lib.rs`:

```rust
/// Invoke fetch_profiles_batch command
#[cfg(any(target_arch = "wasm32", not(feature = "web")))]
pub async fn invoke_fetch_profiles_batch(npubs: Vec<String>) -> Result<Vec<UserProfile>, String> {
    use crate::tauri_invoke::invoke;
    
    let args = serde_json::json!({ "npubs": npubs });
    invoke("fetch_profiles_batch", args).await
}

/// Invoke get_cached_profiles command
#[cfg(any(target_arch = "wasm32", not(feature = "web")))]
pub async fn invoke_get_cached_profiles() -> Result<Vec<UserProfile>, String> {
    use crate::tauri_invoke::invoke;
    
    invoke("get_cached_profiles", serde_json::json!({})).await
}

/// Invoke preload_profiles command (non-blocking)
#[cfg(any(target_arch = "wasm32", not(feature = "web")))]
pub async fn invoke_preload_profiles(npubs: Vec<String>) -> Result<(), String> {
    use crate::tauri_invoke::invoke;
    
    let args = serde_json::json!({ "npubs": npubs });
    invoke("preload_profiles", args).await
}
```

- [ ] **Step 4: Update BrowseView to preload profiles**

Modify `app/src/components/browse.rs`:

```rust
use crate::{invoke_preload_profiles, fetch_missing_profiles};
use crate::store::profiles::use_profile_store;

#[component]
pub fn BrowseView(
    on_select: Callback<GameListing>,
) -> impl IntoView {
    let _auth = use_context::<AuthContext>().expect("AuthContext not provided");
    let profile_store = use_profile_store();
    
    let listings = RwSignal::new(Vec::<GameListing>::new());
    let is_loading = RwSignal::new(true);
    let error = RwSignal::new(None::<String>);
    
    // Fetch listings and preload profiles
    Effect::new(move |_| {
        spawn_local(async move {
            is_loading.set(true);
            error.set(None);
            
            match invoke_fetch_listings(20).await {
                Ok(fetched) => {
                    // Extract unique publisher npubs
                    let npubs: Vec<String> = fetched.iter()
                        .map(|l| l.publisher_npub.clone())
                        .collect::<std::collections::HashSet<_>>()
                        .into_iter()
                        .collect();
                    
                    // Preload all profiles in background
                    if !npubs.is_empty() {
                        let _ = invoke_preload_profiles(npubs).await;
                    }
                    
                    listings.set(fetched);
                    is_loading.set(false);
                }
                Err(e) => {
                    error.set(Some(e));
                    is_loading.set(false);
                }
            }
        });
    });
    
    // ... rest of component ...
}
```

---

## Testing Checklist

- [ ] Unit tests for UserCache (CRUD operations)
- [ ] Integration test: ProfileFetcher persists to SQLite
- [ ] Frontend: Profile store updates when events received
- [ ] UI: ListingCard displays profile avatar + name
- [ ] E2E: Browse view shows profiles for all listings
- [ ] Performance: Batch fetch 50+ profiles efficiently

## Migration Notes

Existing users will automatically get the new `users` table on next app start (migration system handles this).

## Rollback Plan

If issues arise:
1. Disable event listeners in app
2. Revert to npub-only display
3. Clear `users` table if data corruption suspected

## Success Metrics

1. Profiles load within 2 seconds for cached users
2. No raw npubs visible in UI (except as fallback during load)
3. Browse view displays profile pics for 95%+ of listings
4. Background fetch doesn't block UI interactions

---

**Plan complete. Ready for implementation.**
