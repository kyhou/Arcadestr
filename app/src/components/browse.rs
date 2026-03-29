// Browse view and listing card components

use leptos::prelude::*;
use wasm_bindgen_futures::spawn_local;

use crate::models::GameListing;
use crate::{invoke_fetch_listings, AuthContext};

/// Browse view component - displays a grid of game listings.
#[component]
pub fn BrowseView(
    on_select: Callback<GameListing>,
) -> impl IntoView {
    let _auth = use_context::<AuthContext>().expect("AuthContext not provided");
    
    // State signals
    let listings = RwSignal::new(Vec::<GameListing>::new());
    let is_loading = RwSignal::new(true);
    let error = RwSignal::new(None::<String>);
    
    // Fetch listings on mount
    Effect::new(move |_| {
        spawn_local(async move {
            is_loading.set(true);
            error.set(None);
            
            match invoke_fetch_listings(20).await {
                Ok(fetched) => {
                    listings.set(fetched);
                    is_loading.set(false);
                }
                Err(e) => {
                    error.set(Some(e));
                    is_loading.set(false);
                }
            }
        });
    });
    
    view! {
        <div class="browse-container">
            <h2 class="browse-title">"Browse Games"</h2>
            
            {move || {
                if is_loading.get() {
                    view! {
                        <div class="loading-state">
                            <p>"Fetching listings from relays..."</p>
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
fn ListingCard(
    listing: GameListing,
    on_select: Callback<GameListing>,
) -> impl IntoView {
    // Clone listing for use in closures
    let listing_clone = listing.clone();
    let listing_for_click = listing.clone();
    
    // Format publisher npub (first 16 chars + "...")
    let publisher_display = move || {
        let npub = &listing_clone.publisher_npub;
        if npub.len() > 16 {
            format!("{}...", &npub[..16])
        } else {
            npub.clone()
        }
    };
    
    // Format price display
    let price_sats = listing_clone.price_sats;
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
                <p class="listing-publisher">{publisher_display}</p>
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
