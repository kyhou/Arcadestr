#![cfg(feature = "native")]

use arcadestr_core::adp_settlement::{
    PaymentLeg, PaymentRole, QuoteState, RefundDestination, SignedQuote,
};
use arcadestr_core::settlement_repository::{account_can_continue, SettlementRepository};
use arcadestr_core::storage::Database;

#[tokio::test]
async fn quote_state_is_account_scoped_and_resumable() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db = Database::new(&dir.path().join("db.sqlite"))
        .await
        .expect("db");
    let repository = SettlementRepository::new(db.pool().clone());
    let destination = RefundDestination {
        endpoint: "buyer@example.com".into(),
        min_sendable_msat: 1,
        max_sendable_msat: 2_000,
    };
    let quote = SignedQuote::new(
        "q".into(),
        "alice".into(),
        "listing".into(),
        "30402:dev:g".into(),
        "processor".into(),
        1_000,
        10,
        &destination,
        vec![PaymentLeg {
            leg_id: "developer".into(),
            role: PaymentRole::Developer,
            recipient_pubkey: "dev".into(),
            destination: "dev@example.com".into(),
            amount_msat: 1_000,
            accepted_terms_id: None,
        }],
    )
    .expect("quote");
    repository
        .save_quote("alice", "https://processor", "{}", &quote, 11)
        .await
        .expect("save");
    assert_eq!(
        repository
            .load("alice", "q")
            .await
            .expect("load")
            .expect("row")
            .state,
        "draft"
    );
    assert!(repository.load("bob", "q").await.expect("load").is_none());
    assert!(account_can_continue("alice", Some("alice")));
    assert!(!account_can_continue("alice", Some("bob")));
    assert!(!account_can_continue("alice", None));
    assert_eq!(quote.state, QuoteState::Draft);
}

#[tokio::test]
async fn recovery_history_is_scoped_to_account_and_quote() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db = Database::new(&dir.path().join("db.sqlite"))
        .await
        .expect("db");
    let repository = SettlementRepository::new(db.pool().clone());
    let destination = RefundDestination {
        endpoint: "buyer@example.com".into(),
        min_sendable_msat: 1,
        max_sendable_msat: 2_000,
    };
    let quote = SignedQuote::new(
        "recovery-q".into(),
        "alice".into(),
        "listing".into(),
        "30402:dev:g".into(),
        "processor".into(),
        1_000,
        10,
        &destination,
        vec![PaymentLeg {
            leg_id: "developer".into(),
            role: PaymentRole::Developer,
            recipient_pubkey: "dev".into(),
            destination: "dev@example.com".into(),
            amount_msat: 1_000,
            accepted_terms_id: None,
        }],
    )
    .expect("quote");
    repository
        .save_quote("alice", "https://processor", "{}", &quote, 11)
        .await
        .expect("save");
    repository
        .save_recovery_obligation(
            "alice",
            "recovery-q",
            "developer",
            1_000,
            700,
            None,
            "refund_owed",
        )
        .await
        .expect("save recovery");

    let alice = repository
        .list_recovery_for_quote("alice", "recovery-q")
        .await
        .expect("alice history");
    assert_eq!(alice.len(), 1);
    assert_eq!(alice[0].principal_msat, 1_000);
    assert_eq!(alice[0].due_at, 700);
    assert!(repository
        .list_recovery_for_quote("bob", "recovery-q")
        .await
        .expect("bob history")
        .is_empty());
    assert!(repository
        .list_recovery_for_quote("alice", "another-q")
        .await
        .expect("other quote history")
        .is_empty());
}

#[tokio::test]
async fn pending_payment_or_proof_checkpoint_blocks_replacement_across_restart() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("db.sqlite");
    let db = Database::new(&path).await.expect("db");
    let repository = SettlementRepository::new(db.pool().clone());
    let quote = SignedQuote::new(
        "durable-q".into(),
        "alice".into(),
        "listing".into(),
        "30402:dev:g".into(),
        "processor".into(),
        1000,
        10,
        &RefundDestination {
            endpoint: "buyer@example.com".into(),
            min_sendable_msat: 1,
            max_sendable_msat: 2000,
        },
        vec![PaymentLeg {
            leg_id: "developer".into(),
            role: PaymentRole::Developer,
            recipient_pubkey: "dev".into(),
            destination: "dev@example.com".into(),
            amount_msat: 1000,
            accepted_terms_id: None,
        }],
    )
    .expect("quote");
    repository
        .save_quote("alice", "https://processor", "{}", &quote, 11)
        .await
        .expect("quote");
    repository
        .record_invoice_attempt(
            "alice",
            "durable-q",
            "developer",
            "attempt",
            b"encrypted invoice",
            "hash",
            "disclosed",
            11,
        )
        .await
        .expect("invoice checkpoint");
    assert!(!repository
        .has_unreconciled_payment("alice", "durable-q")
        .await
        .expect("no payment yet"));
    sqlx::query("INSERT INTO nwc_payment_journal (account_pubkey,payment_hash,wallet_identity,invoice_commitment,amount_msat,encrypted_invoice,created_at,updated_at) VALUES ('alice','hash','wallet:app','commitment',1000,X'01',11,11)")
        .execute(db.pool()).await.expect("durable dispatch marker");
    assert!(repository
        .has_unreconciled_payment("alice", "durable-q")
        .await
        .expect("must not replace invoice"));
    assert!(!repository
        .has_unreconciled_payment("bob", "durable-q")
        .await
        .expect("account scope"));
    assert!(repository
        .checkpoint_proof("alice", "durable-q", "attempt", b"encrypted preimage")
        .await
        .expect("proof checkpoint"));
    assert!(!repository
        .checkpoint_proof("bob", "durable-q", "attempt", b"wrong")
        .await
        .expect("no cross-account write"));
    assert!(!repository
        .checkpoint_proof("alice", "wrong-quote", "attempt", b"wrong")
        .await
        .expect("quote binding"));
    drop(repository);
    db.pool().close().await;
    let db = Database::new(&path).await.expect("restart");
    let repository = SettlementRepository::new(db.pool().clone());
    let attempt = repository
        .invoice_attempt("alice", "durable-q", "attempt")
        .await
        .expect("load")
        .expect("attempt");
    assert_eq!(attempt.encrypted_attempt, b"encrypted invoice");
    assert_eq!(
        attempt.encrypted_proof.as_deref(),
        Some(b"encrypted preimage".as_slice())
    );
    assert!(repository
        .has_unreconciled_payment("alice", "durable-q")
        .await
        .expect("retained gate"));
    repository
        .update_invoice_state("alice", "attempt", "settled", 20)
        .await
        .expect("ADP acknowledged proof");
    assert!(!repository
        .has_unreconciled_payment("alice", "durable-q")
        .await
        .expect("next leg permitted"));
}

fn active_three_leg_quote() -> SignedQuote {
    let mut quote = SignedQuote::new(
        "three".into(),
        "alice".into(),
        "listing".into(),
        "30402:dev:g".into(),
        "processor".into(),
        1000,
        10,
        &RefundDestination {
            endpoint: "buyer@example.com".into(),
            min_sendable_msat: 1,
            max_sendable_msat: 2000,
        },
        vec![
            ("a", "operator-a", 100),
            ("b", "operator-b", 150),
            ("developer", "dev", 750),
        ]
        .into_iter()
        .map(|(leg_id, key, amount)| PaymentLeg {
            leg_id: leg_id.into(),
            role: if leg_id == "developer" {
                PaymentRole::Developer
            } else {
                PaymentRole::Operator
            },
            recipient_pubkey: key.into(),
            destination: "recipient@example.com".into(),
            amount_msat: amount,
            accepted_terms_id: if leg_id == "developer" {
                None
            } else {
                Some(format!("terms-{leg_id}"))
            },
        })
        .collect(),
    )
    .expect("quote");
    quote.state = QuoteState::Active;
    quote
}

#[tokio::test]
async fn acknowledged_attempt_and_receipt_atomically_settle_legs_without_reopening_purchase() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("client.db");
    let db = Database::new(&path).await.expect("db");
    let repo = SettlementRepository::new(db.pool().clone());
    let quote = active_three_leg_quote();
    repo.save_quote(
        "alice",
        "https://processor",
        "original-signed-quote",
        &quote,
        11,
    )
    .await
    .expect("save");
    for leg in &quote.legs {
        repo.record_invoice_attempt(
            "alice",
            "three",
            &leg.leg_id,
            &format!("attempt-{}", leg.leg_id),
            b"encrypted invoice",
            &format!("hash-{}", leg.leg_id),
            "disclosed",
            12,
        )
        .await
        .expect("attempt");
    }
    repo.update_invoice_state("alice", "attempt-a", "settled", 13)
        .await
        .expect("acknowledge");
    let states: Vec<(String, String)> = sqlx::query_as(
        "SELECT leg_id,state FROM settlement_legs WHERE account_pubkey='alice' ORDER BY leg_order",
    )
    .fetch_all(db.pool())
    .await
    .expect("states");
    assert_eq!(
        states,
        vec![
            ("a".into(), "settled".into()),
            ("b".into(), "pending".into()),
            ("developer".into(), "pending".into())
        ]
    );
    assert!(!repo
        .update_invoice_state("bob", "attempt-b", "settled", 13)
        .await
        .expect("scope"));
    assert!(repo
        .mark_receipt("alice", "three", "verified-paid-receipt", 14)
        .await
        .expect("receipt"));
    assert!(repo
        .mark_receipt("alice", "three", "verified-paid-receipt", 15)
        .await
        .expect("idempotent receipt"));
    repo.save_quote(
        "alice",
        "https://processor",
        "original-signed-quote",
        &quote,
        16,
    )
    .await
    .expect("old quote replay");
    assert_eq!(
        repo.load("alice", "three")
            .await
            .expect("load")
            .expect("row")
            .state,
        "settled"
    );
    let settled: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM settlement_legs WHERE state='settled'")
            .fetch_one(db.pool())
            .await
            .expect("legs");
    assert_eq!(settled, 3);
    repo.update_invoice_state("alice", "attempt-a", "disclosed", 17)
        .await
        .expect("stale attempt replay");
    let state: String = sqlx::query_scalar(
        "SELECT state FROM settlement_invoice_attempts WHERE attempt_id='attempt-a'",
    )
    .fetch_one(db.pool())
    .await
    .expect("state");
    assert_eq!(state, "settled");
    drop(repo);
    db.pool().close().await;
    let db = Database::new(&path).await.expect("restart");
    let repo = SettlementRepository::new(db.pool().clone());
    assert_eq!(
        repo.load("alice", "three")
            .await
            .expect("load")
            .expect("row")
            .state,
        "settled"
    );
}
