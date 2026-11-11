use super::fixtures::*;
use arcadestr_core::{
    adp_client::{AdpClient, CreateSettlementQuoteRequest, DownloadAuth, SettlementProofRequest},
    adp_download::{download_candidates, download_verified},
    adp_publish::{
        build_adp_listing_event_builder, build_fulfillment_authorization_event_builder,
        AdpListingInput, FulfillmentAuthorizationInput,
    },
    adp_storage::{
        InstallationId, InstallationRecord, InstallationState, InstallationsRepository,
        RetainedPackage, RetainedPackageId, RetainedPackageReinstallSnapshot,
    },
    authorization::{AuthorizationTerms, CAPABILITY_ISSUE_RECEIPT, CAPABILITY_UPLOAD_BUILD},
    distribution::{host_platform, PackageFormat, Sha256Digest},
    entitlements_repository::EntitlementsRepository,
    marketplace::AcquisitionPolicy,
    nostr::NostrClient,
    ownership::OwnershipService,
    package_extraction::extract_verified_package,
    purchases::PurchasesRepository,
    receipt_portability::{ReceiptPortability, ReceiptSource},
    receipt_sync_contract::ReceiptVerification,
    relay_manager::{RelayManager, RelayManagerConfig},
    signers::NostrSigner,
    storage::Database,
};
use nostr_sdk::{Event, EventBuilder, Filter, Keys, PublicKey, Timestamp};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{
    io::{Cursor, Write},
    sync::{Arc, Mutex},
};
use wiremock::{
    matchers::{method, path},
    Mock, MockServer, ResponseTemplate,
};

struct ReceiptRelays {
    manager: RelayManager,
    urls: Vec<String>,
}
#[async_trait::async_trait]
impl ReceiptSource for ReceiptRelays {
    fn relay_urls(&self) -> Vec<String> {
        self.urls.clone()
    }
    async fn query(&self, relay: &str, filter: Filter) -> Result<Vec<Event>, String> {
        self.manager
            .fetch_events_from_subset(filter, vec![relay.into()])
            .await
            .map_err(|e| e.to_string())
    }
}
fn zip_bytes(contents: &str) -> Vec<u8> {
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    zip.start_file("game.txt", zip::write::SimpleFileOptions::default())
        .expect("ZIP entry");
    zip.write_all(contents.as_bytes()).expect("ZIP contents");
    zip.start_file(
        "game.sh",
        zip::write::SimpleFileOptions::default().unix_permissions(0o755),
    )
    .expect("game executable");
    zip.write_all(format!("#!/bin/sh\n# {contents}\nexit 0\n").as_bytes())
        .expect("harmless local launch fixture");
    zip.finish().expect("ZIP").into_inner()
}

#[tokio::test]
async fn aggregate_paid_receipt_discovery_portability_actual_server_failover_and_install_update() {
    let events: Events = Arc::new(Mutex::new(vec![]));
    let (one, _one) = relay(events.clone()).await;
    let (two, _two) = relay(events.clone()).await;
    let relays = vec![one, two];
    let a = Server::start(&relays).await;
    let b = Server::start(&relays).await;
    let developer = Keys::generate();
    let buyer = Keys::generate();
    let dev_signer = signer(&developer);
    let buyer_signer = signer(&buyer);
    let client_a = AdpClient::new(&a.url, http());
    let client_b = AdpClient::new(&b.url, http());
    let pa = client_a
        .provision(&dev_signer, None)
        .await
        .expect("provision A");
    let pb = client_b
        .provision(&dev_signer, None)
        .await
        .expect("provision B");
    let coordinate = format!("30402:{}:paid-game", developer.public_key());
    let now = Timestamp::now().as_secs();
    let mut authorities = vec![];
    let mut references = vec![];
    for (index, (server, provision)) in [(&a, pa), (&b, pb)].into_iter().enumerate() {
        let root = build_fulfillment_authorization_event_builder(&AuthorizationTerms {
            authorization_id: format!("paid-authority-{index}"),
            coordinate: coordinate.clone(),
            operator_pubkey: server.operator.public_key(),
            fulfillment_pubkey: PublicKey::from_hex(&provision.fulfillment_pubkey)
                .expect("held signer"),
            capabilities: std::collections::BTreeSet::from([
                CAPABILITY_ISSUE_RECEIPT.into(),
                CAPABILITY_UPLOAD_BUILD.into(),
            ]),
            valid_from: now - 30,
        })
        .expect("authority")
        .sign_with_keys(&developer)
        .expect("developer signature");
        references.push(FulfillmentAuthorizationInput {
            root_event_id: root.id.to_hex(),
            fulfillment_pubkey: provision.fulfillment_pubkey,
            relay_hint: None,
        });
        authorities.push(root);
    }
    let metadata = MockServer::start().await;
    let mut offers = vec![];
    for (name, amount, seed, recipient) in [
        ("developer", 800000, 50u8, developer.public_key()),
        ("operator-a", 100000, 51u8, a.operator.public_key()),
        ("operator-b", 100000, 52u8, b.operator.public_key()),
    ] {
        let address = format!("{name}@{}", metadata.address());
        Mock::given(method("GET")).and(path(format!("/.well-known/lnurlp/{name}")))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"callback":format!("{}/{name}/synthetic-invoice",metadata.uri()),"minSendable":1,"maxSendable":1000000000,"nostrPubkey":recipient.to_hex(),"allowsNostr":false}))).mount(&metadata).await;
        Mock::given(method("GET"))
            .and(path(format!("/{name}/synthetic-invoice")))
            .respond_with(ResponseTemplate::new(200).set_body_json(
                json!({"pr":super::quote_response::synthetic_invoice(amount,seed),"routes":[]}),
            ))
            .expect(1)
            .mount(&metadata)
            .await;
        if name != "developer" {
            let operator = if name == "operator-a" {
                &a.operator
            } else {
                &b.operator
            };
            offers.push(
                adp_core::build_terms_event_builder(&adp_core::TermsPayload::Offer {
                    terms: adp_core::CommercialTerms {
                        version: 1,
                        operator_pubkey: operator.public_key().to_hex(),
                        payment_destination: address,
                        fee: adp_core::FeePolicy::Fixed {
                            amount_msat: 100000,
                        },
                        limitations: vec![],
                    },
                })
                .expect("terms")
                .sign_with_keys(operator)
                .expect("signed offer"),
            );
        }
    }
    let bytes = zip_bytes("version one");
    let digest = Sha256Digest::parse(&hex::encode(Sha256::digest(&bytes))).expect("digest");
    let profiles = vec![arcadestr_core::distribution::LaunchProfile {
        id: arcadestr_core::distribution::ProfileId::parse("play").expect("profile id"),
        display_name: "Play fixture".into(),
        executable: arcadestr_core::distribution::RelativePath::parse("game.sh")
            .expect("executable"),
        working_dir: arcadestr_core::distribution::WorkingDir::parse(".")
            .expect("working directory"),
        arguments: vec![],
        launch_type: arcadestr_core::distribution::LaunchType::Game,
    }];
    let input = AdpListingInput {
        d_tag: "paid-game".into(),
        title: "Paid integration fixture".into(),
        description: "No live Lightning node".into(),
        price_sats: 1000,
        lud16: Some(format!("developer@{}", metadata.address())),
        tags: vec!["game".into()],
        images: vec![],
        servers: vec![a.url.clone(), b.url.clone()],
        file_hash: Some(hex::encode(Sha256::digest(&bytes))),
        version: Some("1".into()),
        fulfillment_authorizations: references,
        acquisition: AcquisitionPolicy::Gated,
        platforms: vec![],
        campaigns: vec![],
        nip94_event_id: None,
        preserved_tags: vec![],
    };
    let tier = arcadestr_core::adp_publish_contract::AdpLaunchTierIntent {
        package_format: PackageFormat::Zip,
        platforms: vec![host_platform()],
        launch_profiles: profiles.clone(),
        publisher_defaults: vec![
            arcadestr_core::adp_publish_contract::PublisherLaunchDefault {
                profile_id: profiles[0].id.clone(),
                targets: vec![host_platform()],
            },
        ],
    };
    let sale =
        build_adp_listing_event_builder(arcadestr_core::adp_publish::AdpListingPublishInput {
            listing: &input,
            launch_tier: Some(&tier),
        })
        .expect("listing")
        .tags(
            offers
                .iter()
                .map(|offer| tag(&["adp_terms", &offer.pubkey.to_hex(), &offer.id.to_hex()])),
        )
        .sign_with_keys(&developer)
        .expect("listing signature");
    let publisher = nostr_sdk::Client::new(developer.clone());
    for url in &relays {
        publisher.add_relay(url).await.expect("relay");
    }
    publisher.connect().await;
    for event in authorities.iter().chain(offers.iter()).chain([&sale]) {
        publisher
            .send_event(event)
            .await
            .expect("local publication");
    }
    let dir = tempfile::tempdir_in("/tmp/opencode").expect("buyer isolation");
    let archive = dir.path().join("build.zip");
    tokio::fs::write(&archive, &bytes).await.expect("artifact");
    client_a
        .upload(&dev_signer, &sale, &archive)
        .await
        .expect("actual upload A");
    client_b
        .upload(&dev_signer, &sale, &archive)
        .await
        .expect("actual upload B");
    let quote = client_a
        .create_settlement_quote(
            &buyer_signer,
            CreateSettlementQuoteRequest {
                purchase_id: "synthetic-multi-operator".into(),
                listing_event: sale.clone(),
                terms_events: offers.clone(),
                refund_destination: arcadestr_core::adp_settlement::RefundDestination {
                    endpoint: "buyer@refund.example.test".into(),
                    min_sendable_msat: 1,
                    max_sendable_msat: 1000000000,
                },
                zap_requests: Default::default(),
            },
        )
        .await
        .expect("actual signed private quote");
    let plaintext = buyer_signer
        .nip44_decrypt(&quote.quote.pubkey, &quote.quote.content)
        .await
        .expect("buyer decrypts quote");
    let verified = arcadestr_core::adp_settlement::verify_quote_event(&quote.quote, &plaintext)
        .expect("quote signature/commitments");
    arcadestr_core::adp_settlement::verify_quote_listing_terms(&verified, &sale, &offers)
        .expect("immutable developer terms");
    assert_eq!(verified.legs.len(), 3);
    let mut completion = None;
    let mut last_attempt = None;
    for _ in 0..3 {
        let invoice = client_a
            .settlement_next_invoice(&buyer_signer, &verified.quote_id)
            .await
            .expect("original synthetic attempt");
        let repeated = client_a
            .settlement_next_invoice(&buyer_signer, &verified.quote_id)
            .await
            .expect("idempotent disclosure");
        assert_eq!(invoice.attempt_id, repeated.attempt_id);
        let seed = (50u8..=52)
            .find(|seed| hex::encode(Sha256::digest([*seed; 32])) == invoice.payment_hash)
            .expect("offline fixture proof");
        let request = SettlementProofRequest {
            attempt_id: invoice.attempt_id,
            preimage: hex::encode([seed; 32]),
        };
        let result = client_a
            .submit_settlement_proof(&buyer_signer, &verified.quote_id, request.clone())
            .await
            .expect("actual preimage verification, no wallet dispatch");
        if result.receipt.is_some() {
            completion = Some(result);
            last_attempt = Some(request);
        }
    }
    let complete = completion.expect("aggregate receipt");
    complete
        .validate_completion_fields()
        .expect("complete response");
    let receipt = complete.receipt.expect("signed receipt");
    let parsed = arcadestr_core::purchases::parse_receipt_event(&receipt)
        .expect("Arcadestr aggregate parser");
    assert_eq!(
        parsed
            .aggregate
            .as_ref()
            .expect("aggregate profile")
            .legs
            .len(),
        3
    );
    let replay = client_a
        .submit_settlement_proof(
            &buyer_signer,
            &verified.quote_id,
            last_attempt.expect("original proof"),
        )
        .await
        .expect("idempotent settlement recovery");
    assert_eq!(replay.receipt.expect("same receipt").id, receipt.id);
    // Legacy paid proof compatibility is verified independently, with a buyer
    // that has no aggregate receipt or grant to mask a failed authorization.
    let legacy_buyer = Keys::generate();
    let legacy_signer = signer(&legacy_buyer);
    let legacy_invoice = super::quote_response::synthetic_invoice(1000000, 90);
    let legacy_request = |preimage: String| arcadestr_core::adp_client::PurchaseConfirmRequest {
        game_coordinate: coordinate.clone(),
        listing_event: sale.clone(),
        zap_receipt_event: None,
        bolt11: Some(legacy_invoice.clone()),
        preimage: Some(preimage),
        quote_id: None,
    };
    assert!(
        client_a
            .purchase_confirm(&legacy_signer, legacy_request("ff".repeat(32)))
            .await
            .is_err(),
        "invalid payment proof never creates paid ownership"
    );
    assert!(client_a
        .download(
            &coordinate,
            DownloadAuth::Nip98 {
                signer: &legacy_signer
            },
            &dir.path().join("unpaid.zip"),
            |_, _| {}
        )
        .await
        .is_err());
    let legacy = client_a
        .purchase_confirm(&legacy_signer, legacy_request(hex::encode([90; 32])))
        .await
        .expect("actual legacy BOLT11/preimage verification");
    assert!(
        client_a
            .purchase_confirm(&legacy_signer, legacy_request(hex::encode([90; 32])))
            .await
            .is_err(),
        "legacy one-time proof fails closed on replay rather than creating another receipt"
    );
    assert_eq!(events.lock().expect("local relay evidence").iter().filter(|event|event.kind.as_u16()==1020 && event.tags.iter().any(|tag|matches!(tag.as_slice(),[name,value] if name=="p" && value==&legacy_buyer.public_key().to_hex()))).count(),1,"original legacy receipt remains recoverable on the local relays");
    assert!(
        client_a
            .purchase_confirm(
                &signer(&Keys::generate()),
                legacy_request(hex::encode([90; 32]))
            )
            .await
            .is_err(),
        "settlement proof cannot be reused by another buyer"
    );
    let legacy_db = Database::new(&dir.path().join("legacy-portable-buyer.db"))
        .await
        .expect("independent legacy buyer");
    let legacy_import = ReceiptPortability::new(legacy_db.pool().clone())
        .import_json(
            legacy_buyer.public_key(),
            &serde_json::to_string(&vec![legacy.receipt.clone(), authorities[0].clone()])
                .expect("legacy JSON"),
            None,
            &|| Ok(()),
        )
        .await
        .expect("legacy portability");
    assert!(legacy_import
        .chains
        .iter()
        .any(|chain| chain.verification == ReceiptVerification::Verified));
    client_b
        .download(
            &coordinate,
            DownloadAuth::Nip98 {
                signer: &legacy_signer,
            },
            &dir.path().join("legacy-independent-server.zip"),
            |_, _| {},
        )
        .await
        .expect("actual other-server legacy receipt ownership");
    assert!(
        client_b
            .download(
                &coordinate,
                DownloadAuth::Token(complete.download_token.expect("server A token")),
                &dir.path().join("wrong-scope.zip"),
                |_, _| {}
            )
            .await
            .is_err(),
        "credentials never transfer between servers"
    );
    let buyer_db = Database::new(&dir.path().join("buyer-import.db"))
        .await
        .expect("separate buyer device");
    let mut portable = authorities.clone();
    portable.push(receipt.clone());
    let imported = ReceiptPortability::new(buyer_db.pool().clone())
        .import_json(
            buyer.public_key(),
            &serde_json::to_string(&portable).expect("portable JSON"),
            None,
            &|| Ok(()),
        )
        .await
        .expect("offline portable import");
    assert!(imported
        .chains
        .iter()
        .any(|chain| chain.verification == ReceiptVerification::Verified));
    let discovered_db = Database::new(&dir.path().join("buyer-discovery.db"))
        .await
        .expect("another independent buyer installation");
    let nostr = NostrClient::new(
        "gate7-receipt-discovery".into(),
        vec![],
        Some(RelayManagerConfig {
            debug_relays: Some(relays.clone()),
            block_discovery: true,
            query_timeout_secs: 3,
            ..Default::default()
        }),
    )
    .await
    .expect("real buyer Nostr client");
    nostr.connect().await;
    let shared = nostr.get_relay_manager().clone();
    let manager = { shared.lock().await.clone() };
    let sync = ReceiptPortability::new(discovered_db.pool().clone())
        .synchronize(
            buyer.public_key(),
            &ReceiptRelays {
                manager,
                urls: relays,
            },
            true,
            Timestamp::now().as_secs(),
            &|| Ok(()),
        )
        .await
        .expect("actual local relay receipt discovery");
    assert!(sync
        .chains
        .iter()
        .any(|chain| chain.verification == ReceiptVerification::Verified));
    let ownership = OwnershipService::new(
        PurchasesRepository::new(buyer_db.pool().clone()),
        EntitlementsRepository::new(buyer_db.pool().clone()),
    );
    assert!(ownership
        .is_owned(&buyer.public_key().to_hex(), &coordinate)
        .await
        .expect("portable ownership"));
    client_b
        .download(
            &coordinate,
            DownloadAuth::Nip98 {
                signer: &buyer_signer,
            },
            &dir.path().join("portable.zip"),
            |_, _| {},
        )
        .await
        .expect("token-free actual server B download");
    // Stop the actual first router: fallback is real connection failure, never a
    // replacement server. Both candidates remain pinned to the same signed hash.
    a._task.0.abort();
    let private = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .open(dir.path().join("private.zip"))
        .expect("private candidate");
    let downloaded = download_verified(
        http(),
        &coordinate,
        &buyer.public_key().to_hex(),
        &buyer_signer,
        download_candidates(&[a.url.clone(), b.url.clone()], &[], true),
        private,
        digest.clone(),
        PackageFormat::Zip,
        |_| Ok(()),
        |_, _| {},
        |_| {},
    )
    .await
    .expect("authorized actual-server fallback with pinned integrity");
    let installation_root = dir.path().join("installed-v1");
    let extracted = extract_verified_package(&downloaded.package, &installation_root, &profiles)
        .expect("real policy-checked extraction");
    assert_eq!(
        std::fs::read_to_string(installation_root.join("game.txt")).expect("installed contents"),
        "version one"
    );
    let installations = InstallationsRepository::new(buyer_db.pool().clone());
    let record = InstallationRecord {
        id: InstallationId::generate(),
        game_coordinate: coordinate.clone(),
        installation_root: installation_root.clone(),
        package_digest: digest,
        package_format: PackageFormat::Zip,
        resolved_target: host_platform(),
        raw_event_json: serde_json::to_string(&sale).expect("snapshot"),
        publisher_default_profile_id: Some(profiles[0].id.clone()),
        state: InstallationState::Installing,
        recovery_discriminator: None,
        retained_package_coordinate: None,
        created_at: 1,
        updated_at: 1,
    };
    installations
        .begin(&record, &extracted.profiles)
        .await
        .expect("installation intent");
    installations
        .commit_extraction(
            &record.id,
            &record.installation_root,
            &record.package_digest,
            &record.package_format,
            &extracted.profiles,
            &extracted.manifest,
            2,
        )
        .await
        .expect("atomic ready state");
    let ready = installations
        .get_by_coordinate(&coordinate)
        .await
        .expect("ready snapshot")
        .expect("installed");
    let bytes2 = zip_bytes("version two");
    let digest2 = Sha256Digest::parse(&hex::encode(Sha256::digest(&bytes2))).expect("new digest");
    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    let tags = sale
        .tags
        .iter()
        .map(|tag_value| match tag_value.as_slice() {
            [name, _] if name == "file_hash" => {
                tag(&["file_hash", &hex::encode(Sha256::digest(&bytes2))])
            }
            [name, _] if name == "version" => tag(&["version", "2"]),
            _ => tag_value.clone(),
        })
        .collect::<Vec<_>>();
    let update = EventBuilder::new(sale.kind, sale.content.clone())
        .tags(tags)
        .sign_with_keys(&developer)
        .expect("new signed revision");
    publisher
        .send_event(&update)
        .await
        .expect("local update publication");
    tokio::fs::write(&archive, &bytes2)
        .await
        .expect("update archive");
    client_b
        .upload(&dev_signer, &update, &archive)
        .await
        .expect("register update B");
    let path2 = dir.path().join("private-v2.zip");
    let file2 = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .open(&path2)
        .expect("v2 candidate");
    let downloaded2 = download_verified(
        http(),
        &coordinate,
        &buyer.public_key().to_hex(),
        &buyer_signer,
        download_candidates(&[b.url.clone()], &[], true),
        file2,
        digest2.clone(),
        PackageFormat::Zip,
        |_| Ok(()),
        |_, _| {},
        |_| {},
    )
    .await
    .expect("actual server update download");
    let root2 = dir.path().join("installed-v2");
    let extraction2 = extract_verified_package(&downloaded2.package, &root2, &profiles)
        .expect("new build extraction");
    let candidate = InstallationRecord {
        installation_root: root2.clone(),
        package_digest: digest2.clone(),
        raw_event_json: serde_json::to_string(&update).expect("new snapshot"),
        state: InstallationState::Installing,
        ..record.clone()
    };
    let journal = installations
        .begin_update(&ready, &candidate, &extraction2.profiles, &path2)
        .await
        .expect("durable side-by-side update intent");
    let reopened = Database::new(&dir.path().join("buyer-import.db"))
        .await
        .expect("interrupted buyer restart");
    let recovery = InstallationsRepository::new(reopened.pool().clone());
    assert_eq!(
        recovery
            .pending_updates()
            .await
            .expect("restart journal")
            .len(),
        1
    );
    assert_eq!(
        recovery
            .get_by_coordinate(&coordinate)
            .await
            .expect("old snapshot")
            .expect("ready"),
        ready,
        "interruption never promotes candidate"
    );
    let package = RetainedPackage {
        game_coordinate: coordinate.clone(),
        package_id: RetainedPackageId::generate(),
        package_path: path2.clone(),
        package_digest: digest2,
        package_format: PackageFormat::Zip,
        reinstall: RetainedPackageReinstallSnapshot {
            raw_event_json: candidate.raw_event_json.clone(),
            resolved_target: host_platform(),
            launch_profiles: profiles,
            publisher_default_profile_id: record.publisher_default_profile_id.clone(),
        },
        size_bytes: bytes2.len() as u64,
        created_at: 3,
        updated_at: 3,
    };
    let mut invalid = package.clone();
    invalid.package_digest = record.package_digest.clone();
    assert!(
        recovery
            .commit_update(
                &journal,
                &invalid,
                &extraction2.profiles,
                &extraction2.manifest,
                3
            )
            .await
            .is_err(),
        "invalid update transaction rolls back"
    );
    assert_eq!(
        recovery
            .get_by_coordinate(&coordinate)
            .await
            .expect("rollback")
            .expect("old ready"),
        ready
    );
    recovery
        .commit_update(
            &journal,
            &package,
            &extraction2.profiles,
            &extraction2.manifest,
            4,
        )
        .await
        .expect("valid atomic update");
    let upgraded = recovery
        .get_by_coordinate(&coordinate)
        .await
        .expect("upgraded snapshot")
        .expect("ready");
    assert_eq!(upgraded.installation.installation_root, root2);
    assert_eq!(upgraded.installation.state, InstallationState::Ready);
    assert!(
        upgraded.any_profile_available(),
        "installed game profile remains available after atomic update"
    );
    assert_eq!(
        std::fs::read_to_string(installation_root.join("game.txt"))
            .expect("old build retained until guarded cleanup"),
        "version one"
    );
    // Only this fixture-owned obsolete root is removed, after promotion. The
    // production lifecycle cleanup/rollback guards are exercised by Gate 5 tests.
    tokio::fs::remove_dir_all(&installation_root)
        .await
        .expect("remove owned obsolete fixture root");
    recovery
        .finish_update(&journal)
        .await
        .expect("cleanup checkpoint");
    // A synthetic developer-signed refund status tests receipt-chain policy,
    // not a real Lightning refund. It must not affect the unrelated legacy buyer.
    let mut status_tags = receipt
        .tags
        .iter()
        .filter(|value| {
            matches!(
                value.as_slice().first().map(String::as_str),
                Some("order" | "a" | "p" | "authorization")
            )
        })
        .cloned()
        .collect::<Vec<_>>();
    status_tags.extend([
        tag(&["e", &receipt.id.to_hex()]),
        tag(&["status", "refunded"]),
    ]);
    let refunded = EventBuilder::new(
        nostr_sdk::Kind::Custom(1020),
        adp_core::encrypt_receipt_content(&developer, &buyer.public_key(), "{}")
            .expect("private status body"),
    )
    .tags(status_tags)
    .sign_with_keys(&developer)
    .expect("developer receipt status");
    publisher
        .send_event(&refunded)
        .await
        .expect("local receipt status publication");
    ReceiptPortability::new(buyer_db.pool().clone())
        .import_json(
            buyer.public_key(),
            &serde_json::to_string(&refunded).expect("status JSON"),
            None,
            &|| Ok(()),
        )
        .await
        .expect("receipt status reconciliation");
    assert!(!ownership
        .is_owned(&buyer.public_key().to_hex(), &coordinate)
        .await
        .expect("refunded ownership"));
    assert!(
        client_b
            .download(
                &coordinate,
                DownloadAuth::Nip98 {
                    signer: &buyer_signer
                },
                &dir.path().join("refunded.zip"),
                |_, _| {}
            )
            .await
            .is_err(),
        "actual server independently enforces the paid receipt status chain"
    );
    client_b
        .download(
            &coordinate,
            DownloadAuth::Nip98 {
                signer: &legacy_signer,
            },
            &dir.path().join("unrelated-buyer-update.zip"),
            |_, _| {},
        )
        .await
        .expect("unrelated paid buyer remains authorized");
    let registered = b
        .storage
        .stored_files()
        .await
        .expect("registered artifacts")
        .into_iter()
        .find(|file| file.file_hash == package.package_digest.as_str())
        .expect("new registered artifact");
    tokio::fs::write(&registered.file_path, b"corrupted after registration")
        .await
        .expect("isolated server corruption fixture");
    let damaged = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .open(dir.path().join("damaged-download.zip"))
        .expect("isolated failed candidate");
    assert!(
        download_verified(
            http(),
            &coordinate,
            &legacy_buyer.public_key().to_hex(),
            &legacy_signer,
            download_candidates(&[b.url.clone()], &[], true),
            damaged,
            package.package_digest.clone(),
            PackageFormat::Zip,
            |_| Ok(()),
            |_, _| {},
            |_| {}
        )
        .await
        .is_err(),
        "actual server corruption never changes the pinned artifact"
    );
    assert_eq!(
        std::fs::read_to_string(root2.join("game.txt"))
            .expect("ready build survives failed download"),
        "version two"
    );
}
