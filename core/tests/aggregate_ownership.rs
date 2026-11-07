#![cfg(feature = "native")]

use arcadestr_core::{
    adp_settlement::{aggregate_commitment, AggregateProofLeg, PaymentRole},
    purchases::{
        parse_and_validate_receipt_with_authorization, resolve_receipt_chain, PurchasesRepository,
        ReceiptEvidence,
    },
    storage::Database,
};
use nostr::{Event, EventBuilder, Keys, Kind, Tag, Timestamp};

fn receipt(developer: &Keys, buyer: &Keys, timestamp: u64, gross: u64) -> Event {
    let leg = AggregateProofLeg {
        leg_id: "developer".into(),
        role: PaymentRole::Developer,
        recipient_pubkey: developer.public_key().to_hex(),
        amount_msat: 1000,
        payment_hash: "11".repeat(32),
        proof_type: "bolt11-preimage".into(),
        proof_reference: "11".repeat(32),
        accepted_terms_id: None,
    };
    let commitment = aggregate_commitment(1000, &[leg]).expect("commitment");
    let coordinate = format!("30402:{}:regtest-fixture", developer.public_key());
    let tags = vec![
        vec!["order".into(), "aggregate-order".into()],
        vec!["p".into(), buyer.public_key().to_hex()],
        vec!["a".into(), coordinate],
        vec!["status".into(), "paid".into()],
        vec!["sale_listing".into(), "22".repeat(32)],
        vec!["sale_quote".into(), "33".repeat(32)],
        vec!["sale_processor".into(), developer.public_key().to_hex()],
        vec![
            "payment_set".into(),
            "1".into(),
            gross.to_string(),
            commitment,
        ],
        vec!["settled_at".into(), "100".into()],
        vec![
            "payment_leg".into(),
            "developer".into(),
            "developer".into(),
            developer.public_key().to_hex(),
            "1000".into(),
            "11".repeat(32),
            "bolt11-preimage".into(),
            "11".repeat(32),
            "".into(),
        ],
    ];
    EventBuilder::new(Kind::Custom(1020), "encrypted synthetic fixture")
        .tags(tags.into_iter().map(|v| Tag::parse(v).expect("tag")))
        .custom_created_at(Timestamp::from_secs(timestamp))
        .sign_with_keys(developer)
        .expect("sign")
}

#[tokio::test]
async fn aggregate_history_uses_the_same_receipt_validation_as_ownership() {
    use arcadestr_core::{
        entitlements_repository::EntitlementsRepository,
        ownership::{DurableCredentialStatus, OwnershipService},
    };
    let dir = tempfile::tempdir().expect("tempdir");
    let db = Database::new(&dir.path().join("history.db"))
        .await
        .expect("db");
    let developer = Keys::generate();
    let buyer = Keys::generate();
    let event = receipt(&developer, &buyer, 100, 1000);
    let repo = PurchasesRepository::new(db.pool().clone());
    let verified = parse_and_validate_receipt_with_authorization(
        &event,
        &buyer.public_key().to_hex(),
        None,
        ReceiptEvidence::default(),
    )
    .expect("aggregate");
    repo.upsert_receipt(&verified).await.expect("persist");
    let ownership = OwnershipService::new(repo, EntitlementsRepository::new(db.pool().clone()));
    let records = ownership
        .durable_records_for(&buyer.public_key().to_hex())
        .await
        .expect("history");
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].status, DurableCredentialStatus::Active);
    assert!(records[0].validation_error.is_none());
    assert_eq!(records[0].amount, Some(1));
    assert_eq!(records[0].currency.as_deref(), Some("SATS"));
    let malformed = receipt(&developer, &buyer, 101, 2000);
    let mut corrupted = verified;
    corrupted.event_id = malformed.id.to_hex();
    corrupted.raw_event = serde_json::to_string(&malformed).expect("JSON");
    corrupted.created_at = 101;
    PurchasesRepository::new(db.pool().clone())
        .upsert_receipt(&corrupted)
        .await
        .expect("corrupt history fixture");
    let records = ownership
        .durable_records_for(&buyer.public_key().to_hex())
        .await
        .expect("history");
    assert_eq!(records[0].status, DurableCredentialStatus::Unverified);
    assert!(records[0].validation_error.is_some());
}

#[tokio::test]
async fn verified_persisted_aggregate_grants_account_scoped_ownership_and_rejects_forks() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db = Database::new(&dir.path().join("app.db")).await.expect("db");
    let repo = PurchasesRepository::new(db.pool().clone());
    let developer = Keys::generate();
    let buyer = Keys::generate();
    let coordinate = format!("30402:{}:regtest-fixture", developer.public_key());
    let event = receipt(&developer, &buyer, 100, 1000);
    let verified = parse_and_validate_receipt_with_authorization(
        &event,
        &buyer.public_key().to_hex(),
        None,
        ReceiptEvidence::default(),
    )
    .expect("valid aggregate");
    repo.upsert_receipt(&verified).await.expect("persist");
    repo.upsert_receipt(&verified)
        .await
        .expect("idempotent persist");
    assert_eq!(
        resolve_receipt_chain(&[event.clone()], developer.public_key(), None)
            .expect("aggregate chain")
            .id,
        event.id
    );
    assert!(
        repo.is_owned(&buyer.public_key().to_hex(), &coordinate)
            .await
            .expect("ownership"),
        "a validated aggregate receipt must grant ownership after persistence"
    );
    assert!(!repo
        .is_owned(&developer.public_key().to_hex(), &coordinate)
        .await
        .expect("account scope"));
    assert!(
        parse_and_validate_receipt_with_authorization(
            &receipt(&developer, &buyer, 101, 2000),
            &buyer.public_key().to_hex(),
            None,
            ReceiptEvidence::default()
        )
        .is_err(),
        "wrong gross must not validate"
    );
    let fork = parse_and_validate_receipt_with_authorization(
        &receipt(&developer, &buyer, 101, 1000),
        &buyer.public_key().to_hex(),
        None,
        ReceiptEvidence::default(),
    )
    .expect("second root");
    repo.upsert_receipt(&fork)
        .await
        .expect("persist competing root");
    assert!(!repo
        .is_owned(&buyer.public_key().to_hex(), &coordinate)
        .await
        .expect("fail closed on fork"));
    let ownership = arcadestr_core::ownership::OwnershipService::new(
        repo,
        arcadestr_core::entitlements_repository::EntitlementsRepository::new(db.pool().clone()),
    );
    let records = ownership
        .durable_records_for(&buyer.public_key().to_hex())
        .await
        .expect("fork history");
    assert_eq!(
        records[0].status,
        arcadestr_core::ownership::DurableCredentialStatus::Unverified
    );
}
