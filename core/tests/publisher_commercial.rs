//! Isolated signed publisher-commercial fixtures. No network, invoice or payment.
use arcadestr_core::adp_settlement::*;
use nostr::{Event, EventBuilder, JsonUtil, Keys, Kind, Tag, Timestamp};

fn offer(keys: &Keys, fee: FeePolicy) -> Event {
    build_terms_event_builder(&TermsPayload::Offer {
        terms: CommercialTerms {
            version: 1,
            operator_pubkey: keys.public_key().to_hex(),
            payment_destination: "operator@example.test".into(),
            fee,
            limitations: vec!["Operator-authored limitation; no client-invented semantics".into()],
        },
    })
    .expect("offer")
    .custom_created_at(Timestamp::from(10))
    .sign_with_keys(keys)
    .expect("signed")
}

fn setup(
    root: &Event,
) -> (
    PublisherTermsContext,
    Vec<PublisherTermsServer>,
    Vec<AcceptedTermsRef>,
) {
    let url = "https://operator.example.test".to_owned();
    (
        PublisherTermsContext {
            publisher_pubkey: Keys::generate().public_key().to_hex(),
            servers: vec![url.clone()],
            price_sats: 1000,
            developer_destination: "developer@example.test".into(),
        },
        vec![PublisherTermsServer {
            url,
            operator_pubkey: root.pubkey.to_hex(),
            version: 1,
            split_enabled: true,
            advertised_terms: Some(root.id.to_hex()),
        }],
        vec![AcceptedTermsRef {
            operator_pubkey: root.pubkey.to_hex(),
            terms_event_id: root.id.to_hex(),
        }],
    )
}

fn listing(keys: &Keys, roots: &[AcceptedTermsRef]) -> Event {
    EventBuilder::new(Kind::Custom(30402), "historical listing")
        .tags([
            Tag::parse(["d", "game"]).expect("coordinate"),
            Tag::parse(["price", "1000", "SATS"]).expect("price"),
            Tag::parse(["lud16", "developer@example.test"]).expect("destination"),
        ])
        .tags(roots.iter().map(|r| {
            Tag::parse(["adp_terms", &r.operator_pubkey, &r.terms_event_id]).expect("tag")
        }))
        .sign_with_keys(keys)
        .expect("listing")
}

#[test]
fn publisher_multiple_operators_use_buyer_fee_math_and_deduplicate_endpoint_fees() {
    let first = offer(
        &Keys::generate(),
        FeePolicy::BasisPoints {
            basis_points: 125,
            minimum_msat: Some(20_000),
            maximum_msat: Some(30_000),
        },
    );
    let second = offer(
        &Keys::generate(),
        FeePolicy::Fixed {
            amount_msat: 40_000,
        },
    );
    let (mut context, mut servers, mut refs) = setup(&first);
    let (_, mut more_servers, more_refs) = setup(&second);
    more_servers[0].url = "https://second.example.test".into();
    context.servers.push(more_servers[0].url.clone());
    servers.extend(more_servers);
    refs.extend(more_refs);
    let mut mirror = servers[0].clone();
    mirror.url = "https://mirror.example.test".into();
    context.servers.push(mirror.url.clone());
    servers.push(mirror);
    let review = review_publisher_terms(&context, &servers, &refs, &[first, second], true, 100)
        .expect("review");
    assert_eq!(review.offers.len(), 2);
    assert_eq!(
        review
            .legs
            .iter()
            .map(|leg| leg.amount_msat)
            .collect::<Vec<_>>(),
        vec![20_000, 40_000, 940_000]
    );
    assert_eq!(
        review.legs.iter().map(|leg| leg.amount_msat).sum::<u64>(),
        1_000_000
    );
    assert_eq!(review.offers[0].limitations.len(), 1);
}

#[test]
fn publisher_rejects_missing_forged_wrong_operator_and_future_roots() {
    let root = offer(&Keys::generate(), FeePolicy::None);
    let (context, servers, refs) = setup(&root);
    assert!(review_publisher_terms(&context, &servers, &refs, &[], true, 100).is_err());
    let mut json = serde_json::to_value(&root).expect("JSON");
    json["sig"] = serde_json::json!("00".repeat(64));
    let forged = serde_json::from_value(json).expect("forged event");
    assert!(review_publisher_terms(&context, &servers, &refs, &[forged], true, 100).is_err());
    let other = offer(&Keys::generate(), FeePolicy::None);
    assert!(review_publisher_terms(&context, &servers, &refs, &[other], true, 100).is_err());
    assert!(review_publisher_terms(&context, &servers, &refs, &[root], true, 9).is_err());
}

#[test]
fn publisher_self_signed_offer_cannot_impersonate_the_endpoint_operator() {
    let operator = Keys::generate();
    let attacker = Keys::generate();
    let root = build_terms_event_builder(&TermsPayload::Offer {
        terms: CommercialTerms {
            version: 1,
            operator_pubkey: operator.public_key().to_hex(),
            payment_destination: "attacker@example.test".into(),
            fee: FeePolicy::None,
            limitations: vec![],
        },
    })
    .expect("shape only")
    .custom_created_at(Timestamp::from(10))
    .sign_with_keys(&attacker)
    .expect("attacker signature");
    let (context, mut servers, mut refs) = setup(&root);
    servers[0].operator_pubkey = operator.public_key().to_hex();
    refs[0].operator_pubkey = operator.public_key().to_hex();
    assert!(review_publisher_terms(&context, &servers, &refs, &[root], true, 100).is_err());
}

#[test]
fn publisher_fee_preview_floors_basis_points_and_applies_maximum_exactly() {
    let root = offer(
        &Keys::generate(),
        FeePolicy::BasisPoints {
            basis_points: 125,
            minimum_msat: None,
            maximum_msat: Some(30_000),
        },
    );
    let (mut context, servers, refs) = setup(&root);
    context.price_sats = 101;
    let review = review_publisher_terms(&context, &servers, &refs, &[root.clone()], true, 100)
        .expect("floor");
    assert_eq!(review.legs[0].amount_msat, 1262);
    context.price_sats = 10_000;
    let review =
        review_publisher_terms(&context, &servers, &refs, &[root], true, 100).expect("maximum");
    assert_eq!(review.legs[0].amount_msat, 30_000);
}

#[test]
fn publisher_withdrawal_blocks_new_acceptance_without_rewriting_historical_evidence() {
    let operator = Keys::generate();
    let root = offer(&operator, FeePolicy::None);
    let (context, servers, refs) = setup(&root);
    let original = root.as_json();
    let withdrawal = build_terms_event_builder(&TermsPayload::Withdraw {
        version: 1,
        terms_event_id: root.id.to_hex(),
    })
    .expect("withdrawal")
    .custom_created_at(Timestamp::from(50))
    .sign_with_keys(&operator)
    .expect("signed");
    assert!(review_publisher_terms(
        &context,
        &servers,
        &refs,
        &[root.clone(), withdrawal.clone()],
        true,
        100
    )
    .is_err());
    assert!(review_publisher_terms(
        &context,
        &servers,
        &refs,
        &[root.clone(), withdrawal],
        false,
        100
    )
    .is_err());
    assert!(parse_terms_event(&root).is_ok());
    assert_eq!(root.as_json(), original);
}

#[test]
fn publisher_unrelated_or_forged_withdrawal_cannot_retire_an_offer() {
    let operator = Keys::generate();
    let root = offer(&operator, FeePolicy::None);
    let (context, servers, refs) = setup(&root);
    let withdrawal = build_terms_event_builder(&TermsPayload::Withdraw {
        version: 1,
        terms_event_id: root.id.to_hex(),
    })
    .expect("withdrawal")
    .custom_created_at(Timestamp::from(50))
    .sign_with_keys(&Keys::generate())
    .expect("signed");
    assert!(
        review_publisher_terms(&context, &servers, &refs, &[root, withdrawal], true, 100).is_ok()
    );
}

#[test]
fn publisher_preserve_uses_exact_acceptance_not_the_latest_advertisement() {
    let operator = Keys::generate();
    let root = offer(&operator, FeePolicy::None);
    let replacement = offer(
        &operator,
        FeePolicy::Fixed {
            amount_msat: 900_000,
        },
    );
    let (context, mut servers, refs) = setup(&root);
    let source = listing(&Keys::generate(), &refs);
    let before = source.as_json();
    servers[0].advertised_terms = Some(replacement.id.to_hex());
    let preserved =
        publisher_commercial_references(&PublisherCommercialIntent::Preserve, Some(&source), None)
            .expect("preserved");
    assert_eq!(preserved, refs);
    assert!(
        review_publisher_terms(&context, &servers, &preserved, &[root.clone()], false, 100).is_ok()
    );
    assert!(review_publisher_terms(&context, &servers, &preserved, &[root], true, 100).is_err());
    assert_eq!(source.as_json(), before);
    assert_eq!(
        PublisherCommercialIntent::default(),
        PublisherCommercialIntent::Preserve
    );
}

#[test]
fn publisher_replacement_approval_binds_price_destination_roots_and_allocation() {
    let root = offer(&Keys::generate(), FeePolicy::None);
    let (context, servers, refs) = setup(&root);
    let review =
        review_publisher_terms(&context, &servers, &refs, &[root], true, 100).expect("review");
    let intent = PublisherCommercialIntent::ReplaceAfterReview {
        review: review.clone(),
    };
    assert_eq!(
        publisher_commercial_references(&intent, None, Some(&review)).expect("approved"),
        refs
    );
    assert!(publisher_commercial_references(&intent, None, None).is_err());
    let mut changed = review.clone();
    changed.context.price_sats += 1;
    assert!(publisher_commercial_references(&intent, None, Some(&changed)).is_err());
    let mut changed = review.clone();
    changed.context.developer_destination = "elsewhere@example.test".into();
    assert!(publisher_commercial_references(&intent, None, Some(&changed)).is_err());
    let mut changed = review.clone();
    changed.accepted_terms[0].terms_event_id = "ab".repeat(32);
    assert!(publisher_commercial_references(&intent, None, Some(&changed)).is_err());
    let mut changed = review;
    changed.legs[0].amount_msat -= 1;
    assert!(publisher_commercial_references(&intent, None, Some(&changed)).is_err());
}

#[test]
fn publisher_rejects_incompatible_missing_duplicate_and_noncanonical_coverage() {
    let root = offer(&Keys::generate(), FeePolicy::None);
    let (context, mut servers, refs) = setup(&root);
    for version in [0, 2] {
        servers[0].version = version;
        assert!(
            review_publisher_terms(&context, &servers, &refs, &[root.clone()], true, 100).is_err()
        );
    }
    servers[0].version = 1;
    servers[0].split_enabled = false;
    assert!(review_publisher_terms(&context, &servers, &refs, &[root.clone()], true, 100).is_err());
    servers[0].split_enabled = true;
    assert!(review_publisher_terms(&context, &servers, &[], &[root.clone()], true, 100).is_err());
    assert!(review_publisher_terms(
        &context,
        &servers,
        &[refs[0].clone(), refs[0].clone()],
        &[root.clone()],
        true,
        100
    )
    .is_err());
    let mut upper = refs;
    upper[0].terms_event_id.make_ascii_uppercase();
    assert!(review_publisher_terms(&context, &servers, &upper, &[root], false, 100).is_err());
}

#[test]
fn publisher_invalid_allocations_bounds_and_overflow_fail_before_consent() {
    for fee in [
        FeePolicy::Fixed { amount_msat: 0 },
        FeePolicy::Fixed {
            amount_msat: 1_000_000,
        },
        FeePolicy::BasisPoints {
            basis_points: 1,
            minimum_msat: Some(2),
            maximum_msat: Some(1),
        },
        FeePolicy::BasisPoints {
            basis_points: u64::MAX,
            minimum_msat: None,
            maximum_msat: None,
        },
    ] {
        let root = offer(&Keys::generate(), fee);
        let (context, servers, refs) = setup(&root);
        assert!(review_publisher_terms(&context, &servers, &refs, &[root], true, 100).is_err());
    }
    let root = offer(&Keys::generate(), FeePolicy::None);
    let (mut context, servers, refs) = setup(&root);
    context.price_sats = u64::MAX;
    assert!(review_publisher_terms(&context, &servers, &refs, &[root], true, 100).is_err());
}

#[test]
fn publisher_retirement_is_explicitly_unavailable_and_preserves_source() {
    let root = offer(&Keys::generate(), FeePolicy::None);
    let (_, _, refs) = setup(&root);
    let source = listing(&Keys::generate(), &refs);
    let before = source.as_json();
    assert!(publisher_commercial_references(
        &PublisherCommercialIntent::Retire,
        Some(&source),
        None
    )
    .is_err());
    assert_eq!(source.as_json(), before);
    assert_eq!(listing_accepted_terms(&source).expect("unchanged"), refs);
}

#[test]
fn publisher_replacement_can_change_an_offer_but_cannot_bypass_operator_retirement() {
    let operator = Keys::generate();
    let old = offer(&operator, FeePolicy::None);
    let new = offer(&operator, FeePolicy::Fixed { amount_msat: 1000 });
    let (context, servers, refs) = setup(&new);
    let review =
        review_publisher_terms(&context, &servers, &refs, &[new], true, 100).expect("new review");
    let intent = PublisherCommercialIntent::ReplaceAfterReview {
        review: review.clone(),
    };
    let (_, _, old_refs) = setup(&old);
    let source = listing(&Keys::generate(), &old_refs);
    assert_eq!(
        publisher_commercial_references(&intent, Some(&source), Some(&review))
            .expect("same operator, reviewed root"),
        refs
    );
    assert_eq!(
        listing_accepted_terms(&source).expect("history unchanged"),
        old_refs
    );
    let retired_root = offer(&Keys::generate(), FeePolicy::None);
    let (_, _, retired_refs) = setup(&retired_root);
    let source = listing(&Keys::generate(), &retired_refs);
    assert!(publisher_commercial_references(&intent, Some(&source), Some(&review)).is_err());
    assert_eq!(
        listing_accepted_terms(&source).expect("not retired"),
        retired_refs
    );
}

#[test]
fn publisher_public_access_preservation_never_creates_payment_legs() {
    let root = offer(
        &Keys::generate(),
        FeePolicy::Fixed {
            amount_msat: 50_000,
        },
    );
    let (mut context, servers, refs) = setup(&root);
    context.price_sats = 0;
    context.developer_destination.clear();
    let review = review_publisher_terms(&context, &servers, &refs, &[root.clone()], false, 100)
        .expect("preserve");
    assert!(review.legs.is_empty());
    assert!(review_publisher_terms(&context, &servers, &refs, &[root], true, 100).is_err());
}

#[test]
fn publisher_withdrawal_and_retirement_do_not_reinterpret_a_completed_sale_snapshot() {
    let developer = Keys::generate();
    let operator = Keys::generate();
    let root = offer(
        &operator,
        FeePolicy::Fixed {
            amount_msat: 10_000,
        },
    );
    let (mut context, servers, refs) = setup(&root);
    context.publisher_pubkey = developer.public_key().to_hex();
    let source = listing(&developer, &refs);
    let review = review_publisher_terms(&context, &servers, &refs, &[root.clone()], true, 20)
        .expect("original review");
    let quote = SignedQuote::new(
        "historical-sale".into(),
        Keys::generate().public_key().to_hex(),
        source.id.to_hex(),
        format!("30402:{}:game", developer.public_key().to_hex()),
        operator.public_key().to_hex(),
        1_000_000,
        20,
        &RefundDestination {
            endpoint: "buyer@example.test".into(),
            min_sendable_msat: 1,
            max_sendable_msat: 1_000_000,
        },
        review.legs,
    )
    .expect("original quote");
    let original_quote = serde_json::to_string(&quote).expect("snapshot");
    let withdrawal = build_terms_event_builder(&TermsPayload::Withdraw {
        version: 1,
        terms_event_id: root.id.to_hex(),
    })
    .expect("withdrawal")
    .custom_created_at(Timestamp::from(50))
    .sign_with_keys(&operator)
    .expect("signed");
    assert!(review_publisher_terms(
        &context,
        &servers,
        &refs,
        &[root.clone(), withdrawal],
        true,
        100
    )
    .is_err());
    assert!(publisher_commercial_references(
        &PublisherCommercialIntent::Retire,
        Some(&source),
        None
    )
    .is_err());
    verify_quote_listing_terms(&quote, &source, &[root])
        .expect("original sale evidence still verifies");
    assert_eq!(
        serde_json::to_string(&quote).expect("snapshot"),
        original_quote
    );
}
