use std::collections::{HashMap, HashSet};

use nostr::{Event, EventId, PublicKey};

#[cfg(feature = "native")]
use nostr::{Filter, Kind};

pub use crate::adp_protocol::FULFILLMENT_AUTHORIZATION_KIND;
use crate::adp_protocol::{coordinate_publisher, exact_tag, ChainError, ProtocolEventError};

#[cfg(feature = "native")]
use crate::relay_manager::{RelayManager, RelayManagerError};

#[cfg(feature = "native")]
#[derive(Debug, thiserror::Error)]
pub enum AuthorizationDiscoveryError {
    #[error(transparent)]
    Relay(#[from] RelayManagerError),
    #[error("authorization root event is unavailable")]
    Unavailable,
    #[error(transparent)]
    Event(#[from] ProtocolEventError),
    #[error(transparent)]
    Chain(#[from] ChainError),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthorizationTerms {
    pub authorization_id: String,
    pub coordinate: String,
    pub fulfillment_pubkey: PublicKey,
    pub valid_from: u64,
    pub valid_until: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthorizationTransition {
    ActiveRoot(AuthorizationTerms),
    Revoke,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthorizationEvent {
    pub event: Event,
    pub transition_d: String,
    pub authorization_id: String,
    pub coordinate: String,
    pub fulfillment_pubkey: PublicKey,
    pub predecessor: Option<EventId>,
    pub transition: AuthorizationTransition,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedAuthorization {
    pub root_event_id: EventId,
    pub developer_pubkey: PublicKey,
    pub terms: AuthorizationTerms,
    pub events: Vec<AuthorizationEvent>,
    pub revoked_at: Option<u64>,
}

fn validate_terms(terms: &AuthorizationTerms) -> Result<(), ProtocolEventError> {
    if terms.authorization_id.is_empty()
        || coordinate_publisher(&terms.coordinate).is_none()
        || terms
            .valid_until
            .is_some_and(|until| until <= terms.valid_from)
    {
        return Err(ProtocolEventError::MalformedTag(
            "authorization terms".into(),
        ));
    }
    Ok(())
}

fn reject_unlisted_tags(event: &Event, allowed: &[&str]) -> Result<(), ProtocolEventError> {
    for tag in event.tags.iter() {
        let values = tag.clone().to_vec();
        let Some(name) = values.first() else {
            return Err(ProtocolEventError::ForbiddenTag("tag".into()));
        };
        if !allowed.contains(&name.as_str()) {
            return Err(ProtocolEventError::ForbiddenTag(name.clone()));
        }
    }
    Ok(())
}

pub fn parse_authorization_event(event: &Event) -> Result<AuthorizationEvent, ProtocolEventError> {
    if event.kind.as_u16() != FULFILLMENT_AUTHORIZATION_KIND {
        return Err(ProtocolEventError::WrongKind {
            expected: FULFILLMENT_AUTHORIZATION_KIND,
            found: event.kind.as_u16(),
        });
    }
    event
        .verify()
        .map_err(|_| ProtocolEventError::InvalidSignature)?;

    let transition_d = exact_tag(event, "d", true)?.ok_or(ProtocolEventError::MissingTag("d"))?;
    let authorization_id = exact_tag(event, "authorization_id", true)?
        .ok_or(ProtocolEventError::MissingTag("authorization_id"))?;
    let coordinate = exact_tag(event, "a", true)?.ok_or(ProtocolEventError::MissingTag("a"))?;
    if coordinate_publisher(&coordinate) != Some(event.pubkey) {
        return Err(ProtocolEventError::WrongPublisher);
    }
    let fulfillment_pubkey = PublicKey::from_hex(
        &exact_tag(event, "p", true)?.ok_or(ProtocolEventError::MissingTag("p"))?,
    )
    .map_err(|_| ProtocolEventError::MalformedTag("p".into()))?;
    let status =
        exact_tag(event, "status", true)?.ok_or(ProtocolEventError::MissingTag("status"))?;
    let predecessor = exact_tag(event, "e", false)?
        .map(|value| {
            EventId::from_hex(&value).map_err(|_| ProtocolEventError::MalformedTag("e".into()))
        })
        .transpose()?;

    let transition = match (status.as_str(), predecessor) {
        ("active", None) => {
            reject_unlisted_tags(
                event,
                &[
                    "d",
                    "authorization_id",
                    "a",
                    "p",
                    "valid_from",
                    "valid_until",
                    "status",
                ],
            )?;
            let valid_from = exact_tag(event, "valid_from", true)?
                .and_then(|value| value.parse::<u64>().ok())
                .ok_or_else(|| ProtocolEventError::MalformedTag("valid_from".into()))?;
            let valid_until = exact_tag(event, "valid_until", false)?
                .map(|value| {
                    value
                        .parse::<u64>()
                        .map_err(|_| ProtocolEventError::MalformedTag("valid_until".into()))
                })
                .transpose()?;
            let terms = AuthorizationTerms {
                authorization_id: authorization_id.clone(),
                coordinate: coordinate.clone(),
                fulfillment_pubkey,
                valid_from,
                valid_until,
            };
            validate_terms(&terms)?;
            AuthorizationTransition::ActiveRoot(terms)
        }
        ("active", Some(_)) => return Err(ProtocolEventError::ForbiddenTag("e".into())),
        ("revoked", Some(_)) => {
            reject_unlisted_tags(event, &["d", "authorization_id", "a", "p", "status", "e"])?;
            AuthorizationTransition::Revoke
        }
        ("revoked", None) => return Err(ProtocolEventError::MissingTag("e")),
        _ => return Err(ProtocolEventError::MalformedTag("status".into())),
    };

    Ok(AuthorizationEvent {
        event: event.clone(),
        transition_d,
        authorization_id,
        coordinate,
        fulfillment_pubkey,
        predecessor,
        transition,
    })
}

pub fn resolve_authorization(
    expected_root_id: EventId,
    nodes: &[AuthorizationEvent],
) -> Result<ResolvedAuthorization, ChainError> {
    if nodes.is_empty() {
        return Err(ChainError::Empty);
    }
    let nodes = nodes
        .iter()
        .map(|node| {
            let parsed = parse_authorization_event(&node.event)
                .map_err(|error| ChainError::InvalidTransition(error.to_string()))?;
            if parsed != *node {
                return Err(ChainError::InvariantMutation("signed event metadata"));
            }
            Ok(parsed)
        })
        .collect::<Result<Vec<_>, _>>()?;
    if nodes
        .iter()
        .any(|node| node.predecessor == Some(node.event.id))
    {
        return Err(ChainError::Cycle);
    }

    let by_id = nodes
        .iter()
        .map(|node| (node.event.id, node))
        .collect::<HashMap<_, _>>();
    if by_id.len() != nodes.len() {
        return Err(ChainError::Disconnected);
    }
    for node in &nodes {
        if let Some(predecessor) = node.predecessor {
            if !by_id.contains_key(&predecessor) {
                return Err(ChainError::MissingPredecessor(predecessor));
            }
        }
    }
    let roots = nodes
        .iter()
        .filter(|node| node.predecessor.is_none())
        .collect::<Vec<_>>();
    if roots.is_empty() {
        return Err(ChainError::Cycle);
    }
    if roots.len() > 1 {
        return Err(ChainError::MultipleRoots);
    }
    let root = roots[0];
    if root.event.id != expected_root_id {
        return Err(ChainError::Disconnected);
    }
    let terms = match &root.transition {
        AuthorizationTransition::ActiveRoot(terms) => terms.clone(),
        AuthorizationTransition::Revoke => {
            return Err(ChainError::InvalidTransition(
                "authorization root must be active".into(),
            ));
        }
    };
    let developer = root.event.pubkey;
    let mut transition_ds = HashSet::with_capacity(nodes.len());
    for node in &nodes {
        if node.event.pubkey != developer {
            return Err(ChainError::InvariantMutation("developer"));
        }
        if node.authorization_id != terms.authorization_id {
            return Err(ChainError::InvariantMutation("authorization_id"));
        }
        if node.coordinate != terms.coordinate {
            return Err(ChainError::InvariantMutation("a"));
        }
        if node.fulfillment_pubkey != terms.fulfillment_pubkey {
            return Err(ChainError::InvariantMutation("p"));
        }
        if !transition_ds.insert(node.transition_d.as_str()) {
            return Err(ChainError::InvalidTransition(
                "duplicate transition d".into(),
            ));
        }
    }
    let mut successors = HashMap::new();
    for node in &nodes {
        if let Some(predecessor) = node.predecessor {
            if successors.insert(predecessor, node.event.id).is_some() {
                return Err(ChainError::Fork(predecessor));
            }
        }
    }
    let mut ordered = Vec::with_capacity(nodes.len());
    let mut visited = HashSet::with_capacity(nodes.len());
    let mut current = root.event.id;
    loop {
        if !visited.insert(current) {
            return Err(ChainError::Cycle);
        }
        ordered.push((*by_id[&current]).clone());
        match successors.get(&current) {
            Some(next) => current = *next,
            None => break,
        }
    }
    if ordered.len() != nodes.len() {
        return Err(ChainError::Disconnected);
    }
    if ordered.len() > 2 {
        return Err(ChainError::InvalidTransition(
            "revocation is terminal".into(),
        ));
    }
    let revoked_at = if let Some(revoke) = ordered.get(1) {
        if revoke.event.created_at <= ordered[0].event.created_at {
            return Err(ChainError::TimestampRegression);
        }
        if !matches!(revoke.transition, AuthorizationTransition::Revoke) {
            return Err(ChainError::InvalidTransition(
                "successor must revoke authorization".into(),
            ));
        }
        Some(revoke.event.created_at.as_secs())
    } else {
        None
    };

    Ok(ResolvedAuthorization {
        root_event_id: root.event.id,
        developer_pubkey: developer,
        terms,
        events: ordered,
        revoked_at,
    })
}

impl ResolvedAuthorization {
    pub fn authorizes(&self, signer: &PublicKey, coordinate: &str, at: u64) -> bool {
        let Ok(resolved) = resolve_authorization(self.root_event_id, &self.events) else {
            return false;
        };
        resolved == *self
            && signer == &self.terms.fulfillment_pubkey
            && coordinate == self.terms.coordinate
            && self.terms.valid_from <= at
            && self
                .terms
                .valid_until
                .map_or(true, |valid_until| at < valid_until)
            && self.revoked_at.map_or(true, |revoked_at| at < revoked_at)
    }
}

#[cfg(feature = "native")]
pub async fn discover_authorization(
    relays: &RelayManager,
    root_event_id: EventId,
    developer_pubkey: PublicKey,
) -> Result<ResolvedAuthorization, AuthorizationDiscoveryError> {
    let mut events = relays
        .fetch_events_best_effort(Filter::new().id(root_event_id))
        .await?;
    events.extend(
        relays
            .fetch_events_best_effort(
                Filter::new()
                    .kind(Kind::Custom(FULFILLMENT_AUTHORIZATION_KIND))
                    .author(developer_pubkey),
            )
            .await?,
    );

    let root = events
        .iter()
        .find(|event| event.id == root_event_id)
        .ok_or(AuthorizationDiscoveryError::Unavailable)
        .and_then(|event| parse_authorization_event(event).map_err(Into::into))?;
    if root.event.pubkey != developer_pubkey {
        return Err(ProtocolEventError::WrongPublisher.into());
    }

    let mut seen = HashSet::new();
    let nodes = events
        .iter()
        .filter_map(|event| parse_authorization_event(event).ok())
        .filter(|event| {
            event.event.pubkey == developer_pubkey
                && event.authorization_id == root.authorization_id
                && seen.insert(event.event.id)
        })
        .collect::<Vec<_>>();
    resolve_authorization(root_event_id, &nodes).map_err(Into::into)
}
