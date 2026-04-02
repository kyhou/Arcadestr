// Core business logic: NOSTR events, Lightning payments, signer integration.

// Signer is needed for both native and WASM targets
pub mod signers;

// Auth and storage are native-only (require sqlx, encryption, etc.)
#[cfg(feature = "native")]
pub mod auth;

#[cfg(feature = "native")]
pub mod storage;

// NIP-46 remote signing module (native-only, uses OS keychain)
#[cfg(feature = "native")]
pub mod nip46;

pub mod saved_users;
pub mod version;

#[cfg(feature = "native")]
pub mod relay_cache;

#[cfg(feature = "native")]
pub use relay_cache::{RelayCache, CachedRelayList, RelayCacheError, RelayType, RelayHealth};

#[cfg(feature = "native")]
pub mod nostr;

#[cfg(feature = "native")]
pub mod subscriptions;

#[cfg(feature = "native")]
pub mod profile_fetcher;

#[cfg(feature = "native")]
pub mod user_cache;

#[cfg(feature = "native")]
pub use profile_fetcher::{ProfileFetcher, ProfileCache, LruProfileCache, BATCH_SIZE, MAX_PROFILE_ATTEMPTS};

#[cfg(feature = "native")]
pub use user_cache::UserCache;

#[cfg(feature = "native")]
pub mod lightning;

// WASM-compatible stubs
#[cfg(feature = "wasm")]
pub mod wasm_stub;
