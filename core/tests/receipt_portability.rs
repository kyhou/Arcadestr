#![cfg(feature = "native")]
use arcadestr_core::{
    entitlements_repository::EntitlementsRepository,
    ownership::OwnershipService,
    purchases::PurchasesRepository,
    receipt_portability::{ReceiptPortability, ReceiptSource, PAGE_SIZE},
    receipt_sync_contract::ReceiptVerification as V,
    storage::Database,
};
use async_trait::async_trait;
use nostr::{Event, EventBuilder, JsonUtil, Keys, Kind, PublicKey, Tag, Timestamp};
use std::{
    collections::BTreeMap,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Mutex,
    },
};

struct Source {
    relays: BTreeMap<String, Vec<Event>>,
    fail: AtomicBool,
    calls: AtomicUsize,
    orders: AtomicUsize,
    indexed: bool,
    fail_relay: Option<String>,
    lsp: Option<PublicKey>,
    filters: Mutex<Vec<serde_json::Value>>,
}
#[async_trait]
impl ReceiptSource for Source {
    fn relay_urls(&self) -> Vec<String> {
        self.relays.keys().cloned().collect()
    }
    async fn query(&self, relay: &str, filter: nostr::Filter) -> Result<Vec<Event>, String> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.filters
            .lock()
            .expect("queries")
            .push(serde_json::to_value(&filter).expect("filter"));
        if self.fail.load(Ordering::SeqCst) || self.fail_relay.as_deref() == Some(relay) {
            return Err("mock outage".into());
        }
        let mut events = self
            .relays
            .get(relay)
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .filter(|e| filter.match_event(e, Default::default()))
            .collect::<Vec<_>>();
        events.sort_by(|a, b| b.created_at.cmp(&a.created_at).then(a.id.cmp(&b.id)));
        events.truncate(filter.limit.unwrap_or(usize::MAX));
        Ok(events)
    }
    fn supports_order_filter(&self, _: &str) -> bool {
        self.indexed
    }
    async fn query_order(&self, _: &str, _: PublicKey, _: &str) -> Result<Vec<Event>, String> {
        self.orders.fetch_add(1, Ordering::SeqCst);
        Ok(Vec::new())
    } // silently ignored index: fallback is required
    async fn lsp_pubkey(&self, _: &str) -> Result<Option<PublicKey>, String> {
        Ok(self.lsp)
    }
    async fn aggregate_lsp(
        &self,
        _: &arcadestr_core::purchases::ParsedReceipt,
        _: &arcadestr_core::adp_settlement::AggregateProofLeg,
    ) -> Result<Option<arcadestr_core::receipt_portability::AggregateLspEvidence>, String> {
        Ok(self.lsp.map(
            |key| arcadestr_core::receipt_portability::AggregateLspEvidence {
                key,
                signed_events: Vec::new(),
            },
        ))
    }
}
fn source(events: Vec<Event>) -> Source {
    Source {
        relays: BTreeMap::from([("mock-a".into(), events.clone()), ("mock-b".into(), events)]),
        fail: AtomicBool::new(false),
        calls: AtomicUsize::new(0),
        orders: AtomicUsize::new(0),
        indexed: true,
        fail_relay: None,
        lsp: None,
        filters: Mutex::new(Vec::new()),
    }
}
struct Fixture {
    _temp: tempfile::TempDir,
    db: Database,
    buyer: Keys,
    developer: Keys,
}
impl Fixture {
    async fn new() -> Self {
        let temp = tempfile::tempdir().expect("temp");
        let db = Database::new(&temp.path().join("app.db"))
            .await
            .expect("db");
        Self {
            _temp: temp,
            db,
            buyer: Keys::generate(),
            developer: Keys::generate(),
        }
    }
    fn coordinate(&self) -> String {
        format!("30402:{}:game", self.developer.public_key().to_hex())
    }
    fn event(
        &self,
        signer: &Keys,
        status: &str,
        previous: Option<&Event>,
        order: &str,
        at: u64,
        auth: Option<&Event>,
    ) -> Event {
        let mut tags = vec![
            vec!["p".into(), self.buyer.public_key().to_hex()],
            vec!["a".into(), self.coordinate()],
            vec!["order".into(), order.into()],
            vec!["status".into(), status.into()],
        ];
        if let Some(prev) = previous {
            tags.push(vec!["e".into(), prev.id.to_hex()]);
        } else {
            tags.extend([
                vec!["payment_hash".into(), "11".repeat(32)],
                vec!["amount_msat".into(), "1000".into()],
                vec!["proof".into(), "bolt11-preimage".into()],
                vec!["settled_at".into(), at.to_string()],
            ]);
        }
        if let Some(auth) = auth {
            tags.push(vec!["authorization".into(), auth.id.to_hex()]);
        }
        let content = nostr::nips::nip44::encrypt(
            signer.secret_key(),
            &self.buyer.public_key(),
            "PRIVATE_INVOICE_PREIMAGE_SENTINEL",
            nostr::nips::nip44::Version::V2,
        )
        .expect("encrypt");
        EventBuilder::new(Kind::Custom(1020), content)
            .tags(tags.into_iter().map(|t| Tag::parse(t).expect("tag")))
            .custom_created_at(Timestamp::from_secs(at))
            .sign_with_keys(signer)
            .expect("sign")
    }
    fn root(&self) -> Event {
        self.event(&self.developer, "paid", None, "order", 100, None)
    }
    async fn owned(&self) -> bool {
        PurchasesRepository::new(self.db.pool().clone())
            .is_owned(&self.buyer.public_key().to_hex(), &self.coordinate())
            .await
            .expect("owned")
    }
    fn service(&self) -> ReceiptPortability {
        ReceiptPortability::new(self.db.pool().clone())
    }
    fn auth(&self, key: &Keys, previous: Option<&Event>, at: u64) -> Event {
        let mut tags = vec![
            vec!["d".into(), "authorization".into()],
            vec!["a".into(), self.coordinate()],
            vec!["p".into(), key.public_key().to_hex()],
            vec!["fulfillment_pubkey".into(), key.public_key().to_hex()],
            vec!["valid_from".into(), "1".into()],
            vec!["capability".into(), "issue_receipt".into()],
            vec![
                "status".into(),
                if previous.is_some() {
                    "cancelled"
                } else {
                    "active"
                }
                .into(),
            ],
        ];
        if let Some(prev) = previous {
            tags.push(vec!["e".into(), prev.id.to_hex()]);
        }
        let event = EventBuilder::new(Kind::Custom(30406), "")
            .tags(tags.into_iter().map(|t| Tag::parse(t).expect("tag")))
            .custom_created_at(Timestamp::from_secs(at))
            .sign_with_keys(&self.developer)
            .expect("auth");
        arcadestr_core::authorization::parse_authorization_event(&event)
            .expect("valid authorization fixture");
        event
    }
}

#[tokio::test]
async fn independent_device_discovery_deduplicates_relays_and_preserves_encrypted_evidence() {
    let f = Fixture::new().await;
    let root = f.root();
    let relays = source(vec![root.clone()]);
    let result = f
        .service()
        .synchronize(f.buyer.public_key(), &relays, false, 1000, &|| Ok(()))
        .await
        .expect("sync");
    assert_eq!(result.chains.len(), 1);
    assert_eq!(result.chains[0].verification, V::Verified);
    assert!(f.owned().await);
    assert!(relays.orders.load(Ordering::SeqCst) > 0);
    f.service()
        .synchronize(f.buyer.public_key(), &relays, false, 1001, &|| Ok(()))
        .await
        .expect("repeat");
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM purchases")
        .fetch_one(f.db.pool())
        .await
        .expect("count");
    assert_eq!(count, 1);
    let raw: String = sqlx::query_scalar("SELECT raw_event FROM purchases")
        .fetch_one(f.db.pool())
        .await
        .expect("raw");
    assert!(!raw.contains("PRIVATE_INVOICE_PREIMAGE_SENTINEL"));
    assert_eq!(
        serde_json::from_str::<Event>(&raw).expect("event").content,
        root.content
    );
    let owned = OwnershipService::new(
        PurchasesRepository::new(f.db.pool().clone()),
        EntitlementsRepository::new(f.db.pool().clone()),
    );
    assert_eq!(
        owned
            .durable_records_for(&f.buyer.public_key().to_hex())
            .await
            .expect("history")
            .len(),
        1
    );
    assert_eq!(
        owned
            .library_entries_for(&f.buyer.public_key().to_hex(), Vec::new())
            .await
            .expect("library")
            .len(),
        1
    );
    let other = Keys::generate();
    assert!(owned
        .library_entries_for(&other.public_key().to_hex(), Vec::new())
        .await
        .expect("isolation")
        .is_empty());
}

#[tokio::test]
async fn import_full_chain_is_idempotent_and_status_is_not_selected_by_timestamp() {
    let f = Fixture::new().await;
    let root = f.root();
    let fulfilled = f.event(&f.developer, "fulfilled", Some(&root), "order", 101, None);
    let refunded = f.event(
        &f.developer,
        "refunded",
        Some(&fulfilled),
        "order",
        102,
        None,
    );
    let json = serde_json::to_string(&vec![
        refunded.clone(),
        root.clone(),
        fulfilled.clone(),
        root,
    ])
    .expect("json");
    for _ in 0..2 {
        let result = f
            .service()
            .import_json(f.buyer.public_key(), &json, None, &|| Ok(()))
            .await
            .expect("import");
        assert_eq!(result.chains[0].verification, V::Verified);
        assert_eq!(result.chains[0].tip_event_id, Some(refunded.id.to_hex()));
        assert!(!f.owned().await);
    }
    let reopened = Database::new(&f._temp.path().join("app.db"))
        .await
        .expect("restart");
    assert!(!PurchasesRepository::new(reopened.pool().clone())
        .is_owned(&f.buyer.public_key().to_hex(), &f.coordinate())
        .await
        .expect("restored status"));
}

#[tokio::test]
async fn missing_predecessor_never_grants_and_recovers_when_evidence_arrives() {
    let f = Fixture::new().await;
    let root = f.root();
    let update = f.event(&f.developer, "fulfilled", Some(&root), "order", 101, None);
    let result = f
        .service()
        .ingest(f.buyer.public_key(), vec![update], None, &|| Ok(()))
        .await
        .expect("stage");
    assert_eq!(result.chains[0].verification, V::Indeterminate);
    assert!(!f.owned().await);
    let result = f
        .service()
        .ingest(f.buyer.public_key(), vec![root], None, &|| Ok(()))
        .await
        .expect("resolve");
    assert_eq!(result.chains[0].verification, V::Verified);
    assert!(f.owned().await);
}

#[tokio::test]
async fn delegated_roots_require_authorization_and_historical_validity() {
    for cancel_at in [None, Some(50), Some(200)] {
        let f = Fixture::new().await;
        let key = Keys::generate();
        let auth = f.auth(&key, None, 1);
        let root = f.event(&key, "paid", None, "order", 100, Some(&auth));
        let result = f
            .service()
            .ingest(f.buyer.public_key(), vec![root], None, &|| Ok(()))
            .await
            .expect("missing auth");
        assert_eq!(result.chains[0].verification, V::Indeterminate);
        assert!(!f.owned().await);
        let mut evidence = vec![auth.clone()];
        if let Some(at) = cancel_at {
            evidence.push(f.auth(&key, Some(&auth), at));
        }
        let result = f
            .service()
            .ingest(f.buyer.public_key(), evidence, None, &|| Ok(()))
            .await
            .expect("authority");
        assert_eq!(
            result.chains[0].verification,
            if cancel_at == Some(50) {
                V::Invalid
            } else {
                V::Verified
            }
        );
        assert_eq!(f.owned().await, cancel_at != Some(50));
    }
}

#[tokio::test]
async fn forks_unreachable_updates_and_untrusted_roots_fail_closed() {
    for case in 0..3 {
        let f = Fixture::new().await;
        let root = f.root();
        let a = f.event(&f.developer, "fulfilled", Some(&root), "order", 101, None);
        let b = f.event(&f.developer, "disputed", Some(&root), "order", 102, None);
        let events = match case {
            0 => vec![root, a, b],
            1 => vec![
                root,
                f.event(&f.developer, "refunded", Some(&a), "order", 103, None),
            ],
            _ => vec![f.event(&Keys::generate(), "paid", None, "order", 100, None)],
        };
        let result = f
            .service()
            .ingest(f.buyer.public_key(), events, None, &|| Ok(()))
            .await
            .expect("invalid chain");
        assert_ne!(result.chains[0].verification, V::Verified);
        assert!(!f.owned().await);
    }
}

#[tokio::test]
async fn previously_owned_chain_is_blocked_by_new_unresolved_extension_not_erased() {
    let f = Fixture::new().await;
    let root = f.root();
    f.service()
        .ingest(f.buyer.public_key(), vec![root.clone()], None, &|| Ok(()))
        .await
        .expect("root");
    assert!(f.owned().await);
    let missing = f.event(&f.developer, "fulfilled", Some(&root), "order", 101, None);
    let refund = f.event(&f.developer, "refunded", Some(&missing), "order", 102, None);
    f.service()
        .ingest(f.buyer.public_key(), vec![refund], None, &|| Ok(()))
        .await
        .expect("partial chain");
    assert!(!f.owned().await);
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM purchases")
        .fetch_one(f.db.pool())
        .await
        .expect("preserved");
    assert_eq!(count, 1);
    let history = OwnershipService::new(
        PurchasesRepository::new(f.db.pool().clone()),
        EntitlementsRepository::new(f.db.pool().clone()),
    )
    .durable_records_for(&f.buyer.public_key().to_hex())
    .await
    .expect("history");
    assert_eq!(history[0].verification, Some(V::Indeterminate));
}

#[tokio::test]
async fn outages_do_not_destroy_verified_local_ownership_and_guards_abort_commits() {
    let f = Fixture::new().await;
    let relays = source(vec![f.root()]);
    f.service()
        .synchronize(f.buyer.public_key(), &relays, false, 1000, &|| Ok(()))
        .await
        .expect("first");
    relays.fail.store(true, Ordering::SeqCst);
    let result = f
        .service()
        .synchronize(f.buyer.public_key(), &relays, false, 1001, &|| Ok(()))
        .await
        .expect("offline");
    assert!(result.incomplete);
    assert!(f.owned().await);
    let other = Fixture::new().await;
    let guard_calls = AtomicUsize::new(0);
    let result = other
        .service()
        .ingest(other.buyer.public_key(), vec![other.root()], None, &|| {
            if guard_calls.fetch_add(1, Ordering::SeqCst) > 0 {
                Err("account changed".into())
            } else {
                Ok(())
            }
        })
        .await;
    assert!(result.is_err());
    assert!(!other.owned().await);
}

#[tokio::test]
async fn malformed_foreign_buyer_events_and_forged_signature_cannot_be_imported() {
    let f = Fixture::new().await;
    let mut forged = f.root();
    forged.content.push_str("tampered");
    assert_eq!(
        f.service()
            .ingest(f.buyer.public_key(), vec![forged], None, &|| Ok(()))
            .await
            .expect("reject")
            .rejected_events,
        1
    );
    let other = Keys::generate();
    assert_eq!(
        f.service()
            .ingest(other.public_key(), vec![f.root()], None, &|| Ok(()))
            .await
            .expect("wrong buyer")
            .rejected_events,
        1
    );
    assert!(f
        .service()
        .import_json(f.buyer.public_key(), "not JSON", None, &|| Ok(()))
        .await
        .is_err());
    assert!(!f.owned().await);
}

#[tokio::test]
async fn saturated_timestamp_page_reports_incomplete_without_skipping_the_boundary() {
    let f = Fixture::new().await;
    let events = (0..PAGE_SIZE)
        .map(|i| f.event(&f.developer, "paid", None, &format!("order-{i}"), 100, None))
        .collect();
    let relays = source(events);
    let result = f
        .service()
        .synchronize(f.buyer.public_key(), &relays, false, 1000, &|| Ok(()))
        .await
        .expect("bounded sync");
    assert!(result.incomplete);
    let cursor: Option<i64> =
        sqlx::query_scalar("SELECT backfill_until FROM receipt_sync_cursors LIMIT 1")
            .fetch_one(f.db.pool())
            .await
            .expect("cursor");
    assert_eq!(cursor, Some(100));
}

#[tokio::test]
async fn aggregate_receipt_import_requires_no_private_quote_wallet_or_local_payment() {
    use arcadestr_core::adp_settlement::{aggregate_commitment, AggregateProofLeg, PaymentRole};
    let f = Fixture::new().await;
    let root = f.root();
    let leg = AggregateProofLeg {
        leg_id: "developer".into(),
        role: PaymentRole::Developer,
        recipient_pubkey: f.developer.public_key().to_hex(),
        amount_msat: 1000,
        payment_hash: "11".repeat(32),
        proof_type: "bolt11-preimage".into(),
        proof_reference: "11".repeat(32),
        accepted_terms_id: None,
    };
    let commitment = aggregate_commitment(1000, &[leg.clone()]).expect("commitment");
    let mut tags = root
        .tags
        .iter()
        .filter(|t| {
            !matches!(
                t.as_slice().first().map(String::as_str),
                Some("payment_hash" | "amount_msat" | "proof")
            )
        })
        .cloned()
        .collect::<Vec<_>>();
    for t in [
        vec!["payment_set".into(), "1".into(), "1000".into(), commitment],
        vec![
            "payment_leg".into(),
            "developer".into(),
            "developer".into(),
            f.developer.public_key().to_hex(),
            "1000".into(),
            "11".repeat(32),
            "bolt11-preimage".into(),
            "11".repeat(32),
            String::new(),
        ],
        vec!["sale_listing".into(), "22".repeat(32)],
        vec!["sale_quote".into(), "33".repeat(32)],
        vec!["sale_processor".into(), f.developer.public_key().to_hex()],
    ] {
        tags.push(Tag::parse(t).expect("aggregate tag"));
    }
    let event = EventBuilder::new(Kind::Custom(1020), root.content)
        .tags(tags)
        .custom_created_at(Timestamp::from_secs(100))
        .sign_with_keys(&f.developer)
        .expect("aggregate");
    let report = f
        .service()
        .import_json(f.buyer.public_key(), &event.as_json(), None, &|| Ok(()))
        .await
        .expect("portable aggregate");
    assert_eq!(report.chains[0].verification, V::Verified);
    assert!(f.owned().await);
    for table in [
        "nwc_payment_journal",
        "purchase_execution_admissions",
        "settlement_quotes",
    ] {
        let count: i64 = sqlx::query_scalar(&format!("SELECT COUNT(*) FROM {table}"))
            .fetch_one(f.db.pool())
            .await
            .expect("no payment side effect");
        assert_eq!(count, 0);
    }
    let mut unknown = leg;
    unknown.proof_type = "invented-proof".into();
    let commitment = aggregate_commitment(1000, &[unknown])
        .expect("hash alone does not validate proof declarations");
    let tags = event
        .tags
        .iter()
        .map(|t| {
            let mut values = t.as_slice().to_vec();
            if values[0] == "payment_leg" {
                values[6] = "invented-proof".into();
            }
            if values[0] == "payment_set" {
                values[3] = commitment.clone();
            }
            Tag::parse(values).expect("tag")
        })
        .collect::<Vec<_>>();
    let signed_invalid = EventBuilder::new(Kind::Custom(1020), event.content)
        .tags(tags)
        .custom_created_at(Timestamp::from_secs(100))
        .sign_with_keys(&f.developer)
        .expect("signed malformed profile");
    assert!(signed_invalid.verify().is_ok());
    assert!(arcadestr_core::purchases::parse_receipt_event(&signed_invalid).is_err());
}

#[tokio::test]
async fn later_authorization_evidence_reevaluates_ownership_and_history() {
    let f = Fixture::new().await;
    let key = Keys::generate();
    let auth = f.auth(&key, None, 1);
    let root = f.event(&key, "paid", None, "order", 100, Some(&auth));
    f.service()
        .ingest(
            f.buyer.public_key(),
            vec![root, auth.clone()],
            None,
            &|| Ok(()),
        )
        .await
        .expect("initial evidence");
    assert!(f.owned().await);
    let result = f
        .service()
        .ingest(
            f.buyer.public_key(),
            vec![f.auth(&key, Some(&auth), 50)],
            None,
            &|| Ok(()),
        )
        .await
        .expect("cancellation discovered");
    assert_eq!(result.chains[0].verification, V::Invalid);
    assert!(!f.owned().await);
    let service = OwnershipService::new(
        PurchasesRepository::new(f.db.pool().clone()),
        EntitlementsRepository::new(f.db.pool().clone()),
    );
    assert_eq!(
        service
            .durable_records_for(&f.buyer.public_key().to_hex())
            .await
            .expect("history")[0]
            .verification,
        Some(V::Invalid)
    );
    assert!(service
        .library_entries_for(&f.buyer.public_key().to_hex(), Vec::new())
        .await
        .expect("library")
        .is_empty());
}

#[tokio::test]
async fn one_unavailable_relay_and_unsupported_order_index_do_not_block_valid_evidence() {
    let f = Fixture::new().await;
    let mut relays = source(vec![f.root()]);
    relays.fail_relay = Some("mock-b".into());
    relays.indexed = false;
    let result = f
        .service()
        .synchronize(f.buyer.public_key(), &relays, false, 1000, &|| Ok(()))
        .await
        .expect("best effort");
    assert!(result.incomplete);
    assert!(result.relay_failures > 0);
    assert_eq!(result.chains[0].verification, V::Verified);
    assert!(f.owned().await);
    assert_eq!(relays.orders.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn changed_order_successor_cannot_leave_the_old_root_owned() {
    let f = Fixture::new().await;
    let root = f.root();
    let changed = f.event(
        &f.developer,
        "refunded",
        Some(&root),
        "other-order",
        101,
        None,
    );
    let report = f
        .service()
        .ingest(f.buyer.public_key(), vec![root, changed], None, &|| Ok(()))
        .await
        .expect("contradiction");
    assert!(report.chains.iter().all(|c| c.verification == V::Invalid));
    assert!(!f.owned().await);
}

#[tokio::test]
async fn public_zap_evidence_needs_the_listing_lsp_and_is_recoverable_after_import() {
    use bitcoin::{
        hashes::{sha256, Hash},
        secp256k1::{Secp256k1, SecretKey},
    };
    use lightning_invoice::{Currency, InvoiceBuilder, PaymentSecret};
    use std::time::Duration;
    let f = Fixture::new().await;
    let lsp = Keys::generate();
    let hash = sha256::Hash::hash(&[7; 32]);
    let key = SecretKey::from_slice(&[42; 32]).expect("key");
    let invoice = InvoiceBuilder::new(Currency::Regtest)
        .description("offline zap vector".into())
        .payment_hash(hash)
        .payment_secret(PaymentSecret([9; 32]))
        .amount_milli_satoshis(1000)
        .duration_since_epoch(Duration::from_secs(90))
        .min_final_cltv_expiry_delta(144)
        .build_signed(|m| Secp256k1::new().sign_ecdsa_recoverable(m, &key))
        .expect("invoice")
        .to_string();
    let zap = EventBuilder::new(Kind::Custom(9735), "")
        .tags([
            Tag::parse(["P", f.buyer.public_key().to_hex().as_str()]).expect("buyer"),
            Tag::parse(["a", f.coordinate().as_str()]).expect("coordinate"),
            Tag::parse(["bolt11", invoice.as_str()]).expect("public proof"),
        ])
        .custom_created_at(Timestamp::from_secs(95))
        .sign_with_keys(&lsp)
        .expect("zap");
    let old = f.root();
    let mut tags = old
        .tags
        .iter()
        .filter(|t| {
            !matches!(
                t.as_slice().first().map(String::as_str),
                Some("payment_hash" | "proof")
            )
        })
        .cloned()
        .collect::<Vec<_>>();
    tags.push(Tag::parse(["payment_hash", hash.to_string().as_str()]).expect("hash"));
    tags.push(Tag::parse(["proof", "zap", zap.id.to_hex().as_str()]).expect("zap reference"));
    let root = EventBuilder::new(Kind::Custom(1020), old.content)
        .tags(tags)
        .custom_created_at(Timestamp::from_secs(100))
        .sign_with_keys(&f.developer)
        .expect("root");
    let report = f
        .service()
        .ingest(
            f.buyer.public_key(),
            vec![root.clone(), zap.clone()],
            None,
            &|| Ok(()),
        )
        .await
        .expect("import pending public metadata");
    assert_eq!(report.chains[0].verification, V::Indeterminate);
    assert!(!f.owned().await);
    let mut relays = source(vec![root.clone(), zap.clone()]);
    relays.lsp = Some(lsp.public_key());
    let report = f
        .service()
        .synchronize(f.buyer.public_key(), &relays, false, 1000, &|| Ok(()))
        .await
        .expect("resolve proof");
    assert_eq!(report.chains[0].verification, V::Verified);
    assert!(f.owned().await);
    relays.fail.store(true, Ordering::SeqCst);
    relays.lsp = None;
    f.service()
        .synchronize(f.buyer.public_key(), &relays, false, 1001, &|| Ok(()))
        .await
        .expect("offline cached evidence");
    assert!(f.owned().await);
    // Aggregate Zap legs use the same proof verifier with the exact leg hash,
    // amount and destination-bound LSP, not a preimage/recipient-key inference.
    use arcadestr_core::adp_settlement::{aggregate_commitment, AggregateProofLeg, PaymentRole};
    let leg = AggregateProofLeg {
        leg_id: "developer".into(),
        role: PaymentRole::Developer,
        recipient_pubkey: f.developer.public_key().to_hex(),
        amount_msat: 1000,
        payment_hash: hash.to_string(),
        proof_type: "zap".into(),
        proof_reference: zap.id.to_hex(),
        accepted_terms_id: None,
    };
    let commitment = aggregate_commitment(1000, &[leg]).expect("aggregate");
    let mut tags = root
        .tags
        .iter()
        .filter(|t| {
            !matches!(
                t.as_slice().first().map(String::as_str),
                Some("payment_hash" | "amount_msat" | "proof")
            )
        })
        .cloned()
        .collect::<Vec<_>>();
    for tag in [
        vec!["payment_set".into(), "1".into(), "1000".into(), commitment],
        vec![
            "payment_leg".into(),
            "developer".into(),
            "developer".into(),
            f.developer.public_key().to_hex(),
            "1000".into(),
            hash.to_string(),
            "zap".into(),
            zap.id.to_hex(),
            String::new(),
        ],
        vec!["sale_listing".into(), "22".repeat(32)],
        vec!["sale_quote".into(), "33".repeat(32)],
        vec!["sale_processor".into(), f.developer.public_key().to_hex()],
    ] {
        tags.push(Tag::parse(tag).expect("tag"));
    }
    let aggregate = EventBuilder::new(Kind::Custom(1020), root.content)
        .tags(tags)
        .custom_created_at(Timestamp::from_secs(100))
        .sign_with_keys(&f.developer)
        .expect("aggregate receipt");
    let fresh = Database::new(&f._temp.path().join("aggregate-zap.db"))
        .await
        .expect("fresh device");
    let service = ReceiptPortability::new(fresh.pool().clone());
    let report = service
        .ingest(
            f.buyer.public_key(),
            vec![aggregate.clone(), zap.clone()],
            None,
            &|| Ok(()),
        )
        .await
        .expect("offline missing LSP");
    assert_eq!(report.chains[0].verification, V::Indeterminate);
    let mut source = source(vec![aggregate, zap]);
    source.lsp = Some(lsp.public_key());
    let report = service
        .synchronize(f.buyer.public_key(), &source, false, 1000, &|| Ok(()))
        .await
        .expect("aggregate public proof");
    assert_eq!(report.chains[0].verification, V::Verified);
}

#[tokio::test]
async fn aggregate_zap_destinations_are_pinned_to_signed_listing_and_accepted_terms() {
    use arcadestr_core::{
        adp_settlement::{
            build_terms_event_builder, AggregateProofLeg, CommercialTerms, FeePolicy, PaymentRole,
            TermsPayload,
        },
        purchases::{parse_receipt_event, AggregateReceipt},
        receipt_portability::aggregate_zap_destination,
    };
    let f = Fixture::new().await;
    let operator = Keys::generate();
    let terms = build_terms_event_builder(&TermsPayload::Offer {
        terms: CommercialTerms {
            version: 1,
            operator_pubkey: operator.public_key().to_hex(),
            payment_destination: "carol@example.test".into(),
            fee: FeePolicy::Fixed { amount_msat: 100 },
            limitations: Vec::new(),
        },
    })
    .expect("offer")
    .sign_with_keys(&operator)
    .expect("terms");
    let listing = EventBuilder::new(Kind::Custom(30402), "")
        .tags([
            Tag::parse(["d", "game"]).expect("d"),
            Tag::parse(["lud16", "bob@example.test"]).expect("address"),
            Tag::parse([
                "adp_terms",
                operator.public_key().to_hex().as_str(),
                terms.id.to_hex().as_str(),
            ])
            .expect("accept"),
        ])
        .sign_with_keys(&f.developer)
        .expect("listing");
    let mut parsed = parse_receipt_event(&f.root()).expect("root");
    parsed.aggregate = Some(AggregateReceipt {
        version: 1,
        gross_msat: 1000,
        commitment: String::new(),
        listing_event_id: listing.id,
        quote_event_id: listing.id,
        processor_pubkey: operator.public_key(),
        legs: Vec::new(),
    });
    let mut leg = AggregateProofLeg {
        leg_id: "developer".into(),
        role: PaymentRole::Developer,
        recipient_pubkey: f.developer.public_key().to_hex(),
        amount_msat: 900,
        payment_hash: "11".repeat(32),
        proof_type: "zap".into(),
        proof_reference: "22".repeat(32),
        accepted_terms_id: None,
    };
    assert_eq!(
        aggregate_zap_destination(&parsed, &leg, &listing, None).expect("developer"),
        "bob@example.test"
    );
    leg.role = PaymentRole::Operator;
    leg.recipient_pubkey = operator.public_key().to_hex();
    leg.accepted_terms_id = Some(terms.id.to_hex());
    assert_eq!(
        aggregate_zap_destination(&parsed, &leg, &listing, Some(&terms)).expect("operator"),
        "carol@example.test"
    );
    assert!(aggregate_zap_destination(&parsed, &leg, &listing, None).is_err());
    leg.recipient_pubkey = Keys::generate().public_key().to_hex();
    assert!(aggregate_zap_destination(&parsed, &leg, &listing, Some(&terms)).is_err());
    let mut forged = listing;
    forged.content.push_str("tampered");
    assert!(aggregate_zap_destination(&parsed, &leg, &forged, Some(&terms)).is_err());
}

#[tokio::test]
async fn restart_uses_persisted_incremental_overlap_and_rescan_is_explicit() {
    let f = Fixture::new().await;
    let root = f.event(&f.developer, "paid", None, "order", 1000, None);
    let relays = source(vec![root]);
    f.service()
        .synchronize(f.buyer.public_key(), &relays, false, 2000, &|| Ok(()))
        .await
        .expect("backfill");
    relays.filters.lock().expect("queries").clear();
    let reopened = Database::new(&f._temp.path().join("app.db"))
        .await
        .expect("restart");
    ReceiptPortability::new(reopened.pool().clone())
        .synchronize(f.buyer.public_key(), &relays, false, 2001, &|| Ok(()))
        .await
        .expect("incremental restart");
    assert!(relays
        .filters
        .lock()
        .expect("queries")
        .iter()
        .any(|f| f["since"] == 700));
    relays.filters.lock().expect("queries").clear();
    f.service()
        .synchronize(f.buyer.public_key(), &relays, true, 2002, &|| Ok(()))
        .await
        .expect("rescan");
    assert!(relays
        .filters
        .lock()
        .expect("queries")
        .iter()
        .any(|f| f["since"] == 0));
}

#[tokio::test]
async fn cancellation_before_commit_rolls_back_staging_projection_and_assessment() {
    let f = Fixture::new().await;
    let calls = AtomicUsize::new(0);
    let result = f
        .service()
        .ingest(f.buyer.public_key(), vec![f.root()], None, &|| {
            if calls.fetch_add(1, Ordering::SeqCst) >= 2 {
                Err("session interrupted".into())
            } else {
                Ok(())
            }
        })
        .await;
    assert!(result.is_err());
    for table in [
        "purchases",
        "receipt_sync_events",
        "receipt_chain_verifications",
    ] {
        let count: i64 = sqlx::query_scalar(&format!("SELECT COUNT(*) FROM {table}"))
            .fetch_one(f.db.pool())
            .await
            .expect("rolled back");
        assert_eq!(count, 0);
    }
}

#[tokio::test]
async fn simultaneous_imports_merge_forks_before_projecting_ownership() {
    let f = Fixture::new().await;
    let root = f.root();
    let a = f.event(&f.developer, "fulfilled", Some(&root), "order", 101, None);
    let b = f.event(&f.developer, "disputed", Some(&root), "order", 102, None);
    let one = f.service();
    let two = f.service();
    let (a, b) = tokio::join!(
        one.ingest(
            f.buyer.public_key(),
            vec![root.clone(), a],
            None,
            &|| Ok(())
        ),
        two.ingest(f.buyer.public_key(), vec![root, b], None, &|| Ok(()))
    );
    a.expect("first commit");
    b.expect("second commit");
    assert!(!f.owned().await);
    let verification: String =
        sqlx::query_scalar("SELECT verification FROM receipt_chain_verifications")
            .fetch_one(f.db.pool())
            .await
            .expect("assessment");
    assert_eq!(verification, "invalid");
}

#[tokio::test]
async fn older_verified_local_delegation_survives_outage_without_trusting_new_imports() {
    use arcadestr_core::{
        authorization::resolve_authorization,
        purchases::{parse_and_validate_receipt_with_authorization, ReceiptEvidence},
    };
    let f = Fixture::new().await;
    let key = Keys::generate();
    let auth = f.auth(&key, None, 1);
    let root = f.event(&key, "paid", None, "order", 100, Some(&auth));
    let authority =
        resolve_authorization(auth.id, &[auth.clone()]).expect("prior verified authority");
    let verified = parse_and_validate_receipt_with_authorization(
        &root,
        &f.buyer.public_key().to_hex(),
        Some(&authority),
        ReceiptEvidence::default(),
    )
    .expect("existing local confirmation");
    PurchasesRepository::new(f.db.pool().clone())
        .upsert_receipt(&verified)
        .await
        .expect("pre-portability record");
    let relays = source(Vec::new());
    relays.fail.store(true, Ordering::SeqCst);
    for now in [1000, 1001] {
        let report = f
            .service()
            .synchronize(f.buyer.public_key(), &relays, false, now, &|| Ok(()))
            .await
            .expect("outage");
        assert!(report.incomplete);
        assert_eq!(report.chains[0].verification, V::Verified);
        assert!(f.owned().await);
    }
    let other = Database::new(&f._temp.path().join("another-device.db"))
        .await
        .expect("fresh device");
    let report = ReceiptPortability::new(other.pool().clone())
        .ingest(f.buyer.public_key(), vec![root], None, &|| Ok(()))
        .await
        .expect("unverifiable new import");
    assert_eq!(report.chains[0].verification, V::Indeterminate);
    assert!(!PurchasesRepository::new(other.pool().clone())
        .is_owned(&f.buyer.public_key().to_hex(), &f.coordinate())
        .await
        .expect("no new grant"));
}
