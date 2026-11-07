use arcadestr_core::adp_settlement::*;
use nostr::{Event, EventBuilder, Keys, Kind, Tag};

fn fixture() -> (SignedQuote, Event, Vec<Event>) {
    let developer = Keys::generate();
    let operator = Keys::generate();
    let terms = CommercialTerms {
        version: SETTLEMENT_VERSION,
        operator_pubkey: operator.public_key().to_hex(),
        payment_destination: "carol@example.test".into(),
        fee: FeePolicy::Fixed {
            amount_msat: 100000,
        },
        limitations: vec![],
    };
    let root = build_terms_event_builder(&TermsPayload::Offer {
        terms: terms.clone(),
    })
    .expect("builder")
    .sign_with_keys(&operator)
    .expect("root");
    let listing = EventBuilder::new(Kind::Custom(30402), "offline test")
        .tags([
            Tag::parse(["d", "game"]).expect("tag"),
            Tag::parse(["lud16", "bob@example.test"]).expect("tag"),
            Tag::parse(["price", "200", "SATS"]).expect("tag"),
            Tag::parse([
                "adp_terms",
                &operator.public_key().to_hex(),
                &root.id.to_hex(),
            ])
            .expect("tag"),
        ])
        .sign_with_keys(&developer)
        .expect("listing");
    let legs = build_payment_legs(
        200000,
        &developer.public_key().to_hex(),
        "bob@example.test",
        &[(terms, root.id.to_hex())],
    )
    .expect("legs");
    let quote = SignedQuote::new(
        "purchase".into(),
        Keys::generate().public_key().to_hex(),
        listing.id.to_hex(),
        format!("30402:{}:game", developer.public_key().to_hex()),
        operator.public_key().to_hex(),
        200000,
        100,
        &RefundDestination {
            endpoint: "buyer@example.test".into(),
            min_sendable_msat: 1,
            max_sendable_msat: 200000,
        },
        legs,
    )
    .expect("quote");
    (quote, listing, vec![root])
}

#[test]
fn exact_accepted_split_is_required_not_just_a_consistent_total() {
    let (q, l, t) = fixture();
    assert!(verify_quote_listing_terms(&q, &l, &t).is_ok());
    let mut changed = q.clone();
    changed.legs[0].recipient_pubkey = Keys::generate().public_key().to_hex();
    assert!(verify_quote_listing_terms(&changed, &l, &t).is_err());
    let mut changed = q.clone();
    changed.legs[0].amount_msat -= 1;
    changed.legs[1].amount_msat += 1;
    assert!(verify_quote_listing_terms(&changed, &l, &t).is_err());
    let mut changed = q.clone();
    changed.legs[0].destination = "attacker@example.test".into();
    assert!(verify_quote_listing_terms(&changed, &l, &t).is_err());
    let mut changed = q.clone();
    changed.legs.reverse();
    assert!(verify_quote_listing_terms(&changed, &l, &t).is_err());
    assert!(verify_quote_listing_terms(&q, &l, &[]).is_err());
    let (_, _, other) = fixture();
    assert!(verify_quote_listing_terms(&q, &l, &other).is_err());
}

#[test]
fn changed_terms_price_listing_or_processor_are_not_implicitly_accepted() {
    let (q, l, t) = fixture();
    for mutation in 0..3 {
        let mut changed = q.clone();
        match mutation {
            0 => changed.listing_event_id = "00".repeat(32),
            1 => changed.processor_pubkey = Keys::generate().public_key().to_hex(),
            _ => changed.gross_msat += 1,
        };
        assert!(verify_quote_listing_terms(&changed, &l, &t).is_err());
    }
    let mut duplicate = t.clone();
    duplicate.push(t[0].clone());
    assert!(verify_quote_listing_terms(&q, &l, &duplicate).is_err());
}

#[test]
fn listing_evidence_must_retain_its_signature_and_unique_commercial_tags() {
    let (q, l, t) = fixture();
    let mut forged = l.clone();
    forged.content.push_str("forged");
    assert!(verify_quote_listing_terms(&q, &forged, &t).is_err());
    let signer = Keys::generate();
    for extra in [
        ["price", "200", "SATS"].as_slice(),
        ["lud16", "attacker@example.test"].as_slice(),
        ["d", "other"].as_slice(),
    ] {
        let mut tags = l.tags.iter().cloned().collect::<Vec<_>>();
        tags.push(Tag::parse(extra.iter().copied()).expect("duplicate tag"));
        let ambiguous = EventBuilder::new(Kind::Custom(30402), "ambiguous evidence")
            .tags(tags)
            .sign_with_keys(&signer)
            .expect("sign");
        let mut quote = q.clone();
        quote.listing_event_id = ambiguous.id.to_hex();
        quote.listing_coordinate = format!("30402:{}:game", signer.public_key().to_hex());
        quote.legs[0].recipient_pubkey = signer.public_key().to_hex();
        assert!(verify_quote_listing_terms(&quote, &ambiguous, &t).is_err());
    }
}
