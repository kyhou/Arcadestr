use aes_gcm::{
    aead::{Aead, AeadCore, KeyInit, OsRng},
    Aes256Gcm, Nonce,
};
use chacha20poly1305::{aead::Payload, XChaCha20Poly1305, XNonce};
use rand::TryRng;
use scrypt::Params as ScryptKdfParams;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use unicode_normalization::UnicodeNormalization;
use zeroize::{Zeroize, Zeroizing};

const NIP49_HRP: &str = "ncryptsec";
const NIP49_VERSION_V2: u8 = 0x02;
const NIP49_PAYLOAD_LEN: usize = 91;
const NIP49_SALT_LEN: usize = 16;
const NIP49_NONCE_LEN: usize = 24;
const NIP49_PRIVATE_KEY_LEN: usize = 32;
const NIP49_KEY_SECURITY_UNKNOWN: u8 = 0x02;
const NIP49_KEY_DERIVATION_LEN: usize = 32;
const NIP49_SCRYPT_R: u32 = 8;
const NIP49_SCRYPT_P: u32 = 1;
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
    #[error("NIP-49 bech32 validation failed: {0}")]
    Nip49Validation(#[from] Nip49ValidationError),
    #[error("NIP-49 unsupported version: 0x{0:02x}")]
    Nip49UnsupportedVersion(u8),
    #[error("NIP-49 payload length mismatch: expected 91 bytes, got {0}")]
    Nip49InvalidPayloadLength(usize),
    #[error("NIP-49 private key must be 32 bytes, got {0}")]
    Nip49InvalidPrivateKeyLength(usize),
    #[error("NIP-49 private key must be valid hex: {0}")]
    Nip49InvalidPrivateKeyHex(String),
    #[error("NIP-49 key derivation params are invalid")]
    Nip49InvalidKdfParams,
    #[error("NIP-49 key derivation output length is invalid")]
    Nip49InvalidKdfOutputLength,
    #[error("NIP-49 random generation failed")]
    Nip49RandomGenerationFailed,
    #[error("NIP-49 cipher init failed")]
    Nip49CipherInit,
    #[error("NIP-49 encryption failed")]
    Nip49EncryptionFailed,
    #[error("NIP-49 decryption failed")]
    Nip49DecryptionFailed,
}

/// Parsed NIP-49 `ncryptsec` envelope.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ncryptsec {
    pub version: u8,
    pub scrypt_n: u32,
    pub scrypt_r: u32,
    pub scrypt_p: u32,
    pub salt: [u8; 16],
    pub nonce: [u8; 24],
    pub ciphertext: Vec<u8>,
}

/// Scrypt parameters for NIP-49 key derivation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScryptParams {
    /// Linear scrypt cost parameter N. Must be a power of two.
    pub n: u32,
    /// Scrypt block size (NIP-49 uses 8).
    pub r: u32,
    /// Scrypt parallelization parameter (NIP-49 uses 1).
    pub p: u32,
}

impl ScryptParams {
    /// Build default NIP-49-compatible params (`N=131072`, `r=8`, `p=1`).
    pub fn default_nip49() -> Self {
        Self {
            n: 1 << 17,
            r: NIP49_SCRYPT_R,
            p: NIP49_SCRYPT_P,
        }
    }

    /// Build fast test params (`N=1024`, `r=8`, `p=1`).
    pub fn for_testing() -> Self {
        Self {
            n: 1 << 10,
            r: NIP49_SCRYPT_R,
            p: NIP49_SCRYPT_P,
        }
    }

    fn to_scrypt_params(self) -> Result<ScryptKdfParams, EncryptionError> {
        let log_n = scrypt_n_to_log_n(self.n)?;
        ScryptKdfParams::new(log_n, self.r, self.p, NIP49_KEY_DERIVATION_LEN)
            .map_err(|_| EncryptionError::Nip49InvalidKdfParams)
    }
}

fn scrypt_n_to_log_n(n: u32) -> Result<u8, EncryptionError> {
    if n == 0 || !n.is_power_of_two() {
        return Err(EncryptionError::Nip49InvalidKdfParams);
    }

    let log_n = n.ilog2();
    u8::try_from(log_n).map_err(|_| EncryptionError::Nip49InvalidKdfParams)
}

/// Derive a 32-byte NIP-49 encryption key using scrypt.
pub fn derive_key_scrypt(
    password: &str,
    salt: &[u8; NIP49_SALT_LEN],
    params: &ScryptParams,
) -> Result<[u8; NIP49_KEY_DERIVATION_LEN], EncryptionError> {
    let normalized_password = Zeroizing::new(password.nfkc().collect::<String>());
    let mut derived_key = [0u8; NIP49_KEY_DERIVATION_LEN];
    let scrypt_params = params.to_scrypt_params()?;

    scrypt::scrypt(
        normalized_password.as_bytes(),
        salt,
        &scrypt_params,
        &mut derived_key,
    )
    .map_err(|_| EncryptionError::Nip49InvalidKdfOutputLength)?;

    Ok(derived_key)
}

/// Encrypt a hex-encoded 32-byte private key into a typed `Ncryptsec` envelope.
pub fn encrypt_private_key_nip49(
    private_key_hex: &str,
    password: &str,
    params: Option<ScryptParams>,
) -> Result<Ncryptsec, EncryptionError> {
    let params = params.unwrap_or_else(ScryptParams::default_nip49);
    let private_key = parse_private_key_hex(private_key_hex)?;

    let mut salt = [0u8; NIP49_SALT_LEN];
    rand::rng()
        .try_fill_bytes(&mut salt)
        .map_err(|_| EncryptionError::Nip49RandomGenerationFailed)?;

    let derived_key = derive_key_scrypt(password, &salt, &params)?;

    let mut nonce = [0u8; NIP49_NONCE_LEN];
    rand::rng()
        .try_fill_bytes(&mut nonce)
        .map_err(|_| EncryptionError::Nip49RandomGenerationFailed)?;

    let key_security_byte = NIP49_KEY_SECURITY_UNKNOWN;
    let cipher = XChaCha20Poly1305::new_from_slice(&derived_key)
        .map_err(|_| EncryptionError::Nip49CipherInit)?;
    let ciphertext = cipher
        .encrypt(
            XNonce::from_slice(&nonce),
            Payload {
                msg: &private_key,
                aad: &[key_security_byte],
            },
        )
        .map_err(|_| EncryptionError::Nip49EncryptionFailed)?;

    Ok(Ncryptsec {
        version: NIP49_VERSION_V2,
        scrypt_n: params.n,
        scrypt_r: params.r,
        scrypt_p: params.p,
        salt,
        nonce,
        ciphertext,
    })
}

/// Decrypt an `Ncryptsec` envelope into a lowercase hex private key.
pub fn decrypt_private_key_nip49(
    ncryptsec: &Ncryptsec,
    password: &str,
) -> Result<String, EncryptionError> {
    if ncryptsec.version != NIP49_VERSION_V2 {
        return Err(EncryptionError::Nip49UnsupportedVersion(ncryptsec.version));
    }

    let params = ScryptParams {
        n: ncryptsec.scrypt_n,
        r: ncryptsec.scrypt_r,
        p: ncryptsec.scrypt_p,
    };
    let derived_key = derive_key_scrypt(password, &ncryptsec.salt, &params)?;
    let cipher = XChaCha20Poly1305::new_from_slice(&derived_key)
        .map_err(|_| EncryptionError::Nip49CipherInit)?;

    let private_key = cipher
        .decrypt(
            XNonce::from_slice(&ncryptsec.nonce),
            Payload {
                msg: ncryptsec.ciphertext.as_slice(),
                aad: &[NIP49_KEY_SECURITY_UNKNOWN],
            },
        )
        .map_err(|_| EncryptionError::Nip49DecryptionFailed)?;

    if private_key.len() != NIP49_PRIVATE_KEY_LEN {
        return Err(EncryptionError::Nip49InvalidPrivateKeyLength(
            private_key.len(),
        ));
    }

    Ok(hex::encode(private_key))
}

/// Parse a bech32-encoded `ncryptsec` string into a typed envelope.
pub fn parse_ncryptsec(ncryptsec_str: &str) -> Result<Ncryptsec, EncryptionError> {
    let payload_bytes = decode_nip49_payload(ncryptsec_str)?;
    let payload = Nip49Payload::from_bytes(&payload_bytes)?;
    let scrypt_n = 1u32
        .checked_shl(u32::from(payload.log_n))
        .ok_or(EncryptionError::Nip49InvalidKdfParams)?;

    Ok(Ncryptsec {
        version: payload.version,
        scrypt_n,
        scrypt_r: NIP49_SCRYPT_R,
        scrypt_p: NIP49_SCRYPT_P,
        salt: payload.salt,
        nonce: payload.nonce,
        ciphertext: payload.ciphertext,
    })
}

/// Serialize an `Ncryptsec` envelope into bech32 (`ncryptsec1...`).
pub fn serialize_ncryptsec(ncryptsec: &Ncryptsec) -> Result<String, EncryptionError> {
    let log_n = scrypt_n_to_log_n(ncryptsec.scrypt_n)?;
    let payload = Nip49Payload::new_v2(
        log_n,
        ncryptsec.salt,
        ncryptsec.nonce,
        NIP49_KEY_SECURITY_UNKNOWN,
        ncryptsec.ciphertext.clone(),
    )
    .into_bytes();

    Ok(encode_nip49_payload(&payload))
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Nip49Payload {
    version: u8,
    log_n: u8,
    salt: [u8; NIP49_SALT_LEN],
    nonce: [u8; NIP49_NONCE_LEN],
    key_security_byte: u8,
    ciphertext: Vec<u8>,
}

impl Nip49Payload {
    fn new_v2(
        log_n: u8,
        salt: [u8; NIP49_SALT_LEN],
        nonce: [u8; NIP49_NONCE_LEN],
        key_security_byte: u8,
        ciphertext: Vec<u8>,
    ) -> Self {
        Self {
            version: NIP49_VERSION_V2,
            log_n,
            salt,
            nonce,
            key_security_byte,
            ciphertext,
        }
    }

    fn from_bytes(payload: &[u8]) -> Result<Self, EncryptionError> {
        if payload.len() != NIP49_PAYLOAD_LEN {
            return Err(EncryptionError::Nip49InvalidPayloadLength(payload.len()));
        }

        let version = payload[0];
        if version != NIP49_VERSION_V2 {
            return Err(EncryptionError::Nip49UnsupportedVersion(version));
        }

        let mut salt = [0u8; NIP49_SALT_LEN];
        salt.copy_from_slice(&payload[2..18]);

        let mut nonce = [0u8; NIP49_NONCE_LEN];
        nonce.copy_from_slice(&payload[18..42]);

        Ok(Self {
            version,
            log_n: payload[1],
            salt,
            nonce,
            key_security_byte: payload[42],
            ciphertext: payload[43..].to_vec(),
        })
    }

    fn into_bytes(self) -> Vec<u8> {
        let mut payload = Vec::with_capacity(NIP49_PAYLOAD_LEN);
        payload.push(self.version);
        payload.push(self.log_n);
        payload.extend_from_slice(&self.salt);
        payload.extend_from_slice(&self.nonce);
        payload.push(self.key_security_byte);
        payload.extend_from_slice(&self.ciphertext);
        payload
    }
}

fn parse_private_key_hex(
    private_key_hex: &str,
) -> Result<[u8; NIP49_PRIVATE_KEY_LEN], EncryptionError> {
    let key_bytes = hex::decode(private_key_hex)
        .map_err(|error| EncryptionError::Nip49InvalidPrivateKeyHex(error.to_string()))?;

    if key_bytes.len() != NIP49_PRIVATE_KEY_LEN {
        return Err(EncryptionError::Nip49InvalidPrivateKeyLength(
            key_bytes.len(),
        ));
    }

    let mut private_key = [0u8; NIP49_PRIVATE_KEY_LEN];
    private_key.copy_from_slice(&key_bytes);
    Ok(private_key)
}

fn encode_nip49_payload(payload: &[u8]) -> String {
    let data = convert_bits_8_to_5_padded(payload);
    let checksum = create_bech32_checksum(NIP49_HRP, &data);

    let encoded_data = data
        .into_iter()
        .chain(checksum)
        .map(|value| BECH32_CHARSET.as_bytes()[value as usize] as char)
        .collect::<String>();

    format!("{NIP49_HRP}1{encoded_data}")
}

fn convert_bits_8_to_5_padded(data: &[u8]) -> Vec<u8> {
    let mut accumulator: u32 = 0;
    let mut bits: u32 = 0;
    let max_value = (1u32 << 5) - 1;
    let mut output = Vec::new();

    for value in data {
        accumulator = (accumulator << 8) | u32::from(*value);
        bits += 8;

        while bits >= 5 {
            bits -= 5;
            output.push(((accumulator >> bits) & max_value) as u8);
        }
    }

    if bits > 0 {
        output.push(((accumulator << (5 - bits)) & max_value) as u8);
    }

    output
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

    #[test]
    fn nip49_scrypt_derivation_is_deterministic() {
        let params = ScryptParams::for_testing();
        let salt = [0x11u8; 16];

        let key1 = derive_key_scrypt("Password123", &salt, &params)
            .expect("key derivation should succeed");
        let key2 = derive_key_scrypt("Password123", &salt, &params)
            .expect("key derivation should succeed");

        assert_eq!(key1, key2);
    }

    #[test]
    fn nip49_encrypt_decrypt_roundtrip() {
        let params = ScryptParams::for_testing();
        let private_key_hex = "3501454135014541350145413501453fefb02227e449e57cf4d3a3ce05378683";

        let ncryptsec = encrypt_private_key_nip49(private_key_hex, "nostr-password", Some(params))
            .expect("encryption should succeed");

        let serialized =
            serialize_ncryptsec(&ncryptsec).expect("serialize should succeed for valid envelope");

        assert!(serialized.starts_with("ncryptsec1"));
        assert_eq!(extract_nip49_version(&serialized), Ok(NIP49_VERSION_V2));

        let parsed = parse_ncryptsec(&serialized).expect("parse should succeed");

        let decrypted = decrypt_private_key_nip49(&parsed, "nostr-password")
            .expect("decryption should succeed");

        assert_eq!(decrypted, private_key_hex);
    }

    #[test]
    fn nip49_serialize_parse_roundtrip_preserves_binary_fields() {
        let mut salt = [0u8; NIP49_SALT_LEN];
        for (idx, byte) in salt.iter_mut().enumerate() {
            *byte = idx as u8;
        }

        let mut nonce = [0u8; NIP49_NONCE_LEN];
        for (idx, byte) in nonce.iter_mut().enumerate() {
            *byte = (idx as u8).wrapping_add(16);
        }

        let ciphertext = (0..48).map(|value| value as u8).collect::<Vec<u8>>();
        let envelope = Ncryptsec {
            version: 0x7f,
            scrypt_n: 1 << 15,
            scrypt_r: 999,
            scrypt_p: 999,
            salt,
            nonce,
            ciphertext,
        };

        let serialized =
            serialize_ncryptsec(&envelope).expect("serialize should succeed for valid envelope");
        assert!(serialized.starts_with("ncryptsec1"));
        let parsed = parse_ncryptsec(&serialized).expect("parse should succeed");

        assert_eq!(parsed.version, NIP49_VERSION_V2);
        assert_eq!(parsed.scrypt_n, 1 << 15);
        assert_eq!(parsed.scrypt_r, NIP49_SCRYPT_R);
        assert_eq!(parsed.scrypt_p, NIP49_SCRYPT_P);
        assert_eq!(parsed.salt, envelope.salt);
        assert_eq!(parsed.nonce, envelope.nonce);
        assert_eq!(parsed.ciphertext, envelope.ciphertext);
    }

    #[test]
    fn nip49_serialize_with_invalid_scrypt_n_returns_error() {
        let envelope = Ncryptsec {
            version: NIP49_VERSION_V2,
            scrypt_n: 0,
            scrypt_r: NIP49_SCRYPT_R,
            scrypt_p: NIP49_SCRYPT_P,
            salt: [0x11; NIP49_SALT_LEN],
            nonce: [0x22; NIP49_NONCE_LEN],
            ciphertext: vec![0x33; 48],
        };

        let serialized_result = serialize_ncryptsec(&envelope);
        assert!(matches!(
            serialized_result,
            Err(EncryptionError::Nip49InvalidKdfParams)
        ));
    }

    #[test]
    fn nip49_parse_rejects_wrong_hrp() {
        let valid = build_valid_nip49_with_version(NIP49_VERSION_V2);
        let invalid = valid.replacen("ncryptsec", "nsec", 1);

        let result = parse_ncryptsec(&invalid);
        assert!(matches!(
            result,
            Err(EncryptionError::Nip49Validation(
                Nip49ValidationError::InvalidPrefix
            ))
        ));
    }

    #[test]
    fn nip49_parse_rejects_unsupported_version_byte() {
        let unsupported = build_valid_nip49_with_version(0x01);

        let result = parse_ncryptsec(&unsupported);
        assert!(matches!(
            result,
            Err(EncryptionError::Nip49UnsupportedVersion(0x01))
        ));
    }

    #[test]
    fn nip49_decrypt_fails_with_wrong_password() {
        let params = ScryptParams::for_testing();
        let private_key_hex = "3501454135014541350145413501453fefb02227e449e57cf4d3a3ce05378683";

        let ncryptsec =
            encrypt_private_key_nip49(private_key_hex, "correct-password", Some(params))
                .expect("encryption should succeed");

        let result = decrypt_private_key_nip49(&ncryptsec, "wrong-password");
        assert!(matches!(
            result,
            Err(EncryptionError::Nip49DecryptionFailed)
        ));
    }

    #[test]
    fn nip49_scrypt_derivation_rejects_non_power_of_two_n() {
        let params = ScryptParams {
            n: 1000,
            r: NIP49_SCRYPT_R,
            p: NIP49_SCRYPT_P,
        };
        let salt = [0x11u8; 16];

        let result = derive_key_scrypt("Password123", &salt, &params);
        assert!(matches!(
            result,
            Err(EncryptionError::Nip49InvalidKdfParams)
        ));
    }
}
