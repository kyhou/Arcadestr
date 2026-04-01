//! Authentication and account management module
//! 
//! This module provides secure local storage of Nostr accounts with:
//! - AES-256-GCM encryption for nsec
//! - SQLite database for account metadata
//! - Fast local signing without NIP-46 reconnection

mod account;
mod account_manager;
mod auth_state;

pub use account::{Account, AccountInfo, SigningMode};
pub use account_manager::{AccountManager, AccountManagerError};
pub use auth_state::{AuthState, PendingNostrConnectState};
