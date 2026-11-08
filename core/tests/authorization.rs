use std::collections::BTreeSet;

use arcadestr_core::authorization::{
    parse_attestation_event, parse_authorization_event, resolve_authorization, AuthorizationTerms,
    CAPABILITY_ISSUE_GRANT, CAPABILITY_ISSUE_RECEIPT, CAPABILITY_UPLOAD_BUILD,
    FULFILLMENT_AUTHORIZATION_KIND,
};
use nostr::{Event, EventBuilder, Keys, Kind, Tag, Timestamp};

fn tag(values: &[&str]) -> Tag {
    Tag::parse(values.iter().copied()).expect("tag")
}

fn coordinate(developer: &Keys) -> String {
    format!("30402:{}:game", developer.public_key().to_hex())
}

fn terms(developer: &Keys, operator: &Keys, fulfillment: &Keys) -> AuthorizationTerms {
    AuthorizationTerms {
        authorization_id: "authorization-1".into(),
        coordinate: coordinate(developer),
        operator_pubkey: operator.public_key(),
        fulfillment_pubkey: fulfillment.public_key(),
        capabilities: BTreeSet::from([
            CAPABILITY_ISSUE_RECEIPT.into(),
            CAPABILITY_ISSUE_GRANT.into(),
            CAPABILITY_UPLOAD_BUILD.into(),
        ]),
        valid_from: 100,
    }
}

fn root(developer: &Keys, terms: &AuthorizationTerms) -> Event {
    let mut tags = vec![
        tag(&["d", &terms.authorization_id]),
        tag(&["a", &terms.coordinate]),
        tag(&["p", &terms.operator_pubkey.to_hex()]),
        tag(&["fulfillment_pubkey", &terms.fulfillment_pubkey.to_hex()]),
    ];
    for capability in &terms.capabilities {
        tags.push(tag(&["capability", capability]));
    }
    tags.extend([
        tag(&["valid_from", &terms.valid_from.to_string()]),
        tag(&["status", "active"]),
    ]);
    EventBuilder::new(Kind::Custom(FULFILLMENT_AUTHORIZATION_KIND), "")
        .tags(tags)
        .custom_created_at(Timestamp::from_secs(100))
        .sign_with_keys(developer)
        .expect("root")
}

fn cancel(developer: &Keys, terms: &AuthorizationTerms, predecessor: &Event, at: u64) -> Event {
    let mut tags = vec![
        tag(&["d", &terms.authorization_id]),
        tag(&["a", &terms.coordinate]),
        tag(&["p", &terms.operator_pubkey.to_hex()]),
        tag(&["fulfillment_pubkey", &terms.fulfillment_pubkey.to_hex()]),
    ];
    for capability in &terms.capabilities {
        tags.push(tag(&["capability", capability]));
    }
    tags.extend([
        tag(&["valid_from", &terms.valid_from.to_string()]),
        tag(&["status", "cancelled"]),
        tag(&["e", &predecessor.id.to_hex()]),
    ]);
    EventBuilder::new(Kind::Custom(FULFILLMENT_AUTHORIZATION_KIND), "")
        .tags(tags)
        .custom_created_at(Timestamp::from_secs(at))
        .sign_with_keys(developer)
        .expect("cancel")
}

#[test]
fn authorization_root_enforces_capabilities_and_current_shape() {
    let developer = Keys::generate();
    let operator = Keys::generate();
    let fulfillment = Keys::generate();
    let terms = terms(&developer, &operator, &fulfillment);
    let event = root(&developer, &terms);
    let parsed = parse_authorization_event(&event).expect("parse");
    assert_eq!(parsed.terms, terms);
    let resolved = resolve_authorization(event.id, &[event]).expect("resolve");
    assert!(resolved.has_capability(CAPABILITY_ISSUE_RECEIPT));
    assert!(!resolved.has_capability("future-capability"));
}

#[test]
fn unknown_capabilities_grant_no_authority() {
    let developer = Keys::generate();
    let operator = Keys::generate();
    let fulfillment = Keys::generate();
    let mut terms = terms(&developer, &operator, &fulfillment);
    terms.capabilities = BTreeSet::from(["future-capability".into()]);
    let event = root(&developer, &terms);
    assert!(parse_authorization_event(&event).is_err());
}

#[test]
fn credentials_before_cancellation_remain_authorized_but_equality_is_denied() {
    let developer = Keys::generate();
    let operator = Keys::generate();
    let fulfillment = Keys::generate();
    let terms = terms(&developer, &operator, &fulfillment);
    let root = root(&developer, &terms);
    let cancellation = cancel(&developer, &terms, &root, 200);
    let resolved = resolve_authorization(root.id, &[root, cancellation]).expect("resolve");
    assert!(resolved.authorizes(
        &fulfillment.public_key(),
        &terms.coordinate,
        CAPABILITY_ISSUE_RECEIPT,
        199,
    ));
    assert!(!resolved.authorizes(
        &fulfillment.public_key(),
        &terms.coordinate,
        CAPABILITY_ISSUE_RECEIPT,
        200,
    ));
}

#[test]
fn imported_metadata_preserves_sibling_roots_hints_and_subset_capabilities_without_local_rows() {
    use arcadestr_core::authorization::{
        select_reusable_authorization, validate_listing_authorization_references,
    };
    let developer = Keys::generate();
    let first_operator = Keys::generate();
    let second_operator = Keys::generate();
    let first_key = Keys::generate();
    let second_key = Keys::generate();
    let mut first_terms = terms(&developer, &first_operator, &first_key);
    first_terms.capabilities = BTreeSet::from([CAPABILITY_ISSUE_RECEIPT.into()]);
    let first = root(&developer, &first_terms);
    let mut second_terms = terms(&developer, &second_operator, &second_key);
    second_terms.authorization_id = "independent-sibling".into();
    let second = root(&developer, &second_terms);
    let cancellation = cancel(&developer, &first_terms, &first, 200);
    let authorizations = vec![
        resolve_authorization(first.id, &[first.clone(), cancellation]).expect("historical root"),
        resolve_authorization(second.id, &[second.clone()]).expect("current sibling"),
    ];
    let listing = EventBuilder::new(Kind::Custom(30402), "imported metadata")
        .tags([
            tag(&["d", "game"]),
            tag(&["title", "Imported"]),
            tag(&["t", "game"]),
            tag(&[
                "fulfillment_authorization",
                &first.id.to_hex(),
                &first_key.public_key().to_hex(),
                "wss://first.example",
            ]),
            tag(&[
                "fulfillment_authorization",
                &second.id.to_hex(),
                &second_key.public_key().to_hex(),
                "wss://second.example",
            ]),
        ])
        .sign_with_keys(&developer)
        .expect("listing");
    let references = validate_listing_authorization_references(&listing, &authorizations)
        .expect("no local database relationship required");
    assert_eq!(references.len(), 2);
    assert_eq!(references[0].root_event_id, first.id);
    assert_eq!(
        references[0].relay_hint.as_deref(),
        Some("wss://first.example")
    );
    assert_eq!(references[1].root_event_id, second.id);
    assert_eq!(
        authorizations[0].terms.capabilities,
        first_terms.capabilities
    );
    assert!(authorizations[0].authorizes(
        &first_key.public_key(),
        &coordinate(&developer),
        CAPABILITY_ISSUE_RECEIPT,
        199
    ));
    assert!(!authorizations[0].authorizes(
        &first_key.public_key(),
        &coordinate(&developer),
        CAPABILITY_ISSUE_RECEIPT,
        201
    ));
    let candidates = [first_key.public_key(), second_key.public_key()]
        .into_iter()
        .collect();
    assert!(select_reusable_authorization(
        &authorizations,
        &references,
        &candidates,
        first_operator.public_key(),
        &coordinate(&developer),
        &BTreeSet::from([CAPABILITY_UPLOAD_BUILD.into()]),
        201
    )
    .is_none());
    assert_eq!(
        select_reusable_authorization(
            &authorizations,
            &references,
            &candidates,
            second_operator.public_key(),
            &coordinate(&developer),
            &BTreeSet::from([CAPABILITY_UPLOAD_BUILD.into()]),
            201
        )
        .expect("independent sibling remains usable")
        .root_event_id,
        second.id
    );
    assert!(validate_listing_authorization_references(&listing, &authorizations[1..]).is_err());
}

#[test]
fn metadata_authority_rejects_malformed_or_misbound_siblings_instead_of_discarding_them() {
    use arcadestr_core::authorization::validate_listing_authorization_references;
    let developer = Keys::generate();
    let operator = Keys::generate();
    let fulfillment = Keys::generate();
    let terms = terms(&developer, &operator, &fulfillment);
    let root = root(&developer, &terms);
    let resolved = resolve_authorization(root.id, &[root.clone()]).expect("root");
    for reference in [
        vec!["fulfillment_authorization".to_string(), "broken".into()],
        vec![
            "fulfillment_authorization".into(),
            root.id.to_hex(),
            Keys::generate().public_key().to_hex(),
        ],
    ] {
        let listing = EventBuilder::new(Kind::Custom(30402), "listing")
            .tags([
                tag(&["d", "game"]),
                tag(&["title", "Imported"]),
                Tag::parse(reference).expect("tag"),
            ])
            .sign_with_keys(&developer)
            .expect("listing");
        assert!(validate_listing_authorization_references(&listing, &[resolved.clone()]).is_err());
    }
}

#[test]
fn malformed_successor_does_not_block_valid_sibling_recovery() {
    let developer = Keys::generate();
    let operator = Keys::generate();
    let fulfillment = Keys::generate();
    let terms = terms(&developer, &operator, &fulfillment);
    let root = root(&developer, &terms);
    let malformed = EventBuilder::new(Kind::Custom(FULFILLMENT_AUTHORIZATION_KIND), "")
        .tags([
            tag(&["e", &root.id.to_hex()]),
            tag(&["status", "cancelled"]),
        ])
        .custom_created_at(Timestamp::from_secs(150))
        .sign_with_keys(&developer)
        .expect("malformed");
    let valid = cancel(&developer, &terms, &root, 200);
    let resolved = resolve_authorization(root.id, &[root, malformed, valid]).expect("recover");
    assert_eq!(resolved.cancelled_at, Some(200));
}

#[test]
fn two_valid_cancellation_siblings_fail_closed() {
    let developer = Keys::generate();
    let operator = Keys::generate();
    let fulfillment = Keys::generate();
    let terms = terms(&developer, &operator, &fulfillment);
    let root = root(&developer, &terms);
    let first = cancel(&developer, &terms, &root, 200);
    let second = cancel(&developer, &terms, &root, 201);
    assert!(resolve_authorization(root.id, &[root, first, second]).is_err());
}

#[cfg(feature = "native")]
#[test]
fn capability_superset_reuse_selects_lowest_typed_root() {
    use arcadestr_core::authorization::{select_reusable_authorization, ResolvedAuthorization};
    use arcadestr_core::marketplace::FulfillmentAuthorizationReference;
    use std::collections::HashSet;

    let developer = Keys::generate();
    let operator = Keys::generate();
    let fulfillment = Keys::generate();
    let terms = terms(&developer, &operator, &fulfillment);
    let low = nostr::EventId::from_hex(&"11".repeat(32)).expect("low");
    let high = nostr::EventId::from_hex(&"ee".repeat(32)).expect("high");
    let make = |root_event_id| ResolvedAuthorization {
        root_event_id,
        developer_pubkey: developer.public_key(),
        terms: terms.clone(),
        events: Vec::new(),
        cancelled_at: None,
    };
    let authorizations = vec![make(high), make(low)];
    let references = vec![
        FulfillmentAuthorizationReference {
            root_event_id: high,
            fulfillment_pubkey: fulfillment.public_key(),
            relay_hint: None,
        },
        FulfillmentAuthorizationReference {
            root_event_id: low,
            fulfillment_pubkey: fulfillment.public_key(),
            relay_hint: None,
        },
    ];
    let selected = select_reusable_authorization(
        &authorizations,
        &references,
        &HashSet::from([fulfillment.public_key()]),
        operator.public_key(),
        &terms.coordinate,
        &BTreeSet::from([CAPABILITY_UPLOAD_BUILD.into()]),
        100,
    )
    .expect("superset is reusable");
    assert_eq!(selected.root_event_id, low);

    let mismatched = vec![FulfillmentAuthorizationReference {
        root_event_id: low,
        fulfillment_pubkey: Keys::generate().public_key(),
        relay_hint: None,
    }];
    assert!(select_reusable_authorization(
        &authorizations,
        &mismatched,
        &HashSet::from([fulfillment.public_key()]),
        operator.public_key(),
        &terms.coordinate,
        &BTreeSet::from([CAPABILITY_UPLOAD_BUILD.into()]),
        100,
    )
    .is_none());
}

#[test]
fn revoked_attestation_blocks_new_use_without_invalidating_historical_authorization() {
    let developer = Keys::generate();
    let operator = Keys::generate();
    let fulfillment = Keys::generate();
    let terms = terms(&developer, &operator, &fulfillment);
    let root = root(&developer, &terms);
    let resolved = resolve_authorization(root.id, &[root]).expect("authorization");
    let d = format!(
        "{}:{}",
        developer.public_key().to_hex(),
        fulfillment.public_key().to_hex()
    );
    let attestation = EventBuilder::new(Kind::Custom(30404), "")
        .tags([
            tag(&["d", &d]),
            tag(&["p", &developer.public_key().to_hex()]),
            tag(&[
                "fulfillment_pubkey",
                &fulfillment.public_key().to_hex(),
                "90",
                "150",
            ]),
            tag(&["scope", "game"]),
        ])
        .custom_created_at(Timestamp::from_secs(150))
        .sign_with_keys(&operator)
        .expect("attestation");
    let attestation = parse_attestation_event(&attestation).expect("attestation parses");

    assert!(!attestation.allows_new_operations_at(150));
    assert!(resolved.authorizes(
        &fulfillment.public_key(),
        &terms.coordinate,
        CAPABILITY_ISSUE_RECEIPT,
        140,
    ));
}

fn attestation(
    valid_from: u64,
    revoked_at: Option<u64>,
) -> arcadestr_core::authorization::ParsedAttestation {
    let developer = Keys::generate();
    let operator = Keys::generate();
    let fulfillment = Keys::generate();
    let d = format!(
        "{}:{}",
        developer.public_key().to_hex(),
        fulfillment.public_key().to_hex()
    );
    let revoked_at = revoked_at.map(|at| at.to_string()).unwrap_or_default();
    let event = EventBuilder::new(Kind::Custom(30404), "")
        .tags([
            tag(&["d", &d]),
            tag(&["p", &developer.public_key().to_hex()]),
            tag(&[
                "fulfillment_pubkey",
                &fulfillment.public_key().to_hex(),
                &valid_from.to_string(),
                &revoked_at,
            ]),
            tag(&["scope", "game"]),
        ])
        .custom_created_at(Timestamp::from_secs(valid_from))
        .sign_with_keys(&operator)
        .expect("attestation");
    parse_attestation_event(&event).expect("attestation parses")
}

#[test]
fn a_just_minted_attestation_is_usable_by_a_reader_whose_clock_trails_the_operator() {
    // The operator stamps `valid_from` from its own clock while provisioning;
    // the developer reads the attestation moments later, off a clock that may
    // sit a little behind.
    let attestation = attestation(1_000, None);

    assert!(!attestation.allows_new_operations_at(999));
    assert!(attestation.allows_new_operations_observed_at(999));
    assert!(attestation.allows_new_operations_observed_at(1_000));
}

#[test]
fn an_attestation_far_in_the_readers_future_stays_unusable() {
    let attestation = attestation(10_000, None);

    assert!(!attestation.allows_new_operations_observed_at(1_000));
}

#[test]
fn clock_tolerance_never_revives_a_revoked_attestation() {
    let attestation = attestation(100, Some(150));

    assert!(!attestation.allows_new_operations_observed_at(150));
    assert!(!attestation.allows_new_operations_observed_at(200));
    assert!(attestation.allows_new_operations_observed_at(149));
}
