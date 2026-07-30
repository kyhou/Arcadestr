use std::collections::{BTreeMap, HashSet};

use async_trait::async_trait;
use nostr::{Alphabet, Event, Filter, Kind, PublicKey, SingleLetterTag};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::adp_protocol::EXPERIMENTAL_STORE_PAGE_KIND;
use crate::relay_manager::RelayManager;
use crate::store_page::{
    listing_coordinate, parse_store_page_event, parse_store_page_pointers,
    resolve_store_page_events, store_page_coordinate, validate_store_page_association,
    validate_store_page_draft, NormalizedStorePage, ParsedStorePage, SanitizedStorePageContent,
    StorePageAssociation, StorePageBuildParams, StorePageContentV1, StorePageError,
    StorePagePointerDiagnostic, StorePagePointerError, StorePagePointerReport,
    StorePagePublishError,
};
use crate::store_page_repository::{
    StorePageCacheLookup, StorePageRepository, StorePageRepositoryError,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StorePageDiscoverySource {
    Cache,
    Relay,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveredStorePage {
    pub association: StorePageAssociation,
    pub source: StorePageDiscoverySource,
    pub relay_refresh_unavailable: bool,
    pub invalid_candidate_errors: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StorePageLookupResult {
    Associated(DiscoveredStorePage),
    NotAssociated {
        pointer_report: StorePagePointerReport,
        recovered_coordinates: Vec<String>,
    },
    MalformedPointer {
        pointer_report: StorePagePointerReport,
        recovered_coordinates: Vec<String>,
    },
    InvalidStorePage {
        pointer_report: StorePagePointerReport,
        errors: Vec<String>,
    },
    RelayUnavailable {
        pointer_report: StorePagePointerReport,
        cached_coordinate: Option<String>,
        reason: String,
    },
    StaleCachedPolicy {
        pointer_report: StorePagePointerReport,
        coordinate: String,
        reason: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StorePageRelayQuery {
    pub events: Vec<Event>,
    pub coverage_complete: bool,
}

#[async_trait]
pub trait StorePageRelaySource: Send + Sync {
    async fn add_advisory_relay(&self, relay: &str);
    async fn query(&self, filter: Filter) -> Result<StorePageRelayQuery, String>;
}

#[async_trait]
impl StorePageRelaySource for RelayManager {
    async fn add_advisory_relay(&self, relay: &str) {
        let _ = self.add_discovered_relay(relay.to_string()).await;
    }

    async fn query(&self, filter: Filter) -> Result<StorePageRelayQuery, String> {
        self.fetch_events(filter)
            .await
            .map(|events| StorePageRelayQuery {
                events,
                coverage_complete: false,
            })
            .map_err(|error| error.to_string())
    }
}

#[async_trait]
impl StorePageRelaySource for std::sync::Arc<tokio::sync::Mutex<RelayManager>> {
    async fn add_advisory_relay(&self, relay: &str) {
        let manager = self.lock().await.clone();
        let _ = manager.add_discovered_relay(relay.to_string()).await;
    }

    async fn query(&self, filter: Filter) -> Result<StorePageRelayQuery, String> {
        let manager = self.lock().await.clone();
        manager
            .fetch_events(filter)
            .await
            .map(|events| StorePageRelayQuery {
                events,
                coverage_complete: false,
            })
            .map_err(|error| error.to_string())
    }
}

#[derive(Debug, Error)]
pub enum StorePageDiscoveryError {
    #[error(transparent)]
    InvalidListing(#[from] StorePagePointerError),
    #[error(transparent)]
    Repository(#[from] StorePageRepositoryError),
    #[error("Store Page enrichment batch exceeds {0} listings")]
    BatchTooLarge(usize),
}

pub struct StorePageDiscoveryService<'a, R: StorePageRelaySource> {
    relays: &'a R,
    repository: &'a StorePageRepository,
}

impl<'a, R: StorePageRelaySource> StorePageDiscoveryService<'a, R> {
    pub fn new(relays: &'a R, repository: &'a StorePageRepository) -> Self {
        Self { relays, repository }
    }

    pub async fn discover(
        &self,
        listing: &Event,
    ) -> Result<StorePageLookupResult, StorePageDiscoveryError> {
        let pointer_report = parse_store_page_pointers(listing)?;
        let listing_coordinate = listing_coordinate(listing)?;
        let active_pointer = pointer_report.active.clone();
        let mut cached = None;
        let mut stale_policy = None;
        if let Some(pointer) = &active_pointer {
            match self.repository.load(&pointer.coordinate).await? {
                StorePageCacheLookup::Current(entry) => {
                    if let Ok(association) = validate_store_page_association(listing, &entry.parsed)
                    {
                        cached = Some(DiscoveredStorePage {
                            association,
                            source: StorePageDiscoverySource::Cache,
                            relay_refresh_unavailable: false,
                            invalid_candidate_errors: Vec::new(),
                        });
                    }
                }
                StorePageCacheLookup::StalePolicy {
                    coordinate, reason, ..
                } => {
                    stale_policy = Some((coordinate, reason));
                }
                StorePageCacheLookup::InvalidCachedEvent { .. } | StorePageCacheLookup::Missing => {
                }
            }
            if let Some(relay_hint) = &pointer.relay_hint {
                self.relays.add_advisory_relay(relay_hint).await;
            }
        }

        let mut invalid_errors = Vec::new();
        let mut relay_failure = None;
        let mut pointer_detached = false;
        let mut preferred_complete = false;
        if let Some(pointer) = &active_pointer {
            let filter = pointer_filter(pointer);
            match self.relays.query(filter).await {
                Ok(query) => {
                    preferred_complete = query.coverage_complete;
                    let (discovered, valid_current_seen) = self
                        .select_for_pointer(
                            listing,
                            pointer,
                            &query.events,
                            cached.as_ref(),
                            query.coverage_complete,
                            &mut invalid_errors,
                        )
                        .await?;
                    if let Some(discovered) = discovered {
                        return Ok(StorePageLookupResult::Associated(discovered));
                    }
                    if valid_current_seen {
                        cached = None;
                        pointer_detached = true;
                    }
                    if !query.coverage_complete {
                        relay_failure = Some("preferred relay coverage was incomplete".to_string());
                    }
                }
                Err(error) => relay_failure = Some(error),
            }
        }

        let recovery = match self
            .relays
            .query(recovery_filter(&listing_coordinate))
            .await
        {
            Ok(query) => Some(query),
            Err(error) => {
                relay_failure.get_or_insert(error);
                None
            }
        };
        let mut recovered_coordinates = Vec::new();
        if let Some(query) = recovery {
            let recovered =
                resolve_recovery_candidates(listing, &query.events, &mut invalid_errors);
            recovered_coordinates = recovered.keys().cloned().collect();
            for parsed in recovered.values() {
                let _ = self.repository.upsert_valid(parsed).await?;
            }
            if preferred_complete && !pointer_detached {
                if let Some(pointer) = &active_pointer {
                    if let Some(parsed) = recovered.get(&pointer.coordinate) {
                        if let Ok(association) = validate_store_page_association(listing, parsed) {
                            return Ok(StorePageLookupResult::Associated(DiscoveredStorePage {
                                association,
                                source: StorePageDiscoverySource::Relay,
                                relay_refresh_unavailable: !query.coverage_complete,
                                invalid_candidate_errors: invalid_errors,
                            }));
                        }
                    }
                }
            }
            if !query.coverage_complete {
                relay_failure.get_or_insert(
                    "listing-coordinate recovery coverage was incomplete".to_string(),
                );
            }
        }

        if let Some(mut cached) = cached {
            cached.relay_refresh_unavailable = relay_failure.is_some();
            cached.invalid_candidate_errors = invalid_errors;
            return Ok(StorePageLookupResult::Associated(cached));
        }
        if let Some((coordinate, reason)) = stale_policy {
            return Ok(StorePageLookupResult::StaleCachedPolicy {
                pointer_report,
                coordinate,
                reason,
            });
        }
        if let Some(reason) = relay_failure {
            return Ok(StorePageLookupResult::RelayUnavailable {
                pointer_report,
                cached_coordinate: active_pointer.map(|pointer| pointer.coordinate),
                reason,
            });
        }
        if !invalid_errors.is_empty() && active_pointer.is_some() {
            return Ok(StorePageLookupResult::InvalidStorePage {
                pointer_report,
                errors: invalid_errors,
            });
        }
        if pointer_report.pointer_tag_count > 0 && pointer_report.active.is_none() {
            return Ok(StorePageLookupResult::MalformedPointer {
                pointer_report,
                recovered_coordinates,
            });
        }
        Ok(StorePageLookupResult::NotAssociated {
            pointer_report,
            recovered_coordinates,
        })
    }

    async fn select_for_pointer(
        &self,
        listing: &Event,
        pointer: &crate::store_page::StorePagePointer,
        events: &[Event],
        cached: Option<&DiscoveredStorePage>,
        coverage_complete: bool,
        invalid_errors: &mut Vec<String>,
    ) -> Result<(Option<DiscoveredStorePage>, bool), StorePageDiscoveryError> {
        let publisher = PublicKey::from_hex(&pointer.publisher_pubkey)
            .map_err(|_| StorePagePointerError::InvalidListingIdentifier)?;
        let locally_matched = events
            .iter()
            .filter(|event| {
                event.pubkey == publisher
                    && event.kind.as_u16() == EXPERIMENTAL_STORE_PAGE_KIND
                    && event.tags.iter().any(|tag| {
                        let values = tag.clone().to_vec();
                        values.as_slice() == ["d", pointer.presentation_id.as_str()]
                    })
            })
            .collect::<Vec<_>>();
        for event in &locally_matched {
            if let Err(error) = parse_store_page_event(event) {
                invalid_errors.push(format!("{}: {error}", event.id.to_hex()));
            }
        }
        let mut candidates = locally_matched;
        if let Some(cached) = cached {
            candidates.push(&cached.association.store_page.event);
        }
        let Some(parsed) =
            resolve_store_page_events(candidates, publisher, &pointer.presentation_id)
        else {
            return Ok((None, false));
        };
        let source = if cached
            .is_some_and(|cached| cached.association.store_page.event.id == parsed.event.id)
        {
            StorePageDiscoverySource::Cache
        } else {
            let _ = self.repository.upsert_valid(&parsed).await?;
            StorePageDiscoverySource::Relay
        };
        let Ok(association) = validate_store_page_association(listing, &parsed) else {
            return Ok((None, true));
        };
        Ok((
            Some(DiscoveredStorePage {
                association,
                source,
                relay_refresh_unavailable: !coverage_complete,
                invalid_candidate_errors: invalid_errors.clone(),
            }),
            true,
        ))
    }
}

pub fn pointer_filter(pointer: &crate::store_page::StorePagePointer) -> Filter {
    let mut filter = Filter::new().kind(Kind::Custom(EXPERIMENTAL_STORE_PAGE_KIND));
    if let Ok(publisher) = PublicKey::from_hex(&pointer.publisher_pubkey) {
        filter = filter.author(publisher);
    }
    filter.custom_tags(
        SingleLetterTag::lowercase(Alphabet::D),
        [pointer.presentation_id.clone()],
    )
}

pub fn recovery_filter(listing_coordinate: &str) -> Filter {
    Filter::new()
        .kind(Kind::Custom(EXPERIMENTAL_STORE_PAGE_KIND))
        .custom_tags(
            SingleLetterTag::lowercase(Alphabet::A),
            [listing_coordinate.to_string()],
        )
}

pub fn resolve_recovery_candidates(
    listing: &Event,
    events: &[Event],
    invalid_errors: &mut Vec<String>,
) -> BTreeMap<String, ParsedStorePage> {
    let Ok(listing_coordinate) = listing_coordinate(listing) else {
        return BTreeMap::new();
    };
    let mut groups: BTreeMap<String, Vec<&Event>> = BTreeMap::new();
    for event in events {
        match parse_store_page_event(event) {
            Ok(parsed)
                if parsed.event.pubkey == listing.pubkey
                    && parsed.listing_coordinates.contains(&listing_coordinate) =>
            {
                groups
                    .entry(store_page_coordinate(
                        parsed.event.pubkey,
                        &parsed.presentation_id,
                    ))
                    .or_default()
                    .push(event);
            }
            Ok(_) => {}
            Err(error) => invalid_errors.push(format!("{}: {error}", event.id.to_hex())),
        }
    }

    groups
        .into_iter()
        .filter_map(|(coordinate, candidates)| {
            let first = candidates.first()?;
            let parsed = parse_store_page_event(first).ok()?;
            resolve_store_page_events(candidates, parsed.event.pubkey, &parsed.presentation_id)
                .map(|resolved| (coordinate, resolved))
        })
        .collect()
}

pub fn pointer_has_conflict(report: &StorePagePointerReport) -> bool {
    report.diagnostics.iter().any(|diagnostic| {
        matches!(
            diagnostic,
            StorePagePointerDiagnostic::DuplicatePointers
                | StorePagePointerDiagnostic::ConflictingPointers
        )
    })
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StorePageListingRef {
    pub listing_coordinate: String,
    pub listing_event_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StorePageCardPresentation {
    pub listing_coordinate: String,
    pub store_page_coordinate: String,
    pub event_id: String,
    pub title: Option<String>,
    pub summary: Option<String>,
    pub capsule_url: Option<String>,
    pub hero_url: Option<String>,
    pub genres: Vec<String>,
    pub features: Vec<String>,
    pub release_date: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", content = "presentation", rename_all = "snake_case")]
pub enum StorePageEnrichmentState {
    Enriched(StorePageCardPresentation),
    NotAssociated,
    NotFound,
    Invalid,
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StorePageEnrichmentResult {
    pub listing_coordinate: String,
    pub listing_event_id: String,
    pub state: StorePageEnrichmentState,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StorePageBatchEnrichment {
    pub generation: u64,
    pub cached: Vec<StorePageEnrichmentResult>,
    pub refreshed: Vec<StorePageEnrichmentResult>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StorePageDetailMedia {
    pub id: String,
    pub media_type: String,
    pub role: String,
    pub url: String,
    pub thumbnail_url: Option<String>,
    pub alt: Option<String>,
    pub caption: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StorePageDetailSection {
    pub id: String,
    pub heading: String,
    pub body_html: String,
    pub media_id: Option<String>,
    pub layout: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StorePageDetailLanguage {
    pub code: String,
    pub interface: bool,
    pub audio: bool,
    pub subtitles: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StorePageDetailRequirementTier {
    pub os: Option<String>,
    pub processor: Option<String>,
    pub memory: Option<String>,
    pub graphics: Option<String>,
    pub storage: Option<String>,
    pub additional: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StorePageDetailPlatformRequirements {
    pub platform: String,
    pub minimum: Option<StorePageDetailRequirementTier>,
    pub recommended: Option<StorePageDetailRequirementTier>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StorePageDetailAccessibility {
    pub feature: String,
    pub supported: bool,
    pub notes: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct StorePageDetailLinks {
    pub website: Option<String>,
    pub support: Option<String>,
    pub documentation: Option<String>,
    pub source: Option<String>,
    pub community: Option<String>,
    pub privacy_policy: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StorePageDetailPresentation {
    pub listing_coordinate: String,
    pub listing_event_id: String,
    pub store_page_coordinate: String,
    pub event_id: String,
    pub title: Option<String>,
    pub summary: Option<String>,
    pub description_html: Option<String>,
    pub media: Vec<StorePageDetailMedia>,
    pub sections: Vec<StorePageDetailSection>,
    pub genres: Vec<String>,
    pub features: Vec<String>,
    pub languages: Vec<StorePageDetailLanguage>,
    pub requirements: Vec<StorePageDetailPlatformRequirements>,
    pub accessibility: Vec<StorePageDetailAccessibility>,
    pub links: StorePageDetailLinks,
    pub developer: Option<String>,
    pub publisher: Option<String>,
    pub release_date: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", content = "presentation", rename_all = "snake_case")]
pub enum StorePageDetailState {
    Enriched(StorePageDetailPresentation),
    NotAssociated,
    NotFound,
    Invalid,
    Unsupported,
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StorePageDetailEnrichment {
    pub generation: u64,
    pub listing_event_current: bool,
    pub cached: Option<StorePageDetailPresentation>,
    pub refreshed: StorePageDetailState,
}

pub struct StorePageBatchService<'a, R: StorePageRelaySource> {
    relays: &'a R,
    repository: &'a StorePageRepository,
}

pub const MAX_STORE_PAGE_ENRICHMENT_BATCH: usize = 64;

impl<'a, R: StorePageRelaySource> StorePageBatchService<'a, R> {
    pub fn new(relays: &'a R, repository: &'a StorePageRepository) -> Self {
        Self { relays, repository }
    }

    pub async fn enrich(
        &self,
        generation: u64,
        requested: &[StorePageListingRef],
    ) -> Result<StorePageBatchEnrichment, StorePageDiscoveryError> {
        if requested.len() > MAX_STORE_PAGE_ENRICHMENT_BATCH {
            return Err(StorePageDiscoveryError::BatchTooLarge(
                MAX_STORE_PAGE_ENRICHMENT_BATCH,
            ));
        }
        let requested = requested
            .iter()
            .filter_map(|item| {
                parse_listing_coordinate_ref(&item.listing_coordinate).map(
                    |(coordinate, (author, identifier))| {
                        (
                            coordinate,
                            (author, identifier, item.listing_event_id.clone()),
                        )
                    },
                )
            })
            .collect::<BTreeMap<_, _>>();
        if requested.is_empty() {
            return Ok(StorePageBatchEnrichment {
                generation,
                cached: Vec::new(),
                refreshed: Vec::new(),
            });
        }
        let authors = requested
            .values()
            .map(|(author, _, _)| *author)
            .collect::<HashSet<_>>();
        let identifiers = requested
            .values()
            .map(|(_, identifier, _)| identifier.clone())
            .collect::<HashSet<_>>();
        let listing_query = self
            .relays
            .query(
                Filter::new()
                    .kind(Kind::Custom(crate::adp_protocol::NIP99_LISTING_KIND))
                    .authors(authors)
                    .custom_tags(SingleLetterTag::lowercase(Alphabet::D), identifiers),
            )
            .await;
        let (mut listing_events, listing_coverage_complete) = match listing_query {
            Ok(query) => (query.events, query.coverage_complete),
            Err(_) => (Vec::new(), false),
        };
        for (coordinate, (_, _, expected_event_id)) in &requested {
            if let Some(cached_listing) = self.repository.load_listing_event(coordinate).await? {
                if cached_listing.id.to_hex() == *expected_event_id {
                    listing_events.push(cached_listing);
                }
            }
        }
        let listings = select_current_listings(&requested, &listing_events);
        let mut cached = Vec::new();
        let mut cached_pages = BTreeMap::new();
        let mut pointers = BTreeMap::new();
        let mut refreshed = BTreeMap::new();

        for coordinate in requested.keys() {
            let Some(listing) = listings.get(coordinate) else {
                refreshed.insert(
                    coordinate.clone(),
                    if listing_coverage_complete {
                        StorePageEnrichmentState::NotFound
                    } else {
                        StorePageEnrichmentState::Unavailable
                    },
                );
                continue;
            };
            if requested
                .get(coordinate)
                .is_none_or(|(_, _, event_id)| event_id != &listing.id.to_hex())
            {
                refreshed.insert(coordinate.clone(), StorePageEnrichmentState::Unavailable);
                continue;
            }
            let _ = self.repository.upsert_listing_event(listing).await?;
            let Ok(report) = parse_store_page_pointers(listing) else {
                refreshed.insert(coordinate.clone(), StorePageEnrichmentState::Invalid);
                continue;
            };
            let Some(pointer) = report.active else {
                refreshed.insert(coordinate.clone(), StorePageEnrichmentState::NotAssociated);
                continue;
            };
            pointers.insert(coordinate.clone(), pointer.clone());
            if let StorePageCacheLookup::Current(entry) =
                self.repository.load(&pointer.coordinate).await?
            {
                if let Ok(association) = validate_store_page_association(listing, &entry.parsed) {
                    let presentation = card_presentation(&association);
                    cached.push(enrichment_result(
                        coordinate,
                        &requested,
                        StorePageEnrichmentState::Enriched(presentation),
                    ));
                    cached_pages.insert(coordinate.clone(), entry.parsed);
                }
            }
        }

        let mut grouped: BTreeMap<String, (PublicKey, HashSet<String>)> = BTreeMap::new();
        for pointer in pointers.values() {
            if let Ok(author) = PublicKey::from_hex(&pointer.publisher_pubkey) {
                grouped
                    .entry(pointer.publisher_pubkey.clone())
                    .or_insert_with(|| (author, HashSet::new()))
                    .1
                    .insert(pointer.presentation_id.clone());
            }
        }
        let mut page_events = Vec::new();
        let mut page_queries_complete = true;
        for (_, (author, identifiers)) in grouped {
            match self
                .relays
                .query(
                    Filter::new()
                        .kind(Kind::Custom(EXPERIMENTAL_STORE_PAGE_KIND))
                        .author(author)
                        .custom_tags(SingleLetterTag::lowercase(Alphabet::D), identifiers),
                )
                .await
            {
                Ok(query) => {
                    page_queries_complete &= query.coverage_complete;
                    page_events.extend(query.events);
                }
                Err(_) => page_queries_complete = false,
            }
        }
        match self
            .relays
            .query(
                Filter::new()
                    .kind(Kind::Custom(EXPERIMENTAL_STORE_PAGE_KIND))
                    .custom_tags(
                        SingleLetterTag::lowercase(Alphabet::A),
                        requested.keys().cloned(),
                    ),
            )
            .await
        {
            Ok(query) => {
                page_queries_complete &= query.coverage_complete;
                page_events.extend(query.events);
            }
            Err(_) => page_queries_complete = false,
        }

        for (coordinate, pointer) in &pointers {
            let Some(listing) = listings.get(coordinate) else {
                continue;
            };
            let Ok(author) = PublicKey::from_hex(&pointer.publisher_pubkey) else {
                refreshed.insert(coordinate.clone(), StorePageEnrichmentState::Invalid);
                continue;
            };
            let mut candidates = page_events
                .iter()
                .filter(|event| event.pubkey == author)
                .collect::<Vec<_>>();
            if let Some(cached) = cached_pages.get(coordinate) {
                candidates.push(&cached.event);
            }
            if let Some(parsed) =
                resolve_store_page_events(candidates, author, &pointer.presentation_id)
            {
                let _ = self.repository.upsert_valid(&parsed).await?;
                match validate_store_page_association(listing, &parsed) {
                    Ok(association) => {
                        refreshed.insert(
                            coordinate.clone(),
                            StorePageEnrichmentState::Enriched(card_presentation(&association)),
                        );
                    }
                    Err(_) => {
                        refreshed
                            .insert(coordinate.clone(), StorePageEnrichmentState::NotAssociated);
                    }
                }
            } else if cached_pages.contains_key(coordinate) {
                let association = validate_store_page_association(
                    listing,
                    cached_pages.get(coordinate).expect("cache checked"),
                );
                if let Ok(association) = association {
                    refreshed.insert(
                        coordinate.clone(),
                        StorePageEnrichmentState::Enriched(card_presentation(&association)),
                    );
                }
            } else {
                refreshed.insert(
                    coordinate.clone(),
                    if page_queries_complete {
                        StorePageEnrichmentState::NotFound
                    } else {
                        StorePageEnrichmentState::Unavailable
                    },
                );
            }
        }

        Ok(StorePageBatchEnrichment {
            generation,
            cached,
            refreshed: requested
                .keys()
                .map(|coordinate| {
                    enrichment_result(
                        coordinate,
                        &requested,
                        refreshed
                            .remove(coordinate)
                            .unwrap_or(StorePageEnrichmentState::NotAssociated),
                    )
                })
                .collect(),
        })
    }

    pub async fn enrich_detail(
        &self,
        generation: u64,
        requested: &StorePageListingRef,
    ) -> Result<StorePageDetailEnrichment, StorePageDiscoveryError> {
        let Some((coordinate, (author, identifier))) =
            parse_listing_coordinate_ref(&requested.listing_coordinate)
        else {
            return Ok(detail_result(
                generation,
                false,
                None,
                StorePageDetailState::Invalid,
            ));
        };
        let listing_query = self
            .relays
            .query(
                Filter::new()
                    .kind(Kind::Custom(crate::adp_protocol::NIP99_LISTING_KIND))
                    .author(author)
                    .custom_tags(
                        SingleLetterTag::lowercase(Alphabet::D),
                        [identifier.clone()],
                    ),
            )
            .await;
        let mut listing_events = listing_query.map(|query| query.events).unwrap_or_default();
        if let Some(cached_listing) = self.repository.load_listing_event(&coordinate).await? {
            if cached_listing.id.to_hex() == requested.listing_event_id {
                listing_events.push(cached_listing);
            }
        }
        let expected = BTreeMap::from([(
            coordinate.clone(),
            (author, identifier, requested.listing_event_id.clone()),
        )]);
        let listings = select_current_listings(&expected, &listing_events);
        let Some(listing) = listings.get(&coordinate) else {
            return Ok(detail_result(
                generation,
                false,
                None,
                StorePageDetailState::Unavailable,
            ));
        };
        if listing.id.to_hex() != requested.listing_event_id {
            return Ok(detail_result(
                generation,
                false,
                None,
                StorePageDetailState::Unavailable,
            ));
        }
        let _ = self.repository.upsert_listing_event(listing).await?;
        let Ok(report) = parse_store_page_pointers(listing) else {
            return Ok(detail_result(
                generation,
                true,
                None,
                StorePageDetailState::Invalid,
            ));
        };
        let Some(pointer) = report.active else {
            return Ok(detail_result(
                generation,
                true,
                None,
                StorePageDetailState::NotAssociated,
            ));
        };
        let mut cached_parsed = None;
        let mut cached = None;
        if let StorePageCacheLookup::Current(entry) =
            self.repository.load(&pointer.coordinate).await?
        {
            if let Ok(association) = validate_store_page_association(listing, &entry.parsed) {
                cached = Some(detail_presentation(&association, listing));
                cached_parsed = Some(entry.parsed);
            }
        }
        if let Some(relay_hint) = &pointer.relay_hint {
            self.relays.add_advisory_relay(relay_hint).await;
        }
        let mut events = Vec::new();
        let mut coverage_complete = true;
        for filter in [pointer_filter(&pointer), recovery_filter(&coordinate)] {
            match self.relays.query(filter).await {
                Ok(query) => {
                    coverage_complete &= query.coverage_complete;
                    events.extend(query.events);
                }
                Err(_) => coverage_complete = false,
            }
        }
        let mut candidates = events.iter().collect::<Vec<_>>();
        let mut unsupported_candidate = false;
        let mut invalid_candidate = false;
        for event in &events {
            if event.pubkey != author
                || !event.tags.iter().any(|tag| {
                    let values = tag.clone().to_vec();
                    values.as_slice() == ["d", pointer.presentation_id.as_str()]
                })
            {
                continue;
            }
            match parse_store_page_event(event) {
                Err(
                    StorePageError::UnsupportedSchema(_)
                    | StorePageError::UnsupportedSchemaVersion(_),
                ) => unsupported_candidate = true,
                Err(_) => invalid_candidate = true,
                Ok(_) => {}
            }
        }
        if let Some(parsed) = &cached_parsed {
            candidates.push(&parsed.event);
        }
        let refreshed =
            match resolve_store_page_events(candidates, author, &pointer.presentation_id) {
                Some(parsed) => {
                    if !coverage_complete
                        && cached_parsed
                            .as_ref()
                            .is_some_and(|cached| cached.event.id == parsed.event.id)
                    {
                        return Ok(detail_result(
                            generation,
                            true,
                            cached,
                            StorePageDetailState::Unavailable,
                        ));
                    }
                    let _ = self.repository.upsert_valid(&parsed).await?;
                    match validate_store_page_association(listing, &parsed) {
                        Ok(association) => StorePageDetailState::Enriched(detail_presentation(
                            &association,
                            listing,
                        )),
                        Err(_) => StorePageDetailState::NotAssociated,
                    }
                }
                None if unsupported_candidate => StorePageDetailState::Unsupported,
                None if invalid_candidate => StorePageDetailState::Invalid,
                None if coverage_complete => StorePageDetailState::NotFound,
                None => StorePageDetailState::Unavailable,
            };
        Ok(detail_result(generation, true, cached, refreshed))
    }
}

fn detail_result(
    generation: u64,
    listing_event_current: bool,
    cached: Option<StorePageDetailPresentation>,
    refreshed: StorePageDetailState,
) -> StorePageDetailEnrichment {
    StorePageDetailEnrichment {
        generation,
        listing_event_current,
        cached,
        refreshed,
    }
}

fn parse_listing_coordinate_ref(value: &str) -> Option<(String, (PublicKey, String))> {
    if value.len() > 512 {
        return None;
    }
    let mut parts = value.splitn(3, ':');
    if parts.next()?.parse::<u16>().ok()? != crate::adp_protocol::NIP99_LISTING_KIND {
        return None;
    }
    let author = PublicKey::from_hex(parts.next()?).ok()?;
    let identifier = parts.next()?.to_string();
    if identifier.is_empty()
        || identifier.chars().count() > crate::store_page_content_policy::MAX_IDENTIFIER_CHARS
    {
        return None;
    }
    Some((value.to_string(), (author, identifier)))
}

fn select_current_listings(
    requested: &BTreeMap<String, (PublicKey, String, String)>,
    events: &[Event],
) -> BTreeMap<String, Event> {
    let mut selected = BTreeMap::new();
    for event in events {
        if event.kind.as_u16() != crate::adp_protocol::NIP99_LISTING_KIND || event.verify().is_err()
        {
            continue;
        }
        let identifiers = event
            .tags
            .iter()
            .filter_map(|tag| {
                let values = tag.clone().to_vec();
                (values.len() == 2 && values.first().is_some_and(|name| name == "d"))
                    .then(|| values[1].clone())
            })
            .collect::<Vec<_>>();
        let [identifier] = identifiers.as_slice() else {
            continue;
        };
        let coordinate = format!(
            "{}:{}:{}",
            crate::adp_protocol::NIP99_LISTING_KIND,
            event.pubkey.to_hex(),
            identifier
        );
        let Some((author, expected_identifier, _)) = requested.get(&coordinate) else {
            continue;
        };
        if *author != event.pubkey || expected_identifier != identifier {
            continue;
        }
        let replace = selected.get(&coordinate).map_or(true, |current: &Event| {
            crate::is_replaceable_event_newer(
                event.created_at.as_secs(),
                Some(event.id.to_hex().as_str()),
                current.created_at.as_secs(),
                Some(current.id.to_hex().as_str()),
            )
        });
        if replace {
            selected.insert(coordinate, event.clone());
        }
    }
    selected
}

fn card_presentation(association: &StorePageAssociation) -> StorePageCardPresentation {
    let page = &association.store_page;
    let media = &page.sanitized_content().media;
    StorePageCardPresentation {
        listing_coordinate: association.listing_coordinate.clone(),
        store_page_coordinate: association.pointer.coordinate.clone(),
        event_id: page.event.id.to_hex(),
        title: page.normalized.title.clone(),
        summary: page.normalized.summary.clone(),
        capsule_url: media
            .iter()
            .find(|item| item.role == "capsule" && item.media_type == "image")
            .map(|item| item.url.clone()),
        hero_url: media
            .iter()
            .find(|item| item.role == "hero" && item.media_type == "image")
            .map(|item| item.url.clone()),
        genres: page.normalized.genres.clone(),
        features: page.normalized.features.clone(),
        release_date: page.normalized.release_date.clone(),
    }
}

fn detail_presentation(
    association: &StorePageAssociation,
    listing: &Event,
) -> StorePageDetailPresentation {
    let page = &association.store_page;
    detail_presentation_from_parts(
        association.listing_coordinate.clone(),
        listing,
        association.pointer.coordinate.clone(),
        page.event.id.to_hex(),
        &page.content,
        &page.normalized,
        page.sanitized_content(),
    )
}

pub fn preview_store_page_detail(
    params: &StorePageBuildParams,
    listing: &Event,
) -> Result<StorePageDetailPresentation, StorePagePublishError> {
    if listing.kind.as_u16() != crate::adp_protocol::NIP99_LISTING_KIND
        || listing.verify().is_err()
        || listing.pubkey != params.publisher
    {
        return Err(StorePagePublishError::InvalidListing);
    }
    let listing_coordinate =
        listing_coordinate(listing).map_err(|_| StorePagePublishError::InvalidListing)?;
    if !params.listing_coordinates.contains(&listing_coordinate) {
        return Err(StorePagePublishError::InvalidPointer(
            "the preview listing is not associated with this draft".to_string(),
        ));
    }
    let validated = validate_store_page_draft(params)?;
    Ok(detail_presentation_from_parts(
        listing_coordinate,
        listing,
        store_page_coordinate(params.publisher, &params.presentation_id),
        "draft-preview".to_string(),
        &params.content,
        &validated.normalized,
        validated.sanitized_content(),
    ))
}

fn detail_presentation_from_parts(
    listing_coordinate: String,
    listing: &Event,
    store_page_coordinate: String,
    event_id: String,
    content: &StorePageContentV1,
    normalized: &NormalizedStorePage,
    sanitized: &SanitizedStorePageContent,
) -> StorePageDetailPresentation {
    let listing_platforms = listing
        .tags
        .iter()
        .filter_map(|tag| {
            let values = tag.clone().to_vec();
            (values.len() == 2 && values.first().is_some_and(|name| name == "platform"))
                .then(|| values[1].clone())
        })
        .collect::<HashSet<_>>();
    let requirements = content
        .requirements
        .iter()
        .filter(|(platform, _)| listing_platforms.contains(*platform))
        .map(
            |(platform, requirement)| StorePageDetailPlatformRequirements {
                platform: platform.clone(),
                minimum: requirement.minimum.as_ref().map(detail_requirement_tier),
                recommended: requirement
                    .recommended
                    .as_ref()
                    .map(detail_requirement_tier),
            },
        )
        .collect();
    StorePageDetailPresentation {
        listing_coordinate,
        listing_event_id: listing.id.to_hex(),
        store_page_coordinate,
        event_id,
        title: normalized.title.clone(),
        summary: normalized.summary.clone(),
        description_html: (!sanitized.description_html.as_str().is_empty())
            .then(|| sanitized.description_html.as_str().to_string()),
        media: sanitized
            .media
            .iter()
            .map(|item| StorePageDetailMedia {
                id: item.id.clone(),
                media_type: item.media_type.clone(),
                role: item.role.clone(),
                url: item.url.clone(),
                thumbnail_url: item.thumbnail_url.clone(),
                alt: item.alt.clone(),
                caption: item.caption.clone(),
            })
            .collect(),
        sections: sanitized
            .sections
            .iter()
            .map(|section| StorePageDetailSection {
                id: section.id.clone(),
                heading: section.heading.clone(),
                body_html: section.body_html.as_str().to_string(),
                media_id: section.media_id.clone(),
                layout: section.layout.clone(),
            })
            .collect(),
        genres: normalized.genres.clone(),
        features: normalized.features.clone(),
        languages: content
            .languages
            .clone()
            .unwrap_or_default()
            .into_iter()
            .map(|language| StorePageDetailLanguage {
                code: language.code,
                interface: language.interface,
                audio: language.audio,
                subtitles: language.subtitles,
            })
            .collect(),
        requirements,
        accessibility: content
            .accessibility
            .iter()
            .map(|entry| StorePageDetailAccessibility {
                feature: entry.feature.clone(),
                supported: entry.supported,
                notes: entry.notes.clone(),
            })
            .collect(),
        links: StorePageDetailLinks {
            website: sanitized.links.website.clone(),
            support: sanitized.links.support.clone(),
            documentation: sanitized.links.documentation.clone(),
            source: sanitized.links.source.clone(),
            community: sanitized.links.community.clone(),
            privacy_policy: sanitized.links.privacy_policy.clone(),
        },
        developer: normalized.developer.clone(),
        publisher: normalized.publisher.clone(),
        release_date: normalized.release_date.clone(),
    }
}

fn detail_requirement_tier(
    tier: &crate::store_page::RequirementTier,
) -> StorePageDetailRequirementTier {
    StorePageDetailRequirementTier {
        os: tier.os.clone(),
        processor: tier.processor.clone(),
        memory: tier.memory.clone(),
        graphics: tier.graphics.clone(),
        storage: tier.storage.clone(),
        additional: tier.additional.clone(),
    }
}

fn enrichment_result(
    coordinate: &str,
    requested: &BTreeMap<String, (PublicKey, String, String)>,
    state: StorePageEnrichmentState,
) -> StorePageEnrichmentResult {
    StorePageEnrichmentResult {
        listing_coordinate: coordinate.to_string(),
        listing_event_id: requested
            .get(coordinate)
            .map(|(_, _, event_id)| event_id.clone())
            .unwrap_or_default(),
        state,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::sync::Mutex;

    use nostr::{EventBuilder, Keys, Tag, Timestamp};
    use tempfile::TempDir;

    use super::*;
    use crate::adp_protocol::NIP99_LISTING_KIND;
    use crate::storage::Database;
    use crate::store_page::{
        build_store_page_event_builder, StorePageBasic, StorePageBuildParams, StorePageCompactTags,
        StorePageContentV1,
    };

    struct FakeRelays {
        responses: Mutex<VecDeque<Result<StorePageRelayQuery, String>>>,
        advisory_relays: Mutex<Vec<String>>,
        query_count: Mutex<usize>,
    }

    impl FakeRelays {
        fn new(responses: Vec<Result<StorePageRelayQuery, String>>) -> Self {
            Self {
                responses: Mutex::new(responses.into()),
                advisory_relays: Mutex::new(Vec::new()),
                query_count: Mutex::new(0),
            }
        }
    }

    #[async_trait]
    impl StorePageRelaySource for FakeRelays {
        async fn add_advisory_relay(&self, relay: &str) {
            self.advisory_relays
                .lock()
                .expect("advisory relay lock")
                .push(relay.to_string());
        }

        async fn query(&self, _filter: Filter) -> Result<StorePageRelayQuery, String> {
            *self.query_count.lock().expect("query count lock") += 1;
            self.responses
                .lock()
                .expect("response lock")
                .pop_front()
                .unwrap_or_else(|| {
                    Ok(StorePageRelayQuery {
                        events: Vec::new(),
                        coverage_complete: true,
                    })
                })
        }
    }

    async fn repository() -> (TempDir, StorePageRepository) {
        let directory = TempDir::new().expect("temp directory");
        let database = Database::new(&directory.path().join("discovery.db"))
            .await
            .expect("database");
        (directory, StorePageRepository::new(database.pool().clone()))
    }

    fn signed_listing(keys: &Keys, id: &str, pointer_id: Option<&str>) -> Event {
        let mut tags = vec![Tag::parse(["d", id]).expect("d tag")];
        if let Some(pointer_id) = pointer_id {
            tags.push(
                Tag::parse([
                    "store_page",
                    store_page_coordinate(keys.public_key(), pointer_id).as_str(),
                ])
                .expect("pointer tag"),
            );
        }
        EventBuilder::new(Kind::Custom(NIP99_LISTING_KIND), "listing")
            .tags(tags)
            .sign_with_keys(keys)
            .expect("signed listing")
    }

    fn page(keys: &Keys, id: &str, listings: Vec<String>, title: &str, at: u64) -> Event {
        build_store_page_event_builder(&StorePageBuildParams {
            publisher: keys.public_key(),
            presentation_id: id.to_string(),
            listing_coordinates: listings,
            content: StorePageContentV1 {
                basic: StorePageBasic {
                    title: Some(title.to_string()),
                    ..StorePageBasic::default()
                },
                ..StorePageContentV1::default()
            },
            compact_tags: StorePageCompactTags::default(),
        })
        .expect("page builder")
        .custom_created_at(Timestamp::from(at))
        .sign_with_keys(keys)
        .expect("signed page")
    }

    fn query(events: Vec<Event>) -> Result<StorePageRelayQuery, String> {
        Ok(StorePageRelayQuery {
            events,
            coverage_complete: true,
        })
    }

    #[test]
    fn discovery_filters_encode_exact_pointer_and_recovery_coordinates() {
        let keys = Keys::generate();
        let listing = signed_listing(&keys, "game", Some("page"));
        let report = parse_store_page_pointers(&listing).expect("pointer report");
        let pointer = report.active.expect("active pointer");
        let preferred = serde_json::to_value(pointer_filter(&pointer)).expect("preferred filter");
        assert_eq!(
            preferred["kinds"],
            serde_json::json!([EXPERIMENTAL_STORE_PAGE_KIND])
        );
        assert_eq!(
            preferred["authors"],
            serde_json::json!([keys.public_key().to_hex()])
        );
        assert_eq!(preferred["#d"], serde_json::json!(["page"]));

        let coordinate = listing_coordinate(&listing).expect("listing coordinate");
        let recovery = serde_json::to_value(recovery_filter(&coordinate)).expect("recovery filter");
        assert_eq!(recovery["#a"], serde_json::json!([coordinate]));
    }

    #[tokio::test]
    async fn relay_hint_is_added_advisory_without_replacing_normal_query() {
        let (_directory, repository) = repository().await;
        let keys = Keys::generate();
        let coordinate = store_page_coordinate(keys.public_key(), "page");
        let listing = EventBuilder::new(Kind::Custom(NIP99_LISTING_KIND), "listing")
            .tags([
                Tag::parse(["d", "game"]).expect("d tag"),
                Tag::parse(["store_page", coordinate.as_str(), "wss://hint.example.com"])
                    .expect("pointer tag"),
            ])
            .sign_with_keys(&keys)
            .expect("signed listing");
        let relays = FakeRelays::new(vec![query(Vec::new()), query(Vec::new())]);
        let _ = StorePageDiscoveryService::new(&relays, &repository)
            .discover(&listing)
            .await
            .expect("discovery");
        assert_eq!(
            relays
                .advisory_relays
                .lock()
                .expect("advisory relay lock")
                .as_slice(),
            ["wss://hint.example.com/"]
        );
    }

    #[tokio::test]
    async fn pointer_fast_path_discovers_and_persists_reciprocal_page() {
        let (_directory, repository) = repository().await;
        let keys = Keys::generate();
        let listing = signed_listing(&keys, "game", Some("page"));
        let page = page(
            &keys,
            "page",
            vec![listing_coordinate(&listing).expect("coordinate")],
            "Title",
            10,
        );
        let relays = FakeRelays::new(vec![query(vec![page.clone()])]);
        let result = StorePageDiscoveryService::new(&relays, &repository)
            .discover(&listing)
            .await
            .expect("discovery");
        let StorePageLookupResult::Associated(discovered) = result else {
            panic!("expected association");
        };
        assert_eq!(discovered.source, StorePageDiscoverySource::Relay);
        assert_eq!(discovered.association.store_page.event.id, page.id);
        assert!(matches!(
            repository
                .load(&store_page_coordinate(keys.public_key(), "page"))
                .await
                .expect("cache"),
            StorePageCacheLookup::Current(_)
        ));
    }

    #[tokio::test]
    async fn recovery_locates_but_does_not_attach_without_pointer() {
        let (_directory, repository) = repository().await;
        let keys = Keys::generate();
        let listing = signed_listing(&keys, "game", None);
        let page = page(
            &keys,
            "page",
            vec![listing_coordinate(&listing).expect("coordinate")],
            "Title",
            10,
        );
        let relays = FakeRelays::new(vec![query(vec![page])]);
        let result = StorePageDiscoveryService::new(&relays, &repository)
            .discover(&listing)
            .await
            .expect("discovery");
        let StorePageLookupResult::NotAssociated {
            recovered_coordinates,
            ..
        } = result
        else {
            panic!("recovery must not attach one-sided page");
        };
        assert_eq!(recovered_coordinates.len(), 1);
    }

    #[tokio::test]
    async fn recovery_rejects_false_author_and_identifier_tuple_matches() {
        let (_directory, repository) = repository().await;
        let keys = Keys::generate();
        let foreign = Keys::generate();
        let listing = signed_listing(&keys, "game", None);
        let foreign_listing = format!(
            "{NIP99_LISTING_KIND}:{}:game",
            foreign.public_key().to_hex()
        );
        let false_candidate = page(&foreign, "page", vec![foreign_listing], "False", 10);
        let wrong_listing = signed_listing(&keys, "other", None);
        let wrong_d_candidate = page(
            &keys,
            "page",
            vec![listing_coordinate(&wrong_listing).expect("coordinate")],
            "Wrong",
            10,
        );
        let relays = FakeRelays::new(vec![query(vec![false_candidate, wrong_d_candidate])]);
        let result = StorePageDiscoveryService::new(&relays, &repository)
            .discover(&listing)
            .await
            .expect("discovery");
        assert!(matches!(
            result,
            StorePageLookupResult::NotAssociated {
                recovered_coordinates,
                ..
            } if recovered_coordinates.is_empty()
        ));
    }

    #[tokio::test]
    async fn invalid_newer_candidate_retains_valid_cached_page() {
        let (_directory, repository) = repository().await;
        let keys = Keys::generate();
        let listing = signed_listing(&keys, "game", Some("page"));
        let coordinate = listing_coordinate(&listing).expect("coordinate");
        let cached = page(&keys, "page", vec![coordinate.clone()], "Cached", 10);
        let parsed_cached = parse_store_page_event(&cached).expect("cached page");
        repository
            .upsert_valid(&parsed_cached)
            .await
            .expect("cache insert");
        let mut invalid_content = parsed_cached.content.clone();
        invalid_content.version += 1;
        let invalid = EventBuilder::new(
            Kind::Custom(EXPERIMENTAL_STORE_PAGE_KIND),
            serde_json::to_string(&invalid_content).expect("content JSON"),
        )
        .tags(cached.tags.clone())
        .custom_created_at(Timestamp::from(20))
        .sign_with_keys(&keys)
        .expect("signed invalid page");
        let relays = FakeRelays::new(vec![query(vec![invalid]), query(Vec::new())]);
        let result = StorePageDiscoveryService::new(&relays, &repository)
            .discover(&listing)
            .await
            .expect("discovery");
        let StorePageLookupResult::Associated(discovered) = result else {
            panic!("cached page should remain associated");
        };
        assert_eq!(discovered.source, StorePageDiscoverySource::Cache);
        assert_eq!(discovered.association.store_page.event.id, cached.id);
        assert!(!discovered.invalid_candidate_errors.is_empty());
    }

    #[tokio::test]
    async fn missing_cache_and_only_invalid_candidates_reports_invalid_page() {
        let (_directory, repository) = repository().await;
        let keys = Keys::generate();
        let listing = signed_listing(&keys, "game", Some("page"));
        let valid_shape = page(
            &keys,
            "page",
            vec![listing_coordinate(&listing).expect("coordinate")],
            "Shape",
            10,
        );
        let mut unsupported: serde_json::Value =
            serde_json::from_str(&valid_shape.content).expect("content JSON");
        unsupported["version"] = serde_json::json!(999);
        let invalid = EventBuilder::new(
            Kind::Custom(EXPERIMENTAL_STORE_PAGE_KIND),
            unsupported.to_string(),
        )
        .tags(valid_shape.tags)
        .custom_created_at(Timestamp::from(20))
        .sign_with_keys(&keys)
        .expect("signed invalid page");
        let relays = FakeRelays::new(vec![query(vec![invalid]), query(Vec::new())]);
        let result = StorePageDiscoveryService::new(&relays, &repository)
            .discover(&listing)
            .await
            .expect("discovery");
        assert!(matches!(
            result,
            StorePageLookupResult::InvalidStorePage { .. }
        ));
    }

    #[tokio::test]
    async fn valid_newer_candidate_wins_when_invalid_older_is_also_returned() {
        let (_directory, repository) = repository().await;
        let keys = Keys::generate();
        let listing = signed_listing(&keys, "game", Some("page"));
        let valid = page(
            &keys,
            "page",
            vec![listing_coordinate(&listing).expect("coordinate")],
            "Valid",
            20,
        );
        let mut unsupported: serde_json::Value =
            serde_json::from_str(&valid.content).expect("content JSON");
        unsupported["version"] = serde_json::json!(999);
        let invalid_older = EventBuilder::new(
            Kind::Custom(EXPERIMENTAL_STORE_PAGE_KIND),
            unsupported.to_string(),
        )
        .tags(valid.tags.clone())
        .custom_created_at(Timestamp::from(10))
        .sign_with_keys(&keys)
        .expect("signed invalid page");
        let relays = FakeRelays::new(vec![query(vec![invalid_older, valid.clone()])]);
        let result = StorePageDiscoveryService::new(&relays, &repository)
            .discover(&listing)
            .await
            .expect("discovery");
        assert!(matches!(
            result,
            StorePageLookupResult::Associated(DiscoveredStorePage {
                association: StorePageAssociation { store_page, .. },
                ..
            }) if store_page.event.id == valid.id
        ));
    }

    #[tokio::test]
    async fn valid_nonreciprocal_replacement_detaches_cached_page() {
        let (_directory, repository) = repository().await;
        let keys = Keys::generate();
        let listing = signed_listing(&keys, "game", Some("page"));
        let cached = page(
            &keys,
            "page",
            vec![listing_coordinate(&listing).expect("coordinate")],
            "Cached",
            10,
        );
        repository
            .upsert_valid(&parse_store_page_event(&cached).expect("cached page"))
            .await
            .expect("cache insert");
        let other_listing = signed_listing(&keys, "other", None);
        let detached = page(
            &keys,
            "page",
            vec![listing_coordinate(&other_listing).expect("coordinate")],
            "Detached",
            20,
        );
        let relays = FakeRelays::new(vec![query(vec![detached]), query(Vec::new())]);
        let result = StorePageDiscoveryService::new(&relays, &repository)
            .discover(&listing)
            .await
            .expect("discovery");
        assert!(!matches!(result, StorePageLookupResult::Associated(_)));
    }

    #[tokio::test]
    async fn older_nonreciprocal_candidate_cannot_detach_newer_cached_page() {
        let (_directory, repository) = repository().await;
        let keys = Keys::generate();
        let listing = signed_listing(&keys, "game", Some("page"));
        let cached = page(
            &keys,
            "page",
            vec![listing_coordinate(&listing).expect("coordinate")],
            "Cached",
            20,
        );
        repository
            .upsert_valid(&parse_store_page_event(&cached).expect("cached page"))
            .await
            .expect("cache insert");
        let other_listing = signed_listing(&keys, "other", None);
        let older = page(
            &keys,
            "page",
            vec![listing_coordinate(&other_listing).expect("coordinate")],
            "Older",
            10,
        );
        let relays = FakeRelays::new(vec![query(vec![older])]);
        let result = StorePageDiscoveryService::new(&relays, &repository)
            .discover(&listing)
            .await
            .expect("discovery");
        assert!(matches!(
            result,
            StorePageLookupResult::Associated(DiscoveredStorePage {
                source: StorePageDiscoverySource::Cache,
                ..
            })
        ));
    }

    #[tokio::test]
    async fn valid_result_from_incomplete_coverage_is_marked_unavailable() {
        let (_directory, repository) = repository().await;
        let keys = Keys::generate();
        let listing = signed_listing(&keys, "game", Some("page"));
        let page = page(
            &keys,
            "page",
            vec![listing_coordinate(&listing).expect("coordinate")],
            "Title",
            10,
        );
        let relays = FakeRelays::new(vec![Ok(StorePageRelayQuery {
            events: vec![page],
            coverage_complete: false,
        })]);
        let result = StorePageDiscoveryService::new(&relays, &repository)
            .discover(&listing)
            .await
            .expect("discovery");
        assert!(matches!(
            result,
            StorePageLookupResult::Associated(DiscoveredStorePage {
                relay_refresh_unavailable: true,
                ..
            })
        ));
    }

    #[tokio::test]
    async fn changed_listing_pointer_does_not_attach_old_cached_page() {
        let (_directory, repository) = repository().await;
        let keys = Keys::generate();
        let old_listing = signed_listing(&keys, "game", Some("old-page"));
        let old_page = page(
            &keys,
            "old-page",
            vec![listing_coordinate(&old_listing).expect("coordinate")],
            "Old",
            10,
        );
        repository
            .upsert_valid(&parse_store_page_event(&old_page).expect("old page"))
            .await
            .expect("cache insert");
        let changed_listing = signed_listing(&keys, "game", Some("new-page"));
        let relays = FakeRelays::new(vec![query(Vec::new()), query(Vec::new())]);
        let result = StorePageDiscoveryService::new(&relays, &repository)
            .discover(&changed_listing)
            .await
            .expect("discovery");
        assert!(!matches!(result, StorePageLookupResult::Associated(_)));
    }

    #[tokio::test]
    async fn incomplete_relay_coverage_is_unavailable_not_absent() {
        let (_directory, repository) = repository().await;
        let keys = Keys::generate();
        let listing = signed_listing(&keys, "game", None);
        let relays = FakeRelays::new(vec![Ok(StorePageRelayQuery {
            events: Vec::new(),
            coverage_complete: false,
        })]);
        let result = StorePageDiscoveryService::new(&relays, &repository)
            .discover(&listing)
            .await
            .expect("discovery");
        assert!(matches!(
            result,
            StorePageLookupResult::RelayUnavailable { .. }
        ));
    }

    #[tokio::test]
    async fn relay_absence_does_not_delete_valid_cache() {
        let (_directory, repository) = repository().await;
        let keys = Keys::generate();
        let listing = signed_listing(&keys, "game", Some("page"));
        let cached = page(
            &keys,
            "page",
            vec![listing_coordinate(&listing).expect("coordinate")],
            "Cached",
            10,
        );
        repository
            .upsert_valid(&parse_store_page_event(&cached).expect("cached page"))
            .await
            .expect("cache insert");
        let relays = FakeRelays::new(vec![query(Vec::new()), query(Vec::new())]);
        let result = StorePageDiscoveryService::new(&relays, &repository)
            .discover(&listing)
            .await
            .expect("discovery");
        assert!(matches!(
            result,
            StorePageLookupResult::Associated(DiscoveredStorePage {
                source: StorePageDiscoverySource::Cache,
                ..
            })
        ));
    }

    #[tokio::test]
    async fn batch_enrichment_deduplicates_listings_and_groups_shared_page_work() {
        let (_directory, repository) = repository().await;
        let keys = Keys::generate();
        let linux = signed_listing(&keys, "linux", Some("page"));
        let windows = signed_listing(&keys, "windows", Some("page"));
        let page = page(
            &keys,
            "page",
            vec![
                listing_coordinate(&linux).expect("linux coordinate"),
                listing_coordinate(&windows).expect("windows coordinate"),
            ],
            "Shared",
            10,
        );
        let linux_coordinate = listing_coordinate(&linux).expect("linux coordinate");
        let windows_coordinate = listing_coordinate(&windows).expect("windows coordinate");
        let linux_event_id = linux.id.to_hex();
        let windows_event_id = windows.id.to_hex();
        let relays = FakeRelays::new(vec![
            query(vec![linux, windows]),
            query(vec![page.clone()]),
            query(vec![page]),
        ]);
        let response = StorePageBatchService::new(&relays, &repository)
            .enrich(
                7,
                &[
                    StorePageListingRef {
                        listing_coordinate: linux_coordinate.clone(),
                        listing_event_id: linux_event_id.clone(),
                    },
                    StorePageListingRef {
                        listing_coordinate: linux_coordinate,
                        listing_event_id: linux_event_id,
                    },
                    StorePageListingRef {
                        listing_coordinate: windows_coordinate,
                        listing_event_id: windows_event_id,
                    },
                ],
            )
            .await
            .expect("batch enrichment");
        assert_eq!(response.generation, 7);
        assert_eq!(response.refreshed.len(), 2);
        assert!(response
            .refreshed
            .iter()
            .all(|result| matches!(result.state, StorePageEnrichmentState::Enriched(_))));
        assert_eq!(*relays.query_count.lock().expect("query count lock"), 3);
    }

    #[test]
    fn batch_retains_exact_newest_signed_listing_event() {
        let keys = Keys::generate();
        let older = EventBuilder::new(Kind::Custom(NIP99_LISTING_KIND), "older")
            .tags([Tag::parse(["d", "game"]).expect("d tag")])
            .custom_created_at(Timestamp::from(10))
            .sign_with_keys(&keys)
            .expect("older listing");
        let newer = EventBuilder::new(Kind::Custom(NIP99_LISTING_KIND), "newer")
            .tags([Tag::parse(["d", "game"]).expect("d tag")])
            .custom_created_at(Timestamp::from(20))
            .sign_with_keys(&keys)
            .expect("newer listing");
        let coordinate = listing_coordinate(&newer).expect("coordinate");
        let requested = BTreeMap::from([(
            coordinate.clone(),
            (keys.public_key(), "game".to_string(), newer.id.to_hex()),
        )]);
        let selected = select_current_listings(&requested, &[older, newer.clone()]);
        assert_eq!(selected.get(&coordinate), Some(&newer));
        assert!(selected[&coordinate].verify().is_ok());
    }

    #[tokio::test]
    async fn detail_rejects_requested_listing_replaced_by_newer_event() {
        let (_directory, repository) = repository().await;
        let keys = Keys::generate();
        let tags = [
            Tag::parse(["d", "game"]).expect("d tag"),
            Tag::parse([
                "store_page",
                store_page_coordinate(keys.public_key(), "page").as_str(),
            ])
            .expect("pointer tag"),
        ];
        let older = EventBuilder::new(Kind::Custom(NIP99_LISTING_KIND), "older")
            .tags(tags.clone())
            .custom_created_at(Timestamp::from(10))
            .sign_with_keys(&keys)
            .expect("older listing");
        let newer = EventBuilder::new(Kind::Custom(NIP99_LISTING_KIND), "newer")
            .tags(tags)
            .custom_created_at(Timestamp::from(20))
            .sign_with_keys(&keys)
            .expect("newer listing");
        let coordinate = listing_coordinate(&older).expect("coordinate");
        let relays = FakeRelays::new(vec![query(vec![older.clone(), newer])]);

        let result = StorePageBatchService::new(&relays, &repository)
            .enrich_detail(
                7,
                &StorePageListingRef {
                    listing_coordinate: coordinate,
                    listing_event_id: older.id.to_hex(),
                },
            )
            .await
            .expect("detail enrichment");

        assert!(!result.listing_event_current);
        assert!(result.cached.is_none());
        assert_eq!(result.refreshed, StorePageDetailState::Unavailable);
        assert_eq!(*relays.query_count.lock().expect("query count lock"), 1);
    }

    #[test]
    fn game_detail_dto_contains_only_sanitized_content_and_listing_platform_requirements() {
        use crate::store_page::{PlatformRequirement, RequirementTier, StorePageMediaItem};

        let keys = Keys::generate();
        let pointer = store_page_coordinate(keys.public_key(), "page");
        let listing = EventBuilder::new(Kind::Custom(NIP99_LISTING_KIND), "listing")
            .tags([
                Tag::parse(["d", "game"]).expect("d tag"),
                Tag::parse(["store_page", pointer.as_str()]).expect("pointer tag"),
                Tag::parse(["platform", "linux-x86_64"]).expect("platform tag"),
            ])
            .sign_with_keys(&keys)
            .expect("listing");
        let mut content = StorePageContentV1 {
            description_markdown: "Safe **text** <script>bad()</script>".into(),
            ..StorePageContentV1::default()
        };
        let requirement = PlatformRequirement {
            minimum: Some(RequirementTier {
                os: Some("Linux".into()),
                ..RequirementTier::default()
            }),
            recommended: None,
        };
        content
            .requirements
            .insert("linux-x86_64".into(), requirement.clone());
        content
            .requirements
            .insert("windows-x86_64".into(), requirement);
        content.media.push(StorePageMediaItem {
            id: "trailer".into(),
            media_type: "video".into(),
            role: "trailer".into(),
            url: "https://cdn.example.org/trailer.mp4".into(),
            sha256: None,
            mime_type: None,
            size: None,
            thumbnail_url: Some("https://cdn.example.org/poster.webp".into()),
            alt: Some("Trailer".into()),
            caption: None,
            width: None,
            height: None,
        });
        let event = build_store_page_event_builder(&StorePageBuildParams {
            publisher: keys.public_key(),
            presentation_id: "page".into(),
            listing_coordinates: vec![listing_coordinate(&listing).expect("coordinate")],
            content,
            compact_tags: StorePageCompactTags::default(),
        })
        .expect("page builder")
        .sign_with_keys(&keys)
        .expect("page");
        let parsed = parse_store_page_event(&event).expect("parsed page");
        let association = validate_store_page_association(&listing, &parsed).expect("association");
        let detail = detail_presentation(&association, &listing);
        let html = detail.description_html.expect("sanitized description");
        assert!(html.contains("<strong>text</strong>"));
        assert!(
            !html.to_ascii_lowercase().contains("<script"),
            "unsafe preview HTML: {html}"
        );
        assert_eq!(detail.requirements.len(), 1);
        assert_eq!(detail.requirements[0].platform, "linux-x86_64");
        assert_eq!(detail.media[0].url, "https://cdn.example.org/trailer.mp4");
    }

    #[tokio::test]
    async fn game_detail_unavailable_refresh_preserves_cached_presentation() {
        let (_directory, repository) = repository().await;
        let keys = Keys::generate();
        let listing = signed_listing(&keys, "game", Some("page"));
        let coordinate = listing_coordinate(&listing).expect("coordinate");
        let page = page(&keys, "page", vec![coordinate.clone()], "Cached", 10);
        repository
            .upsert_valid(&parse_store_page_event(&page).expect("page"))
            .await
            .expect("cache");
        repository
            .upsert_listing_event(&listing)
            .await
            .expect("listing cache");
        let relays = FakeRelays::new(vec![
            Err("listing unavailable".into()),
            Err("preferred unavailable".into()),
            Err("recovery unavailable".into()),
        ]);
        let response = StorePageBatchService::new(&relays, &repository)
            .enrich_detail(
                9,
                &StorePageListingRef {
                    listing_coordinate: coordinate,
                    listing_event_id: listing.id.to_hex(),
                },
            )
            .await
            .expect("detail enrichment");
        assert_eq!(response.generation, 9);
        assert_eq!(
            response.cached.and_then(|page| page.title),
            Some("Cached".into())
        );
        assert_eq!(response.refreshed, StorePageDetailState::Unavailable);
    }

    #[test]
    fn publisher_preview_uses_canonical_sanitization_and_listing_identity() {
        let keys = Keys::generate();
        let listing = signed_listing(&keys, "game", Some("page"));
        let coordinate = listing_coordinate(&listing).expect("listing coordinate");
        let mut content = StorePageContentV1::default();
        content.basic.title = Some("Preview title".to_string());
        content.description_markdown =
            "<script>alert('unsafe')</script>\n\n**Safe description**".to_string();
        let preview = preview_store_page_detail(
            &StorePageBuildParams {
                publisher: keys.public_key(),
                presentation_id: "page".to_string(),
                listing_coordinates: vec![coordinate.clone()],
                content,
                compact_tags: StorePageCompactTags::default(),
            },
            &listing,
        )
        .expect("preview");

        assert_eq!(preview.listing_coordinate, coordinate);
        assert_eq!(preview.listing_event_id, listing.id.to_hex());
        assert_eq!(preview.title.as_deref(), Some("Preview title"));
        let html = preview.description_html.expect("sanitized description");
        assert!(!html.to_ascii_lowercase().contains("<script"));
        assert!(html.contains("<strong>Safe description</strong>"));
    }
}
