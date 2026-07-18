use arcadestr_core::adp_protocol::{ADP_CAMPAIGN_KIND, ENTITLEMENT_GRANT_KIND};
use arcadestr_core::authorization::{
    parse_authorization_event, resolve_authorization, ResolvedAuthorization,
    FULFILLMENT_AUTHORIZATION_KIND,
};
use arcadestr_core::campaign::{parse_campaign_event, resolve_campaign, ResolvedCampaign};
use arcadestr_core::entitlements::{
    parse_entitlement_event, resolve_entitlement_grant, validate_adp_entitlement, EntitlementError,
    IssuanceDelegation, ResolvedEntitlementGrant,
};
use nostr::{Event, EventBuilder, EventId, Keys, Kind, Tag, Timestamp};

fn tag(values: &[&str]) -> Tag {
    Tag::parse(values.iter().copied()).expect("test tag must be valid")
}

fn coordinate(developer: &Keys) -> String {
    format!("30402:{}:game", developer.public_key())
}

fn campaign(developer: &Keys, coordinate: &str) -> ResolvedCampaign {
    let root = EventBuilder::new(Kind::Custom(ADP_CAMPAIGN_KIND), "")
        .tags([
            tag(&["d", "campaign"]),
            tag(&["a", coordinate]),
            tag(&["mode", "claim"]),
            tag(&["starts", "100"]),
            tag(&["ends", "300"]),
            tag(&["status", "active"]),
        ])
        .custom_created_at(Timestamp::from_secs(90))
        .sign_with_keys(developer)
        .expect("campaign signs");
    resolve_campaign(
        &[parse_campaign_event(&root).expect("campaign parses")],
        developer.public_key(),
        coordinate,
    )
    .expect("campaign resolves")
}

fn authorization_root(
    developer: &Keys,
    fulfillment: &Keys,
    coordinate: &str,
    valid_from: u64,
    valid_until: Option<u64>,
) -> Event {
    let mut tags = vec![
        tag(&["d", "authorization-root"]),
        tag(&["authorization_id", "authorization-1"]),
        tag(&["a", coordinate]),
        tag(&["p", &fulfillment.public_key().to_hex()]),
        tag(&["valid_from", &valid_from.to_string()]),
        tag(&["status", "active"]),
    ];
    if let Some(valid_until) = valid_until {
        tags.push(tag(&["valid_until", &valid_until.to_string()]));
    }
    EventBuilder::new(Kind::Custom(FULFILLMENT_AUTHORIZATION_KIND), "")
        .tags(tags)
        .custom_created_at(Timestamp::from_secs(80))
        .sign_with_keys(developer)
        .expect("authorization signs")
}

fn authorization_revoke(
    developer: &Keys,
    fulfillment: &Keys,
    coordinate: &str,
    predecessor: EventId,
    transition_d: &str,
    created_at: u64,
) -> Event {
    EventBuilder::new(Kind::Custom(FULFILLMENT_AUTHORIZATION_KIND), "")
        .tags([
            tag(&["d", transition_d]),
            tag(&["authorization_id", "authorization-1"]),
            tag(&["a", coordinate]),
            tag(&["p", &fulfillment.public_key().to_hex()]),
            tag(&["status", "revoked"]),
            tag(&["e", &predecessor.to_hex()]),
        ])
        .custom_created_at(Timestamp::from_secs(created_at))
        .sign_with_keys(developer)
        .expect("revocation signs")
}

fn resolve_auth(root: &Event, successors: &[Event]) -> Result<ResolvedAuthorization, String> {
    let nodes = std::iter::once(root)
        .chain(successors)
        .map(|event| parse_authorization_event(event).map_err(|error| error.to_string()))
        .collect::<Result<Vec<_>, _>>()?;
    resolve_authorization(root.id, &nodes).map_err(|error| error.to_string())
}

fn grant(
    signer: &Keys,
    recipient: &Keys,
    coordinate: &str,
    campaign: &ResolvedCampaign,
    authorization_event: Option<EventId>,
    created_at: u64,
) -> ResolvedEntitlementGrant {
    let mut tags = vec![
        tag(&["d", "grant"]),
        tag(&["p", &recipient.public_key().to_hex()]),
        tag(&["a", coordinate]),
        tag(&["source_event", &campaign.root_event_id.to_hex()]),
        tag(&["status", "granted"]),
    ];
    if let Some(authorization_event) = authorization_event {
        tags.push(tag(&["authorization_event", &authorization_event.to_hex()]));
    }
    let event = EventBuilder::new(Kind::Custom(ENTITLEMENT_GRANT_KIND), "")
        .tags(tags)
        .custom_created_at(Timestamp::from_secs(created_at))
        .sign_with_keys(signer)
        .expect("grant signs");
    resolve_entitlement_grant(&[parse_entitlement_event(&event).expect("grant parses")])
        .expect("grant resolves")
}

fn delegation(fulfillment: &Keys, valid_from: u64, revoked_at: Option<u64>) -> IssuanceDelegation {
    IssuanceDelegation {
        pubkey: fulfillment.public_key(),
        valid_from,
        revoked_at,
    }
}

#[test]
fn direct_publisher_grant_needs_no_authorization() {
    let developer = Keys::generate();
    let recipient = Keys::generate();
    let coordinate = coordinate(&developer);
    let campaign = campaign(&developer, &coordinate);
    let grant = grant(&developer, &recipient, &coordinate, &campaign, None, 150);

    validate_adp_entitlement(&grant, &campaign, None, &[])
        .expect("direct publisher grant is valid");
}

#[test]
fn direct_publisher_grant_with_anchor_requires_that_authorization() {
    let developer = Keys::generate();
    let recipient = Keys::generate();
    let coordinate = coordinate(&developer);
    let campaign = campaign(&developer, &coordinate);
    let grant = grant(
        &developer,
        &recipient,
        &coordinate,
        &campaign,
        Some(EventId::all_zeros()),
        150,
    );

    assert!(matches!(
        validate_adp_entitlement(&grant, &campaign, None, &[]),
        Err(EntitlementError::MissingAuthorization)
    ));
}

#[test]
fn delegated_grant_requires_valid_authorization_and_listing_delegation() {
    let developer = Keys::generate();
    let fulfillment = Keys::generate();
    let recipient = Keys::generate();
    let coordinate = coordinate(&developer);
    let campaign = campaign(&developer, &coordinate);
    let root = authorization_root(&developer, &fulfillment, &coordinate, 100, None);
    let authorization = resolve_auth(&root, &[]).expect("authorization resolves");
    let grant = grant(
        &fulfillment,
        &recipient,
        &coordinate,
        &campaign,
        Some(root.id),
        150,
    );

    validate_adp_entitlement(
        &grant,
        &campaign,
        Some(&authorization),
        &[delegation(&fulfillment, 100, None)],
    )
    .expect("both authorization sources are valid");
}

#[test]
fn delegated_grant_without_authorization_event_is_rejected() {
    let developer = Keys::generate();
    let fulfillment = Keys::generate();
    let recipient = Keys::generate();
    let coordinate = coordinate(&developer);
    let campaign = campaign(&developer, &coordinate);
    let grant = grant(&fulfillment, &recipient, &coordinate, &campaign, None, 150);

    assert!(matches!(
        validate_adp_entitlement(
            &grant,
            &campaign,
            None,
            &[delegation(&fulfillment, 100, None)]
        ),
        Err(EntitlementError::MissingAuthorizationEvent)
    ));
}

#[test]
fn authorization_for_another_developer_is_rejected() {
    let developer = Keys::generate();
    let other = Keys::generate();
    let fulfillment = Keys::generate();
    let recipient = Keys::generate();
    let game_coordinate = coordinate(&developer);
    let other_coordinate = coordinate(&other);
    let campaign = campaign(&developer, &game_coordinate);
    let root = authorization_root(&other, &fulfillment, &other_coordinate, 100, None);
    let authorization = resolve_auth(&root, &[]).expect("other authorization resolves");
    let grant = grant(
        &fulfillment,
        &recipient,
        &game_coordinate,
        &campaign,
        Some(root.id),
        150,
    );

    assert!(validate_adp_entitlement(
        &grant,
        &campaign,
        Some(&authorization),
        &[delegation(&fulfillment, 100, None)],
    )
    .is_err());
}

#[test]
fn authorization_for_another_fulfillment_key_is_rejected() {
    let developer = Keys::generate();
    let fulfillment = Keys::generate();
    let other = Keys::generate();
    let recipient = Keys::generate();
    let coordinate = coordinate(&developer);
    let campaign = campaign(&developer, &coordinate);
    let root = authorization_root(&developer, &other, &coordinate, 100, None);
    let authorization = resolve_auth(&root, &[]).expect("authorization resolves");
    let grant = grant(
        &fulfillment,
        &recipient,
        &coordinate,
        &campaign,
        Some(root.id),
        150,
    );

    assert!(validate_adp_entitlement(
        &grant,
        &campaign,
        Some(&authorization),
        &[delegation(&fulfillment, 100, None)],
    )
    .is_err());
}

#[test]
fn authorization_beginning_after_issuance_is_rejected() {
    let developer = Keys::generate();
    let fulfillment = Keys::generate();
    let recipient = Keys::generate();
    let coordinate = coordinate(&developer);
    let campaign = campaign(&developer, &coordinate);
    let root = authorization_root(&developer, &fulfillment, &coordinate, 160, None);
    let authorization = resolve_auth(&root, &[]).expect("authorization resolves");
    let grant = grant(
        &fulfillment,
        &recipient,
        &coordinate,
        &campaign,
        Some(root.id),
        150,
    );

    assert!(validate_adp_entitlement(
        &grant,
        &campaign,
        Some(&authorization),
        &[delegation(&fulfillment, 100, None)],
    )
    .is_err());
}

#[test]
fn authorization_revoked_before_issuance_is_rejected() {
    let developer = Keys::generate();
    let fulfillment = Keys::generate();
    let recipient = Keys::generate();
    let coordinate = coordinate(&developer);
    let campaign = campaign(&developer, &coordinate);
    let root = authorization_root(&developer, &fulfillment, &coordinate, 100, None);
    let revoke = authorization_revoke(
        &developer,
        &fulfillment,
        &coordinate,
        root.id,
        "revoke",
        140,
    );
    let authorization = resolve_auth(&root, &[revoke]).expect("authorization resolves");
    let grant = grant(
        &fulfillment,
        &recipient,
        &coordinate,
        &campaign,
        Some(root.id),
        150,
    );

    assert!(validate_adp_entitlement(
        &grant,
        &campaign,
        Some(&authorization),
        &[delegation(&fulfillment, 100, None)],
    )
    .is_err());
}

#[test]
fn authorization_revoked_after_issuance_preserves_earlier_grant() {
    let developer = Keys::generate();
    let fulfillment = Keys::generate();
    let recipient = Keys::generate();
    let coordinate = coordinate(&developer);
    let campaign = campaign(&developer, &coordinate);
    let root = authorization_root(&developer, &fulfillment, &coordinate, 100, None);
    let revoke = authorization_revoke(
        &developer,
        &fulfillment,
        &coordinate,
        root.id,
        "revoke",
        160,
    );
    let authorization = resolve_auth(&root, &[revoke]).expect("authorization resolves");
    let grant = grant(
        &fulfillment,
        &recipient,
        &coordinate,
        &campaign,
        Some(root.id),
        150,
    );

    validate_adp_entitlement(
        &grant,
        &campaign,
        Some(&authorization),
        &[delegation(&fulfillment, 100, None)],
    )
    .expect("later revocation is prospective");
}

#[test]
fn forked_authorization_lifecycle_is_rejected() {
    let developer = Keys::generate();
    let fulfillment = Keys::generate();
    let coordinate = coordinate(&developer);
    let root = authorization_root(&developer, &fulfillment, &coordinate, 100, None);
    let first = authorization_revoke(
        &developer,
        &fulfillment,
        &coordinate,
        root.id,
        "revoke-1",
        160,
    );
    let second = authorization_revoke(
        &developer,
        &fulfillment,
        &coordinate,
        root.id,
        "revoke-2",
        170,
    );

    assert!(resolve_auth(&root, &[first, second]).is_err());
}

#[test]
fn valid_listing_delegation_cannot_replace_invalid_authorization() {
    let developer = Keys::generate();
    let fulfillment = Keys::generate();
    let recipient = Keys::generate();
    let coordinate = coordinate(&developer);
    let campaign = campaign(&developer, &coordinate);
    let root = authorization_root(&developer, &fulfillment, &coordinate, 100, None);
    let grant = grant(
        &fulfillment,
        &recipient,
        &coordinate,
        &campaign,
        Some(root.id),
        150,
    );

    assert!(validate_adp_entitlement(
        &grant,
        &campaign,
        None,
        &[delegation(&fulfillment, 100, None)],
    )
    .is_err());
}
