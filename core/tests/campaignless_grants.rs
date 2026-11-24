#![cfg(feature = "native")]
//! ADP-01 §Entitlement Grants (gaps C10, C11): publisher-signed grants issued
//! without a campaign are discovered by recipient (`#p`), validated, stored,
//! and revoked only by the publisher. Relay evidence counts only positively.
use arcadestr_core::adp_protocol::ENTITLEMENT_GRANT_KIND;
use arcadestr_core::entitlement_reconciliation::{EntitlementEvidence, EntitlementSource};
use arcadestr_core::entitlements_repository::EntitlementsRepository;
use arcadestr_core::storage::Database;
use async_trait::async_trait;
use nostr::{Event, EventBuilder, EventId, Filter, Keys, Kind, Tag, Timestamp};
use std::sync::Mutex;
use tempfile::TempDir;

fn tag(values: &[&str]) -> Tag {
    Tag::parse(values.iter().copied()).expect("tag")
}

/// Scripted relay answers. Each query applies the filter, as a relay would.
struct Source {
    events: Mutex<Vec<Event>>,
    incomplete: bool,
    unavailable: bool,
}

impl Source {
    fn new(events: Vec<Event>) -> Self {
        Self {
            events: Mutex::new(events),
            incomplete: false,
            unavailable: false,
        }
    }
    fn push(&self, event: Event) {
        self.events.lock().expect("events").push(event);
    }
}

#[async_trait]
impl EntitlementSource for Source {
    async fn query(&self, filter: Filter) -> Result<EntitlementEvidence, String> {
        if self.unavailable {
            return Err("no relay answered".into());
        }
        let events = self
            .events
            .lock()
            .expect("events")
            .iter()
            .filter(|event| filter.match_event(event, Default::default()))
            .cloned()
            .collect();
        Ok(EntitlementEvidence {
            events,
            incomplete: self.incomplete,
        })
    }
}

struct Fixture {
    _temp: TempDir,
    repo: EntitlementsRepository,
    publisher: Keys,
    buyer: Keys,
    coordinate: String,
}

async fn fixture() -> Fixture {
    let temp = TempDir::new().expect("temp");
    let db = Database::new(&temp.path().join("grants.sqlite"))
        .await
        .expect("db");
    let publisher = Keys::generate();
    Fixture {
        repo: EntitlementsRepository::new(db.pool().clone()),
        coordinate: format!("30402:{}:game", publisher.public_key().to_hex()),
        publisher,
        buyer: Keys::generate(),
        _temp: temp,
    }
}

impl Fixture {
    fn root(&self, signer: &Keys, grant_id: &str, at: u64) -> Event {
        self.root_with(signer, grant_id, at, vec![])
    }

    fn root_with(&self, signer: &Keys, grant_id: &str, at: u64, extra: Vec<Tag>) -> Event {
        let mut tags = vec![
            tag(&["d", grant_id]),
            tag(&["p", &self.buyer.public_key().to_hex()]),
            tag(&["a", &self.coordinate]),
            tag(&["reason", "review-copy"]),
            tag(&["status", "granted"]),
        ];
        tags.extend(extra);
        EventBuilder::new(Kind::Custom(ENTITLEMENT_GRANT_KIND), "")
            .tags(tags)
            .custom_created_at(Timestamp::from_secs(at))
            .sign_with_keys(signer)
            .expect("grant")
    }

    fn revoke(&self, signer: &Keys, root: &Event, grant_id: &str, at: u64) -> Event {
        EventBuilder::new(Kind::Custom(ENTITLEMENT_GRANT_KIND), "")
            .tags([
                tag(&["d", grant_id]),
                tag(&["p", &self.buyer.public_key().to_hex()]),
                tag(&["a", &self.coordinate]),
                tag(&["e", &root.id.to_hex()]),
                tag(&["status", "revoked"]),
            ])
            .custom_created_at(Timestamp::from_secs(at))
            .sign_with_keys(signer)
            .expect("revocation")
    }

    async fn discover(
        &self,
        source: &Source,
    ) -> arcadestr_core::entitlement_reconciliation::CampaignlessDiscoveryReport {
        self.repo
            .discover_campaignless_grants(self.buyer.public_key(), source, &|| Ok(()))
            .await
            .expect("discovery")
    }

    async fn owned(&self) -> bool {
        self.repo
            .is_owned(&self.buyer.public_key().to_hex(), &self.coordinate)
            .await
            .expect("ownership")
    }

    async fn stored_ids(&self) -> Vec<EventId> {
        let mut ids = self
            .repo
            .chains_for(&self.buyer.public_key().to_hex(), &self.coordinate)
            .await
            .expect("chains")
            .into_iter()
            .flat_map(|chain| chain.events.into_iter().map(|node| node.event.id))
            .collect::<Vec<_>>();
        ids.sort();
        ids
    }
}

#[tokio::test]
async fn a_valid_campaignless_grant_is_discovered_and_owned() {
    let f = fixture().await;
    let source = Source::new(vec![f.root(&f.publisher, "review-copy", 100)]);
    let report = f.discover(&source).await;
    assert_eq!((report.examined, report.discovered), (1, 1));
    assert!(f.owned().await);
    // Discovery is idempotent.
    let again = f.discover(&source).await;
    assert_eq!(again.discovered, 0);
    assert!(f.owned().await);
}

#[tokio::test]
async fn a_grant_found_with_its_revocation_is_stored_as_revoked() {
    let f = fixture().await;
    let root = f.root(&f.publisher, "review-copy", 100);
    let revoke = f.revoke(&f.publisher, &root, "review-copy", 200);
    let source = Source::new(vec![root, revoke]);
    let report = f.discover(&source).await;
    assert_eq!(report.discovered, 1);
    assert_eq!(report.revoked, 0, "it was never owned locally");
    assert!(!f.owned().await);
}

#[tokio::test]
async fn a_later_publisher_revocation_ends_ownership() {
    let f = fixture().await;
    let root = f.root(&f.publisher, "review-copy", 100);
    let source = Source::new(vec![root.clone()]);
    f.discover(&source).await;
    assert!(f.owned().await);
    source.push(f.revoke(&f.publisher, &root, "review-copy", 200));
    let report = f.discover(&source).await;
    assert_eq!(report.revoked, 1);
    assert!(!f.owned().await);
    // Terminal: a later "re-grant" successor is not a valid transition.
    let mut regrant = f.root(&f.publisher, "review-copy", 300).tags.to_vec();
    regrant.push(tag(&["e", &root.id.to_hex()]));
    let regrant = EventBuilder::new(Kind::Custom(ENTITLEMENT_GRANT_KIND), "")
        .tags(regrant)
        .custom_created_at(Timestamp::from_secs(300))
        .sign_with_keys(&f.publisher)
        .expect("regrant");
    source.push(regrant);
    f.discover(&source).await;
    assert!(!f.owned().await);
}

#[tokio::test]
async fn a_revocation_signed_by_another_key_is_ignored_and_not_stored() {
    let f = fixture().await;
    let root = f.root(&f.publisher, "review-copy", 100);
    let attacker = Keys::generate();
    let forged = f.revoke(&attacker, &root, "review-copy", 200);
    let source = Source::new(vec![root.clone(), forged.clone()]);
    f.discover(&source).await;
    assert!(f.owned().await);
    assert_eq!(f.stored_ids().await, vec![root.id]);
    assert!(f.repo.ingest_campaignless_event(&forged).await.is_err());
}

#[tokio::test]
async fn a_grant_not_signed_by_the_coordinate_publisher_is_rejected() {
    let f = fixture().await;
    let source = Source::new(vec![f.root(&Keys::generate(), "forged", 100)]);
    let report = f.discover(&source).await;
    assert_eq!((report.rejected_chains, report.discovered), (1, 0));
    assert!(!f.owned().await);
}

#[tokio::test]
async fn a_delegated_style_grant_without_a_campaign_is_rejected() {
    // A campaignless grant must not carry an authorization anchor: delegated
    // issuance needs an independently verifiable policy (a campaign).
    let f = fixture().await;
    let anchored = f.root_with(
        &f.publisher,
        "delegated",
        100,
        vec![tag(&["authorization", &"ab".repeat(32)])],
    );
    let report = f.discover(&Source::new(vec![anchored])).await;
    assert_eq!(report.rejected_chains, 1);
    assert!(!f.owned().await);
}

#[tokio::test]
async fn forks_and_duplicate_roots_fail_closed() {
    let f = fixture().await;
    let root = f.root(&f.publisher, "review-copy", 100);
    let fork = Source::new(vec![
        root.clone(),
        f.revoke(&f.publisher, &root, "review-copy", 200),
        f.revoke(&f.publisher, &root, "review-copy", 201),
    ]);
    assert_eq!(f.discover(&fork).await.rejected_chains, 1);
    assert!(!f.owned().await);

    let two_roots = Source::new(vec![
        f.root(&f.publisher, "twin", 100),
        f.root(&f.publisher, "twin", 101),
    ]);
    assert_eq!(f.discover(&two_roots).await.rejected_chains, 1);
    assert!(!f.owned().await);
}

#[tokio::test]
async fn malformed_events_are_counted_and_ignored() {
    let f = fixture().await;
    let good = f.root(&f.publisher, "review-copy", 100);
    let mut tampered = f.root(&f.publisher, "tampered", 100);
    tampered.content = "altered after signing".into();
    let report = f.discover(&Source::new(vec![good, tampered])).await;
    assert_eq!(report.malformed_events, 1);
    assert_eq!(report.discovered, 1);
}

#[tokio::test]
async fn incomplete_evidence_still_accepts_found_grants() {
    let f = fixture().await;
    let mut source = Source::new(vec![f.root(&f.publisher, "review-copy", 100)]);
    source.incomplete = true;
    let report = f.discover(&source).await;
    assert!(report.incomplete);
    assert_eq!(report.discovered, 1, "positive evidence is self-validating");
    assert!(f.owned().await);
}

#[tokio::test]
async fn unavailable_relays_change_nothing() {
    let f = fixture().await;
    let source = Source::new(vec![f.root(&f.publisher, "review-copy", 100)]);
    f.discover(&source).await;
    let mut offline = Source::new(vec![]);
    offline.unavailable = true;
    let report = f.discover(&offline).await;
    assert!(report.unavailable && report.incomplete);
    assert!(f.owned().await, "absence of evidence never removes a grant");
    // An answering relay that simply lacks the grant also removes nothing.
    f.discover(&Source::new(vec![])).await;
    assert!(f.owned().await);
}

#[tokio::test]
async fn campaign_anchored_and_foreign_recipient_grants_are_ignored() {
    let f = fixture().await;
    let anchored = f.root_with(
        &f.publisher,
        "claimed",
        100,
        vec![tag(&["source_event", &"cd".repeat(32)])],
    );
    let other_buyer = EventBuilder::new(Kind::Custom(ENTITLEMENT_GRANT_KIND), "")
        .tags([
            tag(&["d", "theirs"]),
            tag(&["p", &Keys::generate().public_key().to_hex()]),
            tag(&["a", &f.coordinate]),
            tag(&["status", "granted"]),
        ])
        .custom_created_at(Timestamp::from_secs(100))
        .sign_with_keys(&f.publisher)
        .expect("other grant");
    let report = f.discover(&Source::new(vec![anchored, other_buyer])).await;
    assert_eq!((report.examined, report.discovered), (0, 0));
    assert!(!f.owned().await);
}
