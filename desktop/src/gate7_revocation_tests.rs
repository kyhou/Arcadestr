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
    let temp = tempfile::tempdir_in("/tmp/opencode").expect("isolated root");
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
