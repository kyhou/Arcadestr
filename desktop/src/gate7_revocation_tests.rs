// Uses only loopback Nostr and independent disposable publisher/buyer databases.
#[tokio::test]
async fn gate7_native_independent_revocation_reconnect_restart_and_download_denial() {
    use arcadestr_core::{
        adp_protocol::{ADP_CAMPAIGN_KIND, ENTITLEMENT_GRANT_KIND},
        campaign::{parse_campaign_event, resolve_campaign},
        entitlements_repository::EntitlementsRepository,
        ownership::{DurableCredentialStatus, OwnershipService},
        purchases::PurchasesRepository,
        storage::Database,
    };
    let temp = tempfile::tempdir().expect("isolated root");
    let publisher_db = Database::new(&temp.path().join("publisher.db"))
        .await
        .expect("publisher DB");
    let events: Events = Arc::new(Mutex::new(vec![]));
    let (url, _relay) = relay(events.clone()).await;
    let http: Arc<dyn HttpClient> = Arc::new(IsolatedHttp {
        real: ReqwestHttpClient::new(Duration::from_secs(5)).expect("HTTP"),
        urls: vec![],
    });
    let app = test_app_with_http(
        vec![url.clone()],
        temp.path().join("buyer.db"),
        http.clone(),
    )
    .await;
    let developer = Keys::generate();
    let buyer = Keys::generate();
    let coordinate = format!("30402:{}:game", developer.public_key());
    switch(&app, &buyer.secret_key().to_secret_hex()).await;
    let campaign_event = EventBuilder::new(Kind::Custom(ADP_CAMPAIGN_KIND), "")
        .tags([
            tag(&["d", "independent"]),
            tag(&["a", &coordinate]),
            tag(&["mode", "claim"]),
            tag(&["starts", "120"]),
            tag(&["ends", "300"]),
            tag(&["status", "active"]),
        ])
        .custom_created_at(Timestamp::from_secs(100))
        .sign_with_keys(&developer)
        .expect("campaign");
    let campaign = resolve_campaign(
        &[parse_campaign_event(&campaign_event).expect("parsed")],
        developer.public_key(),
        &coordinate,
    )
    .expect("campaign");
    let root_tags = vec![
        tag(&["d", "independent-grant"]),
        tag(&["a", &coordinate]),
        tag(&["p", &buyer.public_key().to_hex()]),
        tag(&["source_event", &campaign.root_event_id.to_hex()]),
        tag(&["status", "granted"]),
    ];
    let root = EventBuilder::new(Kind::Custom(ENTITLEMENT_GRANT_KIND), "")
        .tags(root_tags.clone())
        .custom_created_at(Timestamp::from_secs(150))
        .sign_with_keys(&developer)
        .expect("root");
    let mut tags = root_tags;
    tags.retain(|t| t.as_slice()[0] != "status");
    tags.extend([tag(&["e", &root.id.to_hex()]), tag(&["status", "revoked"])]);
    let revoke = EventBuilder::new(Kind::Custom(ENTITLEMENT_GRANT_KIND), "")
        .tags(tags)
        .custom_created_at(Timestamp::from_secs(160))
        .sign_with_keys(&developer)
        .expect("revocation");
    let publisher = EntitlementsRepository::new(publisher_db.pool().clone());
    publisher
        .ingest_event(&root, &campaign, None)
        .await
        .expect("publisher root");
    let state = app.state::<AppState>();
    let buyer_repo = EntitlementsRepository::new(state.database.pool().clone());
    buyer_repo
        .ingest_event(&root, &campaign, None)
        .await
        .expect("buyer's previously issued grant");
    publisher
        .ingest_event(&revoke, &campaign, None)
        .await
        .expect("publisher revokes in separate DB");
    let report = crate::entitlement_commands::reconcile_for_account(buyer.public_key(), &state)
        .await
        .expect("before relay retention");
    assert_eq!(report.revoked, 0);
    assert!(buyer_repo
        .is_owned(&buyer.public_key().to_hex(), &coordinate)
        .await
        .expect("not inferred"));
    // Local fixture availability, not a publication command or real relay.
    events
        .lock()
        .expect("fixture store")
        .extend([campaign_event, root.clone(), revoke]);
    let report = crate::entitlement_commands::reconcile_for_account(buyer.public_key(), &state)
        .await
        .expect("reconnected evidence");
    assert_eq!(report.revoked, 1);
    let ownership = OwnershipService::new(
        PurchasesRepository::new(state.database.pool().clone()),
        buyer_repo.clone(),
    );
    assert!(!ownership
        .is_owned(&buyer.public_key().to_hex(), &coordinate)
        .await
        .expect("download eligibility"));
    assert!(ownership
        .library_entries_for(&buyer.public_key().to_hex(), vec![])
        .await
        .expect("Library")
        .is_empty());
    assert_eq!(
        ownership
            .durable_records_for(&buyer.public_key().to_hex())
            .await
            .expect("History")[0]
            .status,
        DurableCredentialStatus::Revoked
    );
    // Actual native details/eligibility reader uses this same ownership service.
    assert!(!crate::get_listing_ownership(
        buyer.public_key().to_bech32().expect("buyer npub"),
        developer.public_key().to_bech32().expect("publisher npub"),
        "game".into(),
        app.state()
    )
    .await
    .expect("native eligibility"));
    switch(&app, &Keys::generate().secret_key().to_secret_hex()).await;
    assert!(
        crate::entitlement_commands::reconcile_for_account(buyer.public_key(), &state)
            .await
            .is_err(),
        "explicit initiating-account binding"
    );
    let reopened = Database::new(&temp.path().join("buyer.db"))
        .await
        .expect("independent restart connection");
    assert!(!EntitlementsRepository::new(reopened.pool().clone())
        .is_owned(&buyer.public_key().to_hex(), &coordinate)
        .await
        .expect("durable revocation"));
}

/// ADP-01 §Entitlement Grants (gaps C10, C11): a publisher-signed grant with no
/// campaign is discovered through `#p` by two independent buyer clients that
/// never claimed it. A forged revocation and a forged grant are ignored. The
/// developer then revokes it with the real revocation command, and both
/// clients lose ownership on their next sync. Loopback relay and disposable
/// databases only.
#[tokio::test]
async fn gate7_campaignless_grant_discovery_and_revocation_across_clients() {
    use arcadestr_core::{
        adp_protocol::ENTITLEMENT_GRANT_KIND,
        entitlements_repository::EntitlementsRepository,
        ownership::{DurableCredentialStatus, OwnershipService},
        purchases::PurchasesRepository,
    };
    let temp = tempfile::tempdir().expect("isolated root");
    let events: Events = Arc::new(Mutex::new(vec![]));
    let (url, _relay) = relay(events.clone()).await;
    let http: Arc<dyn HttpClient> = Arc::new(IsolatedHttp {
        real: ReqwestHttpClient::new(Duration::from_secs(5)).expect("HTTP"),
        urls: vec![],
    });
    let developer = Keys::generate();
    let buyer = Keys::generate();
    let attacker = Keys::generate();
    let coordinate = format!("30402:{}:game", developer.public_key());
    let now = Timestamp::now().as_secs();
    let grant_tags = |grant_id: &str| {
        vec![
            tag(&["d", grant_id]),
            tag(&["a", &coordinate]),
            tag(&["p", &buyer.public_key().to_hex()]),
            tag(&["reason", "review-copy"]),
            tag(&["status", "granted"]),
        ]
    };
    // Issued by the developer outside Arcadestr: no campaign, no source_event.
    let root = EventBuilder::new(Kind::Custom(ENTITLEMENT_GRANT_KIND), "")
        .tags(grant_tags("review-copy"))
        .custom_created_at(Timestamp::from_secs(now - 60))
        .sign_with_keys(&developer)
        .expect("grant");
    let forged_revocation = EventBuilder::new(Kind::Custom(ENTITLEMENT_GRANT_KIND), "")
        .tags([
            tag(&["d", "review-copy"]),
            tag(&["a", &coordinate]),
            tag(&["p", &buyer.public_key().to_hex()]),
            tag(&["e", &root.id.to_hex()]),
            tag(&["status", "revoked"]),
        ])
        .custom_created_at(Timestamp::from_secs(now - 30))
        .sign_with_keys(&attacker)
        .expect("forged revocation");
    let forged_grant = EventBuilder::new(Kind::Custom(ENTITLEMENT_GRANT_KIND), "")
        .tags(grant_tags("forged"))
        .custom_created_at(Timestamp::from_secs(now - 60))
        .sign_with_keys(&attacker)
        .expect("forged grant");
    events
        .lock()
        .expect("relay store")
        .extend([root.clone(), forged_revocation, forged_grant]);

    let mut clients = Vec::new();
    for name in ["buyer-laptop", "buyer-desktop"] {
        let app = test_app_with_http(
            vec![url.clone()],
            temp.path().join(format!("{name}.db")),
            http.clone(),
        )
        .await;
        switch(&app, &buyer.secret_key().to_secret_hex()).await;
        clients.push(app);
    }
    let ownership = |app: &tauri::App<tauri::test::MockRuntime>| {
        let pool = app.state::<AppState>().database.pool().clone();
        OwnershipService::new(
            PurchasesRepository::new(pool.clone()),
            EntitlementsRepository::new(pool),
        )
    };
    for app in &clients {
        let (report, discovered) =
            crate::entitlement_commands::sync_for_account(buyer.public_key(), &app.state())
                .await
                .expect("discovery");
        assert_eq!(discovered, 1, "found through #p without a prior claim");
        assert!(report.rejected_events >= 1, "the forged grant is rejected");
        assert!(ownership(app)
            .is_owned(&buyer.public_key().to_hex(), &coordinate)
            .await
            .expect("ownership"));
    }

    // The developer revokes it from a third client, with the real command.
    let publisher_app = test_app_with_http(
        vec![url.clone()],
        temp.path().join("publisher.db"),
        http.clone(),
    )
    .await;
    switch(&publisher_app, &developer.secret_key().to_secret_hex()).await;
    let revocation = crate::publisher_management::revoke_publisher_credential(
        RevokePublisherCredentialRequest {
            expected_publisher_npub: developer.public_key().to_bech32().expect("npub"),
            coordinate: coordinate.clone(),
            root_event_id: root.id.to_hex(),
            expected_tip_event_id: root.id.to_hex(),
            action: RevocationAction::RevokeGrant,
            operation_id: Uuid::new_v4().to_string(),
            confirm: true,
        },
        publisher_app.state(),
        publisher_app.state(),
    )
    .await
    .expect("developer revokes a grant issued without a campaign");
    let parsed = arcadestr_core::entitlements::parse_entitlement_event(&revocation)
        .expect("revocation parses");
    assert_eq!(
        parsed.status,
        arcadestr_core::entitlements::GrantStatus::Revoked
    );
    assert_eq!(parsed.source_event, None, "invariant absence is preserved");
    assert!(
        events
            .lock()
            .expect("relay")
            .iter()
            .any(|e| e.id == revocation.id),
        "revocation was published"
    );

    for app in &clients {
        let (report, _) =
            crate::entitlement_commands::sync_for_account(buyer.public_key(), &app.state())
                .await
                .expect("reconcile");
        assert_eq!(report.revoked, 1);
        let service = ownership(app);
        assert!(!service
            .is_owned(&buyer.public_key().to_hex(), &coordinate)
            .await
            .expect("ownership"));
        assert_eq!(
            service
                .durable_records_for(&buyer.public_key().to_hex())
                .await
                .expect("history")[0]
                .status,
            DurableCredentialStatus::Revoked
        );
    }

    // Only the developer may revoke: the buyer's own key is refused.
    switch(&publisher_app, &buyer.secret_key().to_secret_hex()).await;
    assert!(crate::publisher_management::revoke_publisher_credential(
        RevokePublisherCredentialRequest {
            expected_publisher_npub: buyer.public_key().to_bech32().expect("npub"),
            coordinate: coordinate.clone(),
            root_event_id: root.id.to_hex(),
            expected_tip_event_id: revocation.id.to_hex(),
            action: RevocationAction::RevokeGrant,
            operation_id: Uuid::new_v4().to_string(),
            confirm: true,
        },
        publisher_app.state(),
        publisher_app.state(),
    )
    .await
    .is_err());
}

/// ADP-01 §Relay selection (gap C5): a listing held only by the publisher's
/// NIP-65 write relay is found, because that relay is preferred for reads.
/// The outbox relay is a discovered relay, so it never counts toward the
/// checkout quorum. Loopback relays only.
#[tokio::test]
async fn publisher_nip65_write_relay_is_preferred_without_counting_as_evidence() {
    let temp = tempfile::tempdir().expect("isolated root");
    let configured: Events = Arc::new(Mutex::new(vec![]));
    let outbox: Events = Arc::new(Mutex::new(vec![]));
    let (configured_url, _configured) = relay(configured.clone()).await;
    let (outbox_url, _outbox) = relay(outbox.clone()).await;
    let http: Arc<dyn HttpClient> = Arc::new(IsolatedHttp {
        real: ReqwestHttpClient::new(Duration::from_secs(5)).expect("HTTP"),
        urls: vec![],
    });
    let publisher = Keys::generate();
    let coordinate = format!("30402:{}:outbox-game", publisher.public_key());
    let relay_list = EventBuilder::new(Kind::Custom(10002), "")
        .tags([tag(&["r", &outbox_url, "write"])])
        .sign_with_keys(&publisher)
        .expect("relay list");
    let listing = EventBuilder::new(Kind::Custom(30402), "")
        .tags([tag(&["d", "outbox-game"]), tag(&["title", "Outbox Game"])])
        .sign_with_keys(&publisher)
        .expect("listing");
    configured.lock().expect("configured").push(relay_list);
    outbox.lock().expect("outbox").push(listing.clone());

    let app = test_app_with_relay_discovery(
        vec![configured_url.clone()],
        temp.path().join("buyer.db"),
        http,
        true,
    )
    .await;
    let found = fetch_listing_event_by_coordinate(&app.state(), &coordinate)
        .await
        .expect("listing found on the publisher's write relay");
    assert_eq!(found.id, listing.id);

    let state = app.state::<AppState>();
    let manager = {
        let shared = { state.nostr.lock().await.get_relay_manager().clone() };
        let manager = shared.lock().await.clone();
        manager
    };
    let coverage = manager
        .fetch_relay_coverage(
            nostr::Filter::new()
                .kind(nostr::Kind::Custom(30402))
                .author(publisher.public_key())
                .identifier("outbox-game"),
            Duration::from_secs(5),
        )
        .await;
    assert_eq!(coverage.configured(), 1, "only the configured relay counts");
    assert!(
        coverage.partial_events.iter().any(|e| e.id == listing.id),
        "the outbox relay still contributes positive evidence"
    );
}
