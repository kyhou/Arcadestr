//! Resume an already-authorized NIP-46 app without replaying one-use pairing.
//! Uses the SDK's request/response types and encrypted event builder; no new protocol.
use nostr::{
    nips::{
        nip44,
        nip46::{
            NostrConnectMessage, NostrConnectMethod, NostrConnectRequest, NostrConnectURI,
            ResponseResult,
        },
    },
    signer::{NostrSigner, SignerBackend, SignerError},
    util::BoxedFuture,
    Event, EventBuilder, Filter, JsonUtil, Keys, Kind, PublicKey, UnsignedEvent,
};
use nostr_sdk::{
    prelude::{RelayPoolNotification, SubscribeOptions},
    Client,
};
use std::time::Duration;

pub(crate) struct RestoredNip46Signer {
    client: Client,
    keys: Keys,
    remote: PublicKey,
    user: PublicKey,
    timeout: Duration,
}

impl std::fmt::Debug for RestoredNip46Signer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RestoredNip46Signer")
            .field("remote", &self.remote)
            .field("user", &self.user)
            .finish_non_exhaustive()
    }
}

impl RestoredNip46Signer {
    /// QR approval already authorized this transport. Discover the account key
    /// without starting a second `connect` exchange or replacing the listener.
    pub(crate) async fn from_authorized_client(
        client: Client,
        keys: Keys,
        remote: PublicKey,
        timeout: Duration,
    ) -> Result<(Self, PublicKey), SignerError> {
        let mut signer = Self {
            client,
            keys,
            remote,
            user: remote,
            timeout,
        };
        let user = signer
            .request(NostrConnectRequest::GetPublicKey)
            .await
            .and_then(|result| result.to_get_public_key().map_err(SignerError::backend));
        match user {
            Ok(user) => {
                signer.user = user;
                Ok((signer, user))
            }
            Err(error) => {
                signer.client.disconnect().await;
                Err(error)
            }
        }
    }

    pub(super) async fn connect(
        uri: &NostrConnectURI,
        keys: Keys,
        user: PublicKey,
        timeout: Duration,
    ) -> Result<Self, SignerError> {
        let remote = uri
            .remote_signer_public_key()
            .copied()
            .ok_or_else(|| SignerError::from("Saved remote signer identity unavailable"))?;
        let client = Client::default();
        for relay in uri.relays() {
            client
                .add_relay(relay.clone())
                .await
                .map_err(SignerError::backend)?;
        }
        client.connect().await;
        client
            .pool()
            .subscribe(
                Filter::new()
                    .author(remote)
                    .pubkey(keys.public_key())
                    .kind(Kind::NostrConnect)
                    .limit(0),
                SubscribeOptions::default(),
            )
            .await
            .map_err(SignerError::backend)?;
        let signer = Self {
            client,
            keys,
            remote,
            user,
            timeout,
        };
        if let Err(error) = signer.verify_identity().await {
            signer.client.disconnect().await;
            return Err(error);
        }
        Ok(signer)
    }

    async fn verify_identity(&self) -> Result<(), SignerError> {
        let actual = self
            .request(NostrConnectRequest::GetPublicKey)
            .await?
            .to_get_public_key()
            .map_err(SignerError::backend)?;
        if actual != self.user {
            return Err(SignerError::from("Saved remote signer identity mismatch"));
        }
        Ok(())
    }

    async fn request(&self, request: NostrConnectRequest) -> Result<ResponseResult, SignerError> {
        let message = NostrConnectMessage::request(&request);
        let id = message.id().to_owned();
        let event = EventBuilder::nostr_connect(&self.keys, self.remote, message)
            .map_err(SignerError::backend)?
            .sign_with_keys(&self.keys)
            .map_err(SignerError::backend)?;
        let mut notifications = self.client.notifications();
        self.client
            .send_event(&event)
            .await
            .map_err(SignerError::backend)?;
        tokio::time::timeout(self.timeout, async {
            loop {
                match notifications.recv().await {
                    Ok(RelayPoolNotification::Event { event, .. }) => {
                        if let Some(result) =
                            response(&event, &self.keys, self.remote, &id, request.method())?
                        {
                            return Ok(result);
                        }
                    }
                    Ok(_) => {}
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(_) => {
                        return Err(SignerError::from("Remote signer response channel closed"))
                    }
                }
            }
        })
        .await
        .map_err(|_| SignerError::backend(nostr_connect::error::Error::Timeout))?
    }
}

fn response(
    event: &Event,
    keys: &Keys,
    remote: PublicKey,
    id: &str,
    method: NostrConnectMethod,
) -> Result<Option<ResponseResult>, SignerError> {
    let recipient = keys.public_key().to_hex();
    if event.kind != Kind::NostrConnect
        || event.pubkey != remote
        || event.verify().is_err()
        || !event.tags.iter().any(|t| {
            let v = t.as_slice();
            v.len() >= 2 && v[0] == "p" && v[1] == recipient
        })
    {
        return Ok(None);
    }
    let plaintext = nip44::decrypt(keys.secret_key(), &remote, &event.content)
        .map_err(|_| SignerError::from("Invalid encrypted signer response"))?;
    let message = NostrConnectMessage::from_json(plaintext)
        .map_err(|_| SignerError::from("Invalid signer response envelope"))?;
    if message.id() != id || !message.is_response() {
        return Ok(None);
    }
    let response = message
        .to_response(method)
        .map_err(|_| SignerError::from("Invalid signer response result"))?;
    if let Some(error) = response.error {
        return Err(safe_remote_error(&error));
    }
    response
        .result
        .map(Some)
        .ok_or_else(|| SignerError::from("Remote signer response lacks a result"))
}

fn safe_remote_error(error: &str) -> SignerError {
    let normalized = error.trim().to_ascii_lowercase();
    if normalized
        .strip_prefix("denied: ")
        .is_some_and(|reason| !reason.is_empty())
        || matches!(
            normalized.as_str(),
            "rejected" | "user rejected" | "permission denied" | "authorization denied"
        )
    {
        SignerError::from("denied: rejected")
    } else if normalized == "timeout" {
        SignerError::from("timeout")
    } else {
        SignerError::from("Remote signer returned an error")
    }
}

fn signed_event_matches(expected: &UnsignedEvent, signed: &Event) -> bool {
    signed.verify().is_ok()
        && signed.pubkey == expected.pubkey
        && signed.created_at == expected.created_at
        && signed.kind == expected.kind
        && signed.tags == expected.tags
        && signed.content == expected.content
}

impl NostrSigner for RestoredNip46Signer {
    fn backend(&self) -> SignerBackend<'_> {
        SignerBackend::Custom("restored-nip46".into())
    }
    fn get_public_key(&self) -> BoxedFuture<'_, Result<PublicKey, SignerError>> {
        Box::pin(async move {
            self.verify_identity().await?;
            Ok(self.user)
        })
    }
    fn sign_event(&self, unsigned: UnsignedEvent) -> BoxedFuture<'_, Result<Event, SignerError>> {
        Box::pin(async move {
            if unsigned.pubkey != self.user {
                return Err(SignerError::from("Event belongs to another identity"));
            }
            let expected = unsigned.clone();
            let signed = self
                .request(NostrConnectRequest::SignEvent(unsigned))
                .await?
                .to_sign_event()
                .map_err(SignerError::backend)?;
            if !signed_event_matches(&expected, &signed) {
                return Err(SignerError::from(
                    "Remote signer returned an invalid event binding",
                ));
            }
            Ok(signed)
        })
    }
    fn nip04_encrypt<'a>(
        &'a self,
        _: &'a PublicKey,
        _: &'a str,
    ) -> BoxedFuture<'a, Result<String, SignerError>> {
        Box::pin(async { Err(SignerError::from("NIP-04 unsupported by restored signer")) })
    }
    fn nip04_decrypt<'a>(
        &'a self,
        _: &'a PublicKey,
        _: &'a str,
    ) -> BoxedFuture<'a, Result<String, SignerError>> {
        Box::pin(async { Err(SignerError::from("NIP-04 unsupported by restored signer")) })
    }
    fn nip44_encrypt<'a>(
        &'a self,
        public_key: &'a PublicKey,
        content: &'a str,
    ) -> BoxedFuture<'a, Result<String, SignerError>> {
        Box::pin(async move {
            self.request(NostrConnectRequest::Nip44Encrypt {
                public_key: *public_key,
                text: content.to_owned(),
            })
            .await?
            .to_nip44_encrypt()
            .map_err(SignerError::backend)
        })
    }
    fn nip44_decrypt<'a>(
        &'a self,
        public_key: &'a PublicKey,
        ciphertext: &'a str,
    ) -> BoxedFuture<'a, Result<String, SignerError>> {
        Box::pin(async move {
            self.request(NostrConnectRequest::Nip44Decrypt {
                public_key: *public_key,
                ciphertext: ciphertext.to_owned(),
            })
            .await?
            .to_nip44_decrypt()
            .map_err(SignerError::backend)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_error_payloads_are_not_exposed() {
        for text in [
            "denied: secret=private-pairing-material",
            "unexpected private-payload",
        ] {
            let message = safe_remote_error(text).to_string();
            assert!(!message.contains("private"));
        }
        assert!(matches!(
            crate::signers::SignerError::from(safe_remote_error("denied: secret=private")),
            crate::signers::SignerError::AuthorizationDenied
        ));
        assert!(matches!(
            crate::signers::SignerError::from(safe_remote_error("timeout")),
            crate::signers::SignerError::ConnectionTimeout
        ));
    }
    fn event(remote: &Keys, recipient: PublicKey, id: &str, error: Option<&str>) -> Event {
        let message = NostrConnectMessage::Response {
            id: id.into(),
            result: if error.is_some() {
                None
            } else {
                Some(remote.public_key().to_hex())
            },
            error: error.map(str::to_owned),
        };
        EventBuilder::nostr_connect(remote, recipient, message)
            .expect("test response")
            .sign_with_keys(remote)
            .expect("test signature")
    }
    #[test]
    fn only_correlated_authentic_recipient_bound_responses_are_accepted() {
        let app = Keys::generate();
        let remote = Keys::generate();
        let other = Keys::generate();
        let valid = event(&remote, app.public_key(), "current", None);
        assert!(response(
            &valid,
            &app,
            remote.public_key(),
            "current",
            NostrConnectMethod::GetPublicKey
        )
        .expect("valid")
        .is_some());
        for candidate in [
            event(&other, app.public_key(), "current", None),
            event(&remote, other.public_key(), "current", None),
            event(&remote, app.public_key(), "old", None),
        ] {
            assert!(response(
                &candidate,
                &app,
                remote.public_key(),
                "current",
                NostrConnectMethod::GetPublicKey
            )
            .expect("ignored")
            .is_none());
        }
        let mut invalid = valid;
        invalid.content.push('x');
        assert!(response(
            &invalid,
            &app,
            remote.public_key(),
            "current",
            NostrConnectMethod::GetPublicKey
        )
        .expect("ignored bad signature")
        .is_none());
    }
    #[test]
    fn explicit_refusal_survives_the_restored_transport() {
        let app = Keys::generate();
        let remote = Keys::generate();
        let denied = event(
            &remote,
            app.public_key(),
            "current",
            Some("denied: rejected"),
        );
        let error = response(
            &denied,
            &app,
            remote.public_key(),
            "current",
            NostrConnectMethod::GetPublicKey,
        )
        .expect_err("refusal");
        assert!(matches!(
            crate::signers::SignerError::from(error),
            crate::signers::SignerError::AuthorizationDenied
        ));
    }
    #[test]
    fn returned_event_cannot_change_the_requested_content_or_identity() {
        let keys = Keys::generate();
        let other = Keys::generate();
        let expected = EventBuilder::new(Kind::Custom(27235), "original").build(keys.public_key());
        let valid = expected.clone().sign_with_keys(&keys).expect("signed");
        assert!(signed_event_matches(&expected, &valid));
        let changed = EventBuilder::new(Kind::Custom(27235), "changed")
            .build(keys.public_key())
            .sign_with_keys(&keys)
            .expect("changed");
        assert!(!signed_event_matches(&expected, &changed));
        let foreign = EventBuilder::new(Kind::Custom(27235), "original")
            .build(other.public_key())
            .sign_with_keys(&other)
            .expect("foreign");
        assert!(!signed_event_matches(&expected, &foreign));
    }
}
