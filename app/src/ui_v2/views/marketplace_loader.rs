//! Shared marketplace listing loader and presentation helpers for UI v2 views.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use arcadestr_core::is_replaceable_event_newer;
use gloo_timers::future::TimeoutFuture;
use leptos::prelude::*;
use nostr::nips::nip19::FromBech32;
use wasm_bindgen_futures::spawn_local;

use crate::models::{
    GameListing, ListingSource, StorePageCardPresentation, StorePageEnrichmentRequest,
    StorePageEnrichmentState, StorePageListingRef,
};
use crate::store::{try_use_marketplace_store, DEFAULT_LISTING_TTL_SECS};
use crate::tauri_bridge::{
    invoke_discover_campaign_summaries, invoke_discover_campaigns, invoke_enrich_store_pages,
    CampaignPointerInput, CampaignSummaryListingInput, DiscoverCampaignSummariesRequest,
    DiscoverCampaignsRequest,
};
use crate::{invoke_fetch_marketplace_stream, AuthContext};

// ── Batch flusher for progressive listing updates ────────────────────────────
//
// Instead of updating the `listings` RwSignal on every individual Tauri event
// (which can cause 200+ grid re-renders during streaming), we accumulate
// incoming products into a buffer and flush them after a short debounce.
// This batches rapid arrivals into fewer signal updates, drastically
// reducing re-renders while keeping progressive loading responsive.
//

const DEFAULT_MARKETPLACE_LISTING_LIMIT: usize = 50;

#[derive(Clone, Copy)]
pub struct MarketplaceListingsState {
    pub listings: RwSignal<Vec<GameListing>>,
    pub loading: RwSignal<bool>,
    pub loading_more: RwSignal<bool>,
    pub error: RwSignal<Option<String>>,
    pub received_count: RwSignal<usize>,
    pub requested_limit: RwSignal<usize>,
    pub has_more: RwSignal<bool>,
    pub refreshing: RwSignal<bool>,
    pub using_cached_data: RwSignal<bool>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CampaignAvailability {
    Active,
    Upcoming,
    Ended,
}

#[derive(Clone, Copy)]
pub struct ListingCampaignStates {
    pub states: RwSignal<HashMap<String, CampaignAvailability>>,
    pub loading: RwSignal<bool>,
    pub error: RwSignal<Option<String>>,
}

#[derive(Clone, Copy)]
pub struct ListingStorePagePresentations {
    pub presentations: RwSignal<HashMap<String, StorePageCardPresentation>>,
    pub loading: RwSignal<bool>,
    pub unavailable: RwSignal<bool>,
}

pub fn canonical_listing_coordinate(listing: &GameListing) -> Option<String> {
    let publisher = nostr::PublicKey::from_bech32(&listing.publisher_npub).ok()?;
    Some(format!("30402:{}:{}", publisher.to_hex(), listing.id))
}

pub fn use_listing_store_page_presentations(
    listings: RwSignal<Vec<GameListing>>,
) -> ListingStorePagePresentations {
    let presentations = RwSignal::new(HashMap::<String, StorePageCardPresentation>::new());
    let loading = RwSignal::new(false);
    let unavailable = RwSignal::new(false);
    let generation = RwSignal::new(0_u64);
    let known_events = RwSignal::new(HashMap::<String, Option<String>>::new());

    Effect::new(move |_| {
        let ordered = listings
            .get()
            .into_iter()
            .filter_map(|listing| {
                canonical_listing_coordinate(&listing)
                    .map(|coordinate| (coordinate, listing.event_id))
            })
            .collect::<Vec<_>>();
        let current = ordered.iter().cloned().collect::<HashMap<_, _>>();
        let previous = known_events.get_untracked();
        if current == previous {
            return;
        }
        generation.update(|value| *value = value.wrapping_add(1));
        let request_generation = generation.get_untracked();
        presentations.update(|values| {
            values.retain(|coordinate, _| current.get(coordinate) == previous.get(coordinate))
        });
        known_events.set(current.clone());
        if current.is_empty() {
            loading.set(false);
            return;
        }
        loading.set(true);
        unavailable.set(false);
        let request = StorePageEnrichmentRequest {
            generation: request_generation,
            listings: ordered
                .iter()
                .filter_map(|(coordinate, event_id)| {
                    event_id
                        .clone()
                        .map(|listing_event_id| StorePageListingRef {
                            listing_coordinate: coordinate.clone(),
                            listing_event_id,
                        })
                })
                .take(64)
                .collect(),
        };
        spawn_local(async move {
            let response = match invoke_enrich_store_pages(request).await {
                Ok(response) => response,
                Err(_) => {
                    if generation.get_untracked() == request_generation {
                        unavailable.set(true);
                        loading.set(false);
                    }
                    return;
                }
            };
            if !store_page_generation_matches(
                generation.get_untracked(),
                request_generation,
                response.generation,
            ) {
                return;
            }
            let expected_events = known_events.get_untracked();
            apply_store_page_updates(&presentations, &expected_events, response.cached);
            apply_store_page_updates(&presentations, &expected_events, response.refreshed);
            loading.set(false);
        });
    });

    ListingStorePagePresentations {
        presentations,
        loading,
        unavailable,
    }
}

fn store_page_generation_matches(current: u64, requested: u64, response: u64) -> bool {
    current == requested && response == requested
}

fn apply_store_page_updates(
    presentations: &RwSignal<HashMap<String, StorePageCardPresentation>>,
    expected_events: &HashMap<String, Option<String>>,
    updates: Vec<crate::models::StorePageEnrichmentResult>,
) {
    presentations.update(|current| {
        for update in updates {
            if expected_events
                .get(&update.listing_coordinate)
                .and_then(Clone::clone)
                .as_deref()
                != Some(update.listing_event_id.as_str())
            {
                continue;
            }
            match update.state {
                StorePageEnrichmentState::Enriched(presentation) => {
                    current.insert(update.listing_coordinate, presentation);
                }
                StorePageEnrichmentState::Unavailable => {}
                StorePageEnrichmentState::NotAssociated
                | StorePageEnrichmentState::NotFound
                | StorePageEnrichmentState::Invalid => {
                    current.remove(&update.listing_coordinate);
                }
            }
        }
    });
}

pub fn listing_state_key(listing: &GameListing) -> String {
    format!("{}:{}", listing.publisher_npub, listing.id)
}

pub fn use_listing_campaign_states(listings: RwSignal<Vec<GameListing>>) -> ListingCampaignStates {
    let states = RwSignal::new(HashMap::<String, CampaignAvailability>::new());
    let loading = RwSignal::new(false);
    let error = RwSignal::new(None::<String>);
    let last_fingerprint = RwSignal::new(String::new());
    let generation = RwSignal::new(0_u64);

    Effect::new(move |_| {
        let candidates = listings.get();
        let fingerprint = candidates
            .iter()
            .map(|listing| {
                let pointers = listing
                    .campaigns
                    .iter()
                    .map(|pointer| pointer.root_event_id.as_str())
                    .collect::<Vec<_>>()
                    .join(",");
                format!("{}={pointers}", listing_state_key(listing))
            })
            .collect::<Vec<_>>()
            .join("|");

        if fingerprint == last_fingerprint.get_untracked() {
            return;
        }
        last_fingerprint.set(fingerprint);
        error.set(None);
        generation.update(|value| *value = value.wrapping_add(1));
        let request_generation = generation.get_untracked();
        let candidate_keys = candidates
            .iter()
            .map(listing_state_key)
            .collect::<HashSet<_>>();
        states.update(|current| current.retain(|key, _| candidate_keys.contains(key)));

        if candidates.is_empty() {
            loading.set(false);
            return;
        }

        loading.set(true);
        spawn_local(async move {
            let mut by_publisher = HashMap::<String, Vec<GameListing>>::new();
            for listing in candidates {
                by_publisher
                    .entry(listing.publisher_npub.clone())
                    .or_default()
                    .push(listing);
            }

            for (publisher_npub, publisher_listings) in by_publisher {
                let summary_request = DiscoverCampaignSummariesRequest {
                    publisher_npub: publisher_npub.clone(),
                    listings: publisher_listings
                        .iter()
                        .map(|listing| CampaignSummaryListingInput {
                            listing_id: listing.id.clone(),
                        })
                        .collect(),
                };
                let summaries = match invoke_discover_campaign_summaries(summary_request).await {
                    Ok(summaries) => summaries,
                    Err(message) => {
                        if generation.get_untracked() != request_generation {
                            return;
                        }
                        error.set(Some(message));
                        Vec::new()
                    }
                };
                if generation.get_untracked() != request_generation {
                    return;
                }
                let summaries = summaries
                    .into_iter()
                    .map(|summary| {
                        let availability = if summary.active > 0 {
                            Some(CampaignAvailability::Active)
                        } else if summary.upcoming > 0 {
                            Some(CampaignAvailability::Upcoming)
                        } else {
                            None
                        };
                        (summary.listing_id, availability)
                    })
                    .collect::<HashMap<_, _>>();

                for listing in publisher_listings {
                    let key = listing_state_key(&listing);
                    let mut availability = summaries.get(&listing.id).copied().flatten();
                    if availability.is_none() && !listing.campaigns.is_empty() {
                        let request = DiscoverCampaignsRequest {
                            publisher_npub: publisher_npub.clone(),
                            listing_id: listing.id.clone(),
                            pointers: listing
                                .campaigns
                                .iter()
                                .map(|pointer| CampaignPointerInput {
                                    root_event_id: pointer.root_event_id.clone(),
                                    relay_hint: pointer.relay_hint.clone(),
                                })
                                .collect(),
                        };
                        match invoke_discover_campaigns(request).await {
                            Ok(discovered) => {
                                availability = if discovered
                                    .iter()
                                    .any(|campaign| campaign.classification == "active")
                                {
                                    Some(CampaignAvailability::Active)
                                } else if discovered
                                    .iter()
                                    .any(|campaign| campaign.classification == "upcoming")
                                {
                                    Some(CampaignAvailability::Upcoming)
                                } else if discovered.iter().any(|campaign| {
                                    matches!(
                                        campaign.classification.as_str(),
                                        "ended" | "cancelled"
                                    )
                                }) {
                                    Some(CampaignAvailability::Ended)
                                } else {
                                    None
                                };
                            }
                            Err(message) => error.set(Some(message)),
                        }
                        if generation.get_untracked() != request_generation {
                            return;
                        }
                    }
                    states.update(|current| match availability {
                        Some(availability) => {
                            current.insert(key, availability);
                        }
                        None => {
                            current.remove(&key);
                        }
                    });
                }
            }
            if generation.get_untracked() == request_generation {
                loading.set(false);
            }
        });
    });

    ListingCampaignStates {
        states,
        loading,
        error,
    }
}

pub fn use_marketplace_listings() -> MarketplaceListingsState {
    use_marketplace_listings_with_limit(DEFAULT_MARKETPLACE_LISTING_LIMIT)
}

pub fn use_marketplace_listings_with_limit(limit: usize) -> MarketplaceListingsState {
    let auth = use_context::<AuthContext>().expect("AuthContext not provided");
    let marketplace_store = try_use_marketplace_store();
    let listings = RwSignal::new(Vec::<GameListing>::new());
    let loading = RwSignal::new(true);
    let loading_more = RwSignal::new(false);
    let error = RwSignal::new(None::<String>);
    let received_count = RwSignal::new(0);
    let requested_limit = RwSignal::new(limit);
    let has_more = RwSignal::new(true);
    let refreshing = RwSignal::new(false);
    let using_cached_data = RwSignal::new(false);
    let last_requested_limit = RwSignal::new(0usize);
    let loaded_account = RwSignal::new(Some(auth.npub.get_untracked()));
    let request_generation = RwSignal::new(0u64);

    Effect::new(move |_| {
        let active_account = auth.npub.get();
        if loaded_account.get_untracked().as_ref() != Some(&active_account) {
            loaded_account.set(Some(active_account));
            request_generation.update(|generation| *generation += 1);
            last_requested_limit.set(0);
            listings.set(Vec::new());
            received_count.set(0);
            has_more.set(true);
            refreshing.set(false);
            using_cached_data.set(false);
            if let Some(store) = &marketplace_store {
                store.clear();
            }
        }
        let generation = request_generation.get_untracked();
        let target_limit = requested_limit.get();
        if target_limit <= last_requested_limit.get() {
            return;
        }
        last_requested_limit.set(target_limit);

        let store = marketplace_store.clone();
        spawn_local(async move {
            let loaded_count = listings.get_untracked().len();
            let page_limit = next_fetch_limit(loaded_count, target_limit).unwrap_or(target_limit);
            let until_secs = if loaded_count > 0 {
                older_page_until_secs(&listings.get_untracked())
            } else {
                None
            };

            let is_initial_load = loaded_count == 0;
            loading.set(is_initial_load);
            loading_more.set(!is_initial_load);
            refreshing.set(false);
            error.set(None);
            if loaded_count == 0 {
                received_count.set(0);
            }

            if crate::debug_storefront_bypass_enabled() {
                let mocked = recent_listings(debug_mock_listings(), target_limit);
                if let Some(s) = &store {
                    s.clear();
                    s.put_many(mocked.clone());
                }
                received_count.set(mocked.len());
                listings.set(mocked);
                loading.set(false);
                loading_more.set(false);
                using_cached_data.set(false);
                return;
            }

            let should_fetch = match &store {
                Some(s) => {
                    let cached = s.get_all();
                    let needs_refresh = s.needs_refresh(DEFAULT_LISTING_TTL_SECS);
                    if !cached.is_empty() && !needs_refresh && cached.len() >= target_limit {
                        listings.set(recent_listings(cached, target_limit));
                        loading.set(false);
                        loading_more.set(false);
                        refreshing.set(false);
                        using_cached_data.set(true);
                        false
                    } else {
                        if !cached.is_empty() {
                            listings.set(recent_listings(cached, target_limit));
                            loading.set(false);
                            loading_more.set(false);
                            refreshing.set(true);
                            using_cached_data.set(true);
                        }
                        true
                    }
                }
                None => true,
            };

            if should_fetch {
                let store_for_listing = store.clone();

                // ── Batch-flusher for progressive streaming ─────────────────
                // Accumulates products and flushes to the listings signal
                // after a short debounce. Each flush sorts and truncates once,
                // reducing per-event re-renders while keeping loading progressive.
                let buffer: Rc<RefCell<Vec<GameListing>>> = Rc::new(RefCell::new(Vec::new()));
                let flush_queued: Rc<Cell<bool>> = Rc::new(Cell::new(false));
                let received_coordinates: Rc<RefCell<HashSet<(String, String)>>> =
                    Rc::new(RefCell::new(HashSet::new()));

                let on_listing = {
                    let buffer = Rc::clone(&buffer);
                    let flush_queued = Rc::clone(&flush_queued);
                    let received_coordinates = Rc::clone(&received_coordinates);
                    let listings = listings.clone();
                    let loading = loading.clone();
                    let store_for_listing = store_for_listing.clone();

                    move |listing: GameListing| {
                        if request_generation.get_untracked() != generation {
                            return;
                        }
                        let is_first_coordinate = mark_received_coordinate(
                            &mut *received_coordinates.borrow_mut(),
                            &listing,
                        );

                        // Immediately persist to the global store (deduplicating)
                        let store_ref = store_for_listing.clone();
                        let listing_for_store = listing.clone();
                        batch(move || {
                            if is_first_coordinate {
                                received_count.update(|count| *count += 1);
                            }
                            if let Some(s) = &store_ref {
                                s.put_streaming(listing_for_store);
                            }
                        });

                        // Buffer for batched signal update
                        buffer.borrow_mut().push(listing);

                        // Schedule a debounced flush if not already queued
                        if !flush_queued.replace(true) {
                            let buffer = Rc::clone(&buffer);
                            let flush_queued = Rc::clone(&flush_queued);
                            let listings = listings.clone();
                            let loading = loading.clone();

                            spawn_local(async move {
                                // Wait briefly so bursts of Tauri events coalesce
                                // into one signal update while the UI remains responsive.
                                TimeoutFuture::new(50).await;

                                if request_generation.get_untracked() != generation {
                                    buffer.borrow_mut().clear();
                                    flush_queued.set(false);
                                    return;
                                }

                                let queued: Vec<GameListing> =
                                    std::mem::take(&mut *buffer.borrow_mut());
                                flush_queued.set(false);

                                if !queued.is_empty() {
                                    batch(move || {
                                        listings.update(|items| {
                                            for item in queued {
                                                if upsert_latest_listing(items, item) {
                                                    items.truncate(target_limit);
                                                }
                                            }
                                            items.sort_unstable_by(|a, b| {
                                                b.created_at.cmp(&a.created_at)
                                            });

                                            if has_reached_listing_limit(items.len(), target_limit)
                                            {
                                                loading.set(false);
                                            }
                                        });
                                    });
                                }
                            });
                        }
                    }
                };

                let on_complete = {
                    let buffer = Rc::clone(&buffer);
                    let flush_queued = Rc::clone(&flush_queued);
                    let listings = listings.clone();
                    let loading = loading.clone();
                    let loading_more = loading_more.clone();
                    let store_for_complete = store.clone();

                    move || {
                        if request_generation.get_untracked() != generation {
                            return;
                        }
                        // Flush any remaining buffered items before finalising
                        let remaining: Vec<GameListing> = std::mem::take(&mut *buffer.borrow_mut());
                        flush_queued.set(false);

                        batch(move || {
                            if !remaining.is_empty() {
                                listings.update(|items| {
                                    for item in remaining {
                                        let _ = upsert_latest_listing(items, item);
                                    }
                                    items.sort_unstable_by(|a, b| b.created_at.cmp(&a.created_at));
                                    items.truncate(target_limit);
                                });
                            }

                            if let Some(s) = &store_for_complete {
                                s.mark_fresh();
                            }
                            has_more.set(has_more_after_page(
                                loaded_count,
                                listings.get_untracked().len(),
                            ));
                            loading.set(false);
                            loading_more.set(false);
                            refreshing.set(false);
                            using_cached_data.set(false);
                        });
                    }
                };

                let on_complete = Some(on_complete);

                match invoke_fetch_marketplace_stream(
                    page_limit,
                    Some(30),
                    until_secs,
                    on_listing,
                    on_complete,
                )
                .await
                {
                    Ok((product_cleanup, completion_cleanup)) => {
                        // Stream command has returned; unregister listeners.
                        product_cleanup();
                        completion_cleanup();

                        if request_generation.get_untracked() != generation {
                            return;
                        }

                        // Fallback in case completion event was missed.
                        if loading.get_untracked()
                            || loading_more.get_untracked()
                            || refreshing.get_untracked()
                        {
                            listings.update(|items| {
                                items.sort_unstable_by(|a, b| b.created_at.cmp(&a.created_at));
                            });
                            has_more.set(has_more_after_page(
                                loaded_count,
                                listings.get_untracked().len(),
                            ));
                            loading.set(false);
                            loading_more.set(false);
                            refreshing.set(false);
                            using_cached_data.set(false);
                        }
                    }
                    Err(e) => {
                        if request_generation.get_untracked() != generation {
                            return;
                        }
                        batch(move || {
                            if let Some(s) = &store {
                                let cached = s.get_all();
                                if !cached.is_empty() {
                                    listings.set(recent_listings(cached, target_limit));
                                    error.set(Some(e));
                                    loading.set(false);
                                    loading_more.set(false);
                                    refreshing.set(false);
                                    using_cached_data.set(true);
                                } else {
                                    error.set(Some(e));
                                    loading.set(false);
                                    loading_more.set(false);
                                    refreshing.set(false);
                                }
                            } else {
                                error.set(Some(e));
                                loading.set(false);
                                loading_more.set(false);
                                refreshing.set(false);
                            }
                        });
                    }
                }
            }
        });
    });

    MarketplaceListingsState {
        listings,
        loading,
        loading_more,
        error,
        received_count,
        requested_limit,
        has_more,
        refreshing,
        using_cached_data,
    }
}

fn recent_listings(mut listings: Vec<GameListing>, limit: usize) -> Vec<GameListing> {
    listings.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    listings.truncate(limit);
    listings
}

fn mark_received_coordinate(seen: &mut HashSet<(String, String)>, listing: &GameListing) -> bool {
    seen.insert((listing.publisher_npub.clone(), listing.id.clone()))
}

fn upsert_latest_listing(listings: &mut Vec<GameListing>, listing: GameListing) -> bool {
    match listings.iter_mut().find(|current| {
        current.publisher_npub == listing.publisher_npub && current.id == listing.id
    }) {
        Some(current)
            if is_replaceable_event_newer(
                listing.created_at,
                listing.event_id.as_deref(),
                current.created_at,
                current.event_id.as_deref(),
            ) =>
        {
            *current = listing;
            false
        }
        Some(_) => false,
        None => {
            listings.push(listing);
            true
        }
    }
}

fn has_reached_listing_limit(listing_count: usize, limit: usize) -> bool {
    limit > 0 && listing_count >= limit
}

fn next_fetch_limit(loaded_count: usize, requested_limit: usize) -> Option<usize> {
    requested_limit
        .checked_sub(loaded_count)
        .filter(|limit| *limit > 0)
}

fn older_page_until_secs(listings: &[GameListing]) -> Option<u64> {
    listings
        .iter()
        .map(|listing| listing.created_at)
        .min()
        .map(|oldest| oldest.saturating_sub(1))
}

fn has_more_after_page(before_count: usize, after_count: usize) -> bool {
    after_count > before_count
}

fn debug_mock_listings() -> Vec<GameListing> {
    vec![
        GameListing {
            id: "debug-neon-velocity".to_string(),
            source: ListingSource::Nip15Product,
            title: "Neon Velocity".to_string(),
            description: "Drift through a neon skyline in a high-speed action experience built for multiplayer tournaments.".to_string(),
            images: vec!["https://lh3.googleusercontent.com/aida-public/AB6AXuAkSqV1ZOY7qDKBQQ-nU-WKmOwR16envOE_TMPHQep0afObsmDW51MoGnuCDehLWvRSiX2M-G1ipCeBVnLuSnk_GtSaKNiiKAL3NGBqfTVvkZErj92gogHgjo8Dm9s9qZAoKzMpmCEwTLAaasaklmpG0EvebxYhk_pgx9zFciCa6eEvQAequV2_VwSfxkp8qFHQKZDgSfcZX7ItUvMkkVo9gJMVU1kvmoEqtnEdxEgw_XCFEvda_kG_L7oqZh2ranJukSzpwKvU8ow".to_string()],
            download_url: "https://example.com/neon-velocity".to_string(),
            price: 21000.0,
            currency: "SATS".to_string(),
            price_sats: 21000,
            quantity: None,
            tags: vec!["action".to_string(), "multiplayer".to_string()],
            specs: vec![("genre".to_string(), "Action".to_string())],
            publisher_npub: "npub1debugstorefront000000000000000000000000000000000000000000".to_string(),
            stall_id: "debug-stall-1".to_string(),
            stall_name: Some("Neon Arcade".to_string()),
            lud16: "debug@arcadestr.dev".to_string(),
            event_id: None,
            created_at: 1_710_000_001,
            platforms: Vec::new(),
            nip94_event_id: None,
            acquisition: crate::models::AcquisitionPolicy::Gated,
            campaigns: Vec::new(),
            is_owned: false,
            #[cfg(debug_assertions)]
            nip99_raw_event_json: None,
        },
        GameListing {
            id: "debug-bit-runners".to_string(),
            source: ListingSource::Nip15Product,
            title: "Bit-Runners".to_string(),
            description: "Cyberpunk platformer with chain-linked item economy and community-run speedrun ladders.".to_string(),
            images: vec!["https://lh3.googleusercontent.com/aida-public/AB6AXuBppEh6duJunDcRrAlDyCHwjcLgKSLNrLn7urlFTA1JDEkbmtnYBzd_8RTWxEH0yhfLUX6wQa3QLRpQt89K69EpDGFa4DG6BcbpzyvRD9MKUR4kFURF1OHnGUsMf8pBgOnoVi2rpRC8MhhdLRTwAZGOCXgv4HUTOLToqmpkDBz1btGwBcD05i3nH5GAd2JOlqCOUwMiPrEuVPBCjSKOLd6HZ8owiUNaSNfVauMEYH3RM5Gx5tWR72rlRSNaHzmv2votTLxYPeMXM5k".to_string()],
            download_url: "https://example.com/bit-runners".to_string(),
            price: 0.0,
            currency: "SATS".to_string(),
            price_sats: 0,
            quantity: None,
            tags: vec!["platformer".to_string(), "cyberpunk".to_string()],
            specs: vec![("genre".to_string(), "Platformer".to_string())],
            publisher_npub: "npub1debugstorefront000000000000000000000000000000000000000000".to_string(),
            stall_id: "debug-stall-1".to_string(),
            stall_name: Some("Neon Arcade".to_string()),
            lud16: "debug@arcadestr.dev".to_string(),
            event_id: None,
            created_at: 1_710_000_002,
            platforms: Vec::new(),
            nip94_event_id: None,
            acquisition: crate::models::AcquisitionPolicy::Gated,
            campaigns: Vec::new(),
            is_owned: false,
            #[cfg(debug_assertions)]
            nip99_raw_event_json: None,
        },
        GameListing {
            id: "debug-dune-settlers".to_string(),
            source: ListingSource::Nip15Product,
            title: "Dune Settlers".to_string(),
            description: "Build a resilient off-world economy in a tactical strategy game tuned for async league play.".to_string(),
            images: vec!["https://lh3.googleusercontent.com/aida-public/AB6AXuBalkh2NCA6UZ04qa-pFXIL4N2iVby1eMnZRzDd9a2oAa9WYnFWl8OIykQNH3c4AcYN_aUwFcdGEXpllBbQf7Hz_j2HDQGKQaQXRZAmXB0nrdVNADrOeO4o5chwWjZYJKlC9Zp48Rwgt9m66yqG-k_rZ-Aot35r46iWmWCWdpye8690JqLNYoO0KmKmmTtAS2g8EsoY7eG58kSRXTaRsTqPVGPu7q43eYjpHizKEqucFvwzRT8C14m3Gji3_-ym2ZZqXrJI8pdohOA".to_string()],
            download_url: "https://example.com/dune-settlers".to_string(),
            price: 39000.0,
            currency: "SATS".to_string(),
            price_sats: 39000,
            quantity: None,
            tags: vec!["strategy".to_string(), "scifi".to_string()],
            specs: vec![("genre".to_string(), "Strategy".to_string())],
            publisher_npub: "npub1debugstorefront000000000000000000000000000000000000000000".to_string(),
            stall_id: "debug-stall-1".to_string(),
            stall_name: Some("Neon Arcade".to_string()),
            lud16: "debug@arcadestr.dev".to_string(),
            event_id: None,
            created_at: 1_710_000_003,
            platforms: Vec::new(),
            nip94_event_id: None,
            acquisition: crate::models::AcquisitionPolicy::Gated,
            campaigns: Vec::new(),
            is_owned: false,
            #[cfg(debug_assertions)]
            nip99_raw_event_json: None,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::ListingSource;

    fn listing_with_sats(price_sats: u64) -> GameListing {
        GameListing {
            id: "test".into(),
            source: ListingSource::Nip15Product,
            title: "Test".into(),
            description: "Desc".into(),
            images: vec![],
            download_url: "https://example.com".into(),
            price: price_sats as f64,
            currency: "SATS".into(),
            price_sats,
            quantity: None,
            tags: vec![],
            specs: vec![],
            publisher_npub: "npub1test0000".into(),
            stall_id: "stall".into(),
            stall_name: Some("Test Publisher".into()),
            lud16: "test@example.com".into(),
            event_id: None,
            created_at: 0,
            platforms: Vec::new(),
            nip94_event_id: None,
            acquisition: crate::models::AcquisitionPolicy::Gated,
            campaigns: Vec::new(),
            is_owned: false,
            #[cfg(debug_assertions)]
            nip99_raw_event_json: None,
        }
    }

    #[test]
    fn recent_listings_returns_limited_newest_first() {
        let listings = debug_mock_listings();

        let recent = recent_listings(listings, 2);

        let ids = recent
            .into_iter()
            .map(|listing| listing.id)
            .collect::<Vec<_>>();
        assert_eq!(ids, vec!["debug-dune-settlers", "debug-bit-runners"]);
    }

    #[test]
    fn limited_loader_can_show_once_limit_is_available() {
        assert!(!has_reached_listing_limit(2, 3));
        assert!(has_reached_listing_limit(3, 3));
        assert!(has_reached_listing_limit(4, 3));
    }

    #[test]
    fn older_page_cursor_excludes_already_loaded_oldest_listing() {
        let listings = debug_mock_listings();

        assert_eq!(older_page_until_secs(&listings), Some(1_710_000_000));
    }

    #[test]
    fn pagination_exhausts_only_when_page_returns_no_new_items() {
        assert!(has_more_after_page(50, 51));
        assert!(!has_more_after_page(50, 50));
    }

    #[test]
    fn upsert_latest_listing_rejects_stale_listing_after_newer_listing() {
        let mut stale = listing_with_sats(0);
        stale.created_at = 10;
        let mut newer = stale.clone();
        newer.created_at = 20;
        let mut listings = Vec::new();

        assert!(upsert_latest_listing(&mut listings, newer.clone()));
        assert!(!upsert_latest_listing(&mut listings, stale));

        assert_eq!(listings, vec![newer]);
    }

    #[test]
    fn upsert_latest_listing_uses_lower_event_id_for_equal_timestamps_in_both_orders() {
        let mut lower_id = listing_with_sats(0);
        lower_id.created_at = 20;
        lower_id.event_id = Some("aaa".into());
        let mut higher_id = lower_id.clone();
        higher_id.event_id = Some("bbb".into());

        for arrivals in [
            [higher_id.clone(), lower_id.clone()],
            [lower_id.clone(), higher_id.clone()],
        ] {
            let mut listings = Vec::new();
            for listing in arrivals {
                upsert_latest_listing(&mut listings, listing);
            }

            assert_eq!(listings, vec![lower_id.clone()]);
        }
    }

    #[test]
    fn received_coordinates_count_replacements_only_once() {
        let first = listing_with_sats(0);
        let mut replacement = first.clone();
        replacement.event_id = Some("replacement".into());
        let mut other_publisher = first.clone();
        other_publisher.publisher_npub = "npub1other".into();
        let mut seen = std::collections::HashSet::new();

        let unique_count = [first, replacement, other_publisher]
            .iter()
            .filter(|listing| mark_received_coordinate(&mut seen, listing))
            .count();

        assert_eq!(unique_count, 2);
    }

    #[test]
    fn game_card_store_page_stale_request_generation_is_ignored() {
        assert!(store_page_generation_matches(4, 4, 4));
        assert!(!store_page_generation_matches(5, 4, 4));
        assert!(!store_page_generation_matches(4, 4, 3));
    }
}
