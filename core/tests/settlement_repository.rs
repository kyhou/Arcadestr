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
