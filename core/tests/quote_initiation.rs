#![cfg(feature = "native")]
use arcadestr_core::{settlement_repository::SettlementRepository, storage::Database};
use nostr::Keys;

#[tokio::test]
async fn initiation_is_durable_bound_and_account_and_processor_scoped() {
    let dir = tempfile::tempdir_in("/tmp/opencode").expect("directory");
    let path = dir.path().join("buyer.db");
    let db = Database::new(&path).await.expect("DB");
    let repo = SettlementRepository::new(db.pool().clone());
    let buyer = Keys::generate().public_key().to_hex();
    let other = Keys::generate().public_key().to_hex();
    let coordinate = format!("30402:{}:game", Keys::generate().public_key());
    let listing = "11".repeat(32);
    let commitment = "22".repeat(32);
    let first = repo
        .reserve_quote_initiation(
            &buyer,
            "https://processor.example/",
            &coordinate,
            &listing,
            &commitment,
            None,
        )
        .await
        .expect("first");
    let again = repo
        .reserve_quote_initiation(
            &buyer,
            "https://processor.example",
            &coordinate,
            &listing,
            &commitment,
            Some(&first),
        )
        .await
        .expect("exact retry");
    assert_eq!(first, again);
    for (revision, binding, id) in [
        (&listing, &commitment, Some("different-id")),
        (&"33".repeat(32), &commitment, None),
        (&listing, &"44".repeat(32), None),
    ] {
        assert!(
            repo.reserve_quote_initiation(
                &buyer,
                "https://processor.example",
                &coordinate,
                revision,
                binding,
                id
            )
            .await
            .is_err(),
            "unresolved request cannot be rebound"
        );
    }
    let different_account = repo
        .reserve_quote_initiation(
            &other,
            "https://processor.example",
            &coordinate,
            &listing,
            &commitment,
            None,
        )
        .await
        .expect("other account");
    assert_ne!(first, different_account);
    let different_processor = repo
        .reserve_quote_initiation(
            &buyer,
            "https://other.example",
            &coordinate,
            &listing,
            &commitment,
            None,
        )
        .await
        .expect("other processor");
    assert_ne!(first, different_processor);
    for processor in [
        "http://remote.example",
        "https://user:password@processor.example",
        "ftp://processor.example",
    ] {
        assert!(repo
            .reserve_quote_initiation(&buyer, processor, &coordinate, &listing, &commitment, None)
            .await
            .is_err());
    }
    db.close().await;
    let db = Database::new(&path).await.expect("restart");
    let repo = SettlementRepository::new(db.pool().clone());
    assert_eq!(
        repo.reserve_quote_initiation(
            &buyer,
            "https://processor.example",
            &coordinate,
            &listing,
            &commitment,
            None
        )
        .await
        .expect("durable retry"),
        first
    );
    let columns: Vec<String> =
        sqlx::query_scalar("SELECT name FROM pragma_table_info('quote_initiations')")
            .fetch_all(db.pool())
            .await
            .expect("columns");
    assert!(
        !columns
            .iter()
            .any(|c| c.contains("refund") || c.contains("preimage") || c.contains("invoice")),
        "checkpoint stores no private payment/refund payload"
    );
}

#[tokio::test]
async fn concurrent_reservations_never_allocate_two_remote_identities() {
    let dir = tempfile::tempdir_in("/tmp/opencode").expect("directory");
    let db = Database::new(&dir.path().join("db")).await.expect("DB");
    let repo = SettlementRepository::new(db.pool().clone());
    let buyer = Keys::generate().public_key().to_hex();
    let coordinate = format!("30402:{}:game", Keys::generate().public_key());
    let listing = "11".repeat(32);
    let commitment = "22".repeat(32);
    let (a, b) = tokio::join!(
        repo.reserve_quote_initiation(
            &buyer,
            "https://processor.example",
            &coordinate,
            &listing,
            &commitment,
            None
        ),
        repo.reserve_quote_initiation(
            &buyer,
            "https://processor.example",
            &coordinate,
            &listing,
            &commitment,
            None
        )
    );
    let allocated = a.ok().into_iter().chain(b.ok()).collect::<Vec<_>>();
    assert!(!allocated.is_empty());
    assert!(allocated.iter().all(|id| id == &allocated[0]));
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM quote_initiations")
        .fetch_one(db.pool())
        .await
        .expect("rows");
    assert_eq!(count, 1);
    assert_eq!(
        repo.reserve_quote_initiation(
            &buyer,
            "https://processor.example",
            &coordinate,
            &listing,
            &commitment,
            None
        )
        .await
        .expect("retry after contention"),
        allocated[0]
    );
}

#[tokio::test]
async fn only_a_completed_local_receipt_checkpoint_permits_a_new_initiation() {
    let dir = tempfile::tempdir_in("/tmp/opencode").expect("directory");
    let db = Database::new(&dir.path().join("db")).await.expect("DB");
    let repo = SettlementRepository::new(db.pool().clone());
    let buyer = Keys::generate().public_key().to_hex();
    let coordinate = format!("30402:{}:game", Keys::generate().public_key());
    let listing = "11".repeat(32);
    let commitment = "22".repeat(32);
    let original = repo
        .reserve_quote_initiation(
            &buyer,
            "https://processor.example",
            &coordinate,
            &listing,
            &commitment,
            None,
        )
        .await
        .expect("original identity");
    // Repository state sentinels only. Actual signature/proof verification and
    // receipt persistence are exercised by the cross-repository router test.
    sqlx::query("INSERT INTO settlement_quotes(account_pubkey,quote_id,processor_url,quote_event_json,listing_event_id,state,updated_at) VALUES(?,?,?,'original immutable quote',?,'active',1)")
        .bind(&buyer).bind(&original).bind("https://processor.example").bind(&listing).execute(db.pool()).await.expect("local quote");
    for state in ["active", "failed", "settled"] {
        sqlx::query("UPDATE settlement_quotes SET state=? WHERE quote_id=?")
            .bind(state)
            .bind(&original)
            .execute(db.pool())
            .await
            .expect("state fixture");
        assert_eq!(
            repo.reserve_quote_initiation(
                &buyer,
                "https://processor.example",
                &coordinate,
                &listing,
                &commitment,
                None
            )
            .await
            .expect("unresolved identity retained"),
            original,
            "{state} without receipt must not authorize a fresh obligation"
        );
    }
    sqlx::query("UPDATE settlement_quotes SET receipt_event_json='completed receipt checkpoint' WHERE quote_id=?").bind(&original).execute(db.pool()).await.expect("completion checkpoint");
    let next = repo
        .reserve_quote_initiation(
            &buyer,
            "https://processor.example",
            &coordinate,
            &listing,
            &commitment,
            None,
        )
        .await
        .expect("new explicit initiation after completion");
    assert_ne!(next, original);
    let retained: String =
        sqlx::query_scalar("SELECT quote_event_json FROM settlement_quotes WHERE quote_id=?")
            .bind(&original)
            .fetch_one(db.pool())
            .await
            .expect("original quote retained");
    assert_eq!(retained, "original immutable quote");
}
