//! Black-box ADP-01 interoperability against the real `adp-server` binary.
//!
//! Unlike `crossflow`, nothing here touches adp-server internals: the server is a
//! separate process configured only through its documented environment, and every
//! assertion goes through ADP HTTP endpoints or Nostr events observed on two
//! loopback relays. Each run uses a private temporary directory, dynamically
//! allocated ports and no public relay; the process is killed on drop, including
//! on assertion failure.
//!
//! `ADP_SERVER_BIN` must name the built server binary (`scripts/interop-test`
//! sets it).

#[allow(dead_code)]
mod fixtures;

use arcadestr_core::{
    adp_client::AdpClient, adp_discovery::parse_adp_server_announcements,
    authorization::parse_attestation_event,
};
use fixtures::{relay, signer, Events};
use nostr_sdk::{Event, Keys};
use std::{
    process::Stdio,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

const STARTUP_TIMEOUT: Duration = Duration::from_secs(60);

struct RunningServer {
    url: String,
    operator: Keys,
    child: tokio::process::Child,
    _dir: tempfile::TempDir,
}

impl RunningServer {
    async fn start(relays: &[String]) -> Self {
        let binary = std::env::var_os("ADP_SERVER_BIN")
            .expect("ADP_SERVER_BIN must name the adp-server binary; use scripts/interop-test");
        let dir = tempfile::tempdir().expect("private server directory");
        let port = {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("free port");
            listener.local_addr().expect("address").port()
        };
        let url = format!("http://127.0.0.1:{port}");
        let operator = Keys::generate();
        let child = tokio::process::Command::new(binary)
            .current_dir(dir.path())
            .env_clear()
            .env("PATH", std::env::var_os("PATH").unwrap_or_default())
            .env("RUST_LOG", "warn")
            .env("ADP_OPERATOR_NSEC", operator.secret_key().to_secret_hex())
            .env("ADP_MASTER_KEY", hex::encode([23u8; 32]))
            .env("ADP_SERVER_URL", &url)
            .env("ADP_BIND_ADDR", format!("127.0.0.1:{port}"))
            .env("ADP_DB_PATH", dir.path().join("adp.db"))
            .env("ADP_FILES_DIR", dir.path().join("files"))
            .env("ADP_RELAYS", relays.join(","))
            .env("ADP_PUBLISHER_RELAYS", "disabled")
            .stdin(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .expect("spawn adp-server");
        let mut server = Self {
            url,
            operator,
            child,
            _dir: dir,
        };
        server.wait_ready().await;
        server
    }

    async fn wait_ready(&mut self) {
        let deadline = Instant::now() + STARTUP_TIMEOUT;
        let health = format!("{}/health", self.url);
        loop {
            if let Some(status) = self.child.try_wait().expect("server status") {
                panic!("adp-server exited during startup: {status}");
            }
            if let Ok(response) = reqwest::get(&health).await {
                if response.status().is_success() {
                    return;
                }
            }
            assert!(Instant::now() < deadline, "adp-server did not become ready");
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }
}

fn stored(stores: &[Events]) -> Vec<Event> {
    stores
        .iter()
        .flat_map(|store| store.lock().expect("relay store").clone())
        .collect()
}

#[tokio::test]
async fn real_server_announces_and_provisions_through_adp_http_and_nostr() {
    let stores: Vec<Events> = vec![Arc::new(Mutex::new(vec![])), Arc::new(Mutex::new(vec![]))];
    let (relay_a, _relay_a) = relay(stores[0].clone()).await;
    let (relay_b, _relay_b) = relay(stores[1].clone()).await;
    let server = RunningServer::start(&[relay_a, relay_b]).await;
    let operator = server.operator.public_key();
    let client = AdpClient::new(server.url.clone(), fixtures::http());

    // Discovery: the startup kind:30403 announcement reached the local relays and
    // Arcadestr's own parser resolves it to this operator and URL.
    let announcements = parse_adp_server_announcements(stored(&stores));
    assert!(
        announcements
            .iter()
            .any(|a| a.url == server.url && a.pubkey == operator.to_hex()),
        "server announcement must be discoverable on the configured relays"
    );

    let info = client.well_known().await.expect("well-known");
    assert!(!info.adp_version.is_empty());
    assert_eq!(info.pubkey, operator.to_hex(), "well-known operator key");

    // NIP-98 is mandatory for provisioning; no anonymous fallback.
    let anonymous = reqwest::Client::new()
        .post(format!("{}/provision", server.url))
        .json(&serde_json::json!({ "scope": "anonymous" }))
        .send()
        .await
        .expect("anonymous provision request");
    assert_eq!(anonymous.status(), reqwest::StatusCode::UNAUTHORIZED);

    let developer = Keys::generate();
    let developer_signer = signer(&developer);
    let first = client
        .provision(&developer_signer, Some("black-box"))
        .await
        .expect("first provision");
    let second = client
        .provision(&developer_signer, Some("black-box"))
        .await
        .expect("idempotent provision");
    assert_eq!(first.scope.as_deref(), Some("black-box"));
    assert_eq!(second.scope.as_deref(), Some("black-box"));
    assert_eq!(first.fulfillment_pubkey, second.fulfillment_pubkey);
    assert_eq!(first.attestation_event_id, second.attestation_event_id);

    // The attestation the server published verifies under Arcadestr's parser and
    // binds the authenticated developer, this operator and the returned key.
    let published = stored(&stores)
        .into_iter()
        .find(|event| event.id.to_hex() == first.attestation_event_id)
        .expect("attestation published to a configured relay");
    assert_eq!(published.pubkey, operator, "attestation signed by operator");
    let parsed = parse_attestation_event(&published).expect("valid ADP attestation");
    assert_eq!(parsed.developer_pubkey, developer.public_key());
    assert_eq!(parsed.fulfillment_pubkey.to_hex(), first.fulfillment_pubkey);
    assert_eq!(parsed.scope.as_deref(), Some("black-box"));
    assert_eq!(parsed.revoked_at, None);
    if let Some(returned) = first.attestation {
        assert_eq!(returned.id, published.id, "response carries the same event");
    }

    // A different developer never receives another developer's key.
    let other = signer(&Keys::generate());
    let other_key = client
        .provision(&other, Some("black-box"))
        .await
        .expect("other developer provision");
    assert_ne!(other_key.fulfillment_pubkey, first.fulfillment_pubkey);
}
