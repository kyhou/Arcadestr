#![cfg(feature = "native")]
use arcadestr_core::{
    adp_client::SettlementInvoiceResponse,
    adp_settlement::{
        PaymentLeg, PaymentRole, QuoteState, RefundDestination, SignedQuote, QUOTE_KIND,
    },
    durable_payments::PaymentWallet,
    nwc_client::{NwcClientError, PayInvoiceResult},
    purchase_execution::{InvoiceNetwork, PurchaseExecutor, PurchaseLimits},
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
use nostr::{EventBuilder, JsonUtil, Keys, Kind};
use std::{
    collections::BTreeMap,
    str::FromStr,
    sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
    time::Duration,
};

struct Wallet {
    calls: AtomicUsize,
    lost: AtomicBool,
    denied: AtomicBool,
    hang: AtomicBool,
    known: AtomicBool,
    visible: AtomicBool,
    omit_fees: AtomicBool,
    fees: AtomicU64,
    name: &'static str,
}
#[async_trait]
impl PaymentWallet for Wallet {
    fn identity(&self) -> Result<String, NwcClientError> {
        Ok(self.name.into())
    }
    fn maximum_fee_msat(&self, _: &str) -> Option<u64> {
        self.known.load(Ordering::SeqCst).then_some(10000)
    }
    async fn pay(&self, invoice: &str) -> Result<PayInvoiceResult, NwcClientError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if self.denied.load(Ordering::SeqCst) {
            return Err(NwcClientError::WalletError {
                code: "UNAUTHORIZED".into(),
                message: "mock read-only connection".into(),
            });
        }
        if self.hang.load(Ordering::SeqCst) {
            std::future::pending::<()>().await;
        }
        if self.lost.load(Ordering::SeqCst) {
            return Err(NwcClientError::WalletTimeout);
        }
        let mut result = result(invoice);
        result.fees_paid_msat = if self.omit_fees.load(Ordering::SeqCst) {
            None
        } else {
            Some(self.fees.load(Ordering::SeqCst))
        };
        Ok(result)
    }
    async fn lookup(&self, hash: &str, _: u64) -> Result<Option<PayInvoiceResult>, NwcClientError> {
        let byte = if hash == sha256::Hash::hash(&[7; 32]).to_string() {
            7
        } else {
            8
        };
        Ok(self
            .visible
            .load(Ordering::SeqCst)
            .then(|| PayInvoiceResult {
                preimage: hex::encode([byte; 32]),
                fees_paid_msat: if self.omit_fees.load(Ordering::SeqCst) {
                    None
                } else {
                    Some(self.fees.load(Ordering::SeqCst))
                },
            }))
    }
}
fn result(invoice: &str) -> PayInvoiceResult {
    use std::str::FromStr;
    let parsed = lightning_invoice::Bolt11Invoice::from_str(invoice).expect("test vector");
    let byte = if parsed.payment_hash().to_string() == sha256::Hash::hash(&[7; 32]).to_string() {
        7
    } else {
        8
    };
    PayInvoiceResult {
        preimage: hex::encode([byte; 32]),
        fees_paid_msat: Some(0),
    }
}
struct Fixture {
    _dir: tempfile::TempDir,
    db: Database,
    signer: ActiveSigner,
    quote: SignedQuote,
    event_id: String,
    invoices: Vec<SettlementInvoiceResponse>,
    wallet: Wallet,
}
async fn fixture() -> Fixture {
    fixture_with_expired(false).await
}
async fn fixture_with_expired(expired: bool) -> Fixture {
    let dir = tempfile::tempdir().expect("tempdir");
    let db = Database::new(&dir.path().join("app.db")).await.expect("db");
    let signer = ActiveSigner::DirectKey(DirectKeySigner::from_key(&"01".repeat(32)).expect("key"));
    let buyer = signer.get_public_key().await.expect("buyer");
    let processor = Keys::parse(&"02".repeat(32)).expect("processor");
    let now = nostr::Timestamp::now().as_secs();
    let mut quote = SignedQuote::new(
        "purchase".into(),
        buyer.to_hex(),
        "03".repeat(32),
        "30402:developer:test".into(),
        processor.public_key().to_hex(),
        200000,
        now,
        &RefundDestination {
            endpoint: "buyer@example.test".into(),
            min_sendable_msat: 1,
            max_sendable_msat: 200000,
        },
        vec![
            PaymentLeg {
                leg_id: "developer".into(),
                role: PaymentRole::Developer,
                recipient_pubkey: "04".repeat(32),
                destination: "bob@example.test".into(),
                amount_msat: 100000,
                accepted_terms_id: None,
            },
            PaymentLeg {
                leg_id: "operator".into(),
                role: PaymentRole::Operator,
                recipient_pubkey: processor.public_key().to_hex(),
                destination: "carol@example.test".into(),
                amount_msat: 100000,
                accepted_terms_id: Some("05".repeat(32)),
            },
        ],
    )
    .expect("quote");
    quote.state = QuoteState::Active;
    let plaintext = serde_json::to_string(&quote).expect("json");
    let encrypted = nostr::nips::nip44::encrypt(
        processor.secret_key(),
        &buyer,
        plaintext,
        nostr::nips::nip44::Version::V2,
    )
    .expect("encrypt");
    let event = EventBuilder::new(Kind::Custom(QUOTE_KIND), encrypted)
        .sign_with_keys(&processor)
        .expect("event");
    let repo = SettlementRepository::new(db.pool().clone());
    repo.save_quote(
        &buyer.to_hex(),
        "https://processor.test",
        &event.as_json(),
        &quote,
        now,
    )
    .await
    .expect("persist");
    let mut invoices = vec![];
    for (i, leg) in quote.legs.iter().enumerate() {
        let invoice_now = if expired { now - 1000 } else { now };
        let byte = 7 + i as u8;
        let bolt11 = InvoiceBuilder::new(Currency::Regtest)
            .description("offline purchase vector".into())
            .payment_hash(sha256::Hash::hash(&[byte; 32]))
            .payment_secret(PaymentSecret([9; 32]))
            .amount_milli_satoshis(100000)
            .duration_since_epoch(Duration::from_secs(invoice_now))
            .expiry_time(Duration::from_secs(600))
            .min_final_cltv_expiry_delta(144)
            .build_signed(|msg| {
                Secp256k1::new().sign_ecdsa_recoverable(
                    msg,
                    &SecretKey::from_slice(&[42 + i as u8; 32]).expect("key"),
                )
            })
            .expect("invoice")
            .to_string();
        let invoice = SettlementInvoiceResponse {
            quote_id: quote.quote_id.clone(),
            attempt_id: format!("attempt-{i}"),
            leg_id: leg.leg_id.clone(),
            amount_msat: 100000,
            payment_hash: sha256::Hash::hash(&[byte; 32]).to_string(),
            bolt11,
            expires_at: invoice_now + 600,
        };
        let encrypted = signer
            .nip44_encrypt(&buyer, &serde_json::to_string(&invoice).expect("json"))
            .await
            .expect("encrypt");
        repo.record_invoice_attempt(
            &buyer.to_hex(),
            &quote.quote_id,
            &leg.leg_id,
            &invoice.attempt_id,
            encrypted.as_bytes(),
            &invoice.payment_hash,
            "disclosed",
            now,
        )
        .await
        .expect("persist");
        invoices.push(invoice);
    }
    Fixture {
        _dir: dir,
        db,
        signer,
        quote,
        event_id: event.id.to_hex(),
        invoices,
        wallet: Wallet {
            calls: AtomicUsize::new(0),
            lost: AtomicBool::new(false),
            denied: AtomicBool::new(false),
            hang: AtomicBool::new(false),
            known: AtomicBool::new(true),
            visible: AtomicBool::new(false),
            omit_fees: AtomicBool::new(false),
            fees: AtomicU64::new(0),
            name: "standard-provider-A",
        },
    }
}

#[tokio::test]
async fn stored_invoice_is_not_buyer_approval() {
    let f = fixture().await;
    assert!(PurchaseExecutor::new(f.db.pool().clone())
        .pay_leg(&f.signer, &f.wallet, &f.quote.quote_id, "attempt-0")
        .await
        .is_err());
    assert_eq!(f.wallet.calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn missing_wallet_permission_never_retries_or_expands_an_existing_admission() {
    let f = fixture().await;
    approve(&f).await;
    f.wallet.denied.store(true, Ordering::SeqCst);
    let executor = PurchaseExecutor::new(f.db.pool().clone());
    assert!(executor
        .pay_leg(&f.signer, &f.wallet, &f.quote.quote_id, "attempt-0")
        .await
        .is_err());
    f.wallet.denied.store(false, Ordering::SeqCst);
    assert!(executor
        .pay_leg(&f.signer, &f.wallet, &f.quote.quote_id, "attempt-0")
        .await
        .is_err());
    assert!(executor
        .pay_leg(&f.signer, &f.wallet, &f.quote.quote_id, "attempt-1")
        .await
        .is_err());
    assert_eq!(f.wallet.calls.load(Ordering::SeqCst), 1);
    let review = executor
        .review(&f.signer, Some(&f.wallet), &f.quote.quote_id)
        .await
        .expect("safe status");
    assert!(!review.purchase_completed);
    assert_eq!(
        review.legs[0].progress,
        arcadestr_core::purchase_contract::LegProgress::Uncertain
    );
}

fn limits() -> PurchaseLimits {
    PurchaseLimits {
        principal_msat: 200000,
        fee_reserve_msat: 50000,
        total_msat: 250000,
        maximum_dispatches: 2,
    }
}
fn payees(f: &Fixture) -> BTreeMap<String, String> {
    f.invoices
        .iter()
        .map(|i| {
            (
                i.leg_id.clone(),
                lightning_invoice::Bolt11Invoice::from_str(&i.bolt11)
                    .expect("vector")
                    .recover_payee_pub_key()
                    .to_string(),
            )
        })
        .collect()
}
async fn approve(f: &Fixture) {
    PurchaseExecutor::new(f.db.pool().clone())
        .approve(
            &f.signer,
            &f.wallet,
            &f.quote.quote_id,
            &f.event_id,
            limits(),
            InvoiceNetwork::Regtest,
            payees(f),
        )
        .await
        .expect("explicit test buyer approval")
}
async fn acknowledge(f: &Fixture, attempt: &str) {
    let buyer = f.signer.get_public_key().await.expect("buyer");
    SettlementRepository::new(f.db.pool().clone())
        .update_invoice_state(
            &buyer.to_hex(),
            attempt,
            "settled",
            nostr::Timestamp::now().as_secs(),
        )
        .await
        .expect("simulate independently accepted server proof");
}

#[tokio::test]
async fn approval_is_immutable_and_two_original_legs_do_not_redispatch() {
    let f = fixture().await;
    approve(&f).await;
    approve(&f).await;
    let executor = PurchaseExecutor::new(f.db.pool().clone());
    let mut changed = limits();
    changed.total_msat += 1;
    assert!(executor
        .approve(
            &f.signer,
            &f.wallet,
            &f.quote.quote_id,
            &f.event_id,
            changed,
            InvoiceNetwork::Regtest,
            payees(&f)
        )
        .await
        .is_err());
    executor
        .pay_leg(&f.signer, &f.wallet, &f.quote.quote_id, "attempt-0")
        .await
        .expect("first");
    assert!(
        executor
            .pay_leg(&f.signer, &f.wallet, &f.quote.quote_id, "attempt-1")
            .await
            .is_err(),
        "server must acknowledge the first leg before another payment"
    );
    acknowledge(&f, "attempt-0").await;
    executor
        .pay_leg(&f.signer, &f.wallet, &f.quote.quote_id, "attempt-1")
        .await
        .expect("second");
    executor
        .pay_leg(&f.signer, &f.wallet, &f.quote.quote_id, "attempt-0")
        .await
        .expect("cached original proof");
    assert_eq!(f.wallet.calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn uncertainty_halts_purchase_across_restart_and_recovery_is_read_only() {
    let f = fixture().await;
    approve(&f).await;
    f.wallet.lost.store(true, Ordering::SeqCst);
    let executor = PurchaseExecutor::new(f.db.pool().clone());
    assert!(executor
        .pay_leg(&f.signer, &f.wallet, &f.quote.quote_id, "attempt-0")
        .await
        .is_err());
    f.db.pool().close().await;
    let db = Database::new(&f._dir.path().join("app.db"))
        .await
        .expect("restart");
    let executor = PurchaseExecutor::new(db.pool().clone());
    assert!(executor
        .pay_leg(&f.signer, &f.wallet, &f.quote.quote_id, "attempt-1")
        .await
        .is_err());
    assert!(executor
        .reconcile_leg(&f.signer, &f.wallet, &f.quote.quote_id, "attempt-0")
        .await
        .is_err());
    assert_eq!(f.wallet.calls.load(Ordering::SeqCst), 1);
    f.wallet.visible.store(true, Ordering::SeqCst);
    executor
        .reconcile_leg(&f.signer, &f.wallet, &f.quote.quote_id, "attempt-0")
        .await
        .expect("original proof");
    assert_eq!(f.wallet.calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn unknown_provider_fee_exposure_fails_before_dispatch() {
    let f = fixture().await;
    approve(&f).await;
    f.wallet.known.store(false, Ordering::SeqCst);
    assert!(PurchaseExecutor::new(f.db.pool().clone())
        .pay_leg(&f.signer, &f.wallet, &f.quote.quote_id, "attempt-0")
        .await
        .is_err());
    assert_eq!(f.wallet.calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn parallel_purchase_attempts_admit_at_most_one_unresolved_leg() {
    let f = fixture().await;
    approve(&f).await;
    f.wallet.lost.store(true, Ordering::SeqCst);
    let a = PurchaseExecutor::new(f.db.pool().clone());
    let b = PurchaseExecutor::new(f.db.pool().clone());
    let _ = tokio::join!(
        a.pay_leg(&f.signer, &f.wallet, &f.quote.quote_id, "attempt-0"),
        b.pay_leg(&f.signer, &f.wallet, &f.quote.quote_id, "attempt-1")
    );
    assert_eq!(f.wallet.calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn plan_recipient_network_and_wallet_are_independent_bindings() {
    let f = fixture().await;
    let executor = PurchaseExecutor::new(f.db.pool().clone());
    let mut wrong = payees(&f);
    wrong.insert("developer".into(), wrong["operator"].clone());
    executor
        .approve(
            &f.signer,
            &f.wallet,
            &f.quote.quote_id,
            &f.event_id,
            limits(),
            InvoiceNetwork::Regtest,
            wrong,
        )
        .await
        .expect("explicit plan with different payee");
    assert!(executor
        .pay_leg(&f.signer, &f.wallet, &f.quote.quote_id, "attempt-0")
        .await
        .is_err());
    assert_eq!(f.wallet.calls.load(Ordering::SeqCst), 0);
    let f = fixture().await;
    let executor = PurchaseExecutor::new(f.db.pool().clone());
    executor
        .approve(
            &f.signer,
            &f.wallet,
            &f.quote.quote_id,
            &f.event_id,
            limits(),
            InvoiceNetwork::Bitcoin,
            payees(&f),
        )
        .await
        .expect("different network plan");
    assert!(executor
        .pay_leg(&f.signer, &f.wallet, &f.quote.quote_id, "attempt-0")
        .await
        .is_err());
    let mut f = fixture().await;
    approve(&f).await;
    f.wallet.name = "unrelated-standard-provider";
    assert!(PurchaseExecutor::new(f.db.pool().clone())
        .pay_leg(&f.signer, &f.wallet, &f.quote.quote_id, "attempt-0")
        .await
        .is_err());
}

#[tokio::test]
async fn replacement_attempt_cannot_replace_original_disclosed_leg() {
    let f = fixture().await;
    approve(&f).await;
    let buyer = f.signer.get_public_key().await.expect("buyer");
    let mut changed = f.invoices[0].clone();
    changed.attempt_id = "replacement".into();
    changed.payment_hash = sha256::Hash::hash(&[12; 32]).to_string();
    changed.bolt11 = InvoiceBuilder::new(Currency::Regtest)
        .description("replacement offline vector".into())
        .payment_hash(sha256::Hash::hash(&[12; 32]))
        .payment_secret(PaymentSecret([9; 32]))
        .amount_milli_satoshis(100000)
        .duration_since_epoch(Duration::from_secs(nostr::Timestamp::now().as_secs()))
        .expiry_time(Duration::from_secs(600))
        .min_final_cltv_expiry_delta(144)
        .build_signed(|msg| {
            Secp256k1::new()
                .sign_ecdsa_recoverable(msg, &SecretKey::from_slice(&[42; 32]).expect("key"))
        })
        .expect("vector")
        .to_string();
    let encrypted = f
        .signer
        .nip44_encrypt(&buyer, &serde_json::to_string(&changed).expect("json"))
        .await
        .expect("encrypt");
    SettlementRepository::new(f.db.pool().clone())
        .record_invoice_attempt(
            &buyer.to_hex(),
            &f.quote.quote_id,
            &changed.leg_id,
            &changed.attempt_id,
            encrypted.as_bytes(),
            &changed.payment_hash,
            "disclosed",
            nostr::Timestamp::now().as_secs(),
        )
        .await
        .expect("store history, not authority");
    assert!(PurchaseExecutor::new(f.db.pool().clone())
        .pay_leg(&f.signer, &f.wallet, &f.quote.quote_id, "replacement")
        .await
        .is_err());
    assert_eq!(f.wallet.calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn checked_budgets_reject_expansion_and_fee_violations_halt_remaining_legs() {
    let f = fixture().await;
    let executor = PurchaseExecutor::new(f.db.pool().clone());
    for changed in [
        PurchaseLimits {
            total_msat: 249999,
            ..limits()
        },
        PurchaseLimits {
            principal_msat: u64::MAX,
            ..limits()
        },
        PurchaseLimits {
            maximum_dispatches: 3,
            ..limits()
        },
    ] {
        assert!(executor
            .approve(
                &f.signer,
                &f.wallet,
                &f.quote.quote_id,
                &f.event_id,
                changed,
                InvoiceNetwork::Regtest,
                payees(&f)
            )
            .await
            .is_err());
    }
    approve(&f).await;
    f.wallet.fees.store(10001, Ordering::SeqCst);
    assert!(executor
        .pay_leg(&f.signer, &f.wallet, &f.quote.quote_id, "attempt-0")
        .await
        .is_err());
    assert!(executor
        .pay_leg(&f.signer, &f.wallet, &f.quote.quote_id, "attempt-1")
        .await
        .is_err());
    assert_eq!(f.wallet.calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn quote_projection_cannot_rewrite_approved_recipients() {
    let f = fixture().await;
    approve(&f).await;
    let buyer = f.signer.get_public_key().await.expect("buyer");
    let repo = SettlementRepository::new(f.db.pool().clone());
    let saved = repo
        .load(&buyer.to_hex(), &f.quote.quote_id)
        .await
        .expect("load")
        .expect("quote");
    let mut changed = f.quote.clone();
    changed.legs[0].recipient_pubkey = "06".repeat(32);
    assert!(repo
        .save_quote(
            &buyer.to_hex(),
            &saved.processor_url,
            &saved.quote_event_json,
            &changed,
            nostr::Timestamp::now().as_secs()
        )
        .await
        .is_err());
}

#[tokio::test]
async fn interruption_before_wallet_dispatch_keeps_a_closed_gate_after_restart() {
    let f = fixture().await;
    approve(&f).await;
    let entered = AtomicUsize::new(0);
    let guard = || {
        if entered.fetch_add(1, Ordering::SeqCst) == 0 {
            Ok(())
        } else {
            Err("simulated session termination".into())
        }
    };
    assert!(PurchaseExecutor::new(f.db.pool().clone())
        .pay_leg_guarded(&f.signer, &f.wallet, &f.quote.quote_id, "attempt-0", &guard)
        .await
        .is_err());
    assert_eq!(f.wallet.calls.load(Ordering::SeqCst), 0);
    f.db.pool().close().await;
    let db = Database::new(&f._dir.path().join("app.db"))
        .await
        .expect("restart");
    let executor = PurchaseExecutor::new(db.pool().clone());
    assert!(executor
        .pay_leg(&f.signer, &f.wallet, &f.quote.quote_id, "attempt-0")
        .await
        .is_err());
    assert!(executor
        .pay_leg(&f.signer, &f.wallet, &f.quote.quote_id, "attempt-1")
        .await
        .is_err());
    assert_eq!(f.wallet.calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn cancelled_wallet_await_recovers_without_redispatch() {
    let f = fixture().await;
    approve(&f).await;
    f.wallet.hang.store(true, Ordering::SeqCst);
    let executor = PurchaseExecutor::new(f.db.pool().clone());
    assert!(tokio::time::timeout(
        Duration::from_secs(1),
        executor.pay_leg(&f.signer, &f.wallet, &f.quote.quote_id, "attempt-0")
    )
    .await
    .is_err());
    assert_eq!(f.wallet.calls.load(Ordering::SeqCst), 1);
    f.db.pool().close().await;
    let db = Database::new(&f._dir.path().join("app.db"))
        .await
        .expect("restart");
    f.wallet.visible.store(true, Ordering::SeqCst);
    PurchaseExecutor::new(db.pool().clone())
        .reconcile_leg(&f.signer, &f.wallet, &f.quote.quote_id, "attempt-0")
        .await
        .expect("read-only original proof");
    assert_eq!(f.wallet.calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn duplicate_hash_and_stale_invoice_are_not_dispatch_authority() {
    let f = fixture().await;
    approve(&f).await;
    let buyer = f.signer.get_public_key().await.expect("buyer");
    let mut duplicate = f.invoices[0].clone();
    duplicate.attempt_id = "duplicate".into();
    duplicate.leg_id = "operator".into();
    let encrypted = f
        .signer
        .nip44_encrypt(&buyer, &serde_json::to_string(&duplicate).expect("json"))
        .await
        .expect("encrypt");
    assert!(SettlementRepository::new(f.db.pool().clone())
        .record_invoice_attempt(
            &buyer.to_hex(),
            &f.quote.quote_id,
            "operator",
            "duplicate",
            encrypted.as_bytes(),
            &duplicate.payment_hash,
            "disclosed",
            nostr::Timestamp::now().as_secs()
        )
        .await
        .is_err());
    assert_eq!(f.wallet.calls.load(Ordering::SeqCst), 0);
    let stale = fixture_with_expired(true).await;
    approve(&stale).await;
    assert!(PurchaseExecutor::new(stale.db.pool().clone())
        .pay_leg(
            &stale.signer,
            &stale.wallet,
            &stale.quote.quote_id,
            "attempt-0"
        )
        .await
        .is_err());
    assert_eq!(stale.wallet.calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn provider_independence_and_receipt_retry_never_repeat_payments() {
    let mut f = fixture().await;
    f.wallet.name = "unrelated-NWC-provider-B";
    approve(&f).await;
    let executor = PurchaseExecutor::new(f.db.pool().clone());
    executor
        .pay_leg(&f.signer, &f.wallet, &f.quote.quote_id, "attempt-0")
        .await
        .expect("first");
    acknowledge(&f, "attempt-0").await;
    executor
        .pay_leg(&f.signer, &f.wallet, &f.quote.quote_id, "attempt-1")
        .await
        .expect("second");
    // Receipt submission/publication has not completed: return original proofs,
    // never re-run Lightning payments while the existing receipt is recovered.
    executor
        .reconcile_leg(&f.signer, &f.wallet, &f.quote.quote_id, "attempt-0")
        .await
        .expect("proof zero");
    executor
        .reconcile_leg(&f.signer, &f.wallet, &f.quote.quote_id, "attempt-1")
        .await
        .expect("proof one");
    assert_eq!(f.wallet.calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn changed_operator_terms_cannot_rebind_an_in_progress_purchase() {
    let f = fixture().await;
    approve(&f).await;
    let buyer = f.signer.get_public_key().await.expect("buyer");
    let repo = SettlementRepository::new(f.db.pool().clone());
    let mut changed = f.quote.clone();
    changed.legs[1].accepted_terms_id = Some("06".repeat(32));
    let processor = Keys::parse(&"02".repeat(32)).expect("test key");
    let text = serde_json::to_string(&changed).expect("json");
    let encrypted = nostr::nips::nip44::encrypt(
        processor.secret_key(),
        &buyer,
        text,
        nostr::nips::nip44::Version::V2,
    )
    .expect("encrypt");
    let event = EventBuilder::new(Kind::Custom(QUOTE_KIND), encrypted)
        .sign_with_keys(&processor)
        .expect("signed alternate snapshot");
    assert!(repo
        .save_quote(
            &buyer.to_hex(),
            "https://processor.test",
            &event.as_json(),
            &changed,
            nostr::Timestamp::now().as_secs()
        )
        .await
        .is_err());
    PurchaseExecutor::new(f.db.pool().clone())
        .pay_leg(&f.signer, &f.wallet, &f.quote.quote_id, "attempt-0")
        .await
        .expect("original immutable plan only");
    assert_eq!(f.wallet.calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn another_account_cannot_execute_the_buyers_plan() {
    let f = fixture().await;
    approve(&f).await;
    let other = ActiveSigner::DirectKey(
        DirectKeySigner::from_key(&"03".repeat(32)).expect("different test buyer"),
    );
    assert!(PurchaseExecutor::new(f.db.pool().clone())
        .pay_leg(&other, &f.wallet, &f.quote.quote_id, "attempt-0")
        .await
        .is_err());
    assert_eq!(f.wallet.calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn missing_fee_visibility_blocks_further_dispatch_until_read_only_lookup() {
    let f = fixture().await;
    approve(&f).await;
    f.wallet.omit_fees.store(true, Ordering::SeqCst);
    let executor = PurchaseExecutor::new(f.db.pool().clone());
    assert!(executor
        .pay_leg(&f.signer, &f.wallet, &f.quote.quote_id, "attempt-0")
        .await
        .is_err());
    assert!(executor
        .pay_leg(&f.signer, &f.wallet, &f.quote.quote_id, "attempt-1")
        .await
        .is_err());
    assert_eq!(f.wallet.calls.load(Ordering::SeqCst), 1);
    f.wallet.omit_fees.store(false, Ordering::SeqCst);
    f.wallet.visible.store(true, Ordering::SeqCst);
    executor
        .reconcile_leg(&f.signer, &f.wallet, &f.quote.quote_id, "attempt-0")
        .await
        .expect("independently recovered fees");
    assert_eq!(f.wallet.calls.load(Ordering::SeqCst), 1);
}

async fn verified_fixture() -> Fixture {
    use arcadestr_core::adp_settlement::{
        build_payment_legs, build_terms_event_builder, CommercialTerms, FeePolicy, TermsPayload,
    };
    use nostr::Tag;
    let dir = tempfile::tempdir().expect("tempdir");
    let db = Database::new(&dir.path().join("app.db")).await.expect("db");
    let signer =
        ActiveSigner::DirectKey(DirectKeySigner::from_key(&"01".repeat(32)).expect("buyer"));
    let buyer = signer.get_public_key().await.expect("buyer");
    let developer = Keys::parse(&"04".repeat(32)).expect("developer");
    let operator = Keys::parse(&"02".repeat(32)).expect("operator");
    let terms = CommercialTerms {
        version: 1,
        operator_pubkey: operator.public_key().to_hex(),
        payment_destination: "carol@example.test".into(),
        fee: FeePolicy::Fixed {
            amount_msat: 100000,
        },
        limitations: vec![],
    };
    let root = build_terms_event_builder(&TermsPayload::Offer {
        terms: terms.clone(),
    })
    .expect("root")
    .sign_with_keys(&operator)
    .expect("sign");
    let listing = EventBuilder::new(Kind::Custom(30402), "offline listing")
        .tags([
            Tag::parse(["d", "verified"]).expect("tag"),
            Tag::parse(["price", "200", "SATS"]).expect("tag"),
            Tag::parse(["lud16", "bob@example.test"]).expect("tag"),
            Tag::parse([
                "adp_terms",
                &operator.public_key().to_hex(),
                &root.id.to_hex(),
            ])
            .expect("tag"),
        ])
        .sign_with_keys(&developer)
        .expect("listing");
    let now = nostr::Timestamp::now().as_secs();
    let legs = build_payment_legs(
        200000,
        &developer.public_key().to_hex(),
        "bob@example.test",
        &[(terms, root.id.to_hex())],
    )
    .expect("legs");
    let mut quote = SignedQuote::new(
        "verified-purchase".into(),
        buyer.to_hex(),
        listing.id.to_hex(),
        format!("30402:{}:verified", developer.public_key().to_hex()),
        operator.public_key().to_hex(),
        200000,
        now,
        &RefundDestination {
            endpoint: "buyer@example.test".into(),
            min_sendable_msat: 1,
            max_sendable_msat: 200000,
        },
        legs,
    )
    .expect("quote");
    quote.state = QuoteState::Active;
    let ciphertext = nostr::nips::nip44::encrypt(
        operator.secret_key(),
        &buyer,
        serde_json::to_string(&quote).expect("json"),
        nostr::nips::nip44::Version::V2,
    )
    .expect("encrypt");
    let event = EventBuilder::new(Kind::Custom(QUOTE_KIND), ciphertext)
        .sign_with_keys(&operator)
        .expect("event");
    SettlementRepository::new(db.pool().clone())
        .save_quote(
            &buyer.to_hex(),
            "https://processor.test",
            &event.as_json(),
            &quote,
            now,
        )
        .await
        .expect("save");
    PurchaseExecutor::new(db.pool().clone())
        .verify_sources(&signer, &quote.quote_id, &listing, &[root])
        .await
        .expect("local source verification");
    let mut invoices = Vec::new();
    for (i, leg) in quote.legs.iter().enumerate() {
        let byte = 7 + i as u8;
        let bolt11 = InvoiceBuilder::new(Currency::Regtest)
            .description("verified offline vector".into())
            .payment_hash(sha256::Hash::hash(&[byte; 32]))
            .payment_secret(PaymentSecret([9; 32]))
            .amount_milli_satoshis(100000)
            .duration_since_epoch(Duration::from_secs(now))
            .expiry_time(Duration::from_secs(600))
            .min_final_cltv_expiry_delta(144)
            .build_signed(|msg| {
                Secp256k1::new().sign_ecdsa_recoverable(
                    msg,
                    &SecretKey::from_slice(&[42 + i as u8; 32]).expect("key"),
                )
            })
            .expect("vector")
            .to_string();
        invoices.push(SettlementInvoiceResponse {
            quote_id: quote.quote_id.clone(),
            attempt_id: format!("attempt-{i}"),
            leg_id: leg.leg_id.clone(),
            amount_msat: 100000,
            payment_hash: sha256::Hash::hash(&[byte; 32]).to_string(),
            bolt11,
            expires_at: now + 600,
        });
    }
    let f = Fixture {
        _dir: dir,
        db,
        signer,
        quote,
        event_id: event.id.to_hex(),
        invoices,
        wallet: Wallet {
            calls: AtomicUsize::new(0),
            lost: AtomicBool::new(false),
            denied: AtomicBool::new(false),
            hang: AtomicBool::new(false),
            known: AtomicBool::new(true),
            visible: AtomicBool::new(false),
            omit_fees: AtomicBool::new(false),
            fees: AtomicU64::new(0),
            name: "standard-provider-A",
        },
    };
    disclose(&f, 0).await;
    f
}
async fn disclose(f: &Fixture, index: usize) {
    let buyer = f.signer.get_public_key().await.expect("buyer");
    let invoice = &f.invoices[index];
    let encrypted = f
        .signer
        .nip44_encrypt(&buyer, &serde_json::to_string(invoice).expect("json"))
        .await
        .expect("encrypt");
    SettlementRepository::new(f.db.pool().clone())
        .record_invoice_attempt(
            &buyer.to_hex(),
            &f.quote.quote_id,
            &invoice.leg_id,
            &invoice.attempt_id,
            encrypted.as_bytes(),
            &invoice.payment_hash,
            "disclosed",
            nostr::Timestamp::now().as_secs(),
        )
        .await
        .expect("disclose original response");
}

#[tokio::test]
async fn verified_review_approval_and_deferred_original_invoices_complete_without_atomicity_claims()
{
    use arcadestr_core::purchase_contract::LegProgress;
    let f = verified_fixture().await;
    let executor = PurchaseExecutor::new(f.db.pool().clone());
    let review = executor
        .review(&f.signer, Some(&f.wallet), &f.quote.quote_id)
        .await
        .expect("review");
    assert!(review.terms_verified);
    assert!(!review.approved);
    assert!(!review.purchase_completed);
    assert_eq!(review.legs[1].invoice_payee, None);
    executor
        .approve_verified(
            &f.signer,
            &f.wallet,
            &f.quote.quote_id,
            &review.quote_event_id,
            &review.commitment,
            limits(),
            InvoiceNetwork::Regtest,
        )
        .await
        .expect("explicit complete-plan approval");
    assert!(
        executor
            .review(&f.signer, Some(&f.wallet), &f.quote.quote_id)
            .await
            .expect("review")
            .approved
    );
    executor
        .pay_leg(&f.signer, &f.wallet, &f.quote.quote_id, "attempt-0")
        .await
        .expect("first");
    let partial = executor
        .review(&f.signer, Some(&f.wallet), &f.quote.quote_id)
        .await
        .expect("partial");
    assert_eq!(partial.legs[0].progress, LegProgress::Paid);
    assert!(!partial.purchase_completed);
    acknowledge(&f, "attempt-0").await;
    disclose(&f, 1).await;
    executor
        .pay_leg(&f.signer, &f.wallet, &f.quote.quote_id, "attempt-1")
        .await
        .expect("original second disclosure");
    let pending_receipt = executor
        .review(&f.signer, Some(&f.wallet), &f.quote.quote_id)
        .await
        .expect("status");
    assert!(!pending_receipt.purchase_completed);
    assert_eq!(f.wallet.calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn unverified_or_stale_reviews_never_create_execution_authority() {
    let unverified = fixture().await;
    let executor = PurchaseExecutor::new(unverified.db.pool().clone());
    let review = executor
        .review(
            &unverified.signer,
            Some(&unverified.wallet),
            &unverified.quote.quote_id,
        )
        .await
        .expect("historical review");
    assert!(!review.terms_verified);
    assert!(executor
        .approve_verified(
            &unverified.signer,
            &unverified.wallet,
            &unverified.quote.quote_id,
            &review.quote_event_id,
            &review.commitment,
            limits(),
            InvoiceNetwork::Regtest
        )
        .await
        .is_err());
    let mut f = verified_fixture().await;
    let executor = PurchaseExecutor::new(f.db.pool().clone());
    let review = executor
        .review(&f.signer, Some(&f.wallet), &f.quote.quote_id)
        .await
        .expect("review");
    f.wallet.name = "different-provider-or-app";
    assert!(executor
        .approve_verified(
            &f.signer,
            &f.wallet,
            &f.quote.quote_id,
            &review.quote_event_id,
            &review.commitment,
            limits(),
            InvoiceNetwork::Regtest
        )
        .await
        .is_err());
    assert_eq!(f.wallet.calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn unresolved_admissions_remain_visible_to_startup_after_server_proof_acceptance() {
    let f = fixture().await;
    approve(&f).await;
    let entered = AtomicUsize::new(0);
    let guard = || {
        if entered.fetch_add(1, Ordering::SeqCst) == 0 {
            Ok(())
        } else {
            Err("interrupted before low-level intent".into())
        }
    };
    let executor = PurchaseExecutor::new(f.db.pool().clone());
    assert!(executor
        .pay_leg_guarded(&f.signer, &f.wallet, &f.quote.quote_id, "attempt-0", &guard)
        .await
        .is_err());
    acknowledge(&f, "attempt-0").await;
    let buyer = f.signer.get_public_key().await.expect("buyer");
    let queue = SettlementRepository::new(f.db.pool().clone())
        .recovery_attempts(&buyer.to_hex())
        .await
        .expect("queue");
    assert!(queue.iter().any(|v| v.attempt_id == "attempt-0"));
    assert_eq!(f.wallet.calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn completion_requires_a_validated_bound_aggregate_receipt_not_a_status_flag() {
    use arcadestr_core::{
        adp_settlement::{aggregate_commitment, AggregateProofLeg},
        purchases::{
            parse_and_validate_receipt_with_authorization, PurchasesRepository, ReceiptEvidence,
        },
    };
    use nostr::Tag;
    let f = verified_fixture().await;
    let buyer = f.signer.get_public_key().await.expect("buyer");
    SettlementRepository::new(f.db.pool().clone())
        .mark_receipt(
            &buyer.to_hex(),
            &f.quote.quote_id,
            r#"{"unverified":"server status"}"#,
            nostr::Timestamp::now().as_secs(),
        )
        .await
        .expect("corrupt status fixture");
    assert!(
        !PurchaseExecutor::new(f.db.pool().clone())
            .review(&f.signer, Some(&f.wallet), &f.quote.quote_id)
            .await
            .expect("review")
            .purchase_completed
    );
    let f = verified_fixture().await;
    let buyer = f.signer.get_public_key().await.expect("buyer");
    let proofs: Vec<_> = f
        .quote
        .legs
        .iter()
        .zip(&f.invoices)
        .map(|(leg, invoice)| AggregateProofLeg {
            leg_id: leg.leg_id.clone(),
            role: leg.role.clone(),
            recipient_pubkey: leg.recipient_pubkey.clone(),
            amount_msat: leg.amount_msat,
            payment_hash: invoice.payment_hash.clone(),
            proof_type: "bolt11-preimage".into(),
            proof_reference: invoice.payment_hash.clone(),
            accepted_terms_id: leg.accepted_terms_id.clone(),
        })
        .collect();
    let commitment = aggregate_commitment(200000, &proofs).expect("aggregate");
    let mut tags = vec![
        vec!["order".into(), f.quote.quote_id.clone()],
        vec!["p".into(), buyer.to_hex()],
        vec!["a".into(), f.quote.listing_coordinate.clone()],
        vec!["status".into(), "paid".into()],
        vec!["sale_listing".into(), f.quote.listing_event_id.clone()],
        vec!["sale_quote".into(), f.event_id.clone()],
        vec!["sale_processor".into(), f.quote.processor_pubkey.clone()],
        vec![
            "payment_set".into(),
            "1".into(),
            "200000".into(),
            commitment,
        ],
        vec![
            "settled_at".into(),
            nostr::Timestamp::now().as_secs().to_string(),
        ],
    ];
    for proof in proofs {
        tags.push(vec![
            "payment_leg".into(),
            proof.leg_id,
            if proof.role == PaymentRole::Developer {
                "developer"
            } else {
                "operator"
            }
            .into(),
            proof.recipient_pubkey,
            proof.amount_msat.to_string(),
            proof.payment_hash,
            proof.proof_type,
            proof.proof_reference,
            proof.accepted_terms_id.unwrap_or_default(),
        ]);
    }
    let issuer = Keys::parse(&"04".repeat(32)).expect("synthetic developer");
    let event = EventBuilder::new(Kind::Custom(1020), "offline receipt fixture")
        .tags(tags.into_iter().map(|v| Tag::parse(v).expect("tag")))
        .sign_with_keys(&issuer)
        .expect("receipt");
    let raw = event.as_json();
    let executor = PurchaseExecutor::new(f.db.pool().clone());
    assert!(!executor
        .validated_completion(&buyer.to_hex(), &f.quote, &f.event_id, Some(&raw))
        .await
        .expect("not yet validated"));
    let verified = parse_and_validate_receipt_with_authorization(
        &event,
        &buyer.to_hex(),
        None,
        ReceiptEvidence::default(),
    )
    .expect("authorized aggregate fixture");
    PurchasesRepository::new(f.db.pool().clone())
        .upsert_receipt(&verified)
        .await
        .expect("persist validated receipt");
    SettlementRepository::new(f.db.pool().clone())
        .mark_receipt(
            &buyer.to_hex(),
            &f.quote.quote_id,
            &raw,
            nostr::Timestamp::now().as_secs(),
        )
        .await
        .expect("mark");
    assert!(
        executor
            .review(&f.signer, Some(&f.wallet), &f.quote.quote_id)
            .await
            .expect("completed")
            .purchase_completed
    );
    assert_eq!(
        f.wallet.calls.load(Ordering::SeqCst),
        0,
        "this is a receipt-verification fixture, not live payment evidence"
    );
}

#[tokio::test]
async fn standard_wallet_reported_fee_mode_requires_separate_risk_consent() {
    use arcadestr_core::purchase_contract::RoutingFeePolicy;
    let f = verified_fixture().await;
    f.wallet.known.store(false, Ordering::SeqCst);
    let executor = PurchaseExecutor::new(f.db.pool().clone());
    let review = executor
        .review(&f.signer, Some(&f.wallet), &f.quote.quote_id)
        .await
        .expect("review");
    assert!(executor
        .approve_verified_with_fee_policy(
            &f.signer,
            &f.wallet,
            &f.quote.quote_id,
            &review.quote_event_id,
            &review.commitment,
            limits(),
            InvoiceNetwork::Regtest,
            RoutingFeePolicy::ReportedFees,
            false,
            &|| Ok(())
        )
        .await
        .is_err());
    executor
        .approve_verified_with_fee_policy(
            &f.signer,
            &f.wallet,
            &f.quote.quote_id,
            &review.quote_event_id,
            &review.commitment,
            limits(),
            InvoiceNetwork::Regtest,
            RoutingFeePolicy::ReportedFees,
            true,
            &|| Ok(()),
        )
        .await
        .expect("acknowledged weaker policy");
    executor
        .pay_leg(&f.signer, &f.wallet, &f.quote.quote_id, "attempt-0")
        .await
        .expect("standard supported pay operation");
    acknowledge(&f, "attempt-0").await;
    disclose(&f, 1).await;
    executor
        .pay_leg(&f.signer, &f.wallet, &f.quote.quote_id, "attempt-1")
        .await
        .expect("second original leg");
    assert_eq!(f.wallet.calls.load(Ordering::SeqCst), 2);
    assert!(arcadestr_core::purchase_contract::require_live_purchase_authorization().is_err());
}

#[tokio::test]
async fn reported_fee_overrun_halts_new_dispatch_but_preserves_paid_evidence() {
    use arcadestr_core::purchase_contract::{LegProgress, RoutingFeePolicy};
    let f = verified_fixture().await;
    f.wallet.known.store(false, Ordering::SeqCst);
    f.wallet.fees.store(60000, Ordering::SeqCst);
    let executor = PurchaseExecutor::new(f.db.pool().clone());
    let review = executor
        .review(&f.signer, Some(&f.wallet), &f.quote.quote_id)
        .await
        .expect("review");
    executor
        .approve_verified_with_fee_policy(
            &f.signer,
            &f.wallet,
            &f.quote.quote_id,
            &review.quote_event_id,
            &review.commitment,
            limits(),
            InvoiceNetwork::Regtest,
            RoutingFeePolicy::ReportedFees,
            true,
            &|| Ok(()),
        )
        .await
        .expect("risk acknowledged");
    assert!(executor
        .pay_leg(&f.signer, &f.wallet, &f.quote.quote_id, "attempt-0")
        .await
        .is_err());
    let status = executor
        .review(&f.signer, Some(&f.wallet), &f.quote.quote_id)
        .await
        .expect("overrun status");
    assert!(status.halted);
    assert!(!status.purchase_completed);
    assert_eq!(status.legs[0].progress, LegProgress::Paid);
    assert_eq!(status.legs[0].fees_paid_msat, Some(60000));
    f.wallet.visible.store(true, Ordering::SeqCst);
    executor
        .reconcile_leg(&f.signer, &f.wallet, &f.quote.quote_id, "attempt-0")
        .await
        .expect("paid proof remains recoverable");
    disclose(&f, 1).await;
    assert!(executor
        .pay_leg(&f.signer, &f.wallet, &f.quote.quote_id, "attempt-1")
        .await
        .is_err());
    assert_eq!(f.wallet.calls.load(Ordering::SeqCst), 1);
}
