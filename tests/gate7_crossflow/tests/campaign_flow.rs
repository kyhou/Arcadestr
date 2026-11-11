use super::fixtures::*;
use arcadestr_core::{
    adp_client::{AdpClient, DownloadAuth, EntitlementClaimRequest},
    adp_publish::{
        build_adp_listing_event_builder, build_fulfillment_authorization_event_builder,
        AdpListingInput, FulfillmentAuthorizationInput,
    },
    authorization::{
        discover_authorization, AuthorizationTerms, CAPABILITY_ISSUE_GRANT,
        CAPABILITY_ISSUE_RECEIPT, CAPABILITY_UPLOAD_BUILD,
    },
    campaign::{
        build_campaign_event_builder, parse_campaign_event, resolve_campaign, CampaignBuildParams,
    },
    entitlement_reconciliation::{EntitlementEvidence, EntitlementSource},
    entitlements::{parse_entitlement_event, resolve_entitlement_grant},
    entitlements_repository::EntitlementsRepository,
    marketplace::AcquisitionPolicy,
    nostr::NostrClient,
    ownership::{DurableCredentialStatus, OwnershipService},
    purchases::PurchasesRepository,
    relay_manager::{RelayManager, RelayManagerConfig},
    storage::Database,
    store_page::{
        build_listing_store_page_replacement, build_store_page_event_builder,
        parse_store_page_event, store_page_coordinate, validate_store_page_association,
        StorePageBuildParams, StorePageContentV1, StorePagePointerAction,
    },
};
use nostr_sdk::{Filter, Keys, PublicKey, Timestamp};
use sha2::{Digest, Sha256};
use std::sync::{Arc, Mutex};

struct Source(RelayManager);
#[async_trait::async_trait]
impl EntitlementSource for Source {
    async fn query(&self, filter: Filter) -> Result<EntitlementEvidence, String> {
        let events = self
            .0
            .fetch_events_best_effort(filter)
            .await
            .map_err(|e| e.to_string())?;
        Ok(EntitlementEvidence {
            events,
            incomplete: false,
        })
    }
}

#[tokio::test]
async fn actual_nostr_store_page_claim_cancel_and_independent_grant_revocation() {
    let events: Events = Arc::new(Mutex::new(vec![]));
    let (one, _one) = relay(events.clone()).await;
    let (two, _two) = relay(events.clone()).await;
    let relays = vec![one, two];
    let server = Server::start(&relays).await;
    let developer = Keys::generate();
    let buyer = Keys::generate();
    let developer_signer = signer(&developer);
    let buyer_signer = signer(&buyer);
    let client = AdpClient::new(&server.url, http());
    let provision = client
        .provision(&developer_signer, None)
        .await
        .expect("held fulfillment signer");
    let coordinate = format!("30402:{}:claim-game", developer.public_key());
    let now = Timestamp::now().as_secs();
    let authority = build_fulfillment_authorization_event_builder(&AuthorizationTerms {
        authorization_id: "claim-authority".into(),
        coordinate: coordinate.clone(),
        operator_pubkey: server.operator.public_key(),
        fulfillment_pubkey: PublicKey::from_hex(&provision.fulfillment_pubkey).expect("held key"),
        capabilities: std::collections::BTreeSet::from([
            CAPABILITY_ISSUE_GRANT.into(),
            CAPABILITY_ISSUE_RECEIPT.into(),
            CAPABILITY_UPLOAD_BUILD.into(),
        ]),
        valid_from: now - 30,
    })
    .expect("authorization builder")
    .sign_with_keys(&developer)
    .expect("developer authorization");
    let campaign = build_campaign_event_builder(&CampaignBuildParams::active(
        "keep".into(),
        coordinate.clone(),
        now - 20,
        now + 3600,
        None,
    ))
    .expect("campaign")
    .custom_created_at(Timestamp::from_secs(now - 25))
    .sign_with_keys(&developer)
    .expect("developer campaign");
    let bytes = b"claim fixture build";
    let initial = build_adp_listing_event_builder(&AdpListingInput {
        d_tag: "claim-game".into(),
        title: "Claim fixture".into(),
        description: "actual local server flow".into(),
        price_sats: 0,
        lud16: None,
        tags: vec!["game".into()],
        images: vec![],
        servers: vec![server.url.clone()],
        file_hash: Some(hex::encode(Sha256::digest(bytes))),
        version: Some("1".into()),
        fulfillment_authorizations: vec![FulfillmentAuthorizationInput {
            root_event_id: authority.id.to_hex(),
            fulfillment_pubkey: provision.fulfillment_pubkey,
            relay_hint: None,
        }],
        acquisition: AcquisitionPolicy::Gated,
        platforms: vec![],
        campaigns: vec![(campaign.id.to_hex(), None)],
        nip94_event_id: None,
        preserved_tags: vec![],
    })
    .expect("listing builder")
    .custom_created_at(Timestamp::from_secs(now - 10))
    .sign_with_keys(&developer)
    .expect("listing");
    let mut content = StorePageContentV1::default();
    content.basic.title = Some("Actual signed Store Page".into());
    let page = build_store_page_event_builder(&StorePageBuildParams {
        publisher: developer.public_key(),
        presentation_id: "claim-presentation".into(),
        listing_coordinates: vec![coordinate.clone()],
        content,
        compact_tags: Default::default(),
    })
    .expect("Store Page builder")
    .sign_with_keys(&developer)
    .expect("Store Page signature");
    let sale = build_listing_store_page_replacement(
        &initial,
        developer.public_key(),
        &StorePagePointerAction::Link {
            store_page_coordinate: store_page_coordinate(
                developer.public_key(),
                "claim-presentation",
            ),
            relay_hint: None,
        },
    )
    .expect("reciprocal pointer")
    .custom_created_at(Timestamp::from_secs(now - 5))
    .sign_with_keys(&developer)
    .expect("listing association");
    validate_store_page_association(
        &sale,
        &parse_store_page_event(&page).expect("Store Page validation"),
    )
    .expect("reciprocal integrity");
    let publisher = nostr_sdk::Client::new(developer.clone());
    for url in &relays {
        publisher.add_relay(url).await.expect("relay");
    }
    publisher.connect().await;
    for event in [&authority, &campaign, &page, &sale] {
        publisher
            .send_event(event)
            .await
            .expect("local signed publication");
    }
    let dir = tempfile::tempdir_in("/tmp/opencode").expect("separate databases");
    let artifact = dir.path().join("build.zip");
    tokio::fs::write(&artifact, bytes).await.expect("artifact");
    client
        .upload(&developer_signer, &sale, &artifact)
        .await
        .expect("actual signed upload");
    let nostr = NostrClient::new(
        "gate7-independent-buyer".into(),
        vec![],
        Some(RelayManagerConfig {
            debug_relays: Some(relays),
            block_discovery: true,
            query_timeout_secs: 3,
            ..Default::default()
        }),
    )
    .await
    .expect("actual Arcadestr Nostr infrastructure");
    nostr.connect().await;
    let discovered = nostr
        .fetch_nip99_listings(20, None)
        .await
        .expect("actual marketplace discovery");
    let found = discovered
        .iter()
        .find(|l| l.event_id == sale.id.to_hex())
        .expect("exact signed listing discovered");
    assert_eq!(found.file_hash, Some(hex::encode(Sha256::digest(bytes))));
    let shared = nostr.get_relay_manager().clone();
    let manager = { shared.lock().await.clone() };
    let authorization = discover_authorization(&manager, authority.id, developer.public_key())
        .await
        .expect("authorization discovered and verified");
    let claim = client
        .entitlement_claim(
            &buyer_signer,
            EntitlementClaimRequest {
                game_coordinate: coordinate.clone(),
                campaign_event_id: campaign.id.to_hex(),
            },
        )
        .await
        .expect("actual claim issuance");
    let repeat = client
        .entitlement_claim(
            &buyer_signer,
            EntitlementClaimRequest {
                game_coordinate: coordinate.clone(),
                campaign_event_id: campaign.id.to_hex(),
            },
        )
        .await
        .expect("idempotent claim");
    assert_eq!(claim.grant.id, repeat.grant.id);
    let resolved_campaign = resolve_campaign(
        &[parse_campaign_event(&campaign).expect("campaign parse")],
        developer.public_key(),
        &coordinate,
    )
    .expect("campaign rules");
    let publisher_db = Database::new(&dir.path().join("publisher.db"))
        .await
        .expect("publisher DB");
    let buyer_db = Database::new(&dir.path().join("buyer.db"))
        .await
        .expect("independent buyer DB");
    let publisher_repo = EntitlementsRepository::new(publisher_db.pool().clone());
    let buyer_repo = EntitlementsRepository::new(buyer_db.pool().clone());
    for repo in [&publisher_repo, &buyer_repo] {
        repo.ingest_event(&claim.grant, &resolved_campaign, Some(&authorization))
            .await
            .expect("historically verified issued grant");
    }
    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    let cancel = build_campaign_event_builder(&CampaignBuildParams::cancel(
        "keep".into(),
        coordinate.clone(),
        campaign.id,
    ))
    .expect("cancel")
    .custom_created_at(Timestamp::now())
    .sign_with_keys(&developer)
    .expect("cancel signature");
    publisher
        .send_event(&cancel)
        .await
        .expect("local cancellation");
    let cancelled = client
        .entitlement_claim(
            &signer(&Keys::generate()),
            EntitlementClaimRequest {
                game_coordinate: coordinate.clone(),
                campaign_event_id: campaign.id.to_hex(),
            },
        )
        .await;
    assert!(
        matches!(
            cancelled,
            Err(arcadestr_core::adp_client::AdpClientError::ClaimInvalidCampaign(ref message))
                if message.contains("campaign is not claimable")
        ),
        "cancelled campaign rejects a new claimant by campaign policy"
    );
    client
        .download(
            &coordinate,
            DownloadAuth::Nip98 {
                signer: &buyer_signer,
            },
            &dir.path().join("historical.zip"),
            |_, _| {},
        )
        .await
        .expect("historically valid grant survives campaign cancellation");
    let grant = resolve_entitlement_grant(&[parse_entitlement_event(&claim.grant).expect("grant")])
        .expect("grant chain");
    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    let revoke = arcadestr_core::publisher_management::revoke_grant_builder(
        &grant,
        developer.public_key(),
        claim.grant.id,
        Timestamp::now().as_secs(),
    )
    .expect("exact individual revocation")
    .sign_with_keys(&developer)
    .expect("developer revokes");
    publisher_repo
        .ingest_event(&revoke, &resolved_campaign, Some(&authorization))
        .await
        .expect("publisher persistence");
    publisher
        .send_event(&revoke)
        .await
        .expect("local revocation publication");
    assert!(buyer_repo
        .is_owned(&buyer.public_key().to_hex(), &coordinate)
        .await
        .expect("independent DB has not magically changed"));
    let report = buyer_repo
        .reconcile_known_grants(buyer.public_key(), &Source(manager), &|| Ok(()))
        .await
        .expect("actual buyer relay reconciliation");
    assert_eq!(report.revoked, 1);
    let ownership = OwnershipService::new(
        PurchasesRepository::new(buyer_db.pool().clone()),
        buyer_repo,
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
    assert!(
        client
            .download(
                &coordinate,
                DownloadAuth::Nip98 {
                    signer: &buyer_signer
                },
                &dir.path().join("revoked.zip"),
                |_, _| {}
            )
            .await
            .is_err(),
        "actual server independently denies revoked ownership"
    );
    buyer_db.close().await;
    let reopened = Database::new(&dir.path().join("buyer.db"))
        .await
        .expect("restart");
    assert!(!EntitlementsRepository::new(reopened.pool().clone())
        .is_owned(&buyer.public_key().to_hex(), &coordinate)
        .await
        .expect("durable revocation after restart"));
}
