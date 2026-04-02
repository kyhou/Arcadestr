//! Global profile store for managing Nostr user profiles.
//! Mirrors YakiHonne's nostrAuthors Redux slice.

use crate::models::UserProfile;
use leptos::prelude::*;
use std::collections::HashMap;

/// Global profile store - reactive HashMap keyed by npub
#[derive(Clone, Debug)]
pub struct ProfileStore {
    profiles: RwSignal<HashMap<String, UserProfile>>,
}

impl ProfileStore {
    /// Create a new empty profile store
    pub fn new() -> Self {
        Self {
            profiles: RwSignal::new(HashMap::new()),
        }
    }

    /// Get a profile by npub
    pub fn get(&self, npub: &str) -> Option<UserProfile> {
        self.profiles.get().get(npub).cloned()
    }

    /// Add or update a single profile
    pub fn put(&self, profile: UserProfile) {
        self.profiles.update(|map| {
            map.insert(profile.npub.clone(), profile);
        });
    }

    /// Add or update multiple profiles
    pub fn put_many(&self, profiles: Vec<UserProfile>) {
        self.profiles.update(|map| {
            for profile in profiles {
                map.insert(profile.npub.clone(), profile);
            }
        });
    }

    /// Check if a profile exists in the store
    pub fn has(&self, npub: &str) -> bool {
        self.profiles.get().contains_key(npub)
    }

    /// Get all profiles as a vector
    pub fn get_all(&self) -> Vec<UserProfile> {
        self.profiles.get().values().cloned().collect()
    }

    /// Get the raw signal for reactive access
    pub fn signal(&self) -> RwSignal<HashMap<String, UserProfile>> {
        self.profiles
    }
}

impl Default for ProfileStore {
    fn default() -> Self {
        Self::new()
    }
}

/// Provide the profile store as a context
pub fn provide_profile_store() {
    provide_context(ProfileStore::new());
}

/// Hook to access the profile store from any component
/// Panics if not provided - use only when you're sure the store is available
pub fn use_profile_store() -> ProfileStore {
    use_context::<ProfileStore>().expect("ProfileStore not provided")
}

/// Try to get the profile store without panicking
/// Returns None if the store hasn't been provided yet
pub fn try_use_profile_store() -> Option<ProfileStore> {
    use_context::<ProfileStore>()
}

/// Hook to get a specific profile signal - reactive access to a single profile
pub fn use_profile(npub: String) -> Signal<Option<UserProfile>> {
    let store = use_profile_store();

    Signal::derive(move || store.get(&npub))
}
