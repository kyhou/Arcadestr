//! Browse-all games page using live marketplace listings with template-parity layout.

use leptos::prelude::*;

use crate::models::GameListing;
use crate::ui_v2::views::marketplace_loader::{
    listing_presentation, listing_publisher, use_marketplace_listings,
};

const FALLBACK_COVER: &str = "https://lh3.googleusercontent.com/aida-public/AB6AXuDcG9Zo3aR9Vrpk5pP2jenw1AoVFoOzbAQ-t57kQtlbwGQVsLLwmHyFuyzRVsOh71iN4mHyhfw0Sx4YgdJ9duL9ANv3Xa1W7jYKWeVgj5_rE7KzitErwV3dtgEFGsGCSXtFQxyw6tQoGmP3V-Ci9Vs9_ZQXh6WXrFi6eperEaPm3YutXUIImUuC5sKm2hgyVb6sMBnpn0Imy94ETrJ9WO2XeC6tTMddB6EA-x1LgnN3Ezj_dPitegkcYmXGBSWZyCTZgxINu01kmdM";

#[component]
pub fn BrowseGamesView(on_select: Callback<GameListing>) -> impl IntoView {
    let marketplace = use_marketplace_listings();
    let listings = marketplace.listings;
    let loading = marketplace.loading;
    let error = marketplace.error;
    let received_count = marketplace.received_count;

    let featured_listing = Signal::derive(move || listings.get().first().cloned());

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
                    <div class="flex items-center gap-4 bg-surface-container-low p-1 rounded-lg">
                        <button class="px-4 py-2 rounded-md bg-surface-bright text-on-surface text-sm font-medium shadow-sm">"Popularity"</button>
                        <button class="px-4 py-2 rounded-md text-on-surface-variant text-sm font-medium hover:text-on-surface transition-colors">"Newest"</button>
                        <button class="px-4 py-2 rounded-md text-on-surface-variant text-sm font-medium hover:text-on-surface transition-colors">"Price"</button>
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
                                let all = listings.get();
                                let cards = all.iter().skip(1).cloned().collect::<Vec<_>>();

                                cards
                                    .into_iter()
                                    .enumerate()
                                    .flat_map(|(idx, listing)| {
                                        let card = render_listing_card(listing, on_select);

                                        if idx == 3 {
                                            if let Some(featured) = featured_listing.get() {
                                                vec![card, render_featured_card(featured, on_select)]
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
                                let all = listings.get();
                                if all.len() <= 4 {
                                    featured_listing
                                        .get()
                                        .map(|featured| render_featured_card(featured, on_select))
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

            <div class="mt-16 flex justify-center">
                <button class="px-10 py-4 bg-surface-container-low border border-outline-variant/15 text-on-surface-variant font-bold rounded-full hover:bg-surface-container-high hover:text-on-surface transition-all active:scale-95 flex items-center gap-3">
                    <span class="material-symbols-outlined">"expand_more"</span>
                    "Load More Decentralized Experiences"
                </button>
            </div>
        </section>
    }
}

fn render_listing_card(listing: GameListing, on_select: Callback<GameListing>) -> AnyView {
    let selected = listing.clone();
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
        <article class="group bg-surface-container-high rounded-xl overflow-hidden hover:scale-[1.02] hover:bg-surface-bright transition-[transform,background-color] duration-300 ease-out motion-safe:will-change-transform">
            <div class="relative aspect-[16/10] overflow-hidden bg-surface-high">
                <img alt={listing.title.clone()} class="w-full h-full object-cover group-hover:scale-110 transition-transform duration-700 will-change-transform transform-gpu backface-hidden antialiased" src={image_url} />
                <div class="absolute inset-0 bg-gradient-to-t from-black/80 via-transparent to-transparent"></div>
                <div class="absolute bottom-4 left-4 flex items-center gap-2">
                    <span class="px-2 py-0.5 bg-tertiary-container/20 backdrop-blur-md border border-tertiary/30 rounded-sm text-[10px] font-bold text-tertiary uppercase tracking-wider">"⚡ LIVE"</span>
                </div>
            </div>
            <div class="p-5">
                <div class="flex justify-between items-start mb-1 gap-3">
                    <h3 class="font-headline text-xl font-bold text-on-surface leading-tight">{listing.title.clone()}</h3>
                    <div class="flex flex-col items-end">
                        <span class={if presentation.is_free { "text-secondary font-bold font-headline" } else { "text-primary font-bold font-headline" }}>{presentation.price_primary}</span>
                        {presentation.price_hint.clone().map(|hint| {
                            view! { <span class="text-[10px] text-on-surface-variant font-medium">{hint}</span> }
                        })}
                    </div>
                </div>
                <p class="text-on-surface-variant text-xs mb-6 font-medium">{listing_publisher(&listing)}</p>
                <div class="flex items-center justify-between gap-3">
                    <div class="flex flex-col">
                        <span class="text-[10px] text-on-surface-variant uppercase font-bold tracking-widest">{meta.split_whitespace().next().unwrap_or("TYPE").to_string()}</span>
                        <span class="text-xs text-on-surface">{meta.split_once(' ').map(|(_, v)| v.to_string()).unwrap_or_else(|| "Arcadestr".to_string())}</span>
                    </div>
                    <button
                        class={if presentation.is_free {
                            "bg-secondary text-on-secondary font-bold px-6 py-2.5 rounded-md text-sm hover:brightness-110 transition-all active:scale-95 shadow-lg shadow-secondary/10"
                        } else {
                            "bg-gradient-to-r from-primary to-primary-dim text-on-primary font-bold px-6 py-2.5 rounded-md text-sm hover:brightness-110 transition-all active:scale-95 shadow-lg shadow-primary/10"
                        }}
                        on:click=move |_| on_select.run(selected.clone())
                    >
                        {presentation.cta_label}
                    </button>
                </div>
            </div>
        </article>
    }
    .into_any()
}

fn render_featured_card(listing: GameListing, on_select: Callback<GameListing>) -> AnyView {
    let selected = listing.clone();
    let image_url = listing
        .images
        .first()
        .cloned()
        .unwrap_or_else(|| FALLBACK_COVER.to_string());
    let presentation = listing_presentation(&listing);

    view! {
        <article class="md:col-span-2 group bg-surface-container-high rounded-xl overflow-hidden hover:bg-surface-bright transition-[background-color] duration-300 ease-out motion-safe:will-change-transform relative">
            <div class="flex flex-col lg:flex-row h-full">
                <div class="lg:w-3/5 relative overflow-hidden aspect-[16/9] lg:aspect-auto bg-surface-high">
                    <img alt={listing.title.clone()} class="w-full h-full object-cover group-hover:scale-105 transition-transform duration-1000 will-change-transform transform-gpu backface-hidden antialiased" src={image_url} />
                    <div class="absolute inset-0 bg-gradient-to-r from-transparent via-black/20 to-surface-container-high hidden lg:block"></div>
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
                        <button class="bg-gradient-to-r from-primary to-primary-dim text-on-primary font-bold px-8 py-3 rounded-md text-base hover:brightness-110 transition-all active:scale-95 shadow-xl shadow-primary/20" on:click=move |_| on_select.run(selected.clone())>
                            "Buy Key"
                        </button>
                    </div>
                </div>
            </div>
        </article>
    }
    .into_any()
}
