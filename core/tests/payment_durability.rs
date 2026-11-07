#![cfg(feature = "native")]
use arcadestr_core::{
    durable_payments::{DurableNwcPayments, DurablePaymentError, PaymentWallet},
    nwc_client::{parse_lookup_payment_response, NwcClientError, PayInvoiceResult},
    signers::{ActiveSigner, DirectKeySigner},
    storage::Database,
};
use async_trait::async_trait;
use bitcoin::{
    hashes::{sha256, Hash},
    secp256k1::{Secp256k1, SecretKey},
};
use lightning_invoice::{Currency, InvoiceBuilder};
use lightning_types::payment::PaymentSecret;
use serde_json::json;
use std::{
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc,
    },
    time::Duration,
};

fn invoice(timestamp: u64) -> String {
    InvoiceBuilder::new(Currency::Regtest)
        .description("durability fixture".into())
        .payment_hash(sha256::Hash::hash(&[7; 32]))
        .payment_secret(PaymentSecret([11; 32]))
        .amount_milli_satoshis(1000)
        .duration_since_epoch(Duration::from_secs(timestamp))
        .expiry_time(Duration::from_secs(600))
        .min_final_cltv_expiry_delta(144)
        .build_signed(|hash| {
            Secp256k1::new()
                .sign_ecdsa_recoverable(hash, &SecretKey::from_slice(&[42; 32]).expect("key"))
        })
        .expect("invoice")
        .to_string()
}
fn signer() -> ActiveSigner {
    ActiveSigner::DirectKey(DirectKeySigner::from_key(&"01".repeat(32)).expect("signer"))
}
fn proof() -> PayInvoiceResult {
    PayInvoiceResult {
        preimage: hex::encode([7; 32]),
        fees_paid_msat: Some(0),
    }
}
struct Wallet {
    calls: AtomicUsize,
    lookups: AtomicUsize,
    paid: AtomicBool,
    lose_response: bool,
    lookup_error: bool,
    invalid_proof: bool,
    id: &'static str,
}
#[async_trait]
impl PaymentWallet for Wallet {
    fn identity(&self) -> Result<String, NwcClientError> {
        Ok(self.id.into())
    }
    async fn pay(&self, _: &str) -> Result<PayInvoiceResult, NwcClientError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.paid.store(true, Ordering::SeqCst);
        if self.lose_response {
            Err(NwcClientError::WalletTimeout)
        } else if self.invalid_proof {
            Ok(PayInvoiceResult {
                preimage: "00".repeat(32),
                fees_paid_msat: Some(0),
            })
        } else {
            Ok(proof())
        }
    }
    async fn lookup(&self, _: &str, _: u64) -> Result<Option<PayInvoiceResult>, NwcClientError> {
        self.lookups.fetch_add(1, Ordering::SeqCst);
        if self.lookup_error {
            Err(NwcClientError::WalletTimeout)
        } else {
            Ok(self.paid.load(Ordering::SeqCst).then(proof))
        }
    }
}
fn wallet(lose_response: bool) -> Wallet {
    Wallet {
        calls: AtomicUsize::new(0),
        lookups: AtomicUsize::new(0),
        paid: AtomicBool::new(false),
        lose_response,
        lookup_error: false,
        invalid_proof: false,
        id: "existing-wallet:existing-app",
    }
}

#[tokio::test]
async fn lost_wallet_response_reopens_database_recovers_preimage_and_never_pays_twice() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("app.db");
    let db = Database::new(&path).await.expect("db");
    let wallet = wallet(true);
    let signer = signer();
    let invoice = invoice(nostr::Timestamp::now().as_secs());
    assert!(matches!(
        DurableNwcPayments::new(db.pool().clone())
            .pay_or_resume(&signer, &wallet, &invoice)
            .await,
        Err(DurablePaymentError::Unresolved)
    ));
    assert_eq!(wallet.calls.load(Ordering::SeqCst), 1);
    db.pool().close().await;
    let db = Database::new(&path).await.expect("restart");
    assert_eq!(
        DurableNwcPayments::new(db.pool().clone())
            .reconcile(&signer, &wallet, &invoice)
            .await
            .expect("recovered proof"),
        proof()
    );
    assert_eq!(
        DurableNwcPayments::new(db.pool().clone())
            .pay_or_resume(&signer, &wallet, &invoice)
            .await
            .expect("cached proof"),
        proof()
    );
    assert_eq!(wallet.calls.load(Ordering::SeqCst), 1);
    assert_eq!(wallet.lookups.load(Ordering::SeqCst), 1);
    let other = ActiveSigner::DirectKey(
        DirectKeySigner::from_key(&"02".repeat(32)).expect("other account"),
    );
    assert!(matches!(
        DurableNwcPayments::new(db.pool().clone())
            .reconcile(&other, &wallet, &invoice)
            .await,
        Err(DurablePaymentError::NoIntent)
    ));
    let blob: Vec<u8> = sqlx::query_scalar("SELECT encrypted_payment FROM nwc_payment_journal")
        .fetch_one(db.pool())
        .await
        .expect("checkpoint");
    assert!(!String::from_utf8_lossy(&blob).contains(&proof().preimage));
    let synchronous: i64 = sqlx::query_scalar("PRAGMA synchronous")
        .fetch_one(db.pool())
        .await
        .expect("synchronous");
    assert_eq!(
        synchronous, 2,
        "payment intent commits require synchronous FULL"
    );
}

#[tokio::test]
async fn concurrent_dispatch_is_claimed_once_and_proofs_are_reused_after_restart() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("app.db");
    let db = Database::new(&path).await.expect("db");
    let wallet = Arc::new(wallet(false));
    let signer = signer();
    let invoice = invoice(nostr::Timestamp::now().as_secs());
    let payments = DurableNwcPayments::new(db.pool().clone());
    let (a, b) = tokio::join!(
        payments.pay_or_resume(&signer, wallet.as_ref(), &invoice),
        payments.pay_or_resume(&signer, wallet.as_ref(), &invoice)
    );
    assert!(a.is_ok() || b.is_ok());
    assert_eq!(wallet.calls.load(Ordering::SeqCst), 1);
    db.pool().close().await;
    let db = Database::new(&path).await.expect("restart");
    assert_eq!(
        DurableNwcPayments::new(db.pool().clone())
            .reconcile(&signer, wallet.as_ref(), &invoice)
            .await
            .expect("stored proof"),
        proof()
    );
    assert_eq!(wallet.calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn unknown_outcomes_wallet_changes_and_resume_without_intent_never_dispatch() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db = Database::new(&dir.path().join("app.db")).await.expect("db");
    let signer = signer();
    let invoice = invoice(nostr::Timestamp::now().as_secs());
    let mut wallet = wallet(true);
    let payments = DurableNwcPayments::new(db.pool().clone());
    assert!(matches!(
        payments.reconcile(&signer, &wallet, &invoice).await,
        Err(DurablePaymentError::NoIntent)
    ));
    assert_eq!(wallet.calls.load(Ordering::SeqCst), 0);
    assert!(payments
        .pay_or_resume(&signer, &wallet, &invoice)
        .await
        .is_err());
    wallet.paid.store(false, Ordering::SeqCst); // NOT_FOUND/pending is not permission to pay again.
    assert!(matches!(
        payments.pay_or_resume(&signer, &wallet, &invoice).await,
        Err(DurablePaymentError::Unresolved)
    ));
    wallet.lookup_error = true;
    assert!(payments
        .reconcile(&signer, &wallet, &invoice)
        .await
        .is_err());
    wallet.id = "another-wallet:another-app";
    assert!(matches!(
        payments.pay_or_resume(&signer, &wallet, &invoice).await,
        Err(DurablePaymentError::Binding)
    ));
    assert_eq!(wallet.calls.load(Ordering::SeqCst), 1);
}

#[test]
fn lookup_requires_outgoing_exact_hash_amount_and_proof_and_never_treats_not_found_as_unpaid() {
    let hash = hex::encode(sha256::Hash::hash(&[7; 32]).to_byte_array());
    let good = json!({"result_type":"lookup_invoice","result":{"type":"outgoing","state":"settled",
        "payment_hash":hash,"amount":1000,"preimage":proof().preimage,"fees_paid":0}});
    assert_eq!(
        parse_lookup_payment_response(&good.to_string(), &hash, 1000).expect("lookup"),
        Some(proof())
    );
    for (key, value) in [
        ("type", json!("incoming")),
        ("payment_hash", json!("00".repeat(32))),
        ("amount", json!(1001)),
        ("preimage", json!("not-a-preimage")),
    ] {
        let mut bad = good.clone();
        bad["result"][key] = value;
        assert!(parse_lookup_payment_response(&bad.to_string(), &hash, 1000).is_err());
    }
    for state in ["pending", "accepted", "expired", "failed"] {
        let mut pending = good.clone();
        pending["result"]["state"] = json!(state);
        assert!(
            parse_lookup_payment_response(&pending.to_string(), &hash, 1000)
                .expect("unresolved")
                .is_none()
        );
    }
    assert!(parse_lookup_payment_response(
        &json!({"result_type":"lookup_invoice","error":{"code":"NOT_FOUND"}}).to_string(),
        &hash,
        1000
    )
    .expect("not found")
    .is_none());
    assert!(!format!("{:?}", proof()).contains(&proof().preimage));
}

#[tokio::test]
async fn committed_proof_survives_restart_and_retries_even_when_wallet_is_offline() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("app.db");
    let db = Database::new(&path).await.expect("db");
    let signer = signer();
    let mut wallet = wallet(false);
    let invoice = invoice(nostr::Timestamp::now().as_secs());
    DurableNwcPayments::new(db.pool().clone())
        .pay_or_resume(&signer, &wallet, &invoice)
        .await
        .expect("paid checkpoint");
    db.pool().close().await;
    wallet.lookup_error = true;
    let db = Database::new(&path).await.expect("restart");
    for _ in 0..3 {
        assert_eq!(
            DurableNwcPayments::new(db.pool().clone())
                .reconcile(&signer, &wallet, &invoice)
                .await
                .expect("durable proof"),
            proof()
        );
    }
    assert_eq!(wallet.calls.load(Ordering::SeqCst), 1);
    assert_eq!(wallet.lookups.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn invalid_wallet_proof_is_not_committed_and_cannot_authorize_a_second_dispatch() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db = Database::new(&dir.path().join("app.db")).await.expect("db");
    let signer = signer();
    let mut wallet = wallet(false);
    wallet.invalid_proof = true;
    let invoice = invoice(nostr::Timestamp::now().as_secs());
    let payments = DurableNwcPayments::new(db.pool().clone());
    assert!(matches!(
        payments.pay_or_resume(&signer, &wallet, &invoice).await,
        Err(DurablePaymentError::InvalidProof)
    ));
    let saved: Option<Vec<u8>> =
        sqlx::query_scalar("SELECT encrypted_payment FROM nwc_payment_journal")
            .fetch_one(db.pool())
            .await
            .expect("marker");
    assert!(saved.is_none());
    payments
        .reconcile(&signer, &wallet, &invoice)
        .await
        .expect("recover correct proof through lookup");
    assert_eq!(wallet.calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn expired_new_invoice_cannot_pay_but_expired_existing_intent_can_reconcile() {
    use arcadestr_core::signers::NostrSigner;
    use sha2::Digest;
    let dir = tempfile::tempdir().expect("tempdir");
    let db = Database::new(&dir.path().join("app.db")).await.expect("db");
    let signer = signer();
    let wallet = wallet(false);
    let invoice = invoice(1);
    let payments = DurableNwcPayments::new(db.pool().clone());
    assert!(matches!(
        payments.pay_or_resume(&signer, &wallet, &invoice).await,
        Err(DurablePaymentError::InvalidInvoice)
    ));
    let account = signer.get_public_key().await.expect("account");
    let encrypted = signer
        .nip44_encrypt(&account, &invoice)
        .await
        .expect("encrypt checkpoint");
    let hash = hex::encode(sha256::Hash::hash(&[7; 32]).to_byte_array());
    sqlx::query("INSERT INTO nwc_payment_journal (account_pubkey,payment_hash,wallet_identity,invoice_commitment,amount_msat,encrypted_invoice,created_at,updated_at) VALUES (?,?,?,?,1000,?,1,1)")
        .bind(account.to_hex()).bind(hash).bind(wallet.identity().expect("wallet"))
        .bind(hex::encode(sha2::Sha256::digest(invoice.as_bytes()))).bind(encrypted.as_bytes())
        .execute(db.pool()).await.expect("historical committed intent");
    assert!(
        matches!(
            payments.pay_or_resume(&signer, &wallet, &invoice).await,
            Err(DurablePaymentError::Unresolved)
        ),
        "a committed intent with no wallet record cannot be dispatched again, even after expiry"
    );
    wallet.paid.store(true, Ordering::SeqCst);
    assert_eq!(
        payments
            .reconcile(&signer, &wallet, &invoice)
            .await
            .expect("expired paid proof recovered"),
        proof()
    );
    assert_eq!(wallet.calls.load(Ordering::SeqCst), 0);
}
