// Included only inside adp_commands::tests. No real relays, operators or wallets.
mod isolated_lifecycle {
    use super::*;
    include!("gate7_claim_tests.rs");
    include!("gate7_revocation_tests.rs");
    use arcadestr_core::{
        http_client::{HttpClientError, HttpDownloadOutcome},
        publisher_management::*,
        publisher_recovery::repository::PublisherRepository,
    };
    use axum::{
        body::Bytes,
        extract::{Request, State as AxumState},
        response::{IntoResponse, Response},
        Router,
    };
    use base64::{engine::general_purpose::STANDARD, Engine};
    use futures_util::{SinkExt, StreamExt};
    use nostr::{Event, EventBuilder, Filter, Kind, PublicKey, Tag, Timestamp};
    use sha2::Digest;
    use std::{
        path::Path,
        sync::{atomic::AtomicBool, Mutex},
    };
    use tokio_tungstenite::tungstenite::Message;
    fn digest(bytes: &[u8]) -> String {
        format!("{:x}", sha2::Sha256::digest(bytes))
    }

    type Events = Arc<Mutex<Vec<Event>>>;
    fn tag(values: &[&str]) -> Tag {
        Tag::parse(values.iter().copied()).expect("fixture tag")
    }
    struct FixtureTask(tokio::task::JoinHandle<()>);
    impl Drop for FixtureTask {
        fn drop(&mut self) {
            self.0.abort();
        }
    }

    async fn relay(events: Events) -> (String, FixtureTask) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("loopback relay");
        let url = format!("ws://{}", listener.local_addr().expect("relay address"));
        let task = tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                let events = events.clone();
                tokio::spawn(async move {
                    let Ok(mut ws) = tokio_tungstenite::accept_async(stream).await else {
                        return;
                    };
                    while let Some(Ok(message)) = ws.next().await {
                        let Message::Text(text) = message else {
                            if let Message::Ping(bytes) = message {
                                let _ = ws.send(Message::Pong(bytes)).await;
                            }
                            continue;
                        };
                        let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) else {
                            continue;
                        };
                        match value[0].as_str() {
                            Some("EVENT") => {
                                let event: Event = serde_json::from_value(value[1].clone())
                                    .expect("signed fixture publication");
                                event.verify().expect("valid publication");
                                {
                                    let mut store = events.lock().expect("event store");
                                    if !store.iter().any(|old| old.id == event.id) {
                                        store.push(event.clone());
                                    }
                                }
                                ws.send(Message::Text(
                                    json!(["OK", event.id, true, ""]).to_string().into(),
                                ))
                                .await
                                .expect("publication ack");
                            }
                            Some("REQ") => {
                                let subscription = value[1].clone();
                                let filters = value
                                    .as_array()
                                    .expect("request")
                                    .iter()
                                    .skip(2)
                                    .filter_map(|value| {
                                        serde_json::from_value::<Filter>(value.clone()).ok()
                                    })
                                    .collect::<Vec<_>>();
                                let snapshot = events.lock().expect("snapshot").clone();
                                for event in snapshot.iter().filter(|event| {
                                    filters
                                        .iter()
                                        .any(|filter| filter.match_event(event, Default::default()))
                                }) {
                                    if ws
                                        .send(Message::Text(
                                            json!(["EVENT", subscription, event])
                                                .to_string()
                                                .into(),
                                        ))
                                        .await
                                        .is_err()
                                    {
                                        return;
                                    }
                                }
                                if ws
                                    .send(Message::Text(
                                        json!(["EOSE", subscription]).to_string().into(),
                                    ))
                                    .await
                                    .is_err()
                                {
                                    return;
                                }
                            }
                            _ => {}
                        }
                    }
                });
            }
        });
        (url, FixtureTask(task))
    }

    struct Operator {
        url: String,
        operator: Keys,
        held_key: Keys,
        events: Events,
        terms: Event,
        provision_calls: AtomicU64,
        upload_calls: AtomicU64,
        fail_upload_once: AtomicBool,
        archive: Mutex<Vec<u8>>,
        pinned_events: Mutex<Vec<String>>,
    }
    fn authenticated(request: &Request, url: &str) -> Event {
        let value = request
            .headers()
            .get("authorization")
            .expect("NIP-98 header")
            .to_str()
            .expect("header text");
        let event: Event = serde_json::from_slice(
            &STANDARD
                .decode(value.strip_prefix("Nostr ").expect("NIP-98 scheme"))
                .expect("NIP-98 encoding"),
        )
        .expect("NIP-98 event");
        event.verify().expect("NIP-98 signature");
        assert_eq!(event.kind.as_u16(), 27235);
        assert!(event.tags.iter().any(|tag| tag.as_slice() == ["u", url]));
        assert!(event
            .tags
            .iter()
            .any(|tag| tag.as_slice() == ["method", request.method().as_str()]));
        event
    }
    fn field<'a>(body: &'a [u8], name: &str) -> &'a [u8] {
        let marker = format!("name=\"{name}\"");
        let start = body
            .windows(marker.len())
            .position(|bytes| bytes == marker.as_bytes())
            .expect("multipart field")
            + marker.len();
        let header_end = body[start..]
            .windows(4)
            .position(|bytes| bytes == b"\r\n\r\n")
            .expect("multipart headers")
            + start
            + 4;
        let end = body[header_end..]
            .windows(4)
            .position(|bytes| bytes == b"\r\n--")
            .expect("multipart boundary")
            + header_end;
        &body[header_end..end]
    }
    async fn operator_handler(
        AxumState(operator): AxumState<Arc<Operator>>,
        request: Request,
    ) -> Response {
        let path = request.uri().path().to_owned();
        let endpoint = format!("{}{}", operator.url, path);
        if path == "/.well-known/adp" {
            return axum::Json(json!({"adp_version":"0.2.0","pubkey":operator.operator.public_key(),"max_upload_bytes":1048576,"event_kinds":{"entitlement_grant":1030,"adp_campaign":1031,"commercial_terms":arcadestr_core::adp_settlement::TERMS_KIND,"private_quote":arcadestr_core::adp_settlement::QUOTE_KIND,"recovery_evidence":arcadestr_core::adp_settlement::RECOVERY_KIND},"settlement":{"version":1,"split_enabled":true,"terms_event_id":operator.terms.id,"fee":{"type":"none"},"legacy_paid_sales":"unsupported"}})).into_response();
        }
        let auth = authenticated(&request, &endpoint);
        if path == "/provision" {
            let body = axum::body::to_bytes(request.into_body(), 65536)
                .await
                .expect("provision request");
            let input: serde_json::Value = serde_json::from_slice(&body).expect("scope JSON");
            let scope = input["scope"].as_str().expect("pinned scope");
            operator.provision_calls.fetch_add(1, Ordering::SeqCst);
            let now = Timestamp::now().as_secs().saturating_sub(10);
            let attestation = EventBuilder::new(Kind::Custom(30404), "")
                .tags([
                    tag(&[
                        "d",
                        &format!("{}:{}", auth.pubkey, operator.held_key.public_key()),
                    ]),
                    tag(&["p", &auth.pubkey.to_hex()]),
                    tag(&["scope", scope]),
                    tag(&[
                        "fulfillment_pubkey",
                        &operator.held_key.public_key().to_hex(),
                        &now.to_string(),
                        "",
                    ]),
                ])
                .custom_created_at(Timestamp::from_secs(now))
                .sign_with_keys(&operator.operator)
                .expect("operator attestation");
            operator
                .events
                .lock()
                .expect("store attestation")
                .push(attestation.clone());
            return axum::Json(json!({"fulfillment_pubkey":operator.held_key.public_key(),"attestation_event_id":attestation.id,"attestation":attestation,"scope":scope})).into_response();
        }
        if path == "/upload" {
            let body = axum::body::to_bytes(request.into_body(), 1048576)
                .await
                .expect("leased multipart bytes");
            let listing: Event =
                serde_json::from_slice(field(&body, "listing_event")).expect("listing part");
            listing.verify().expect("signed listing");
            assert_eq!(listing.pubkey, auth.pubkey);
            let bytes = field(&body, "file");
            let digest = digest(bytes);
            let parsed =
                arcadestr_core::marketplace::parse_listing(listing.clone()).expect("listing");
            assert_eq!(parsed.file_hash.as_deref(), Some(digest.as_str()));
            *operator.archive.lock().expect("registered bytes") = bytes.to_vec();
            operator
                .pinned_events
                .lock()
                .expect("upload identity")
                .push(listing.id.to_hex());
            operator.upload_calls.fetch_add(1, Ordering::SeqCst);
            if operator.fail_upload_once.swap(false, Ordering::SeqCst) {
                return (
                    axum::http::StatusCode::SERVICE_UNAVAILABLE,
                    "fixture lost acknowledgement after registration",
                )
                    .into_response();
            }
            return axum::Json(json!({"game_coordinate":format!("30402:{}:{}",listing.pubkey,parsed.id),"file_hash":digest,"download_url":format!("{}/artifact",operator.url)})).into_response();
        }
        if path == "/entitlement/claim" {
            let body = axum::body::to_bytes(request.into_body(), 65536)
                .await
                .expect("claim body");
            let input: serde_json::Value = serde_json::from_slice(&body).expect("claim JSON");
            let coordinate = input["game_coordinate"].as_str().expect("coordinate");
            let events = operator.events.lock().expect("fixture events").clone();
            let root = events
                .iter()
                .find(|event| {
                    event.kind.as_u16() == 30406
                        && arcadestr_core::authorization::parse_authorization_event(event)
                            .is_ok_and(|auth| {
                                auth.terms.operator_pubkey == operator.operator.public_key()
                                    && auth.terms.coordinate == coordinate
                            })
                })
                .expect("held authority");
            let grant = EventBuilder::new(Kind::Custom(1030), "")
                .tags([
                    tag(&["d", "fixture-entitlement"]),
                    tag(&["p", &auth.pubkey.to_hex()]),
                    tag(&["a", coordinate]),
                    tag(&[
                        "source_event",
                        input["campaign_event_id"].as_str().expect("campaign"),
                    ]),
                    tag(&["authorization", &root.id.to_hex()]),
                    tag(&["status", "granted"]),
                ])
                .sign_with_keys(&operator.held_key)
                .expect("held-key grant");
            operator
                .events
                .lock()
                .expect("store public grant")
                .push(grant.clone());
            return axum::Json(json!({"grant":grant,"download_token":"synthetic-fixture-token","token_expires_at":Timestamp::now().as_secs()+600,"already_claimed":false})).into_response();
        }
        if path.starts_with("/game/") {
            return Bytes::from(operator.archive.lock().expect("registered bytes").clone())
                .into_response();
        }
        (
            axum::http::StatusCode::NOT_FOUND,
            "no fixture handler; no external forwarding",
        )
            .into_response()
    }
    async fn operator(events: Events, fail_upload: bool) -> (Arc<Operator>, FixtureTask) {
        use arcadestr_core::adp_settlement::*;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("loopback ADP");
        let url = format!(
            "http://{}",
            listener.local_addr().expect("operator address")
        );
        let keys = Keys::generate();
        let terms = build_terms_event_builder(&TermsPayload::Offer {
            terms: CommercialTerms {
                version: 1,
                operator_pubkey: keys.public_key().to_hex(),
                payment_destination: "operator@fixture.invalid".into(),
                fee: FeePolicy::None,
                limitations: vec!["Synthetic operator; not external acceptance".into()],
            },
        })
        .expect("terms")
        .custom_created_at(Timestamp::from_secs(Timestamp::now().as_secs() - 30))
        .sign_with_keys(&keys)
        .expect("signed terms");
        events.lock().expect("terms").push(terms.clone());
        let operator = Arc::new(Operator {
            url,
            operator: keys,
            held_key: Keys::generate(),
            events,
            terms,
            provision_calls: AtomicU64::new(0),
            upload_calls: AtomicU64::new(0),
            fail_upload_once: AtomicBool::new(fail_upload),
            archive: Mutex::new(Vec::new()),
            pinned_events: Mutex::new(Vec::new()),
        });
        let router = Router::new()
            .fallback(operator_handler)
            .with_state(operator.clone());
        let task = tokio::spawn(async move {
            axum::serve(listener, router)
                .await
                .expect("operator fixture");
        });
        (operator, FixtureTask(task))
    }

    struct IsolatedHttp {
        real: ReqwestHttpClient,
        urls: Vec<String>,
    }
    impl IsolatedHttp {
        fn local(&self, url: &str) -> Result<(), HttpClientError> {
            if self
                .urls
                .iter()
                .any(|base| url.starts_with(&format!("{base}/")))
            {
                Ok(())
            } else {
                Err(HttpClientError::Request("Non-fixture HTTP blocked".into()))
            }
        }
    }
    #[async_trait::async_trait]
    impl HttpClient for IsolatedHttp {
        async fn get_json(&self, url: &str) -> Result<serde_json::Value, HttpClientError> {
            if url == "https://publisher.invalid/.well-known/lnurlp/dev" {
                return Ok(
                    json!({"callback":"https://publisher.invalid/never-requested-pay","minSendable":1000,"maxSendable":10000000,"metadata":"[[\"text/plain\",\"isolated fixture\"]]","tag":"payRequest"}),
                );
            }
            self.local(url)?;
            self.real.get_json(url).await
        }
        async fn get_json_no_redirects(
            &self,
            url: &str,
        ) -> Result<serde_json::Value, HttpClientError> {
            self.get_json(url).await
        }
        async fn post_json(
            &self,
            url: &str,
            body: serde_json::Value,
            headers: Vec<(String, String)>,
        ) -> Result<serde_json::Value, HttpClientError> {
            self.local(url)?;
            self.real.post_json(url, body, headers).await
        }
        async fn post_multipart(
            &self,
            url: &str,
            auth: String,
            form: reqwest::multipart::Form,
        ) -> Result<serde_json::Value, HttpClientError> {
            self.local(url)?;
            self.real.post_multipart(url, auth, form).await
        }
        async fn download_to_path(
            &self,
            url: &str,
            headers: Vec<(String, String)>,
            dest: &Path,
            progress: &mut (dyn FnMut(u64, Option<u64>) + Send),
        ) -> Result<HttpDownloadOutcome, HttpClientError> {
            self.local(url)?;
            self.real
                .download_to_path(url, headers, dest, progress)
                .await
        }
    }
    async fn selected(
        app: &tauri::App<tauri::test::MockRuntime>,
        path: &Path,
        publisher: &str,
    ) -> BuildSelectionRef {
        use arcadestr_core::archive_inspection_contract::*;
        let state = app.state::<AppState>();
        let signer = app.state::<Arc<AsyncMutex<AppSignerState>>>();
        let service = app.state::<ArchiveSelectionService>();
        let selection = service
            .stage_source(
                path.to_owned(),
                publisher,
                crate::archive_commands::account_check(&state, &signer),
            )
            .await
            .expect("native selection");
        let id = || ArchiveRequestId::parse(&Uuid::new_v4().to_string()).expect("request UUID");
        service
            .inspect(
                InspectArchiveRequest {
                    request_id: id(),
                    selection_id: selection.selection_id.clone(),
                    expected_publisher_npub: publisher.into(),
                    expected_format: None,
                },
                publisher,
                crate::archive_commands::account_check(&state, &signer),
            )
            .await
            .expect("inspect archive");
        let hash = service
            .hash(
                HashSelectedArchiveRequest {
                    request_id: id(),
                    selection_id: selection.selection_id,
                    expected_publisher_npub: publisher.into(),
                },
                publisher,
                crate::archive_commands::account_check(&state, &signer),
            )
            .await
            .expect("hash archive");
        BuildSelectionRef {
            selection_id: hash.selection_id,
            sha256: hash.sha256,
        }
    }
    async fn switch(app: &tauri::App<tauri::test::MockRuntime>, secret: &str) -> PublicKey {
        let state = app.state::<AppState>();
        let public = {
            let mut auth = state.auth.lock().await;
            auth.connect_with_key(secret).expect("synthetic account");
            auth.public_key().expect("public key")
        };
        state.settlement_recovery.activate(Some(public.to_hex()));
        public
    }
    async fn after(event: &Event) {
        let now = Timestamp::now().as_secs();
        if now <= event.created_at.as_secs() {
            tokio::time::sleep(Duration::from_secs(event.created_at.as_secs() + 1 - now)).await;
        }
    }
    fn current(events: &Events, coordinate: &str) -> Event {
        let snapshot = events.lock().expect("events").clone();
        let parts = coordinate.splitn(3, ':').collect::<Vec<_>>();
        arcadestr_core::marketplace::current_listing_revision(
            &snapshot,
            PublicKey::from_hex(parts[1]).expect("publisher"),
            parts[2],
        )
        .expect("current listing")
    }

    #[tokio::test]
    async fn gate6_native_publisher_lifecycle_isolated_end_to_end() {
        let temp = tempfile::tempdir_in("/tmp/opencode").expect("isolated root");
        let db_path = temp.path().join("publisher.db");
        let events: Events = Arc::new(Mutex::new(Vec::new()));
        let (relay_one, _relay_one) = relay(events.clone()).await;
        let (relay_two, _relay_two) = relay(events.clone()).await;
        let relays = vec![relay_one, relay_two];
        let (one, _one) = operator(events.clone(), false).await;
        let (two, _two) = operator(events.clone(), true).await;
        let http: Arc<dyn HttpClient> = Arc::new(IsolatedHttp {
            real: ReqwestHttpClient::new(Duration::from_secs(5)).expect("HTTP"),
            urls: vec![one.url.clone(), two.url.clone()],
        });
        let app = test_app_with_http(relays.clone(), db_path.clone(), http.clone()).await;
        let developer =
            Keys::parse("0000000000000000000000000000000000000000000000000000000000000001")
                .expect("synthetic developer");
        let publisher = developer.public_key().to_bech32().expect("npub");
        let source = write_live_test_file().await;
        let archive = temp.path().join("fixture.zip");
        tokio::fs::copy(&source, &archive)
            .await
            .expect("fixture archive");
        tokio::fs::remove_file(source)
            .await
            .expect("remove owned source fixture");
        let bytes = tokio::fs::read(&archive).await.expect("archive bytes");
        let build = selected(&app, &archive, &publisher).await;
        let review = preview_publisher_terms(
            arcadestr_core::adp_settlement::PublisherTermsContext {
                publisher_pubkey: developer.public_key().to_hex(),
                servers: vec![one.url.clone(), two.url.clone()],
                price_sats: 1000,
                developer_destination: "dev@publisher.invalid".into(),
            },
            app.state(),
        )
        .await
        .expect("exact signed commercial review");
        assert_eq!(review.offers.len(), 2);
        assert_eq!(
            review.legs.iter().map(|leg| leg.amount_msat).sum::<u64>(),
            1000000
        );
        let operation_id = Uuid::new_v4().to_string();
        let request = PublishAdpListingRequest {
            expected_publisher_npub: publisher.clone(),
            existing_event_id: None,
            existing_d_tag: None,
            pending_d_tag: None,
            title: "Isolated lifecycle".into(),
            description: "Native command fixture".into(),
            price_sats: 1000,
            lud16: Some("dev@publisher.invalid".into()),
            tags: vec![],
            images: vec![],
            fulfillment_mode: FulfillmentMode::Delegate,
            operator_url: Some(one.url.clone()),
            servers: vec![one.url.clone(), two.url.clone()],
            accepted_terms: Vec::new(),
            file_path: None,
            selected_build: Some(build.clone()),
            existing_file_hash: None,
            existing_fulfillment_pubkey: None,
            version: Some("1.0".into()),
            acquisition: arcadestr_core::marketplace::AcquisitionPolicy::Gated,
            platforms: vec!["linux-x86_64".into()],
            campaigns: vec![],
            nip94_event_id: None,
        };
        let extension = PublishAdpListingRequestExtension {
            operation_id: Some(operation_id.clone()),
            commercial_intent:
                arcadestr_core::adp_settlement::PublisherCommercialIntent::ReplaceAfterReview {
                    review,
                },
            server_roles: vec![
                ServerRoleIntent {
                    server_url: one.url.clone(),
                    issue_receipt: true,
                    issue_grant: true,
                },
                ServerRoleIntent {
                    server_url: two.url.clone(),
                    issue_receipt: true,
                    issue_grant: true,
                },
            ],
            ..Default::default()
        };
        let mut malformed = request.clone();
        malformed.campaigns = vec![CampaignPointerInput {
            root_event_id: "not-an-event-id".into(),
            relay_hint: None,
        }];
        let mut malformed_extension = extension.clone();
        malformed_extension.operation_id = Some(Uuid::new_v4().to_string());
        assert!(
            publish_adp_listing_with_contract(
                malformed,
                malformed_extension,
                app.handle().clone(),
                app.state(),
                app.state(),
                app.state()
            )
            .await
            .is_err(),
            "malformed cached campaign pointer must stop before authorization/provisioning"
        );
        assert_eq!(one.provision_calls.load(Ordering::SeqCst), 0);
        assert_eq!(two.provision_calls.load(Ordering::SeqCst), 0);
        let result = publish_adp_listing_with_contract(
            request.clone(),
            extension,
            app.handle().clone(),
            app.state(),
            app.state(),
            app.state(),
        )
        .await;
        assert!(
            matches!(
                result,
                Err(PublishAdpListingError::PartialPublication { .. })
            ),
            "second operator interruption must remain partial: {result:?}"
        );
        let repo = PublisherRepository::new(app.state::<AppState>().database.pool().clone());
        let operation = repo
            .operation(&developer.public_key().to_hex(), &operation_id)
            .await
            .expect("durable operation");
        let coordinate = operation.coordinate.clone();
        let listing = current(&events, &coordinate);
        let parsed = arcadestr_core::marketplace::parse_listing(listing.clone()).expect("listing");
        assert_eq!(parsed.fulfillment_authorizations.len(), 2);
        assert_eq!(one.upload_calls.load(Ordering::SeqCst), 1);
        assert_eq!(two.upload_calls.load(Ordering::SeqCst), 1);
        assert_eq!(*one.archive.lock().expect("bytes"), bytes);
        assert_eq!(*two.archive.lock().expect("bytes"), bytes);
        let draft = arcadestr_core::publisher_recovery::PublisherDraft {
            draft_id: "listing:fixture".into(),
            kind: "listing".into(),
            payload: json!({"title":"Restored draft","campaigns":[]}),
            revision: 0,
        };
        crate::publisher_commands::save_publisher_draft(
            crate::publisher_commands::SavePublisherDraftRequest {
                expected_publisher_npub: publisher.clone(),
                draft,
                expected_revision: None,
            },
            app.state(),
        )
        .await
        .expect("durable draft");
        drop(repo);
        app.state::<AppState>().database.pool().close().await;
        drop(app);
        let app = test_app_with_http(relays, db_path, http).await;
        let restored =
            crate::publisher_commands::publisher_recovery_state(publisher.clone(), app.state())
                .await
                .expect("restart read-only state");
        assert_eq!(restored.drafts.len(), 1);
        assert!(restored
            .operations
            .iter()
            .any(|op| op.operation_id == operation_id && op.state == "incomplete"));
        assert_eq!(one.upload_calls.load(Ordering::SeqCst), 1);
        assert_eq!(two.upload_calls.load(Ordering::SeqCst), 1);
        let resume = |selected_build, confirm_uncertain_retry| {
            crate::publisher_commands::ResumePublisherOperationRequest {
                expected_publisher_npub: publisher.clone(),
                operation_id: operation_id.clone(),
                confirm_resume: true,
                confirm_uncertain_retry,
                selected_build,
            }
        };
        assert!(
            crate::publisher_commands::resume_publisher_operation(
                resume(Some(build), true),
                app.handle().clone(),
                app.state(),
                app.state(),
                app.state()
            )
            .await
            .is_err(),
            "expired handle must not survive restart"
        );
        let reselection = selected(&app, &archive, &publisher).await;
        assert!(
            crate::publisher_commands::resume_publisher_operation(
                resume(Some(reselection.clone()), false),
                app.handle().clone(),
                app.state(),
                app.state(),
                app.state()
            )
            .await
            .is_err(),
            "unknown upload outcome needs separate explicit acknowledgement"
        );
        crate::publisher_commands::resume_publisher_operation(
            resume(Some(reselection), true),
            app.handle().clone(),
            app.state(),
            app.state(),
            app.state(),
        )
        .await
        .expect("explicit same-artifact reconciliation");
        assert_eq!(one.provision_calls.load(Ordering::SeqCst), 1);
        assert_eq!(two.provision_calls.load(Ordering::SeqCst), 1);
        assert_eq!(one.upload_calls.load(Ordering::SeqCst), 1);
        assert_eq!(two.upload_calls.load(Ordering::SeqCst), 2);
        assert!(two
            .pinned_events
            .lock()
            .expect("exact retry")
            .iter()
            .all(|id| id == &listing.id.to_hex()));
        let evidence = crate::publisher_management::publisher_distribution_state(
            publisher.clone(),
            coordinate.clone(),
            app.state(),
        )
        .await
        .expect("distribution projection");
        assert_eq!(evidence.len(), 2);
        assert!(evidence
            .iter()
            .all(|row| row.state == DistributionState::RegistrationAcknowledged));
        let d = coordinate.splitn(3, ':').nth(2).expect("d").to_string();
        let page = crate::store_page_commands::publish_store_page(
            app.state(),
            app.state(),
            crate::store_page_commands::PublishStorePageRequest {
                operation_id: None,
                expected_publisher_npub: publisher.clone(),
                draft: arcadestr_core::store_page::StorePageDraft::new(
                    "fixture-page".into(),
                    vec![coordinate.clone()],
                ),
                listing_mutations: vec![crate::store_page_commands::StorePageListingMutation {
                    listing_coordinate: coordinate.clone(),
                    expected_event_id: listing.id.to_hex(),
                    action: crate::store_page_commands::ListingPointerMutation::Link,
                    relay_hint: None,
                    published_event_id: None,
                }],
            },
        )
        .await
        .expect("publish and associate Store Page");
        assert!(page.complete);
        let mut campaign = publish_campaign(
            PublishCampaignRequest {
                publisher_npub: publisher.clone(),
                listing_id: d.clone(),
                campaign_id: "upcoming".into(),
                starts_at: Some(Timestamp::now().as_secs() + 600),
                ends_at: Some(Timestamp::now().as_secs() + 3600),
                predecessor_event_id: None,
                cancel: false,
                update_listing_pointer: true,
            },
            app.state(),
            app.state(),
        )
        .await
        .expect("create campaign");
        if let Some(error) = &campaign.pointer_update_error {
            assert!(
                error.contains("clock"),
                "unexpected partial campaign result: {campaign:?}"
            );
            let repo = PublisherRepository::new(app.state::<AppState>().database.pool().clone());
            let pending = repo
                .operations(&developer.public_key().to_hex())
                .await
                .expect("campaign journal")
                .into_iter()
                .find(|op| op.kind == "campaign" && op.intent["campaign_id"] == "upcoming")
                .expect("pending campaign");
            after(&current(&events, &coordinate)).await;
            let result = crate::publisher_commands::resume_publisher_operation(
                crate::publisher_commands::ResumePublisherOperationRequest {
                    expected_publisher_npub: publisher.clone(),
                    operation_id: pending.operation_id,
                    confirm_resume: true,
                    confirm_uncertain_retry: false,
                    selected_build: None,
                },
                app.handle().clone(),
                app.state(),
                app.state(),
                app.state(),
            )
            .await
            .expect("explicit campaign-pointer reconciliation");
            assert_eq!(
                result["event_id"].as_str(),
                Some(campaign.event_id.as_str()),
                "retry does not create another campaign root"
            );
            assert!(
                result["pointer_update_error"].is_null(),
                "reconciled campaign: {result}"
            );
            campaign.pointer_update_error = None;
        }
        assert!(
            campaign.pointer_update_error.is_none(),
            "campaign pointer result: {campaign:?}"
        );
        let campaign_event = events
            .lock()
            .expect("events")
            .iter()
            .find(|event| event.id.to_hex() == campaign.event_id)
            .expect("campaign root")
            .clone();
        after(&campaign_event).await;
        let changed = publish_campaign(
            PublishCampaignRequest {
                publisher_npub: publisher.clone(),
                listing_id: d.clone(),
                campaign_id: "upcoming".into(),
                starts_at: Some(Timestamp::now().as_secs() + 700),
                ends_at: Some(Timestamp::now().as_secs() + 3700),
                predecessor_event_id: Some(campaign.event_id.clone()),
                cancel: false,
                update_listing_pointer: false,
            },
            app.state(),
            app.state(),
        )
        .await
        .expect("modify upcoming campaign");
        let before = current(&events, &coordinate);
        let projected = arcadestr_core::nostr::GameListing::from_listing(
            arcadestr_core::marketplace::parse_listing(before.clone()).expect("project"),
        );
        app.state::<AppState>()
            .marketplace_cache
            .upsert_listing(&projected, Some(&before.id.to_hex()))
            .await
            .expect("Browse projection");
        let cached = app
            .state::<AppState>()
            .marketplace_cache
            .load_listings(10, None, None)
            .await
            .expect("Browse restore");
        assert_eq!(cached[0].campaigns, projected.campaigns);
        let browse = crate::app_listing_from_cached_listing(cached[0].clone());
        let managed = arcadestr_app::models::GameListing::from_listing(
            arcadestr_core::marketplace::parse_listing(before.clone()).expect("managed projection"),
        );
        assert_eq!(
            browse.campaigns, managed.campaigns,
            "Browse and management retain the same associations"
        );
        assert_eq!(browse.event_id, managed.event_id);
        for prefix in ["fulfillment_authorization", "adp_terms"] {
            let references = |listing: &arcadestr_app::models::GameListing| {
                listing
                    .specs
                    .iter()
                    .filter(|(key, _)| key.starts_with(prefix))
                    .cloned()
                    .collect::<Vec<_>>()
            };
            assert_eq!(
                references(&browse),
                references(&managed),
                "cache must not substitute {prefix} evidence"
            );
        }
        after(&before).await;
        let mut edit = request;
        edit.existing_event_id = Some(before.id.to_hex());
        edit.existing_d_tag = Some(d.clone());
        edit.selected_build = None;
        edit.existing_file_hash = Some(digest(&bytes));
        edit.title = "Metadata updated from an older cache".into();
        edit.campaigns.clear();
        publish_adp_listing_with_contract(
            edit,
            PublishAdpListingRequestExtension::default(),
            app.handle().clone(),
            app.state(),
            app.state(),
            app.state(),
        )
        .await
        .expect("metadata edit retains verified associations");
        let after_edit = current(&events, &coordinate);
        for name in [
            "fulfillment_authorization",
            "adp_terms",
            "campaign",
            "presentation",
        ] {
            assert_eq!(
                tag_values(&before, name),
                tag_values(&after_edit, name),
                "metadata edit lost {name}"
            );
        }
        after(&after_edit).await;
        let claim_start = Timestamp::now().as_secs() + 5;
        let active = publish_campaign(
            PublishCampaignRequest {
                publisher_npub: publisher.clone(),
                listing_id: d.clone(),
                campaign_id: "claim-now".into(),
                starts_at: Some(claim_start),
                ends_at: Some(claim_start + 3600),
                predecessor_event_id: None,
                cancel: false,
                update_listing_pointer: true,
            },
            app.state(),
            app.state(),
        )
        .await
        .expect("active claim campaign");
        assert!(
            active.pointer_update_error.is_none(),
            "active pointer result: {active:?}"
        );
        let now = Timestamp::now().as_secs();
        if now < claim_start {
            tokio::time::sleep(Duration::from_secs(claim_start - now)).await;
        }
        let buyer = switch(
            &app,
            "0000000000000000000000000000000000000000000000000000000000000002",
        )
        .await;
        let claimed = claim_entitlement(
            ClaimEntitlementRequest {
                expected_account_pubkey: buyer.to_hex(),
                publisher_npub: publisher.clone(),
                listing_id: d.clone(),
                server_url: one.url.clone(),
                campaign_event_id: active.root_event_id.clone(),
            },
            app.state(),
        )
        .await
        .expect("native claim verifies held-key grant");
        let destination = temp.path().join("download.zip");
        let buyer_signer = LocalSigner::from_hex(
            "0000000000000000000000000000000000000000000000000000000000000002",
        )
        .expect("fixture buyer");
        AdpClient::new(one.url.clone(), app.state::<AppState>().http_client.clone())
            .download(
                &coordinate,
                arcadestr_core::adp_client::DownloadAuth::Nip98 {
                    signer: &buyer_signer,
                },
                &destination,
                |_, _| {},
            )
            .await
            .expect("existing authenticated download path");
        assert_eq!(
            tokio::fs::read(destination).await.expect("download bytes"),
            bytes
        );
        assert!(
            crate::publisher_management::review_publisher_credential(
                publisher.clone(),
                coordinate.clone(),
                parsed.fulfillment_authorizations[0].root_event_id.to_hex(),
                RevocationAction::CancelAuthority,
                app.state()
            )
            .await
            .is_err(),
            "buyer cannot manage developer authority"
        );
        switch(
            &app,
            "0000000000000000000000000000000000000000000000000000000000000001",
        )
        .await;
        let changed_event = events
            .lock()
            .expect("events")
            .iter()
            .find(|event| event.id.to_hex() == changed.event_id)
            .expect("changed event")
            .clone();
        after(&changed_event).await;
        publish_campaign(
            PublishCampaignRequest {
                publisher_npub: publisher.clone(),
                listing_id: d.clone(),
                campaign_id: "upcoming".into(),
                starts_at: None,
                ends_at: None,
                predecessor_event_id: Some(changed.event_id),
                cancel: true,
                update_listing_pointer: false,
            },
            app.state(),
            app.state(),
        )
        .await
        .expect("prospective cancellation");
        after(&claimed.grant).await;
        let revoke = |action, root: String| RevokePublisherCredentialRequest {
            expected_publisher_npub: publisher.clone(),
            coordinate: coordinate.clone(),
            root_event_id: root.clone(),
            expected_tip_event_id: root,
            action,
            operation_id: Uuid::new_v4().to_string(),
            confirm: true,
        };
        let revoked = crate::publisher_management::revoke_publisher_credential(
            revoke(RevocationAction::RevokeGrant, claimed.grant.id.to_hex()),
            app.state(),
            app.state(),
        )
        .await
        .expect("developer-only grant revocation");
        assert_eq!(
            arcadestr_core::entitlements::parse_entitlement_event(&revoked)
                .expect("revocation")
                .status,
            arcadestr_core::entitlements::GrantStatus::Revoked
        );
        let entitlement_repo = arcadestr_core::entitlements_repository::EntitlementsRepository::new(
            app.state::<AppState>().database.pool().clone(),
        );
        assert!(!entitlement_repo
            .is_owned(&buyer.to_hex(), &coordinate)
            .await
            .expect("only this grant revoked"));
        let root = parsed
            .fulfillment_authorizations
            .iter()
            .find(|reference| {
                events.lock().expect("events").iter().any(|event| {
                    event.id == reference.root_event_id
                        && arcadestr_core::authorization::parse_authorization_event(event)
                            .is_ok_and(|auth| {
                                auth.terms.operator_pubkey == one.operator.public_key()
                            })
                })
            })
            .expect("first operator root")
            .root_event_id;
        let cancelled = crate::publisher_management::revoke_publisher_credential(
            revoke(RevocationAction::CancelAuthority, root.to_hex()),
            app.state(),
            app.state(),
        )
        .await
        .expect("explicit authority cancellation, not offboarding");
        let resolved = arcadestr_core::authorization::resolve_authorization(
            root,
            &events.lock().expect("events").clone(),
        )
        .expect("chain");
        assert!(resolved.authorizes(
            &one.held_key.public_key(),
            &coordinate,
            CAPABILITY_ISSUE_GRANT,
            claimed.grant.created_at.as_secs()
        ));
        assert!(!resolved.authorizes(
            &one.held_key.public_key(),
            &coordinate,
            CAPABILITY_ISSUE_RECEIPT,
            cancelled.created_at.as_secs()
        ));
        let state = app.state::<AppState>();
        let fence = state.settlement_recovery.session_fence().expect("session");
        switch(
            &app,
            "0000000000000000000000000000000000000000000000000000000000000002",
        )
        .await;
        switch(
            &app,
            "0000000000000000000000000000000000000000000000000000000000000001",
        )
        .await;
        assert!(
            fence.ensure_current().is_err(),
            "A-B-A cannot revive an asynchronous publisher session"
        );
        println!("PASS isolated native lifecycle: two relays, two operators, leased upload bytes, exact revision, restart/reselection, Store Page, cache, metadata preservation, campaigns, claims, download, revocation and account fencing; no live services or payments");
    }
}
