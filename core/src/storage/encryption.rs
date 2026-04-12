use aes_gcm::{
    aead::{Aead, AeadCore, KeyInit, OsRng},
    Aes256Gcm, Nonce,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use zeroize::{Zeroize, Zeroizing};

const NIP49_HRP: &str = "ncryptsec";
const NIP49_VERSION_V2: u8 = 0x02;
const NIP49_PAYLOAD_LEN: usize = 91;
const NIP49_MIN_PASSWORD_CHARS: usize = 8;
const BECH32_CHARSET: &str = "qpzry9x8gf2tvdw0s3jn54khce6mua7l";

#[derive(Debug, Error, PartialEq, Eq)]
pub enum Nip49ValidationError {
    #[error("NIP-49 payload must start with 'ncryptsec1'")]
    InvalidPrefix,
    #[error("NIP-49 bech32 payload encoding is invalid")]
    InvalidEncoding,
    #[error("NIP-49 payload is too short")]
    TooShort,
    #[error("NIP-49 password is missing or shorter than 8 characters")]
    MissingPassword,
}

/// Validate an `ncryptsec` string structure and enforce NIP-49 version `0x02`.
pub fn validate_nip49_format(ncryptsec: &str) -> Result<(), Nip49ValidationError> {
    let payload = decode_nip49_payload(ncryptsec)?;

    if payload.len() != NIP49_PAYLOAD_LEN {
        return Err(Nip49ValidationError::InvalidEncoding);
    }

    let version = payload
        .first()
        .copied()
        .ok_or(Nip49ValidationError::InvalidEncoding)?;
    if version != NIP49_VERSION_V2 {
        return Err(Nip49ValidationError::InvalidEncoding);
    }

    Ok(())
}

/// Extract the NIP-49 version byte from an `ncryptsec` payload.
pub fn extract_nip49_version(ncryptsec: &str) -> Result<u8, Nip49ValidationError> {
    let payload = decode_nip49_payload(ncryptsec)?;
    payload
        .first()
        .copied()
        .ok_or(Nip49ValidationError::InvalidEncoding)
}

/// Validate password input for the NIP-49 normalization/encryption path.
pub fn validate_nip49_password(password: &str) -> Result<(), Nip49ValidationError> {
    if password.trim().is_empty() || password.chars().count() < NIP49_MIN_PASSWORD_CHARS {
        return Err(Nip49ValidationError::MissingPassword);
    }

    Ok(())
}

fn decode_nip49_payload(ncryptsec: &str) -> Result<Vec<u8>, Nip49ValidationError> {
    if ncryptsec.len() <= NIP49_HRP.len() + 1 {
        return Err(Nip49ValidationError::TooShort);
    }

    let has_lower = ncryptsec.chars().any(|ch| ch.is_ascii_lowercase());
    let has_upper = ncryptsec.chars().any(|ch| ch.is_ascii_uppercase());
    if has_lower && has_upper {
        return Err(Nip49ValidationError::InvalidEncoding);
    }

    let normalized = ncryptsec.to_ascii_lowercase();
    let (hrp, data_part) = normalized
        .split_once('1')
        .ok_or(Nip49ValidationError::InvalidPrefix)?;

    if hrp != NIP49_HRP {
        return Err(Nip49ValidationError::InvalidPrefix);
    }

    if data_part.len() < 6 {
        return Err(Nip49ValidationError::TooShort);
    }

    let data_values = data_part
        .chars()
        .map(bech32_char_value)
        .collect::<Result<Vec<u8>, Nip49ValidationError>>()?;

    if !verify_bech32_checksum(hrp, &data_values) {
        return Err(Nip49ValidationError::InvalidEncoding);
    }

    let payload_five_bit = &data_values[..data_values.len() - 6];
    convert_bits(payload_five_bit, 5, 8, false)
}

fn bech32_char_value(ch: char) -> Result<u8, Nip49ValidationError> {
    BECH32_CHARSET
        .chars()
        .position(|candidate| candidate == ch)
        .map(|value| value as u8)
        .ok_or(Nip49ValidationError::InvalidEncoding)
}

fn verify_bech32_checksum(hrp: &str, data: &[u8]) -> bool {
    bech32_polymod(
        bech32_hrp_expand(hrp)
            .into_iter()
            .chain(data.iter().copied()),
    ) == 1
}

fn bech32_hrp_expand(hrp: &str) -> Vec<u8> {
    let mut expanded = Vec::with_capacity(hrp.len() * 2 + 1);
    expanded.extend(hrp.bytes().map(|b| b >> 5));
    expanded.push(0);
    expanded.extend(hrp.bytes().map(|b| b & 0x1f));
    expanded
}

fn bech32_polymod(values: impl IntoIterator<Item = u8>) -> u32 {
    const GENERATOR: [u32; 5] = [
        0x3b6a57b2,
        0x2650_8e6d,
        0x1ea1_19fa,
        0x3d42_33dd,
        0x2a14_62b3,
    ];

    let mut checksum: u32 = 1;
    for value in values {
        let top = checksum >> 25;
        checksum = ((checksum & 0x01ff_ffff) << 5) ^ u32::from(value);
        for (index, item) in GENERATOR.iter().enumerate() {
            if ((top >> index) & 1) == 1 {
                checksum ^= item;
            }
        }
    }

    checksum
}

fn convert_bits(
    data: &[u8],
    from_bits: u32,
    to_bits: u32,
    pad: bool,
) -> Result<Vec<u8>, Nip49ValidationError> {
    let mut accumulator: u32 = 0;
    let mut bits: u32 = 0;
    let max_value = (1u32 << to_bits) - 1;
    let max_accumulator = (1u32 << (from_bits + to_bits - 1)) - 1;
    let mut output = Vec::new();

    for value in data {
        let value_u32 = u32::from(*value);
        if (value_u32 >> from_bits) != 0 {
            return Err(Nip49ValidationError::InvalidEncoding);
        }

        accumulator = ((accumulator << from_bits) | value_u32) & max_accumulator;
        bits += from_bits;

        while bits >= to_bits {
            bits -= to_bits;
            output.push(((accumulator >> bits) & max_value) as u8);
        }
    }

    if pad {
        if bits > 0 {
            output.push(((accumulator << (to_bits - bits)) & max_value) as u8);
        }
    } else if bits >= from_bits || ((accumulator << (to_bits - bits)) & max_value) != 0 {
        return Err(Nip49ValidationError::InvalidEncoding);
    }

    Ok(output)
}

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

    fn create_bech32_checksum(hrp: &str, data: &[u8]) -> Vec<u8> {
        let values = bech32_hrp_expand(hrp)
            .into_iter()
            .chain(data.iter().copied())
            .chain([0u8; 6])
            .collect::<Vec<u8>>();

        let polymod = bech32_polymod(values) ^ 1;
        (0..6)
            .map(|idx| ((polymod >> (5 * (5 - idx))) & 0x1f) as u8)
            .collect()
    }

    fn build_valid_nip49_with_version(version: u8) -> String {
        let mut payload = vec![0u8; NIP49_PAYLOAD_LEN];
        payload[0] = version;

        let data = convert_bits(&payload, 8, 5, true).expect("5-bit conversion should work");
        let checksum = create_bech32_checksum(NIP49_HRP, &data);

        let encoded_data = data
            .into_iter()
            .chain(checksum)
            .map(|value| BECH32_CHARSET.as_bytes()[value as usize] as char)
            .collect::<String>();

        format!("{NIP49_HRP}1{encoded_data}")
    }

    #[test]
    fn nip49_validate_format_accepts_valid_v2_payload() {
        let valid = build_valid_nip49_with_version(NIP49_VERSION_V2);
        assert_eq!(validate_nip49_format(&valid), Ok(()));
    }

    #[test]
    fn nip49_validate_format_rejects_wrong_hrp() {
        let valid = build_valid_nip49_with_version(NIP49_VERSION_V2);
        let invalid = valid.replacen("ncryptsec", "nsec", 1);

        assert_eq!(
            validate_nip49_format(&invalid),
            Err(Nip49ValidationError::InvalidPrefix)
        );
    }

    #[test]
    fn nip49_validate_format_rejects_unsupported_version() {
        let unsupported = build_valid_nip49_with_version(0x01);

        assert_eq!(
            validate_nip49_format(&unsupported),
            Err(Nip49ValidationError::InvalidEncoding)
        );
    }

    #[test]
    fn nip49_validate_format_rejects_too_short_payload() {
        assert_eq!(
            validate_nip49_format("ncryptsec1"),
            Err(Nip49ValidationError::TooShort)
        );
    }

    #[test]
    fn nip49_extract_version_returns_first_payload_byte() {
        let encoded = build_valid_nip49_with_version(NIP49_VERSION_V2);
        assert_eq!(extract_nip49_version(&encoded), Ok(NIP49_VERSION_V2));
    }

    #[test]
    fn nip49_validate_password_rejects_invalid_inputs() {
        assert_eq!(
            validate_nip49_password(""),
            Err(Nip49ValidationError::MissingPassword)
        );
        assert_eq!(
            validate_nip49_password("   \n\t"),
            Err(Nip49ValidationError::MissingPassword)
        );
        assert_eq!(
            validate_nip49_password("short"),
            Err(Nip49ValidationError::MissingPassword)
        );
    }

    #[test]
    fn nip49_validate_password_accepts_regular_unicode_password() {
        assert_eq!(validate_nip49_password("Pąßwørd 123"), Ok(()));
    }
}
