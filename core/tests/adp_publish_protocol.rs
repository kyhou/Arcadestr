#![cfg(feature = "native")]

use arcadestr_core::adp_publish::{
    build_adp_listing_event_builder, AdpListingInput, FulfillmentAuthorizationInput,
};
use arcadestr_core::marketplace::AcquisitionPolicy;
use nostr::{EventId, Keys};

#[test]
fn replacement_preserves_unknown_and_authorization_siblings_with_byte_deduplication() {
    let developer = Keys::generate();
    let fulfillment = Keys::generate();
    let first = EventId::from_hex(&"11".repeat(32)).expect("first").to_hex();
    let second = EventId::from_hex(&"22".repeat(32))
        .expect("second")
        .to_hex();
    let first_raw = vec![
        "fulfillment_authorization".into(),
        first.clone(),
        fulfillment.public_key().to_hex(),
    ];
    let malformed = vec!["fulfillment_authorization".into(), "bad".into()];
    let builder = build_adp_listing_event_builder(&AdpListingInput {
        d_tag: "game".into(),
        title: "Game".into(),
        description: "Description".into(),
        price_sats: 0,
        lud16: None,
        tags: Vec::new(),
        images: Vec::new(),
        servers: vec!["https://dist.example.com".into()],
        file_hash: Some("11".repeat(32)),
        version: Some("1.0.0".into()),
        fulfillment_authorizations: vec![
            FulfillmentAuthorizationInput {
                root_event_id: first.clone(),
                fulfillment_pubkey: fulfillment.public_key().to_hex(),
                relay_hint: None,
            },
            FulfillmentAuthorizationInput {
                root_event_id: second,
                fulfillment_pubkey: fulfillment.public_key().to_hex(),
                relay_hint: None,
            },
        ],
        acquisition: AcquisitionPolicy::Gated,
        platforms: Vec::new(),
        campaigns: Vec::new(),
        nip94_event_id: None,
        preserved_tags: vec![
            vec!["custom-client-tag".into(), "keep".into()],
            first_raw.clone(),
            first_raw,
            malformed,
        ],
    })
    .expect("listing builder");
    let event = builder.sign_with_keys(&developer).expect("listing");
    let tags = event
        .tags
        .iter()
        .map(|tag| tag.clone().to_vec())
        .collect::<Vec<_>>();
    assert!(tags.contains(&vec!["custom-client-tag".into(), "keep".into()]));
    assert_eq!(
        tags.iter()
            .filter(|tag| tag
                .first()
                .is_some_and(|name| name == "fulfillment_authorization")
                && tag.get(1) == Some(&first))
            .count(),
        1
    );
    assert_eq!(
        tags.iter()
            .filter(|tag| tag
                .first()
                .is_some_and(|name| name == "fulfillment_authorization"))
            .count(),
        3
    );
}
