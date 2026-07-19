#![cfg(feature = "native")]

use arcadestr_core::adp_protocol::ADP_CAMPAIGN_KIND;
use arcadestr_core::campaign_discovery::{
    resolve_campaign_candidates, resolve_campaign_candidates_report, CampaignClassification,
    CampaignDiscoveryResult,
};
use arcadestr_core::marketplace::CampaignPointer;
use nostr::{Event, EventBuilder, EventId, Keys, Kind, Tag, Timestamp};

fn tag(values: &[&str]) -> Tag {
    Tag::parse(values.iter().copied()).expect("test tag must be valid")
}

fn campaign(
    publisher: &Keys,
    coordinate: &str,
    id: &str,
    predecessor: Option<EventId>,
    status: &str,
    created_at: u64,
) -> Event {
    let mut tags = vec![
        tag(&["d", id]),
        tag(&["a", coordinate]),
        tag(&["status", status]),
    ];
    if let Some(predecessor) = predecessor {
        tags.push(tag(&["e", &predecessor.to_hex()]));
    }
    if status == "active" {
        tags.extend([
            tag(&["mode", "claim"]),
            tag(&["starts", "120"]),
            tag(&["ends", "300"]),
        ]);
    }
    EventBuilder::new(Kind::Custom(ADP_CAMPAIGN_KIND), "")
        .tags(tags)
        .custom_created_at(Timestamp::from_secs(created_at))
        .sign_with_keys(publisher)
        .expect("campaign must sign")
}

#[test]
fn pointer_candidate_discovers_complete_campaign_chain() {
    let publisher = Keys::generate();
    let coordinate = format!("30402:{}:game", publisher.public_key());
    let root = campaign(&publisher, &coordinate, "launch", None, "active", 100);
    let cancel = campaign(
        &publisher,
        &coordinate,
        "launch",
        Some(root.id),
        "cancelled",
        180,
    );
    let pointers = [CampaignPointer {
        root_event_id: root.id,
        relay_hint: Some("wss://relay.example.com".into()),
    }];

    let discovered = resolve_campaign_candidates(
        &pointers,
        &[root.clone()],
        &[],
        &[root, cancel],
        publisher.public_key(),
        &coordinate,
        170,
    );

    assert!(matches!(
        discovered.as_slice(),
        [CampaignDiscoveryResult {
            classification: CampaignClassification::Active,
            ..
        }]
    ));
    assert_eq!(discovered[0].campaign.events.len(), 2);
}

#[test]
fn coordinate_fallback_discovers_campaign_without_pointer() {
    let publisher = Keys::generate();
    let coordinate = format!("30402:{}:game", publisher.public_key());
    let root = campaign(&publisher, &coordinate, "launch", None, "active", 100);

    let discovered = resolve_campaign_candidates(
        &[],
        &[],
        &[root.clone()],
        &[root],
        publisher.public_key(),
        &coordinate,
        110,
    );

    assert_eq!(discovered.len(), 1);
    assert_eq!(
        discovered[0].classification,
        CampaignClassification::Upcoming
    );
}

#[test]
fn stale_pointer_does_not_block_valid_fallback_campaign() {
    let publisher = Keys::generate();
    let coordinate = format!("30402:{}:game", publisher.public_key());
    let fallback = campaign(&publisher, &coordinate, "fallback", None, "active", 100);
    let pointers = [CampaignPointer {
        root_event_id: EventId::from_hex(
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        )
        .expect("synthetic id"),
        relay_hint: None,
    }];

    let discovered = resolve_campaign_candidates(
        &pointers,
        &[],
        &[fallback.clone()],
        &[fallback],
        publisher.public_key(),
        &coordinate,
        150,
    );

    assert_eq!(discovered.len(), 1);
    assert_eq!(discovered[0].classification, CampaignClassification::Active);
}

#[test]
fn ended_and_cancelled_campaigns_are_classified_from_chain_state() {
    let publisher = Keys::generate();
    let coordinate = format!("30402:{}:game", publisher.public_key());
    let ended = campaign(&publisher, &coordinate, "ended", None, "active", 100);
    let cancelled = campaign(&publisher, &coordinate, "cancelled", None, "active", 100);
    let cancellation = campaign(
        &publisher,
        &coordinate,
        "cancelled",
        Some(cancelled.id),
        "cancelled",
        180,
    );

    let discovered = resolve_campaign_candidates(
        &[],
        &[],
        &[ended.clone(), cancelled.clone()],
        &[ended, cancelled, cancellation],
        publisher.public_key(),
        &coordinate,
        350,
    );

    assert!(discovered
        .iter()
        .any(|item| item.classification == CampaignClassification::Ended));
    assert!(discovered
        .iter()
        .any(|item| item.classification == CampaignClassification::Cancelled));
}

#[test]
fn forked_campaign_is_reported_invalid_without_hiding_valid_campaigns() {
    let publisher = Keys::generate();
    let coordinate = format!("30402:{}:game", publisher.public_key());
    let valid = campaign(&publisher, &coordinate, "valid", None, "active", 100);
    let forked = campaign(&publisher, &coordinate, "forked", None, "active", 100);
    let first = campaign(
        &publisher,
        &coordinate,
        "forked",
        Some(forked.id),
        "cancelled",
        180,
    );
    let second = campaign(
        &publisher,
        &coordinate,
        "forked",
        Some(forked.id),
        "cancelled",
        190,
    );

    let report = resolve_campaign_candidates_report(
        &[],
        &[],
        &[valid.clone(), forked.clone()],
        &[valid, forked, first, second],
        publisher.public_key(),
        &coordinate,
        150,
    );

    assert_eq!(report.campaigns.len(), 1);
    assert_eq!(report.invalid_campaign_ids, vec!["forked"]);
}
