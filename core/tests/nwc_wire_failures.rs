#![cfg(feature = "native")]
//! Gate 8 Phase 4C: negative payment outcomes through the real `NwcClient`
//! wire path (NIP-44 requests, signed/correlated responses, timeouts and
//! NIP-47 error codes) into the durable payment journal. The wallet is a
//! private loopback WebSocket fixture; no LND, Alby Hub, ADP or public relay.
use arcadestr_core::{
    durable_payments::{DurableNwcPayments, DurablePaymentError, PaymentWallet},
    nwc_client::{NwcClient, NwcConnection},
    signers::{ActiveSigner, DirectKeySigner},
    storage::Database,
};
use bitcoin::{
    hashes::{sha256, Hash},
    secp256k1::{Secp256k1, SecretKey as BtcSecretKey},
};
use futures_util::{SinkExt, StreamExt};
use lightning_invoice::{Currency, InvoiceBuilder};
use lightning_types::payment::PaymentSecret;
use nostr::{nips::nip44, EventBuilder, Keys, Kind, SecretKey, Tag};
use serde_json::{json, Value};
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio_tungstenite::{accept_async, tungstenite::Message};

const CLIENT_SECRET: &str = "4c1b8b8f5f5e7cd4b8f1e2a8e0f8e9a7c6d5b4a39281706f5e4d3c2b1a090807";
const PREIMAGE: [u8; 32] = [7; 32];
const AMOUNT_MSAT: u64 = 100_000;

fn invoice() -> String {
    // One fixed creation second per process: every call yields the same bolt11.
    static NOW: std::sync::OnceLock<u64> = std::sync::OnceLock::new();
    let now = *NOW.get_or_init(|| nostr::Timestamp::now().as_secs());
    InvoiceBuilder::new(Currency::Regtest)
        .description("phase 4c wire fixture".into())
        .payment_hash(sha256::Hash::hash(&PREIMAGE))
        .payment_secret(PaymentSecret([11; 32]))
        .amount_milli_satoshis(AMOUNT_MSAT)
        .duration_since_epoch(Duration::from_secs(now))
        .expiry_time(Duration::from_secs(600))
        .min_final_cltv_expiry_delta(144)
        .build_signed(|hash| {
            Secp256k1::new()
                .sign_ecdsa_recoverable(hash, &BtcSecretKey::from_slice(&[42; 32]).expect("key"))
        })
        .expect("invoice")
        .to_string()
}

fn payment_hash() -> String {
    sha256::Hash::hash(&PREIMAGE).to_string()
}

fn signer() -> ActiveSigner {
    ActiveSigner::DirectKey(DirectKeySigner::from_key(&"01".repeat(32)).expect("signer"))
}

#[derive(Clone, Copy, PartialEq)]
enum PayMode {
    /// Definitive NIP-47 refusal; the wallet did not pay.
    Refuse,
    /// The wallet pays but its response never reaches the client.
    DropResponse,
    /// A success response signed by a key other than the wallet's.
    ForgedResponse,
}

#[derive(Default)]
struct Log {
    methods: Vec<String>,
    paid: bool,
}

/// Loopback relay + wallet in one: answers each correlated request itself.
async fn wallet_fixture(mode: PayMode) -> (NwcConnection, Arc<Mutex<Log>>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let relay = format!("ws://{}", listener.local_addr().expect("addr"));
    let wallet = Keys::generate();
    let client = Keys::new(SecretKey::from_hex(CLIENT_SECRET).expect("secret"));
    let connection = NwcConnection::parse(&format!(
        "nostr+walletconnect://{}?relay={}&secret={CLIENT_SECRET}",
        wallet.public_key(),
        urlencoding::encode(&relay)
    ))
    .expect("connection");
    let log = Arc::new(Mutex::new(Log::default()));
    let shared = Arc::clone(&log);
    tokio::spawn(async move {
        while let Ok((socket, _)) = listener.accept().await {
            let (wallet, client, log) = (wallet.clone(), client.clone(), Arc::clone(&shared));
            tokio::spawn(async move {
                let Ok(mut ws) = accept_async(socket).await else {
                    return;
                };
                let mut subscription = Value::Null;
                while let Some(Ok(message)) = ws.next().await {
                    let Message::Text(text) = message else {
                        continue;
                    };
                    let value: Value = serde_json::from_str(&text).expect("frame");
                    if value[0] == "REQ" {
                        subscription = value[1].clone();
                        let eose = json!(["EOSE", subscription]).to_string();
                        let _ = ws.send(Message::Text(eose.into())).await;
                        continue;
                    }
                    if value[0] != "EVENT" {
                        continue;
                    }
                    let request: nostr::Event =
                        serde_json::from_value(value[1].clone()).expect("event");
                    request.verify().expect("signed request");
                    let payload: Value = serde_json::from_str(
                        &nip44::decrypt(
                            wallet.secret_key(),
                            &client.public_key(),
                            &request.content,
                        )
                        .expect("decrypt"),
                    )
                    .expect("payload");
                    let method = payload["method"].as_str().unwrap_or_default().to_owned();
                    // Decide under the lock, never hold it across an await.
                    let (response, signer) = {
                        let mut log = log.lock().expect("log");
                        log.methods.push(method.clone());
                        match method.as_str() {
                            "get_info" => (
                                Some(
                                    json!({"result_type":"get_info","result":{"methods":["pay_invoice","lookup_invoice"],"network":"regtest"}}),
                                ),
                                wallet.clone(),
                            ),
                            "pay_invoice" => match mode {
                                PayMode::Refuse => (
                                    Some(
                                        json!({"result_type":"pay_invoice","error":{"code":"PAYMENT_FAILED","message":"no route"}}),
                                    ),
                                    wallet.clone(),
                                ),
                                PayMode::DropResponse => {
                                    log.paid = true;
                                    (None, wallet.clone())
                                }
                                PayMode::ForgedResponse => (
                                    Some(
                                        json!({"result_type":"pay_invoice","result":{"preimage":hex::encode(PREIMAGE),"fees_paid":0}}),
                                    ),
                                    Keys::generate(),
                                ),
                            },
                            "lookup_invoice" if log.paid => (
                                Some(json!({"result_type":"lookup_invoice","result":{
                                    "type":"outgoing","state":"settled","payment_hash":payment_hash(),
                                    "amount":AMOUNT_MSAT,"preimage":hex::encode(PREIMAGE),"fees_paid":0}})),
                                wallet.clone(),
                            ),
                            "lookup_invoice" => (
                                Some(
                                    json!({"result_type":"lookup_invoice","error":{"code":"NOT_FOUND","message":"not found"}}),
                                ),
                                wallet.clone(),
                            ),
                            _ => (None, wallet.clone()),
                        }
                    };
                    let ok = json!(["OK", request.id, true, ""]).to_string();
                    let _ = ws.send(Message::Text(ok.into())).await;
                    let Some(response) = response else {
                        continue;
                    };
                    let content = nip44::encrypt(
                        signer.secret_key(),
                        &client.public_key(),
                        response.to_string(),
                        nip44::Version::V2,
                    )
                    .expect("encrypt");
                    let event = EventBuilder::new(Kind::WalletConnectResponse, content)
                        .tags([Tag::public_key(client.public_key()), Tag::event(request.id)])
                        .sign_with_keys(&signer)
                        .expect("sign");
                    let frame = json!(["EVENT", subscription, event]).to_string();
                    let _ = ws.send(Message::Text(frame.into())).await;
                }
            });
        }
    });
    (connection, log)
}

fn count(log: &Arc<Mutex<Log>>, method: &str) -> usize {
    log.lock()
        .expect("log")
        .methods
        .iter()
        .filter(|m| m.as_str() == method)
        .count()
}

#[tokio::test]
async fn wallet_change_during_preflight_does_not_queue_stale_payment() {
    use arcadestr_core::{nwc_client::NwcClientError, settlement_recovery::RecoveryCoordinator};
    let (connection, log) = wallet_fixture(PayMode::Refuse).await;
    let coordinator = RecoveryCoordinator::default();
    coordinator.activate(Some("buyer".into()));
    let wallet = NwcClient::new(connection).with_timeout(Duration::from_secs(5));
    let stale = wallet
        .clone()
        .with_dispatch_fence(coordinator.wallet_dispatch_fence("buyer").unwrap());
    let (ready, suspended) = tokio::sync::oneshot::channel();
    let (resume, resumed) = tokio::sync::oneshot::channel();
    let task = tokio::spawn(async move {
        stale.capabilities().await.expect("preflight get_info");
        ready.send(()).unwrap();
        resumed.await.unwrap();
        stale.pay_invoice(&invoice()).await
    });
    suspended.await.unwrap();
    coordinator.change_wallet(|| Ok(())).unwrap();
    resume.send(()).unwrap();
    assert!(matches!(
        task.await.unwrap(),
        Err(NwcClientError::StaleWallet)
    ));
    // Positive control: the real fenced enqueue reaches the fixture for a
    // current generation. Only this request may be visible on the wire.
    let current = wallet.with_dispatch_fence(coordinator.wallet_dispatch_fence("buyer").unwrap());
    assert!(matches!(
        current.pay_invoice(&invoice()).await,
        Err(NwcClientError::WalletError { .. })
    ));
    assert_eq!(count(&log, "pay_invoice"), 1);
}

async fn journal(db: &Database) -> (i64, i64, bool) {
    sqlx::query_as(
        "SELECT dispatch_count,lookup_count,encrypted_payment IS NOT NULL FROM nwc_payment_journal",
    )
    .fetch_one(db.pool())
    .await
    .expect("journal")
}

/// Wallet unavailable before dispatch: get_info (the boundary's precondition)
/// fails and nothing is journaled. Once dispatched, a dead transport is
/// unresolved, never a reason to send again.
#[tokio::test]
async fn unreachable_wallet_fails_get_info_and_a_dead_transport_is_never_redispatched() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().expect("addr").port();
    drop(listener);
    let connection = NwcConnection::parse(&format!(
        "nostr+walletconnect://{}?relay={}&secret={CLIENT_SECRET}",
        Keys::generate().public_key(),
        urlencoding::encode(&format!("ws://127.0.0.1:{port}"))
    ))
    .expect("connection");
    let client = NwcClient::new(connection).with_timeout(Duration::from_secs(2));
    assert!(client.capabilities().await.is_err());
    let dir = tempfile::tempdir().expect("tempdir");
    let db = Database::new(&dir.path().join("app.db")).await.expect("db");
    let journaled: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM nwc_payment_journal")
        .fetch_one(db.pool())
        .await
        .expect("count");
    assert_eq!(journaled, 0, "a failed precondition journals nothing");
    let payments = DurableNwcPayments::new(db.pool().clone());
    for _ in 0..2 {
        assert!(matches!(
            payments.pay_or_resume(&signer(), &client, &invoice()).await,
            Err(DurablePaymentError::Unresolved)
        ));
    }
    let (dispatches, lookups, proof) = journal(&db).await;
    assert_eq!((dispatches, lookups, proof), (1, 1, false));
}

/// Definitive refusal (PAYMENT_FAILED on the wire) is still journaled as an
/// unresolved dispatch: later attempts only look up; NOT_FOUND never re-pays.
#[tokio::test]
async fn definitive_wire_refusal_is_unresolved_and_only_looked_up_afterwards() {
    let (connection, log) = wallet_fixture(PayMode::Refuse).await;
    let client = NwcClient::new(connection).with_timeout(Duration::from_secs(5));
    let caps = client.capabilities().await.expect("get_info");
    assert!(format!("{caps:?}").contains("lookup_invoice"));
    let dir = tempfile::tempdir().expect("tempdir");
    let db = Database::new(&dir.path().join("app.db")).await.expect("db");
    let payments = DurableNwcPayments::new(db.pool().clone());
    for _ in 0..3 {
        assert!(matches!(
            payments.pay_or_resume(&signer(), &client, &invoice()).await,
            Err(DurablePaymentError::Unresolved)
        ));
    }
    assert_eq!(count(&log, "pay_invoice"), 1);
    assert_eq!(count(&log, "lookup_invoice"), 2);
    assert_eq!(journal(&db).await, (1, 2, false));
}

/// Response lost after the wallet paid: timeout is unresolved; after a
/// restart, read-only lookup recovers the original proof; the cached proof
/// then answers without any wallet request. Exactly one pay_invoice.
#[tokio::test]
async fn lost_wire_response_reconciles_after_restart_without_second_payment() {
    let (connection, log) = wallet_fixture(PayMode::DropResponse).await;
    let client = NwcClient::new(connection).with_timeout(Duration::from_secs(2));
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("app.db");
    let db = Database::new(&path).await.expect("db");
    assert!(matches!(
        DurableNwcPayments::new(db.pool().clone())
            .pay_or_resume(&signer(), &client, &invoice())
            .await,
        Err(DurablePaymentError::Unresolved)
    ));
    db.pool().close().await;
    let db = Database::new(&path).await.expect("restart");
    let payments = DurableNwcPayments::new(db.pool().clone());
    let recovered = payments
        .reconcile(&signer(), &client, &invoice())
        .await
        .expect("lookup recovers proof");
    assert_eq!(recovered.preimage, hex::encode(PREIMAGE));
    let cached = payments
        .pay_or_resume(&signer(), &client, &invoice())
        .await
        .expect("cached proof");
    assert_eq!(cached, recovered);
    assert_eq!(count(&log, "pay_invoice"), 1);
    assert_eq!(count(&log, "lookup_invoice"), 1);
    assert_eq!(journal(&db).await, (1, 1, true));
}

/// A success response signed by another key is ignored (times out), and a
/// different wallet connection cannot resume or reconcile the journaled intent.
#[tokio::test]
async fn forged_response_and_wallet_switch_never_complete_or_redispatch() {
    let (connection, log) = wallet_fixture(PayMode::ForgedResponse).await;
    let client = NwcClient::new(connection).with_timeout(Duration::from_secs(2));
    let dir = tempfile::tempdir().expect("tempdir");
    let db = Database::new(&dir.path().join("app.db")).await.expect("db");
    let payments = DurableNwcPayments::new(db.pool().clone());
    assert!(matches!(
        payments.pay_or_resume(&signer(), &client, &invoice()).await,
        Err(DurablePaymentError::Unresolved)
    ));
    assert_eq!(journal(&db).await, (1, 0, false));
    let (other_connection, other_log) = wallet_fixture(PayMode::DropResponse).await;
    let other = NwcClient::new(other_connection).with_timeout(Duration::from_secs(2));
    assert_ne!(
        other.identity().expect("identity"),
        client.identity().expect("identity")
    );
    assert!(matches!(
        payments.pay_or_resume(&signer(), &other, &invoice()).await,
        Err(DurablePaymentError::Binding)
    ));
    assert!(matches!(
        payments.reconcile(&signer(), &other, &invoice()).await,
        Err(DurablePaymentError::Binding)
    ));
    assert!(other_log.lock().expect("log").methods.is_empty());
    assert_eq!(count(&log, "pay_invoice"), 1);
}
