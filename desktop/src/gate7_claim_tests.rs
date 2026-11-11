// Reuse the isolated lifecycle's local relays and disposable native command app.
struct ClaimHttp {
    response: serde_json::Value,
    entered: Arc<tokio::sync::Notify>,
    release: Arc<tokio::sync::Notify>,
    calls: AtomicU64,
}

#[async_trait::async_trait]
impl HttpClient for ClaimHttp {
    async fn get_json(&self, _: &str) -> Result<serde_json::Value, HttpClientError> {
        panic!("claim fixture must not access external metadata")
    }
    async fn get_json_no_redirects(&self, _: &str) -> Result<serde_json::Value, HttpClientError> {
        panic!("claim fixture must not access external metadata")
    }
    async fn post_json(
        &self,
        url: &str,
        _: serde_json::Value,
        _: Vec<(String, String)>,
    ) -> Result<serde_json::Value, HttpClientError> {
        assert_eq!(url, "https://claim.example.test/entitlement/claim");
        self.calls.fetch_add(1, AtomicOrdering::SeqCst);
        self.entered.notify_one();
        self.release.notified().await;
        Ok(self.response.clone())
    }
    async fn download_to_path(
        &self,
        _: &str,
        _: Vec<(String, String)>,
        _: &Path,
        _: &mut (dyn FnMut(u64, Option<u64>) + Send),
    ) -> Result<HttpDownloadOutcome, HttpClientError> {
        panic!("claim fixture must not download")
    }
}

async fn claim_response_case(transition: &str) {
    let temp = tempfile::tempdir_in("/tmp/opencode").expect("isolated claim root");
    let developer = Keys::generate();
    let buyer = Keys::parse("0000000000000000000000000000000000000000000000000000000000000001")
        .expect("synthetic buyer");
    let coordinate = format!("30402:{}:claim-game", developer.public_key());
    let now = Timestamp::now().as_secs();
    let campaign = EventBuilder::new(Kind::Custom(1031), "")
        .tags([
            tag(&["d", "claim-campaign"]),
            tag(&["a", &coordinate]),
            tag(&["mode", "claim"]),
            tag(&["status", "active"]),
            tag(&["starts", &(now - 10).to_string()]),
            tag(&["ends", &(now + 3600).to_string()]),
        ])
        .custom_created_at(Timestamp::from_secs(now - 20))
        .sign_with_keys(&developer)
        .expect("campaign");
    let listing = EventBuilder::new(Kind::Custom(30402), "")
        .tags([
            tag(&["d", "claim-game"]),
            tag(&["title", "Isolated claim fixture"]),
            tag(&["server", "https://claim.example.test"]),
            tag(&["campaign", &campaign.id.to_hex()]),
        ])
        .sign_with_keys(&developer)
        .expect("listing");
    let grant = EventBuilder::new(Kind::Custom(1030), "")
        .tags([
            tag(&["d", "claimed-grant"]),
            tag(&["p", &buyer.public_key().to_hex()]),
            tag(&["a", &coordinate]),
            tag(&["source_event", &campaign.id.to_hex()]),
            tag(&["status", "granted"]),
        ])
        .sign_with_keys(&developer)
        .expect("valid grant");
    let events = Arc::new(Mutex::new(vec![listing, campaign.clone()]));
    let (one, _one) = relay(events.clone()).await;
    let (two, _two) = relay(events).await;
    let http = Arc::new(ClaimHttp {
        response: json!({"grant":grant,"download_token":"claim-token","token_expires_at":now+600}),
        entered: Arc::new(Default::default()),
        release: Arc::new(Default::default()),
        calls: AtomicU64::new(0),
    });
    let app = test_app_with_http(vec![one, two], temp.path().join("claim.db"), http.clone()).await;
    let mut request = ClaimEntitlementRequest {
        expected_account_pubkey: buyer.public_key().to_hex(),
        publisher_npub: developer.public_key().to_bech32().expect("npub"),
        listing_id: "claim-game".into(),
        campaign_event_id: campaign.id.to_hex(),
        server_url: "https://claim.example.test".into(),
    };
    if transition == "wrong-account" {
        request.expected_account_pubkey = developer.public_key().to_hex();
        claim_entitlement(request, app.state())
            .await
            .expect_err("queued claim cannot bind another account");
        assert_eq!(http.calls.load(AtomicOrdering::SeqCst), 0);
        return;
    }
    let handle = app.handle().clone();
    let mut task = tokio::spawn(async move { claim_entitlement(request, handle.state()).await });
    tokio::select! {
        _ = http.entered.notified() => {},
        result = &mut task => panic!("claim stopped before fixture: {result:?}"),
        _ = tokio::time::sleep(Duration::from_secs(10)) => panic!("claim did not reach fixture"),
    }
    match transition {
        "switch" | "roundtrip" => {
            switch(
                &app,
                "0000000000000000000000000000000000000000000000000000000000000002",
            )
            .await;
            if transition == "roundtrip" {
                switch(
                    &app,
                    "0000000000000000000000000000000000000000000000000000000000000001",
                )
                .await;
            }
        }
        "logout" => {
            let state = app.state::<AppState>();
            state.auth.lock().await.disconnect();
            state.settlement_recovery.activate(None);
        }
        "reconnect" => {
            app.state::<AppState>()
                .settlement_recovery
                .activate(Some(buyer.public_key().to_hex()));
        }
        "current" => {}
        _ => panic!("unknown fixture transition"),
    }
    http.release.notify_one();
    let result = task.await.expect("claim task");
    let state = app.state::<AppState>();
    let owned = arcadestr_core::entitlements_repository::EntitlementsRepository::new(
        state.database.pool().clone(),
    )
    .is_owned(&buyer.public_key().to_hex(), &coordinate)
    .await
    .expect("ownership");
    let token = DownloadTokensRepository::new(state.database.pool().clone())
        .valid_token(
            &buyer.public_key().to_hex(),
            &coordinate,
            "https://claim.example.test",
            now as i64,
        )
        .await
        .expect("tokens");
    assert_eq!(
        http.calls.load(AtomicOrdering::SeqCst),
        1,
        "no automatic retry"
    );
    if transition == "current" {
        assert!(result.is_ok(), "valid current claim: {result:?}");
        assert!(owned);
        assert_eq!(token.expect("current token").token, "claim-token");
    } else {
        assert!(
            result.is_err(),
            "{transition} must not revive a stale claim response"
        );
        assert!(!owned);
        assert!(token.is_none());
        // The remote side may already have issued the grant. A fresh explicit
        // same-campaign request can recover that evidence, never an automatic replay.
        switch(
            &app,
            "0000000000000000000000000000000000000000000000000000000000000001",
        )
        .await;
        http.release.notify_one();
        claim_entitlement(
            ClaimEntitlementRequest {
                expected_account_pubkey: buyer.public_key().to_hex(),
                publisher_npub: developer.public_key().to_bech32().expect("npub"),
                listing_id: "claim-game".into(),
                campaign_event_id: campaign.id.to_hex(),
                server_url: "https://claim.example.test".into(),
            },
            app.state(),
        )
        .await
        .expect("fresh explicit request recovers original grant");
    }
}

#[tokio::test]
async fn gate7_claim_response_cannot_revive_an_old_account_session() {
    claim_response_case("roundtrip").await;
}

#[tokio::test]
async fn gate7_claim_logout_switch_and_reconnect_reject_stale_results_but_current_claims_work() {
    for transition in ["logout", "switch", "reconnect", "current", "wrong-account"] {
        claim_response_case(transition).await;
    }
}

struct DelayedClaimKeySigner {
    keys: Keys,
    entered: tokio::sync::Notify,
    release: tokio::sync::Notify,
}

#[async_trait::async_trait]
impl NostrSigner for DelayedClaimKeySigner {
    async fn get_public_key(&self) -> Result<PublicKey, SignerError> {
        self.entered.notify_one();
        self.release.notified().await;
        Ok(self.keys.public_key())
    }
    async fn sign_event(&self, _: nostr::UnsignedEvent) -> Result<Event, SignerError> {
        panic!("stale key lookup must not reach signing")
    }
}

#[tokio::test]
async fn gate7_claim_signer_never_dispatches_after_delayed_identity_or_signature() {
    use arcadestr_core::settlement_recovery::RecoveryCoordinator;
    for transition in ["logout", "switch", "roundtrip", "reconnect"] {
        for phase in ["identity", "signature"] {
            let keys = Keys::generate();
            let account = keys.public_key().to_hex();
            let coordinator = RecoveryCoordinator::default();
            coordinator.activate(Some(account.clone()));
            let session = coordinator.session_fence().expect("initial session");
            let http = Arc::new(ClaimHttp {
                response: json!({}),
                entered: Arc::new(Default::default()),
                release: Arc::new(Default::default()),
                calls: AtomicU64::new(0),
            });
            http.release.notify_one();
            let delayed_key = Arc::new(DelayedClaimKeySigner {
                keys: keys.clone(),
                entered: Default::default(),
                release: Default::default(),
            });
            let delayed_signature = Arc::new(DelayedPublicationSigner {
                keys,
                started: Default::default(),
                release: Default::default(),
                reject: false,
            });
            let signer: Arc<dyn NostrSigner> = if phase == "identity" {
                delayed_key.clone()
            } else {
                delayed_signature.clone()
            };
            let task_http = http.clone();
            let task = tokio::spawn(async move {
                let fenced = ClaimSigner {
                    signer: signer.as_ref(),
                    session,
                };
                AdpClient::new("https://claim.example.test", task_http)
                    .entitlement_claim(
                        &fenced,
                        CoreEntitlementClaimRequest {
                            game_coordinate: "30402:fixture:game".into(),
                            campaign_event_id: "11".repeat(32),
                        },
                    )
                    .await
            });
            let entered = if phase == "identity" {
                &delayed_key.entered
            } else {
                &delayed_signature.started
            };
            tokio::time::timeout(Duration::from_secs(5), entered.notified())
                .await
                .expect("delayed signer entered");
            match transition {
                "logout" => coordinator.activate(None),
                "switch" => coordinator.activate(Some("other-account".into())),
                "roundtrip" => {
                    coordinator.activate(Some("other-account".into()));
                    coordinator.activate(Some(account));
                }
                "reconnect" => coordinator.activate(Some(account)),
                _ => unreachable!(),
            }
            if phase == "identity" {
                delayed_key.release.notify_one();
            } else {
                delayed_signature.release.notify_one();
            }
            assert!(
                task.await.expect("claim task").is_err(),
                "{phase}/{transition}"
            );
            assert_eq!(
                http.calls.load(AtomicOrdering::SeqCst),
                0,
                "no stale NIP-98 dispatch"
            );
        }
    }
}
