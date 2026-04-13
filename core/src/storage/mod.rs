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

pub use backup::{
    fetch_backup_events, publish_backup_to_relays, BackupAccountData, BackupError, BackupMetadata,
    RelayBackup, BACKUP_D_TAG, BACKUP_EVENT_KIND,
};
pub use db::{Database, DatabaseError};
pub use encryption::{
    decrypt_private_key_nip49, encrypt_private_key_nip49, extract_nip49_version, parse_ncryptsec,
    serialize_ncryptsec, validate_nip49_format, validate_nip49_password, EncryptedData, Encryption,
    EncryptionError, Ncryptsec, Nip49ValidationError, ScryptParams,
};
pub use master_key::{MasterKeyError, MasterKeyManager};
