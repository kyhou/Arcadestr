//! Browse-all games using shared, progressively streamed marketplace state.

use std::cmp::Ordering;
use std::collections::HashSet;

use leptos::prelude::*;
use leptos::task::spawn_local;

use crate::models::{AcquisitionPolicy, GameListing, PlatformInfo};
use crate::tauri_bridge::{invoke_get_installed_games, invoke_get_platform_info};
#[cfg(feature = "web")]
use crate::ui_v2::components::GameCardAction;
use crate::ui_v2::components::{
    GameCard, GameCardCampaign, GameCardPresentation, PageHeader, PlatformCompatibility,
};
use crate::ui_v2::views::marketplace_loader::{
    listing_state_key, use_listing_campaign_states, use_marketplace_listings, CampaignAvailability,
};

const BROWSE_INITIAL_VISIBLE_COUNT: usize = 50;
const BROWSE_VISIBLE_INCREMENT: usize = 50;
const MAX_PLATFORM_AUTO_FETCHES: usize = 4;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BrowseRequest {
    pub category: Option<String>,
    pub query: Option<String>,
}

impl BrowseRequest {
    pub fn for_category(category: impl Into<String>) -> Self {
        Self {
            category: Some(normalize_filter_value(&category.into())),
            query: None,
        }
    }

    pub fn for_query(query: impl Into<String>) -> Self {
        let query = query.into().trim().to_string();
        Self {
            category: None,
            query: (!query.is_empty()).then_some(query),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum AcquisitionFilter {
    #[default]
    All,
    Paid,
    Claim,
    Public,
    Timed,
    Owned,
}

impl AcquisitionFilter {
    fn value(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::Paid => "paid",
            Self::Claim => "claim",
            Self::Public => "public",
            Self::Timed => "timed",
            Self::Owned => "owned",
        }
    }

    fn from_value(value: &str) -> Self {
        match value {
            "paid" => Self::Paid,
            "claim" => Self::Claim,
            "public" => Self::Public,
            "timed" => Self::Timed,
            "owned" => Self::Owned,
            _ => Self::All,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum SortOption {
    #[default]
    Recommended,
    Newest,
    PriceLow,
    PriceHigh,
    Title,
}

impl SortOption {
    fn value(self) -> &'static str {
        match self {
            Self::Recommended => "recommended",
            Self::Newest => "newest",
            Self::PriceLow => "price-low",
            Self::PriceHigh => "price-high",
            Self::Title => "title",
        }
    }

    fn from_value(value: &str) -> Self {
        match value {
            "newest" => Self::Newest,
            "price-low" => Self::PriceLow,
            "price-high" => Self::PriceHigh,
            "title" => Self::Title,
            _ => Self::Recommended,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CategoryOption {
    pub key: String,
    pub label: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct PlatformAutoFetchState {
    baseline_displayed_count: usize,
    baseline_loaded_count: usize,
    attempts: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PlatformAutoFetchDecision {
    Wait,
    Fetch,
    Stop,
}

#[component]
pub fn BrowseGamesView(on_select: Callback<GameListing>, request: BrowseRequest) -> impl IntoView {
    let marketplace = use_marketplace_listings();
    let listings = marketplace.listings;
    let campaign_state = use_listing_campaign_states(listings);
    let query = RwSignal::new(request.query.unwrap_or_default());
    let active_category =
        RwSignal::new(request.category.map(|value| normalize_filter_value(&value)));
    let acquisition_filter = RwSignal::new(AcquisitionFilter::All);
    let sort = RwSignal::new(SortOption::Recommended);
    let visible_count = RwSignal::new(BROWSE_INITIAL_VISIBLE_COUNT);
    let platform_info = RwSignal::new(None::<PlatformInfo>);
    let active_platform_filter = RwSignal::new(None::<String>);
    let platform_auto_fetch = RwSignal::new(None::<PlatformAutoFetchState>);
    let installed_coordinates = RwSignal::new(HashSet::<String>::new());

    Effect::new(move |_| {
        spawn_local(async move {
            if let Ok(info) = invoke_get_platform_info().await {
                platform_info.set(Some(info));
            }
        });
    });

    Effect::new(move |_| {
        spawn_local(async move {
            if let Ok(installed) = invoke_get_installed_games().await {
                installed_coordinates.set(
                    installed
                        .into_iter()
                        .map(|game| game.game_coordinate)
                        .collect(),
                );
            }
        });
    });

    let host_platform_tag = Signal::derive(move || platform_info.get().map(|info| info.tag()));
    let categories = Signal::derive(move || extract_categories(&listings.get()));
    let platform_choices = Signal::derive(move || extract_platform_choices(&listings.get()));

    let filtered_listings = Signal::derive(move || {
        let query = normalize_search(&query.get());
        let category = active_category.get();
        let acquisition = acquisition_filter.get();
        let platform = active_platform_filter.get();
        let campaign_states = campaign_state.states.get();
        let mut filtered = listings
            .get()
            .into_iter()
            .filter(|listing| {
                let campaign = campaign_states.get(&listing_state_key(listing)).copied();
                matches_search(listing, &query)
                    && matches_category(listing, category.as_deref())
                    && matches_acquisition_filter(listing, campaign, acquisition)
                    && listing_matches_platform_filter(&listing.platforms, platform.as_deref())
            })
            .collect::<Vec<_>>();
        sort_listings(
            &mut filtered,
            sort.get(),
            &campaign_states,
            current_unix_secs(),
        );
        filtered
    });

    Effect::new(move |_| {
        let decision = decide_platform_auto_fetch(
            platform_auto_fetch.get(),
            filtered_listings.get().len(),
            listings.get().len(),
            active_platform_filter.get().as_deref(),
            marketplace.has_more.get(),
            MAX_PLATFORM_AUTO_FETCHES,
        );
        match decision {
            PlatformAutoFetchDecision::Wait => {}
            PlatformAutoFetchDecision::Stop => platform_auto_fetch.set(None),
            PlatformAutoFetchDecision::Fetch => {
                let loaded_count = listings.get_untracked().len();
                let displayed_count = filtered_listings.get_untracked().len();
                let attempts = platform_auto_fetch
                    .get_untracked()
                    .map(|state| state.attempts.saturating_add(1))
                    .unwrap_or(1);
                platform_auto_fetch.set(Some(PlatformAutoFetchState {
                    baseline_displayed_count: displayed_count,
                    baseline_loaded_count: loaded_count,
                    attempts,
                }));
                marketplace.requested_limit.update(|limit| {
                    *limit = (*limit).max(loaded_count.saturating_add(BROWSE_VISIBLE_INCREMENT));
                });
            }
        }
    });

    let filters_active = Signal::derive(move || {
        !normalize_search(&query.get()).is_empty()
            || active_category.get().is_some()
            || acquisition_filter.get() != AcquisitionFilter::All
            || active_platform_filter.get().is_some()
            || sort.get() != SortOption::Recommended
    });

    view! {
        <section class="p-4 md:p-8">
            <PageHeader
                eyebrow="Catalog".to_string()
                title="Browse games".to_string()
                description="Search and filter listings currently loaded from your connected Nostr relays.".to_string()
            />

            <div class="mb-6 grid gap-4 rounded-2xl bg-surface-container-low p-4 lg:grid-cols-[minmax(0,1fr)_repeat(3,minmax(10rem,auto))]">
                <label class="block min-w-0">
                    <span class="mb-1.5 block text-xs font-semibold uppercase tracking-widest text-on-surface-variant">"Search"</span>
                    <span class="relative block">
                        <span class="material-symbols-outlined pointer-events-none absolute left-3 top-1/2 -translate-y-1/2 text-lg text-on-surface-variant" aria-hidden="true">"search"</span>
                        <input
                            type="search"
                            class="w-full rounded-xl border border-white/10 bg-surface-container-high py-3 pl-10 pr-4 text-sm text-on-surface outline-none ring-primary/60 placeholder:text-on-surface-variant focus-visible:ring-2"
                            placeholder="Title, description, publisher, or category"
                            prop:value=move || query.get()
                            on:input=move |event| query.set(event_target_value(&event))
                        />
                    </span>
                </label>

                <label class="block">
                    <span class="mb-1.5 block text-xs font-semibold uppercase tracking-widest text-on-surface-variant">"Access"</span>
                    <select
                        class="w-full rounded-xl border border-white/10 bg-surface-container-high px-3 py-3 text-sm text-on-surface outline-none ring-primary/60 focus-visible:ring-2"
                        prop:value=move || acquisition_filter.get().value()
                        on:change=move |event| acquisition_filter.set(AcquisitionFilter::from_value(&event_target_value(&event)))
                    >
                        <option value="all">"All access"</option>
                        <option value="paid">"Paid"</option>
                        <option value="claim">"Claim and keep"</option>
                        <option value="public">"Public access"</option>
                        <option value="timed">"Timed access"</option>
                        <option value="owned">"Owned"</option>
                    </select>
                </label>

                <label class="block">
                    <span class="mb-1.5 block text-xs font-semibold uppercase tracking-widest text-on-surface-variant">"Platform"</span>
                    <select
                        class="w-full rounded-xl border border-white/10 bg-surface-container-high px-3 py-3 text-sm text-on-surface outline-none ring-primary/60 focus-visible:ring-2"
                        prop:value=move || active_platform_filter.get().unwrap_or_default()
                        on:change=move |event| {
                            let value = event_target_value(&event);
                            let next = (!value.is_empty()).then_some(value);
                            active_platform_filter.set(next.clone());
                            visible_count.set(BROWSE_INITIAL_VISIBLE_COUNT);
                            if next.is_some() && marketplace.has_more.get_untracked() {
                                let loaded = listings.get_untracked().len();
                                let displayed = filtered_listings.get_untracked().len();
                                platform_auto_fetch.set(Some(PlatformAutoFetchState {
                                    baseline_displayed_count: displayed,
                                    baseline_loaded_count: loaded,
                                    attempts: 0,
                                }));
                                marketplace.requested_limit.update(|limit| {
                                    *limit = (*limit).max(loaded.saturating_add(BROWSE_VISIBLE_INCREMENT));
                                });
                            } else {
                                platform_auto_fetch.set(None);
                            }
                        }
                    >
                        <option value="">"All platforms"</option>
                        {move || host_platform_tag.get().map(|tag| view! {
                            <option value=tag.clone()>{format!("My platform ({tag})")}</option>
                        })}
                        {move || platform_choices.get().into_iter().filter(|tag| Some(tag) != host_platform_tag.get().as_ref()).map(|tag| {
                            view! { <option value=tag.clone()>{tag.clone()}</option> }
                        }).collect_view()}
                    </select>
                </label>

                <label class="block">
                    <span class="mb-1.5 block text-xs font-semibold uppercase tracking-widest text-on-surface-variant">"Sort"</span>
                    <select
                        class="w-full rounded-xl border border-white/10 bg-surface-container-high px-3 py-3 text-sm text-on-surface outline-none ring-primary/60 focus-visible:ring-2"
                        prop:value=move || sort.get().value()
                        on:change=move |event| sort.set(SortOption::from_value(&event_target_value(&event)))
                    >
                        <option value="recommended">"Recommended"</option>
                        <option value="newest">"Newest"</option>
                        <option value="price-low">"Price: low to high"</option>
                        <option value="price-high">"Price: high to low"</option>
                        <option value="title">"Title"</option>
                    </select>
                </label>
            </div>

            <div class="mb-6 flex flex-wrap items-center gap-2">
                <span class="mr-1 text-xs font-semibold uppercase tracking-widest text-on-surface-variant">"Category"</span>
                <button
                    type="button"
                    class=move || filter_chip_class(active_category.get().is_none())
                    aria-pressed=move || active_category.get().is_none()
                    on:click=move |_| active_category.set(None)
                >"All"</button>
                {move || categories.get().into_iter().map(|category| {
                    let key = category.key.clone();
                    let key_for_active = key.clone();
                    let key_for_click = key.clone();
                    view! {
                        <button
                            type="button"
                            class=move || filter_chip_class(active_category.get().as_deref() == Some(key_for_active.as_str()))
                            aria-pressed=move || active_category.get().as_deref() == Some(key.as_str())
                            on:click=move |_| active_category.set(Some(key_for_click.clone()))
                        >{category.label}</button>
                    }
                }).collect_view()}
            </div>

            <div class="mb-5 flex flex-wrap items-center justify-between gap-3">
                <p class="text-sm text-on-surface-variant" aria-live="polite">
                    {move || format!("{} loaded result{}", filtered_listings.get().len(), if filtered_listings.get().len() == 1 { "" } else { "s" })}
                </p>
                <Show when=move || filters_active.get()>
                    <button
                        type="button"
                        class="rounded-xl px-3 py-2 text-sm font-semibold text-primary outline-none ring-primary/60 hover:bg-primary/10 focus-visible:ring-2"
                        on:click=move |_| {
                            query.set(String::new());
                            active_category.set(None);
                            acquisition_filter.set(AcquisitionFilter::All);
                            active_platform_filter.set(None);
                            sort.set(SortOption::Recommended);
                            platform_auto_fetch.set(None);
                            visible_count.set(BROWSE_INITIAL_VISIBLE_COUNT);
                        }
                    >"Clear filters"</button>
                </Show>
            </div>

            <Show when=move || marketplace.refreshing.get() && !listings.get().is_empty()>
                <p class="mb-5 rounded-xl border border-secondary/20 bg-secondary/10 px-4 py-3 text-sm text-secondary" role="status">
                    "Showing cached listings while refreshing from relays."
                </p>
            </Show>
            <Show when=move || marketplace.error.get().is_some() && !listings.get().is_empty()>
                <p class="mb-5 rounded-xl border border-tertiary/25 bg-tertiary/10 px-4 py-3 text-sm text-tertiary" role="status">
                    {move || format!("Relay refresh failed; cached listings remain available. {}", marketplace.error.get().unwrap_or_default())}
                </p>
            </Show>
            <Show when=move || campaign_state.loading.get() && !listings.get().is_empty()>
                <p class="mb-5 text-xs text-on-surface-variant" role="status">"Checking claim-and-keep campaigns..."</p>
            </Show>
            <Show when=move || campaign_state.error.get().is_some() && !listings.get().is_empty()>
                <p class="mb-5 text-xs text-tertiary" role="status">"Some claim campaign statuses are currently unavailable."</p>
            </Show>
            {move || {
                let loaded = listings.get();
                let filtered = filtered_listings.get();
                if loaded.is_empty() && marketplace.loading.get() {
                    view! {
                        <div class="rounded-2xl bg-surface-container-high p-10 text-center text-on-surface-variant" role="status">
                            {if marketplace.received_count.get() > 0 {
                                format!("Loading marketplace... {} listings received", marketplace.received_count.get())
                            } else {
                                "Loading marketplace from connected relays...".to_string()
                            }}
                        </div>
                    }.into_any()
                } else if loaded.is_empty() && marketplace.error.get().is_some() {
                    view! {
                        <div class="rounded-2xl border border-error/30 bg-error/10 p-10 text-center">
                            <h2 class="font-display text-xl font-semibold text-on-surface">"Marketplace unavailable"</h2>
                            <p class="mt-2 text-sm text-error">{marketplace.error.get().unwrap_or_default()}</p>
                        </div>
                    }.into_any()
                } else if loaded.is_empty() {
                    view! {
                        <div class="rounded-2xl bg-surface-container-high p-10 text-center">
                            <h2 class="font-display text-xl font-semibold">"No marketplace listings"</h2>
                            <p class="mt-2 text-sm text-on-surface-variant">"No valid game listings were returned by the connected relays."</p>
                        </div>
                    }.into_any()
                } else if filtered.is_empty() {
                    view! {
                        <div class="rounded-2xl bg-surface-container-high p-10 text-center">
                            <h2 class="font-display text-xl font-semibold">"No games match these filters"</h2>
                            <p class="mt-2 text-sm text-on-surface-variant">"Clear or adjust filters to see the currently loaded marketplace results."</p>
                        </div>
                    }.into_any()
                } else {
                    let host_tag = host_platform_tag.get();
                    let active_platform = active_platform_filter.get();
                    let campaign_states = campaign_state.states.get();
                    let installed = installed_coordinates.get();
                    view! {
                        <div class="grid grid-cols-1 gap-5 sm:grid-cols-2 xl:grid-cols-3 2xl:grid-cols-4">
                            {filtered.into_iter().take(visible_count.get()).map(|listing| {
                                render_game_card(
                                    listing,
                                    on_select,
                                    &campaign_states,
                                    &installed,
                                    host_tag.as_deref(),
                                    active_platform.as_deref(),
                                )
                            }).collect::<Vec<_>>()}
                        </div>
                    }.into_any()
                }
            }}

            {move || {
                let filtered_count = filtered_listings.get().len();
                if marketplace.loading_more.get() {
                    view! {
                        <div class="mt-10 flex justify-center" role="status">
                            <span class="rounded-full bg-surface-container-low px-6 py-3 text-sm font-semibold text-on-surface-variant">"Loading more listings..."</span>
                        </div>
                    }.into_any()
                } else if can_load_more_browse_cards(visible_count.get(), filtered_count, marketplace.has_more.get()) {
                    view! {
                        <div class="mt-10 flex justify-center">
                            <button
                                type="button"
                                class="rounded-full border border-white/10 bg-surface-container-low px-8 py-3 text-sm font-semibold text-on-surface outline-none ring-primary/60 hover:bg-surface-container-high focus-visible:ring-2"
                                on:click=move |_| {
                                    let displayed_count = filtered_listings.get_untracked().len();
                                    let loaded_count = listings.get_untracked().len();
                                    let next_visible = next_visible_count(visible_count.get_untracked(), displayed_count, BROWSE_VISIBLE_INCREMENT);
                                    visible_count.set(next_visible);
                                    let required_limit = required_listing_limit(next_visible);
                                    let fetch_limit = platform_load_more_fetch_limit(
                                        active_platform_filter.get_untracked().as_deref(),
                                        marketplace.has_more.get_untracked(),
                                        displayed_count,
                                        next_visible,
                                        loaded_count,
                                        BROWSE_VISIBLE_INCREMENT,
                                    ).or_else(|| next_fetch_limit(loaded_count, required_limit).map(|_| required_limit));
                                    if let Some(fetch_limit) = fetch_limit {
                                        if active_platform_filter.get_untracked().is_some() && marketplace.has_more.get_untracked() {
                                            platform_auto_fetch.set(Some(PlatformAutoFetchState {
                                                baseline_displayed_count: displayed_count,
                                                baseline_loaded_count: loaded_count,
                                                attempts: 0,
                                            }));
                                        }
                                        marketplace.requested_limit.update(|limit| *limit = (*limit).max(fetch_limit));
                                    }
                                }
                            >"Load more"</button>
                        </div>
                    }.into_any()
                } else if show_no_more_platform_message(
                    active_platform_filter.get().as_deref(),
                    marketplace.has_more.get(),
                    marketplace.loading.get(),
                    marketplace.loading_more.get(),
                ) {
                    view! {
                        <p class="mt-10 text-center text-sm text-on-surface-variant">"No more games are available for this platform in the loaded marketplace."</p>
                    }.into_any()
                } else {
                    view! { <></> }.into_any()
                }
            }}
        </section>
    }
}

fn render_game_card(
    listing: GameListing,
    on_select: Callback<GameListing>,
    campaign_states: &std::collections::HashMap<String, CampaignAvailability>,
    installed_coordinates: &HashSet<String>,
    host_tag: Option<&str>,
    active_platform_filter: Option<&str>,
) -> AnyView {
    let campaign = campaign_states.get(&listing_state_key(&listing)).copied();
    let installed = game_coordinate(&listing)
        .as_ref()
        .is_some_and(|coordinate| installed_coordinates.contains(coordinate));
    let compatibility = listing_compatibility(&listing.platforms, host_tag, active_platform_filter);
    let presentation = card_presentation(&listing, campaign, compatibility, installed);
    let categories = listing_categories(&listing)
        .into_iter()
        .map(|category| category.label)
        .collect::<Vec<_>>();
    let listing_for_open = listing.clone();
    let listing_for_action = listing.clone();
    let action_callback = Callback::new(move |_| on_select.run(listing_for_action.clone()));

    view! {
        <GameCard
            listing=listing
            presentation=presentation
            categories=categories
            on_open=Callback::new(move |_| on_select.run(listing_for_open.clone()))
            on_action=action_callback
        />
    }
    .into_any()
}

fn card_presentation(
    listing: &GameListing,
    campaign: Option<CampaignAvailability>,
    compatibility: PlatformCompatibility,
    installed: bool,
) -> GameCardPresentation {
    let mut presentation =
        GameCardPresentation::from_listing(listing, current_unix_secs(), compatibility, installed);
    presentation =
        presentation.with_campaign((!listing.is_owned).then_some(campaign).flatten().map(
            |campaign| match campaign {
                CampaignAvailability::Active => GameCardCampaign::Active,
                CampaignAvailability::Upcoming => GameCardCampaign::Upcoming,
                CampaignAvailability::Ended => GameCardCampaign::Ended,
            },
        ));
    #[cfg(feature = "web")]
    if presentation.action == Some(GameCardAction::Install) {
        presentation.action = Some(GameCardAction::ViewDetails);
    }
    presentation
}

fn game_coordinate(listing: &GameListing) -> Option<String> {
    use nostr::nips::nip19::FromBech32;
    nostr::PublicKey::from_bech32(&listing.publisher_npub)
        .ok()
        .map(|publisher| format!("30402:{}:{}", publisher.to_hex(), listing.id))
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

fn filter_chip_class(active: bool) -> &'static str {
    if active {
        "rounded-full bg-primary/20 px-3 py-1.5 text-xs font-semibold text-primary outline-none ring-primary/60 focus-visible:ring-2"
    } else {
        "rounded-full bg-surface-container-high px-3 py-1.5 text-xs font-medium text-on-surface-variant outline-none ring-primary/60 hover:text-on-surface focus-visible:ring-2"
    }
}

fn normalize_search(value: &str) -> String {
    value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

fn normalize_filter_value(value: &str) -> String {
    normalize_search(value)
}

fn matches_search(listing: &GameListing, normalized_query: &str) -> bool {
    if normalized_query.is_empty() {
        return true;
    }
    let searchable = [
        listing.title.as_str(),
        listing.description.as_str(),
        listing.stall_name.as_deref().unwrap_or_default(),
        listing.publisher_npub.as_str(),
    ]
    .into_iter()
    .chain(listing.tags.iter().map(String::as_str))
    .chain(
        listing
            .specs
            .iter()
            .flat_map(|(key, value)| [key.as_str(), value.as_str()]),
    )
    .collect::<Vec<_>>()
    .join(" ")
    .to_lowercase();
    normalized_query
        .split_whitespace()
        .all(|term| searchable.contains(term))
}

pub(crate) fn extract_categories(listings: &[GameListing]) -> Vec<CategoryOption> {
    let mut seen = HashSet::new();
    let mut categories = Vec::new();
    for listing in listings {
        for category in listing_categories(listing) {
            if seen.insert(category.key.clone()) {
                categories.push(category);
            }
        }
    }
    categories
}

pub(crate) fn listing_categories(listing: &GameListing) -> Vec<CategoryOption> {
    listing
        .tags
        .iter()
        .filter_map(|tag| {
            let trimmed = tag.trim();
            let key = normalize_filter_value(trimmed);
            (!key.is_empty() && !is_technical_tag(&key, &listing.platforms)).then(|| {
                CategoryOption {
                    key,
                    label: trimmed.to_string(),
                }
            })
        })
        .collect()
}

fn is_technical_tag(normalized: &str, platforms: &[String]) -> bool {
    const TECHNICAL: &[&str] = &[
        "game",
        "games",
        "gaming",
        "nostr",
        "adp",
        "nip-15",
        "nip15",
        "nip-94",
        "nip94",
        "nip-99",
        "nip99",
        "30402",
        "30403",
        "digital",
        "download",
        "sha256",
        "application/zip",
    ];
    TECHNICAL.contains(&normalized)
        || normalized.starts_with("platform:")
        || normalized.starts_with("protocol:")
        || normalized.starts_with("mime:")
        || platforms
            .iter()
            .any(|platform| normalize_filter_value(platform) == normalized)
        || ["windows-", "linux-", "macos-", "android-", "ios-"]
            .iter()
            .any(|prefix| normalized.starts_with(prefix))
}

fn matches_category(listing: &GameListing, category: Option<&str>) -> bool {
    category.is_none_or(|category| {
        listing_categories(listing)
            .iter()
            .any(|option| option.key == category)
    })
}

fn matches_acquisition_filter(
    listing: &GameListing,
    campaign: Option<CampaignAvailability>,
    filter: AcquisitionFilter,
) -> bool {
    match filter {
        AcquisitionFilter::All => true,
        AcquisitionFilter::Paid => {
            matches!(listing.acquisition, AcquisitionPolicy::Gated) && listing.has_declared_price()
        }
        AcquisitionFilter::Claim => {
            !listing.is_owned && campaign == Some(CampaignAvailability::Active)
        }
        AcquisitionFilter::Public => matches!(listing.acquisition, AcquisitionPolicy::Public),
        AcquisitionFilter::Timed => {
            matches!(listing.acquisition, AcquisitionPolicy::TimedAccess { .. })
        }
        AcquisitionFilter::Owned => listing.is_owned,
    }
}

fn sort_listings(
    listings: &mut [GameListing],
    sort: SortOption,
    campaigns: &std::collections::HashMap<String, CampaignAvailability>,
    now: u64,
) {
    match sort {
        SortOption::Recommended => {}
        SortOption::Newest => {
            listings.sort_by(|left, right| right.created_at.cmp(&left.created_at))
        }
        SortOption::PriceLow => listings.sort_by(|left, right| {
            price_sort_key(left, campaigns.get(&listing_state_key(left)).copied(), now).cmp(
                &price_sort_key(
                    right,
                    campaigns.get(&listing_state_key(right)).copied(),
                    now,
                ),
            )
        }),
        SortOption::PriceHigh => listings.sort_by(|left, right| {
            compare_price_high(
                price_sort_key(left, campaigns.get(&listing_state_key(left)).copied(), now),
                price_sort_key(
                    right,
                    campaigns.get(&listing_state_key(right)).copied(),
                    now,
                ),
            )
        }),
        SortOption::Title => listings
            .sort_by(|left, right| left.title.to_lowercase().cmp(&right.title.to_lowercase())),
    }
}

fn compare_price_high(left: (u8, u64), right: (u8, u64)) -> Ordering {
    match (left, right) {
        ((1, left_price), (1, right_price)) => right_price.cmp(&left_price),
        (left_key, right_key) => high_price_bucket(left_key.0).cmp(&high_price_bucket(right_key.0)),
    }
}

fn high_price_bucket(bucket: u8) -> u8 {
    match bucket {
        1 => 0,
        0 => 1,
        _ => 2,
    }
}

fn price_sort_key(
    listing: &GameListing,
    campaign: Option<CampaignAvailability>,
    now: u64,
) -> (u8, u64) {
    let currently_no_cost = matches!(listing.acquisition, AcquisitionPolicy::Public)
        || (!listing.is_owned && campaign == Some(CampaignAvailability::Active))
        || matches!(
            listing.acquisition,
            AcquisitionPolicy::TimedAccess { .. } if listing.acquisition.allows_access_at(now)
        );
    if currently_no_cost {
        (0, 0)
    } else if listing.has_declared_price() && listing.price_sats > 0 {
        (1, listing.price_sats)
    } else {
        (2, u64::MAX)
    }
}

fn extract_platform_choices(listings: &[GameListing]) -> Vec<String> {
    let mut choices = Vec::new();
    let mut seen = HashSet::new();
    for platform in listings.iter().flat_map(|listing| listing.platforms.iter()) {
        if seen.insert(platform.clone()) {
            choices.push(platform.clone());
        }
    }
    choices
}

fn next_visible_count(current: usize, _total: usize, increment: usize) -> usize {
    current.saturating_add(increment)
}

fn has_more_browse_cards(visible: usize, total: usize) -> bool {
    visible < total
}

fn can_load_more_browse_cards(visible: usize, total_cards: usize, has_more: bool) -> bool {
    has_more_browse_cards(visible, total_cards) || has_more
}

fn required_listing_limit(visible_cards: usize) -> usize {
    visible_cards
}

fn next_fetch_limit(loaded_count: usize, requested_limit: usize) -> Option<usize> {
    requested_limit
        .checked_sub(loaded_count)
        .filter(|limit| *limit > 0)
}

fn platform_load_more_fetch_limit(
    active_filter: Option<&str>,
    has_more: bool,
    displayed_cards: usize,
    next_visible: usize,
    loaded_count: usize,
    increment: usize,
) -> Option<usize> {
    if active_filter.is_some() && has_more && displayed_cards <= next_visible {
        Some(loaded_count.saturating_add(increment))
    } else {
        None
    }
}

fn decide_platform_auto_fetch(
    state: Option<PlatformAutoFetchState>,
    current_displayed_count: usize,
    current_loaded_count: usize,
    active_filter: Option<&str>,
    has_more: bool,
    max_attempts: usize,
) -> PlatformAutoFetchDecision {
    let Some(state) = state else {
        return PlatformAutoFetchDecision::Wait;
    };
    if active_filter.is_none()
        || current_displayed_count > state.baseline_displayed_count
        || !has_more
        || state.attempts >= max_attempts
    {
        return PlatformAutoFetchDecision::Stop;
    }
    if current_loaded_count <= state.baseline_loaded_count {
        PlatformAutoFetchDecision::Wait
    } else {
        PlatformAutoFetchDecision::Fetch
    }
}

fn show_no_more_platform_message(
    active_filter: Option<&str>,
    has_more: bool,
    loading: bool,
    loading_more: bool,
) -> bool {
    active_filter.is_some() && !has_more && !loading && !loading_more
}

fn listing_matches_platform_filter(platforms: &[String], active_filter: Option<&str>) -> bool {
    match active_filter {
        Some(tag) => platforms.is_empty() || platforms.iter().any(|platform| platform == tag),
        None => true,
    }
}

fn listing_compatibility(
    platforms: &[String],
    host_tag: Option<&str>,
    active_filter: Option<&str>,
) -> PlatformCompatibility {
    if platforms.is_empty() {
        PlatformCompatibility::Compatible
    } else if let Some(active) = active_filter {
        if platforms.iter().any(|platform| platform == active) {
            PlatformCompatibility::Compatible
        } else {
            PlatformCompatibility::Incompatible
        }
    } else if let Some(host) = host_tag {
        if platforms.iter().any(|platform| platform == host) {
            PlatformCompatibility::Compatible
        } else {
            PlatformCompatibility::Incompatible
        }
    } else {
        PlatformCompatibility::Unknown
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::ListingSource;

    fn listing(id: &str, title: &str) -> GameListing {
        GameListing {
            id: id.into(),
            source: ListingSource::Nip99Listing,
            title: title.into(),
            description: "A tactical action game".into(),
            images: Vec::new(),
            download_url: String::new(),
            price: 0.0,
            currency: "SATS".into(),
            price_sats: 0,
            quantity: None,
            tags: vec!["Action".into()],
            specs: Vec::new(),
            publisher_npub: "npub1publisher".into(),
            stall_id: String::new(),
            stall_name: Some("Studio".into()),
            lud16: String::new(),
            event_id: None,
            created_at: 0,
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
    fn search_normalizes_whitespace_and_matches_user_facing_fields() {
        let mut value = listing("one", "Neon Runner");
        value.description = "Fast cyberpunk racing".into();
        value.tags = vec!["Arcade Racing".into()];
        assert_eq!(normalize_search("  NEON   racing "), "neon racing");
        assert!(matches_search(&value, &normalize_search("neon racing")));
        assert!(matches_search(
            &value,
            &normalize_search("studio cyberpunk")
        ));
        assert!(!matches_search(&value, &normalize_search("strategy")));
    }

    #[test]
    fn category_extraction_is_stable_and_excludes_technical_tags() {
        let mut first = listing("one", "One");
        first.tags = vec![
            "Action".into(),
            "NIP-99".into(),
            "linux-x86_64".into(),
            "RPG".into(),
        ];
        first.platforms = vec!["linux-x86_64".into()];
        let mut second = listing("two", "Two");
        second.tags = vec!["action".into(), "Strategy".into(), "download".into()];
        assert_eq!(
            extract_categories(&[first, second]),
            vec![
                CategoryOption {
                    key: "action".into(),
                    label: "Action".into()
                },
                CategoryOption {
                    key: "rpg".into(),
                    label: "RPG".into()
                },
                CategoryOption {
                    key: "strategy".into(),
                    label: "Strategy".into()
                },
            ]
        );
    }

    #[test]
    fn acquisition_filters_never_infer_public_from_zero_price() {
        let zero = listing("zero", "Zero");
        assert!(!matches_acquisition_filter(
            &zero,
            None,
            AcquisitionFilter::Public
        ));
        assert!(!matches_acquisition_filter(
            &zero,
            None,
            AcquisitionFilter::Paid
        ));

        let mut public = zero.clone();
        public.acquisition = AcquisitionPolicy::Public;
        assert!(matches_acquisition_filter(
            &public,
            None,
            AcquisitionFilter::Public
        ));
        assert!(!matches_acquisition_filter(
            &public,
            None,
            AcquisitionFilter::Timed
        ));

        let mut timed = zero.clone();
        timed.acquisition = AcquisitionPolicy::TimedAccess {
            starts_at: 10,
            ends_at: 20,
        };
        assert!(matches_acquisition_filter(
            &timed,
            None,
            AcquisitionFilter::Timed
        ));
        assert!(!matches_acquisition_filter(
            &timed,
            None,
            AcquisitionFilter::Public
        ));
    }

    #[test]
    fn active_campaign_and_owned_entitlement_are_distinct_filter_states() {
        let active = listing("active", "Active");
        assert!(matches_acquisition_filter(
            &active,
            Some(CampaignAvailability::Active),
            AcquisitionFilter::Claim,
        ));
        let mut owned = active.clone();
        owned.is_owned = true;
        assert!(!matches_acquisition_filter(
            &owned,
            Some(CampaignAvailability::Active),
            AcquisitionFilter::Claim,
        ));
        assert!(matches_acquisition_filter(
            &owned,
            Some(CampaignAvailability::Active),
            AcquisitionFilter::Owned,
        ));
    }

    #[test]
    fn timed_card_state_distinguishes_upcoming_active_and_expired() {
        let mut value = listing("timed", "Timed");
        value.acquisition = AcquisitionPolicy::TimedAccess {
            starts_at: 100,
            ends_at: 200,
        };
        assert_eq!(
            GameCardPresentation::from_listing(
                &value,
                99,
                PlatformCompatibility::Compatible,
                false
            )
            .access,
            crate::ui_v2::components::GameCardAccess::TimedUpcoming
        );
        assert_eq!(
            GameCardPresentation::from_listing(
                &value,
                150,
                PlatformCompatibility::Compatible,
                false
            )
            .access,
            crate::ui_v2::components::GameCardAccess::TimedActive
        );
        assert_eq!(
            GameCardPresentation::from_listing(
                &value,
                200,
                PlatformCompatibility::Compatible,
                false
            )
            .access,
            crate::ui_v2::components::GameCardAccess::TimedExpired
        );
    }

    #[test]
    fn sorting_preserves_default_order_and_handles_prices_stably() {
        let mut first = listing("first", "Zulu");
        first.created_at = 10;
        first.price_sats = 500;
        let mut second = listing("second", "Alpha");
        second.created_at = 30;
        second.price_sats = 100;
        let mut missing = listing("missing", "Missing");
        missing.created_at = 20;
        missing.price = f64::NAN;

        let original = vec![first.clone(), second.clone(), missing.clone()];
        let campaigns = std::collections::HashMap::new();
        let mut recommended = original.clone();
        sort_listings(&mut recommended, SortOption::Recommended, &campaigns, 100);
        assert_eq!(
            recommended
                .iter()
                .map(|item| item.id.as_str())
                .collect::<Vec<_>>(),
            vec!["first", "second", "missing"]
        );

        let mut newest = original.clone();
        sort_listings(&mut newest, SortOption::Newest, &campaigns, 100);
        assert_eq!(
            newest
                .iter()
                .map(|item| item.id.as_str())
                .collect::<Vec<_>>(),
            vec!["second", "missing", "first"]
        );

        let mut low = original.clone();
        sort_listings(&mut low, SortOption::PriceLow, &campaigns, 100);
        assert_eq!(
            low.iter().map(|item| item.id.as_str()).collect::<Vec<_>>(),
            vec!["second", "first", "missing"]
        );

        let mut high = original;
        sort_listings(&mut high, SortOption::PriceHigh, &campaigns, 100);
        assert_eq!(
            high.iter().map(|item| item.id.as_str()).collect::<Vec<_>>(),
            vec!["first", "second", "missing"]
        );
    }

    #[test]
    fn store_category_request_normalizes_for_browse_handoff() {
        assert_eq!(
            BrowseRequest::for_category("  Action RPG ")
                .category
                .as_deref(),
            Some("action rpg")
        );
    }

    #[test]
    fn platform_filter_and_sparse_loading_behavior_is_preserved() {
        assert!(listing_matches_platform_filter(&[], Some("linux-x86_64")));
        assert!(listing_matches_platform_filter(
            &["linux-x86_64".into()],
            Some("linux-x86_64")
        ));
        assert!(!listing_matches_platform_filter(
            &["windows-x86_64".into()],
            Some("linux-x86_64")
        ));
        assert_eq!(
            platform_load_more_fetch_limit(Some("linux-x86_64"), true, 3, 100, 60, 50),
            Some(110)
        );
        let pending = PlatformAutoFetchState {
            baseline_displayed_count: 3,
            baseline_loaded_count: 60,
            attempts: 0,
        };
        assert_eq!(
            decide_platform_auto_fetch(Some(pending), 3, 72, Some("linux-x86_64"), true, 4),
            PlatformAutoFetchDecision::Fetch
        );
    }

    #[test]
    fn browse_batches_remain_fifty_products() {
        assert_eq!(BROWSE_INITIAL_VISIBLE_COUNT, 50);
        assert_eq!(BROWSE_VISIBLE_INCREMENT, 50);
    }

    #[test]
    fn visibility_growth_and_hidden_card_detection_are_preserved() {
        assert_eq!(next_visible_count(50, 120, 50), 100);
        assert_eq!(next_visible_count(100, 120, 50), 150);
        assert!(has_more_browse_cards(12, 30));
        assert!(!has_more_browse_cards(30, 30));
    }

    #[test]
    fn backend_fetch_limits_cover_underfilled_and_exhausted_pages() {
        assert_eq!(next_fetch_limit(12, 24), Some(12));
        assert_eq!(next_fetch_limit(24, 24), None);
        assert!(can_load_more_browse_cards(72, 55, true));
        assert!(!can_load_more_browse_cards(72, 55, false));
    }

    #[test]
    fn platform_auto_fetch_without_pending_state_is_noop() {
        assert_eq!(
            decide_platform_auto_fetch(None, 0, 0, Some("linux-x86_64"), true, 4),
            PlatformAutoFetchDecision::Wait
        );
    }

    #[test]
    fn platform_auto_fetch_waits_and_honors_growth_and_attempt_guard() {
        let pending = PlatformAutoFetchState {
            baseline_displayed_count: 3,
            baseline_loaded_count: 60,
            attempts: 0,
        };
        assert_eq!(
            decide_platform_auto_fetch(Some(pending), 3, 60, Some("linux-x86_64"), true, 4),
            PlatformAutoFetchDecision::Wait
        );
        assert_eq!(
            decide_platform_auto_fetch(Some(pending), 4, 72, Some("linux-x86_64"), true, 4),
            PlatformAutoFetchDecision::Stop
        );
        assert_eq!(
            decide_platform_auto_fetch(
                Some(PlatformAutoFetchState {
                    attempts: 4,
                    ..pending
                }),
                3,
                72,
                Some("linux-x86_64"),
                true,
                4,
            ),
            PlatformAutoFetchDecision::Stop
        );
    }

    #[test]
    fn exhausted_platform_message_requires_an_active_platform_filter() {
        assert!(show_no_more_platform_message(
            Some("linux-x86_64"),
            false,
            false,
            false,
        ));
        assert!(!show_no_more_platform_message(None, false, false, false));
        assert!(!show_no_more_platform_message(
            Some("linux-x86_64"),
            true,
            false,
            false,
        ));
    }

    #[test]
    fn compatibility_is_truthful_for_restricted_and_unknown_hosts() {
        assert_eq!(
            listing_compatibility(&[], Some("linux-x86_64"), None),
            PlatformCompatibility::Compatible
        );
        assert_eq!(
            listing_compatibility(&["windows-x86_64".into()], Some("linux-x86_64"), None,),
            PlatformCompatibility::Incompatible
        );
        assert_eq!(
            listing_compatibility(&["windows-x86_64".into()], None, None),
            PlatformCompatibility::Unknown
        );
    }
}
