use aes_gcm::{
    aead::{Aead, AeadCore, KeyInit, OsRng},
    Aes256Gcm, Nonce,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use zeroize::{Zeroize, Zeroizing};

/// AES-256-GCM encryption for nsec storage
#[derive(Clone)]
pub struct Encryption {
    cipher: Aes256Gcm,
}

/// Encrypted data structure with nonce
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EncryptedData {
    pub nonce: Vec<u8>,
    pub ciphertext: Vec<u8>,
}

#[derive(Debug, Error)]
pub enum EncryptionError {
    #[error("Decryption failed - invalid key or corrupted data")]
    DecryptionFailed,
    #[error("Invalid UTF-8 in decrypted data")]
    InvalidUtf8,
    #[error("Serialization failed: {0}")]
    Serialization(String),
    #[error("Invalid key length: expected 32 bytes, got {0}")]
    InvalidKeyLength(usize),
}

impl Encryption {
    /// Create new encryption instance with master key
    ///
    /// # Arguments
    /// * `master_key` - 32-byte (256-bit) master key from OS keychain
    ///
    /// # Panics
    /// Panics if master_key is not exactly 32 bytes
    pub fn new(master_key: &[u8]) -> Result<Self, EncryptionError> {
        if master_key.len() != 32 {
            return Err(EncryptionError::InvalidKeyLength(master_key.len()));
        }

        let cipher = Aes256Gcm::new_from_slice(master_key)
            .map_err(|_| EncryptionError::InvalidKeyLength(master_key.len()))?;

        Ok(Self { cipher })
    }

    /// Encrypt nsec with AES-256-GCM
    ///
    /// # Arguments
    /// * `nsec` - The nsec string to encrypt
    ///
    /// # Returns
    /// Encrypted data structure containing nonce and ciphertext
    pub fn encrypt_nsec(&self, nsec: &str) -> EncryptedData {
        let nonce = Aes256Gcm::generate_nonce(&mut OsRng);

        let ciphertext = self
            .cipher
            .encrypt(&nonce, nsec.as_bytes())
            .expect("Encryption failed - this should never happen with valid inputs");

        EncryptedData {
            nonce: nonce.to_vec(),
            ciphertext,
        }
    }

    /// Decrypt nsec with AES-256-GCM
    ///
    /// # Arguments
    /// * `data` - The encrypted data structure
    ///
    /// # Returns
    /// Zeroizing string containing the decrypted nsec
    pub fn decrypt_nsec(&self, data: &EncryptedData) -> Result<Zeroizing<String>, EncryptionError> {
        let nonce = Nonce::from_slice(&data.nonce);

        let plaintext = self
            .cipher
            .decrypt(nonce, data.ciphertext.as_ref())
            .map_err(|_| EncryptionError::DecryptionFailed)?;

        let s = String::from_utf8(plaintext).map_err(|_| EncryptionError::InvalidUtf8)?;

        Ok(Zeroizing::new(s))
    }

    /// Encrypt arbitrary data with AES-256-GCM
    ///
    /// # Arguments
    /// * `data` - The data to encrypt
    ///
    /// # Returns
    /// Encrypted data structure containing nonce and ciphertext
    pub fn encrypt(&self, data: &[u8]) -> EncryptedData {
        let nonce = Aes256Gcm::generate_nonce(&mut OsRng);

        let ciphertext = self
            .cipher
            .encrypt(&nonce, data)
            .expect("Encryption failed - this should never happen with valid inputs");

        EncryptedData {
            nonce: nonce.to_vec(),
            ciphertext,
        }
    }

    /// Decrypt arbitrary data with AES-256-GCM
    ///
    /// # Arguments
    /// * `data` - The encrypted data structure
    ///
    /// # Returns
    /// Decrypted bytes
    pub fn decrypt(&self, data: &EncryptedData) -> Result<Vec<u8>, EncryptionError> {
        let nonce = Nonce::from_slice(&data.nonce);

        let plaintext = self
            .cipher
            .decrypt(nonce, data.ciphertext.as_ref())
            .map_err(|_| EncryptionError::DecryptionFailed)?;

        Ok(plaintext)
    }

    /// Serialize encrypted data to bytes
    pub fn serialize(data: &EncryptedData) -> Result<Vec<u8>, EncryptionError> {
        serde_json::to_vec(data).map_err(|e| EncryptionError::Serialization(e.to_string()))
    }

    /// Deserialize encrypted data from bytes
    pub fn deserialize(bytes: &[u8]) -> Result<EncryptedData, EncryptionError> {
        serde_json::from_slice(bytes).map_err(|e| EncryptionError::Serialization(e.to_string()))
    }
}

impl Drop for Encryption {
    fn drop(&mut self) {
        // Note: aes-gcm doesn't expose key clearing
        // The cipher will be dropped normally
        // For additional security, we rely on process memory isolation
    }
}

// Zeroize the EncryptedData when dropped
impl Zeroize for EncryptedData {
    fn zeroize(&mut self) {
        self.nonce.zeroize();
        self.ciphertext.zeroize();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encrypt_decrypt_roundtrip() {
        let master_key = [0u8; 32]; // Test key (all zeros - don't use in production!)
        let encryption = Encryption::new(&master_key).unwrap();

        let nsec = "nsec1...test...";
        let encrypted = encryption.encrypt_nsec(nsec);

        // Verify structure
        assert_eq!(encrypted.nonce.len(), 12); // GCM nonce is 12 bytes
        assert!(!encrypted.ciphertext.is_empty());

        // Decrypt and verify
        let decrypted = encryption.decrypt_nsec(&encrypted).unwrap();
        assert_eq!(*decrypted, nsec);
    }

    #[test]
    fn nonce_is_unique_per_encryption() {
        let master_key = [0u8; 32];
        let encryption = Encryption::new(&master_key).unwrap();

        let nsec = "nsec1...test...";
        let encrypted1 = encryption.encrypt_nsec(nsec);
        let encrypted2 = encryption.encrypt_nsec(nsec);

        // Same plaintext should produce different ciphertext (different nonces)
        assert_ne!(encrypted1.nonce, encrypted2.nonce);
        assert_ne!(encrypted1.ciphertext, encrypted2.ciphertext);

        // But both should decrypt to same value
        assert_eq!(*encryption.decrypt_nsec(&encrypted1).unwrap(), nsec);
        assert_eq!(*encryption.decrypt_nsec(&encrypted2).unwrap(), nsec);
    }

    #[test]
    fn different_keys_cannot_decrypt() {
        let key_a = [1u8; 32];
        let key_b = [2u8; 32];
        let encryption_a = Encryption::new(&key_a).expect("key A should be valid");
        let encryption_b = Encryption::new(&key_b).expect("key B should be valid");

        let encrypted = encryption_a.encrypt_nsec("nsec1test");
        let result = encryption_b.decrypt_nsec(&encrypted);

        assert!(matches!(result, Err(EncryptionError::DecryptionFailed)));
    }

    #[test]
    fn ciphertext_differs_from_plaintext() {
        let master_key = [3u8; 32];
        let encryption = Encryption::new(&master_key).expect("key should be valid");
        let plaintext = b"plain-text";

        let encrypted = encryption.encrypt(plaintext);

        assert_ne!(encrypted.ciphertext.as_slice(), plaintext);
    }

    #[test]
    fn test_invalid_key_length() {
        let short_key = [0u8; 16];
        let result = Encryption::new(&short_key);
        assert!(matches!(result, Err(EncryptionError::InvalidKeyLength(16))));

        let long_key = [0u8; 64];
        let result = Encryption::new(&long_key);
        assert!(matches!(result, Err(EncryptionError::InvalidKeyLength(64))));
    }

    #[test]
    fn test_corrupted_data() {
        let master_key = [0u8; 32];
        let encryption = Encryption::new(&master_key).unwrap();

        let nsec = "nsec1...test...";
        let mut encrypted = encryption.encrypt_nsec(nsec);

        // Corrupt the ciphertext
        if let Some(byte) = encrypted.ciphertext.first_mut() {
            *byte ^= 0xFF; // Flip all bits in first byte
        }

        // Decryption should fail
        let result = encryption.decrypt_nsec(&encrypted);
        assert!(matches!(result, Err(EncryptionError::DecryptionFailed)));
    }

    #[test]
    fn test_serialization() {
        let master_key = [0u8; 32];
        let encryption = Encryption::new(&master_key).unwrap();

        let nsec = "nsec1...test...";
        let encrypted = encryption.encrypt_nsec(nsec);

        // Serialize
        let bytes = Encryption::serialize(&encrypted).unwrap();

        // Deserialize
        let deserialized = Encryption::deserialize(&bytes).unwrap();

        // Verify decryption still works
        let decrypted = encryption.decrypt_nsec(&deserialized).unwrap();
        assert_eq!(*decrypted, nsec);
    }
}
