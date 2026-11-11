use arcadestr_core::entitlement_reconciliation::{EntitlementEvidence, EntitlementSource};

struct EvidenceSource {
    events: Vec<Event>,
    unavailable: bool,
    incomplete: bool,
}
#[async_trait::async_trait]
impl EntitlementSource for EvidenceSource {
    async fn query(&self, filter: nostr::Filter) -> Result<EntitlementEvidence, String> {
        let json = serde_json::to_string(&filter).expect("filter");
        assert!(
            json.contains("1030")
                && json.contains("authors")
                && json.contains("#p")
                && json.contains("#d")
        );
        assert!(
            !json.contains("since"),
            "old revocations must not be hidden by a receipt cursor"
        );
        if self.unavailable {
            Err("isolated outage".into())
        } else {
            Ok(EntitlementEvidence {
                events: self.events.clone(),
                incomplete: self.incomplete,
            })
        }
    }
}
fn evidence(events: Vec<Event>) -> EvidenceSource {
    EvidenceSource {
        events,
        unavailable: false,
        incomplete: false,
    }
}
fn revise(event: &Event, signer: &Keys, name: &str, value: &str) -> Event {
    let mut tags = event
        .tags
        .iter()
        .filter(|t| t.as_slice()[0] != name)
        .cloned()
        .collect::<Vec<_>>();
    tags.push(tag(&[name, value]));
    EventBuilder::new(event.kind, event.content.clone())
        .tags(tags)
        .custom_created_at(event.created_at)
        .sign_with_keys(signer)
        .expect("fixture revision")
}

#[tokio::test]
async fn gate7_historical_campaign_and_delegation_survive_cancellation_but_allow_developer_revocation(
) {
    use arcadestr_core::{
        adp_protocol::FULFILLMENT_AUTHORIZATION_KIND,
        authorization::{resolve_authorization_at, CAPABILITY_ISSUE_GRANT},
    };
    let dir = tempfile::tempdir_in("/tmp/opencode").expect("directory");
    let db = Database::new(&dir.path().join("buyer.db"))
        .await
        .expect("db");
    let repo = EntitlementsRepository::new(db.pool().clone());
    let developer = Keys::generate();
    let issuer = Keys::generate();
    let buyer = Keys::generate();
    let coordinate = format!("30402:{}:game", developer.public_key());
    let original = campaign(&developer, &coordinate);
    let cancelled = EventBuilder::new(
        Kind::Custom(arcadestr_core::adp_protocol::ADP_CAMPAIGN_KIND),
        "",
    )
    .tags([
        tag(&["d", "campaign-1"]),
        tag(&["a", &coordinate]),
        tag(&["status", "cancelled"]),
        tag(&["e", &original.root_event_id.to_hex()]),
    ])
    .custom_created_at(Timestamp::from_secs(170))
    .sign_with_keys(&developer)
    .expect("campaign cancellation");
    let mut campaign_events = original.events.clone();
    campaign_events.push(parse_campaign_event(&cancelled).expect("parse cancellation"));
    let historical = resolve_campaign(&campaign_events, developer.public_key(), &coordinate)
        .expect("historical campaign");
    let operator = Keys::generate();
    let tags = vec![
        tag(&["d", "authorization"]),
        tag(&["a", &coordinate]),
        tag(&["p", &operator.public_key().to_hex()]),
        tag(&["fulfillment_pubkey", &issuer.public_key().to_hex()]),
        tag(&["capability", CAPABILITY_ISSUE_GRANT]),
        tag(&["valid_from", "110"]),
        tag(&["status", "active"]),
    ];
    let authority = EventBuilder::new(Kind::Custom(FULFILLMENT_AUTHORIZATION_KIND), "")
        .tags(tags.clone())
        .custom_created_at(Timestamp::from_secs(100))
        .sign_with_keys(&developer)
        .expect("authority");
    let mut cancelled_tags = tags;
    cancelled_tags.retain(|t| t.as_slice()[0] != "status");
    cancelled_tags.extend([
        tag(&["status", "cancelled"]),
        tag(&["e", &authority.id.to_hex()]),
    ]);
    let cancelled = EventBuilder::new(Kind::Custom(FULFILLMENT_AUTHORIZATION_KIND), "")
        .tags(cancelled_tags)
        .custom_created_at(Timestamp::from_secs(170))
        .sign_with_keys(&developer)
        .expect("authority cancellation");
    let authorization =
        resolve_authorization_at(authority.id, &[authority.clone(), cancelled], 200)
            .expect("historical authority");
    let root = grant(
        &issuer,
        &buyer,
        &coordinate,
        "historical",
        historical.root_event_id,
        GrantStatus::Granted,
        None,
        150,
    );
    let root = revise(&root, &issuer, "authorization", &authority.id.to_hex());
    repo.ingest_event(&root, &historical, Some(&authorization))
        .await
        .expect("valid before cancellation");
    let late = grant(
        &issuer,
        &buyer,
        &coordinate,
        "late",
        historical.root_event_id,
        GrantStatus::Granted,
        None,
        180,
    );
    let late = revise(&late, &issuer, "authorization", &authority.id.to_hex());
    assert!(
        repo.ingest_event(&late, &historical, Some(&authorization))
            .await
            .is_err(),
        "late claim remains forbidden"
    );
    let report = repo
        .reconcile_known_grants(buyer.public_key(), &evidence(vec![]), &|| Ok(()))
        .await
        .expect("no revocation");
    assert_eq!(report.revoked, 0);
    assert!(repo
        .is_owned(&buyer.public_key().to_hex(), &coordinate)
        .await
        .expect("historical grant"));
    let revoke = grant(
        &developer,
        &buyer,
        &coordinate,
        "historical",
        historical.root_event_id,
        GrantStatus::Revoked,
        Some(root.id),
        200,
    );
    let revoke = revise(&revoke, &developer, "authorization", &authority.id.to_hex());
    let unauthorized = grant(
        &issuer,
        &buyer,
        &coordinate,
        "historical",
        historical.root_event_id,
        GrantStatus::Revoked,
        Some(root.id),
        190,
    );
    let unauthorized = revise(
        &unauthorized,
        &issuer,
        "authorization",
        &authority.id.to_hex(),
    );
    let report = repo
        .reconcile_known_grants(
            buyer.public_key(),
            &evidence(vec![unauthorized]),
            &|| Ok(()),
        )
        .await
        .expect("ignore fulfillment revoker");
    assert_eq!(report.revoked, 0);
    let report = repo
        .reconcile_known_grants(buyer.public_key(), &evidence(vec![revoke]), &|| Ok(()))
        .await
        .expect("developer can revoke historical grant");
    assert_eq!(report.revoked, 1);
    assert!(!repo
        .is_owned(&buyer.public_key().to_hex(), &coordinate)
        .await
        .expect("revoked"));
}

#[tokio::test]
async fn gate7_paid_ownership_other_grants_and_other_accounts_are_untouched() {
    use arcadestr_core::{
        receipt_portability::ReceiptPortability, receipt_sync_contract::ReceiptVerification,
    };
    let dir = tempfile::tempdir_in("/tmp/opencode").expect("directory");
    let db = Database::new(&dir.path().join("buyer.db"))
        .await
        .expect("db");
    let repo = EntitlementsRepository::new(db.pool().clone());
    let developer = Keys::generate();
    let buyer = Keys::generate();
    let other = Keys::generate();
    let paid_coordinate = format!("30402:{}:paid", developer.public_key());
    let free_coordinate = format!("30402:{}:free", developer.public_key());
    let mut roots = vec![];
    let mut revocations = vec![];
    for (index, (recipient, coordinate)) in [
        (&buyer, &paid_coordinate),
        (&buyer, &free_coordinate),
        (&buyer, &free_coordinate),
        (&other, &free_coordinate),
    ]
    .into_iter()
    .enumerate()
    {
        let campaign = campaign(&developer, coordinate);
        let id = format!("grant-{index}");
        let root = grant(
            &developer,
            recipient,
            coordinate,
            &id,
            campaign.root_event_id,
            GrantStatus::Granted,
            None,
            150,
        );
        repo.ingest_event(&root, &campaign, None)
            .await
            .expect("known grant");
        revocations.push(grant(
            &developer,
            recipient,
            coordinate,
            &id,
            campaign.root_event_id,
            GrantStatus::Revoked,
            Some(root.id),
            160,
        ));
        roots.push(root);
    }
    let receipt = EventBuilder::new(Kind::Custom(1020), "encrypted fixture")
        .tags([
            tag(&["a", &paid_coordinate]),
            tag(&["p", &buyer.public_key().to_hex()]),
            tag(&["order", "already-paid"]),
            tag(&["status", "paid"]),
            tag(&["settled_at", "140"]),
            tag(&["payment_hash", &"11".repeat(32)]),
            tag(&["amount_msat", "1000"]),
            tag(&["proof", "bolt11-preimage"]),
        ])
        .custom_created_at(Timestamp::from_secs(140))
        .sign_with_keys(&developer)
        .expect("signed paid receipt");
    let report = ReceiptPortability::new(db.pool().clone())
        .import_json(
            buyer.public_key(),
            &serde_json::to_string(&vec![receipt]).expect("JSON"),
            None,
            &|| Ok(()),
        )
        .await
        .expect("existing paid receipt");
    assert!(report
        .chains
        .iter()
        .any(|c| c.verification == ReceiptVerification::Verified));
    let before: Vec<(String, String)> = sqlx::query_as("SELECT event_id,raw_event FROM purchases")
        .fetch_all(db.pool())
        .await
        .expect("paid snapshot");
    repo.reconcile_known_grants(
        buyer.public_key(),
        &evidence(vec![
            revocations[0].clone(),
            revocations[1].clone(),
            revocations[3].clone(),
        ]),
        &|| Ok(()),
    )
    .await
    .expect("reconcile");
    let ownership =
        OwnershipService::new(PurchasesRepository::new(db.pool().clone()), repo.clone());
    assert_eq!(
        ownership
            .source_for(&buyer.public_key().to_hex(), &paid_coordinate)
            .await
            .expect("paid"),
        OwnershipSource::PurchaseReceipt
    );
    assert!(repo
        .is_owned(&buyer.public_key().to_hex(), &free_coordinate)
        .await
        .expect("unrelated grant stays active"));
    assert!(repo
        .is_owned(&other.public_key().to_hex(), &free_coordinate)
        .await
        .expect("other account not reconciled"));
    let after: Vec<(String, String)> = sqlx::query_as("SELECT event_id,raw_event FROM purchases")
        .fetch_all(db.pool())
        .await
        .expect("paid after");
    assert_eq!(before, after, "NIP-102 storage untouched");
    let history = ownership
        .durable_records_for(&buyer.public_key().to_hex())
        .await
        .expect("History");
    assert!(history
        .iter()
        .any(|h| h.status == arcadestr_core::ownership::DurableCredentialStatus::Revoked));
    assert_eq!(
        ownership
            .library_entries_for(&buyer.public_key().to_hex(), vec![])
            .await
            .expect("Library")
            .len(),
        2,
        "paid + independent valid grant remain downloadable"
    );
}

#[tokio::test]
async fn gate7_adversarial_revocation_batches_never_invent_or_remove_ownership() {
    let developer = Keys::generate();
    let buyer = Keys::generate();
    let other = Keys::generate();
    let coordinate = format!("30402:{}:game", developer.public_key());
    let campaign = campaign(&developer, &coordinate);
    let root = grant(
        &developer,
        &buyer,
        &coordinate,
        "known",
        campaign.root_event_id,
        GrantStatus::Granted,
        None,
        150,
    );
    let revocation = grant(
        &developer,
        &buyer,
        &coordinate,
        "known",
        campaign.root_event_id,
        GrantStatus::Revoked,
        Some(root.id),
        160,
    );
    for case in [
        "outage",
        "empty",
        "incomplete-empty",
        "unauthorized",
        "bad-signature",
        "malformed",
        "wrong-buyer",
        "wrong-coordinate",
        "wrong-grant",
        "wrong-source",
        "wrong-authorization",
        "missing-predecessor",
        "equal-time",
        "older-time",
        "fork",
        "unrelated-root",
        "terminal-extension",
        "valid-partial",
        "valid-duplicates",
    ] {
        let dir = tempfile::tempdir_in("/tmp/opencode").expect("db directory");
        let db = Database::new(&dir.path().join("buyer.db"))
            .await
            .expect("database");
        let repo = EntitlementsRepository::new(db.pool().clone());
        repo.ingest_event(&root, &campaign, None)
            .await
            .expect("known root");
        let mut source = evidence(vec![revocation.clone()]);
        match case {
            "outage" => source.unavailable = true,
            "empty" => source.events.clear(),
            "incomplete-empty" => {
                source.events.clear();
                source.incomplete = true;
            }
            "unauthorized" => {
                source.events = vec![grant(
                    &other,
                    &buyer,
                    &coordinate,
                    "known",
                    campaign.root_event_id,
                    GrantStatus::Revoked,
                    Some(root.id),
                    160,
                )]
            }
            "bad-signature" => source.events[0].content = "tampered".into(),
            "malformed" => source.events[0] = revise(&revocation, &developer, "status", "unknown"),
            "wrong-buyer" => {
                source.events[0] =
                    revise(&revocation, &developer, "p", &other.public_key().to_hex())
            }
            "wrong-coordinate" => {
                source.events[0] = revise(
                    &revocation,
                    &developer,
                    "a",
                    &format!("30402:{}:other", developer.public_key()),
                )
            }
            "wrong-grant" => source.events[0] = revise(&revocation, &developer, "d", "other"),
            "wrong-source" => {
                source.events[0] = revise(
                    &revocation,
                    &developer,
                    "source_event",
                    &EventId::all_zeros().to_hex(),
                )
            }
            "wrong-authorization" => {
                source.events[0] = revise(
                    &revocation,
                    &developer,
                    "authorization",
                    &EventId::all_zeros().to_hex(),
                )
            }
            "missing-predecessor" => {
                source.events[0] =
                    revise(&revocation, &developer, "e", &EventId::all_zeros().to_hex())
            }
            "equal-time" | "older-time" => {
                source.events = vec![grant(
                    &developer,
                    &buyer,
                    &coordinate,
                    "known",
                    campaign.root_event_id,
                    GrantStatus::Revoked,
                    Some(root.id),
                    if case == "equal-time" { 150 } else { 149 },
                )]
            }
            "fork" => source.events.push(grant(
                &developer,
                &buyer,
                &coordinate,
                "known",
                campaign.root_event_id,
                GrantStatus::Revoked,
                Some(root.id),
                161,
            )),
            "unrelated-root" => {
                source.events = vec![grant(
                    &developer,
                    &buyer,
                    &coordinate,
                    "unseen",
                    campaign.root_event_id,
                    GrantStatus::Granted,
                    None,
                    151,
                )]
            }
            "terminal-extension" => source.events.push(grant(
                &developer,
                &buyer,
                &coordinate,
                "known",
                campaign.root_event_id,
                GrantStatus::Revoked,
                Some(revocation.id),
                170,
            )),
            "valid-partial" => source.incomplete = true,
            "valid-duplicates" => source.events.extend([
                root.clone(),
                revocation.clone(),
                grant(
                    &other,
                    &buyer,
                    &coordinate,
                    "known",
                    campaign.root_event_id,
                    GrantStatus::Revoked,
                    Some(root.id),
                    160,
                ),
            ]),
            _ => {}
        }
        let report = repo
            .reconcile_known_grants(buyer.public_key(), &source, &|| Ok(()))
            .await
            .expect("reconciliation");
        let changed = matches!(case, "valid-partial" | "valid-duplicates");
        assert_eq!(report.revoked, usize::from(changed), "{case}");
        assert!(
            report.incomplete,
            "relay absence is never a proof of global completeness"
        );
        assert_eq!(
            repo.is_owned(&buyer.public_key().to_hex(), &coordinate)
                .await
                .expect("ownership"),
            !changed,
            "{case}"
        );
        let history = repo
            .history_for_buyer(&buyer.public_key().to_hex())
            .await
            .expect("history");
        assert_eq!(history.len(), 1, "no new roots imported in {case}");
        assert_eq!(
            history[0].status,
            Some(if changed {
                GrantStatus::Revoked
            } else {
                GrantStatus::Granted
            }),
            "{case}"
        );
        if matches!(
            case,
            "outage"
                | "incomplete-empty"
                | "fork"
                | "missing-predecessor"
                | "bad-signature"
                | "malformed"
                | "wrong-source"
                | "wrong-authorization"
                | "terminal-extension"
        ) {
            assert!(report.indeterminate > 0, "{case}");
        }
        db.close().await;
    }
}

struct SwitchingSource {
    coordinator: arcadestr_core::settlement_recovery::RecoveryCoordinator,
    events: Vec<Event>,
    next: Option<String>,
    original: String,
}

struct DelayedEvidence {
    started: tokio::sync::Notify,
    release: tokio::sync::Notify,
    snapshot: Vec<Event>,
}
#[async_trait::async_trait]
impl EntitlementSource for DelayedEvidence {
    async fn query(&self, _: nostr::Filter) -> Result<EntitlementEvidence, String> {
        self.started.notify_one();
        self.release.notified().await;
        Ok(EntitlementEvidence {
            events: self.snapshot.clone(),
            incomplete: false,
        })
    }
}

#[tokio::test]
async fn gate7_delayed_old_snapshot_cannot_restore_a_concurrently_revoked_grant() {
    let dir = tempfile::tempdir_in("/tmp/opencode").expect("directory");
    let db = Database::new(&dir.path().join("buyer.db"))
        .await
        .expect("database");
    let repo = EntitlementsRepository::new(db.pool().clone());
    let developer = Keys::generate();
    let buyer = Keys::generate();
    let coordinate = format!("30402:{}:game", developer.public_key());
    let campaign = campaign(&developer, &coordinate);
    let root = grant(
        &developer,
        &buyer,
        &coordinate,
        "known",
        campaign.root_event_id,
        GrantStatus::Granted,
        None,
        150,
    );
    let revoke = grant(
        &developer,
        &buyer,
        &coordinate,
        "known",
        campaign.root_event_id,
        GrantStatus::Revoked,
        Some(root.id),
        160,
    );
    repo.ingest_event(&root, &campaign, None)
        .await
        .expect("root");
    let delayed = std::sync::Arc::new(DelayedEvidence {
        started: Default::default(),
        release: Default::default(),
        snapshot: vec![root],
    });
    let old_repo = repo.clone();
    let old_source = delayed.clone();
    let account = buyer.public_key();
    let old = tokio::spawn(async move {
        old_repo
            .reconcile_known_grants(account, old_source.as_ref(), &|| Ok(()))
            .await
    });
    delayed.started.notified().await;
    assert_eq!(
        repo.reconcile_known_grants(account, &evidence(vec![revoke]), &|| Ok(()))
            .await
            .expect("newer response")
            .revoked,
        1
    );
    delayed.release.notify_one();
    assert_eq!(
        old.await.expect("old task").expect("old response").revoked,
        0
    );
    assert!(!repo
        .is_owned(&account.to_hex(), &coordinate)
        .await
        .expect("append-only revocation"));
}
#[async_trait::async_trait]
impl EntitlementSource for SwitchingSource {
    async fn query(&self, _: nostr::Filter) -> Result<EntitlementEvidence, String> {
        self.coordinator.activate(self.next.clone());
        if self.next.is_some() {
            self.coordinator.activate(Some(self.original.clone()));
        }
        Ok(EntitlementEvidence {
            events: self.events.clone(),
            incomplete: false,
        })
    }
}

#[tokio::test]
async fn gate7_session_changes_and_precommit_cancellation_rollback_reconciliation() {
    use arcadestr_core::settlement_recovery::RecoveryCoordinator;
    let dir = tempfile::tempdir_in("/tmp/opencode").expect("directory");
    let db = Database::new(&dir.path().join("buyer.db"))
        .await
        .expect("db");
    let repo = EntitlementsRepository::new(db.pool().clone());
    let developer = Keys::generate();
    let buyer = Keys::generate();
    let coordinate = format!("30402:{}:game", developer.public_key());
    let campaign = campaign(&developer, &coordinate);
    let root = grant(
        &developer,
        &buyer,
        &coordinate,
        "known",
        campaign.root_event_id,
        GrantStatus::Granted,
        None,
        150,
    );
    let revoke = grant(
        &developer,
        &buyer,
        &coordinate,
        "known",
        campaign.root_event_id,
        GrantStatus::Revoked,
        Some(root.id),
        160,
    );
    repo.ingest_event(&root, &campaign, None)
        .await
        .expect("root");
    let coordinator = RecoveryCoordinator::default();
    for next in [
        None,
        Some(Keys::generate().public_key().to_hex()),
        Some(buyer.public_key().to_hex()),
    ] {
        coordinator.activate(Some(buyer.public_key().to_hex()));
        let session = coordinator.session_fence().expect("fence");
        let source = SwitchingSource {
            coordinator: coordinator.clone(),
            events: vec![revoke.clone()],
            next,
            original: buyer.public_key().to_hex(),
        };
        assert!(repo
            .reconcile_known_grants(buyer.public_key(), &source, &|| session.ensure_current())
            .await
            .is_err());
        assert!(repo
            .is_owned(&buyer.public_key().to_hex(), &coordinate)
            .await
            .expect("unchanged"));
    }
    // Inject a session cancellation after the INSERT but before COMMIT.
    let checks = std::sync::atomic::AtomicUsize::new(0);
    let result = repo
        .reconcile_known_grants(buyer.public_key(), &evidence(vec![revoke.clone()]), &|| {
            if checks.fetch_add(1, std::sync::atomic::Ordering::SeqCst) >= 5 {
                Err("cancel before commit".into())
            } else {
                Ok(())
            }
        })
        .await;
    assert!(result.is_err());
    assert!(repo
        .is_owned(&buyer.public_key().to_hex(), &coordinate)
        .await
        .expect("rollback"));
    repo.reconcile_known_grants(buyer.public_key(), &evidence(vec![revoke]), &|| Ok(()))
        .await
        .expect("fresh explicit retry");
    // Delayed old/no-evidence response cannot overwrite the append-only revocation.
    repo.reconcile_known_grants(buyer.public_key(), &evidence(vec![root]), &|| Ok(()))
        .await
        .expect("late snapshot");
    assert!(!repo
        .is_owned(&buyer.public_key().to_hex(), &coordinate)
        .await
        .expect("monotonic"));
    db.close().await;
    let reopened = Database::new(&dir.path().join("buyer.db"))
        .await
        .expect("restart");
    let repo = EntitlementsRepository::new(reopened.pool().clone());
    let report = repo
        .reconcile_known_grants(
            buyer.public_key(),
            &EvidenceSource {
                events: vec![],
                unavailable: true,
                incomplete: true,
            },
            &|| Ok(()),
        )
        .await
        .expect("offline restart");
    assert_eq!(report.revoked, 0);
    assert!(!repo
        .is_owned(&buyer.public_key().to_hex(), &coordinate)
        .await
        .expect("durable revocation"));
}
