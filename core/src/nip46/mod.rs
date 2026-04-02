// NIP-46 Remote Signing (Nostr Connect) Module
//
// Implements NIP-46 "Nostr Connect" remote signing system for Arcadestr.
// This module provides secure bunker-based authentication using OS-native
// keychain storage via tauri-plugin-keyring.
//
// ## Security Model
//
// - All secrets (app_keys, bunker URIs) are stored in the OS keychain
// - The frontend (Leptos) never handles raw keys or secrets
// - One active session at a time - previous sessions are dropped before new ones
// - NIP-44 v2 encryption is used for all kind 24133 events (handled by nostr-connect crate)
//
// ## Flows
//
// **Flow A**: User provides bunker:// URI or NIP-05 identifier
// **Flow B**: App generates nostrconnect:// URI as QR code for mobile wallet scan

pub mod auth;
pub mod methods;
pub mod session;
pub mod storage;
pub mod types;

pub use auth::{init_signer_session, init_signer_session_fast, generate_login_qr, wait_for_qr_connection, TauriAuthUrlHandler};
pub use methods::{get_public_key, sign_event, nip44_encrypt, nip44_decrypt, ping, get_relay_status};
pub use session::{activate_profile, logout, ping_active_signer, restore_session_on_startup, 
                 cancel_bunker_retry, attempt_manual_reconnect, SessionRestoreResult};
pub use storage::{
    delete_profile_from_keyring, get_profile_metadata_by_id, get_profile_metadata_by_pubkey,
    list_profile_index, load_profile_from_keyring, migrate_profile_to_keyring, profile_exists,
    save_profile_to_keyring, set_last_active_profile_id, get_last_active_profile_id, 
    clear_last_active_profile_id, set_profile_cache_dir,
};
pub use types::{
    keyring_keys, Nip46KeyringError, AppSignerState, PendingQrState, ProfileMetadata,
    SavedProfile, session_config, ConnectionState,
};
