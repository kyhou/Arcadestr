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
pub mod relay_hints;

#[cfg(feature = "native")]
pub mod relay_pool;

#[cfg(feature = "native")]
pub mod relay_manager;

#[cfg(feature = "native")]
pub use relay_cache::{CachedRelayList, RelayCache, RelayCacheError, RelayHealth, RelayType};

#[cfg(feature = "native")]
pub use relay_pool::{RelayPool, RelaySource};

#[cfg(feature = "native")]
pub use relay_manager::{RelayManager, RelayManagerConfig, RelayManagerError, SendEventResult, RelaySendResult};

#[cfg(feature = "native")]
pub mod nostr;

#[cfg(feature = "native")]
pub mod subscriptions;

#[cfg(feature = "native")]
pub use subscriptions::{
    dispatch_ephemeral_read,
    dispatch_ephemeral_reads_batch,
    dispatch_permanent_subscriptions,
    run_notification_loop,
    ConnectionKind,
    SerializableEvent,
    SubscriptionRegistry,
};

#[cfg(feature = "native")]
pub mod profile_fetcher;

#[cfg(feature = "native")]
pub mod nip05_validator;

#[cfg(feature = "native")]
pub mod user_cache;

#[cfg(feature = "native")]
pub mod social_graph;

#[cfg(feature = "native")]
pub mod extended_network;

#[cfg(feature = "native")]
pub use profile_fetcher::{
    LruProfileCache, ProfileCache, ProfileFetcher, BATCH_SIZE, MAX_PROFILE_ATTEMPTS,
};

#[cfg(feature = "native")]
pub use user_cache::UserCache;

#[cfg(feature = "native")]
pub use nip05_validator::{Nip05Validator, ValidationCommand, ValidationResult};

#[cfg(feature = "native")]
pub mod lightning;

// WASM-compatible stubs
#[cfg(feature = "wasm")]
pub mod wasm_stub;
