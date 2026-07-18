use arcadestr_core::adp_protocol::{
    ADP_CAMPAIGN_KIND, ENTITLEMENT_GRANT_KIND, TAG_AUTHORIZATION_EVENT,
};
use arcadestr_core::campaign::{
    build_campaign_event_builder, parse_campaign_event, resolve_campaign, CampaignBuildParams,
    CampaignStatus, CampaignTransition,
};
use arcadestr_core::entitlements::{
    parse_entitlement_event, resolve_entitlement_grant, validate_adp_entitlement, EntitlementError,
    GrantStatus, IssuanceDelegation,
};
use nostr::{Event, EventBuilder, EventId, Keys, Kind, Tag, Timestamp};

fn tag(values: &[&str]) -> Tag {
    Tag::parse(values.iter().copied()).expect("test tag must be valid")
}

fn coordinate(publisher: &Keys, game: &str) -> String {
    format!("30402:{}:{game}", publisher.public_key())
}

#[allow(clippy::too_many_arguments)]
fn campaign_event(
    signer: &Keys,
    coordinate: &str,
    campaign_id: &str,
    status: &str,
    predecessor: Option<EventId>,
    terms: Option<(u64, u64)>,
    created_at: u64,
) -> Event {
    let mut tags = vec![
        tag(&["d", campaign_id]),
        tag(&["a", coordinate]),
        tag(&["status", status]),
    ];
    if let Some(predecessor) = predecessor {
        tags.push(tag(&["e", &predecessor.to_hex()]));
    }
    if let Some((starts, ends)) = terms {
        tags.extend([
            tag(&["mode", "claim"]),
            tag(&["starts", &starts.to_string()]),
            tag(&["ends", &ends.to_string()]),
        ]);
    }
    EventBuilder::new(Kind::Custom(ADP_CAMPAIGN_KIND), "")
        .tags(tags)
        .custom_created_at(Timestamp::from_secs(created_at))
        .sign_with_keys(signer)
        .expect("campaign must sign")
}

fn campaign_root(publisher: &Keys, coordinate: &str) -> Event {
    campaign_event(
        publisher,
        coordinate,
        "campaign-1",
        "active",
        None,
        Some((120, 300)),
        100,
    )
}

#[allow(clippy::too_many_arguments)]
fn grant_event(
    signer: &Keys,
    recipient: &Keys,
    coordinate: &str,
    grant_id: &str,
    source_event: EventId,
    status: &str,
    predecessor: Option<EventId>,
    created_at: u64,
    authorization_event: Option<EventId>,
) -> Event {
    let mut tags = vec![
        tag(&["d", grant_id]),
        tag(&["p", &recipient.public_key().to_hex()]),
        tag(&["a", coordinate]),
        tag(&["source_event", &source_event.to_hex()]),
        tag(&["status", status]),
    ];
    if let Some(predecessor) = predecessor {
        tags.push(tag(&["e", &predecessor.to_hex()]));
    }
    if let Some(authorization_event) = authorization_event {
        tags.push(tag(&[
            TAG_AUTHORIZATION_EVENT,
            &authorization_event.to_hex(),
        ]));
    }
    EventBuilder::new(Kind::Custom(ENTITLEMENT_GRANT_KIND), "")
        .tags(tags)
        .custom_created_at(Timestamp::from_secs(created_at))
        .sign_with_keys(signer)
        .expect("grant must sign")
}

fn resolved_campaign(
    publisher: &Keys,
    coordinate: &str,
) -> arcadestr_core::campaign::ResolvedCampaign {
    let root = campaign_root(publisher, coordinate);
    resolve_campaign(
        &[parse_campaign_event(&root).expect("valid campaign")],
        publisher.public_key(),
        coordinate,
    )
    .expect("campaign must resolve")
}

#[test]
fn campaign_valid_root_and_half_open_interval() {
    let publisher = Keys::generate();
    let coordinate = coordinate(&publisher, "game");
    let campaign = resolved_campaign(&publisher, &coordinate);

    assert!(campaign.is_claimable_at(120));
    assert!(campaign.is_claimable_at(299));
    assert!(!campaign.is_claimable_at(300));
}

#[test]
fn campaign_wrong_publisher_is_rejected() {
    let publisher = Keys::generate();
    let unrelated = Keys::generate();
    let coordinate = coordinate(&publisher, "game");

    assert!(parse_campaign_event(&campaign_root(&unrelated, &coordinate)).is_err());
}

#[test]
fn campaign_invalid_interval_is_rejected() {
    let publisher = Keys::generate();
    let coordinate = coordinate(&publisher, "game");
    let event = campaign_event(
        &publisher,
        &coordinate,
        "campaign-1",
        "active",
        None,
        Some((200, 200)),
        100,
    );
    let parsed = parse_campaign_event(&event).expect("shape is parseable");

    assert!(resolve_campaign(&[parsed], publisher.public_key(), &coordinate).is_err());
}

#[test]
fn campaign_post_start_term_update_is_rejected() {
    let publisher = Keys::generate();
    let coordinate = coordinate(&publisher, "game");
    let root = campaign_root(&publisher, &coordinate);
    let update = campaign_event(
        &publisher,
        &coordinate,
        "campaign-1",
        "active",
        Some(root.id),
        Some((200, 400)),
        121,
    );
    let nodes = [
        parse_campaign_event(&root).expect("root parses"),
        parse_campaign_event(&update).expect("update parses"),
    ];

    assert!(resolve_campaign(&nodes, publisher.public_key(), &coordinate).is_err());
}

#[test]
fn campaign_cancellation_is_accepted() {
    let publisher = Keys::generate();
    let coordinate = coordinate(&publisher, "game");
    let root = campaign_root(&publisher, &coordinate);
    let cancel = campaign_event(
        &publisher,
        &coordinate,
        "campaign-1",
        "cancelled",
        Some(root.id),
        None,
        180,
    );
    let campaign = resolve_campaign(
        &[
            parse_campaign_event(&root).expect("root parses"),
            parse_campaign_event(&cancel).expect("cancellation parses"),
        ],
        publisher.public_key(),
        &coordinate,
    )
    .expect("cancellation resolves");

    assert_eq!(
        campaign.state_at(180).expect("state").status,
        CampaignStatus::Cancelled
    );
}

#[test]
fn campaign_fork_is_rejected() {
    let publisher = Keys::generate();
    let coordinate = coordinate(&publisher, "game");
    let root = campaign_root(&publisher, &coordinate);
    let first = campaign_event(
        &publisher,
        &coordinate,
        "campaign-1",
        "active",
        Some(root.id),
        Some((200, 400)),
        105,
    );
    let second = campaign_event(
        &publisher,
        &coordinate,
        "campaign-1",
        "active",
        Some(root.id),
        Some((220, 420)),
        110,
    );
    let nodes = [&root, &first, &second]
        .into_iter()
        .map(|event| parse_campaign_event(event).expect("event parses"))
        .collect::<Vec<_>>();

    assert!(resolve_campaign(&nodes, publisher.public_key(), &coordinate).is_err());
}

#[test]
fn grant_valid_publisher_issuance() {
    let publisher = Keys::generate();
    let recipient = Keys::generate();
    let coordinate = coordinate(&publisher, "game");
    let campaign = resolved_campaign(&publisher, &coordinate);
    let grant = grant_event(
        &publisher,
        &recipient,
        &coordinate,
        "grant-1",
        campaign.root_event_id,
        "granted",
        None,
        150,
        None,
    );
    let grant =
        resolve_entitlement_grant(&[parse_entitlement_event(&grant).expect("grant parses")])
            .expect("grant resolves");

    validate_adp_entitlement(&grant, &campaign, None, &[]).expect("publisher grant is valid");
}

#[test]
fn grant_listing_delegation_without_authorization_is_rejected() {
    let publisher = Keys::generate();
    let fulfillment = Keys::generate();
    let recipient = Keys::generate();
    let coordinate = coordinate(&publisher, "game");
    let campaign = resolved_campaign(&publisher, &coordinate);
    let authorization = campaign.root_event_id;
    let grant = grant_event(
        &fulfillment,
        &recipient,
        &coordinate,
        "grant-1",
        campaign.root_event_id,
        "granted",
        None,
        150,
        Some(authorization),
    );
    let grant =
        resolve_entitlement_grant(&[parse_entitlement_event(&grant).expect("grant parses")])
            .expect("grant resolves");
    let delegation = IssuanceDelegation {
        pubkey: fulfillment.public_key(),
        valid_from: 100,
        revoked_at: None,
    };

    assert!(matches!(
        validate_adp_entitlement(&grant, &campaign, None, &[delegation]),
        Err(EntitlementError::MissingAuthorization)
    ));
}

#[test]
fn grant_unrelated_signer_is_rejected() {
    let publisher = Keys::generate();
    let unrelated = Keys::generate();
    let recipient = Keys::generate();
    let coordinate = coordinate(&publisher, "game");
    let campaign = resolved_campaign(&publisher, &coordinate);
    let grant = grant_event(
        &unrelated,
        &recipient,
        &coordinate,
        "grant-1",
        campaign.root_event_id,
        "granted",
        None,
        150,
        None,
    );
    let grant =
        resolve_entitlement_grant(&[parse_entitlement_event(&grant).expect("grant parses")])
            .expect("grant resolves");

    assert!(matches!(
        validate_adp_entitlement(&grant, &campaign, None, &[]),
        Err(EntitlementError::MissingAuthorizationEvent)
    ));
}

#[test]
fn grant_fulfillment_key_revocation_is_rejected() {
    let publisher = Keys::generate();
    let fulfillment = Keys::generate();
    let recipient = Keys::generate();
    let coordinate = coordinate(&publisher, "game");
    let campaign = resolved_campaign(&publisher, &coordinate);
    let root = grant_event(
        &publisher,
        &recipient,
        &coordinate,
        "grant-1",
        campaign.root_event_id,
        "granted",
        None,
        150,
        None,
    );
    let revoke = grant_event(
        &fulfillment,
        &recipient,
        &coordinate,
        "grant-1",
        campaign.root_event_id,
        "revoked",
        Some(root.id),
        160,
        None,
    );
    let nodes = [&root, &revoke]
        .into_iter()
        .map(|event| parse_entitlement_event(event).expect("event parses"))
        .collect::<Vec<_>>();
    let grant = resolve_entitlement_grant(&nodes).expect("grant resolves structurally");
    let delegation = IssuanceDelegation {
        pubkey: fulfillment.public_key(),
        valid_from: 100,
        revoked_at: None,
    };

    assert!(matches!(
        validate_adp_entitlement(&grant, &campaign, None, &[delegation]),
        Err(EntitlementError::UnauthorizedRevoker)
    ));
}

#[test]
fn grant_invariant_mutation_is_rejected() {
    let publisher = Keys::generate();
    let recipient = Keys::generate();
    let coordinate = coordinate(&publisher, "game");
    let campaign = resolved_campaign(&publisher, &coordinate);
    let root = grant_event(
        &publisher,
        &recipient,
        &coordinate,
        "grant-1",
        campaign.root_event_id,
        "granted",
        None,
        150,
        None,
    );
    let revoke = grant_event(
        &publisher,
        &recipient,
        &coordinate,
        "grant-2",
        campaign.root_event_id,
        "revoked",
        Some(root.id),
        160,
        None,
    );
    let nodes = [&root, &revoke]
        .into_iter()
        .map(|event| parse_entitlement_event(event).expect("event parses"))
        .collect::<Vec<_>>();

    assert!(resolve_entitlement_grant(&nodes).is_err());
}

#[test]
fn grant_fork_is_rejected() {
    let publisher = Keys::generate();
    let recipient = Keys::generate();
    let coordinate = coordinate(&publisher, "game");
    let campaign = resolved_campaign(&publisher, &coordinate);
    let root = grant_event(
        &publisher,
        &recipient,
        &coordinate,
        "grant-1",
        campaign.root_event_id,
        "granted",
        None,
        150,
        None,
    );
    let first = grant_event(
        &publisher,
        &recipient,
        &coordinate,
        "grant-1",
        campaign.root_event_id,
        "revoked",
        Some(root.id),
        160,
        None,
    );
    let second = grant_event(
        &publisher,
        &recipient,
        &coordinate,
        "grant-1",
        campaign.root_event_id,
        "revoked",
        Some(root.id),
        170,
        None,
    );
    let nodes = [&root, &first, &second]
        .into_iter()
        .map(|event| parse_entitlement_event(event).expect("event parses"))
        .collect::<Vec<_>>();

    assert!(resolve_entitlement_grant(&nodes).is_err());
}

#[test]
fn parser_preserves_server_authorization_anchor() {
    let publisher = Keys::generate();
    let recipient = Keys::generate();
    let coordinate = coordinate(&publisher, "game");
    let campaign = campaign_root(&publisher, &coordinate);
    let grant = grant_event(
        &publisher,
        &recipient,
        &coordinate,
        "grant-1",
        campaign.id,
        "granted",
        None,
        150,
        Some(campaign.id),
    );

    assert_eq!(
        parse_entitlement_event(&grant)
            .expect("grant parses")
            .authorization_event,
        Some(campaign.id)
    );
    assert!(matches!(
        parse_campaign_event(&campaign)
            .expect("campaign parses")
            .transition,
        CampaignTransition::Root(_)
    ));
    assert_eq!(GrantStatus::Granted.as_str(), "granted");
}

#[test]
fn campaign_builder_creates_root_update_and_cancellation_shapes() {
    let publisher = Keys::generate();
    let coordinate = coordinate(&publisher, "game");
    let root = build_campaign_event_builder(&CampaignBuildParams::active(
        "campaign-1".into(),
        coordinate.clone(),
        120,
        300,
        None,
    ))
    .expect("root builder")
    .custom_created_at(Timestamp::from_secs(100))
    .sign_with_keys(&publisher)
    .expect("root signs");
    let update = build_campaign_event_builder(&CampaignBuildParams::active(
        "campaign-1".into(),
        coordinate.clone(),
        220,
        400,
        Some(root.id),
    ))
    .expect("update builder")
    .custom_created_at(Timestamp::from_secs(110))
    .sign_with_keys(&publisher)
    .expect("update signs");
    let cancel = build_campaign_event_builder(&CampaignBuildParams::cancel(
        "campaign-1".into(),
        coordinate.clone(),
        update.id,
    ))
    .expect("cancel builder")
    .custom_created_at(Timestamp::from_secs(150))
    .sign_with_keys(&publisher)
    .expect("cancel signs");

    let nodes = [&root, &update, &cancel]
        .into_iter()
        .map(|event| parse_campaign_event(event).expect("campaign event parses"))
        .collect::<Vec<_>>();
    let resolved = resolve_campaign(&nodes, publisher.public_key(), &coordinate)
        .expect("campaign chain resolves");
    assert_eq!(
        resolved.state_at(150).expect("state").status,
        CampaignStatus::Cancelled
    );
}

#[test]
fn cancellation_blocks_new_grants_but_preserves_earlier_grant() {
    let publisher = Keys::generate();
    let recipient = Keys::generate();
    let coordinate = coordinate(&publisher, "game");
    let root = campaign_root(&publisher, &coordinate);
    let cancel = campaign_event(
        &publisher,
        &coordinate,
        "campaign-1",
        "cancelled",
        Some(root.id),
        None,
        180,
    );
    let campaign = resolve_campaign(
        &[
            parse_campaign_event(&root).expect("root parses"),
            parse_campaign_event(&cancel).expect("cancellation parses"),
        ],
        publisher.public_key(),
        &coordinate,
    )
    .expect("campaign resolves");

    for (grant_id, issued_at, valid) in [("early", 150, true), ("late", 180, false)] {
        let event = grant_event(
            &publisher,
            &recipient,
            &coordinate,
            grant_id,
            campaign.root_event_id,
            "granted",
            None,
            issued_at,
            None,
        );
        let grant =
            resolve_entitlement_grant(&[parse_entitlement_event(&event).expect("grant parses")])
                .expect("grant resolves");
        assert_eq!(
            validate_adp_entitlement(&grant, &campaign, None, &[]).is_ok(),
            valid,
            "grant {grant_id}"
        );
    }
}
