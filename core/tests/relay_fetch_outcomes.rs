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
    assert_eq!(
        error,
        arcadestr_core::marketplace_fetch::MarketplaceFetchError::Unreachable {
            failed: 1,
            timed_out: 0
        }
    );
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

fn listing_event(d: &str) -> nostr_sdk::Event {
    EventBuilder::new(Kind::Custom(30402), "A game")
        .tags([
            Tag::parse(["d", d]).expect("d tag"),
            Tag::parse(["title", "Partial Game"]).expect("title tag"),
            Tag::parse(["price", "0", "sats"]).expect("price tag"),
            Tag::parse(["t", "game"]).expect("game tag"),
        ])
        .sign_with_keys(&Keys::generate())
        .expect("signed listing")
}

async fn marketplace_fetch(
    relays: &[&LocalRelay],
) -> (
    Result<u32, arcadestr_core::marketplace_fetch::MarketplaceFetchError>,
    usize,
) {
    let manager = Arc::new(tokio::sync::Mutex::new(manager(relays).await));
    let received = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&received);
    let result = fetch_nip99_listings_streaming_since(&manager, 10, None, None, move |listing| {
        sink.lock().expect("sink").push(listing)
    })
    .await;
    let count = received.lock().expect("received").len();
    (result, count)
}

// On Nostr some relay is always down, slow or refusing: partial coverage is
// the normal outcome of a refresh, not a failure.
#[tokio::test]
async fn marketplace_partial_coverage_is_a_successful_refresh() {
    let good = LocalRelay::start(Reply::Events(vec![listing_event("partial-game")])).await;
    let bad = LocalRelay::start(Reply::Closed).await;
    let (result, delivered) = marketplace_fetch(&[&good, &bad]).await;
    assert_eq!(result.expect("partial coverage succeeds"), 1);
    assert_eq!(delivered, 1);
}

#[tokio::test]
async fn marketplace_answered_empty_is_successful_despite_failed_relays() {
    let empty = LocalRelay::start(Reply::Events(Vec::new())).await;
    let bad = LocalRelay::start(Reply::Closed).await;
    let (result, delivered) = marketplace_fetch(&[&empty, &bad]).await;
    assert_eq!(result.expect("an EOSE answer is a real empty result"), 0);
    assert_eq!(delivered, 0);
}

#[tokio::test]
async fn marketplace_listings_from_a_relay_that_then_timed_out_count_as_an_answer() {
    let slow = LocalRelay::start(Reply::EventsThenSilent(vec![listing_event("slow-game")])).await;
    let (result, delivered) = marketplace_fetch(&[&slow]).await;
    assert_eq!(result.expect("received listings are an answer"), 1);
    assert_eq!(delivered, 1);
}

#[tokio::test]
async fn marketplace_with_no_relay_answering_is_unreachable() {
    let silent = LocalRelay::start(Reply::Silent).await;
    let closed = LocalRelay::start(Reply::Closed).await;
    let (result, delivered) = marketplace_fetch(&[&silent, &closed]).await;
    assert_eq!(
        result.expect_err("nothing answered"),
        arcadestr_core::marketplace_fetch::MarketplaceFetchError::Unreachable {
            failed: 1,
            timed_out: 1
        }
    );
    assert_eq!(delivered, 0);
}
