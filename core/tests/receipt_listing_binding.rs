#![cfg(feature = "native")]
use arcadestr_core::purchases::{parse_receipt_event, validate_receipt_listing_binding};
use nostr::{Event, EventBuilder, Keys, Kind, Tag};

fn listing(key: &Keys, id: &str) -> Event {
    EventBuilder::new(Kind::Custom(30402), "offline listing")
        .tags([Tag::parse(["d", id]).expect("tag")])
        .sign_with_keys(key)
        .expect("listing")
}

#[test]
fn signed_receipt_must_bind_the_requested_listing_not_just_its_own_issuer() {
    let developer = Keys::generate();
    let buyer = Keys::generate();
    let other = Keys::generate();
    let expected = listing(&developer, "purchased");
    let coordinate = format!("30402:{}:purchased", developer.public_key().to_hex());
    let receipt = EventBuilder::new(
        Kind::Custom(1020),
        "encrypted evidence omitted in binding-only fixture",
    )
    .tags([
        Tag::parse(["a", coordinate.as_str()]).expect("a"),
        Tag::parse(["p", buyer.public_key().to_hex().as_str()]).expect("p"),
        Tag::parse(["order", "offline-order"]).expect("order"),
        Tag::parse(["status", "paid"]).expect("status"),
        Tag::parse(["payment_hash", "11".repeat(32).as_str()]).expect("hash"),
        Tag::parse(["amount_msat", "1000"]).expect("amount"),
        Tag::parse(["settled_at", "100"]).expect("time"),
        Tag::parse(["proof", "bolt11-preimage"]).expect("proof"),
    ])
    .sign_with_keys(&developer)
    .expect("receipt");
    let parsed = parse_receipt_event(&receipt).expect("signed receipt structure");
    assert!(validate_receipt_listing_binding(&parsed, &expected).is_ok());
    assert!(
        validate_receipt_listing_binding(&parsed, &listing(&developer, "another-game")).is_err()
    );
    assert!(validate_receipt_listing_binding(&parsed, &listing(&other, "purchased")).is_err());
    let mut forged = expected.clone();
    forged.content.push_str("tampered");
    assert!(validate_receipt_listing_binding(&parsed, &forged).is_err());
}
