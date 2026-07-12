//! NIP-98 authentication helpers for ADP HTTP calls.

use base64::Engine;
use nostr::{EventBuilder, Kind, Tag, TagKind};
use thiserror::Error;

use crate::signers::{NostrSigner, SignerError};

/// Errors returned while building NIP-98 authentication headers.
#[derive(Debug, Error)]
pub enum Nip98ClientError {
    /// The active signer could not sign the NIP-98 event.
    #[error("failed to sign NIP-98 event: {0}")]
    Signer(#[from] SignerError),

    /// The signed NIP-98 event could not be encoded as JSON.
    #[error("failed to serialize NIP-98 event: {0}")]
    Serialize(#[from] serde_json::Error),
}

/// Builds an `Authorization` header value for a NIP-98 authenticated request.
///
/// The returned value includes the `Nostr ` scheme prefix expected by ADP servers.
///
/// # Errors
/// Returns [`Nip98ClientError`] if signing or event serialization fails.
pub async fn build_nip98_auth_header(
    signer: &dyn NostrSigner,
    url: &str,
    method: &str,
) -> Result<String, Nip98ClientError> {
    let pubkey = signer.get_public_key().await?;
    let method = method.to_ascii_uppercase();
    let unsigned = EventBuilder::new(Kind::Custom(27235), "")
        .tags([
            Tag::custom(TagKind::Custom("u".into()), [url.to_string()]),
            Tag::custom(TagKind::Custom("method".into()), [method]),
        ])
        .build(pubkey);
    let event = signer.sign_event(unsigned).await?;
    let json = serde_json::to_vec(&event)?;
    let encoded = base64::engine::general_purpose::STANDARD.encode(json);

    Ok(format!("Nostr {encoded}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine;
    use serde_json::Value;

    use crate::signers::{LocalSigner, NostrSigner};

    #[tokio::test]
    async fn build_nip98_auth_header_encodes_signed_event_with_url_and_method_tags() {
        let signer = LocalSigner::from_hex(
            "0000000000000000000000000000000000000000000000000000000000000001",
        )
        .expect("test private key should be valid");

        let header = build_nip98_auth_header(&signer, "https://dist.example.com/provision", "post")
            .await
            .expect("header should build");

        let encoded = header
            .strip_prefix("Nostr ")
            .expect("header should use Nostr auth scheme");
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(encoded)
            .expect("header should contain base64 event json");
        let event: Value = serde_json::from_slice(&decoded).expect("event should be json");

        assert_eq!(event["kind"], 27235);
        assert_eq!(
            event["pubkey"],
            signer.get_public_key().await.unwrap().to_hex()
        );
        assert!(event["sig"].as_str().is_some_and(|sig| !sig.is_empty()));
        assert_eq!(
            event["tags"],
            serde_json::json!([
                ["u", "https://dist.example.com/provision"],
                ["method", "POST"]
            ])
        );
    }
}
