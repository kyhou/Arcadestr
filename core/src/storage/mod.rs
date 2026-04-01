//! Secure storage module for encrypted account credentials
//! 
//! This module provides:
//! - SQLite database for account storage
//! - AES-256-GCM encryption for nsec
//! - Linux Secret Service integration for master key storage
//! - Migration from legacy JSON storage
//! - NIP-78 encrypted relay backup

mod backup;
mod db;
mod encryption;
mod master_key;

pub use backup::{RelayBackup, BackupError, BackupMetadata, BackupAccountData, publish_backup_to_relays, fetch_backup_events, BACKUP_EVENT_KIND, BACKUP_D_TAG};
pub use db::{Database, DatabaseError};
pub use encryption::{Encryption, EncryptedData, EncryptionError};
pub use master_key::{MasterKeyManager, MasterKeyError};
