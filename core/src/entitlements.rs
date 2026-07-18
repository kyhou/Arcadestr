use std::collections::{HashMap, HashSet};

use nostr::{Event, EventId, PublicKey};
use thiserror::Error;

use crate::adp_protocol::{
    coordinate_publisher, exact_tag, ChainError, ProtocolEventError, ENTITLEMENT_GRANT_KIND,
    TAG_AUTHORIZATION_EVENT, TAG_COORDINATE, TAG_IDENTIFIER, TAG_PREDECESSOR, TAG_REASON,
    TAG_RECIPIENT, TAG_SOURCE_EVENT, TAG_STATUS,
};
use crate::authorization::ResolvedAuthorization;
use crate::campaign::ResolvedCampaign;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GrantStatus {
    Granted,
    Revoked,
}

impl GrantStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Granted => "granted",
            Self::Revoked => "revoked",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedEntitlementGrant {
    pub event: Event,
    pub grant_id: String,
    pub recipient: PublicKey,
    pub coordinate: String,
    pub source_event: EventId,
    pub authorization_event: Option<EventId>,
    pub reason: Option<String>,
    pub status: GrantStatus,
    pub predecessor: Option<EventId>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedEntitlementGrant {
    pub root_event_id: EventId,
    pub events: Vec<ParsedEntitlementGrant>,
}

impl ResolvedEntitlementGrant {
    pub fn status(&self) -> Option<GrantStatus> {
        self.events.last().map(|event| event.status)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssuanceDelegation {
    pub pubkey: PublicKey,
    pub valid_from: u64,
    pub revoked_at: Option<u64>,
}

impl IssuanceDelegation {
    fn authorizes(&self, signer: PublicKey, at: u64) -> bool {
        self.pubkey == signer
            && self.valid_from <= at
            && self.revoked_at.map_or(true, |revoked_at| at < revoked_at)
    }
}

#[derive(Debug, Error)]
pub enum EntitlementError {
    #[error(transparent)]
    Event(#[from] ProtocolEventError),
    #[error(transparent)]
    Chain(#[from] ChainError),
    #[error("grant coordinate is not an ADP game coordinate")]
    InvalidCoordinate,
    #[error("grant source does not identify the campaign root")]
    CampaignMismatch,
    #[error("grant was not issued during an active campaign")]
    CampaignNotClaimable,
    #[error("delegated grant does not reference an authorization event")]
    MissingAuthorizationEvent,
    #[error("referenced authorization lifecycle was not resolved")]
    MissingAuthorization,
    #[error("authorization does not match this delegated grant")]
    InvalidAuthorization,
    #[error("grant issuer is not the publisher or an authorized fulfillment key")]
    UnauthorizedIssuer,
    #[error("only the publisher may revoke an entitlement")]
    UnauthorizedRevoker,
}

pub fn parse_entitlement_event(
    event: &Event,
) -> Result<ParsedEntitlementGrant, ProtocolEventError> {
    if event.kind.as_u16() != ENTITLEMENT_GRANT_KIND {
        return Err(ProtocolEventError::WrongKind {
            expected: ENTITLEMENT_GRANT_KIND,
            found: event.kind.as_u16(),
        });
    }
    event
        .verify()
        .map_err(|_| ProtocolEventError::InvalidSignature)?;
    let grant_id = exact_tag(event, TAG_IDENTIFIER, true)?
        .ok_or(ProtocolEventError::MissingTag(TAG_IDENTIFIER))?;
    let recipient = PublicKey::from_hex(
        &exact_tag(event, TAG_RECIPIENT, true)?
            .ok_or(ProtocolEventError::MissingTag(TAG_RECIPIENT))?,
    )
    .map_err(|_| ProtocolEventError::MalformedTag(TAG_RECIPIENT.into()))?;
    let coordinate = exact_tag(event, TAG_COORDINATE, true)?
        .ok_or(ProtocolEventError::MissingTag(TAG_COORDINATE))?;
    if coordinate_publisher(&coordinate).is_none() {
        return Err(ProtocolEventError::MalformedTag(TAG_COORDINATE.into()));
    }
    let source_event = EventId::from_hex(
        &exact_tag(event, TAG_SOURCE_EVENT, true)?
            .ok_or(ProtocolEventError::MissingTag(TAG_SOURCE_EVENT))?,
    )
    .map_err(|_| ProtocolEventError::MalformedTag(TAG_SOURCE_EVENT.into()))?;
    let authorization_event = exact_tag(event, TAG_AUTHORIZATION_EVENT, false)?
        .map(|value| {
            EventId::from_hex(&value)
                .map_err(|_| ProtocolEventError::MalformedTag(TAG_AUTHORIZATION_EVENT.into()))
        })
        .transpose()?;
    let reason = exact_tag(event, TAG_REASON, false)?;
    let status = match exact_tag(event, TAG_STATUS, true)?
        .ok_or(ProtocolEventError::MissingTag(TAG_STATUS))?
        .as_str()
    {
        "granted" => GrantStatus::Granted,
        "revoked" => GrantStatus::Revoked,
        _ => return Err(ProtocolEventError::MalformedTag(TAG_STATUS.into())),
    };
    let predecessor = exact_tag(event, TAG_PREDECESSOR, false)?
        .map(|value| {
            EventId::from_hex(&value)
                .map_err(|_| ProtocolEventError::MalformedTag(TAG_PREDECESSOR.into()))
        })
        .transpose()?;
    match (status, predecessor) {
        (GrantStatus::Granted, None) | (GrantStatus::Revoked, Some(_)) => {}
        (GrantStatus::Granted, Some(_)) => {
            return Err(ProtocolEventError::ForbiddenTag(TAG_PREDECESSOR.into()));
        }
        (GrantStatus::Revoked, None) => {
            return Err(ProtocolEventError::MissingTag(TAG_PREDECESSOR));
        }
    }
    Ok(ParsedEntitlementGrant {
        event: event.clone(),
        grant_id,
        recipient,
        coordinate,
        source_event,
        authorization_event,
        reason,
        status,
        predecessor,
    })
}

pub fn resolve_entitlement_grant(
    nodes: &[ParsedEntitlementGrant],
) -> Result<ResolvedEntitlementGrant, EntitlementError> {
    if nodes.is_empty() {
        return Err(ChainError::Empty.into());
    }
    let mut seen = HashSet::new();
    let nodes = nodes
        .iter()
        .filter(|node| seen.insert(node.event.id))
        .map(|node| {
            let parsed = parse_entitlement_event(&node.event)?;
            if parsed != *node {
                return Err(EntitlementError::Chain(ChainError::InvariantMutation(
                    "signed event metadata",
                )));
            }
            Ok(parsed)
        })
        .collect::<Result<Vec<_>, _>>()?;
    for node in &nodes {
        if node.predecessor == Some(node.event.id) {
            return Err(ChainError::Cycle.into());
        }
    }
    let by_id = nodes
        .iter()
        .map(|node| (node.event.id, node))
        .collect::<HashMap<_, _>>();
    for node in &nodes {
        if let Some(predecessor) = node.predecessor {
            if !by_id.contains_key(&predecessor) {
                return Err(ChainError::MissingPredecessor(predecessor).into());
            }
        }
    }
    let roots = nodes
        .iter()
        .filter(|node| node.predecessor.is_none())
        .collect::<Vec<_>>();
    if roots.is_empty() {
        return Err(ChainError::Cycle.into());
    }
    if roots.len() != 1 {
        return Err(ChainError::MultipleRoots.into());
    }
    let root = roots[0];
    for node in &nodes {
        if node.grant_id != root.grant_id {
            return Err(ChainError::InvariantMutation(TAG_IDENTIFIER).into());
        }
        if node.recipient != root.recipient {
            return Err(ChainError::InvariantMutation(TAG_RECIPIENT).into());
        }
        if node.coordinate != root.coordinate {
            return Err(ChainError::InvariantMutation(TAG_COORDINATE).into());
        }
        if node.source_event != root.source_event {
            return Err(ChainError::InvariantMutation(TAG_SOURCE_EVENT).into());
        }
        if node.authorization_event != root.authorization_event {
            return Err(ChainError::InvariantMutation(TAG_AUTHORIZATION_EVENT).into());
        }
    }
    let mut successors = HashMap::new();
    for node in &nodes {
        if let Some(predecessor) = node.predecessor {
            if successors.insert(predecessor, node.event.id).is_some() {
                return Err(ChainError::Fork(predecessor).into());
            }
        }
    }
    let mut ordered = Vec::with_capacity(nodes.len());
    let mut visited = HashSet::with_capacity(nodes.len());
    let mut current = root.event.id;
    loop {
        if !visited.insert(current) {
            return Err(ChainError::Cycle.into());
        }
        ordered.push((*by_id[&current]).clone());
        match successors.get(&current) {
            Some(next) => current = *next,
            None => break,
        }
    }
    if ordered.len() != nodes.len() {
        return Err(ChainError::Disconnected.into());
    }
    if root.status != GrantStatus::Granted {
        return Err(ChainError::InvalidTransition("root must be granted".into()).into());
    }
    for pair in ordered.windows(2) {
        if pair[1].event.created_at <= pair[0].event.created_at {
            return Err(ChainError::TimestampRegression.into());
        }
        if pair[0].status == GrantStatus::Revoked || pair[1].status != GrantStatus::Revoked {
            return Err(ChainError::InvalidTransition("revocation is terminal".into()).into());
        }
    }
    Ok(ResolvedEntitlementGrant {
        root_event_id: root.event.id,
        events: ordered,
    })
}

pub fn validate_adp_entitlement(
    grant: &ResolvedEntitlementGrant,
    campaign: &ResolvedCampaign,
    authorization: Option<&ResolvedAuthorization>,
    delegations: &[IssuanceDelegation],
) -> Result<(), EntitlementError> {
    let resolved = resolve_entitlement_grant(&grant.events)?;
    if resolved != *grant {
        return Err(ChainError::InvariantMutation("resolved grant metadata").into());
    }
    let root = resolved.events.first().ok_or(ChainError::Empty)?;
    let publisher =
        coordinate_publisher(&root.coordinate).ok_or(EntitlementError::InvalidCoordinate)?;
    if campaign.root_event_id != root.source_event
        || campaign.publisher_pubkey != publisher
        || campaign.coordinate != root.coordinate
    {
        return Err(EntitlementError::CampaignMismatch);
    }
    let issued_at = root.event.created_at.as_secs();
    if !campaign.is_claimable_at(issued_at) {
        return Err(EntitlementError::CampaignNotClaimable);
    }
    match (root.event.pubkey == publisher, root.authorization_event) {
        (true, None) => {}
        (_, Some(authorization_event)) => {
            let authorization = authorization.ok_or(EntitlementError::MissingAuthorization)?;
            if authorization.root_event_id != authorization_event
                || authorization.developer_pubkey != publisher
                || !authorization.authorizes(&root.event.pubkey, &root.coordinate, issued_at)
            {
                return Err(EntitlementError::InvalidAuthorization);
            }
        }
        (false, None) => return Err(EntitlementError::MissingAuthorizationEvent),
    }
    if root.event.pubkey != publisher {
        if !delegations
            .iter()
            .any(|delegation| delegation.authorizes(root.event.pubkey, issued_at))
        {
            return Err(EntitlementError::UnauthorizedIssuer);
        }
    }
    if resolved
        .events
        .iter()
        .skip(1)
        .any(|event| event.event.pubkey != publisher)
    {
        return Err(EntitlementError::UnauthorizedRevoker);
    }
    Ok(())
}
