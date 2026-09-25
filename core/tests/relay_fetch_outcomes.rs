#![cfg(feature = "native")]

#[path = "support/local_relay.rs"]
mod local_relay;

use arcadestr_core::marketplace::fetch_nip99_listings_streaming_since;
use arcadestr_core::relay_manager::{RelayManager, RelayManagerConfig, RelayManagerError};
use local_relay::{LocalRelay, Reply};
use nostr_sdk::{EventBuilder, Filter, Keys, Kind, Tag};
use std::sync::{Arc, Mutex};

async fn manager(relays: &[&LocalRelay]) -> RelayManager {
    let manager = RelayManager::new(
        "fetch-outcome-test".into(),
        RelayManagerConfig {
            debug_relays: Some(relays.iter().map(|relay| relay.url.clone()).collect()),
            block_discovery: true,
            ..Default::default()
        },
        None,
    )
    .await
    .expect("local relay manager");
    for relay in manager.get_client().relays().await.into_values() {
        relay
            .wait_for_connection(std::time::Duration::from_secs(3))
            .await;
        assert!(relay.is_connected());
    }
    manager
}

#[tokio::test]
async fn eose_is_successful_empty_not_failure() {
    let relay = LocalRelay::start(Reply::Events(vec![])).await;
    let manager = manager(&[&relay]).await;
    let result = manager
        .fetch_events_streaming(Filter::new(), 2, 2, |_, events| assert!(events.is_empty()))
        .await;
    assert_eq!(result.expect("EOSE success"), 0);
    let manager = Arc::new(tokio::sync::Mutex::new(manager));
    assert_eq!(
        fetch_nip99_listings_streaming_since(&manager, 10, None, None, |_| {})
            .await
            .expect("empty marketplace success"),
        0
    );
}

#[tokio::test]
async fn no_connected_relays_is_failure_not_empty() {
    let manager = manager(&[]).await;
    let result = manager
        .fetch_events_streaming(Filter::new(), 1, 1, |_, _| panic!("no relay responded"))
        .await;
    assert!(matches!(result, Err(RelayManagerError::Connection(_))));
}

#[tokio::test]
async fn closed_is_failure_not_pool_successful_empty() {
    let relay = LocalRelay::start(Reply::Closed).await;
    let manager = manager(&[&relay]).await;
    let result = manager
        .fetch_events_streaming(Filter::new(), 2, 2, |_, _| panic!("failure is not EOSE"))
        .await;
    assert!(matches!(
        result,
        Err(RelayManagerError::FetchIncomplete {
            failed: 1,
            timed_out: 0,
            succeeded: 0,
            events: 0
        })
    ));
    let manager = Arc::new(tokio::sync::Mutex::new(manager));
    let error = fetch_nip99_listings_streaming_since(&manager, 10, None, None, |_| {})
        .await
        .expect_err("marketplace must propagate failure");
    assert!(error.contains("1 failed"));
}

#[tokio::test]
async fn silent_relay_is_timeout_not_empty() {
    let relay = LocalRelay::start(Reply::Silent).await;
    let manager = manager(&[&relay]).await;
    let result = manager
        .fetch_events_streaming(Filter::new(), 1, 2, |_, _| panic!("timeout is not EOSE"))
        .await;
    assert!(matches!(
        result,
        Err(RelayManagerError::FetchIncomplete {
            failed: 0,
            timed_out: 1,
            succeeded: 0,
            events: 0
        })
    ));
}

#[tokio::test]
async fn disconnected_relay_is_failure_not_empty() {
    let relay = LocalRelay::start(Reply::Disconnect).await;
    let manager = manager(&[&relay]).await;
    let result = manager
        .fetch_events_streaming(Filter::new(), 2, 2, |_, _| panic!("disconnect is not EOSE"))
        .await;
    assert!(matches!(
        result,
        Err(RelayManagerError::FetchIncomplete {
            failed: 1,
            timed_out: 0,
            succeeded: 0,
            events: 0
        })
    ));
}

#[tokio::test]
async fn events_before_timeout_are_retained_without_claiming_eose() {
    let event = EventBuilder::new(Kind::TextNote, "received before timeout")
        .sign_with_keys(&Keys::generate())
        .expect("signed event");
    let relay = LocalRelay::start(Reply::EventsThenSilent(vec![event.clone()])).await;
    let manager = manager(&[&relay]).await;
    let received = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&received);
    let result = manager
        .fetch_events_streaming(Filter::new(), 5, 1, move |_, events| {
            sink.lock().expect("sink").extend(events)
        })
        .await;
    assert!(matches!(
        result,
        Err(RelayManagerError::FetchIncomplete {
            succeeded: 0,
            timed_out: 1,
            failed: 0,
            events: 1
        })
    ));
    assert_eq!(*received.lock().expect("received"), vec![event]);
}

#[tokio::test]
async fn malformed_event_is_not_delivered_or_persistable() {
    let mut event = EventBuilder::new(Kind::TextNote, "signed")
        .sign_with_keys(&Keys::generate())
        .expect("signed event");
    event.content = "tampered after signing".into();
    let relay = LocalRelay::start(Reply::Events(vec![event])).await;
    let manager = manager(&[&relay]).await;
    let result = manager
        .fetch_events_streaming(Filter::new(), 2, 2, |_, events| assert!(events.is_empty()))
        .await;
    assert_eq!(result.expect("EOSE with no valid events"), 0);
}

#[tokio::test]
async fn inactivity_timeout_retains_completed_relay_results() {
    let event = EventBuilder::new(Kind::TextNote, "partial result")
        .sign_with_keys(&Keys::generate())
        .expect("signed event");
    let good = LocalRelay::start(Reply::Events(vec![event.clone()])).await;
    let silent = LocalRelay::start(Reply::Silent).await;
    let manager = manager(&[&good, &silent]).await;
    let received = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&received);
    let result = manager
        .fetch_events_streaming(Filter::new(), 5, 1, move |_, events| {
            sink.lock().expect("sink").extend(events)
        })
        .await;
    assert!(matches!(
        result,
        Err(RelayManagerError::FetchIncomplete {
            succeeded: 1,
            timed_out: 1,
            failed: 0,
            events: 1
        })
    ));
    assert_eq!(*received.lock().expect("received"), vec![event]);
}

#[tokio::test]
async fn marketplace_retains_partial_listings_and_propagates_failure() {
    let event = EventBuilder::new(Kind::Custom(30402), "A game")
        .tags([
            Tag::parse(["d", "partial-game"]).expect("d tag"),
            Tag::parse(["title", "Partial Game"]).expect("title tag"),
            Tag::parse(["price", "0", "sats"]).expect("price tag"),
            Tag::parse(["t", "game"]).expect("game tag"),
        ])
        .sign_with_keys(&Keys::generate())
        .expect("signed listing");
    let good = LocalRelay::start(Reply::Events(vec![event])).await;
    let bad = LocalRelay::start(Reply::Closed).await;
    let manager = Arc::new(tokio::sync::Mutex::new(manager(&[&good, &bad]).await));
    let received = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&received);
    let result = fetch_nip99_listings_streaming_since(&manager, 10, None, None, move |listing| {
        sink.lock().expect("sink").push(listing)
    })
    .await;
    assert!(result.expect_err("partial warning").contains("1 failed"));
    assert_eq!(received.lock().expect("received").len(), 1);
}
