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
