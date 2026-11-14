//! One continuously subscribed, cancelable transport per QR login attempt.

use super::{
    auth::{handle_nip46_connect_event, send_connect_ack},
    SavedProfile,
};
use nostr::{nips::nip46::NostrConnectURI, signer::NostrSigner, Filter, Keys, Kind};
use nostr_sdk::{Client, RelayPoolNotification};
use std::{
    fmt,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};
use tokio::sync::watch;

#[derive(Clone)]
pub enum QrLoginStatus {
    Waiting,
    Ready {
        profile: SavedProfile,
        client: Client,
    },
    Failed(String),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Control {
    Waiting,
    Accepted,
    Canceled,
}

pub struct QrLoginSession {
    status: watch::Receiver<QrLoginStatus>,
    control: watch::Sender<Control>,
    accepted: AtomicBool,
}

impl fmt::Debug for QrLoginSession {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("QrLoginSession").finish_non_exhaustive()
    }
}

impl Drop for QrLoginSession {
    fn drop(&mut self) {
        if !self.accepted.load(Ordering::Acquire) {
            let _ = self.control.send(Control::Canceled);
        }
    }
}

impl QrLoginSession {
    /// Establish the subscription before exposing the QR URI to the frontend.
    pub async fn start(
        uri: &str,
        keys: Keys,
        secret: String,
        timeout_secs: u64,
    ) -> anyhow::Result<Self> {
        let uri = url::Url::parse(uri).map_err(|_| anyhow::anyhow!("Invalid QR connection URI"))?;
        anyhow::ensure!(
            uri.scheme() == "nostrconnect"
                && uri.host_str() == Some(keys.public_key().to_hex().as_str()),
            "QR client public key mismatch"
        );
        anyhow::ensure!(
            !secret.is_empty()
                && uri
                    .query_pairs()
                    .any(|(key, value)| key == "secret" && value == secret),
            "QR pairing secret mismatch"
        );
        anyhow::ensure!(timeout_secs > 0, "QR lifetime must be positive");
        let relays = uri
            .query_pairs()
            .filter(|(key, _)| key == "relay")
            .map(|(_, value)| value.parse::<nostr::RelayUrl>())
            .collect::<Result<Vec<_>, _>>()?;
        anyhow::ensure!(!relays.is_empty(), "QR connection requires a relay");
        let client = Client::new(keys.clone());
        for relay in &relays {
            client.add_relay(relay.clone()).await?;
        }
        let mut notifications = client.notifications();
        client.connect().await;
        // Never use poll-time `since`: phones can have clock skew, and approval
        // may already be stored. The unique pairing secret binds this attempt.
        let subscribed = client
            .subscribe(
                Filter::new()
                    .kind(Kind::NostrConnect)
                    .pubkey(keys.public_key())
                    .limit(100),
                None,
            )
            .await;
        if let Err(error) = subscribed {
            client.disconnect().await;
            return Err(error.into());
        }
        let (status, receiver) = watch::channel(QrLoginStatus::Waiting);
        let (control, mut cancellation) = watch::channel(Control::Waiting);
        let deadline = tokio::time::Instant::now() + Duration::from_secs(timeout_secs);
        tracing::info!("QR login listener started on {} relay(s)", relays.len());
        tokio::spawn(async move {
            let wait = async {
                loop {
                    match notifications.recv().await {
                        Ok(RelayPoolNotification::Event { event, .. }) => {
                            let Ok((remote, acknowledgement)) =
                                handle_nip46_connect_event(&event, &keys, &secret).await
                            else {
                                continue;
                            };
                            tracing::info!(
                                "QR signer approval validated; requesting account identity"
                            );
                            // A standard signer sends a response, not a request:
                            // responses must not receive a second acknowledgement.
                            if let Some(id) = acknowledgement {
                                send_connect_ack(&client, &keys, remote, &relays, &id).await?;
                            }
                            let (signer, user) =
                                crate::signers::RestoredNip46Signer::from_authorized_client(
                                    client.clone(),
                                    keys.clone(),
                                    remote,
                                    Duration::from_secs(30.min(timeout_secs)),
                                )
                                .await
                                .map_err(|_| {
                                    anyhow::anyhow!(
                                        "QR signer did not provide an account public key"
                                    )
                                })?;
                            let bunker_uri = NostrConnectURI::Bunker {
                                remote_signer_public_key: remote,
                                relays: relays.clone(),
                                secret: None,
                            };
                            let profile = SavedProfile {
                                id: uuid::Uuid::new_v4().to_string(),
                                name: "QR Connected Account".into(),
                                user_pubkey: user,
                                bunker_uri,
                                app_keys: keys.clone(),
                            };
                            let client = Client::new(Arc::new(signer) as Arc<dyn NostrSigner>);
                            return Ok::<_, anyhow::Error>((profile, client));
                        }
                        Ok(_) => {}
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                        Err(_) => anyhow::bail!("QR connection channel closed"),
                    }
                }
            };
            let result = tokio::select! {
                biased;
                _ = cancellation.changed() => Err(anyhow::anyhow!("QR login canceled")),
                result = tokio::time::timeout_at(deadline, wait) => {
                    result.unwrap_or_else(|_| Err(anyhow::anyhow!("QR code expired. Please generate a new one.")))
                }
            };
            match result {
                Ok((profile, connected)) => {
                    tracing::info!("QR signer identity verified; waiting for account activation");
                    if status
                        .send(QrLoginStatus::Ready {
                            profile,
                            client: connected,
                        })
                        .is_ok()
                    {
                        // Retain cleanup ownership until the IPC layer commits the
                        // account, or cancellation drops the unclaimed approval.
                        while *cancellation.borrow_and_update() == Control::Waiting {
                            if !matches!(
                                tokio::time::timeout_at(deadline, cancellation.changed()).await,
                                Ok(Ok(()))
                            ) {
                                let _ = status.send(QrLoginStatus::Failed(
                                    "QR code expired. Please generate a new one.".into(),
                                ));
                                break;
                            }
                        }
                        if *cancellation.borrow() == Control::Accepted {
                            return;
                        }
                    }
                }
                Err(error) => {
                    let _ = status.send(QrLoginStatus::Failed(error.to_string()));
                }
            }
            client.disconnect().await;
        });
        Ok(Self {
            status: receiver,
            control,
            accepted: AtomicBool::new(false),
        })
    }

    pub fn status(&self) -> QrLoginStatus {
        self.status.borrow().clone()
    }

    /// Transfer the authorized transport to the committed account session.
    pub fn accept(&self) {
        self.accepted.store(true, Ordering::Release);
        let _ = self.control.send(Control::Accepted);
    }

    pub async fn wait(mut self) -> anyhow::Result<(SavedProfile, Client)> {
        loop {
            match self.status() {
                QrLoginStatus::Waiting => self
                    .status
                    .changed()
                    .await
                    .map_err(|_| anyhow::anyhow!("QR connection task stopped"))?,
                QrLoginStatus::Ready { profile, client } => {
                    self.accept();
                    return Ok((profile, client));
                }
                QrLoginStatus::Failed(error) => anyhow::bail!(error),
            }
        }
    }
}
