#![cfg(feature = "native")]

use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
use std::time::Duration;

use arcadestr_core::{nostr::NostrClient, storage::Database, RelayManagerConfig, UserCache};
use futures_util::{SinkExt, StreamExt};
use nostr::{Event, EventBuilder, Keys, Kind, Timestamp, ToBech32};
use serde_json::json;
use tokio::{net::TcpListener, sync::RwLock};

fn metadata(keys: &Keys, name: &str, time: u64) -> Event {
    EventBuilder::new(Kind::Metadata, json!({"name":name}).to_string())
        .custom_created_at(Timestamp::from(time))
        .sign_with_keys(keys)
        .expect("signed metadata")
}

async fn relay(
    event: Arc<RwLock<Option<Event>>>,
    delay: Duration,
) -> (String, Arc<AtomicUsize>, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("listener");
    let url = format!("ws://{}", listener.local_addr().expect("address"));
    let requests = Arc::new(AtomicUsize::new(0));
    let counted = requests.clone();
    let task = tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            let event = event.clone();
            let counted = counted.clone();
            tokio::spawn(async move {
                let Ok(mut socket) = tokio_tungstenite::accept_async(stream).await else {
                    return;
                };
                while let Some(Ok(message)) = socket.next().await {
                    let Ok(text) = message.to_text() else {
                        continue;
                    };
                    let Ok(request) = serde_json::from_str::<serde_json::Value>(text) else {
                        continue;
                    };
                    if request[0] != "REQ" {
                        continue;
                    }
                    let id = &request[1];
                    let kind = request[2]["kinds"][0].as_u64();
                    if kind == Some(0) {
                        counted.fetch_add(1, Ordering::SeqCst);
                        tokio::time::sleep(delay).await;
                        if let Some(event) = event.read().await.as_ref() {
                            if socket
                                .send(json!(["EVENT", id, event]).to_string().into())
                                .await
                                .is_err()
                            {
                                break;
                            }
                        }
                    } else {
                        // Relay discovery is intentionally slower than metadata.
                        tokio::time::sleep(Duration::from_millis(800)).await;
                    }
                    if socket
                        .send(json!(["EOSE", id]).to_string().into())
                        .await
                        .is_err()
                    {
                        break;
                    }
                }
            });
        }
    });
    (url, requests, task)
}

#[tokio::test]
async fn refresh_queries_relays_despite_fresh_cache_and_keeps_saved_data_on_empty_results() {
    let temp = tempfile::TempDir::new().expect("temp directory");
    let database = Database::new(&temp.path().join("profiles.db"))
        .await
        .expect("database");
    let cache = Arc::new(UserCache::new(database.pool().clone()));
    let keys = Keys::generate();
    let npub = keys.public_key().to_bech32().expect("npub");
    cache
        .put_event(&npub, &metadata(&keys, "saved", 10))
        .await
        .expect("saved metadata");
    let served = Arc::new(RwLock::new(Some(metadata(&keys, "updated", 20))));
    let (url, requests, server) = relay(served.clone(), Duration::ZERO).await;
    let client = NostrClient::new_with_cache(
        "metadata-refresh-test".into(),
        vec![],
        cache.clone(),
        Some(RelayManagerConfig {
            debug_relays: Some(vec![url]),
            block_discovery: true,
            query_timeout_secs: 2,
            ..Default::default()
        }),
    )
    .await
    .expect("client");
    client.wait_for_connections().await.expect("connected");
    assert_eq!(
        client
            .fetch_profile(&npub, None)
            .await
            .expect("cache")
            .name
            .as_deref(),
        Some("saved")
    );
    assert_eq!(requests.load(Ordering::SeqCst), 0);
    assert_eq!(
        client
            .fetch_profile_with_relay_discovery(&npub)
            .await
            .expect("refresh")
            .name
            .as_deref(),
        Some("updated")
    );
    assert!(requests.load(Ordering::SeqCst) > 0);
    *served.write().await = None;
    assert_eq!(
        client
            .refresh_profile(&npub)
            .await
            .expect("empty relay")
            .name
            .as_deref(),
        Some("updated")
    );
    *served.write().await = Some(metadata(&keys, "stale", 1));
    assert_eq!(
        client
            .refresh_profile(&npub)
            .await
            .expect("stale relay")
            .name
            .as_deref(),
        Some("updated")
    );
    server.abort();
}

#[tokio::test]
async fn first_profile_does_not_wait_for_relay_discovery() {
    let keys = Keys::generate();
    let npub = keys.public_key().to_bech32().expect("npub");
    let served = Arc::new(RwLock::new(Some(metadata(&keys, "first-login", 10))));
    let (url, _, server) = relay(served, Duration::ZERO).await;
    let client = NostrClient::new(
        "metadata-first-login-test".into(),
        vec![],
        Some(RelayManagerConfig {
            debug_relays: Some(vec![url]),
            block_discovery: false,
            query_timeout_secs: 2,
            ..Default::default()
        }),
    )
    .await
    .expect("client");
    client.wait_for_connections().await.expect("connected");
    let value = tokio::time::timeout(
        Duration::from_millis(500),
        client.fetch_profile_with_relay_discovery(&npub),
    )
    .await
    .expect("metadata must not wait for 800ms discovery")
    .expect("profile");
    assert_eq!(value.name.as_deref(), Some("first-login"));
    server.abort();
}

#[tokio::test]
async fn fast_relay_updates_ui_before_slow_relay_and_newest_event_wins() {
    let temp = tempfile::TempDir::new().expect("temp directory");
    let database = Database::new(&temp.path().join("profiles.db"))
        .await
        .expect("database");
    let cache = Arc::new(UserCache::new(database.pool().clone()));
    let keys = Keys::generate();
    let npub = keys.public_key().to_bech32().expect("npub");
    let (fast, _, fast_task) = relay(
        Arc::new(RwLock::new(Some(metadata(&keys, "fast", 10)))),
        Duration::ZERO,
    )
    .await;
    let (slow, _, slow_task) = relay(
        Arc::new(RwLock::new(Some(metadata(&keys, "newest", 20)))),
        Duration::from_millis(800),
    )
    .await;
    let client = NostrClient::new_with_cache(
        "metadata-stream-test".into(),
        vec![],
        cache.clone(),
        Some(RelayManagerConfig {
            debug_relays: Some(vec![fast, slow]),
            block_discovery: true,
            ..Default::default()
        }),
    )
    .await
    .expect("client");
    // Wait for both test sockets so the slow response is part of the same query.
    let manager = { client.relay_manager().lock().await.clone() };
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if manager
                .get_client()
                .relays()
                .await
                .values()
                .filter(|relay| relay.is_connected())
                .count()
                == 2
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("both connected");
    let (sender, mut updates) = tokio::sync::mpsc::unbounded_channel();
    let requested_npub = npub.clone();
    let query = tokio::spawn(async move {
        client
            .fetch_profile_with_updates(&requested_npub, move |profile| {
                let _ = sender.send(profile);
            })
            .await
    });
    let first = tokio::time::timeout(Duration::from_millis(500), updates.recv())
        .await
        .expect("fast update")
        .expect("profile");
    assert_eq!(first.name.as_deref(), Some("fast"));
    assert!(
        !query.is_finished(),
        "UI should update before the slow relay finishes"
    );
    assert_eq!(
        query
            .await
            .expect("query task")
            .expect("profile")
            .name
            .as_deref(),
        Some("newest")
    );
    assert_eq!(
        cache.get(&npub).await.expect("saved").name.as_deref(),
        Some("newest")
    );
    fast_task.abort();
    slow_task.abort();
}
