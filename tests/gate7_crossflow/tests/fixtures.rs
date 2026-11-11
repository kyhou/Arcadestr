use adp_server::{
    config::Config, keystore::KeyStore, relay::RelayClient, state::AppState, storage::Storage,
};
use futures_util::{SinkExt, StreamExt};
use nostr_sdk::{Event, Filter, Keys, Tag};
use serde_json::{json, Value};
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio_tungstenite::tungstenite::Message;

pub fn tag(values: &[&str]) -> Tag {
    Tag::parse(values.iter().copied()).expect("synthetic tag")
}
pub struct Task(pub tokio::task::JoinHandle<()>);
impl Drop for Task {
    fn drop(&mut self) {
        self.0.abort();
    }
}
pub type Events = Arc<Mutex<Vec<Event>>>;

/// Actual Nostr WebSocket messages, signed-event validation and EOSE. The fixture
/// retains immutable sale snapshots; it makes no public-relay retention promise.
pub async fn relay(events: Events) -> (String, Task) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("local relay");
    let url = format!("ws://{}", listener.local_addr().expect("address"));
    let task = tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            let events = events.clone();
            tokio::spawn(async move {
                let Ok(mut ws) = tokio_tungstenite::accept_async(stream).await else {
                    return;
                };
                while let Some(Ok(message)) = ws.next().await {
                    let Message::Text(text) = message else {
                        if let Message::Ping(bytes) = message {
                            let _ = ws.send(Message::Pong(bytes)).await;
                        }
                        continue;
                    };
                    let Ok(value) = serde_json::from_str::<Value>(&text) else {
                        continue;
                    };
                    match value[0].as_str() {
                        Some("EVENT") => {
                            let event: Event =
                                serde_json::from_value(value[1].clone()).expect("event");
                            event.verify().expect("local publication signature");
                            {
                                let mut store = events.lock().expect("store");
                                if !store.iter().any(|e| e.id == event.id) {
                                    store.push(event.clone());
                                }
                            }
                            if ws
                                .send(Message::Text(
                                    json!(["OK", event.id, true, ""]).to_string().into(),
                                ))
                                .await
                                .is_err()
                            {
                                return;
                            }
                        }
                        Some("REQ") => {
                            let filters = value
                                .as_array()
                                .expect("REQ")
                                .iter()
                                .skip(2)
                                .map(|v| {
                                    serde_json::from_value::<Filter>(v.clone()).expect("filter")
                                })
                                .collect::<Vec<_>>();
                            let snapshot = events.lock().expect("snapshot").clone();
                            for event in snapshot.iter().filter(|e| {
                                filters.iter().any(|f| f.match_event(e, Default::default()))
                            }) {
                                if ws
                                    .send(Message::Text(
                                        json!(["EVENT", value[1], event]).to_string().into(),
                                    ))
                                    .await
                                    .is_err()
                                {
                                    return;
                                }
                            }
                            if ws
                                .send(Message::Text(json!(["EOSE", value[1]]).to_string().into()))
                                .await
                                .is_err()
                            {
                                return;
                            }
                        }
                        _ => {}
                    }
                }
            });
        }
    });
    (url, Task(task))
}

pub struct Server {
    pub url: String,
    pub state: AppState,
    pub operator: Keys,
    pub storage: Storage,
    pub _dir: tempfile::TempDir,
    pub _task: Task,
}
impl Server {
    pub async fn start(relays: &[String]) -> Self {
        let dir = tempfile::tempdir_in("/tmp/opencode").expect("private server");
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("HTTP listener");
        let url = format!("http://{}", listener.local_addr().expect("address"));
        let operator = Keys::generate();
        let storage = Storage::connect(dir.path().join("adp.db").to_str().expect("db path"))
            .await
            .expect("server DB");
        let files = dir.path().join("files");
        tokio::fs::create_dir_all(&files).await.expect("files");
        let config = Config {
            server_url: url.clone(),
            bind_addr: "127.0.0.1:0".into(),
            db_path: dir.path().join("adp.db").to_string_lossy().into(),
            operator_nsec: operator.secret_key().to_secret_hex(),
            master_key: [17; 32],
            relays: relays.to_vec(),
            max_upload_bytes: 1024 * 1024,
            files_dir: files.to_string_lossy().into(),
            allow_insecure_lnurl: true,
            download_token_ttl_secs: 900,
            file_reverify_interval_secs: 0,
        };
        let relay = RelayClient::connect_with_timeout(
            operator.clone(),
            relays,
            url.clone(),
            Duration::from_secs(3),
        )
        .await
        .expect("actual server Nostr client");
        let keystore = KeyStore::new(storage.pool(), config.master_key);
        let state = AppState {
            operator_keys: operator.clone(),
            config: Arc::new(config),
            storage: storage.clone(),
            keystore: Arc::new(keystore),
            relay: Arc::new(relay),
        };
        let router = adp_server::routes::router(state.clone());
        let task = Task(tokio::spawn(async move {
            axum::serve(listener, router)
                .await
                .expect("actual ADP router");
        }));
        Self {
            url,
            state,
            operator,
            storage,
            _dir: dir,
            _task: task,
        }
    }
}

pub fn signer(keys: &Keys) -> arcadestr_core::signers::ActiveSigner {
    arcadestr_core::signers::ActiveSigner::DirectKey(
        arcadestr_core::signers::DirectKeySigner::from_key(&keys.secret_key().to_secret_hex())
            .expect("synthetic desktop signer"),
    )
}
pub fn http() -> Arc<dyn arcadestr_core::http_client::HttpClient> {
    Arc::new(
        arcadestr_core::http_client::ReqwestHttpClient::new(Duration::from_secs(10))
            .expect("actual HTTP transport"),
    )
}
