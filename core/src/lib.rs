// Core business logic: NOSTR events, Lightning payments, signer integration.

// Signer and auth are needed for both native and WASM targets
pub mod signer;
pub mod auth;
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
pub use profile_fetcher::{ProfileFetcher, ProfileCache, LruProfileCache, BATCH_SIZE, MAX_PROFILE_ATTEMPTS};

#[cfg(feature = "native")]
pub mod lightning;

// WASM-compatible stubs
#[cfg(feature = "wasm")]
pub mod wasm_stub;
