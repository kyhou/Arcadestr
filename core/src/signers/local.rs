#![cfg(not(target_arch = "wasm32"))]

use crate::signers::{NostrSigner, SignerError};
use nostr::{Event, Keys, ToBech32, UnsignedEvent};
use async_trait::async_trait;
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
        let keys = Keys::parse(nsec)
            .map_err(|e| SignerError::SigningFailed(e.to_string()))?;
        let pubkey_hex = keys.public_key().to_hex();
        
        Ok(Self {
            keys,
            pubkey_hex,
        })
    }
    
    /// Create a LocalSigner from a hex private key
    pub fn from_hex(hex: &str) -> Result<Self, SignerError> {
        let secret_key = nostr::SecretKey::from_hex(hex)
            .map_err(|e| SignerError::SigningFailed(e.to_string()))?;
        let keys = Keys::new(secret_key);
        let pubkey_hex = keys.public_key().to_hex();
        
        Ok(Self {
            keys,
            pubkey_hex,
        })
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
        let signed = event.sign(&self.keys).await
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
    
    #[test]
    fn test_local_signer_from_nsec() {
        // This is a test key - DO NOT USE IN PRODUCTION
        let nsec = "nsec1...test...";
        
        // For actual testing, we'd need a valid nsec
        // This test is a placeholder for the structure
    }
}
