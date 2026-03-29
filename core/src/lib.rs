// Core business logic: NOSTR events, Lightning payments, signer integration.

// Signer and auth are needed for both native and WASM targets
pub mod signer;
pub mod auth;
pub mod saved_users;
pub mod relay_cache;
pub use relay_cache::{RelayCache, CachedRelayList, RelayCacheError, RelayType, RelayHealth};

#[cfg(feature = "native")]
pub mod nostr;

#[cfg(feature = "native")]
pub mod lightning;

// WASM-compatible stubs
#[cfg(feature = "wasm")]
pub mod wasm_stub;
