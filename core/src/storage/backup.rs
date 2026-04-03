//! NIP-78 Encrypted Relay Backup
//!
//! This module implements encrypted backup of account data to Nostr relays
//! using NIP-78 (Application-Specific Data) events.
//!
//! ## Backup Format
//! - Event kind: 30078 (NIP-78 parameterized replaceable)
//! - d tag: "arcadestr_backup_v1"
//! - Content: Encrypted JSON containing account data
//!
//! ## Security
//! - Uses the same AES-256-GCM encryption as local storage
//! - Master key never leaves the device
//! - Only encrypted data is published to relays

use nostr::{Event, EventBuilder, Kind, PublicKey, Tag};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tracing::{error, info, warn};

use crate::auth::{Account, AccountManagerError};
use crate::signers::{NostrSigner, SignerError};
use crate::storage::{EncryptedData, Encryption, EncryptionError};

/// Errors that can occur during backup operations
#[derive(Debug, Error)]
pub enum BackupError {
    #[error("Encryption error: {0}")]
    Encryption(#[from] EncryptionError),

    #[error("Account manager error: {0}")]
    AccountManager(#[from] AccountManagerError),

    #[error("Signer error: {0}")]
    Signer(#[from] SignerError),

    #[error("Serialization error: {0}")]
    Serialization(String),

    #[error("Relay error: {0}")]
    Relay(String),

    #[error("No relays available")]
    NoRelays,

    #[error("Backup not found")]
    NotFound,

    #[error("Invalid backup data")]
    InvalidData,
}

/// NIP-78 event kind for application-specific data (parameterized replaceable)
pub const BACKUP_EVENT_KIND: u16 = 30078;

/// d tag identifier for Arcadestr backups
pub const BACKUP_D_TAG: &str = "arcadestr_backup_v1";

/// Version of the backup format
pub const BACKUP_VERSION: u32 = 1;

/// Account data for backup (serializable)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackupAccountData {
    /// Account ID
    pub id: String,
    /// Public key (hex)
    pub pubkey: String,
    /// Public key (npub)
    pub npub: String,
    /// Signing mode
    pub signing_mode: String,
    /// Encrypted nsec (base64 encoded)
    pub encrypted_nsec: Option<String>,
    /// Display name
    pub display_name: Option<String>,
    /// Picture URL
    pub picture: Option<String>,
    /// Created at timestamp
    pub created_at: i64,
    /// Last used timestamp
    pub last_used: i64,
}

/// Backup data structure (encrypted and published)
#[derive(Debug, Clone, Serialize, Deserialize)]
struct BackupData {
    /// Backup format version
    pub version: u32,
    /// Timestamp when backup was created
    pub backup_timestamp: i64,
    /// Encrypted accounts data (serialized and encrypted)
    pub encrypted_accounts: String,
}

/// Backup metadata (stored locally)
#[derive(Debug, Clone)]
pub struct BackupMetadata {
    /// Backup ID (ULID)
    pub id: String,
    /// Account ID this backup belongs to
    pub account_id: String,
    /// Relay URL where backup was published
    pub relay_url: String,
    /// Event ID of the backup
    pub event_id: String,
    /// Timestamp when backup was created
    pub created_at: i64,
}

/// Manages encrypted relay backups
pub struct RelayBackup<'a> {
    encryption: &'a Encryption,
}

impl<'a> RelayBackup<'a> {
    /// Creates a new RelayBackup instance
    ///
    /// # Arguments
    /// * `encryption` - The encryption instance to use for encrypting/decrypting backups
    pub fn new(encryption: &'a Encryption) -> Self {
        Self { encryption }
    }

    /// Creates an encrypted backup of account data
    ///
    /// # Arguments
    /// * `accounts` - List of accounts to backup
    ///
    /// # Returns
    /// Encrypted backup data ready to be published
    pub fn create_backup(&self, accounts: &[Account]) -> Result<String, BackupError> {
        // Convert accounts to backup format
        let backup_accounts: Vec<BackupAccountData> = accounts
            .iter()
            .map(|acc| BackupAccountData {
                id: acc.id.clone(),
                pubkey: acc.pubkey.clone(),
                npub: acc.npub.clone(),
                signing_mode: format!("{:?}", acc.signing_mode),
                encrypted_nsec: acc.encrypted_nsec.as_ref().map(|v| base64::encode(v)),
                display_name: acc.display_name.clone(),
                picture: acc.picture.clone(),
                created_at: acc.created_at,
                last_used: acc.last_used,
            })
            .collect();

        // Serialize accounts to JSON
        let accounts_json = serde_json::to_string(&backup_accounts)
            .map_err(|e| BackupError::Serialization(e.to_string()))?;

        // Encrypt the accounts data
        let encrypted = self.encryption.encrypt(accounts_json.as_bytes());

        // Serialize encrypted data to base64
        let encrypted_bytes = Encryption::serialize(&encrypted)
            .map_err(|e| BackupError::Serialization(e.to_string()))?;
        let encrypted_base64 = base64::encode(&encrypted_bytes);

        // Create backup structure
        let backup = BackupData {
            version: BACKUP_VERSION,
            backup_timestamp: chrono::Utc::now().timestamp(),
            encrypted_accounts: encrypted_base64,
        };

        // Serialize backup to JSON string
        let backup_json = serde_json::to_string(&backup)
            .map_err(|e| BackupError::Serialization(e.to_string()))?;

        info!("Created encrypted backup for {} accounts", accounts.len());

        Ok(backup_json)
    }

    /// Decrypts backup data and returns accounts
    ///
    /// # Arguments
    /// * `encrypted_backup` - The encrypted backup JSON string
    ///
    /// # Returns
    /// List of accounts from the backup
    pub fn restore_backup(
        &self,
        encrypted_backup: &str,
    ) -> Result<Vec<BackupAccountData>, BackupError> {
        // Parse backup JSON
        let backup: BackupData = serde_json::from_str(encrypted_backup)
            .map_err(|e| BackupError::Serialization(e.to_string()))?;

        // Check version
        if backup.version != BACKUP_VERSION {
            warn!(
                "Backup version mismatch: expected {}, got {}",
                BACKUP_VERSION, backup.version
            );
        }

        // Decode base64 encrypted data
        let encrypted_bytes = base64::decode(&backup.encrypted_accounts)
            .map_err(|e| BackupError::Serialization(format!("Base64 decode failed: {}", e)))?;

        // Deserialize encrypted data
        let encrypted: EncryptedData = Encryption::deserialize(&encrypted_bytes)
            .map_err(|e| BackupError::Serialization(e.to_string()))?;

        // Decrypt
        let decrypted = self
            .encryption
            .decrypt(&encrypted)
            .map_err(BackupError::Encryption)?;

        // Parse accounts JSON
        let accounts: Vec<BackupAccountData> = serde_json::from_slice(&decrypted)
            .map_err(|e| BackupError::Serialization(e.to_string()))?;

        info!("Restored {} accounts from backup", accounts.len());

        Ok(accounts)
    }

    /// Builds a NIP-78 backup event
    ///
    /// # Arguments
    /// * `backup_data` - The encrypted backup data
    /// * `signer` - The signer to use for signing the event
    ///
    /// # Returns
    /// A signed Nostr event ready to be published
    pub async fn build_backup_event<S: NostrSigner>(
        &self,
        backup_data: String,
        signer: &S,
    ) -> Result<Event, BackupError> {
        // Get public key from signer
        let public_key = signer.get_public_key().await?;

        // Build event with NIP-78 format
        let builder = EventBuilder::new(Kind::from(BACKUP_EVENT_KIND), backup_data)
            .tags(vec![Tag::identifier(BACKUP_D_TAG)]);

        // Build unsigned event
        let unsigned = builder.build(public_key);

        // Sign the event
        let signed = signer.sign_event(unsigned).await?;

        info!("Built NIP-78 backup event: {}", signed.id);

        Ok(signed)
    }

    /// Parses a backup event and extracts the backup data
    ///
    /// # Arguments
    /// * `event` - The Nostr event to parse
    ///
    /// # Returns
    /// The backup data string if valid
    pub fn parse_backup_event(event: &Event) -> Result<String, BackupError> {
        // Check event kind
        if event.kind != Kind::from(BACKUP_EVENT_KIND) {
            return Err(BackupError::InvalidData);
        }

        // Check d tag using the identifier method
        let d_tag = event.tags.identifier().ok_or(BackupError::InvalidData)?;

        if d_tag != BACKUP_D_TAG {
            return Err(BackupError::InvalidData);
        }

        Ok(event.content.clone())
    }
}

/// Publishes a backup event to relays
///
/// # Arguments
/// * `client` - The Nostr client to use for publishing
/// * `event` - The signed backup event
///
/// # Returns
/// Result indicating success or failure
pub async fn publish_backup_to_relays(
    client: &nostr_sdk::Client,
    event: &Event,
) -> Result<(), BackupError> {
    match client.send_event(event).await {
        Ok(_) => {
            info!("Published backup event {} to relays", event.id);
            Ok(())
        }
        Err(e) => {
            error!("Failed to publish backup: {}", e);
            Err(BackupError::Relay(e.to_string()))
        }
    }
}

/// Fetches backup events from relays
///
/// # Arguments
/// * `client` - The Nostr client
/// * `public_key` - The public key to fetch backups for
/// * `relay_url` - Optional specific relay URL (uses client's relays if None)
///
/// # Returns
/// List of backup events
pub async fn fetch_backup_events(
    client: &nostr_sdk::Client,
    public_key: PublicKey,
    relay_url: Option<&str>,
) -> Result<Vec<Event>, BackupError> {
    use nostr_sdk::Filter;
    use std::time::Duration;

    // Build filter for NIP-78 backup events
    let filter = Filter::new()
        .author(public_key)
        .kind(Kind::from(BACKUP_EVENT_KIND))
        .identifier(BACKUP_D_TAG);

    // Fetch events
    let events = if let Some(url) = relay_url {
        // Add the specific relay temporarily if not already connected
        if let Err(e) = client.add_relay(url).await {
            warn!("Failed to add relay {}: {}", url, e);
        }

        // Fetch from all connected relays (including the one we just added)
        match client.fetch_events(filter, Duration::from_secs(10)).await {
            Ok(events) => events.into_iter().collect(),
            Err(e) => {
                warn!("Failed to fetch backup events: {}", e);
                vec![]
            }
        }
    } else {
        // Fetch from all connected relays
        match client.fetch_events(filter, Duration::from_secs(10)).await {
            Ok(events) => events.into_iter().collect(),
            Err(e) => {
                warn!("Failed to fetch backup events: {}", e);
                vec![]
            }
        }
    };

    info!("Fetched {} backup events", events.len());

    Ok(events)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::{Database, MasterKeyManager};
    use std::path::PathBuf;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_backup_roundtrip() {
        // Create temp directory
        let temp_dir = TempDir::new().unwrap();
        let data_dir = temp_dir.path();

        // Initialize master key and encryption
        let master_key_manager = MasterKeyManager::new(data_dir);
        let master_key = master_key_manager.initialize().await.unwrap();
        let encryption = Encryption::new(&master_key).unwrap();

        // Create backup manager
        let backup = RelayBackup::new(&encryption);

        // Create test accounts
        let accounts = vec![Account {
            id: "test_1".to_string(),
            pubkey: "abc123".to_string(),
            npub: "npub1test".to_string(),
            signing_mode: crate::auth::SigningMode::Local,
            encrypted_nsec: Some(b"encrypted_data".to_vec()),
            display_name: Some("Test User".to_string()),
            picture: None,
            created_at: 1234567890,
            last_used: 1234567890,
            is_active: true,
        }];

        // Create backup
        let backup_data = backup.create_backup(&accounts).unwrap();

        // Restore backup
        let restored = backup.restore_backup(&backup_data).unwrap();

        // Verify
        assert_eq!(restored.len(), 1);
        assert_eq!(restored[0].id, "test_1");
        assert_eq!(restored[0].npub, "npub1test");
    }
}
