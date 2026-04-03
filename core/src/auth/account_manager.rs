use crate::auth::{Account, AccountInfo, SigningMode};
use crate::signers::{LocalSigner, NostrSigner, SignerError};
use crate::storage::{
    Database, DatabaseError, EncryptedData, Encryption, EncryptionError, MasterKeyError,
    MasterKeyManager,
};
use nostr::{Keys, ToBech32};
use std::path::Path;
use thiserror::Error;
use zeroize::Zeroizing;

/// Manages user accounts with secure local storage
///
/// This is the main interface for:
/// - Creating new accounts from nsec
/// - Loading existing accounts
/// - Decrypting nsec for signing
/// - Account switching and management
pub struct AccountManager {
    db: Database,
    encryption: Encryption,
    master_key_manager: MasterKeyManager,
}

#[derive(Debug, Error)]
pub enum AccountManagerError {
    #[error("Database error: {0}")]
    Database(#[from] DatabaseError),
    #[error("SQL error: {0}")]
    Sql(#[from] sqlx::Error),
    #[error("Master key error: {0}")]
    MasterKey(#[from] MasterKeyError),
    #[error("Encryption error: {0}")]
    Encryption(#[from] EncryptionError),
    #[error("Signer error: {0}")]
    Signer(#[from] SignerError),
    #[error("Invalid nsec format")]
    InvalidNsec,
    #[error("Account not found: {0}")]
    AccountNotFound(String),
    #[error("No active account")]
    NoActiveAccount,
    #[error("Serialization error: {0}")]
    Serialization(String),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

impl AccountManager {
    /// Create a new AccountManager
    ///
    /// # Arguments
    /// * `data_dir` - Directory for database and master key storage
    pub async fn new(data_dir: &Path) -> Result<Self, AccountManagerError> {
        // Initialize master key manager
        let master_key_manager = MasterKeyManager::new(data_dir);

        // Initialize or retrieve master key
        let master_key = master_key_manager.initialize().await?;

        // Initialize database
        let db_path = data_dir.join("accounts.db");
        let db = Database::new(&db_path).await?;

        // Initialize encryption
        let encryption = Encryption::new(&master_key)?;

        Ok(Self {
            db,
            encryption,
            master_key_manager,
        })
    }

    /// Login with nsec (creates a local encrypted account)
    ///
    /// This is the fast-path login that enables ~4 second startup times.
    /// The nsec is encrypted with AES-256-GCM and stored in the database.
    ///
    /// # Arguments
    /// * `nsec` - The nsec private key (will be zeroized after encryption)
    ///
    /// # Returns
    /// The newly created account
    pub async fn login_with_nsec(&self, nsec: &str) -> Result<Account, AccountManagerError> {
        // Validate nsec and extract keys
        let keys = Keys::parse(nsec).map_err(|_| AccountManagerError::InvalidNsec)?;
        let pubkey_hex = keys.public_key().to_hex();
        let npub = keys
            .public_key()
            .to_bech32()
            .map_err(|_| AccountManagerError::InvalidNsec)?;

        // Encrypt nsec
        let encrypted = self.encryption.encrypt_nsec(nsec);
        let encrypted_nsec = Encryption::serialize(&encrypted)
            .map_err(|e| AccountManagerError::Serialization(e.to_string()))?;

        // Create account
        let now = chrono::Utc::now().timestamp();
        let account = Account {
            id: format!("user_{}", ulid::Ulid::new()),
            pubkey: pubkey_hex,
            npub: npub.clone(),
            signing_mode: SigningMode::Local,
            encrypted_nsec: Some(encrypted_nsec),
            display_name: None,
            picture: None,
            created_at: now,
            last_used: now,
            is_active: true,
        };

        // Save to database
        self.save_account(&account).await?;

        // Deactivate other accounts
        self.set_active_account(&account.id).await?;

        tracing::info!("Created local account for {}", npub);

        Ok(account)
    }

    /// Load the currently active account
    ///
    /// This is called on app startup for fast login.
    /// Returns instantly from local database - no network required.
    pub async fn load_active_account(&self) -> Result<Option<Account>, AccountManagerError> {
        let account =
            sqlx::query_as::<_, Account>("SELECT * FROM accounts WHERE is_active = 1 LIMIT 1")
                .fetch_optional(self.db.pool())
                .await?;

        if let Some(ref acc) = account {
            // Update last_used timestamp
            let now = chrono::Utc::now().timestamp();
            sqlx::query("UPDATE accounts SET last_used = ? WHERE id = ?")
                .bind(now)
                .bind(&acc.id)
                .execute(self.db.pool())
                .await?;
        }

        Ok(account)
    }

    /// Get a signer for an account
    ///
    /// For local accounts, this decrypts the nsec and creates a LocalSigner.
    /// For remote accounts, this would create a NIP-46 signer (not yet implemented).
    ///
    /// # Arguments
    /// * `account` - The account to create a signer for
    ///
    /// # Returns
    /// A boxed NostrSigner trait object
    pub fn get_signer(
        &self,
        account: &Account,
    ) -> Result<Box<dyn NostrSigner>, AccountManagerError> {
        match account.signing_mode {
            SigningMode::Local => {
                let encrypted_bytes = account
                    .encrypted_nsec
                    .as_ref()
                    .ok_or(AccountManagerError::Signer(SignerError::NoPrivateKey))?;

                let encrypted: EncryptedData = Encryption::deserialize(encrypted_bytes)
                    .map_err(|e| AccountManagerError::Serialization(e.to_string()))?;

                let nsec = self.encryption.decrypt_nsec(&encrypted)?;
                let signer = LocalSigner::from_nsec(&nsec)?;

                // nsec is automatically zeroized when dropped
                Ok(Box::new(signer))
            }
            SigningMode::Remote => {
                // TODO: Implement NIP-46 remote signer
                Err(AccountManagerError::Signer(
                    SignerError::RemoteNotImplemented,
                ))
            }
            SigningMode::ReadOnly => Err(AccountManagerError::Signer(SignerError::ReadOnlyAccount)),
        }
    }

    /// Get the decrypted nsec for a local account
    ///
    /// # Arguments
    /// * `account` - The account to decrypt nsec for
    ///
    /// # Returns
    /// The decrypted nsec string (zeroized on drop)
    pub fn get_nsec(&self, account: &Account) -> Result<Zeroizing<String>, AccountManagerError> {
        match account.signing_mode {
            SigningMode::Local => {
                let encrypted_bytes = account
                    .encrypted_nsec
                    .as_ref()
                    .ok_or(AccountManagerError::Signer(SignerError::NoPrivateKey))?;

                let encrypted: EncryptedData = Encryption::deserialize(encrypted_bytes)
                    .map_err(|e| AccountManagerError::Serialization(e.to_string()))?;

                let nsec = self.encryption.decrypt_nsec(&encrypted)?;
                Ok(nsec)
            }
            _ => Err(AccountManagerError::Signer(SignerError::NoPrivateKey)),
        }
    }

    /// List all accounts (for account switching UI)
    pub async fn list_accounts(&self) -> Result<Vec<AccountInfo>, AccountManagerError> {
        let accounts =
            sqlx::query_as::<_, Account>("SELECT * FROM accounts ORDER BY last_used DESC")
                .fetch_all(self.db.pool())
                .await?;

        Ok(accounts.into_iter().map(AccountInfo::from).collect())
    }

    /// Switch to a different account
    pub async fn switch_account(&self, account_id: &str) -> Result<Account, AccountManagerError> {
        // Verify account exists
        let account = sqlx::query_as::<_, Account>("SELECT * FROM accounts WHERE id = ?")
            .bind(account_id)
            .fetch_optional(self.db.pool())
            .await?
            .ok_or_else(|| AccountManagerError::AccountNotFound(account_id.to_string()))?;

        // Set as active
        self.set_active_account(account_id).await?;

        tracing::info!("Switched to account {}", account.npub);

        Ok(account)
    }

    /// Delete an account
    pub async fn delete_account(&self, account_id: &str) -> Result<(), AccountManagerError> {
        sqlx::query("DELETE FROM accounts WHERE id = ?")
            .bind(account_id)
            .execute(self.db.pool())
            .await?;

        tracing::info!("Deleted account {}", account_id);

        Ok(())
    }

    /// Add a remote (NIP-46) account
    ///
    /// This creates an account entry for NIP-46 connected accounts (Amber, etc.)
    /// The account is stored with SigningMode::Remote and no encrypted_nsec.
    ///
    /// # Arguments
    /// * `pubkey_hex` - The public key in hex format
    /// * `npub` - The public key in bech32 format
    /// * `display_name` - Optional display name for the account
    ///
    /// # Returns
    /// The newly created account
    pub async fn add_remote_account(
        &self,
        pubkey_hex: &str,
        npub: &str,
        display_name: Option<String>,
    ) -> Result<Account, AccountManagerError> {
        let now = chrono::Utc::now().timestamp();
        let account_id = format!("nip46_{}", &pubkey_hex[..16.min(pubkey_hex.len())]);

        let account = Account {
            id: account_id.clone(),
            pubkey: pubkey_hex.to_string(),
            npub: npub.to_string(),
            signing_mode: SigningMode::Remote,
            encrypted_nsec: None,
            display_name,
            picture: None,
            created_at: now,
            last_used: now,
            is_active: true,
        };

        // Save to database
        self.save_account(&account).await?;

        // Deactivate other accounts
        self.set_active_account(&account_id).await?;

        tracing::info!("Created remote account for {}", npub);

        Ok(account)
    }

    /// Update account profile information
    pub async fn update_profile(
        &self,
        account_id: &str,
        display_name: Option<String>,
        picture: Option<String>,
    ) -> Result<(), AccountManagerError> {
        sqlx::query("UPDATE accounts SET display_name = ?, picture = ? WHERE id = ?")
            .bind(&display_name)
            .bind(&picture)
            .bind(account_id)
            .execute(self.db.pool())
            .await?;

        Ok(())
    }

    /// Save account to database (internal helper)
    async fn save_account(&self, account: &Account) -> Result<(), AccountManagerError> {
        sqlx::query(
            "INSERT INTO accounts (id, pubkey, npub, signing_mode, encrypted_nsec, 
             display_name, picture, created_at, last_used, is_active)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT(id) DO UPDATE SET
             encrypted_nsec = excluded.encrypted_nsec,
             display_name = excluded.display_name,
             picture = excluded.picture,
             last_used = excluded.last_used,
             is_active = excluded.is_active",
        )
        .bind(&account.id)
        .bind(&account.pubkey)
        .bind(&account.npub)
        .bind(&account.signing_mode)
        .bind(&account.encrypted_nsec)
        .bind(&account.display_name)
        .bind(&account.picture)
        .bind(account.created_at)
        .bind(account.last_used)
        .bind(account.is_active)
        .execute(self.db.pool())
        .await?;

        Ok(())
    }

    /// Set active account (deactivates all others)
    async fn set_active_account(&self, account_id: &str) -> Result<(), AccountManagerError> {
        // Deactivate all accounts
        sqlx::query("UPDATE accounts SET is_active = 0")
            .execute(self.db.pool())
            .await?;

        // Activate specified account
        sqlx::query("UPDATE accounts SET is_active = 1 WHERE id = ?")
            .bind(account_id)
            .execute(self.db.pool())
            .await?;

        Ok(())
    }

    /// Get account by ID
    pub async fn get_account(
        &self,
        account_id: &str,
    ) -> Result<Option<Account>, AccountManagerError> {
        let account = sqlx::query_as::<_, Account>("SELECT * FROM accounts WHERE id = ?")
            .bind(account_id)
            .fetch_optional(self.db.pool())
            .await?;

        Ok(account)
    }

    /// Get account by npub
    pub async fn get_account_by_npub(
        &self,
        npub: &str,
    ) -> Result<Option<Account>, AccountManagerError> {
        let account = sqlx::query_as::<_, Account>("SELECT * FROM accounts WHERE npub = ?")
            .bind(npub)
            .fetch_optional(self.db.pool())
            .await?;

        Ok(account)
    }

    /// Check if any accounts exist
    pub async fn has_accounts(&self) -> Result<bool, AccountManagerError> {
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM accounts")
            .fetch_one(self.db.pool())
            .await?;

        Ok(count > 0)
    }

    /// Get database pool reference (for advanced queries)
    pub fn db_pool(&self) -> &sqlx::SqlitePool {
        self.db.pool()
    }

    /// Create encrypted backup of all accounts
    ///
    /// # Returns
    /// Encrypted backup data as JSON string
    pub async fn backup_accounts(&self) -> Result<String, AccountManagerError> {
        use crate::storage::RelayBackup;

        // Get all accounts
        let accounts = self.list_accounts().await?;

        // Fetch full account data for each
        let mut full_accounts = Vec::new();
        for info in accounts {
            if let Some(account) = self.get_account(&info.id).await? {
                full_accounts.push(account);
            }
        }

        // Create backup manager
        let backup = RelayBackup::new(&self.encryption);

        // Create backup
        let backup_data = backup.create_backup(&full_accounts).map_err(|e| {
            AccountManagerError::Encryption(crate::storage::EncryptionError::Serialization(
                e.to_string(),
            ))
        })?;

        Ok(backup_data)
    }

    /// Restore accounts from encrypted backup
    ///
    /// # Arguments
    /// * `backup_data` - The encrypted backup JSON string
    ///
    /// # Returns
    /// Number of accounts restored
    pub async fn restore_accounts(&self, backup_data: &str) -> Result<usize, AccountManagerError> {
        use crate::auth::SigningMode;
        use crate::storage::{BackupAccountData, RelayBackup};

        // Create backup manager
        let backup = RelayBackup::new(&self.encryption);

        // Restore backup
        let restored_accounts: Vec<BackupAccountData> =
            backup.restore_backup(backup_data).map_err(|e| {
                AccountManagerError::Encryption(crate::storage::EncryptionError::Serialization(
                    e.to_string(),
                ))
            })?;

        let mut count = 0;
        for backup_acc in restored_accounts {
            // Check if account already exists
            if let Ok(Some(_)) = self.get_account_by_npub(&backup_acc.npub).await {
                tracing::info!("Account {} already exists, skipping", backup_acc.npub);
                continue;
            }

            // Parse signing mode
            let signing_mode = match backup_acc.signing_mode.as_str() {
                "Local" => SigningMode::Local,
                "Remote" => SigningMode::Remote,
                "ReadOnly" => SigningMode::ReadOnly,
                _ => SigningMode::Local,
            };

            // Decode encrypted_nsec from base64
            let encrypted_nsec = backup_acc
                .encrypted_nsec
                .and_then(|s| base64::decode(s).ok());

            // Create account
            let account = Account {
                id: format!("user_{}", ulid::Ulid::new()),
                pubkey: backup_acc.pubkey,
                npub: backup_acc.npub,
                signing_mode,
                encrypted_nsec,
                display_name: backup_acc.display_name,
                picture: backup_acc.picture,
                created_at: backup_acc.created_at,
                last_used: backup_acc.last_used,
                is_active: false, // Don't auto-activate restored accounts
            };

            // Save to database
            self.save_account(&account).await?;
            count += 1;

            tracing::info!("Restored account {}", account.npub);
        }

        Ok(count)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    // Note: These tests require a valid nsec. In production tests,
    // we'd use a test fixture with a known key.

    #[tokio::test]
    async fn test_account_manager_creation() {
        let temp_dir = TempDir::new().unwrap();
        let manager = AccountManager::new(temp_dir.path()).await.unwrap();

        // Should have no accounts initially
        assert!(!manager.has_accounts().await.unwrap());
    }
}
