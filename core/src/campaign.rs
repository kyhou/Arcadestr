use std::collections::{HashMap, HashSet};

use nostr::{Event, EventId, PublicKey};

use crate::adp_protocol::{
    coordinate_publisher, exact_tag, ChainError, ProtocolEventError, ADP_CAMPAIGN_KIND,
    TAG_COORDINATE, TAG_ENDS, TAG_IDENTIFIER, TAG_MODE, TAG_PREDECESSOR, TAG_STARTS, TAG_STATUS,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CampaignMode {
    Claim,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CampaignTerms {
    pub mode: CampaignMode,
    pub starts: u64,
    pub ends: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CampaignBuildParams {
    pub campaign_id: String,
    pub coordinate: String,
    pub terms: Option<CampaignTerms>,
    pub predecessor: Option<EventId>,
    pub cancelled: bool,
}

impl CampaignBuildParams {
    pub fn active(
        campaign_id: String,
        coordinate: String,
        starts: u64,
        ends: u64,
        predecessor: Option<EventId>,
    ) -> Self {
        Self {
            campaign_id,
            coordinate,
            terms: Some(CampaignTerms {
                mode: CampaignMode::Claim,
                starts,
                ends,
            }),
            predecessor,
            cancelled: false,
        }
    }

    pub fn cancel(campaign_id: String, coordinate: String, predecessor: EventId) -> Self {
        Self {
            campaign_id,
            coordinate,
            terms: None,
            predecessor: Some(predecessor),
            cancelled: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CampaignTransition {
    Root(CampaignTerms),
    ReplaceTerms(CampaignTerms),
    Cancel,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CampaignStatus {
    Active,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CampaignEvent {
    pub event: Event,
    pub campaign_id: String,
    pub coordinate: String,
    pub predecessor: Option<EventId>,
    pub transition: CampaignTransition,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedCampaign {
    pub root_event_id: EventId,
    pub publisher_pubkey: PublicKey,
    pub campaign_id: String,
    pub coordinate: String,
    pub events: Vec<CampaignEvent>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CampaignState {
    pub status: CampaignStatus,
    pub terms: CampaignTerms,
}

fn parse_terms(event: &Event) -> Result<CampaignTerms, ProtocolEventError> {
    if exact_tag(event, TAG_MODE, true)?.as_deref() != Some("claim") {
        return Err(ProtocolEventError::MalformedTag(TAG_MODE.into()));
    }
    let starts = exact_tag(event, TAG_STARTS, true)?
        .ok_or(ProtocolEventError::MissingTag(TAG_STARTS))?
        .parse()
        .map_err(|_| ProtocolEventError::MalformedTag(TAG_STARTS.into()))?;
    let ends = exact_tag(event, TAG_ENDS, true)?
        .ok_or(ProtocolEventError::MissingTag(TAG_ENDS))?
        .parse()
        .map_err(|_| ProtocolEventError::MalformedTag(TAG_ENDS.into()))?;
    Ok(CampaignTerms {
        mode: CampaignMode::Claim,
        starts,
        ends,
    })
}

pub fn build_campaign_event_builder(
    params: &CampaignBuildParams,
) -> Result<nostr::EventBuilder, ProtocolEventError> {
    if params.campaign_id.is_empty() || coordinate_publisher(&params.coordinate).is_none() {
        return Err(ProtocolEventError::MalformedTag(TAG_IDENTIFIER.into()));
    }
    let mut tags = vec![
        nostr::Tag::parse([TAG_IDENTIFIER, params.campaign_id.as_str()])
            .map_err(|_| ProtocolEventError::MalformedTag(TAG_IDENTIFIER.into()))?,
        nostr::Tag::parse([TAG_COORDINATE, params.coordinate.as_str()])
            .map_err(|_| ProtocolEventError::MalformedTag(TAG_COORDINATE.into()))?,
        nostr::Tag::parse([
            TAG_STATUS,
            if params.cancelled {
                "cancelled"
            } else {
                "active"
            },
        ])
        .map_err(|_| ProtocolEventError::MalformedTag(TAG_STATUS.into()))?,
    ];
    if let Some(predecessor) = params.predecessor {
        tags.push(
            nostr::Tag::parse([TAG_PREDECESSOR, predecessor.to_hex().as_str()])
                .map_err(|_| ProtocolEventError::MalformedTag(TAG_PREDECESSOR.into()))?,
        );
    }
    if params.cancelled {
        if params.predecessor.is_none() || params.terms.is_some() {
            return Err(ProtocolEventError::MissingTag(TAG_PREDECESSOR));
        }
    } else {
        let terms = params
            .terms
            .as_ref()
            .ok_or(ProtocolEventError::MissingTag(TAG_STARTS))?;
        if terms.starts >= terms.ends {
            return Err(ProtocolEventError::MalformedTag(TAG_ENDS.into()));
        }
        tags.extend([
            nostr::Tag::parse([TAG_MODE, "claim"])
                .map_err(|_| ProtocolEventError::MalformedTag(TAG_MODE.into()))?,
            nostr::Tag::parse([TAG_STARTS, terms.starts.to_string().as_str()])
                .map_err(|_| ProtocolEventError::MalformedTag(TAG_STARTS.into()))?,
            nostr::Tag::parse([TAG_ENDS, terms.ends.to_string().as_str()])
                .map_err(|_| ProtocolEventError::MalformedTag(TAG_ENDS.into()))?,
        ]);
    }
    Ok(nostr::EventBuilder::new(nostr::Kind::Custom(ADP_CAMPAIGN_KIND), "").tags(tags))
}

pub fn parse_campaign_event(event: &Event) -> Result<CampaignEvent, ProtocolEventError> {
    if event.kind.as_u16() != ADP_CAMPAIGN_KIND {
        return Err(ProtocolEventError::WrongKind {
            expected: ADP_CAMPAIGN_KIND,
            found: event.kind.as_u16(),
        });
    }
    event
        .verify()
        .map_err(|_| ProtocolEventError::InvalidSignature)?;
    let campaign_id = exact_tag(event, TAG_IDENTIFIER, true)?
        .ok_or(ProtocolEventError::MissingTag(TAG_IDENTIFIER))?;
    let coordinate = exact_tag(event, TAG_COORDINATE, true)?
        .ok_or(ProtocolEventError::MissingTag(TAG_COORDINATE))?;
    if coordinate_publisher(&coordinate) != Some(event.pubkey) {
        return Err(ProtocolEventError::WrongPublisher);
    }
    let status =
        exact_tag(event, TAG_STATUS, true)?.ok_or(ProtocolEventError::MissingTag(TAG_STATUS))?;
    let predecessor = exact_tag(event, TAG_PREDECESSOR, false)?
        .map(|value| {
            EventId::from_hex(&value)
                .map_err(|_| ProtocolEventError::MalformedTag(TAG_PREDECESSOR.into()))
        })
        .transpose()?;
    let transition = match (status.as_str(), predecessor) {
        ("active", None) => CampaignTransition::Root(parse_terms(event)?),
        ("active", Some(_)) => CampaignTransition::ReplaceTerms(parse_terms(event)?),
        ("cancelled", Some(_)) => {
            for name in [TAG_MODE, TAG_STARTS, TAG_ENDS] {
                if exact_tag(event, name, false)?.is_some() {
                    return Err(ProtocolEventError::ForbiddenTag(name.into()));
                }
            }
            CampaignTransition::Cancel
        }
        ("cancelled", None) => return Err(ProtocolEventError::MissingTag(TAG_PREDECESSOR)),
        _ => return Err(ProtocolEventError::MalformedTag(TAG_STATUS.into())),
    };
    Ok(CampaignEvent {
        event: event.clone(),
        campaign_id,
        coordinate,
        predecessor,
        transition,
    })
}

fn valid_terms(terms: &CampaignTerms) -> Result<(), ChainError> {
    if terms.starts >= terms.ends {
        return Err(ChainError::InvalidTransition(
            "campaign starts must precede ends".into(),
        ));
    }
    Ok(())
}

pub fn resolve_campaign(
    nodes: &[CampaignEvent],
    expected_publisher: PublicKey,
    expected_coordinate: &str,
) -> Result<ResolvedCampaign, ChainError> {
    if nodes.is_empty() {
        return Err(ChainError::Empty);
    }
    let nodes = nodes
        .iter()
        .map(|node| {
            let parsed = parse_campaign_event(&node.event)
                .map_err(|error| ChainError::InvalidTransition(error.to_string()))?;
            if parsed != *node {
                return Err(ChainError::InvariantMutation("signed event metadata"));
            }
            Ok(parsed)
        })
        .collect::<Result<Vec<_>, _>>()?;
    for node in &nodes {
        if node.event.pubkey != expected_publisher {
            return Err(ChainError::InvariantMutation("publisher"));
        }
        if node.coordinate != expected_coordinate {
            return Err(ChainError::InvariantMutation(TAG_COORDINATE));
        }
        if node.predecessor == Some(node.event.id) {
            return Err(ChainError::Cycle);
        }
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
    if roots.len() != 1 {
        return Err(ChainError::MultipleRoots);
    }
    let root = roots[0];
    if nodes
        .iter()
        .any(|node| node.campaign_id != root.campaign_id)
    {
        return Err(ChainError::InvariantMutation(TAG_IDENTIFIER));
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
    let mut terms = match &ordered[0].transition {
        CampaignTransition::Root(terms) => {
            valid_terms(terms)?;
            if terms.starts < ordered[0].event.created_at.as_secs() {
                return Err(ChainError::InvalidTransition(
                    "campaign starts before publication".into(),
                ));
            }
            terms.clone()
        }
        _ => return Err(ChainError::InvalidTransition("root must be active".into())),
    };
    let mut cancelled = false;
    for pair in ordered.windows(2) {
        let previous = &pair[0];
        let next = &pair[1];
        if next.event.created_at <= previous.event.created_at {
            return Err(ChainError::TimestampRegression);
        }
        if cancelled {
            return Err(ChainError::InvalidTransition(
                "cancellation is terminal".into(),
            ));
        }
        match &next.transition {
            CampaignTransition::Root(_) => {
                return Err(ChainError::InvalidTransition(
                    "successor cannot be a root".into(),
                ));
            }
            CampaignTransition::ReplaceTerms(replacement) => {
                valid_terms(replacement)?;
                let updated_at = next.event.created_at.as_secs();
                if updated_at >= terms.starts || replacement.starts <= updated_at {
                    return Err(ChainError::InvalidTransition(
                        "campaign terms can only change before start".into(),
                    ));
                }
                terms = replacement.clone();
            }
            CampaignTransition::Cancel => cancelled = true,
        }
    }
    Ok(ResolvedCampaign {
        root_event_id: root.event.id,
        publisher_pubkey: expected_publisher,
        campaign_id: root.campaign_id.clone(),
        coordinate: expected_coordinate.into(),
        events: ordered,
    })
}

impl ResolvedCampaign {
    pub fn state_at(&self, at: u64) -> Option<CampaignState> {
        let resolved =
            resolve_campaign(&self.events, self.publisher_pubkey, &self.coordinate).ok()?;
        if resolved.root_event_id != self.root_event_id || resolved.campaign_id != self.campaign_id
        {
            return None;
        }
        let root = resolved.events.first()?;
        if at < root.event.created_at.as_secs() {
            return None;
        }
        let terms = match &root.transition {
            CampaignTransition::Root(terms) => terms.clone(),
            _ => return None,
        };
        let mut state = CampaignState {
            status: CampaignStatus::Active,
            terms,
        };
        for event in resolved.events.iter().skip(1) {
            if event.event.created_at.as_secs() > at {
                break;
            }
            match &event.transition {
                CampaignTransition::ReplaceTerms(terms) => state.terms = terms.clone(),
                CampaignTransition::Cancel => state.status = CampaignStatus::Cancelled,
                CampaignTransition::Root(_) => return None,
            }
        }
        Some(state)
    }

    pub fn is_claimable_at(&self, at: u64) -> bool {
        self.state_at(at).is_some_and(|state| {
            state.status == CampaignStatus::Active
                && state.terms.starts <= at
                && at < state.terms.ends
        })
    }
}
