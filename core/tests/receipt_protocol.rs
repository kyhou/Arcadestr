#![cfg(feature = "native")]

use arcadestr_core::authorization::{
    resolve_authorization, CAPABILITY_ISSUE_RECEIPT, FULFILLMENT_AUTHORIZATION_KIND,
};
use arcadestr_core::purchases::{
    parse_receipt_event, resolve_receipt_chain, validate_adp_receipt_root,
};
use nostr::{Event, EventBuilder, EventId, Keys, Kind, Tag, Timestamp};

fn tag(values: &[&str]) -> Tag {
    Tag::parse(values.iter().copied()).expect("tag")
}

fn coordinate(developer: &Keys) -> String {
    format!("30402:{}:game", developer.public_key().to_hex())
}

fn authorization_root(developer: &Keys, operator: &Keys, fulfillment: &Keys) -> Event {
    EventBuilder::new(Kind::Custom(FULFILLMENT_AUTHORIZATION_KIND), "")
        .tags([
            tag(&["d", "auth"]),
            tag(&["a", &coordinate(developer)]),
            tag(&["p", &operator.public_key().to_hex()]),
            tag(&["fulfillment_pubkey", &fulfillment.public_key().to_hex()]),
            tag(&["capability", CAPABILITY_ISSUE_RECEIPT]),
            tag(&["valid_from", "100"]),
            tag(&["status", "active"]),
        ])
        .custom_created_at(Timestamp::from_secs(100))
        .sign_with_keys(developer)
        .expect("authorization")
}

fn cancellation(developer: &Keys, root: &Event, operator: &Keys, fulfillment: &Keys) -> Event {
    EventBuilder::new(Kind::Custom(FULFILLMENT_AUTHORIZATION_KIND), "")
        .tags([
            tag(&["d", "auth"]),
            tag(&["a", &coordinate(developer)]),
            tag(&["p", &operator.public_key().to_hex()]),
            tag(&["fulfillment_pubkey", &fulfillment.public_key().to_hex()]),
            tag(&["capability", CAPABILITY_ISSUE_RECEIPT]),
            tag(&["valid_from", "100"]),
            tag(&["status", "cancelled"]),
            tag(&["e", &root.id.to_hex()]),
        ])
        .custom_created_at(Timestamp::from_secs(200))
        .sign_with_keys(developer)
        .expect("cancellation")
}

fn receipt(
    signer: &Keys,
    developer: &Keys,
    buyer: nostr::PublicKey,
    authorization: Option<EventId>,
    predecessor: Option<EventId>,
    status: &str,
    at: u64,
) -> Event {
    let mut tags = vec![
        tag(&["order", "order"]),
        tag(&["p", &buyer.to_hex()]),
        tag(&["a", &coordinate(developer)]),
        tag(&["status", status]),
    ];
    if let Some(authorization) = authorization {
        tags.push(tag(&["authorization", &authorization.to_hex()]));
    }
    if let Some(predecessor) = predecessor {
        tags.push(tag(&["e", &predecessor.to_hex()]));
    } else {
        tags.extend([
            tag(&["payment_hash", &"11".repeat(32)]),
            tag(&["amount_msat", "1000"]),
            tag(&["settled_at", &at.to_string()]),
            tag(&["proof", "bolt11-preimage"]),
        ]);
    }
    EventBuilder::new(Kind::Custom(1020), "encrypted")
        .tags(tags)
        .custom_created_at(Timestamp::from_secs(at))
        .sign_with_keys(signer)
        .expect("receipt")
}

#[test]
fn direct_receipt_rejects_redundant_anchor() {
    let developer = Keys::generate();
    let event = receipt(
        &developer,
        &developer,
        Keys::generate().public_key(),
        Some(EventId::all_zeros()),
        None,
        "paid",
        150,
    );
    let parsed = parse_receipt_event(&event).expect("parse");
    assert!(validate_adp_receipt_root(&parsed, developer.public_key(), None).is_err());
}

#[test]
fn delegated_receipt_uses_exact_root_time_even_when_update_is_after_cancellation() {
    let developer = Keys::generate();
    let operator = Keys::generate();
    let fulfillment = Keys::generate();
    let auth_root = authorization_root(&developer, &operator, &fulfillment);
    let cancel = cancellation(&developer, &auth_root, &operator, &fulfillment);
    let authorization =
        resolve_authorization(auth_root.id, &[auth_root.clone(), cancel]).expect("authorization");
    let buyer = Keys::generate().public_key();
    let root = receipt(
        &fulfillment,
        &developer,
        buyer,
        Some(auth_root.id),
        None,
        "paid",
        199,
    );
    let update = receipt(
        &fulfillment,
        &developer,
        buyer,
        Some(auth_root.id),
        Some(root.id),
        "fulfilled",
        201,
    );
    let events = [root, update.clone()];
    let tip = resolve_receipt_chain(&events, developer.public_key(), Some(&authorization))
        .expect("root authorization is evaluated at root time");
    assert_eq!(tip.id, update.id);
}

#[test]
fn delegated_receipt_at_cancellation_is_rejected() {
    let developer = Keys::generate();
    let operator = Keys::generate();
    let fulfillment = Keys::generate();
    let auth_root = authorization_root(&developer, &operator, &fulfillment);
    let cancel = cancellation(&developer, &auth_root, &operator, &fulfillment);
    let authorization =
        resolve_authorization(auth_root.id, &[auth_root.clone(), cancel]).expect("authorization");
    let root = receipt(
        &fulfillment,
        &developer,
        Keys::generate().public_key(),
        Some(auth_root.id),
        None,
        "paid",
        200,
    );
    let parsed = parse_receipt_event(&root).expect("parse");
    assert!(
        validate_adp_receipt_root(&parsed, developer.public_key(), Some(&authorization)).is_err()
    );
}
