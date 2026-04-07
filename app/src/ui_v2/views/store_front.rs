use leptos::prelude::*;

use crate::models::GameListing;
use crate::ui_v2::views::marketplace_loader::use_marketplace_listings;

const FALLBACK_COVER: &str = "https://lh3.googleusercontent.com/aida-public/AB6AXuDcG9Zo3aR9Vrpk5pP2jenw1AoVFoOzbAQ-t57kQtlbwGQVsLLwmHyFuyzRVsOh71iN4mHyhfw0Sx4YgdJ9duL9ANv3Xa1W7jYKWeVgj5_rE7KzitErwV3dtgEFGsGCSXtFQxyw6tQoGmP3V-Ci9Vs9_ZQXh6WXrFi6eperEaPm3YutXUIImUuC5sKm2hgyVb6sMBnpn0Imy94ETrJ9WO2XeC6tTMddB6EA-x1LgnN3Ezj_dPitegkcYmXGBSWZyCTZgxINu01kmdM";

fn first_valid_image(images: &[String]) -> String {
    images
        .iter()
        .find(|url| {
            let trimmed = url.trim();
            !trimmed.is_empty()
                && (trimmed.starts_with("https://") || trimmed.starts_with("http://"))
                && !trimmed.contains('"')
                && !trimmed.contains('\'')
                && !trimmed.contains(')')
        })
        .cloned()
        .unwrap_or_else(|| FALLBACK_COVER.to_string())
}

#[component]
pub fn StoreFrontView(
    on_select: Callback<GameListing>,
    on_view_all: Callback<()>,
) -> impl IntoView {
    let marketplace = use_marketplace_listings();
    let listings = marketplace.listings;
    let loading = marketplace.loading;
    let error = marketplace.error;
    let received_count = marketplace.received_count;

    let featured_listing = Signal::derive(move || listings.get().first().cloned());

    view! {
        <div class="max-w-[1600px] mx-auto p-8 space-y-12">
            <section class="group relative h-[500px] rounded-xl overflow-hidden transition-transform duration-700 hover:scale-[1.02] [transform:translateZ(0)] [backface-visibility:hidden]">
                <div class="absolute inset-0 bg-surface-high">
                    <img
                        class="w-full h-full object-cover"
                        src=move || {
                            featured_listing
                                .get()
                                .map(|listing| first_valid_image(&listing.images))
                                .unwrap_or_else(|| FALLBACK_COVER.to_string())
                        }
                        alt="featured game cover"
                    />
                </div>
                <div class="absolute inset-0 bg-gradient-to-t from-background via-background/40 to-transparent"></div>

                <div class="absolute bottom-0 left-0 p-12 w-full flex justify-between items-end">
                    {move || {
                        if let Some(listing) = featured_listing.get() {
                            let buy_listing = listing.clone();
                            let details_listing = listing.clone();
                            view! {
                                <>
                                    <div class="max-w-2xl">
                                        <span class="inline-block bg-secondary/20 text-secondary border border-secondary/30 px-3 py-1 rounded-sm text-xs font-bold tracking-widest uppercase mb-4">"Featured Release"</span>
                                        <h1 class="text-6xl font-headline font-bold text-on-surface mb-4 leading-none tracking-tight -ml-1">{listing.title.clone()}</h1>
                                        <p class="text-on-surface-variant text-lg mb-8 line-clamp-2">{listing.description.clone()}</p>
                                        <div class="flex items-center gap-4">
                                            <button class="px-8 py-4 bg-gradient-to-r from-primary to-primary-dim text-on-primary font-bold rounded-md active:scale-95 transition-transform duration-150 ease-out motion-safe:will-change-transform flex items-center gap-2" on:click=move |_| on_select.run(buy_listing.clone())>
                                                <span class="material-symbols-outlined">"shopping_cart"</span>
                                                {format!("Buy Now - {} {}", listing.price, listing.currency)}
                                            </button>
                                            <button class="px-8 py-4 bg-surface-bright/50 backdrop-blur-md text-on-surface font-bold rounded-md hover:bg-surface-bright transition-[background-color,opacity] duration-300 ease-out active:scale-95 motion-safe:will-change-transform" on:click=move |_| on_select.run(details_listing.clone())>
                                                "View Details"
                                            </button>
                                        </div>
                                    </div>

                                    <div class="hidden lg:flex flex-col items-end gap-2">
                                        <div class="bg-surface-container-high/60 backdrop-blur-xl border border-outline-variant/15 p-4 rounded-xl flex items-center gap-4">
                                            <div class="flex -space-x-3">
                                                <img class="w-8 h-8 rounded-full border-2 border-surface-container-high" src="https://lh3.googleusercontent.com/aida-public/AB6AXuBBSzzg0N3F0V_XwE6qHRHM0RJC3AcGT9xmzIkFTAhegUAnD2GGXhD2daCir80vK1Zg5SuCHH7bnN3XnWDFzMaH_lUVdBsZd7obCiQW0KMvZ25QItAdGhxbmCLsLVIJ39vqTz85n6aJbFwbpelvBuW4y4O8hIDkXO4eWLZW-i5Y8lZGNrTT-8rvplClSvhV2ZRZ7lS0E9oNnM1ULSYlFN_xbTELM1SY5nsvzFRvLfBTnTTo4QRSJfMUmKBoe7cA6FL4jE0dC6tRiVs" />
                                                <img class="w-8 h-8 rounded-full border-2 border-surface-container-high" src="https://lh3.googleusercontent.com/aida-public/AB6AXuAWBcYYLJ9cCRLUXIiR-QrL5DYrwiOaphcETFMDhQvq2sqJNeXouZRZM9rWg3hTXxa6OuWCSOa4pu-BS02TKtBql9NOliX1an_CimCjJpmKtaMf-I3FFJumXmr_H1QiftdQAQsceYs70luTGPJgx6K1A56EkHmNO0wflBvVvijJD4hXIuNNU2LDBN3OlMq_njPvcUX8bclMV7TFLWLjhI9kf9nRfj4Q0SuLG_6dmZjs9fulvg5xxbMRmDe2mWaKiKpoMPdhrbgKt7Y" />
                                                <img class="w-8 h-8 rounded-full border-2 border-surface-container-high" src="https://lh3.googleusercontent.com/aida-public/AB6AXuA4errBEe-bhLvIAH6Gg1dCevgRGwF5dtlnanBeDCSMqd_Onp3m13_QK1WOxoXffktshpYXSz1p5ntLmciPJxcUFuQ70BTOtyWuWF-HhBNOW2rql-tqAaOfiVYj7f6UEp1MHeTeI92ieIqDM5v1TbSLV4CEgFPGDK027Ve6YzVO3g0OUeXNbMV4_AQ2rbKC-a6WECOyrSNBB4wNWbg7qLVsjo7pAKoRySdad7FQpslYRnYxMB8-oWxWciwo3gJlcT_7SUhaxOXdj1A" />
                                            </div>
                                            <div>
                                                <p class="text-xs text-on-surface-variant font-medium">"Trending Zaps"</p>
                                                <p class="text-tertiary font-bold">"⚡ 12.8k today"</p>
                                            </div>
                                        </div>
                                    </div>
                                </>
                            }
                            .into_any()
                        } else {
                            view! {
                                <div class="max-w-2xl">
                                    <span class="inline-block bg-secondary/20 text-secondary border border-secondary/30 px-3 py-1 rounded-sm text-xs font-bold tracking-widest uppercase mb-4">"Featured Release"</span>
                                    <h1 class="text-6xl font-headline font-bold text-on-surface mb-4 leading-none tracking-tight -ml-1">"Arcadestr - The Neon Curator"</h1>
                                    <p class="text-on-surface-variant text-lg mb-8 line-clamp-2">"Discover decentralized games from the Nostr network with direct Lightning payments."</p>
                                </div>
                            }
                            .into_any()
                        }
                    }}
                </div>
            </section>

            <section class="grid grid-cols-2 md:grid-cols-4 gap-4">
                <div class="group relative h-24 rounded-lg overflow-hidden cursor-pointer">
                    <div class="absolute inset-0 bg-[#b6a0ff]/20 group-hover:bg-[#b6a0ff]/30 transition-[background-color,opacity] duration-300 ease-out motion-safe:will-change-transform"></div>
                    <div class="absolute inset-0 flex items-center justify-center"><span class="font-headline font-bold text-xl tracking-wide">"ACTION"</span></div>
                </div>
                <div class="group relative h-24 rounded-lg overflow-hidden cursor-pointer">
                    <div class="absolute inset-0 bg-[#00d2fd]/20 group-hover:bg-[#00d2fd]/30 transition-[background-color,opacity] duration-300 ease-out motion-safe:will-change-transform"></div>
                    <div class="absolute inset-0 flex items-center justify-center"><span class="font-headline font-bold text-xl tracking-wide">"RPG"</span></div>
                </div>
                <div class="group relative h-24 rounded-lg overflow-hidden cursor-pointer">
                    <div class="absolute inset-0 bg-[#ff96bb]/20 group-hover:bg-[#ff96bb]/30 transition-[background-color,opacity] duration-300 ease-out motion-safe:will-change-transform"></div>
                    <div class="absolute inset-0 flex items-center justify-center"><span class="font-headline font-bold text-xl tracking-wide">"STRATEGY"</span></div>
                </div>
                <div class="group relative h-24 rounded-lg overflow-hidden cursor-pointer">
                    <div class="absolute inset-0 bg-surface-bright group-hover:bg-surface-container-highest transition-[background-color] duration-300 ease-out motion-safe:will-change-transform"></div>
                    <div class="absolute inset-0 flex items-center justify-center"><span class="font-headline font-bold text-xl tracking-wide">"INDIE"</span></div>
                </div>
            </section>

            <div class="flex flex-col lg:flex-row gap-8">
                <div class="flex-1">
                    <div class="flex items-center justify-between mb-8">
                        <h2 class="text-3xl font-headline font-bold tracking-tight">"Trending Games"</h2>
                        <button
                            class="text-primary text-sm font-bold hover:underline"
                            on:click=move |_| on_view_all.run(())
                        >
                            "View All"
                        </button>
                    </div>

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
                                <div class="grid grid-cols-1 md:grid-cols-2 xl:grid-cols-3 gap-6">
                                    {listings
                                        .get()
                                        .into_iter()
                                        .take(3)
                                        .enumerate()
                                        .map(|(index, listing)| {
                                            let selected = listing.clone();
                                            let subtitle = match index {
                                                0 => "Action-Adventure • Multiplayer",
                                                1 => "Cyberpunk • Platformer",
                                                _ => "Strategy • Sci-Fi",
                                            };
                                            let zaps = match index {
                                                0 => "2.4k",
                                                1 => "842",
                                                _ => "5.1k",
                                            };
                                            let price_label = match index {
                                                1 => "Free to Play".to_string(),
                                                _ => format!("{} {}", listing.price, listing.currency),
                                            };
                                            let secondary_label = match index {
                                                1 => "⚡ Zap to Support".to_string(),
                                                _ => "$5.80 USD".to_string(),
                                            };
                                            let cta_label = if index == 1 { "Play Now" } else { "Quick Buy" };
                                            let image_url = first_valid_image(&listing.images);

                                            view! {
                                                <button class="group bg-surface-container-high rounded-xl overflow-hidden hover:scale-[1.02] hover:bg-surface-bright transition-[transform,background-color] duration-300 ease-out motion-safe:will-change-transform text-left h-full flex flex-col" on:click=move |_| on_select.run(selected.clone())>
                                                    <div class="aspect-video relative">
                                                        <img class="w-full h-full object-cover" src={image_url} alt="game cover" />
                                                        <div class="absolute top-3 right-3 bg-background/80 backdrop-blur-md px-2 py-1 rounded text-[10px] font-bold text-tertiary">{format!("⚡ {}", zaps)}</div>
                                                    </div>
                                                    <div class="p-6 flex flex-1 flex-col">
                                                        <h3 class="text-lg font-bold mb-1 leading-tight line-clamp-2" style="min-height: 3.5rem;">{listing.title.clone()}</h3>
                                                        <p class="text-xs text-on-surface-variant italic line-clamp-1" style="min-height: 1rem;">{subtitle}</p>
                                                        <div class="flex items-center justify-between mt-auto pt-6">
                                                            <div>
                                                                <p class="text-xs text-on-surface-variant font-medium">{price_label}</p>
                                                                <p class="text-sm font-bold text-on-surface">{secondary_label}</p>
                                                            </div>
                                                            <span class="px-4 py-2 bg-secondary text-on-secondary text-xs font-bold rounded-md hover:brightness-110 active:scale-95 transition-all">{cta_label}</span>
                                                        </div>
                                                    </div>
                                                </button>
                                            }
                                        })
                                        .collect::<Vec<_>>()}
                                </div>
                            }
                            .into_any()
                        }
                    }}
                </div>

                <aside class="w-full lg:w-80 space-y-6">
                    <div class="flex items-center justify-between">
                        <h2 class="text-xl font-headline font-bold">"Live on Nostr"</h2>
                        <span class="flex h-2 w-2 rounded-full bg-error animate-pulse"></span>
                    </div>
                    <div class="space-y-4">
                        <div class="glass-panel p-4 rounded-xl border border-outline-variant/10">
                            <div class="flex items-start gap-3 mb-3">
                                <img class="w-8 h-8 rounded-full" src="https://lh3.googleusercontent.com/aida-public/AB6AXuBIoQevk84wqPolEShM7TFe6N2FPFZjPbMa1QZI0SahAk-tDcuYx8FjSpxMwr2sQdqUQLjsUMDqS0eSqCdw73bRvLNoXyaAhhBd0wTFpgZo5ARFVBdeSqQC4MXwNK0Zb7_sCnGo2SGDx6hj3wJPxbTiI_8d7pfZEwe1pe2odh2dT34vlTu94aD-qtAi4fuEJkUioWatebekVzSmSTHCGsXHxq9zOOO7gSrW47xyzHTxdvNG0jL_ZGNfy5F716wnhqfcIWLzxeYBxF8" />
                                <div>
                                    <p class="text-xs font-bold">"alice_blocks"</p>
                                    <p class="text-[10px] text-on-surface-variant">"2m ago • npub1...4x8"</p>
                                </div>
                            </div>
                            <p class="text-sm text-on-surface-variant mb-4 leading-relaxed">"Just hit level 50 in #NeonVelocity! Those tournaments are getting wild ⚡🏆"</p>
                            <div class="flex items-center gap-4 text-xs text-on-surface-variant">
                                <span class="flex items-center gap-1"><span class="material-symbols-outlined text-sm">"favorite"</span>"42"</span>
                                <span class="flex items-center gap-1 text-tertiary font-bold"><span class="material-symbols-outlined text-sm" style="font-variation-settings: 'FILL' 1;">"bolt"</span>"1.2k"</span>
                            </div>
                        </div>

                        <div class="glass-panel p-4 rounded-xl border border-outline-variant/10">
                            <div class="flex items-start gap-3 mb-3">
                                <img class="w-8 h-8 rounded-full" src="https://lh3.googleusercontent.com/aida-public/AB6AXuDagOhSCH7fGjb86JbEnaYg76xpM33hOAK0fQYDvHWOkco61ErwyEKigFKlzJYNJe_QYzyq-WR32Arh2dx4XCx5GDwk7XbCh0H5iOV6gFTKaPzto4LRKyzfU8FbbnBydOQwQ67yizgwE78tJP7UeS2PTwSq3tvWBtEJ0Q6pH5jLlG5wfDiZ_Z64KCW-P7zF8XftwGLamG5AzkQirQcDulvvKIMB_OFg57vQXh5K_yhVkjp5EKCKPearLsjHnIwA893u5yGRfAcn5IM" />
                                <div>
                                    <p class="text-xs font-bold">"satoshi_gamer"</p>
                                    <p class="text-[10px] text-on-surface-variant">"15m ago • npub1...2z9"</p>
                                </div>
                            </div>
                            <p class="text-sm text-on-surface-variant mb-4 leading-relaxed">"Anyone looking to trade items in Bit-Runners? Got a Rare Core available. DM on Nostr! 💾"</p>
                            <div class="flex items-center gap-4 text-xs text-on-surface-variant">
                                <span class="flex items-center gap-1"><span class="material-symbols-outlined text-sm">"favorite"</span>"12"</span>
                                <span class="flex items-center gap-1 text-tertiary font-bold"><span class="material-symbols-outlined text-sm" style="font-variation-settings: 'FILL' 1;">"bolt"</span>"250"</span>
                            </div>
                        </div>

                        <div class="glass-panel p-4 rounded-xl border border-outline-variant/10">
                            <div class="flex items-start gap-3 mb-3">
                                <img class="w-8 h-8 rounded-full" src="https://lh3.googleusercontent.com/aida-public/AB6AXuCHFgozIEgmtwE5Gk8lV7zPpwts-Ihf7fln4Cbgq8RKvBoI0k7uCRLkVOvlM7C6VvgCHMYLmLck4jLDzq3124G9EMTFFTRB4owvIeTjg3rLQmO8OD07ySI0GtZhKvhBqk7AjI3AZS9EuOIApi572uKMxLB6r_nMhYX38zSV7k-_-xyOLTzKIoq_wNbPIlk-gUeZjH7gEWW8OHb-pAcATGxLvn24dOP7kMfOep8dyArDiMV6SOBSNe-jzfb8SV-ZeYcW_1I_dAhujww" />
                                <div>
                                    <p class="text-xs font-bold">"pixel_queen"</p>
                                    <p class="text-[10px] text-on-surface-variant">"1h ago • npub1...7u2"</p>
                                </div>
                            </div>
                            <p class="text-sm text-on-surface-variant mb-4 leading-relaxed">"The art style in Dune Settlers is immaculate. 10/10 recommendation for strategy fans."</p>
                            <div class="flex items-center gap-4 text-xs text-on-surface-variant">
                                <span class="flex items-center gap-1"><span class="material-symbols-outlined text-sm">"favorite"</span>"89"</span>
                                <span class="flex items-center gap-1 text-tertiary font-bold"><span class="material-symbols-outlined text-sm" style="font-variation-settings: 'FILL' 1;">"bolt"</span>"3.1k"</span>
                            </div>
                        </div>
                    </div>
                </aside>
            </div>
        </div>
    }
}
