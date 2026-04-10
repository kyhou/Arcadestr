#![cfg(not(target_arch = "wasm32"))]

use crate::signers::{NostrSigner, SignerError};
use async_trait::async_trait;
use nostr::{Event, Keys, ToBech32, UnsignedEvent};
use zeroize::Zeroizing;

/// Local signer that uses a stored nsec for signing
///
/// This is the fast-path signer that enables ~4 second login times
/// by avoiding NIP-46 reconnection overhead.
///
/// Note: This is native-only (requires sqlx, encryption)
pub struct LocalSigner {
    keys: Keys,
    pubkey_hex: String,
}

impl LocalSigner {
    /// Create a LocalSigner from an nsec string
    pub fn from_nsec(nsec: &str) -> Result<Self, SignerError> {
        let keys = Keys::parse(nsec).map_err(|e| SignerError::SigningFailed(e.to_string()))?;
        let pubkey_hex = keys.public_key().to_hex();

        Ok(Self { keys, pubkey_hex })
    }

    /// Create a LocalSigner from a hex private key
    pub fn from_hex(hex: &str) -> Result<Self, SignerError> {
        let secret_key = nostr::SecretKey::from_hex(hex)
            .map_err(|e| SignerError::SigningFailed(e.to_string()))?;
        let keys = Keys::new(secret_key);
        let pubkey_hex = keys.public_key().to_hex();

        Ok(Self { keys, pubkey_hex })
    }

    /// Get a reference to the internal Keys
    pub fn keys(&self) -> &Keys {
        &self.keys
    }

    /// Get the nsec representation (for backup/export)
    pub fn to_nsec(&self) -> Zeroizing<String> {
        Zeroizing::new(self.keys.secret_key().to_bech32().unwrap_or_default())
    }
}

#[async_trait]
impl NostrSigner for LocalSigner {
    async fn sign_event(&self, event: UnsignedEvent) -> Result<Event, SignerError> {
        let signed = event
            .sign(&self.keys)
            .await
            .map_err(|e| SignerError::SigningFailed(e.to_string()))?;
        Ok(signed)
    }

    async fn get_public_key(&self) -> Result<nostr::PublicKey, SignerError> {
        Ok(self.keys.public_key())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::{EventBuilder, Kind, Keys};

    fn build_unsigned_text_note(pubkey: nostr::PublicKey, content: &str) -> UnsignedEvent {
        EventBuilder::new(Kind::TextNote, content).build(pubkey)
    }

    #[tokio::test]
    async fn local_signer_public_key_matches_private_key() {
        let keys = Keys::generate();
        let nsec = keys
            .secret_key()
            .to_bech32()
            .expect("test secret key should encode to nsec");

        let signer = LocalSigner::from_nsec(&nsec).expect("nsec should create local signer");
        let signer_pubkey = signer
            .get_public_key()
            .await
            .expect("local signer should return public key");

        assert_eq!(signer_pubkey, keys.public_key());
    }

    #[tokio::test]
    async fn local_signer_sign_produces_valid_event() {
        let keys = Keys::generate();
        let signer = LocalSigner::from_hex(&keys.secret_key().to_secret_hex())
            .expect("hex secret should create local signer");

        let signer_pubkey = signer
            .get_public_key()
            .await
            .expect("local signer should return public key");
        let unsigned = build_unsigned_text_note(signer_pubkey, "signer abstraction test");

        let signed = signer
            .sign_event(unsigned)
            .await
            .expect("local signer should sign event");

        assert!(signed.verify().is_ok());
    }

    #[tokio::test]
    async fn unsigned_event_is_rejected() {
        let keys = Keys::generate();
        let signer = LocalSigner::from_hex(&keys.secret_key().to_secret_hex())
            .expect("hex secret should create local signer");

        let signer_pubkey = signer
            .get_public_key()
            .await
            .expect("local signer should return public key");
        let unsigned = build_unsigned_text_note(signer_pubkey, "unsigned should fail verification");

        let signed = signer
            .sign_event(unsigned)
            .await
            .expect("local signer should sign event");

        let mut tampered = serde_json::to_value(&signed).expect("signed event should serialize");
        tampered["sig"] = serde_json::Value::String("0".repeat(128));
        let invalid: Event =
            serde_json::from_value(tampered).expect("tampered event should deserialize");

        assert!(invalid.verify().is_err());
    }

    #[tokio::test]
    async fn sign_event_sets_pubkey() {
        let keys = Keys::generate();
        let signer = LocalSigner::from_hex(&keys.secret_key().to_secret_hex())
            .expect("hex secret should create local signer");

        let signer_pubkey = signer
            .get_public_key()
            .await
            .expect("local signer should return public key");
        let unsigned = build_unsigned_text_note(signer_pubkey, "pubkey should match signer");

        let signed = signer
            .sign_event(unsigned)
            .await
            .expect("local signer should sign event");

        assert_eq!(signed.pubkey, signer_pubkey);
    }
}
