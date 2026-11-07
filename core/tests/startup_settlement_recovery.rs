#![cfg(feature = "native")]
use arcadestr_core::{
    adp_client::SettlementInvoiceResponse,
    adp_settlement::{PaymentLeg, PaymentRole, QuoteState, RefundDestination, SignedQuote},
    durable_payments::{DurableNwcPayments, PaymentWallet},
    nwc_client::{NwcClientError, PayInvoiceResult},
    settlement_recovery::{existing_proof, RecoveryCoordinator},
    settlement_repository::SettlementRepository,
    signers::{ActiveSigner, DirectKeySigner, NostrSigner},
    storage::Database,
};
use async_trait::async_trait;
use bitcoin::{
    hashes::{sha256, Hash},
    secp256k1::{Secp256k1, SecretKey},
};
use lightning_invoice::{Currency, InvoiceBuilder, PaymentSecret};
use std::sync::atomic::{AtomicUsize, Ordering};

struct Wallet {
    pays: AtomicUsize,
    lookups: AtomicUsize,
    settled: bool,
    id: &'static str,
}
#[async_trait]
impl PaymentWallet for Wallet {
    fn identity(&self) -> Result<String, NwcClientError> {
        Ok(self.id.into())
    }
    async fn pay(&self, _: &str) -> Result<PayInvoiceResult, NwcClientError> {
        self.pays.fetch_add(1, Ordering::SeqCst);
        Err(NwcClientError::WalletTimeout)
    }
    async fn lookup(&self, _: &str, _: u64) -> Result<Option<PayInvoiceResult>, NwcClientError> {
        self.lookups.fetch_add(1, Ordering::SeqCst);
        Ok(self.settled.then(|| PayInvoiceResult {
            preimage: hex::encode([7; 32]),
            fees_paid_msat: Some(0),
        }))
    }
}
async fn fixture(path: &std::path::Path) -> (Database, ActiveSigner, SettlementInvoiceResponse) {
    let db = Database::new(path).await.expect("db");
    let signer = ActiveSigner::DirectKey(
        DirectKeySigner::from_key(&"01".repeat(32)).expect("synthetic signer"),
    );
    let account = signer.get_public_key().await.expect("pubkey");
    let bolt11 = InvoiceBuilder::new(Currency::Regtest)
        .description("synthetic startup recovery".into())
        .payment_hash(sha256::Hash::hash(&[7; 32]))
        .payment_secret(PaymentSecret([4; 32]))
        .amount_milli_satoshis(1000)
        .duration_since_epoch(std::time::Duration::from_secs(
            nostr::Timestamp::now().as_secs(),
        ))
        .expiry_time(std::time::Duration::from_secs(600))
        .min_final_cltv_expiry_delta(40)
        .build_signed(|h| {
            Secp256k1::new()
                .sign_ecdsa_recoverable(h, &SecretKey::from_slice(&[42; 32]).expect("key"))
        })
        .expect("invoice")
        .to_string();
    let mut quote = SignedQuote::new(
        "original".into(),
        account.to_hex(),
        "listing".into(),
        "30402:developer:g".into(),
        "processor".into(),
        1000,
        10,
        &RefundDestination {
            endpoint: "buyer@example.com".into(),
            min_sendable_msat: 1,
            max_sendable_msat: 2000,
        },
        vec![PaymentLeg {
            leg_id: "dev".into(),
            role: PaymentRole::Developer,
            recipient_pubkey: "developer".into(),
            destination: "dev@example.com".into(),
            amount_msat: 1000,
            accepted_terms_id: None,
        }],
    )
    .expect("quote");
    quote.state = QuoteState::Active;
    let invoice = SettlementInvoiceResponse {
        quote_id: "original".into(),
        attempt_id: "original-attempt".into(),
        leg_id: "dev".into(),
        amount_msat: 1000,
        payment_hash: sha256::Hash::hash(&[7; 32]).to_string(),
        bolt11,
        expires_at: u32::MAX as u64,
    };
    let repo = SettlementRepository::new(db.pool().clone());
    repo.save_quote(
        &account.to_hex(),
        "https://processor",
        "original-signed-quote",
        &quote,
        10,
    )
    .await
    .expect("quote");
    let encrypted = signer
        .nip44_encrypt(&account, &serde_json::to_string(&invoice).expect("JSON"))
        .await
        .expect("encrypt");
    repo.record_invoice_attempt(
        &account.to_hex(),
        "original",
        "dev",
        "original-attempt",
        encrypted.as_bytes(),
        &invoice.payment_hash,
        "disclosed",
        11,
    )
    .await
    .expect("attempt");
    (db, signer, invoice)
}
#[tokio::test]
async fn undispatched_and_malformed_checkpoints_cannot_start_payment() {
    let dir = tempfile::tempdir().expect("dir");
    let (db, signer, mut invoice) = fixture(&dir.path().join("db")).await;
    let account = signer.get_public_key().await.expect("key");
    let repo = SettlementRepository::new(db.pool().clone());
    assert!(repo
        .recovery_attempts(&account.to_hex())
        .await
        .expect("queue")
        .is_empty());
    invoice.amount_msat += 1;
    let encrypted = signer
        .nip44_encrypt(&account, &serde_json::to_string(&invoice).expect("JSON"))
        .await
        .expect("encrypt");
    sqlx::query("UPDATE settlement_invoice_attempts SET encrypted_attempt=?")
        .bind(encrypted.as_bytes())
        .execute(db.pool())
        .await
        .expect("malformed checkpoint fixture");
    let wallet = Wallet {
        pays: AtomicUsize::new(0),
        lookups: AtomicUsize::new(0),
        settled: true,
        id: "original-wallet:original-app",
    };
    assert!(existing_proof(
        &repo,
        &DurableNwcPayments::new(db.pool().clone()),
        &signer,
        Some(&wallet),
        "original",
        "original-attempt"
    )
    .await
    .is_err());
    assert_eq!(wallet.pays.load(Ordering::SeqCst), 0);
    assert_eq!(wallet.lookups.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn session_fence_blocks_refund_intent_and_dispatch() {
    let dir = tempfile::tempdir().expect("dir");
    let (db, signer, invoice) = fixture(&dir.path().join("db")).await;
    let account = signer.get_public_key().await.expect("key");
    let coordinator = RecoveryCoordinator::default();
    coordinator.activate(Some(account.to_hex()));
    let lease = coordinator
        .claim(&account.to_hex(), "refund")
        .expect("lease");
    coordinator.activate(None);
    let wallet = Wallet {
        pays: AtomicUsize::new(0),
        lookups: AtomicUsize::new(0),
        settled: false,
        id: "original-wallet:original-app",
    };
    assert!(DurableNwcPayments::new(db.pool().clone())
        .pay_or_resume_guarded(&signer, &wallet, &invoice.bolt11, &|| lease
            .ensure_current())
        .await
        .is_err());
    assert_eq!(wallet.pays.load(Ordering::SeqCst), 0);
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM nwc_payment_journal")
        .fetch_one(db.pool())
        .await
        .expect("journal");
    assert_eq!(count, 0);
}

#[tokio::test]
async fn paid_checkpoint_recovers_offline_and_completed_receipt_dominates_stale_legs() {
    let dir = tempfile::tempdir().expect("dir");
    let path = dir.path().join("db");
    let (db, signer, _) = fixture(&path).await;
    let account = signer.get_public_key().await.expect("key");
    let repo = SettlementRepository::new(db.pool().clone());
    let proof = signer
        .nip44_encrypt(&account, &hex::encode([7; 32]))
        .await
        .expect("proof");
    repo.checkpoint_proof(
        &account.to_hex(),
        "original",
        "original-attempt",
        proof.as_bytes(),
    )
    .await
    .expect("checkpoint");
    db.pool().close().await;
    let db = Database::new(&path).await.expect("restart");
    let repo = SettlementRepository::new(db.pool().clone());
    let payments = DurableNwcPayments::new(db.pool().clone());
    assert_eq!(
        existing_proof(
            &repo,
            &payments,
            &signer,
            None,
            "original",
            "original-attempt"
        )
        .await
        .expect("offline cached recovery")
        .1,
        hex::encode([7; 32])
    );
    repo.mark_receipt(&account.to_hex(), "original", "validated-paid-receipt", 20)
        .await
        .expect("receipt");
    sqlx::query("UPDATE settlement_legs SET state='pending'")
        .execute(db.pool())
        .await
        .expect("historical stale projection fault");
    assert!(repo
        .recovery_attempts(&account.to_hex())
        .await
        .expect("queue")
        .is_empty());
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM nwc_payment_journal")
        .fetch_one(db.pool())
        .await
        .expect("journal");
    assert_eq!(count, 0, "startup never creates a payment intent");
}
#[tokio::test]
async fn uncertain_restart_not_found_and_changed_wallet_remain_blocked_without_redispatch() {
    let dir = tempfile::tempdir().expect("dir");
    let path = dir.path().join("db");
    let (db, signer, invoice) = fixture(&path).await;
    let wallet = Wallet {
        pays: AtomicUsize::new(0),
        lookups: AtomicUsize::new(0),
        settled: false,
        id: "original-wallet:original-app",
    };
    assert!(DurableNwcPayments::new(db.pool().clone())
        .pay_or_resume(&signer, &wallet, &invoice.bolt11)
        .await
        .is_err());
    db.pool().close().await;
    let db = Database::new(&path).await.expect("restart");
    let repo = SettlementRepository::new(db.pool().clone());
    let payments = DurableNwcPayments::new(db.pool().clone());
    assert!(existing_proof(
        &repo,
        &payments,
        &signer,
        None,
        "original",
        "original-attempt"
    )
    .await
    .is_err());
    for _ in 0..2 {
        assert!(existing_proof(
            &repo,
            &payments,
            &signer,
            Some(&wallet),
            "original",
            "original-attempt"
        )
        .await
        .is_err());
    }
    let changed = Wallet {
        pays: AtomicUsize::new(0),
        lookups: AtomicUsize::new(0),
        settled: true,
        id: "different-app",
    };
    assert!(existing_proof(
        &repo,
        &payments,
        &signer,
        Some(&changed),
        "original",
        "original-attempt"
    )
    .await
    .is_err());
    assert_eq!(wallet.pays.load(Ordering::SeqCst), 1);
    assert_eq!(wallet.lookups.load(Ordering::SeqCst), 2);
    assert_eq!(changed.pays.load(Ordering::SeqCst), 0);
    assert_eq!(changed.lookups.load(Ordering::SeqCst), 0);
    let counts: (i64, i64, Option<Vec<u8>>) = sqlx::query_as(
        "SELECT dispatch_count,lookup_count,encrypted_payment FROM nwc_payment_journal",
    )
    .fetch_one(db.pool())
    .await
    .expect("audit");
    assert_eq!(counts, (1, 2, None));
}
#[tokio::test]
async fn reconciled_proof_remains_bound_to_original_account_when_session_changes() {
    let dir = tempfile::tempdir().expect("dir");
    let (db, signer, invoice) = fixture(&dir.path().join("db")).await;
    let account = signer.get_public_key().await.expect("key");
    let c = RecoveryCoordinator::default();
    c.activate(Some(account.to_hex()));
    let lease = c.claim(&account.to_hex(), "original").expect("lease");
    let mut wallet = Wallet {
        pays: AtomicUsize::new(0),
        lookups: AtomicUsize::new(0),
        settled: false,
        id: "original-wallet:original-app",
    };
    let payments = DurableNwcPayments::new(db.pool().clone());
    assert!(payments
        .pay_or_resume(&signer, &wallet, &invoice.bolt11)
        .await
        .is_err());
    wallet.settled = true;
    c.activate(Some("another-account".into()));
    let repo = SettlementRepository::new(db.pool().clone());
    existing_proof(
        &repo,
        &payments,
        &signer,
        Some(&wallet),
        "original",
        "original-attempt",
    )
    .await
    .expect("read-only proof encrypted to initiating account");
    assert!(
        lease.ensure_current().is_err(),
        "caller must not submit after a session change"
    );
    assert!(repo
        .invoice_attempt("another-account", "original", "original-attempt")
        .await
        .expect("scope")
        .is_none());
    assert_eq!(wallet.pays.load(Ordering::SeqCst), 1);
    assert_eq!(wallet.lookups.load(Ordering::SeqCst), 1);
}
