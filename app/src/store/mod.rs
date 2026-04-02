//! Global state stores for the Arcadestr application.

pub mod profiles;

pub use profiles::{
    provide_profile_store, try_use_profile_store, use_profile, use_profile_store, ProfileStore,
};
