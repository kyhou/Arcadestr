use std::collections::{HashMap, HashSet};

use nostr::{Alphabet, Event, Filter, Kind, PublicKey, SingleLetterTag};
use thiserror::Error;

use crate::adp_protocol::ADP_CAMPAIGN_KIND;
use crate::campaign::{parse_campaign_event, resolve_campaign, CampaignStatus, ResolvedCampaign};
use crate::marketplace::CampaignPointer;
use crate::relay_manager::{RelayManager, RelayManagerError};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CampaignClassification {
    Upcoming,
    Active,
    Ended,
    Cancelled,
    Invalid,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CampaignDiscoveryResult {
    pub campaign: ResolvedCampaign,
    pub classification: CampaignClassification,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CampaignDiscoveryReport {
    pub campaigns: Vec<CampaignDiscoveryResult>,
    pub invalid_campaign_ids: Vec<String>,
}

#[derive(Debug, Error)]
pub enum CampaignDiscoveryError {
    #[error(transparent)]
    Relay(#[from] RelayManagerError),
    #[error("invalid game coordinate")]
    InvalidCoordinate,
}

pub struct CampaignDiscoveryService<'a> {
    relays: &'a RelayManager,
}

impl<'a> CampaignDiscoveryService<'a> {
    pub fn new(relays: &'a RelayManager) -> Self {
        Self { relays }
    }

    pub async fn discover(
        &self,
        pointers: &[CampaignPointer],
        publisher: PublicKey,
        coordinate: &str,
        now: u64,
    ) -> Result<CampaignDiscoveryReport, CampaignDiscoveryError> {
        let mut pointer_events = Vec::new();
        for pointer in pointers {
            let filter = Filter::new().id(pointer.root_event_id);
            let fetched = if let Some(relay_hint) = &pointer.relay_hint {
                self.relays
                    .fetch_events_from_subset(filter.clone(), vec![relay_hint.clone()])
                    .await
                    .or_else(|_| Ok::<_, RelayManagerError>(Vec::new()))?
            } else {
                campaign_query_events(self.relays.fetch_events_best_effort(filter).await)?
            };
            pointer_events.extend(fetched);
        }

        let fallback_filter = Filter::new()
            .kind(Kind::Custom(ADP_CAMPAIGN_KIND))
            .custom_tags(
                SingleLetterTag::lowercase(Alphabet::A),
                [coordinate.to_owned()],
            );
        let fallback_events =
            campaign_query_events(self.relays.fetch_events_best_effort(fallback_filter).await)?;

        let mut identities = HashSet::new();
        for event in pointer_events.iter().chain(&fallback_events) {
            if let Ok(parsed) = parse_campaign_event(event) {
                if parsed.event.pubkey == publisher && parsed.coordinate == coordinate {
                    identities.insert(parsed.campaign_id);
                }
            }
        }

        let mut chain_events = Vec::new();
        for campaign_id in identities {
            let filter = Filter::new()
                .kind(Kind::Custom(ADP_CAMPAIGN_KIND))
                .author(publisher)
                .custom_tags(SingleLetterTag::lowercase(Alphabet::D), [campaign_id]);
            chain_events.extend(campaign_query_events(
                self.relays.fetch_events_best_effort(filter).await,
            )?);
        }

        Ok(resolve_campaign_candidates_report(
            pointers,
            &pointer_events,
            &fallback_events,
            &chain_events,
            publisher,
            coordinate,
            now,
        ))
    }
}

fn campaign_query_events(
    result: Result<Vec<Event>, RelayManagerError>,
) -> Result<Vec<Event>, CampaignDiscoveryError> {
    match result {
        Ok(events) => Ok(events),
        Err(RelayManagerError::QueryTimeout) => Ok(Vec::new()),
        Err(error) => Err(error.into()),
    }
}

pub fn resolve_campaign_candidates(
    pointers: &[CampaignPointer],
    pointer_events: &[Event],
    fallback_events: &[Event],
    chain_events: &[Event],
    publisher: PublicKey,
    coordinate: &str,
    now: u64,
) -> Vec<CampaignDiscoveryResult> {
    resolve_campaign_candidates_report(
        pointers,
        pointer_events,
        fallback_events,
        chain_events,
        publisher,
        coordinate,
        now,
    )
    .campaigns
}

pub fn resolve_campaign_candidates_report(
    pointers: &[CampaignPointer],
    pointer_events: &[Event],
    fallback_events: &[Event],
    chain_events: &[Event],
    publisher: PublicKey,
    coordinate: &str,
    now: u64,
) -> CampaignDiscoveryReport {
    let pointer_ids = pointers
        .iter()
        .map(|pointer| pointer.root_event_id)
        .collect::<HashSet<_>>();
    let mut campaign_ids = HashSet::new();
    for event in pointer_events
        .iter()
        .filter(|event| pointer_ids.contains(&event.id))
        .chain(fallback_events)
    {
        if let Ok(parsed) = parse_campaign_event(event) {
            if parsed.event.pubkey == publisher && parsed.coordinate == coordinate {
                campaign_ids.insert(parsed.campaign_id);
            }
        }
    }

    let mut groups: HashMap<String, Vec<_>> = HashMap::new();
    for event in chain_events
        .iter()
        .chain(pointer_events)
        .chain(fallback_events)
    {
        if let Ok(parsed) = parse_campaign_event(event) {
            if parsed.event.pubkey == publisher
                && parsed.coordinate == coordinate
                && campaign_ids.contains(&parsed.campaign_id)
            {
                groups
                    .entry(parsed.campaign_id.clone())
                    .or_default()
                    .push(parsed);
            }
        }
    }

    let mut report = CampaignDiscoveryReport::default();
    for (campaign_id, events) in groups {
        let mut seen = HashSet::new();
        let events = events
            .into_iter()
            .filter(|event| seen.insert(event.event.id))
            .collect::<Vec<_>>();
        match resolve_campaign(&events, publisher, coordinate) {
            Ok(campaign) => report.campaigns.push(CampaignDiscoveryResult {
                classification: classify_campaign(&campaign, now),
                campaign,
            }),
            Err(_) => report.invalid_campaign_ids.push(campaign_id),
        }
    }
    report
        .campaigns
        .sort_by(|left, right| left.campaign.campaign_id.cmp(&right.campaign.campaign_id));
    report.invalid_campaign_ids.sort();
    report
}

pub fn classify_campaign(campaign: &ResolvedCampaign, now: u64) -> CampaignClassification {
    let Some(state) = campaign.state_at(now) else {
        return CampaignClassification::Invalid;
    };
    if state.status == CampaignStatus::Cancelled {
        CampaignClassification::Cancelled
    } else if now < state.terms.starts {
        CampaignClassification::Upcoming
    } else if now < state.terms.ends {
        CampaignClassification::Active
    } else {
        CampaignClassification::Ended
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_campaign_query_is_not_an_error() {
        let events = campaign_query_events(Err(RelayManagerError::QueryTimeout))
            .expect("an empty relay result should mean no campaigns");
        assert!(events.is_empty());
    }
}
