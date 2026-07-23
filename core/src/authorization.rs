use std::collections::{BTreeSet, HashMap};

use nostr::{Event, EventId, PublicKey};

#[cfg(feature = "native")]
use nostr::{Filter, Kind};

pub use crate::adp_protocol::FULFILLMENT_AUTHORIZATION_KIND;
use crate::adp_protocol::{coordinate_publisher, exact_tag, ChainError, ProtocolEventError};

#[cfg(feature = "native")]
use crate::relay_manager::{RelayManager, RelayManagerError};

pub const CAPABILITY_ISSUE_RECEIPT: &str = "issue_receipt";
pub const CAPABILITY_ISSUE_GRANT: &str = "issue_grant";
pub const CAPABILITY_UPLOAD_BUILD: &str = "upload_build";

#[cfg(feature = "native")]
#[derive(Debug, thiserror::Error)]
pub enum AuthorizationDiscoveryError {
    #[error("authorization relay evidence is unavailable: {0}")]
    Relay(#[from] RelayManagerError),
    #[error("authorization relay evidence is unavailable")]
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
    pub operator_pubkey: PublicKey,
    pub fulfillment_pubkey: PublicKey,
    pub capabilities: BTreeSet<String>,
    pub valid_from: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthorizationTransition {
    ActiveRoot,
    Cancel,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthorizationEvent {
    pub event: Event,
    pub terms: AuthorizationTerms,
    pub predecessor: Option<EventId>,
    pub transition: AuthorizationTransition,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedAuthorization {
    pub root_event_id: EventId,
    pub developer_pubkey: PublicKey,
    pub terms: AuthorizationTerms,
    pub events: Vec<AuthorizationEvent>,
    pub cancelled_at: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedAttestation {
    pub developer_pubkey: PublicKey,
    pub fulfillment_pubkey: PublicKey,
    pub valid_from: u64,
    pub revoked_at: Option<u64>,
    pub scope: Option<String>,
}

impl ParsedAttestation {
    pub fn allows_new_operations_at(&self, at: u64) -> bool {
        self.valid_from <= at && self.revoked_at.is_none_or(|revoked| at < revoked)
    }
}

fn capabilities(event: &Event) -> Result<BTreeSet<String>, ProtocolEventError> {
    let mut capabilities = BTreeSet::new();
    for tag in event.tags.iter() {
        let values = tag.clone().to_vec();
        if values.first().is_some_and(|name| name == "capability")
            && (values.len() != 2
                || values[1].is_empty()
                || !capabilities.insert(values[1].clone()))
        {
            return Err(ProtocolEventError::MalformedTag("capability".into()));
        }
    }
    if capabilities.is_empty() {
        return Err(ProtocolEventError::MissingTag("capability"));
    }
    if !capabilities.iter().any(|capability| {
        matches!(
            capability.as_str(),
            CAPABILITY_ISSUE_RECEIPT | CAPABILITY_ISSUE_GRANT | CAPABILITY_UPLOAD_BUILD
        )
    }) {
        return Err(ProtocolEventError::MalformedTag("capability".into()));
    }
    Ok(capabilities)
}

fn reject_unknown_tags(event: &Event) -> Result<(), ProtocolEventError> {
    const ALLOWED: &[&str] = &[
        "d",
        "a",
        "p",
        "fulfillment_pubkey",
        "capability",
        "valid_from",
        "status",
        "e",
    ];
    for tag in event.tags.iter() {
        let values = tag.clone().to_vec();
        let name = values
            .first()
            .ok_or_else(|| ProtocolEventError::ForbiddenTag("tag".into()))?;
        if !ALLOWED.contains(&name.as_str()) {
            return Err(ProtocolEventError::ForbiddenTag(name.clone()));
        }
    }
    Ok(())
}

pub fn parse_attestation_event(event: &Event) -> Result<ParsedAttestation, ProtocolEventError> {
    if event.kind.as_u16() != 30404 {
        return Err(ProtocolEventError::WrongKind {
            expected: 30404,
            found: event.kind.as_u16(),
        });
    }
    event
        .verify()
        .map_err(|_| ProtocolEventError::InvalidSignature)?;
    let d = exact_tag(event, "d", true)?.ok_or(ProtocolEventError::MissingTag("d"))?;
    let developer_pubkey = PublicKey::from_hex(
        &exact_tag(event, "p", true)?.ok_or(ProtocolEventError::MissingTag("p"))?,
    )
    .map_err(|_| ProtocolEventError::MalformedTag("p".into()))?;
    let entries = event
        .tags
        .iter()
        .map(|tag| tag.clone().to_vec())
        .filter(|values| {
            values
                .first()
                .is_some_and(|name| name == "fulfillment_pubkey")
        })
        .collect::<Vec<_>>();
    if entries.len() != 1 {
        return Err(if entries.is_empty() {
            ProtocolEventError::MissingTag("fulfillment_pubkey")
        } else {
            ProtocolEventError::DuplicateTag("fulfillment_pubkey".into())
        });
    }
    let values = &entries[0];
    if values.len() != 4 {
        return Err(ProtocolEventError::MalformedTag(
            "fulfillment_pubkey".into(),
        ));
    }
    let fulfillment_pubkey = PublicKey::from_hex(&values[1])
        .map_err(|_| ProtocolEventError::MalformedTag("fulfillment_pubkey".into()))?;
    let valid_from = values[2]
        .parse::<u64>()
        .map_err(|_| ProtocolEventError::MalformedTag("fulfillment_pubkey".into()))?;
    let revoked_at = (!values[3].is_empty())
        .then(|| values[3].parse::<u64>())
        .transpose()
        .map_err(|_| ProtocolEventError::MalformedTag("fulfillment_pubkey".into()))?;
    if d != format!(
        "{}:{}",
        developer_pubkey.to_hex(),
        fulfillment_pubkey.to_hex()
    ) {
        return Err(ProtocolEventError::MalformedTag("d".into()));
    }
    let scope = exact_tag(event, "scope", false)?;
    Ok(ParsedAttestation {
        developer_pubkey,
        fulfillment_pubkey,
        valid_from,
        revoked_at,
        scope,
    })
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
    reject_unknown_tags(event)?;
    let authorization_id =
        exact_tag(event, "d", true)?.ok_or(ProtocolEventError::MissingTag("d"))?;
    let coordinate = exact_tag(event, "a", true)?.ok_or(ProtocolEventError::MissingTag("a"))?;
    if coordinate_publisher(&coordinate) != Some(event.pubkey) {
        return Err(ProtocolEventError::WrongPublisher);
    }
    let operator_pubkey = PublicKey::from_hex(
        &exact_tag(event, "p", true)?.ok_or(ProtocolEventError::MissingTag("p"))?,
    )
    .map_err(|_| ProtocolEventError::MalformedTag("p".into()))?;
    let fulfillment_pubkey = PublicKey::from_hex(
        &exact_tag(event, "fulfillment_pubkey", true)?
            .ok_or(ProtocolEventError::MissingTag("fulfillment_pubkey"))?,
    )
    .map_err(|_| ProtocolEventError::MalformedTag("fulfillment_pubkey".into()))?;
    let capabilities = capabilities(event)?;
    let valid_from = exact_tag(event, "valid_from", true)?
        .ok_or(ProtocolEventError::MissingTag("valid_from"))?
        .parse::<u64>()
        .map_err(|_| ProtocolEventError::MalformedTag("valid_from".into()))?;
    let status =
        exact_tag(event, "status", true)?.ok_or(ProtocolEventError::MissingTag("status"))?;
    let predecessor = exact_tag(event, "e", false)?
        .map(|value| {
            EventId::from_hex(&value).map_err(|_| ProtocolEventError::MalformedTag("e".into()))
        })
        .transpose()?;
    let transition = match (status.as_str(), predecessor) {
        ("active", None) => AuthorizationTransition::ActiveRoot,
        ("cancelled", Some(_)) => AuthorizationTransition::Cancel,
        ("active", Some(_)) => return Err(ProtocolEventError::ForbiddenTag("e".into())),
        ("cancelled", None) => return Err(ProtocolEventError::MissingTag("e")),
        _ => return Err(ProtocolEventError::MalformedTag("status".into())),
    };
    if transition == AuthorizationTransition::ActiveRoot && valid_from < event.created_at.as_secs()
    {
        return Err(ProtocolEventError::MalformedTag("valid_from".into()));
    }
    Ok(AuthorizationEvent {
        event: event.clone(),
        terms: AuthorizationTerms {
            authorization_id,
            coordinate,
            operator_pubkey,
            fulfillment_pubkey,
            capabilities,
            valid_from,
        },
        predecessor,
        transition,
    })
}

pub fn resolve_authorization(
    expected_root_id: EventId,
    events: &[Event],
) -> Result<ResolvedAuthorization, ChainError> {
    resolve_authorization_at(expected_root_id, events, u64::MAX)
}

pub fn resolve_authorization_at(
    expected_root_id: EventId,
    events: &[Event],
    at: u64,
) -> Result<ResolvedAuthorization, ChainError> {
    let by_id = events
        .iter()
        .filter(|event| event.created_at.as_secs() <= at)
        .map(|event| (event.id, event))
        .collect::<HashMap<_, _>>();
    let root_event = by_id.get(&expected_root_id).ok_or(ChainError::Empty)?;
    let root = parse_authorization_event(root_event).map_err(|error| {
        ChainError::InvalidTransition(format!("invalid authorization root: {error}"))
    })?;
    if root.transition != AuthorizationTransition::ActiveRoot || root.predecessor.is_some() {
        return Err(ChainError::InvalidTransition(
            "authorization root must be active".into(),
        ));
    }
    let mut raw_successors: HashMap<EventId, Vec<&Event>> = HashMap::new();
    for event in by_id.values() {
        for tag in event.tags.iter() {
            let values = tag.clone().to_vec();
            if values.len() == 2 && values[0] == "e" {
                if let Ok(predecessor) = EventId::from_hex(&values[1]) {
                    raw_successors.entry(predecessor).or_default().push(event);
                }
            }
        }
    }
    let mut valid = raw_successors
        .get(&root.event.id)
        .into_iter()
        .flatten()
        .filter_map(|candidate| parse_authorization_event(candidate).ok())
        .filter(|candidate| {
            candidate.predecessor == Some(root.event.id)
                && candidate.transition == AuthorizationTransition::Cancel
                && candidate.terms == root.terms
                && candidate.event.created_at > root.event.created_at
        })
        .collect::<Vec<_>>();
    if valid.len() > 1 {
        return Err(ChainError::Fork(root.event.id));
    }
    let cancellation = valid.pop();
    let mut ordered = vec![root.clone()];
    if let Some(cancellation) = cancellation {
        ordered.push(cancellation);
    }
    Ok(ResolvedAuthorization {
        root_event_id: expected_root_id,
        developer_pubkey: root.event.pubkey,
        terms: root.terms,
        cancelled_at: ordered.get(1).map(|event| event.event.created_at.as_secs()),
        events: ordered,
    })
}

impl ResolvedAuthorization {
    pub fn has_capability(&self, capability: &str) -> bool {
        matches!(
            capability,
            CAPABILITY_ISSUE_RECEIPT | CAPABILITY_ISSUE_GRANT | CAPABILITY_UPLOAD_BUILD
        ) && self.terms.capabilities.contains(capability)
    }

    pub fn authorizes(
        &self,
        signer: &PublicKey,
        coordinate: &str,
        capability: &str,
        at: u64,
    ) -> bool {
        signer == &self.terms.fulfillment_pubkey
            && coordinate == self.terms.coordinate
            && self.terms.valid_from <= at
            && self.cancelled_at.is_none_or(|cancelled| at < cancelled)
            && self.has_capability(capability)
    }
}

/// Selects the lowest typed root ID among currently reusable authorizations.
#[cfg(feature = "native")]
pub fn select_reusable_authorization<'a>(
    authorizations: &'a [ResolvedAuthorization],
    references: &[crate::marketplace::FulfillmentAuthorizationReference],
    held_fulfillment_keys: &std::collections::HashSet<PublicKey>,
    operator: PublicKey,
    coordinate: &str,
    required_capabilities: &BTreeSet<String>,
    at: u64,
) -> Option<&'a ResolvedAuthorization> {
    let mut candidates = authorizations
        .iter()
        .filter(|authorization| {
            authorization.terms.operator_pubkey == operator
                && authorization.terms.coordinate == coordinate
                && required_capabilities
                    .iter()
                    .all(|capability| authorization.has_capability(capability))
                && held_fulfillment_keys.contains(&authorization.terms.fulfillment_pubkey)
                && references.iter().any(|reference| {
                    reference.root_event_id == authorization.root_event_id
                        && reference.fulfillment_pubkey == authorization.terms.fulfillment_pubkey
                })
                && authorization.terms.valid_from <= at
                && authorization
                    .cancelled_at
                    .is_none_or(|cancelled| at < cancelled)
        })
        .collect::<Vec<_>>();
    candidates.sort_by_key(|authorization| authorization.root_event_id);
    candidates.into_iter().next()
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
        .ok_or(AuthorizationDiscoveryError::Unavailable)?;
    if root.pubkey != developer_pubkey {
        return Err(ProtocolEventError::WrongPublisher.into());
    }
    let parsed_root = parse_authorization_event(root)?;
    let mut seen = std::collections::HashSet::new();
    events.retain(|event| seen.insert(event.id));
    let event_ids = events
        .iter()
        .map(|event| event.id)
        .collect::<std::collections::HashSet<_>>();
    if events
        .iter()
        .filter_map(|event| parse_authorization_event(event).ok())
        .any(|event| {
            event.terms.authorization_id == parsed_root.terms.authorization_id
                && event
                    .predecessor
                    .is_some_and(|predecessor| !event_ids.contains(&predecessor))
        })
    {
        return Err(AuthorizationDiscoveryError::Unavailable);
    }
    resolve_authorization(root_event_id, &events).map_err(Into::into)
}
