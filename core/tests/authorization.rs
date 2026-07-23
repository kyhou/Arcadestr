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
