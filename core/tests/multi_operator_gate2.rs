//! Non-paying Gate 2 domain/discovery checks. These synthetic keys are NOT the
//! existing live Alice/Bob/Carol/Dave identities and are never published.
use arcadestr_core::{adp_discovery::parse_adp_server_announcements, adp_settlement::*};
use nostr::{Event, EventBuilder, Keys, Kind, Tag};

fn announcement(keys: &Keys, url: &str) -> Event {
    EventBuilder::new(Kind::Custom(30403), "")
        .tags([
            Tag::parse(["d", "gate2-source"]).expect("d"),
            Tag::parse(["url", url]).expect("URL"),
            Tag::parse(["settlement", "1", "split"]).expect("mode"),
        ])
        .sign_with_keys(keys)
        .expect("announcement")
}
fn offer(keys: &Keys, fee: u64) -> (CommercialTerms, String) {
    let terms = CommercialTerms {
        version: SETTLEMENT_VERSION,
        operator_pubkey: keys.public_key().to_hex(),
        payment_destination: "operator@example.com".into(),
        fee: FeePolicy::Fixed { amount_msat: fee },
        limitations: vec![],
    };
    let event = build_terms_event_builder(&TermsPayload::Offer {
        terms: terms.clone(),
    })
    .expect("terms")
    .sign_with_keys(keys)
    .expect("sign");
    assert_eq!(
        parse_terms_event(&event).expect("verify terms"),
        TermsPayload::Offer {
            terms: terms.clone()
        }
    );
    (terms, event.id.to_hex())
}
#[test]
fn missing_or_invalid_second_announcement_is_not_discovered_or_accepted_as_coverage() {
    let a = Keys::generate();
    let b = Keys::generate();
    let event_a = announcement(&a, "http://127.0.0.1:19099");
    let event_b = announcement(&b, "http://127.0.0.1:19100");
    let accepted = vec![
        AcceptedTermsRef {
            operator_pubkey: a.public_key().to_hex(),
            terms_event_id: "01".repeat(32),
        },
        AcceptedTermsRef {
            operator_pubkey: b.public_key().to_hex(),
            terms_event_id: "02".repeat(32),
        },
    ];
    let mut corrupted = serde_json::to_value(&event_b).expect("JSON");
    corrupted["sig"] = serde_json::json!("00".repeat(64));
    let invalid: Event = serde_json::from_value(corrupted).expect("event");
    for events in [vec![event_a.clone()], vec![event_a.clone(), invalid]] {
        let discovered = parse_adp_server_announcements(events);
        assert_eq!(discovered.len(), 1);
        assert!(!discovered
            .iter()
            .any(|s| s.pubkey == b.public_key().to_hex()));
        let servers: Vec<_> = discovered
            .into_iter()
            .map(|s| ServerOperator {
                url: s.url,
                operator_pubkey: s.pubkey,
            })
            .collect();
        assert!(validate_terms_coverage(&servers, &accepted).is_err());
    }
    let discovered = parse_adp_server_announcements(vec![event_a, event_b]);
    assert_eq!(discovered.len(), 2);
    let servers: Vec<_> = discovered
        .into_iter()
        .map(|s| ServerOperator {
            url: s.url,
            operator_pubkey: s.pubkey,
        })
        .collect();
    assert_eq!(
        validate_terms_coverage(&servers, &accepted)
            .expect("complete coverage")
            .len(),
        2
    );
}
#[test]
fn two_independently_signed_fee_roots_produce_exactly_three_canonical_legs() {
    let a = Keys::generate();
    let b = Keys::generate();
    let developer = Keys::generate();
    let operators = vec![offer(&a, 100_000), offer(&b, 150_000)];
    let legs = build_payment_legs(
        1_000_000,
        &developer.public_key().to_hex(),
        "developer@example.com",
        &operators,
    )
    .expect("three legs");
    assert_eq!(legs.len(), 3);
    assert_eq!(
        legs.iter().map(|l| l.amount_msat).collect::<Vec<_>>(),
        vec![100_000, 150_000, 750_000]
    );
    assert_eq!(legs[0].accepted_terms_id, Some(operators[0].1.clone()));
    assert_eq!(legs[1].accepted_terms_id, Some(operators[1].1.clone()));
    assert!(legs[2].accepted_terms_id.is_none());
    validate_leg_set(1_000_000, &legs).expect("exact sum/order");
    let mut duplicate = legs.clone();
    duplicate[1].leg_id = duplicate[0].leg_id.clone();
    assert!(validate_leg_set(1_000_000, &duplicate).is_err());
    let mut reversed = legs;
    reversed.reverse();
    assert!(validate_leg_set(1_000_000, &reversed).is_err());
}
#[test]
fn missing_duplicate_and_conflicting_acceptances_cannot_cover_two_operators() {
    let a = Keys::generate();
    let b = Keys::generate();
    let servers = vec![
        ServerOperator {
            url: "http://127.0.0.1:19099".into(),
            operator_pubkey: a.public_key().to_hex(),
        },
        ServerOperator {
            url: "http://127.0.0.1:19100".into(),
            operator_pubkey: b.public_key().to_hex(),
        },
    ];
    let first = AcceptedTermsRef {
        operator_pubkey: a.public_key().to_hex(),
        terms_event_id: "01".repeat(32),
    };
    assert!(validate_terms_coverage(&servers, std::slice::from_ref(&first)).is_err());
    assert!(validate_terms_coverage(&servers, &[first.clone(), first.clone()]).is_err());
    let other = AcceptedTermsRef {
        operator_pubkey: first.operator_pubkey.clone(),
        terms_event_id: "03".repeat(32),
    };
    assert!(validate_terms_coverage(&servers, &[first, other]).is_err());
}
