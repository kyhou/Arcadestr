//! Arcadestr checkout policy for signed paid-listing evidence before
//! irreversible payments. This is local client policy, not an ADP-01
//! requirement, and never a claim of global relay consensus or completeness.
//!
//! Policy:
//! - Coverage: at least a majority of configured relays (and at least one) must
//!   complete the listing query with EOSE. Otherwise evidence is insufficient.
//! - Candidates: correctly signed kind:30402 revisions for exactly this
//!   coordinate, from completing relays plus revisions this device already
//!   retained (a withholding relay cannot roll back to an older revision).
//! - Selection: NIP-01 replaceable ordering (newest `created_at`, then lowest id).
//!   A different revision with the same timestamp as the selected one is an
//!   ambiguous conflict and is rejected for payments.
//! - A pinned (approved) revision is current only if it is the selected one and
//!   at least one completing relay returned it now.
use nostr::{Event, EventId, Kind, PublicKey};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CheckoutFreshnessPolicy {
    pub minimum_completed_relays: usize,
}

impl CheckoutFreshnessPolicy {
    /// Majority of configured relays, at least one.
    pub fn for_configured_relays(configured: usize) -> Self {
        Self {
            minimum_completed_relays: (configured / 2 + 1).max(1),
        }
    }
}

/// Listing query results per relay. `completed` maps relay URL to the events it
/// returned before EOSE; relays that failed, timed out or were disconnected are
/// listed in `incomplete`. Valid events delivered by incomplete relays are kept
/// in `partial_events`: they never count as coverage but can still reveal a newer
/// or conflicting revision.
#[derive(Debug, Clone, Default)]
pub struct RelayCoverage {
    pub completed: BTreeMap<String, Vec<Event>>,
    pub incomplete: Vec<String>,
    pub partial_events: Vec<Event>,
}

impl RelayCoverage {
    /// Restricts coverage to the trusted relay set (ADP-01 §Relay availability).
    ///
    /// Relays outside `trusted` (for example NIP-65 relays discovered from
    /// another user's relay list) are where to look, not evidence about
    /// completeness: their events are kept as positive evidence in
    /// `partial_events`, so a newer or conflicting revision they hold still
    /// fails closed, but they never count toward or against the quorum.
    pub fn restricted_to(mut self, trusted: &std::collections::HashSet<String>) -> Self {
        let untrusted = self
            .completed
            .keys()
            .filter(|url| !trusted.contains(*url))
            .cloned()
            .collect::<Vec<_>>();
        for url in untrusted {
            if let Some(events) = self.completed.remove(&url) {
                self.partial_events.extend(events);
            }
        }
        self.incomplete.retain(|url| trusted.contains(url));
        self
    }

    pub fn configured(&self) -> usize {
        self.completed.len() + self.incomplete.len()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ListingFreshness {
    Current(Box<Event>),
    InsufficientEvidence { completed: usize, required: usize },
    NotFound,
    PinnedNotObserved,
    Superseded { pinned: EventId, newer: EventId },
    Conflicting { a: EventId, b: EventId },
}

impl ListingFreshness {
    pub fn describe(&self) -> String {
        match self {
            Self::Current(event) => format!("current revision {}", event.id),
            Self::InsufficientEvidence { completed, required } => format!(
                "insufficient relay evidence: {completed} relay(s) completed the listing query, {required} required"
            ),
            Self::NotFound => "no signed listing revision was found".into(),
            Self::PinnedNotObserved => {
                "no completing relay currently returns the approved listing revision".into()
            }
            Self::Superseded { newer, .. } => {
                format!("the listing was revised ({newer}); review and approve the new terms")
            }
            Self::Conflicting { .. } => {
                "conflicting listing revisions share the same timestamp".into()
            }
        }
    }
}

fn coordinate_parts(coordinate: &str) -> Option<(PublicKey, &str)> {
    let mut parts = coordinate.splitn(3, ':');
    if parts.next()? != "30402" {
        return None;
    }
    let author = PublicKey::from_hex(parts.next()?).ok()?;
    let identifier = parts.next().filter(|d| !d.is_empty())?;
    Some((author, identifier))
}

fn is_revision_of(event: &Event, author: PublicKey, identifier: &str) -> bool {
    let identifiers = event
        .tags
        .iter()
        .filter(|tag| tag.as_slice().first().is_some_and(|name| name == "d"))
        .collect::<Vec<_>>();
    event.kind == Kind::Custom(30402)
        && event.pubkey == author
        && identifiers.len() == 1
        && identifiers[0].as_slice().get(1).map(String::as_str) == Some(identifier)
        && event.verify().is_ok()
}

/// NIP-01 replaceable ordering: newer first, ties by lowest id.
fn newer_first(a: &Event, b: &Event) -> std::cmp::Ordering {
    b.created_at
        .cmp(&a.created_at)
        .then_with(|| a.id.cmp(&b.id))
}

pub fn evaluate_checkout_listing(
    coordinate: &str,
    pinned: Option<EventId>,
    coverage: &RelayCoverage,
    retained: &[Event],
    policy: CheckoutFreshnessPolicy,
) -> ListingFreshness {
    let Some((author, identifier)) = coordinate_parts(coordinate) else {
        return ListingFreshness::NotFound;
    };
    let completed = coverage.completed.len();
    let required = policy.minimum_completed_relays.max(1);
    if completed < required {
        return ListingFreshness::InsufficientEvidence {
            completed,
            required,
        };
    }
    let observed = coverage
        .completed
        .values()
        .flatten()
        .filter(|event| is_revision_of(event, author, identifier))
        .collect::<Vec<_>>();
    let mut candidates = observed
        .iter()
        .copied()
        .chain(
            coverage
                .partial_events
                .iter()
                .chain(retained)
                .filter(|e| is_revision_of(e, author, identifier)),
        )
        .collect::<Vec<_>>();
    candidates.sort_by(|a, b| newer_first(a, b));
    candidates.dedup_by(|a, b| a.id == b.id);
    let Some(selected) = candidates.first().copied() else {
        return ListingFreshness::NotFound;
    };
    if let Some(rival) = candidates
        .iter()
        .skip(1)
        .find(|event| event.created_at == selected.created_at)
    {
        return ListingFreshness::Conflicting {
            a: selected.id,
            b: rival.id,
        };
    }
    match pinned {
        Some(pinned) if selected.id != pinned => ListingFreshness::Superseded {
            pinned,
            newer: selected.id,
        },
        _ if !observed.iter().any(|event| event.id == selected.id) => {
            if pinned.is_some() {
                ListingFreshness::PinnedNotObserved
            } else {
                // Only retained locally: no completing relay confirms it now.
                ListingFreshness::InsufficientEvidence {
                    completed,
                    required,
                }
            }
        }
        _ => ListingFreshness::Current(Box::new(selected.clone())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::{EventBuilder, Keys, Tag, Timestamp};

    fn revision(keys: &Keys, d: &str, at: u64, price: &str) -> Event {
        EventBuilder::new(Kind::Custom(30402), "listing")
            .tags([
                Tag::parse(["d", d]).expect("d"),
                Tag::parse(["price", price, "sat"]).expect("price"),
            ])
            .custom_created_at(Timestamp::from_secs(at))
            .sign_with_keys(keys)
            .expect("signed")
    }
    fn coverage(completed: &[(&str, Vec<Event>)], incomplete: &[&str]) -> RelayCoverage {
        RelayCoverage {
            completed: completed
                .iter()
                .map(|(url, events)| (url.to_string(), events.clone()))
                .collect(),
            incomplete: incomplete.iter().map(|s| s.to_string()).collect(),
            partial_events: vec![],
        }
    }

    #[test]
    fn majority_coverage_is_required_before_any_selection() {
        let dev = Keys::generate();
        let coordinate = format!("30402:{}:game", dev.public_key());
        let v1 = revision(&dev, "game", 100, "1000");
        let policy = CheckoutFreshnessPolicy::for_configured_relays(3);
        assert_eq!(policy.minimum_completed_relays, 2);
        assert_eq!(
            CheckoutFreshnessPolicy::for_configured_relays(2).minimum_completed_relays,
            2
        );
        assert_eq!(
            CheckoutFreshnessPolicy::for_configured_relays(1).minimum_completed_relays,
            1
        );
        assert_eq!(
            CheckoutFreshnessPolicy::for_configured_relays(0).minimum_completed_relays,
            1
        );
        let one = coverage(&[("a", vec![v1.clone()])], &["b", "c"]);
        assert_eq!(
            evaluate_checkout_listing(&coordinate, None, &one, &[], policy),
            ListingFreshness::InsufficientEvidence {
                completed: 1,
                required: 2
            }
        );
        let two = coverage(&[("a", vec![v1.clone()]), ("b", vec![])], &["c"]);
        assert_eq!(
            evaluate_checkout_listing(&coordinate, None, &two, &[], policy),
            ListingFreshness::Current(Box::new(v1))
        );
        let none = coverage(&[("a", vec![]), ("b", vec![])], &[]);
        assert_eq!(
            evaluate_checkout_listing(&coordinate, None, &none, &[], policy),
            ListingFreshness::NotFound
        );
    }

    #[test]
    fn newer_revision_on_any_completing_relay_supersedes_the_approval() {
        let dev = Keys::generate();
        let coordinate = format!("30402:{}:game", dev.public_key());
        let v1 = revision(&dev, "game", 100, "1000");
        let v2 = revision(&dev, "game", 200, "5000");
        let policy = CheckoutFreshnessPolicy::for_configured_relays(2);
        let split = coverage(&[("a", vec![v1.clone()]), ("b", vec![v2.clone()])], &[]);
        assert_eq!(
            evaluate_checkout_listing(&coordinate, Some(v1.id), &split, &[], policy),
            ListingFreshness::Superseded {
                pinned: v1.id,
                newer: v2.id
            }
        );
        // A withholding relay cannot roll back to a revision older than one retained locally.
        let stale = coverage(&[("a", vec![v1.clone()]), ("b", vec![v1.clone()])], &[]);
        assert_eq!(
            evaluate_checkout_listing(&coordinate, Some(v1.id), &stale, &[v2.clone()], policy),
            ListingFreshness::Superseded {
                pinned: v1.id,
                newer: v2.id
            }
        );
        // A newer revision delivered by a relay that then timed out still blocks.
        let mut partial = coverage(&[("a", vec![v1.clone()]), ("b", vec![v1.clone()])], &["c"]);
        partial.partial_events.push(v2.clone());
        let three = CheckoutFreshnessPolicy::for_configured_relays(3);
        assert_eq!(
            evaluate_checkout_listing(&coordinate, Some(v1.id), &partial, &[], three),
            ListingFreshness::Superseded {
                pinned: v1.id,
                newer: v2.id
            }
        );
        let current = coverage(&[("a", vec![v2.clone()]), ("b", vec![v1.clone()])], &[]);
        assert_eq!(
            evaluate_checkout_listing(&coordinate, Some(v2.id), &current, &[v1], policy),
            ListingFreshness::Current(Box::new(v2))
        );
    }

    #[test]
    fn same_timestamp_conflicts_forgeries_and_unobserved_pins_fail_closed() {
        let dev = Keys::generate();
        let attacker = Keys::generate();
        let coordinate = format!("30402:{}:game", dev.public_key());
        let a = revision(&dev, "game", 100, "1000");
        let b = revision(&dev, "game", 100, "9000");
        let policy = CheckoutFreshnessPolicy::for_configured_relays(1);
        let both = coverage(&[("a", vec![a.clone(), b.clone()])], &[]);
        assert!(matches!(
            evaluate_checkout_listing(&coordinate, Some(a.id), &both, &[], policy),
            ListingFreshness::Conflicting { .. }
        ));
        // Other authors, other identifiers and bad signatures are not revisions.
        let forged = revision(&attacker, "game", 300, "1");
        let other = revision(&dev, "other", 300, "1");
        let mut tampered = revision(&dev, "game", 400, "1");
        tampered.content = "changed".into();
        let noisy = coverage(&[("a", vec![a.clone(), forged, other, tampered])], &[]);
        assert_eq!(
            evaluate_checkout_listing(&coordinate, Some(a.id), &noisy, &[], policy),
            ListingFreshness::Current(Box::new(a.clone()))
        );
        // Retained locally but not returned by any completing relay now.
        let empty = coverage(&[("a", vec![])], &[]);
        assert_eq!(
            evaluate_checkout_listing(&coordinate, Some(a.id), &empty, &[a.clone()], policy),
            ListingFreshness::PinnedNotObserved
        );
        assert!(matches!(
            evaluate_checkout_listing(&coordinate, None, &empty, &[a], policy),
            ListingFreshness::InsufficientEvidence { .. }
        ));
        assert_eq!(
            evaluate_checkout_listing("30402:not-a-key:game", None, &both, &[], policy),
            ListingFreshness::NotFound
        );
    }
}

#[cfg(test)]
mod trusted_coverage_tests {
    use super::*;
    use nostr::{EventBuilder, Keys, Kind, Tag, Timestamp};
    use std::collections::HashSet;

    fn listing(keys: &Keys, at: u64) -> Event {
        EventBuilder::new(Kind::Custom(30402), "")
            .tags([Tag::parse(["d", "game"]).expect("d")])
            .custom_created_at(Timestamp::from_secs(at))
            .sign_with_keys(keys)
            .expect("listing")
    }

    fn trusted() -> HashSet<String> {
        ["wss://a.example", "wss://b.example", "wss://c.example"]
            .into_iter()
            .map(String::from)
            .collect()
    }

    /// ADP-01 §Relay availability (gap C5): relays learned from someone else's
    /// NIP-65 list must not raise or lower the quorum.
    #[test]
    fn unreachable_discovered_relays_do_not_count_against_the_quorum() {
        let keys = Keys::generate();
        let event = listing(&keys, 100);
        let mut coverage = RelayCoverage::default();
        for url in ["wss://a.example", "wss://b.example"] {
            coverage.completed.insert(url.into(), vec![event.clone()]);
        }
        coverage.incomplete.push("wss://c.example".into());
        for i in 0..10 {
            coverage
                .incomplete
                .push(format!("wss://planted-{i}.example"));
        }
        let coverage = coverage.restricted_to(&trusted());
        assert_eq!(coverage.configured(), 3);
        let policy = CheckoutFreshnessPolicy::for_configured_relays(coverage.configured());
        assert_eq!(policy.minimum_completed_relays, 2);
        assert!(coverage.completed.len() >= policy.minimum_completed_relays);
    }

    #[test]
    fn discovered_relays_cannot_satisfy_the_quorum_on_their_own() {
        let keys = Keys::generate();
        let event = listing(&keys, 100);
        let mut coverage = RelayCoverage::default();
        coverage.incomplete.extend(trusted());
        for i in 0..5 {
            coverage
                .completed
                .insert(format!("wss://planted-{i}.example"), vec![event.clone()]);
        }
        let coverage = coverage.restricted_to(&trusted());
        assert!(coverage.completed.is_empty());
        assert_eq!(coverage.configured(), 3);
        assert_eq!(
            coverage.partial_events.len(),
            5,
            "kept as positive evidence"
        );
    }

    #[test]
    fn a_newer_revision_on_a_discovered_relay_is_still_seen() {
        let keys = Keys::generate();
        let (old, newer) = (listing(&keys, 100), listing(&keys, 200));
        let mut coverage = RelayCoverage::default();
        coverage
            .completed
            .insert("wss://a.example".into(), vec![old.clone()]);
        coverage
            .completed
            .insert("wss://publisher-outbox.example".into(), vec![newer.clone()]);
        coverage
            .completed
            .insert("wss://b.example".into(), vec![old.clone()]);
        let coverage = coverage.restricted_to(&trusted());
        assert!(coverage.partial_events.iter().any(|e| e.id == newer.id));
        let coordinate = format!("30402:{}:game", keys.public_key().to_hex());
        let result = evaluate_checkout_listing(
            &coordinate,
            Some(old.id),
            &coverage,
            &[],
            CheckoutFreshnessPolicy::for_configured_relays(coverage.configured()),
        );
        assert!(
            matches!(result, ListingFreshness::Superseded { newer: id, .. } if id == newer.id),
            "{result:?}"
        );
    }
}
