// Browse view and listing card components

use leptos::prelude::*;
use wasm_bindgen_futures::spawn_local;

use crate::components::{ProfileAvatar, ProfileDisplayName};
use crate::fetch_and_store_profile;
use crate::models::GameListing;
use crate::store::{try_use_marketplace_store, DEFAULT_LISTING_TTL_SECS};
use crate::invoke_fetch_marketplace_stream;
use crate::AuthContext;
use gloo_timers::future::TimeoutFuture;

/// Browse view component - displays a grid of game listings.
/// Uses MarketplaceStore to persist listings across navigation.
#[component]
pub fn BrowseView(on_select: Callback<GameListing>) -> impl IntoView {
    let _auth = use_context::<AuthContext>().expect("AuthContext not provided");
    let marketplace_store = try_use_marketplace_store();

    // State signals
    let listings = RwSignal::new(Vec::<GameListing>::new());
    let is_loading = RwSignal::new(true);
    let error = RwSignal::new(None::<String>);
    let received_count = RwSignal::new(0);

    // Fetch listings on mount using streaming
    Effect::new(move |_| {
        let store = marketplace_store.clone();
        spawn_local(async move {
            is_loading.set(true);
            error.set(None);
            received_count.set(0);

            // Check if we have cached listings that are still fresh
            let should_fetch = match &store {
                Some(s) => {
                    let cached = s.get_all();
                    let needs_refresh = s.needs_refresh(DEFAULT_LISTING_TTL_SECS);

                    if !cached.is_empty() && !needs_refresh {
                        // Use cached listings
                        listings.set(cached);
                        is_loading.set(false);
                        false // Don't fetch from network
                    } else {
                        true // Need to fetch
                    }
                }
                None => true, // No store available, fetch from network
            };

            if should_fetch {
                // Clear cache to prepare for streaming
                if let Some(s) = &store {
                    s.clear();
                }
                
                // Start streaming fetch
                let store_clone = store.clone();
                let on_listing = move |listing: GameListing| {
                    received_count.update(|c| *c += 1);
                    if let Some(s) = &store_clone {
                        s.put_streaming(listing.clone());
                    }
                    // Update listings signal to trigger re-render
                    listings.update(|v| {
                        // Check for duplicates in the vector too
                        if !v.iter().any(|l| l.id == listing.id) {
                            v.push(listing);
                        }
                    });
                };
                
                let on_complete = Some({
                    let store_clone = store.clone();
                    move || {
                        // Mark cache as fresh when complete
                        if let Some(s) = &store_clone {
                            s.mark_fresh();
                        }
                        is_loading.set(false);
                    }
                });
                
                match invoke_fetch_marketplace_stream(50, Some(30), on_listing, on_complete).await {
                    Ok((product_cleanup, completion_cleanup)) => {
                        // Clean up listeners after a delay
                        spawn_local(async move {
                            TimeoutFuture::new(5000).await;
                            product_cleanup();
                            completion_cleanup();
                        });
                    }
                    Err(e) => {
                        // If fetch fails but we have cached data, use it as fallback
                        if let Some(s) = &store {
                            let cached = s.get_all();
                            if !cached.is_empty() {
                                listings.set(cached);
                                is_loading.set(false);
                                #[cfg(target_arch = "wasm32")]
                                web_sys::console::warn_1(
                                    &format!(
                                        "Failed to refresh listings, using cached data: {}",
                                        e
                                    )
                                    .into(),
                                );
                            } else {
                                error.set(Some(e));
                                is_loading.set(false);
                            }
                        } else {
                            error.set(Some(e));
                            is_loading.set(false);
                        }
                    }
                }
            }
        });
    });

    view! {
        <div class="browse-container">
            <h2 class="browse-title">"Browse Games"</h2>

            {move || {
                if is_loading.get() {
                    let count = received_count.get();
                    view! {
                        <div class="loading-state">
                            <p>
                                {if count > 0 {
                                    format!("Loading... {} products found", count)
                                } else {
                                    "Fetching listings from relays...".to_string()
                                }}
                            </p>
                        </div>
                    }.into_any()
                } else if let Some(err) = error.get() {
                    view! {
                        <div class="error-state">
                            <p>{format!("Error: {}", err)}</p>
                        </div>
                    }.into_any()
                } else if listings.get().is_empty() {
                    view! {
                        <div class="empty-state">
                            <p>"No listings found."</p>
                        </div>
                    }.into_any()
                } else {
                    view! {
                        <div class="listings-grid">
                            {listings.get().into_iter().map(|listing| {
                                view! {
                                    <ListingCard
                                        listing={listing.clone()}
                                        on_select={on_select}
                                    />
                                }
                            }).collect::<Vec<_>>()}
                        </div>
                    }.into_any()
                }
            }}
        </div>
    }
}

/// Individual listing card component.
#[component]
pub fn ListingCard(listing: GameListing, on_select: Callback<GameListing>) -> impl IntoView {
    // Clone listing for use in closures
    let listing_for_click = listing.clone();

    // Clone publisher_npub before moving into Effect
    let publisher_npub = listing.publisher_npub.clone();
    let publisher_npub_for_effect = publisher_npub.clone();

    // Fetch profile when component mounts
    Effect::new(move |_| {
        let npub = publisher_npub_for_effect.clone();
        spawn_local(async move {
            // This will use cached version if available
            let _ = fetch_and_store_profile(npub).await;
        });
    });

    // Format price display
    let price_sats = listing.price_sats;
    let price_display = move || -> String {
        if price_sats == 0 {
            "Free".to_string()
        } else {
            format!("⚡ {} sats", price_sats)
        }
    };

    let on_click = {
        move |_| {
            on_select.run(listing_for_click.clone());
        }
    };

    let tags = listing.tags.clone();
    let title = listing.title.clone();

    view! {
        <div class="listing-card">
            <div class="listing-header">
                <h3 class="listing-title">{title}</h3>
                <div
                    class="listing-publisher-row"
                    style:display="flex"
                    style:align-items="center"
                    style:gap="8px"
                    style:margin-top="4px"
                >
                    <ProfileAvatar
                        npub={publisher_npub.clone()}
                        size="24px"
                    />
                    <ProfileDisplayName
                        npub={publisher_npub.clone()}
                        truncate_npub=16
                    />
                </div>
            </div>

            <div class="listing-price">
                <span class={if price_sats == 0 { "price-free" } else { "price-paid" }}>
                    {price_display}
                </span>
            </div>

            <div class="listing-tags">
                {tags.iter().map(|tag| {
                    view! { <span class="tag-badge">{tag.clone()}</span> }
                }).collect::<Vec<_>>()}
            </div>

            <button
                class="view-button"
                on:click={on_click}
            >
                "View"
            </button>
        </div>
    }
}
