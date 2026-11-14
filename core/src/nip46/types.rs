// NIP-46 Remote Signing Types
//
// Data structures for NIP-46 "Nostr Connect" remote signing system.
// These types support both Flow A (bunker URI/NIP-05 login) and Flow B (QR code generation).

use nostr::{nips::nip46::NostrConnectURI, Keys, PublicKey};
use serde::{Deserialize, Serialize};

/// Connection state for NIP-46 signer
#[derive(Debug, Clone, PartialEq)]
pub enum ConnectionState {
    /// Initial state, no connection attempted
    Disconnected,
    /// Connection in progress (async handshake happening)
    Connecting,
    /// Successfully connected and ready
    Connected,
    /// Connection failed
    Failed(String),
}

/// Classification of NIP-46 URI formats.
///
/// NIP-46 has two connection flows with different URI structures:
/// - `Bunker`: Signer-initiated (bunker://<signer_pubkey>?relay=...)
/// - `NostrConnect`: Client-initiated (nostrconnect://<client_pubkey>?relay=...&secret=...)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Nip46UriType {
    /// Signer-initiated bunker:// URI containing remote signer pubkey.
    Bunker,
    /// Client-initiated nostrconnect:// URI where signer pubkey arrives later.
    NostrConnect,
}

impl Nip46UriType {
    /// Detects URI type from string prefix.
    ///
    /// # Panics
    /// Panics if URI doesn't start with either `bunker://` or `nostrconnect://`.
    pub fn from_uri(uri: &str) -> Self {
        if uri.starts_with("bunker://") {
            Self::Bunker
        } else if uri.starts_with("nostrconnect://") {
            Self::NostrConnect
        } else {
            panic!("Invalid NIP-46 URI: must be bunker:// or nostrconnect://")
        }
    }
}

/// A saved profile represents a previously authenticated bunker connection.
/// It is safe to serialize and store the NON-SECRET fields to a config file.
/// The SECRET fields (app_keys secret key, bunker URI secret param) MUST go
/// through tauri-plugin-keyring. See `storage.rs` for the split logic.
#[derive(Debug, Clone)]
pub struct SavedProfile {
    /// Stable unique ID for this profile entry (UUID v4 generated at creation).
    pub id: String,

    /// User-friendly display name (e.g. "Main Account", "Gaming Alt").
    pub name: String,

    /// The Nostr public key of the identity being signed for.
    /// This is the USER's pubkey, NOT the ephemeral app key.
    pub user_pubkey: PublicKey,

    /// The full bunker URI. Contains relay list and optional secret.
    /// Store serialized form in keyring under key: "arcadestr_bunker_uri_{id}"
    pub bunker_uri: NostrConnectURI,

    /// The ephemeral keypair generated specifically for this bunker link.
    /// ONLY the secret key bytes go to keyring: "arcadestr_app_key_{id}"
    /// Reconstruct Keys from stored secret on load.
    pub app_keys: Keys,
}

/// Runtime application state managed by Tauri's State system.
/// This lives in memory only — nothing here is written to disk directly.
pub struct AppSignerState {
    /// Serializes short session commits, never the remote approval wait.
    pub auth_operations: std::sync::Arc<tokio::sync::Semaphore>,
    /// Invalidates pending approvals and connection checks after identity changes.
    pub auth_generation: u64,
    /// Changes only when the selected signing session changes, not on canceled approvals.
    pub session_generation: u64,
    /// All profiles loaded from keyring at startup.
    pub saved_profiles: Vec<SavedProfile>,

    /// The currently active nostr-sdk Client with a Nip46Signer.
    /// None = no active session. Replacing this MUST disconnect the old client.
    pub active_client: Option<nostr_sdk::Client>,

    /// The profile ID currently in use.
    pub active_profile_id: Option<String>,

    /// Pending QR connection state for Flow B.
    /// Set when QR is generated, cleared when connection completes or times out.
    pub pending_qr: Option<PendingQrState>,

    /// Whether the app is in offline mode (bunker unreachable but session exists)
    pub is_offline_mode: bool,

    /// Handle to cancel the periodic bunker retry task
    pub bunker_retry_handle: Option<tokio::task::AbortHandle>,

    /// Current connection state for the active signer
    pub connection_state: ConnectionState,
}

impl AppSignerState {
    /// Creates a new empty AppSignerState
    pub fn new() -> Self {
        Self {
            auth_operations: std::sync::Arc::new(tokio::sync::Semaphore::new(1)),
            auth_generation: 0,
            session_generation: 0,
            saved_profiles: Vec::new(),
            active_client: None,
            active_profile_id: None,
            pending_qr: None,
            is_offline_mode: false,
            bunker_retry_handle: None,
            connection_state: ConnectionState::Disconnected,
        }
    }

    pub fn invalidate_pending_auth(&mut self) -> u64 {
        self.auth_generation = self.auth_generation.wrapping_add(1);
        self.pending_qr = None;
        self.auth_generation
    }

    pub fn can_complete_qr(&self, generation: u64, uri: &str) -> bool {
        self.auth_generation == generation
            && self
                .pending_qr
                .as_ref()
                .is_some_and(|pending| pending.uri == uri)
    }
}

#[cfg(test)]
mod auth_generation_tests {
    use super::*;

    #[test]
    fn cancellation_and_new_attempts_reject_late_qr_approvals() {
        let mut state = AppSignerState::new();
        state.session_generation = 42;
        let generation = state.invalidate_pending_auth();
        let pending = PendingQrState {
            uri: "nostrconnect://test-attempt".into(),
            session: None,
            created_at: std::time::Instant::now(),
        };
        state.pending_qr = Some(pending.clone());
        assert!(state.can_complete_qr(generation, &pending.uri));
        state.invalidate_pending_auth();
        assert_eq!(
            state.session_generation, 42,
            "canceling an approval must not invalidate the active session's verification"
        );
        assert!(!state.can_complete_qr(generation, &pending.uri));
        state.pending_qr = Some(pending.clone());
        assert!(!state.can_complete_qr(generation, &pending.uri));
        assert!(state.can_complete_qr(state.auth_generation, &pending.uri));
        assert!(!state.can_complete_qr(state.auth_generation, "different-attempt"));
    }
}

impl Default for AppSignerState {
    fn default() -> Self {
        Self::new()
    }
}

/// Configuration constants for session management
pub mod session_config {
    /// Interval between bunker reconnection attempts in seconds
    pub const BUNKER_RETRY_INTERVAL_SECS: u64 = 30;
    /// Timeout for bunker connection attempts during auto-restore
    pub const BUNKER_CONNECT_TIMEOUT_SECS: u64 = 30;
}

/// Profile metadata for listing (no secrets).
/// This is what's returned to the UI for the profile switcher.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProfileMetadata {
    /// Profile ID
    pub id: String,
    /// Display name
    pub name: String,
    /// User's public key in bech32 format
    pub pubkey_bech32: String,
    /// User's public key in hex format
    pub pubkey_hex: String,
    /// Bunker pubkey (remote signer pubkey) in hex format - used as keyring key
    #[serde(default)]
    pub bunker_pubkey_hex: String,
    /// Profile picture URL
    #[serde(skip_serializing_if = "Option::is_none")]
    pub picture: Option<String>,
    /// Display name from Nostr profile
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    /// Username/handle from Nostr profile
    #[serde(skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    /// NIP-05 identifier
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nip05: Option<String>,
    /// Bio/about text
    #[serde(skip_serializing_if = "Option::is_none")]
    pub about: Option<String>,
}

/// Pending QR connection state (Flow B).
/// Keeps one live listener until approval is claimed, canceled, or expired.
#[derive(Clone)]
pub struct PendingQrState {
    /// The nostrconnect:// URI that was displayed as QR
    pub uri: String,
    /// A single continuously listening connection. None is used by state-only tests.
    pub session: Option<std::sync::Arc<super::QrLoginSession>>,
    /// Timestamp when the QR was generated
    pub created_at: std::time::Instant,
}

impl std::fmt::Debug for PendingQrState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PendingQrState")
            .field("created_at", &self.created_at)
            .finish_non_exhaustive()
    }
}

/// Keyring key naming constants.
pub mod keyring_keys {
    /// Prefix for all Arcadestr keyring entries
    pub const PREFIX: &str = "arcadestr";

    /// Key for storing app secret key (hex format)
    pub fn app_key(profile_id: &str) -> String {
        format!("{}_app_key_{}", PREFIX, profile_id)
    }

    /// Key for storing bunker URI
    pub fn bunker_uri(profile_id: &str) -> String {
        format!("{}_bunker_uri_{}", PREFIX, profile_id)
    }

    /// Key for storing the profile index (JSON array of metadata)
    pub const PROFILE_INDEX: &str = "arcadestr_profile_index";
}

/// Errors that can occur during NIP-46 keyring operations.
#[derive(Debug, thiserror::Error)]
pub enum Nip46KeyringError {
    #[error("Keyring error: {0}")]
    Keyring(String),

    #[error("Profile not found: {0}")]
    ProfileNotFound(String),

    #[error("Invalid key format: {0}")]
    InvalidKey(String),

    #[error("Serialization error: {0}")]
    Serialization(String),

    #[error("URI parse error: {0}")]
    UriParse(String),
}

impl From<keyring::Error> for Nip46KeyringError {
    fn from(e: keyring::Error) -> Self {
        Nip46KeyringError::Keyring(e.to_string())
    }
}

impl From<nostr::nips::nip46::Error> for Nip46KeyringError {
    fn from(e: nostr::nips::nip46::Error) -> Self {
        Nip46KeyringError::UriParse(e.to_string())
    }
}
