//! Startup and setup must not own a temporary executor for persistent services.

pub(crate) fn block_on<F: std::future::Future>(future: F) -> F::Output {
    tauri::async_runtime::block_on(future)
}

#[cfg(test)]
mod local_relay {
    include!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../core/tests/support/local_relay.rs"
    ));
}

#[cfg(test)]
mod tests {
    use super::{
        block_on,
        local_relay::{LocalRelay, Reply},
    };
    use nostr_sdk::{Client, EventBuilder, Filter, Keys, Kind};
    use std::time::Duration;

    #[test]
    fn relay_query_survives_consumed_setup_callback() {
        let event = EventBuilder::new(Kind::TextNote, "after setup")
            .sign_with_keys(&Keys::generate())
            .expect("signed event");
        let (server, client) = block_on(async {
            let server = LocalRelay::start(Reply::Events(vec![event.clone()])).await;
            let client = Client::default();
            client
                .add_relay(&server.url)
                .await
                .expect("add local relay");
            client.connect().await;
            for relay in client.relays().await.into_values() {
                relay.wait_for_connection(Duration::from_secs(3)).await;
                assert!(relay.is_connected());
            }
            (server, client)
        });

        // Tauri consumes its FnOnce setup callback. Exercise that ownership
        // boundary before issuing any REQ; Connected alone is not evidence.
        let setup_state = String::from("installation recovery");
        let setup = move || {
            block_on(async { tokio::task::yield_now().await });
            drop(setup_state);
        };
        setup();

        block_on(async {
            let relay = client.relay(&server.url).await.expect("local relay");
            let events = relay
                .fetch_events(
                    Filter::new().kind(Kind::TextNote),
                    Duration::from_secs(2),
                    nostr_sdk::pool::relay::ReqExitPolicy::ExitOnEOSE,
                )
                .await
                .expect("live transport after setup");
            assert_eq!(events.into_iter().collect::<Vec<_>>(), vec![event]);
            client.disconnect().await;
        });
    }
}
