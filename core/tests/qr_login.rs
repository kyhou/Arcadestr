#![cfg(feature = "native")]

use arcadestr_core::nip46::{
    wait_for_qr_connection, AppSignerState, PendingQrState, QrLoginSession, QrLoginStatus,
};
use futures_util::{SinkExt, StreamExt};
use nostr::{nips::nip44, Event, EventBuilder, Keys, Kind, Tag, Timestamp};
use serde_json::{json, Value};
use std::{
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};
use tokio::{net::TcpListener, sync::broadcast};

fn response(remote: &Keys, app: &Keys, id: &str, result: Value, timestamp: u64) -> Event {
    let content = nip44::encrypt(
        remote.secret_key(),
        &app.public_key(),
        json!({"id":id,"result":result,"error":null}).to_string(),
        nip44::Version::V2,
    )
    .expect("encrypt response");
    EventBuilder::new(Kind::NostrConnect, content)
        .tags([Tag::public_key(app.public_key())])
        .custom_created_at(Timestamp::from(timestamp))
        .sign_with_keys(remote)
        .expect("signed response")
}

struct Relay {
    url: String,
    task: tokio::task::JoinHandle<()>,
    methods: Arc<Mutex<Vec<String>>>,
    subscriptions: Arc<AtomicUsize>,
    connections: Arc<AtomicUsize>,
    events: broadcast::Sender<Event>,
}

struct Connection(Arc<AtomicUsize>);
impl Drop for Connection {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

impl Drop for Relay {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn relay(app: Keys, remote: Keys, user: Keys, approval: Option<Event>) -> Relay {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("relay listener");
    let url = format!("ws://{}", listener.local_addr().expect("address"));
    let (events, _) = broadcast::channel::<Event>(32);
    let methods = Arc::new(Mutex::new(Vec::new()));
    let seen = methods.clone();
    let subscriptions = Arc::new(AtomicUsize::new(0));
    let connections = Arc::new(AtomicUsize::new(0));
    let (counted_subscriptions, counted_connections, publish) =
        (subscriptions.clone(), connections.clone(), events.clone());
    let task = tokio::spawn(async move {
        while let Ok((socket, _)) = listener.accept().await {
            let (app, remote, user, approval, seen, events) = (
                app.clone(),
                remote.clone(),
                user.clone(),
                approval.clone(),
                seen.clone(),
                events.clone(),
            );
            let (counted_subscriptions, counted_connections) =
                (counted_subscriptions.clone(), counted_connections.clone());
            tokio::spawn(async move {
                let Ok(socket) = tokio_tungstenite::accept_async(socket).await else {
                    return;
                };
                counted_connections.fetch_add(1, Ordering::SeqCst);
                let _connection = Connection(counted_connections);
                let (mut write, mut read) = socket.split();
                let mut updates = events.subscribe();
                let mut subscription = None::<String>;
                loop {
                    tokio::select! {
                        message = read.next() => {
                            let Some(Ok(message)) = message else { break; };
                            let Ok(text) = message.to_text() else { continue; };
                            let Ok(frame) = serde_json::from_str::<Value>(text) else { continue; };
                            match frame[0].as_str() {
                                Some("REQ") => {
                                    let Some(id) = frame[1].as_str() else { continue; };
                                    subscription = Some(id.into());
                                    counted_subscriptions.fetch_add(1, Ordering::SeqCst);
                                    // A relay returns a saved approval only when the filter permits it.
                                    let since = frame[2]["since"].as_u64().unwrap_or(0);
                                    if let Some(approval) = &approval {
                                        if approval.created_at.as_secs() >= since {
                                            if write.send(json!(["EVENT",id,approval]).to_string().into()).await.is_err() { break; }
                                        }
                                    }
                                    if write.send(json!(["EOSE",id]).to_string().into()).await.is_err() { break; }
                                }
                                Some("EVENT") => {
                                    let Ok(event) = serde_json::from_value::<Event>(frame[1].clone()) else { continue; };
                                    if write.send(json!(["OK",event.id.to_hex(),true,""]).to_string().into()).await.is_err() { break; }
                                    if event.pubkey != app.public_key() { continue; }
                                    let Ok(plaintext) = nip44::decrypt(remote.secret_key(), &event.pubkey, &event.content) else { continue; };
                                    let Ok(request) = serde_json::from_str::<Value>(&plaintext) else { continue; };
                                    let Some(method) = request["method"].as_str() else { continue; };
                                    seen.lock().expect("methods").push(method.into());
                                    let id = request["id"].as_str().unwrap_or("missing-id");
                                    let reply = match method {
                                        "get_public_key" => response(&remote, &app, id, json!(user.public_key().to_hex()), Timestamp::now().as_secs()),
                                        // QR approval has already authorized this client. A new connect is a bug.
                                        _ => response(&remote, &app, id, json!("unexpected pairing request"), Timestamp::now().as_secs()),
                                    };
                                    let _ = events.send(reply);
                                }
                                _ => {}
                            }
                        }
                        update = updates.recv() => {
                            if let (Ok(event), Some(id)) = (update, &subscription) {
                                if write.send(json!(["EVENT",id,event]).to_string().into()).await.is_err() { break; }
                            }
                        }
                    }
                }
            });
        }
    });
    Relay {
        url,
        task,
        methods,
        subscriptions,
        connections,
        events: publish,
    }
}

#[tokio::test]
async fn qr_login_accepts_saved_approval_without_repairing_and_uses_account_identity() {
    let app = Keys::generate();
    let remote = Keys::generate();
    let user = Keys::generate();
    let secret = "test-qr-pairing-nonce";
    let approval = response(
        &remote,
        &app,
        "qr-approval",
        json!(secret),
        Timestamp::now().as_secs().saturating_sub(5),
    );
    let relay = relay(app.clone(), remote.clone(), user.clone(), Some(approval)).await;
    let uri = format!(
        "nostrconnect://{}?relay={}&secret={}&name=Arcadestr",
        app.public_key().to_hex(),
        relay.url,
        secret
    );
    let result = tokio::time::timeout(
        Duration::from_secs(3),
        wait_for_qr_connection(&uri, app, secret.into(), 1),
    )
    .await;
    assert_eq!(
        relay.methods.lock().expect("methods").as_slice(),
        ["get_public_key"],
        "approval must not trigger a second pairing handshake"
    );
    let result = result.expect("bounded QR login");
    let (profile, client) =
        result.expect("saved QR approval must complete login even before polling starts");
    assert_eq!(profile.user_pubkey, user.public_key());
    assert_eq!(
        profile.bunker_uri.remote_signer_public_key(),
        Some(&remote.public_key())
    );
    assert_eq!(
        relay.methods.lock().expect("methods").as_slice(),
        ["get_public_key"]
    );
    assert_eq!(
        client
            .signer()
            .await
            .expect("signer")
            .get_public_key()
            .await
            .expect("identity"),
        user.public_key()
    );
}

async fn eventually(mut condition: impl FnMut() -> bool) {
    tokio::time::timeout(Duration::from_secs(2), async {
        while !condition() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("condition should become true");
}

#[tokio::test]
async fn status_polls_share_one_listener_and_late_approval_completes_without_reconnecting() {
    let (app, remote, user) = (Keys::generate(), Keys::generate(), Keys::generate());
    let secret = "late-qr-secret";
    let relay = relay(app.clone(), remote.clone(), user.clone(), None).await;
    let uri = format!(
        "nostrconnect://{}?relay={}&secret={secret}",
        app.public_key().to_hex(),
        relay.url
    );
    let session = QrLoginSession::start(&uri, app.clone(), secret.into(), 5)
        .await
        .expect("listener");
    eventually(|| relay.subscriptions.load(Ordering::SeqCst) == 1).await;
    for _ in 0..20 {
        assert!(matches!(session.status(), QrLoginStatus::Waiting));
    }
    assert_eq!(relay.subscriptions.load(Ordering::SeqCst), 1);
    assert_eq!(relay.connections.load(Ordering::SeqCst), 1);
    relay
        .events
        .send(response(
            &remote,
            &app,
            "late-approval",
            json!(secret),
            Timestamp::now().as_secs().saturating_sub(60),
        ))
        .expect("deliver approval");
    eventually(|| matches!(session.status(), QrLoginStatus::Ready { .. })).await;
    match session.status() {
        QrLoginStatus::Ready { profile, .. } => assert_eq!(profile.user_pubkey, user.public_key()),
        _ => panic!("expected ready session"),
    }
    assert_eq!(relay.subscriptions.load(Ordering::SeqCst), 1);
    // An approved but unclaimed attempt must close too, not become a hidden login.
    drop(session);
    eventually(|| relay.connections.load(Ordering::SeqCst) == 0).await;
}

#[tokio::test]
async fn canceled_attempt_disconnects_and_a_new_qr_does_not_reuse_its_transport() {
    let (app, remote, user) = (Keys::generate(), Keys::generate(), Keys::generate());
    let relay = relay(app.clone(), remote, user, None).await;
    let uri = format!(
        "nostrconnect://{}?relay={}&secret=cancel-secret",
        app.public_key().to_hex(),
        relay.url
    );
    let session = QrLoginSession::start(&uri, app, "cancel-secret".into(), 5)
        .await
        .expect("listener");
    eventually(|| relay.subscriptions.load(Ordering::SeqCst) == 1).await;
    let mut state = AppSignerState::new();
    state.pending_qr = Some(PendingQrState {
        uri,
        session: Some(Arc::new(session)),
        created_at: std::time::Instant::now(),
    });
    state.invalidate_pending_auth();
    eventually(|| relay.connections.load(Ordering::SeqCst) == 0).await;
    let app = Keys::generate();
    let uri = format!(
        "nostrconnect://{}?relay={}&secret=new-secret",
        app.public_key().to_hex(),
        relay.url
    );
    let second = QrLoginSession::start(&uri, app, "new-secret".into(), 5)
        .await
        .expect("new listener");
    eventually(|| relay.subscriptions.load(Ordering::SeqCst) == 2).await;
    assert!(matches!(second.status(), QrLoginStatus::Waiting));
    drop(second);
    eventually(|| relay.connections.load(Ordering::SeqCst) == 0).await;
}

#[tokio::test]
async fn qr_expiration_is_a_terminal_error_and_closes_the_listener() {
    let (app, remote, user) = (Keys::generate(), Keys::generate(), Keys::generate());
    let relay = relay(app.clone(), remote, user, None).await;
    let uri = format!(
        "nostrconnect://{}?relay={}&secret=expiry-secret",
        app.public_key().to_hex(),
        relay.url
    );
    let session = QrLoginSession::start(&uri, app, "expiry-secret".into(), 1)
        .await
        .expect("listener");
    eventually(|| matches!(session.status(), QrLoginStatus::Failed(_))).await;
    assert!(matches!(session.status(), QrLoginStatus::Failed(error) if error.contains("expired")));
    eventually(|| relay.connections.load(Ordering::SeqCst) == 0).await;
}
