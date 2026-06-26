//! Browse-all games page using live marketplace listings with template-parity layout.

use leptos::prelude::*;
use leptos::task::spawn_local;

use crate::models::{GameListing, PlatformInfo};
use crate::tauri_bridge::{invoke_get_platform_info, invoke_install_game};
use crate::ui_v2::views::marketplace_loader::{
    listing_presentation, listing_publisher, use_marketplace_listings,
};

const FALLBACK_COVER: &str = "https://lh3.googleusercontent.com/aida-public/AB6AXuDcG9Zo3aR9Vrpk5pP2jenw1AoVFoOzbAQ-t57kQtlbwGQVsLLwmHyFuyzRVsOh71iN4mHyhfw0Sx4YgdJ9duL9ANv3Xa1W7jYKWeVgj5_rE7KzitErwV3dtgEFGsGCSXtFQxyw6tQoGmP3V-Ci9Vs9_ZQXh6WXrFi6eperEaPm3YutXUIImUuC5sKm2hgyVb6sMBnpn0Imy94ETrJ9WO2XeC6tTMddB6EA-x1LgnN3Ezj_dPitegkcYmXGBSWZyCTZgxINu01kmdM";
const BROWSE_INITIAL_VISIBLE_COUNT: usize = 12;
const BROWSE_VISIBLE_INCREMENT: usize = 12;

#[component]
pub fn BrowseGamesView(on_select: Callback<GameListing>) -> impl IntoView {
    let marketplace = use_marketplace_listings();
    let listings = marketplace.listings;
    let loading = marketplace.loading;
    let loading_more = marketplace.loading_more;
    let error = marketplace.error;
    let received_count = marketplace.received_count;
    let requested_limit = marketplace.requested_limit;
    let has_more = marketplace.has_more;
    let visible_count = RwSignal::new(BROWSE_INITIAL_VISIBLE_COUNT);
    let platform_info = RwSignal::new(None::<PlatformInfo>);
    let active_platform_filter = RwSignal::new(None::<String>);
    let platform_filter_initialized = RwSignal::new(false);

    Effect::new(move |_| {
        spawn_local(async move {
            if let Ok(info) = invoke_get_platform_info().await {
                platform_info.set(Some(info));
            }
        });
    });

    Effect::new(move |_| {
        if platform_filter_initialized.get_untracked() {
            return;
        }

        if let Some(info) = platform_info.get() {
            active_platform_filter.set(Some(info.tag()));
            platform_filter_initialized.set(true);
        }
    });

    let displayed_listings = Signal::derive(move || {
        let filter = active_platform_filter.get();
        listings
            .get()
            .into_iter()
            .filter(|listing| {
                listing_matches_platform_filter(&listing.platforms, filter.as_deref())
            })
            .collect::<Vec<_>>()
    });

    let featured_listing = Signal::derive(move || displayed_listings.get().first().cloned());
    let host_platform_tag = Signal::derive(move || platform_info.get().map(|info| info.tag()));

    view! {
        <section class="max-w-[1600px] mx-auto p-6 lg:p-10">
            <header class="mb-10">
                <div class="flex flex-col md:flex-row md:items-end justify-between gap-6">
                    <div>
                        <h1 class="font-headline text-5xl font-bold tracking-tighter mb-4 text-on-surface">"Browse All Games"</h1>
                        <p class="text-on-surface-variant max-w-xl text-lg leading-relaxed">
                            "Discover the next generation of decentralized gaming. Hand-curated experiences powered by Nostr and Bitcoin."
                        </p>
                    </div>
                    <div class="flex flex-col sm:flex-row sm:items-center gap-3">
                        <label class="flex items-center gap-2 bg-surface-container-low px-3 py-2 rounded-lg text-sm font-medium text-on-surface-variant">
                            <span>"Platform"</span>
                            <select
                                class="bg-surface-bright text-on-surface rounded-md px-3 py-2 text-sm border border-outline-variant/20 focus:outline-none focus:ring-2 focus:ring-primary/30"
                                prop:value=move || {
                                    if active_platform_filter.get().is_some() { "host" } else { "all" }
                                }
                                on:change=move |ev| {
                                    platform_filter_initialized.set(true);
                                    match event_target_value(&ev).as_str() {
                                        "host" => {
                                            if let Some(tag) = host_platform_tag.get_untracked() {
                                                active_platform_filter.set(Some(tag));
                                            }
                                        }
                                        _ => active_platform_filter.set(None),
                                    }
                                }
                            >
                                <option value="all">"All Platforms"</option>
                                {move || {
                                    host_platform_tag
                                        .get()
                                        .map(|tag| {
                                            view! { <option value="host">{format!("My Platform ({})", tag)}</option> }.into_any()
                                        })
                                        .unwrap_or_else(|| {
                                            view! { <option value="host" disabled=true>"Detecting Platform..."</option> }.into_any()
                                        })
                                }}
                            </select>
                        </label>
                        <div class="flex items-center gap-4 bg-surface-container-low p-1 rounded-lg">
                            <button class="px-4 py-2 rounded-md bg-surface-bright text-on-surface text-sm font-medium shadow-sm">"Popularity"</button>
                            <button class="px-4 py-2 rounded-md text-on-surface-variant text-sm font-medium hover:text-on-surface transition-colors">"Newest"</button>
                            <button class="px-4 py-2 rounded-md text-on-surface-variant text-sm font-medium hover:text-on-surface transition-colors">"Price"</button>
                        </div>
                    </div>
                </div>
            </header>

            {move || {
                if loading.get() {
                    view! {
                        <div class="bg-surface-container-high rounded-xl p-6 text-on-surface-variant">
                            {move || {
                                let count = received_count.get();
                                if count > 0 {
                                    format!("Loading... {} products found", count)
                                } else {
                                    "Fetching listings from relays...".to_string()
                                }
                            }}
                        </div>
                    }
                    .into_any()
                } else if let Some(fetch_error) = error.get() {
                    view! {
                        <div class="bg-error-container/30 border border-error/30 rounded-xl p-6 text-error">
                            {format!("Error: {}", fetch_error)}
                        </div>
                    }
                    .into_any()
                } else {
                    view! {
                        <div class="grid grid-cols-1 md:grid-cols-2 xl:grid-cols-3 2xl:grid-cols-4 gap-8">
                            {move || {
                                let all = displayed_listings.get();
                                let host_tag = host_platform_tag.get();
                                let active_filter = active_platform_filter.get();
                                let cards = all
                                    .iter()
                                    .skip(1)
                                    .take(visible_count.get())
                                    .cloned()
                                    .collect::<Vec<_>>();

                                cards
                                    .into_iter()
                                    .enumerate()
                                    .flat_map(|(idx, listing)| {
                                        let card = render_listing_card(
                                            listing,
                                            on_select,
                                            host_tag.clone(),
                                            active_filter.clone(),
                                        );

                                        if idx == 3 {
                                            if let Some(featured) = featured_listing.get() {
                                                vec![
                                                    card,
                                                    render_featured_card(
                                                        featured,
                                                        on_select,
                                                        host_tag.clone(),
                                                        active_filter.clone(),
                                                    ),
                                                ]
                                            } else {
                                                vec![card]
                                            }
                                        } else {
                                            vec![card]
                                        }
                                    })
                                    .collect::<Vec<_>>()
                            }}

                            {move || {
                                let all = displayed_listings.get();
                                let host_tag = host_platform_tag.get();
                                let active_filter = active_platform_filter.get();
                                if all.len() <= 4 {
                                    featured_listing
                                        .get()
                                        .map(|featured| {
                                            render_featured_card(
                                                featured,
                                                on_select,
                                                host_tag,
                                                active_filter,
                                            )
                                        })
                                        .into_iter()
                                        .collect::<Vec<_>>()
                                } else {
                                    Vec::new()
                                }
                            }}
                        </div>
                    }
                    .into_any()
                }
            }}

            {move || {
                let total_cards = displayed_listings.get().len().saturating_sub(1);
                if !loading.get()
                    && !loading_more.get()
                    && can_load_more_browse_cards(visible_count.get(), total_cards, has_more.get())
                {
                    view! {
                        <div class="mt-16 flex justify-center">
                            <button
                                class="px-10 py-4 bg-surface-container-low border border-outline-variant/15 text-on-surface-variant font-bold rounded-full hover:bg-surface-container-high hover:text-on-surface transition-all active:scale-95 flex items-center gap-3"
                                on:click=move |_| {
                                    let total_cards = listings.get_untracked().len().saturating_sub(1);
                                    let next_visible = next_visible_count(
                                        visible_count.get_untracked(),
                                        total_cards,
                                        BROWSE_VISIBLE_INCREMENT,
                                    );
                                    visible_count.set(next_visible);
                                    let required_limit = required_listing_limit(next_visible);
                                    if next_fetch_limit(listings.get_untracked().len(), required_limit).is_some() {
                                        requested_limit.update(|limit| {
                                            *limit = (*limit).max(required_limit);
                                        });
                                    }
                                }
                            >
                                <span class="material-symbols-outlined">"expand_more"</span>
                                "Load More..."
                            </button>
                        </div>
                    }
                    .into_any()
                } else if loading_more.get() {
                    view! {
                        <div class="mt-16 flex justify-center">
                            <div class="px-8 py-4 bg-surface-container-low border border-outline-variant/15 text-on-surface-variant font-bold rounded-full flex items-center gap-3">
                                <span class="material-symbols-outlined animate-spin">"progress_activity"</span>
                                "Loading more products..."
                            </div>
                        </div>
                    }
                    .into_any()
                } else if !loading.get() && total_cards > 0 && !has_more.get() {
                    view! {
                        <div class="mt-16 flex justify-center">
                            <div class="max-w-md rounded-xl border border-outline-variant/15 bg-surface-container-low px-6 py-4 text-center text-on-surface-variant">
                                <p class="font-bold text-on-surface">"No more products to load"</p>
                                <p class="mt-1 text-sm">"You've reached the end of the currently available marketplace listings."</p>
                            </div>
                        </div>
                    }
                    .into_any()
                } else {
                    view! { <></> }.into_any()
                }
            }}
        </section>
    }
}

fn next_visible_count(current: usize, total: usize, increment: usize) -> usize {
    let _ = total;
    current.saturating_add(increment)
}

fn has_more_browse_cards(visible: usize, total: usize) -> bool {
    visible < total
}

fn can_load_more_browse_cards(visible: usize, total_cards: usize, has_more: bool) -> bool {
    has_more_browse_cards(visible, total_cards) || has_more
}

fn required_listing_limit(visible_cards: usize) -> usize {
    visible_cards.saturating_add(1)
}

fn next_fetch_limit(loaded_count: usize, requested_limit: usize) -> Option<usize> {
    requested_limit
        .checked_sub(loaded_count)
        .filter(|limit| *limit > 0)
}

fn listing_matches_platform_filter(platforms: &[String], active_filter: Option<&str>) -> bool {
    match active_filter {
        Some(tag) => platforms.is_empty() || platforms.iter().any(|platform| platform == tag),
        None => true,
    }
}

fn is_incompatible_listing(
    platforms: &[String],
    host_tag: Option<&str>,
    active_filter: Option<&str>,
) -> bool {
    active_filter.is_none()
        && host_tag.is_some_and(|tag| {
            !platforms.is_empty() && !platforms.iter().any(|platform| platform == tag)
        })
}

fn render_status_badges(
    listing: &GameListing,
    host_tag: Option<&str>,
    active_filter: Option<&str>,
) -> AnyView {
    let verified = listing.nip94_event_id.is_some();
    let incompatible = is_incompatible_listing(&listing.platforms, host_tag, active_filter);

    view! {
        <div class="absolute bottom-3 left-3 flex flex-wrap items-center gap-2">
            <span class="px-2 py-0.5 bg-tertiary-container/20 backdrop-blur-md border border-tertiary/30 rounded-sm text-[10px] font-bold text-tertiary uppercase tracking-wider">"⚡ LIVE"</span>
            {verified.then(|| {
                view! { <span class="px-2 py-0.5 bg-secondary/20 backdrop-blur-md border border-secondary/30 rounded-sm text-[10px] font-bold text-secondary uppercase tracking-wider">"Verified Delivery"</span> }
            })}
            {incompatible.then(|| {
                view! { <span class="px-2 py-0.5 bg-error-container/40 backdrop-blur-md border border-error/40 rounded-sm text-[10px] font-bold text-error uppercase tracking-wider">"Incompatible"</span> }
            })}
        </div>
    }
    .into_any()
}

fn render_listing_card(
    listing: GameListing,
    on_select: Callback<GameListing>,
    host_tag: Option<String>,
    active_filter: Option<String>,
) -> AnyView {
    let selected = listing.clone();
    let install_listing = listing.clone();
    let presentation = listing_presentation(&listing);
    let image_url = listing
        .images
        .first()
        .cloned()
        .unwrap_or_else(|| FALLBACK_COVER.to_string());
    let meta = listing
        .specs
        .first()
        .map(|(key, value)| format!("{} {}", key.to_uppercase(), value))
        .unwrap_or_else(|| "OWNERSHIP Digital License".to_string());

    view! {
        <article class="group relative flex flex-col bg-surface-container-high rounded-xl overflow-hidden transition-transform duration-300 hover:scale-[1.02] [transform:translateZ(0)] [backface-visibility:hidden]">
            <div class="relative aspect-[16/10] w-full overflow-hidden">
                <img alt={listing.title.clone()} class="w-full h-full object-cover" src={image_url} />
                <div class="absolute inset-0 bg-gradient-to-t from-black/60 via-transparent to-transparent"></div>
                {render_status_badges(&listing, host_tag.as_deref(), active_filter.as_deref())}
            </div>
            <div class="p-5 flex flex-col gap-4">
                <div class="flex justify-between items-start">
                    <div class="flex flex-col">
                        <h3 class="font-headline text-xl font-bold text-on-surface leading-tight">{listing.title.clone()}</h3>
                        <p class="text-on-surface-variant text-sm">{listing_publisher(&listing)}</p>
                    </div>
                    <div class="text-right">
                        <p class={if presentation.is_free { "text-secondary font-bold font-headline" } else { "text-primary font-bold font-headline" }}>{presentation.price_primary}</p>
                        {presentation.price_hint.clone().map(|hint| {
                            view! { <p class="text-on-surface-variant text-[10px]">{hint}</p> }
                        })}
                    </div>
                </div>
                <div class="flex justify-between items-end">
                    <div class="flex flex-col">
                        <span class="text-[10px] text-on-surface-variant uppercase font-bold tracking-widest">{meta.split_whitespace().next().unwrap_or("TYPE").to_string()}</span>
                        <span class="text-xs text-on-surface">{meta.split_once(' ').map(|(_, v)| v.to_string()).unwrap_or_else(|| "Arcadestr".to_string())}</span>
                    </div>
                    {if listing.is_owned {
                        view! {
                            <button
                                class="bg-secondary text-on-secondary font-bold py-2 px-6 rounded-lg text-sm hover:brightness-110 transition-all active:scale-95 shadow-lg shadow-secondary/10"
                                on:click=move |_| {
                                    let listing = install_listing.clone();
                                    spawn_local(async move {
                                        let _ = invoke_install_game(&listing).await;
                                    });
                                }
                            >
                                "Install"
                            </button>
                        }
                        .into_any()
                    } else {
                        view! {
                            <button
                                class={if presentation.is_free {
                                    "bg-secondary text-on-secondary font-bold py-2 px-6 rounded-lg text-sm hover:brightness-110 transition-all active:scale-95 shadow-lg shadow-secondary/10"
                                } else {
                                    "bg-gradient-to-r from-primary to-primary-dim text-on-primary font-bold py-2 px-6 rounded-lg text-sm hover:brightness-110 transition-all active:scale-95 shadow-lg shadow-primary/10"
                                }}
                                on:click=move |_| on_select.run(selected.clone())
                            >
                                {presentation.cta_label}
                            </button>
                        }
                        .into_any()
                    }}
                </div>
            </div>
        </article>
    }
    .into_any()
}

fn render_featured_card(
    listing: GameListing,
    on_select: Callback<GameListing>,
    host_tag: Option<String>,
    active_filter: Option<String>,
) -> AnyView {
    let selected = listing.clone();
    let install_listing = listing.clone();
    let image_url = listing
        .images
        .first()
        .cloned()
        .unwrap_or_else(|| FALLBACK_COVER.to_string());
    let presentation = listing_presentation(&listing);

    view! {
        <article class="md:col-span-2 group relative rounded-xl bg-surface-container-high overflow-hidden transition-transform duration-300 hover:scale-[1.02] [transform:translateZ(0)] [backface-visibility:hidden]">
            <div class="flex flex-col lg:flex-row h-full">
                <div class="lg:w-3/5 relative aspect-[16/9] lg:aspect-auto bg-surface-high">
                    <img alt={listing.title.clone()} class="w-full h-full object-cover" src={image_url} />
                    <div class="absolute inset-0 bg-gradient-to-r from-transparent via-black/20 to-surface-container-high hidden lg:block"></div>
                    {render_status_badges(&listing, host_tag.as_deref(), active_filter.as_deref())}
                </div>
                <div class="lg:w-2/5 p-8 flex flex-col justify-center">
                    <div class="inline-flex items-center gap-2 px-3 py-1 bg-secondary/10 rounded-full w-fit mb-4 border border-secondary/20">
                        <span class="w-2 h-2 rounded-full bg-secondary animate-pulse"></span>
                        <span class="text-[10px] font-bold text-secondary tracking-widest uppercase">"Editor's Choice"</span>
                    </div>
                    <h3 class="font-headline text-3xl font-bold text-on-surface mb-3 tracking-tight">{listing.title.clone()}</h3>
                    <p class="text-on-surface-variant text-sm mb-8 leading-relaxed line-clamp-4">{listing.description.clone()}</p>
                    <div class="mt-auto flex items-center justify-between gap-4">
                        <div class="flex flex-col">
                            <span class="text-primary font-bold font-headline text-2xl">{presentation.price_primary}</span>
                            <span class="text-xs text-on-surface-variant">"Access Perpetual Key"</span>
                        </div>
                        {if listing.is_owned {
                            view! {
                                <button
                                    class="bg-secondary text-on-secondary font-bold px-8 py-3 rounded-md text-base hover:brightness-110 transition-all active:scale-95 shadow-xl shadow-secondary/20"
                                    on:click=move |_| {
                                        let listing = install_listing.clone();
                                        spawn_local(async move {
                                            let _ = invoke_install_game(&listing).await;
                                        });
                                    }
                                >
                                    "Install"
                                </button>
                            }
                            .into_any()
                        } else {
                            view! {
                                <button class="bg-gradient-to-r from-primary to-primary-dim text-on-primary font-bold px-8 py-3 rounded-md text-base hover:brightness-110 transition-all active:scale-95 shadow-xl shadow-primary/20" on:click=move |_| on_select.run(selected.clone())>
                                    "Buy Key"
                                </button>
                            }
                            .into_any()
                        }}
                    </div>
                </div>
            </div>
        </article>
    }
    .into_any()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_more_increases_desired_visible_count_beyond_loaded_total() {
        assert_eq!(next_visible_count(12, 30, 12), 24);
        assert_eq!(next_visible_count(24, 30, 12), 36);
        assert_eq!(next_visible_count(36, 30, 12), 48);
    }

    #[test]
    fn has_more_browse_cards_detects_hidden_items() {
        assert!(has_more_browse_cards(12, 30));
        assert!(!has_more_browse_cards(30, 30));
        assert!(!has_more_browse_cards(31, 30));
    }

    #[test]
    fn next_fetch_limit_only_requests_missing_products() {
        assert_eq!(next_fetch_limit(12, 24), Some(12));
        assert_eq!(next_fetch_limit(24, 24), None);
        assert_eq!(next_fetch_limit(30, 24), None);
    }

    #[test]
    fn load_more_remains_available_after_underfilled_page() {
        assert!(can_load_more_browse_cards(72, 55, true));
    }

    #[test]
    fn load_more_disappears_when_no_hidden_cards_and_exhausted() {
        assert!(!can_load_more_browse_cards(72, 55, false));
    }

    #[test]
    fn platform_filter_includes_unrestricted_and_matching_listings() {
        let unrestricted = Vec::new();
        let linux_platforms = vec!["linux-x86_64".to_string()];
        let windows_platforms = vec!["windows-x86_64".to_string()];

        assert!(listing_matches_platform_filter(
            &unrestricted,
            Some("linux-x86_64")
        ));
        assert!(listing_matches_platform_filter(
            &linux_platforms,
            Some("linux-x86_64")
        ));
        assert!(!listing_matches_platform_filter(
            &windows_platforms,
            Some("linux-x86_64")
        ));
        assert!(listing_matches_platform_filter(&windows_platforms, None));
    }

    #[test]
    fn incompatible_badge_only_applies_in_all_platforms_view() {
        let windows_platforms = vec!["windows-x86_64".to_string()];
        let unrestricted = Vec::new();

        assert!(is_incompatible_listing(
            &windows_platforms,
            Some("linux-x86_64"),
            None
        ));
        assert!(!is_incompatible_listing(
            &windows_platforms,
            Some("linux-x86_64"),
            Some("linux-x86_64")
        ));
        assert!(!is_incompatible_listing(
            &unrestricted,
            Some("linux-x86_64"),
            None
        ));
    }
}
