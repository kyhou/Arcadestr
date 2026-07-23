use std::collections::{HashMap, HashSet};

use nostr::{Event, EventId, PublicKey};
use thiserror::Error;

use crate::adp_protocol::{
    coordinate_publisher, exact_tag, ChainError, ProtocolEventError, ENTITLEMENT_GRANT_KIND,
    TAG_AUTHORIZATION_EVENT, TAG_COORDINATE, TAG_IDENTIFIER, TAG_PREDECESSOR, TAG_REASON,
    TAG_RECIPIENT, TAG_SOURCE_EVENT, TAG_STATUS,
};
use crate::authorization::{ResolvedAuthorization, CAPABILITY_ISSUE_GRANT};
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
    pub authorization: Option<EventId>,
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
        authorization: authorization_event,
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
    let roots = nodes
        .iter()
        .filter(|node| node.predecessor.is_none() && node.status == GrantStatus::Granted)
        .collect::<Vec<_>>();
    if roots.is_empty() {
        return Err(ChainError::Cycle.into());
    }
    if roots.len() != 1 {
        return Err(ChainError::MultipleRoots.into());
    }
    let root = roots[0];
    let mut ordered = Vec::with_capacity(nodes.len());
    let mut visited = HashSet::with_capacity(nodes.len());
    let mut current = root.event.id;
    loop {
        if !visited.insert(current) {
            return Err(ChainError::Cycle.into());
        }
        let predecessor = by_id[&current];
        ordered.push((*predecessor).clone());
        let mut valid = nodes
            .iter()
            .filter(|candidate| candidate.predecessor == Some(current))
            .filter(|candidate| {
                candidate.grant_id == root.grant_id
                    && candidate.recipient == root.recipient
                    && candidate.coordinate == root.coordinate
                    && candidate.source_event == root.source_event
                    && candidate.authorization == root.authorization
                    && candidate.status == GrantStatus::Revoked
                    && coordinate_publisher(&root.coordinate) == Some(candidate.event.pubkey)
                    && predecessor.status != GrantStatus::Revoked
                    && candidate.event.created_at > predecessor.event.created_at
                    && candidate.predecessor != Some(candidate.event.id)
            })
            .collect::<Vec<_>>();
        if valid.len() > 1 {
            return Err(ChainError::Fork(current).into());
        }
        let Some(next) = valid.pop() else { break };
        current = next.event.id;
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
    match (root.event.pubkey == publisher, root.authorization) {
        (true, None) => {}
        (true, Some(_)) => return Err(EntitlementError::InvalidAuthorization),
        (false, Some(authorization_event)) => {
            let authorization = authorization.ok_or(EntitlementError::MissingAuthorization)?;
            if authorization.root_event_id != authorization_event
                || authorization.developer_pubkey != publisher
                || !authorization.authorizes(
                    &root.event.pubkey,
                    &root.coordinate,
                    CAPABILITY_ISSUE_GRANT,
                    issued_at,
                )
            {
                return Err(EntitlementError::InvalidAuthorization);
            }
        }
        (false, None) => return Err(EntitlementError::MissingAuthorizationEvent),
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
