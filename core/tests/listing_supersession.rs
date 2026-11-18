#![cfg(feature = "native")]
//! A paid listing revised after quote creation must never lead to payment of
//! the stale quote, and must not block a new purchase forever.
use arcadestr_core::{
    purchase_execution::{PurchaseExecutor, SupersedeOutcome},
    settlement_repository::SettlementRepository,
    storage::Database,
};
use nostr::Keys;
use sqlx::SqlitePool;

struct Fixture {
    _dir: tempfile::TempDir,
    db: Database,
    buyer: String,
    coordinate: String,
}

async fn fixture() -> Fixture {
    let dir = tempfile::tempdir_in("/tmp/opencode").expect("directory");
    let db = Database::new(&dir.path().join("buyer.db"))
        .await
        .expect("DB");
    Fixture {
        _dir: dir,
        db,
        buyer: Keys::generate().public_key().to_hex(),
        coordinate: format!("30402:{}:game", Keys::generate().public_key()),
    }
}

async fn store_quote(pool: &SqlitePool, buyer: &str, quote_id: &str, listing: &str, state: &str) {
    sqlx::query("INSERT INTO settlement_quotes (account_pubkey,quote_id,processor_url,quote_event_json,listing_event_id,state,updated_at) VALUES (?,?,'https://processor.example','{}',?,?,1)")
        .bind(buyer).bind(quote_id).bind(listing).bind(state).execute(pool).await.expect("quote");
    sqlx::query("INSERT INTO purchase_execution_approvals(account_pubkey,quote_id,snapshot) VALUES(?,?,'{}')")
        .bind(buyer).bind(quote_id).execute(pool).await.expect("approval");
}

async fn quote_state(pool: &SqlitePool, buyer: &str, quote_id: &str) -> (String, i64) {
    sqlx::query_as("SELECT q.state,a.halted FROM settlement_quotes q JOIN purchase_execution_approvals a USING(account_pubkey,quote_id) WHERE q.account_pubkey=? AND q.quote_id=?")
        .bind(buyer).bind(quote_id).fetch_one(pool).await.expect("state")
}

#[tokio::test]
async fn undispatched_quote_is_superseded_and_unblocks_a_new_purchase() {
    let f = fixture().await;
    let pool = f.db.pool().clone();
    let repo = SettlementRepository::new(pool.clone());
    let (old_listing, new_listing, commitment) =
        ("11".repeat(32), "33".repeat(32), "22".repeat(32));
    let id = repo
        .reserve_quote_initiation(
            &f.buyer,
            "https://processor.example",
            &f.coordinate,
            &old_listing,
            &commitment,
            None,
        )
        .await
        .expect("initiation");
    store_quote(&pool, &f.buyer, &id, &old_listing, "active").await;
    // A stored payable quote pins the old revision: no new purchase until resolved.
    assert!(repo
        .reserve_quote_initiation(
            &f.buyer,
            "https://processor.example",
            &f.coordinate,
            &new_listing,
            &commitment,
            None
        )
        .await
        .is_err());
    let executor = PurchaseExecutor::new(pool.clone());
    assert_eq!(
        executor
            .supersede_unpaid(&f.buyer, &id)
            .await
            .expect("supersede"),
        SupersedeOutcome::Superseded
    );
    assert_eq!(
        quote_state(&pool, &f.buyer, &id).await,
        ("superseded".into(), 1)
    );
    // Idempotent.
    assert_eq!(
        executor
            .supersede_unpaid(&f.buyer, &id)
            .await
            .expect("again"),
        SupersedeOutcome::Superseded
    );
    let fresh = repo
        .reserve_quote_initiation(
            &f.buyer,
            "https://processor.example",
            &f.coordinate,
            &new_listing,
            &commitment,
            None,
        )
        .await
        .expect("new purchase after supersession");
    assert_ne!(fresh, id);
    // The superseded identity is never reused for the new revision.
    assert!(repo
        .reserve_quote_initiation(
            &f.buyer,
            "https://processor.example",
            &f.coordinate,
            &new_listing,
            &commitment,
            Some(&id)
        )
        .await
        .is_err());
}

#[tokio::test]
async fn any_dispatch_evidence_halts_instead_of_superseding() {
    for evidence in ["admission", "journal", "proof", "settled"] {
        let f = fixture().await;
        let pool = f.db.pool().clone();
        let listing = "11".repeat(32);
        store_quote(&pool, &f.buyer, "q", &listing, "active").await;
        let hash = "aa".repeat(32);
        sqlx::query("INSERT INTO settlement_legs (account_pubkey,quote_id,leg_id,leg_order,recipient_pubkey,amount_msat,state) VALUES (?,'q','leg',0,'recipient',1000,'pending')")
            .bind(&f.buyer).execute(&pool).await.expect("leg");
        sqlx::query("INSERT INTO settlement_invoice_attempts (account_pubkey,quote_id,leg_id,attempt_id,payment_hash,encrypted_attempt,state,updated_at) VALUES (?,'q','leg','attempt',?,x'00',?,1)")
            .bind(&f.buyer).bind(&hash).bind(if evidence == "settled" { "settled" } else { "disclosed" })
            .execute(&pool).await.expect("attempt");
        match evidence {
            "admission" => {
                sqlx::query("INSERT INTO purchase_execution_admissions(account_pubkey,quote_id,leg_id,attempt_id,payment_hash,invoice_commitment,amount_msat,fee_bound_msat,state) VALUES(?,'q','leg','attempt',?,'c',1000,10,'uncertain')")
                    .bind(&f.buyer).bind(&hash).execute(&pool).await.expect("admission");
            }
            "journal" => {
                sqlx::query("INSERT INTO nwc_payment_journal (account_pubkey,payment_hash,wallet_identity,invoice_commitment,amount_msat,encrypted_invoice,created_at,updated_at) VALUES (?,?,'wallet','c',1000,x'00',1,1)")
                    .bind(&f.buyer).bind(&hash).execute(&pool).await.expect("journal");
            }
            "proof" => {
                sqlx::query("UPDATE settlement_invoice_attempts SET encrypted_proof=x'01' WHERE account_pubkey=?")
                    .bind(&f.buyer).execute(&pool).await.expect("proof");
            }
            _ => {}
        }
        assert_eq!(
            PurchaseExecutor::new(pool.clone())
                .supersede_unpaid(&f.buyer, "q")
                .await
                .expect("halt"),
            SupersedeOutcome::HaltedAfterDispatch,
            "{evidence}"
        );
        assert_eq!(
            quote_state(&pool, &f.buyer, "q").await,
            ("active".into(), 1),
            "{evidence}"
        );
    }
}

#[tokio::test]
async fn settled_and_unknown_quotes_are_not_changed() {
    let f = fixture().await;
    let pool = f.db.pool().clone();
    store_quote(&pool, &f.buyer, "paid", &"11".repeat(32), "settled").await;
    let executor = PurchaseExecutor::new(pool.clone());
    assert_eq!(
        executor
            .supersede_unpaid(&f.buyer, "paid")
            .await
            .expect("settled"),
        SupersedeOutcome::AlreadySettled
    );
    assert_eq!(
        quote_state(&pool, &f.buyer, "paid").await,
        ("settled".into(), 0)
    );
    assert!(executor
        .supersede_unpaid(&f.buyer, "missing")
        .await
        .is_err());
    assert!(executor
        .supersede_unpaid(&Keys::generate().public_key().to_hex(), "paid")
        .await
        .is_err());
}

#[tokio::test]
async fn lost_quote_response_with_a_revised_listing_gets_a_new_identity() {
    let f = fixture().await;
    let repo = SettlementRepository::new(f.db.pool().clone());
    let commitment = "22".repeat(32);
    let id = repo
        .reserve_quote_initiation(
            &f.buyer,
            "https://processor.example",
            &f.coordinate,
            &"11".repeat(32),
            &commitment,
            None,
        )
        .await
        .expect("initiation");
    // Same revision: same durable identity (lost-response idempotency).
    assert_eq!(
        repo.reserve_quote_initiation(
            &f.buyer,
            "https://processor.example",
            &f.coordinate,
            &"11".repeat(32),
            &commitment,
            None
        )
        .await
        .expect("retry"),
        id
    );
    // Revised listing with no stored local quote: a new identity, never a rebinding.
    let fresh = repo
        .reserve_quote_initiation(
            &f.buyer,
            "https://processor.example",
            &f.coordinate,
            &"33".repeat(32),
            &commitment,
            None,
        )
        .await
        .expect("revised");
    assert_ne!(fresh, id);
}

/// Phase 4C F-1: a processor-confirmed failed quote whose every dispatched leg
/// is resolved (verified admission, settled attempt) no longer blocks a new
/// purchase identity. Any unresolved evidence keeps it blocking, and the old
/// quote, its legs and refund obligations are never changed.
#[tokio::test]
async fn failed_quote_unblocks_a_new_purchase_only_when_every_payment_is_resolved() {
    for case in [
        "resolved",
        "active",
        "missing_failure_time",
        "uncertain_admission",
        "unsubmitted_proof",
        "journal_without_settlement",
    ] {
        let f = fixture().await;
        let pool = f.db.pool().clone();
        let repo = SettlementRepository::new(pool.clone());
        let (listing, commitment) = ("11".repeat(32), "22".repeat(32));
        let id = repo
            .reserve_quote_initiation(
                &f.buyer,
                "https://processor.example",
                &f.coordinate,
                &listing,
                &commitment,
                None,
            )
            .await
            .expect("initiation");
        let state = if case == "active" { "active" } else { "failed" };
        store_quote(&pool, &f.buyer, &id, &listing, state).await;
        if state == "failed" && case != "missing_failure_time" {
            repo.update_status(
                &f.buyer,
                &id,
                arcadestr_core::adp_settlement::QuoteState::Failed,
                Some(100),
                100,
            )
            .await
            .expect("processor failure checkpoint");
        }
        let (paid, unpaid) = ("aa".repeat(32), "bb".repeat(32));
        for (order, leg, hash, attempt_state) in [
            (0, "carol", &paid, "settled"),
            (1, "dave", &unpaid, "disclosed"),
        ] {
            sqlx::query("INSERT INTO settlement_legs (account_pubkey,quote_id,leg_id,leg_order,recipient_pubkey,amount_msat,state) VALUES (?,?,?,?,'recipient',1000,'pending')")
                .bind(&f.buyer).bind(&id).bind(leg).bind(order).execute(&pool).await.expect("leg");
            sqlx::query("INSERT INTO settlement_invoice_attempts (account_pubkey,quote_id,leg_id,attempt_id,payment_hash,encrypted_attempt,state,updated_at) VALUES (?,?,?,?,?,x'00',?,1)")
                .bind(&f.buyer).bind(&id).bind(leg).bind(format!("{leg}-attempt")).bind(hash).bind(attempt_state)
                .execute(&pool).await.expect("attempt");
        }
        // Carol's leg: one verified admission, journaled dispatch and accepted proof.
        let carol_admission = if case == "uncertain_admission" {
            "uncertain"
        } else {
            "verified"
        };
        sqlx::query("INSERT INTO purchase_execution_admissions(account_pubkey,quote_id,leg_id,attempt_id,payment_hash,invoice_commitment,amount_msat,fee_bound_msat,state) VALUES(?,?,'carol','carol-attempt',?,'c',1000,10,?)")
            .bind(&f.buyer).bind(&id).bind(&paid).bind(carol_admission).execute(&pool).await.expect("admission");
        sqlx::query("INSERT INTO nwc_payment_journal (account_pubkey,payment_hash,wallet_identity,invoice_commitment,amount_msat,encrypted_invoice,created_at,updated_at) VALUES (?,?,'wallet','c',1000,x'00',1,1)")
            .bind(&f.buyer).bind(&paid).execute(&pool).await.expect("journal");
        match case {
            "unsubmitted_proof" => {
                sqlx::query("UPDATE settlement_invoice_attempts SET encrypted_proof=x'01' WHERE leg_id='dave'")
                    .execute(&pool).await.expect("proof");
            }
            "journal_without_settlement" => {
                sqlx::query("INSERT INTO nwc_payment_journal (account_pubkey,payment_hash,wallet_identity,invoice_commitment,amount_msat,encrypted_invoice,created_at,updated_at) VALUES (?,?,'wallet','c',1000,x'00',1,1)")
                    .bind(&f.buyer).bind(&unpaid).execute(&pool).await.expect("journal");
            }
            _ => {}
        }
        let resolved = case == "resolved";
        assert_eq!(
            repo.failed_quote_resolved(&f.buyer, &id)
                .await
                .expect("check"),
            resolved,
            "{case}"
        );
        let next = repo
            .reserve_quote_initiation(
                &f.buyer,
                "https://processor.example",
                &f.coordinate,
                &listing,
                &commitment,
                None,
            )
            .await
            .expect("reservation");
        assert_eq!(next != id, resolved, "{case}");
        // Old quote evidence is untouched either way.
        assert_eq!(quote_state(&pool, &f.buyer, &id).await.0, state, "{case}");
        let admissions: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM purchase_execution_admissions WHERE account_pubkey=? AND quote_id=?",
        )
        .bind(&f.buyer)
        .bind(&id)
        .fetch_one(&pool)
        .await
        .expect("count");
        assert_eq!(admissions, 1, "{case}");
        // Another account's identical quote id is never consulted.
        assert!(!repo
            .failed_quote_resolved(&Keys::generate().public_key().to_hex(), &id)
            .await
            .expect("foreign"));
    }
}

/// An expired quote that was never approved or paid is closed the same way,
/// so the processor's stale quote cannot block a new purchase forever.
#[tokio::test]
async fn expired_unapproved_quote_is_superseded_and_unblocks_a_new_purchase() {
    let f = fixture().await;
    let pool = f.db.pool().clone();
    let repo = SettlementRepository::new(pool.clone());
    let (listing, commitment) = ("11".repeat(32), "22".repeat(32));
    let id = repo
        .reserve_quote_initiation(
            &f.buyer,
            "https://processor.example",
            &f.coordinate,
            &listing,
            &commitment,
            None,
        )
        .await
        .expect("initiation");
    sqlx::query("INSERT INTO settlement_quotes (account_pubkey,quote_id,processor_url,quote_event_json,listing_event_id,state,updated_at) VALUES (?,?,'https://processor.example','{}',?,'active',1)")
        .bind(&f.buyer).bind(&id).bind(&listing).execute(&pool).await.expect("quote");
    assert_eq!(
        PurchaseExecutor::new(pool.clone())
            .supersede_unpaid(&f.buyer, &id)
            .await
            .expect("supersede"),
        SupersedeOutcome::Superseded
    );
    let fresh = repo
        .reserve_quote_initiation(
            &f.buyer,
            "https://processor.example",
            &f.coordinate,
            &listing,
            &commitment,
            None,
        )
        .await
        .expect("same listing, new purchase identity");
    assert_ne!(fresh, id);
}
