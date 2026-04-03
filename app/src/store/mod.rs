//! Global state stores for the Arcadestr application.

pub mod marketplace;
pub mod profiles;

pub use marketplace::{
    provide_marketplace_store, try_use_marketplace_store, use_marketplace_store, MarketplaceStore,
    DEFAULT_LISTING_TTL_SECS,
};
pub use profiles::{
    provide_profile_store, try_use_profile_store, use_profile, use_profile_store, ProfileStore,
};
