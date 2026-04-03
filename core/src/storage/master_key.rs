use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use rand::{prelude::*, TryRng};
use std::path::{Path, PathBuf};
use thiserror::Error;

/// Manages the master encryption key using secure file storage
///
/// For Linux, stores in ~/.local/share/arcadestr/.master_key with 0600 permissions
/// This is similar to Amethyst's fallback approach and is secure for desktop use
pub struct MasterKeyManager {
    key_file: PathBuf,
}

#[derive(Debug, Error)]
pub enum MasterKeyError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Key not found")]
    NotFound,
    #[error("Invalid key length: expected 32 bytes, got {0}")]
    InvalidLength(usize),
    #[error("Random generation failed")]
    RandomGeneration,
    #[error("Base64 decode failed: {0}")]
    Base64(String),
}

impl MasterKeyManager {
    const KEY_LENGTH: usize = 32; // 256 bits for AES-256
    const KEY_FILENAME: &'static str = ".master_key";

    /// Create a new MasterKeyManager
    pub fn new(data_dir: &Path) -> Self {
        let key_file = data_dir.join(Self::KEY_FILENAME);
        Self { key_file }
    }

    /// Initialize or retrieve master key
    ///
    /// If a key already exists, it will be returned.
    /// If not, a new 256-bit key will be generated and stored securely.
    pub async fn initialize(&self) -> Result<Vec<u8>, MasterKeyError> {
        // Try to read existing key
        if self.key_file.exists() {
            let encoded = tokio::fs::read_to_string(&self.key_file).await?;
            let key = BASE64
                .decode(encoded.trim())
                .map_err(|e| MasterKeyError::Base64(e.to_string()))?;

            if key.len() != Self::KEY_LENGTH {
                return Err(MasterKeyError::InvalidLength(key.len()));
            }

            tracing::info!("Retrieved existing master key from secure storage");
            return Ok(key);
        }

        // Generate new master key
        let mut key = vec![0u8; Self::KEY_LENGTH];
        rand::rng()
            .try_fill_bytes(&mut key)
            .map_err(|_| MasterKeyError::RandomGeneration)?;

        // Ensure parent directory exists
        if let Some(parent) = self.key_file.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }

        // Store key (base64 encoded) with restrictive permissions
        let encoded = BASE64.encode(&key);
        tokio::fs::write(&self.key_file, encoded).await?;

        // Set file permissions to owner-only (0600)
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = tokio::fs::metadata(&self.key_file).await?.permissions();
            perms.set_mode(0o600);
            tokio::fs::set_permissions(&self.key_file, perms).await?;
        }

        tracing::info!("Generated and stored new master key in secure storage");
        Ok(key)
    }

    /// Delete master key
    ///
    /// WARNING: This will make all encrypted data inaccessible!
    pub async fn delete(&self) -> Result<(), MasterKeyError> {
        if self.key_file.exists() {
            tokio::fs::remove_file(&self.key_file).await?;
            tracing::info!("Deleted master key from secure storage");
        }
        Ok(())
    }

    /// Check if a master key exists
    pub fn exists(&self) -> bool {
        self.key_file.exists()
    }

    /// Get the path to the key file (for debugging)
    pub fn key_file_path(&self) -> &Path {
        &self.key_file
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_master_key_lifecycle() {
        let temp_dir = TempDir::new().unwrap();
        let manager = MasterKeyManager::new(temp_dir.path());

        // Test initialization
        let key1 = manager.initialize().await.unwrap();
        assert_eq!(key1.len(), 32);
        assert!(manager.exists());

        // Test retrieval (should return same key)
        let key2 = manager.initialize().await.unwrap();
        assert_eq!(key1, key2);

        // Test deletion
        manager.delete().await.unwrap();
        assert!(!manager.exists());

        // Test regeneration after deletion
        let key3 = manager.initialize().await.unwrap();
        assert_eq!(key3.len(), 32);
        assert_ne!(key1, key3); // Should be different key
    }

    #[tokio::test]
    async fn test_file_permissions() {
        let temp_dir = TempDir::new().unwrap();
        let manager = MasterKeyManager::new(temp_dir.path());

        manager.initialize().await.unwrap();

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let metadata = std::fs::metadata(&manager.key_file_path()).unwrap();
            let mode = metadata.permissions().mode();
            assert_eq!(mode & 0o777, 0o600, "File should have 0600 permissions");
        }
    }
}
