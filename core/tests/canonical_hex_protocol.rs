#![cfg(feature = "native")]
//! ADP-01 §Canonical Hex Encoding (gap C6): ADP-defined hex fields are
//! accepted only in lowercase, on both the consuming and publishing side.
//! Each case pairs a lowercase control that must still be accepted.
use arcadestr_core::adp_protocol::{ENTITLEMENT_GRANT_KIND, FULFILLMENT_AUTHORIZATION_KIND};
use arcadestr_core::adp_publish::{
    build_adp_listing_event_builder, AdpListingInput, AdpPublishError,
    FulfillmentAuthorizationInput,
};
use arcadestr_core::authorization::{parse_attestation_event, parse_authorization_event};
use arcadestr_core::entitlements::parse_entitlement_event;
use arcadestr_core::marketplace::{parse_listing, AcquisitionPolicy};
use arcadestr_core::purchases::parse_receipt_event;
use nostr::{Event, EventBuilder, Keys, Kind, Tag, Timestamp};

fn tag(values: &[&str]) -> Tag {
    Tag::parse(values.iter().copied()).expect("tag")
}

fn sign(keys: &Keys, kind: u16, tags: Vec<Tag>) -> Event {
    EventBuilder::new(Kind::Custom(kind), "")
        .tags(tags)
        .custom_created_at(Timestamp::from_secs(1_700_000_000))
        .sign_with_keys(keys)
        .expect("sign")
}

fn hex(byte: &str) -> String {
    byte.repeat(32)
}

fn listing(extra: Vec<Tag>) -> arcadestr_core::marketplace::Nip99Listing {
    let mut tags = vec![tag(&["d", "game"]), tag(&["title", "Game"])];
    tags.extend(extra);
    parse_listing(sign(&Keys::generate(), 30402, tags)).expect("listing parses")
}

#[test]
fn listing_file_hash_must_be_lowercase() {
    assert_eq!(
        listing(vec![tag(&["file_hash", &hex("ab")])]).file_hash,
        Some(hex("ab"))
    );
    assert_eq!(
        listing(vec![tag(&["file_hash", &hex("AB")])]).file_hash,
        None,
        "uppercase must not be normalized"
    );
}

#[test]
fn listing_references_must_be_lowercase() {
    let key = Keys::generate().public_key().to_hex();
    let lower = listing(vec![
        tag(&["fulfillment_authorization", &hex("ab"), &key]),
        tag(&["campaign", &hex("cd")]),
        tag(&["adp_terms", &key, &hex("ef")]),
    ]);
    assert_eq!(lower.fulfillment_authorizations.len(), 1);
    assert_eq!(lower.campaigns.len(), 1);
    assert_eq!(lower.accepted_terms.len(), 1);

    let upper = listing(vec![
        tag(&["fulfillment_authorization", &hex("AB"), &key]),
        tag(&["fulfillment_authorization", &hex("ab"), &key.to_uppercase()]),
        tag(&["campaign", &hex("CD")]),
        tag(&["adp_terms", &key.to_uppercase(), &hex("ef")]),
    ]);
    assert!(upper.fulfillment_authorizations.is_empty());
    assert_eq!(upper.malformed_fulfillment_authorization_tags.len(), 2);
    assert!(upper.campaigns.is_empty());
    assert!(upper.accepted_terms.is_empty());
}

fn grant(source: Option<&str>, authorization: Option<&str>) -> Event {
    let publisher = Keys::generate();
    let mut tags = vec![
        tag(&["d", "Review-Copy-9C2F"]),
        tag(&["p", &Keys::generate().public_key().to_hex()]),
        tag(&[
            "a",
            &format!("30402:{}:game", publisher.public_key().to_hex()),
        ]),
        tag(&["status", "granted"]),
    ];
    if let Some(source) = source {
        tags.push(tag(&["source_event", source]));
    }
    if let Some(authorization) = authorization {
        tags.push(tag(&["authorization", authorization]));
    }
    sign(&publisher, ENTITLEMENT_GRANT_KIND, tags)
}

#[test]
fn grant_anchors_must_be_lowercase_but_the_grant_id_is_opaque() {
    // The uppercase, hyphenated `d` is an opaque identifier and stays valid.
    assert!(parse_entitlement_event(&grant(None, None)).is_ok());
    assert!(parse_entitlement_event(&grant(Some(&hex("ab")), None)).is_ok());
    assert!(parse_entitlement_event(&grant(None, Some(&hex("ab")))).is_ok());
    assert!(parse_entitlement_event(&grant(Some(&hex("AB")), None)).is_err());
    assert!(parse_entitlement_event(&grant(None, Some(&hex("AB")))).is_err());
}

#[test]
fn authorization_and_attestation_keys_must_be_lowercase() {
    let developer = Keys::generate();
    let operator = Keys::generate().public_key().to_hex();
    let key = Keys::generate().public_key().to_hex();
    let authorization = |fulfillment: &str| {
        sign(
            &developer,
            FULFILLMENT_AUTHORIZATION_KIND,
            vec![
                tag(&["d", "AUTH-1"]),
                tag(&[
                    "a",
                    &format!("30402:{}:game", developer.public_key().to_hex()),
                ]),
                tag(&["p", &operator]),
                tag(&["fulfillment_pubkey", fulfillment]),
                tag(&["capability", "issue_receipt"]),
                tag(&["valid_from", "1700000000"]),
                tag(&["status", "active"]),
            ],
        )
    };
    assert!(parse_authorization_event(&authorization(&key)).is_ok());
    assert!(parse_authorization_event(&authorization(&key.to_uppercase())).is_err());

    let attestation = |fulfillment: &str| {
        let developer_hex = developer.public_key().to_hex();
        sign(
            &Keys::generate(),
            30404,
            vec![
                tag(&["d", &format!("{developer_hex}:{key}")]),
                tag(&["p", &developer_hex]),
                tag(&["fulfillment_pubkey", fulfillment, "1700000000", ""]),
            ],
        )
    };
    assert!(parse_attestation_event(&attestation(&key)).is_ok());
    assert!(parse_attestation_event(&attestation(&key.to_uppercase())).is_err());
}

fn receipt(extra: Vec<Tag>) -> Event {
    let signer = Keys::generate();
    let mut tags = vec![
        tag(&["order", "ORDER-1"]),
        tag(&["p", &Keys::generate().public_key().to_hex()]),
        tag(&["a", &format!("30402:{}:game", signer.public_key().to_hex())]),
        tag(&["payment_hash", &hex("11")]),
        tag(&["amount_msat", "1000"]),
        tag(&["settled_at", "1700000000"]),
        tag(&["status", "paid"]),
    ];
    tags.extend(extra);
    sign(&signer, 1020, tags)
}

#[test]
fn receipt_anchor_and_zap_proof_must_be_lowercase() {
    let preimage = tag(&["proof", "bolt11-preimage"]);
    assert!(parse_receipt_event(&receipt(vec![
        preimage.clone(),
        tag(&["authorization", &hex("ab")])
    ]))
    .is_ok());
    assert!(parse_receipt_event(&receipt(vec![
        preimage,
        tag(&["authorization", &hex("AB")])
    ]))
    .is_err());
    assert!(parse_receipt_event(&receipt(vec![tag(&["proof", "zap", &hex("ab")])])).is_ok());
    assert!(parse_receipt_event(&receipt(vec![tag(&["proof", "zap", &hex("AB")])])).is_err());
}

fn publish_input() -> AdpListingInput {
    AdpListingInput {
        d_tag: "game".into(),
        title: "Game".into(),
        description: "Description".into(),
        price_sats: 0,
        lud16: None,
        tags: Vec::new(),
        images: Vec::new(),
        servers: vec!["https://dist.example.com".into()],
        file_hash: Some(hex("11")),
        version: Some("1.0.0".into()),
        fulfillment_authorizations: Vec::new(),
        acquisition: AcquisitionPolicy::Gated,
        platforms: Vec::new(),
        campaigns: Vec::new(),
        nip94_event_id: None,
        preserved_tags: Vec::new(),
    }
}

#[test]
fn the_publisher_never_emits_non_canonical_references() {
    let key = Keys::generate().public_key().to_hex();
    let authorization = |root: String, pubkey: String| FulfillmentAuthorizationInput {
        root_event_id: root,
        fulfillment_pubkey: pubkey,
        relay_hint: None,
    };

    let mut input = publish_input();
    input.fulfillment_authorizations = vec![authorization(hex("ab"), key.clone())];
    assert!(build_adp_listing_event_builder(&input).is_ok());
    for bad in [
        authorization(hex("AB"), key.clone()),
        authorization(hex("ab"), key.to_uppercase()),
    ] {
        input.fulfillment_authorizations = vec![bad];
        assert!(matches!(
            build_adp_listing_event_builder(&input),
            Err(AdpPublishError::InvalidFulfillmentAuthorization)
        ));
    }

    let mut input = publish_input();
    input.campaigns = vec![(hex("cd"), None)];
    assert!(build_adp_listing_event_builder(&input).is_ok());
    input.campaigns = vec![(hex("CD"), None)];
    assert!(matches!(
        build_adp_listing_event_builder(&input),
        Err(AdpPublishError::InvalidCampaignPointer)
    ));
}

#[test]
fn a_provision_response_with_non_canonical_identifiers_is_rejected() {
    use arcadestr_core::adp_client::{validate_provision_response, ProvisionResponse};
    let response = |pubkey: String, event_id: String| ProvisionResponse {
        fulfillment_pubkey: pubkey,
        attestation_event_id: event_id,
        attestation: None,
        scope: None,
    };
    let key = Keys::generate().public_key().to_hex();
    assert!(validate_provision_response(&response(key.clone(), hex("ab"))).is_ok());
    assert!(validate_provision_response(&response(key.to_uppercase(), hex("ab"))).is_err());
    assert!(validate_provision_response(&response(key, hex("AB"))).is_err());
}
