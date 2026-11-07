use super::*;
use futures_util::{SinkExt, StreamExt};
use keyring::credential::CredentialApi;
use std::sync::Arc;

const SECRET: &str = "0000000000000000000000000000000000000000000000000000000000000002";

#[test]
#[cfg(debug_assertions)]
fn laboratory_transport_is_opt_in_loopback_only_and_does_not_change_wallet_binding() {
    let connection =
        NwcConnection::parse(&uri("ws://127.0.0.1:7778", SECRET)).expect("fixture connection");
    let original = connection.clone();
    let identity = NwcClient::new(connection.clone())
        .payment_identity()
        .expect("identity");
    assert_eq!(
        regtest_transport_relays(&connection, None, false).expect("default"),
        connection.relay_urls()
    );
    assert_eq!(
        regtest_transport_relays(&connection, Some("ws://127.0.0.1:43178"), true)
            .expect("explicit local proxy"),
        vec!["ws://127.0.0.1:43178"]
    );
    assert!(regtest_transport_relays(&connection, Some("ws://127.0.0.1:43178"), false).is_err());
    for value in [
        "wss://example.com",
        "ws://example.com",
        "ws://127.0.0.1.evil.test",
        "ws://user@127.0.0.1:43178",
        "ws://127.0.0.1:43178?secret=x",
    ] {
        assert!(regtest_transport_relays(&connection, Some(value), true).is_err());
    }
    let remote =
        NwcConnection::parse(&uri("wss://example.com", SECRET)).expect("normal remote connection");
    assert!(regtest_transport_relays(&remote, Some("ws://127.0.0.1:43178"), true).is_err());
    assert_eq!(connection, original);
    assert_eq!(
        NwcClient::new(connection)
            .payment_identity()
            .expect("identity"),
        identity
    );
}

fn uri(relay: &str, secret: &str) -> String {
    format!(
        "nostr+walletconnect://{}?relay={}&secret={secret}",
        Keys::generate().public_key(),
        urlencoding::encode(relay)
    )
}

// Each reconstructed Entry represents a new application session, with the same backing store.
// No global credential builder is replaced and no real user's credentials are touched.
struct SharedCredential(Arc<keyring::mock::MockCredential>);
impl CredentialApi for SharedCredential {
    fn set_secret(&self, value: &[u8]) -> keyring::Result<()> {
        self.0.set_secret(value)
    }
    fn get_secret(&self) -> keyring::Result<Vec<u8>> {
        self.0.get_secret()
    }
    fn set_password(&self, value: &str) -> keyring::Result<()> {
        self.0.set_password(value)
    }
    fn get_password(&self) -> keyring::Result<String> {
        self.0.get_password()
    }
    fn delete_credential(&self) -> keyring::Result<()> {
        self.0.delete_credential()
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
    fn debug_fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SharedCredential(<redacted>)")
    }
}
fn entry(store: &Arc<keyring::mock::MockCredential>) -> keyring::Entry {
    keyring::Entry::new_with_credential(Box::new(SharedCredential(store.clone())))
}

#[test]
fn nwc_account_keyring_entries_are_isolated_canonical_and_persistent() {
    use std::collections::HashMap;
    let a = Keys::generate().public_key();
    let b = Keys::generate().public_key();
    let mut stores = HashMap::new();
    for account in [a, b] {
        stores.insert(
            account_nwc_entry_name(&account),
            Arc::new(keyring::mock::MockCredential::default()),
        );
    }
    let a_key = account_nwc_entry_name(&a);
    let b_key = account_nwc_entry_name(&b);
    assert_ne!(a_key, b_key);
    assert_ne!(a_key, "default-wallet-connect-uri");
    let canonical_a = PublicKey::from_hex(&a.to_hex().to_uppercase()).unwrap();
    assert_eq!(account_nwc_entry_name(&canonical_a), a_key);
    let saved_a = save_nwc_entry(&entry(&stores[&a_key]), &uri("wss://a.example", SECRET)).unwrap();
    assert!(load_nwc_entry(&entry(&stores[&b_key])).unwrap().is_none());
    let saved_b = save_nwc_entry(&entry(&stores[&b_key]), &uri("wss://b.example", SECRET)).unwrap();
    assert_eq!(
        load_nwc_entry(&entry(&stores[&a_key])).unwrap(),
        Some(saved_a)
    );
    remove_nwc_entry(&entry(&stores[&a_key])).unwrap();
    assert!(load_nwc_entry(&entry(&stores[&a_key])).unwrap().is_none());
    assert_eq!(
        load_nwc_entry(&entry(&stores[&b_key])).unwrap(),
        Some(saved_b)
    );
}

#[test]
fn nwc_save_load_replace_restart_remove_and_failures() {
    let store = Arc::new(keyring::mock::MockCredential::default());
    let first = entry(&store);
    assert!(load_nwc_entry(&first).unwrap().is_none());
    let original = uri("wss://wallet.example/", SECRET);
    let expected = save_nwc_entry(&first, &original).unwrap();
    drop(first);
    let restarted = entry(&store);
    assert_eq!(load_nwc_entry(&restarted).unwrap(), Some(expected.clone()));
    assert!(save_nwc_entry(&restarted, "not an NWC URI").is_err());
    assert_eq!(load_nwc_entry(&restarted).unwrap(), Some(expected));
    let replacement = uri("wss://replacement.example/", SECRET);
    let next = save_nwc_entry(&restarted, &replacement).unwrap();
    assert_eq!(load_nwc_entry(&entry(&store)).unwrap(), Some(next));
    store.set_error(keyring::Error::NoStorageAccess(Box::new(
        std::io::Error::other("locked"),
    )));
    assert!(matches!(
        load_nwc_entry(&restarted),
        Err(NwcClientError::Keyring(_))
    ));
    store.set_error(keyring::Error::NoStorageAccess(Box::new(
        std::io::Error::other("locked"),
    )));
    assert!(remove_nwc_entry(&restarted).is_err());
    assert!(load_nwc_entry(&restarted).unwrap().is_some());
    remove_nwc_entry(&restarted).unwrap();
    remove_nwc_entry(&entry(&store)).unwrap();
    assert!(load_nwc_entry(&entry(&store)).unwrap().is_none());
}

#[test]
fn nwc_failed_replacement_keeps_saved_credentials_and_corrupt_entry_can_be_removed() {
    let store = Arc::new(keyring::mock::MockCredential::default());
    let credential = entry(&store);
    let original = uri("wss://wallet.example/", SECRET);
    let expected = save_nwc_entry(&credential, &original).unwrap();
    store.set_error(keyring::Error::NoStorageAccess(Box::new(
        std::io::Error::other("locked"),
    )));
    assert!(save_nwc_entry(&credential, &uri("wss://other.example/", SECRET)).is_err());
    assert_eq!(load_nwc_entry(&credential).unwrap(), Some(expected));
    credential
        .set_password("malformed saved credential")
        .unwrap();
    assert!(matches!(
        load_nwc_entry(&credential),
        Err(NwcClientError::InvalidUri)
    ));
    remove_nwc_entry(&credential).unwrap();
    assert!(load_nwc_entry(&credential).unwrap().is_none());
}

#[test]
fn nwc_malformed_and_relay_security_policy() {
    for invalid in [
        "",
        "https://example.com",
        "nostr+walletconnect://invalid?secret=bad",
        "nostr+walletconnect://",
    ] {
        assert!(NwcConnection::parse(invalid).is_err());
    }
    let valid = uri("wss://relay.example/", SECRET);
    assert!(NwcConnection::parse(&valid.replace(&format!("&secret={SECRET}"), "")).is_err());
    assert!(NwcConnection::parse(&valid.replace("relay=", "other=")).is_err());
    assert!(NwcConnection::parse(&valid.replace(SECRET, &"0".repeat(64))).is_err());
    for relay in [
        "ws://127.0.0.1:7778",
        "ws://localhost:7778",
        "ws://[::1]:7778",
    ] {
        assert_eq!(
            NwcConnection::parse(&uri(relay, SECRET)).is_ok(),
            cfg!(debug_assertions)
        );
    }
    for relay in [
        "ws://public.example",
        "ws://localhost.evil.example",
        "ws://127.0.0.1.evil.example",
        "http://127.0.0.1:7778",
        "wss://user:password@relay.example",
        "",
    ] {
        assert!(NwcConnection::parse(&uri(relay, SECRET)).is_err());
    }
}

#[test]
fn nwc_metadata_and_debug_hide_secret_and_relay_tokens() {
    let value = uri(
        &format!("wss://wallet.example/{SECRET}?token={SECRET}"),
        SECRET,
    );
    let connection =
        NwcConnection::parse(&format!("{value}&lud16={SECRET}%40example.com")).unwrap();
    assert_eq!(
        connection.safe_relay_metadata(),
        vec!["wss://wallet.example"]
    );
    assert_eq!(connection.lud16(), None);
    assert!(!format!("{connection:?}").contains(SECRET));
}

// Private local WebSocket wallet fixture. It decrypts real requests, signs correlated
// responses and never contacts LND, Alby Hub, ADP, or a public relay.
async fn local_wallet(unsupported: bool) -> (NwcConnection, tokio::task::JoinHandle<Vec<String>>) {
    use serde_json::{json, Value};
    use tokio_tungstenite::{accept_async, tungstenite::Message};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let relay = format!("ws://{}", listener.local_addr().unwrap());
    let wallet = Keys::generate();
    let client = Keys::new(SecretKey::from_hex(SECRET).unwrap());
    let connection = NwcConnection::parse(&format!(
        "nostr+walletconnect://{}?relay={}&secret={SECRET}",
        wallet.public_key(),
        urlencoding::encode(&relay)
    ))
    .unwrap();
    let task = tokio::spawn(async move {
        let mut methods = Vec::new();
        for _ in 0..3 {
            let (socket, _) = listener.accept().await.unwrap();
            let mut ws = accept_async(socket).await.unwrap();
            let mut subscription = Value::Null;
            while let Some(Ok(message)) = ws.next().await {
                let Message::Text(text) = message else {
                    continue;
                };
                let value: Value = serde_json::from_str(&text).unwrap();
                if value[0] == "REQ" {
                    subscription = value[1].clone();
                    ws.send(Message::Text(
                        json!(["EOSE", subscription]).to_string().into(),
                    ))
                    .await
                    .unwrap();
                }
                if value[0] != "EVENT" {
                    continue;
                }
                let request: Event = serde_json::from_value(value[1].clone()).unwrap();
                request.verify().unwrap();
                let plaintext =
                    nip44::decrypt(wallet.secret_key(), &client.public_key(), &request.content)
                        .unwrap();
                let payload: Value = serde_json::from_str(&plaintext).unwrap();
                let method = payload["method"].as_str().unwrap();
                methods.push(method.to_owned());
                let response = if method == "get_info" && unsupported {
                    json!({"result_type":"get_info", "error":{"code":"NOT_IMPLEMENTED", "message":"unsupported"}})
                } else if method == "get_info" {
                    json!({"result_type":"get_info", "result":{"methods":["pay_invoice"]}})
                } else {
                    assert_eq!(method, "pay_invoice");
                    assert!(payload["params"].get("amount").is_none());
                    json!({"result_type":"pay_invoice", "result":{"preimage":"test-preimage", "fees_paid":0}})
                };
                let content = nip44::encrypt(
                    wallet.secret_key(),
                    &client.public_key(),
                    response.to_string(),
                    nip44::Version::V2,
                )
                .unwrap();
                let event = EventBuilder::new(Kind::WalletConnectResponse, content)
                    .tags([Tag::public_key(client.public_key()), Tag::event(request.id)])
                    .sign_with_keys(&wallet)
                    .unwrap();
                ws.send(Message::Text(
                    json!(["OK", request.id, true, ""]).to_string().into(),
                ))
                .await
                .unwrap();
                ws.send(Message::Text(
                    json!(["EVENT", subscription, event]).to_string().into(),
                ))
                .await
                .unwrap();
                // Keep the transport open until the client has consumed the response.
            }
        }
        methods
    });
    (connection, task)
}

#[tokio::test]
async fn nwc_loopback_verification_and_single_and_split_invoice_payments() {
    for unsupported in [false, true] {
        let (connection, task) = local_wallet(unsupported).await;
        let client = NwcClient::new(connection).with_timeout(Duration::from_secs(5));
        let result = client.verify().await;
        if unsupported {
            assert!(matches!(
                result,
                Err(NwcClientError::VerificationUnsupported)
            ));
        } else {
            result.unwrap();
        }
        // Same payment operation for legacy invoice and current committed split leg.
        assert_eq!(
            client
                .pay_invoice("legacy-fixed-invoice")
                .await
                .unwrap()
                .preimage,
            "test-preimage"
        );
        assert_eq!(
            client
                .pay_invoice("split-leg-fixed-invoice")
                .await
                .unwrap()
                .preimage,
            "test-preimage"
        );
        assert_eq!(
            task.await.unwrap(),
            ["get_info", "pay_invoice", "pay_invoice"]
        );
    }
}
