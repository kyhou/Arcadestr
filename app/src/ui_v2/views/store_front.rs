use std::collections::HashMap;

use leptos::prelude::*;

use crate::models::{AcquisitionPolicy, GameListing, StorePageCardPresentation};
use crate::ui_v2::components::{
    GameCard, GameCardAction, GameCardCampaign, GameCardPresentation, PageHeader,
    PlatformCompatibility,
};
use crate::ui_v2::views::browse_games::{extract_categories, listing_categories, BrowseRequest};
use crate::ui_v2::views::marketplace_loader::{
    canonical_listing_coordinate, listing_state_key, use_listing_campaign_states,
    use_listing_store_page_presentations, use_marketplace_listings_with_limit,
    CampaignAvailability,
};
use crate::ui_v2::views::{use_fallback_cover, valid_cover_url, FALLBACK_COVER};

const STORE_FRONT_LISTING_LIMIT: usize = 24;
const STORE_CARD_LIMIT: usize = 6;
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
    let featured_listing = Signal::derive(move || select_featured_listing(&listings.get()));
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

    view! {
        <section class="space-y-10 p-4 md:p-8">
            <PageHeader
                eyebrow="Decentralized marketplace".to_string()
                title="Discover games on Nostr".to_string()
                description="Marketplace listings stream from your connected relays. Purchases and claims remain tied to your active identity.".to_string()
            />

            <Show when=move || marketplace.refreshing.get() && !listings.get().is_empty()>
                <p class="rounded-xl border border-secondary/20 bg-secondary/10 px-4 py-3 text-sm text-secondary" role="status">
                    "Showing cached listings while refreshing from relays."
                </p>
            </Show>
            <Show when=move || marketplace.error.get().is_some() && !listings.get().is_empty()>
                <p class="rounded-xl border border-tertiary/25 bg-tertiary/10 px-4 py-3 text-sm text-tertiary" role="status">
                    {move || format!("Relay refresh failed; cached listings remain available. {}", marketplace.error.get().unwrap_or_default())}
                </p>
            </Show>
            <Show when=move || campaign_state.error.get().is_some() && !listings.get().is_empty()>
                <p class="text-xs text-tertiary" role="status">"Some claim campaign statuses are currently unavailable."</p>
            </Show>

            {move || match featured_listing.get() {
                Some(listing) => {
                    let store_page = canonical_listing_coordinate(&listing)
                        .and_then(|coordinate| store_pages.presentations.get().get(&coordinate).cloned());
                    let image_url = store_page.as_ref()
                        .and_then(|page| page.hero_url.clone().or(page.capsule_url.clone()))
                        .or_else(|| valid_cover_url(&listing.images))
                        .unwrap_or_else(|| FALLBACK_COVER.to_string());
                    let title = store_page.as_ref().and_then(|page| page.title.clone())
                        .unwrap_or_else(|| listing.title.clone());
                    let summary = store_page.as_ref().and_then(|page| page.summary.clone())
                        .unwrap_or_else(|| listing.description.clone());
                    let selected = listing.clone();
                    let access_label = featured_access_label(
                        &listing,
                        campaign_state.states.get().get(&listing_state_key(&listing)).copied(),
                        current_unix_secs(),
                    );
                    view! {
                        <article class="relative min-h-[28rem] overflow-hidden rounded-3xl bg-surface-container-high md:min-h-[32rem]">
                            <img
                                src=image_url
                                alt={format!("{} cover art", title)}
                                class="absolute inset-0 h-full w-full object-cover"
                                on:error=use_fallback_cover
                            />
                            <div class="absolute inset-0 bg-gradient-to-t from-background via-background/45 to-transparent" aria-hidden="true"></div>
                            <div class="relative flex min-h-[28rem] flex-col justify-end p-6 md:min-h-[32rem] md:p-12">
                                <span class="mb-4 w-fit rounded-full bg-secondary/15 px-3 py-1 text-xs font-semibold uppercase tracking-[0.2em] text-secondary backdrop-blur">
                                    {format!("Featured · {access_label}")}
                                </span>
                                <h2 class="max-w-4xl font-display text-4xl font-bold leading-[0.95] text-on-surface md:text-6xl">
                                    {title}
                                </h2>
                                <p class="mt-4 max-w-2xl text-sm leading-relaxed text-on-surface-variant md:text-lg">
                                    {summary}
                                </p>
                                <button
                                    type="button"
                                    class="mt-6 w-fit rounded-xl bg-primary px-6 py-3 text-sm font-semibold text-on-primary outline-none ring-primary/60 hover:brightness-110 focus-visible:ring-2"
                                    on:click=move |_| on_select.run(selected.clone())
                                >
                                    "View details"
                                </button>
                            </div>
                        </article>
                    }
                    .into_any()
                }
                None if marketplace.loading.get() => view! {
                    <div class="flex min-h-80 items-center justify-center rounded-3xl bg-surface-container-high p-8 text-center text-on-surface-variant" role="status">
                        {if marketplace.received_count.get() > 0 {
                            format!("Loading marketplace... {} listings received", marketplace.received_count.get())
                        } else {
                            "Loading marketplace from connected relays...".to_string()
                        }}
                    </div>
                }
                .into_any(),
                None if marketplace.error.get().is_some() => view! {
                    <div class="rounded-3xl border border-error/30 bg-error/10 p-10 text-center">
                        <h2 class="font-display text-xl font-semibold">"Marketplace unavailable"</h2>
                        <p class="mt-2 text-sm text-error">{marketplace.error.get().unwrap_or_default()}</p>
                    </div>
                }
                .into_any(),
                None => view! {
                    <div class="rounded-3xl bg-surface-container-high p-10 text-center">
                        <h2 class="font-display text-xl font-semibold">"No marketplace listings"</h2>
                        <p class="mt-2 text-sm text-on-surface-variant">"No valid game listings with usable cover art are currently available from connected relays."</p>
                    </div>
                }
                .into_any(),
            }}

            {move || promotion.get().map(|promotion| {
                let selected = promotion.listing.clone();
                view! {
                    <button
                        type="button"
                        class="flex w-full items-center gap-4 rounded-2xl bg-surface-container-high p-5 text-left outline-none ring-primary/60 hover:bg-surface-bright focus-visible:ring-2"
                        on:click=move |_| on_select.run(selected.clone())
                    >
                        <span class="flex h-12 w-12 shrink-0 items-center justify-center rounded-xl bg-tertiary/15 text-tertiary">
                            <span class="material-symbols-outlined" aria-hidden="true">{promotion.icon}</span>
                        </span>
                        <span class="min-w-0">
                            <span class="block text-xs font-semibold uppercase tracking-widest text-tertiary">{promotion.label}</span>
                            <span class="block truncate font-display text-lg font-semibold">{promotion.listing.title}</span>
                            <span class="mt-1 block text-sm text-on-surface-variant">{promotion.description}</span>
                        </span>
                    </button>
                }
            })}

            <Show when=move || !categories.get().is_empty()>
                <section aria-labelledby="store-categories-title">
                    <div class="mb-4 flex items-center justify-between gap-3">
                        <h2 id="store-categories-title" class="font-display text-2xl font-bold">"Browse by category"</h2>
                        <button
                            type="button"
                            class="text-sm font-semibold text-primary outline-none ring-primary/60 hover:underline focus-visible:ring-2"
                            on:click=move |_| on_browse.run(BrowseRequest::default())
                        >"All games"</button>
                    </div>
                    <div class="grid grid-cols-2 gap-3 lg:grid-cols-4">
                        {move || categories.get().into_iter().enumerate().map(|(index, category)| {
                            let request = BrowseRequest::for_category(category.key.clone());
                            let tint = match index % 4 {
                                0 => "bg-primary/15 hover:bg-primary/25",
                                1 => "bg-secondary/15 hover:bg-secondary/25",
                                2 => "bg-tertiary/15 hover:bg-tertiary/25",
                                _ => "bg-surface-container-high hover:bg-surface-bright",
                            };
                            view! {
                                <button
                                    type="button"
                                    class=format!("min-h-24 rounded-2xl px-4 py-6 text-center font-display text-lg font-bold outline-none ring-primary/60 transition-colors focus-visible:ring-2 {tint}")
                                    on:click=move |_| on_browse.run(request.clone())
                                >{category.label}</button>
                            }
                        }).collect_view()}
                    </div>
                </section>
            </Show>

            <Show when=move || !listings.get().is_empty()>
                <section aria-labelledby="store-listings-title">
                    <div class="mb-4 flex items-center justify-between gap-3">
                        <div>
                            <h2 id="store-listings-title" class="font-display text-3xl font-bold">"Latest listings"</h2>
                            <p class="mt-1 text-sm text-on-surface-variant">"Newest marketplace events currently loaded from relays."</p>
                        </div>
                        <button
                            type="button"
                            class="text-sm font-semibold text-primary outline-none ring-primary/60 hover:underline focus-visible:ring-2"
                            on:click=move |_| on_browse.run(BrowseRequest::default())
                        >"View all"</button>
                    </div>
                    <div class="grid grid-cols-1 gap-5 sm:grid-cols-2 xl:grid-cols-3">
                        {move || {
                            let featured_key = featured_listing.get().map(|listing| listing_state_key(&listing));
                            let campaign_states = campaign_state.states.get();
                            let presentations = store_pages.presentations.get();
                            listings.get().into_iter()
                                .filter(|listing| Some(listing_state_key(listing)) != featured_key)
                                .take(STORE_CARD_LIMIT)
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

fn select_featured_listing(listings: &[GameListing]) -> Option<GameListing> {
    listings
        .iter()
        .find(|listing| valid_cover_url(&listing.images).is_some())
        .cloned()
}

fn featured_access_label(
    listing: &GameListing,
    campaign: Option<CampaignAvailability>,
    now: u64,
) -> &'static str {
    if listing.is_owned {
        "Owned"
    } else if campaign == Some(CampaignAvailability::Active) {
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
            select_featured_listing(&listings)
                .map(|item| item.id)
                .as_deref(),
            Some("newest-covered")
        );
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
