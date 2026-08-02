use std::collections::HashMap;

use leptos::prelude::*;

use crate::models::{AcquisitionPolicy, GameListing, StorePageCardPresentation};
use crate::ui_v2::components::{
    artwork_state_from_url, ArtworkRole, ArtworkState, EmptyState, ErrorSeverity, ErrorState,
    GameArtwork, GameCard, GameCardAction, GameCardCampaign, GameCardPresentation,
    GameCardSkeleton, PartialRelayKind, PartialRelayState, PlatformCompatibility, Skeleton,
    SkeletonKind, StatusChip, StatusChipSize, StatusChipVariant,
};
use crate::ui_v2::views::browse_games::{extract_categories, listing_categories, BrowseRequest};
use crate::ui_v2::views::marketplace_loader::{
    canonical_listing_coordinate, listing_state_key, use_listing_campaign_states,
    use_listing_store_page_presentations, use_marketplace_listings_with_limit,
    CampaignAvailability,
};
use crate::ui_v2::views::valid_cover_url;

const STORE_FRONT_LISTING_LIMIT: usize = 24;
const STORE_CARD_LIMIT: usize = 4;
const STORE_CATEGORY_LIMIT: usize = 4;

#[component]
pub fn StoreFrontView(
    on_select: Callback<GameListing>,
    on_browse: Callback<BrowseRequest>,
) -> impl IntoView {
    let marketplace = use_marketplace_listings_with_limit(STORE_FRONT_LISTING_LIMIT);
    let listings = marketplace.listings;
    let campaign_state = use_listing_campaign_states(listings);
    let store_pages = use_listing_store_page_presentations(listings);
    let active_slide = RwSignal::new(0_usize);
    let featured = Signal::derive(move || featured_candidates(&listings.get()));
    let featured_listing = Signal::derive(move || {
        let candidates = featured.get();
        candidates
            .get(active_slide.get().min(candidates.len().saturating_sub(1)))
            .cloned()
    });
    let categories = Signal::derive(move || {
        extract_categories(&listings.get())
            .into_iter()
            .take(STORE_CATEGORY_LIMIT)
            .collect::<Vec<_>>()
    });
    let promotion = Signal::derive(move || {
        select_access_promotion(
            &listings.get(),
            &campaign_state.states.get(),
            current_unix_secs(),
        )
    });

    Effect::new(move |_| {
        let clamped = clamp_carousel_index(active_slide.get_untracked(), featured.get().len());
        if active_slide.get_untracked() != clamped {
            active_slide.set(clamped);
        }
    });

    view! {
        <section class="arc-store-home">
            {move || match featured_listing.get() {
                Some(listing) => {
                    let candidates = featured.get();
                    let candidate_count = candidates.len();
                    let current_index = active_slide.get().min(candidate_count.saturating_sub(1));
                    let store_page = canonical_listing_coordinate(&listing)
                        .and_then(|coordinate| store_pages.presentations.get().get(&coordinate).cloned());
                    let artwork = featured_artwork_state(&listing, store_page.as_ref());
                    let title = store_page
                        .as_ref()
                        .and_then(|page| page.title.clone())
                        .filter(|title| !title.trim().is_empty())
                        .unwrap_or_else(|| listing.title.clone());
                    let summary = store_page
                        .as_ref()
                        .and_then(|page| page.summary.clone())
                        .filter(|summary| !summary.trim().is_empty())
                        .unwrap_or_else(|| listing.description.clone());
                    let publisher = listing
                        .stall_name
                        .clone()
                        .filter(|name| !name.trim().is_empty())
                        .unwrap_or_else(|| crate::models::npub_fallback_label(&listing.publisher_npub));
                    let platform_label = if listing.platforms.is_empty() {
                        "Platforms not declared".to_string()
                    } else {
                        listing.platforms.iter().take(2).cloned().collect::<Vec<_>>().join(" · ")
                    };
                    let campaign = campaign_state
                        .states
                        .get()
                        .get(&listing_state_key(&listing))
                        .copied();
                    let access_label = featured_access_label(&listing, campaign, current_unix_secs());
                    let access_variant = featured_access_variant(&listing, campaign, current_unix_secs());
                    let selected = listing.clone();
                    let previous_disabled = current_index == 0;
                    let next_disabled = current_index + 1 >= candidate_count;
                    let hero_label = format!("Featured game: {title}");
                    let title_for_art = title.clone();

                    view! {
                        <article
                            class="arc-store-hero arc-clipped-panel"
                            tabindex="0"
                            aria-label=hero_label
                            on:keydown=move |event| {
                                if event.target() != event.current_target() {
                                    return;
                                }
                                match event.key().as_str() {
                                    "ArrowLeft" if !previous_disabled => {
                                        event.prevent_default();
                                        active_slide.set(previous_carousel_index(current_index, candidate_count));
                                    }
                                    "ArrowRight" if !next_disabled => {
                                        event.prevent_default();
                                        active_slide.set(next_carousel_index(current_index, candidate_count));
                                    }
                                    _ => {}
                                }
                            }
                        >
                            <div class="arc-store-hero-copy">
                                <div class="arc-store-hero-kicker"><span>"Featured game"</span><i aria-hidden="true"></i></div>
                                <h1>{title.clone()}</h1>
                                <p class="arc-store-hero-publisher">{format!("{publisher} · {platform_label}")}</p>
                                <div class="arc-store-hero-statuses">
                                    <StatusChip
                                        label=access_label
                                        variant=access_variant
                                        icon=None
                                        size=StatusChipSize::Standard
                                    />
                                    {listing.is_owned.then(|| view! {
                                        <StatusChip
                                            label="Owned"
                                            variant=StatusChipVariant::Owned
                                            icon=None
                                            size=StatusChipSize::Standard
                                        />
                                    })}
                                    {listing_categories(&listing)
                                        .into_iter()
                                        .take(2)
                                        .map(|category| view! {
                                            <span class="arc-store-hero-tag">{category.label}</span>
                                        })
                                        .collect_view()}
                                </div>
                                <p class="arc-store-hero-summary">{summary}</p>
                                <div class="arc-store-hero-actions">
                                    <button
                                        type="button"
                                        class="v2-btn-action arc-btn-clipped"
                                        on:click=move |_| on_select.run(selected.clone())
                                    >
                                        {hero_action_label()}
                                    </button>
                                </div>
                                <div class="arc-store-hero-capabilities" aria-label="Listing capabilities">
                                    <span><span class="material-symbols-outlined" aria-hidden="true">"hub"</span>"Relay discovered"</span>
                                    <span><span class="material-symbols-outlined" aria-hidden="true">"verified"</span>"Signed listing"</span>
                                </div>
                            </div>

                            <div class="arc-store-hero-art">
                                <GameArtwork
                                    title=title_for_art
                                    state=artwork
                                    role=ArtworkRole::Hero
                                />
                                <span class="arc-store-hero-art-shade" aria-hidden="true"></span>
                                {carousel_controls_visible(candidate_count).then(|| view! {
                                    <nav class="arc-store-carousel" aria-label="Featured games carousel">
                                        <div class="arc-store-carousel-indicators">
                                            {candidates
                                                .iter()
                                                .enumerate()
                                                .map(|(index, candidate)| {
                                                    let label = format!("Show featured game {}: {}", index + 1, candidate.title);
                                                    view! {
                                                        <button
                                                            id=format!("store-carousel-slide-{index}")
                                                            type="button"
                                                            class:active=index == current_index
                                                            aria-label=label
                                                            aria-current=(index == current_index).then_some("true")
                                                            on:click=move |_| set_carousel_index(
                                                                active_slide,
                                                                index,
                                                                Some(format!("store-carousel-slide-{index}")),
                                                            )
                                                        ></button>
                                                    }
                                                })
                                                .collect_view()}
                                        </div>
                                        <div class="arc-store-carousel-controls">
                                            <span>{format!("{:02} / {:02}", current_index + 1, candidate_count)}</span>
                                            <button
                                                id="store-carousel-previous"
                                                type="button"
                                                class="arc-icon-button"
                                                aria-label="Previous featured game"
                                                disabled=previous_disabled
                                                on:click=move |_| set_carousel_index(
                                                    active_slide,
                                                    previous_carousel_index(current_index, candidate_count),
                                                    Some("store-carousel-previous".to_string()),
                                                )
                                            >
                                                <span class="material-symbols-outlined" aria-hidden="true">"chevron_left"</span>
                                            </button>
                                            <button
                                                id="store-carousel-next"
                                                type="button"
                                                class="arc-icon-button"
                                                aria-label="Next featured game"
                                                disabled=next_disabled
                                                on:click=move |_| set_carousel_index(
                                                    active_slide,
                                                    next_carousel_index(current_index, candidate_count),
                                                    Some("store-carousel-next".to_string()),
                                                )
                                            >
                                                <span class="material-symbols-outlined" aria-hidden="true">"chevron_right"</span>
                                            </button>
                                        </div>
                                    </nav>
                                })}
                            </div>
                        </article>
                    }
                    .into_any()
                }
                None => match store_fallback_state(
                    listings.get().len(),
                    marketplace.loading.get(),
                    marketplace.error.get().is_some(),
                ) {
                    StoreFallbackState::Loading => view! {
                        <section class="arc-store-hero arc-store-hero-loading" role="status" aria-label="Loading featured game">
                            <div class="arc-store-hero-copy">
                                <Skeleton kind=SkeletonKind::Text class="arc-store-skeleton-kicker" />
                                <Skeleton kind=SkeletonKind::Text class="arc-store-skeleton-title" />
                                <Skeleton kind=SkeletonKind::Text class="arc-store-skeleton-meta" />
                                <Skeleton kind=SkeletonKind::Panel class="arc-store-skeleton-summary" />
                            </div>
                            <Skeleton kind=SkeletonKind::Panel class="arc-store-hero-art" />
                        </section>
                    }.into_any(),
                    StoreFallbackState::Error => view! {
                        <div class="arc-store-hero-state">
                            <ErrorState
                                title="Marketplace unavailable"
                                message="Connected relays could not provide marketplace listings."
                                severity=ErrorSeverity::Recoverable
                            />
                        </div>
                    }.into_any(),
                    StoreFallbackState::Empty => view! {
                        <div class="arc-store-hero-state">
                            <EmptyState
                                title="No marketplace listings"
                                description="No valid game listings were returned by the connected relays."
                                icon="sports_esports"
                            />
                        </div>
                    }.into_any(),
                    StoreFallbackState::NoFeaturedArtwork => view! {
                        <div class="arc-store-hero-state">
                            <EmptyState
                                title="No featured artwork available"
                                description="Loaded listings remain available below while featured artwork is unavailable."
                                icon="image_not_supported"
                            />
                        </div>
                    }.into_any(),
                },
            }}

            <div class="arc-store-content">
                {move || store_partial_result_kind(
                    listings.get().len(),
                    marketplace.refreshing.get(),
                    marketplace.error.get().is_some(),
                ).map(|kind| view! {
                    <PartialRelayState kind=kind result_count=listings.get().len() />
                })}
                <Show when=move || campaign_state.error.get().is_some() && !listings.get().is_empty()>
                    <p class="arc-store-enrichment-warning" role="status">"Some claim campaign statuses are currently unavailable."</p>
                </Show>

                <Show when=move || marketplace.loading.get() && listings.get().is_empty()>
                    <section class="arc-store-section" aria-labelledby="store-loading-title">
                        <div class="arc-store-section-heading"><h2 id="store-loading-title">"More on the store"</h2><i aria-hidden="true"></i></div>
                        <div class="arc-store-grid" role="status" aria-label="Loading store games">
                            <GameCardSkeleton />
                            <GameCardSkeleton />
                            <GameCardSkeleton />
                            <GameCardSkeleton />
                        </div>
                    </section>
                </Show>

                <Show when=move || !listings.get().is_empty()>
                    <section class="arc-store-section" aria-labelledby="store-listings-title">
                        <div class="arc-store-section-heading">
                            <h2 id="store-listings-title">"More on the store"</h2>
                            <i aria-hidden="true"></i>
                            <button type="button" on:click=move |_| on_browse.run(BrowseRequest::default())>"View all"</button>
                        </div>
                        <div class="arc-store-grid">
                            {move || {
                                let featured_key = featured_listing.get().map(|listing| listing_state_key(&listing));
                                let campaign_states = campaign_state.states.get();
                                let presentations = store_pages.presentations.get();
                                home_grid_listings(&listings.get(), featured_key.as_deref(), STORE_CARD_LIMIT)
                                    .into_iter()
                                    .map(|listing| {
                                        let store_page = canonical_listing_coordinate(&listing)
                                            .and_then(|coordinate| presentations.get(&coordinate).cloned());
                                        render_store_card(listing, store_page, on_select, &campaign_states)
                                    })
                                    .collect::<Vec<_>>()
                            }}
                        </div>
                    </section>
                </Show>

                {move || promotion.get().map(|promotion| {
                    let selected = promotion.listing.clone();
                    view! {
                        <section class="arc-store-secondary-section" aria-labelledby="store-access-title">
                            <div class="arc-store-section-heading"><h2 id="store-access-title">"Access highlights"</h2><i aria-hidden="true"></i></div>
                            <button
                                type="button"
                                class="arc-store-promotion arc-clipped-panel"
                                on:click=move |_| on_select.run(selected.clone())
                            >
                                <span class="material-symbols-outlined" aria-hidden="true">{promotion.icon}</span>
                                <span><strong>{promotion.label}</strong><b>{promotion.listing.title}</b><small>{promotion.description}</small></span>
                            </button>
                        </section>
                    }
                })}

                <Show when=move || !categories.get().is_empty()>
                    <section class="arc-store-secondary-section" aria-labelledby="store-categories-title">
                        <div class="arc-store-section-heading"><h2 id="store-categories-title">"Browse categories"</h2><i aria-hidden="true"></i></div>
                        <div class="arc-store-categories">
                            {move || categories.get().into_iter().map(|category| {
                                let request = BrowseRequest::for_category(category.key.clone());
                                view! {
                                    <button type="button" on:click=move |_| on_browse.run(request.clone())>{category.label}</button>
                                }
                            }).collect_view()}
                        </div>
                    </section>
                </Show>
            </div>
        </section>
    }
}

fn render_store_card(
    listing: GameListing,
    store_page: Option<StorePageCardPresentation>,
    on_select: Callback<GameListing>,
    campaign_states: &HashMap<String, CampaignAvailability>,
) -> AnyView {
    let campaign = campaign_states.get(&listing_state_key(&listing)).copied();
    let mut presentation = GameCardPresentation::from_listing(
        &listing,
        current_unix_secs(),
        PlatformCompatibility::Unknown,
        false,
    )
    .with_campaign(
        (!listing.is_owned)
            .then_some(campaign)
            .flatten()
            .map(|state| match state {
                CampaignAvailability::Active => GameCardCampaign::Active,
                CampaignAvailability::Upcoming => GameCardCampaign::Upcoming,
                CampaignAvailability::Ended => GameCardCampaign::Ended,
            }),
    );
    presentation.action = Some(GameCardAction::ViewDetails);
    let categories = listing_categories(&listing)
        .into_iter()
        .map(|category| category.label)
        .collect::<Vec<_>>();
    let selected = listing.clone();
    let selected_for_action = listing.clone();

    view! {
        <GameCard
            listing=listing
            presentation=presentation
            categories=categories
            store_page=store_page
            on_open=Callback::new(move |_| on_select.run(selected.clone()))
            on_action=Callback::new(move |_| on_select.run(selected_for_action.clone()))
        />
    }
    .into_any()
}

#[derive(Clone)]
struct AccessPromotion {
    listing: GameListing,
    icon: &'static str,
    label: &'static str,
    description: &'static str,
}

fn select_access_promotion(
    listings: &[GameListing],
    campaigns: &HashMap<String, CampaignAvailability>,
    now: u64,
) -> Option<AccessPromotion> {
    if let Some(listing) = listings.iter().find(|listing| {
        !listing.is_owned
            && campaigns.get(&listing_state_key(listing)) == Some(&CampaignAvailability::Active)
    }) {
        return Some(AccessPromotion {
            listing: listing.clone(),
            icon: "redeem",
            label: "Claim and keep",
            description:
                "An active campaign can issue a durable entitlement after a successful claim.",
        });
    }
    if let Some(listing) = listings
        .iter()
        .find(|listing| matches!(listing.acquisition, AcquisitionPolicy::Public))
    {
        return Some(AccessPromotion {
            listing: listing.clone(),
            icon: "public",
            label: "Public access",
            description: "This listing is currently accessible without creating durable ownership.",
        });
    }
    listings
        .iter()
        .find(|listing| {
            matches!(listing.acquisition, AcquisitionPolicy::TimedAccess { .. })
                && listing.acquisition.allows_access_at(now)
        })
        .map(|listing| AccessPromotion {
            listing: listing.clone(),
            icon: "schedule",
            label: "Timed access",
            description: "This listing is accessible only during its current configured interval.",
        })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum StoreFallbackState {
    Loading,
    Error,
    Empty,
    NoFeaturedArtwork,
}

fn featured_artwork_state(
    listing: &GameListing,
    store_page: Option<&StorePageCardPresentation>,
) -> ArtworkState {
    let enriched = store_page.and_then(|page| {
        page.hero_url
            .clone()
            .and_then(|url| valid_cover_url(&[url]))
            .or_else(|| {
                page.capsule_url
                    .clone()
                    .and_then(|url| valid_cover_url(&[url]))
            })
    });
    artwork_state_from_url(enriched.or_else(|| valid_cover_url(&listing.images)))
}

fn store_fallback_state(
    listing_count: usize,
    loading: bool,
    has_error: bool,
) -> StoreFallbackState {
    if loading {
        StoreFallbackState::Loading
    } else if has_error && listing_count == 0 {
        StoreFallbackState::Error
    } else if listing_count == 0 {
        StoreFallbackState::Empty
    } else {
        StoreFallbackState::NoFeaturedArtwork
    }
}

fn store_partial_result_kind(
    listing_count: usize,
    refreshing: bool,
    has_error: bool,
) -> Option<PartialRelayKind> {
    if listing_count == 0 {
        None
    } else if has_error {
        Some(PartialRelayKind::Failed)
    } else if refreshing {
        Some(PartialRelayKind::Loading)
    } else {
        None
    }
}

fn featured_candidates(listings: &[GameListing]) -> Vec<GameListing> {
    listings
        .iter()
        .filter(|listing| valid_cover_url(&listing.images).is_some())
        .cloned()
        .collect()
}

fn home_grid_listings(
    listings: &[GameListing],
    featured_key: Option<&str>,
    limit: usize,
) -> Vec<GameListing> {
    listings
        .iter()
        .filter(|listing| Some(listing_state_key(listing).as_str()) != featured_key)
        .take(limit)
        .cloned()
        .collect()
}

fn clamp_carousel_index(index: usize, item_count: usize) -> usize {
    index.min(item_count.saturating_sub(1))
}

fn previous_carousel_index(index: usize, item_count: usize) -> usize {
    clamp_carousel_index(index.saturating_sub(1), item_count)
}

fn next_carousel_index(index: usize, item_count: usize) -> usize {
    clamp_carousel_index(index.saturating_add(1), item_count)
}

fn carousel_controls_visible(item_count: usize) -> bool {
    item_count > 1
}

fn set_carousel_index(active_slide: RwSignal<usize>, index: usize, focus_id: Option<String>) {
    active_slide.set(index);

    #[cfg(target_arch = "wasm32")]
    if let Some(focus_id) = focus_id {
        leptos::task::spawn_local(async move {
            use wasm_bindgen::JsCast;

            gloo_timers::future::TimeoutFuture::new(0).await;
            if let Some(element) = web_sys::window()
                .and_then(|window| window.document())
                .and_then(|document| document.get_element_by_id(&focus_id))
                .and_then(|element| element.dyn_into::<web_sys::HtmlElement>().ok())
            {
                let _ = element.focus();
            }
        });
    }

    #[cfg(not(target_arch = "wasm32"))]
    let _ = focus_id;
}

fn hero_action_label() -> &'static str {
    "View details"
}

fn featured_access_label(
    listing: &GameListing,
    campaign: Option<CampaignAvailability>,
    now: u64,
) -> &'static str {
    if !listing.is_owned && campaign == Some(CampaignAvailability::Active) {
        "Claim and keep"
    } else {
        match listing.acquisition {
            AcquisitionPolicy::Public => "Public access",
            AcquisitionPolicy::TimedAccess { .. } if listing.acquisition.allows_access_at(now) => {
                "Timed access"
            }
            AcquisitionPolicy::TimedAccess { starts_at, .. } if now < starts_at => {
                "Timed access upcoming"
            }
            AcquisitionPolicy::TimedAccess { .. } => "Timed access ended",
            AcquisitionPolicy::Gated if listing.has_declared_price() => "Paid",
            AcquisitionPolicy::Gated => "Unavailable",
        }
    }
}

fn featured_access_variant(
    listing: &GameListing,
    campaign: Option<CampaignAvailability>,
    now: u64,
) -> StatusChipVariant {
    if !listing.is_owned && campaign == Some(CampaignAvailability::Active) {
        StatusChipVariant::Success
    } else {
        match listing.acquisition {
            AcquisitionPolicy::Public => StatusChipVariant::Public,
            AcquisitionPolicy::TimedAccess { .. } if listing.acquisition.allows_access_at(now) => {
                StatusChipVariant::TimedAccess
            }
            AcquisitionPolicy::TimedAccess { starts_at, .. } if now < starts_at => {
                StatusChipVariant::Pending
            }
            AcquisitionPolicy::TimedAccess { .. } => StatusChipVariant::Expired,
            AcquisitionPolicy::Gated if listing.has_declared_price() => StatusChipVariant::Active,
            AcquisitionPolicy::Gated => StatusChipVariant::Unavailable,
        }
    }
}

fn current_unix_secs() -> u64 {
    #[cfg(target_arch = "wasm32")]
    {
        (js_sys::Date::now() / 1000.0) as u64
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_secs())
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::ListingSource;

    fn listing(id: &str, created_at: u64, images: Vec<String>) -> GameListing {
        GameListing {
            id: id.into(),
            source: ListingSource::Nip99Listing,
            title: id.into(),
            description: String::new(),
            images,
            download_url: String::new(),
            price: 0.0,
            currency: "SATS".into(),
            price_sats: 0,
            quantity: None,
            tags: Vec::new(),
            specs: Vec::new(),
            publisher_npub: "npub1publisher".into(),
            stall_id: String::new(),
            stall_name: None,
            lud16: String::new(),
            event_id: None,
            created_at,
            platforms: Vec::new(),
            nip94_event_id: None,
            acquisition: AcquisitionPolicy::Gated,
            campaigns: Vec::new(),
            is_owned: false,
            #[cfg(debug_assertions)]
            nip99_raw_event_json: None,
        }
    }

    #[test]
    fn featured_selection_uses_first_marketplace_listing_with_usable_cover() {
        let listings = vec![
            listing("missing", 30, Vec::new()),
            listing(
                "newest-covered",
                20,
                vec!["https://cdn.arcadestr.test/game.jpg".into()],
            ),
            listing(
                "older-covered",
                10,
                vec!["https://cdn.arcadestr.test/older.jpg".into()],
            ),
        ];
        assert_eq!(
            featured_candidates(&listings)
                .first()
                .map(|item| item.id.as_str())
                .as_deref(),
            Some("newest-covered")
        );
    }

    #[test]
    fn featured_candidates_preserve_marketplace_order_without_duplicates() {
        let listings = vec![
            listing(
                "first",
                30,
                vec!["https://cdn.arcadestr.test/first.jpg".into()],
            ),
            listing("missing", 20, Vec::new()),
            listing(
                "second",
                10,
                vec!["https://cdn.arcadestr.test/second.jpg".into()],
            ),
        ];
        assert_eq!(
            featured_candidates(&listings)
                .into_iter()
                .map(|listing| listing.id)
                .collect::<Vec<_>>(),
            vec!["first", "second"]
        );
    }

    #[test]
    fn home_grid_excludes_active_hero_and_reduces_truthfully() {
        let listings = (0..6)
            .map(|index| listing(&format!("game-{index}"), 10 - index, Vec::new()))
            .collect::<Vec<_>>();
        let featured_key = listing_state_key(&listings[1]);
        let cards = home_grid_listings(&listings, Some(&featured_key), 4);
        assert_eq!(cards.len(), 4);
        assert!(cards.iter().all(|listing| listing.id != listings[1].id));

        let reduced = home_grid_listings(&listings[..2], None, 4);
        assert_eq!(reduced.len(), 2);
    }

    #[test]
    fn carousel_bounds_are_stable_and_controls_need_multiple_real_items() {
        assert_eq!(clamp_carousel_index(4, 2), 1);
        assert_eq!(previous_carousel_index(0, 2), 0);
        assert_eq!(next_carousel_index(0, 2), 1);
        assert_eq!(next_carousel_index(1, 2), 1);
        assert!(!carousel_controls_visible(0));
        assert!(!carousel_controls_visible(1));
        assert!(carousel_controls_visible(2));
    }

    #[test]
    fn hero_fallback_and_action_do_not_simulate_acquisition() {
        let missing = listing("missing", 1, Vec::new());
        assert_eq!(
            featured_artwork_state(&missing, None),
            ArtworkState::Missing
        );
        assert_eq!(hero_action_label(), "View details");

        let covered = listing(
            "covered",
            1,
            vec!["https://cdn.arcadestr.test/listing.jpg".into()],
        );
        let page = StorePageCardPresentation {
            listing_coordinate: "30402:publisher:covered".into(),
            store_page_coordinate: "30407:publisher:page".into(),
            event_id: "event".into(),
            title: None,
            summary: None,
            capsule_url: Some("not-a-url".into()),
            hero_url: Some(" ".into()),
            genres: Vec::new(),
            features: Vec::new(),
            release_date: None,
        };
        assert_eq!(
            featured_artwork_state(&covered, Some(&page)),
            ArtworkState::Available("https://cdn.arcadestr.test/listing.jpg".into())
        );
    }

    #[test]
    fn store_loading_error_empty_and_missing_artwork_are_distinct() {
        assert_eq!(
            store_fallback_state(0, true, false),
            StoreFallbackState::Loading
        );
        assert_eq!(
            store_fallback_state(0, false, true),
            StoreFallbackState::Error
        );
        assert_eq!(
            store_fallback_state(0, false, false),
            StoreFallbackState::Empty
        );
        assert_eq!(
            store_fallback_state(2, false, false),
            StoreFallbackState::NoFeaturedArtwork
        );
        assert_eq!(
            store_partial_result_kind(4, true, false),
            Some(PartialRelayKind::Loading)
        );
        assert_eq!(
            store_partial_result_kind(4, true, true),
            Some(PartialRelayKind::Failed)
        );
        assert_eq!(store_partial_result_kind(0, true, true), None);
    }

    #[test]
    fn hero_access_and_ownership_remain_independent() {
        let mut owned_paid = listing("owned", 1, Vec::new());
        owned_paid.price_sats = 2_100;
        owned_paid.is_owned = true;
        assert_eq!(featured_access_label(&owned_paid, None, 10), "Paid");
        assert_eq!(
            featured_access_variant(&owned_paid, None, 10),
            StatusChipVariant::Active
        );
        assert!(owned_paid.is_owned);
    }

    #[test]
    fn promotion_precedence_is_active_claim_then_public_then_active_timed() {
        let mut claim = listing("claim", 30, Vec::new());
        claim.campaigns.push(crate::models::CampaignPointer {
            root_event_id: "root".into(),
            relay_hint: None,
        });
        let mut public = listing("public", 20, Vec::new());
        public.acquisition = AcquisitionPolicy::Public;
        let mut timed = listing("timed", 10, Vec::new());
        timed.acquisition = AcquisitionPolicy::TimedAccess {
            starts_at: 10,
            ends_at: 30,
        };
        let mut campaigns = HashMap::new();
        campaigns.insert(listing_state_key(&claim), CampaignAvailability::Active);
        assert_eq!(
            select_access_promotion(
                &[public.clone(), timed.clone(), claim.clone()],
                &campaigns,
                20
            )
            .map(|item| item.listing.id)
            .as_deref(),
            Some("claim")
        );
        campaigns.clear();
        assert_eq!(
            select_access_promotion(&[timed, public], &campaigns, 20)
                .map(|item| item.listing.id)
                .as_deref(),
            Some("public")
        );
    }
}
