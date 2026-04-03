// Profile view component - displays the logged-in user's full profile and their listings.

use leptos::prelude::*;
use wasm_bindgen_futures::spawn_local;

use crate::components::ListingCard;
use crate::models::{GameListing, MarketplaceView};
use crate::{invoke_fetch_marketplace, AuthContext};

/// Profile view component - displays the logged-in user's full profile and their listings.
#[component]
pub fn ProfileView(set_view: WriteSignal<MarketplaceView>) -> impl IntoView {
    let auth = use_context::<AuthContext>().expect("AuthContext not provided");

    // Reactive state for listings
    let my_listings: RwSignal<Vec<GameListing>> = RwSignal::new(vec![]);
    let listings_loading: RwSignal<bool> = RwSignal::new(true);
    let listings_error: RwSignal<Option<String>> = RwSignal::new(None);
    let npub_copied: RwSignal<bool> = RwSignal::new(false);

    // Get profile from AuthContext
    let profile = move || auth.profile.get();

    // Get user's npub
    let npub = move || auth.npub.get();

    // Fetch user's listings on mount
    Effect::new(move |_| {
        let current_npub = match npub() {
            Some(n) => n,
            None => return,
        };

        spawn_local(async move {
            listings_loading.set(true);
            listings_error.set(None);

            match invoke_fetch_marketplace(50, Some(30), None).await {
                Ok(all_listings) => {
                    // Filter to only this user's listings
                    let filtered: Vec<GameListing> = all_listings
                        .into_iter()
                        .filter(|l| l.publisher_npub == current_npub)
                        .collect();
                    my_listings.set(filtered);
                }
                Err(e) => {
                    listings_error.set(Some(e));
                }
            }
            listings_loading.set(false);
        });
    });

    // Copy npub to clipboard
    let on_copy_npub = {
        let npub_copied = npub_copied.clone();
        move |_| {
            if let Some(_np) = npub() {
                #[cfg(target_arch = "wasm32")]
                {
                    use leptos::web_sys::window;
                    use wasm_bindgen::closure::Closure;
                    use wasm_bindgen::JsCast;
                    if let Some(win) = window() {
                        let _ = win.navigator().clipboard().write_text(&_np);
                        // Set timeout to revert the button
                        let npub_copied2 = npub_copied.clone();
                        let closure = Closure::wrap(Box::new(move || {
                            npub_copied2.set(false);
                        }) as Box<dyn FnMut()>);
                        let _ = win.set_timeout_with_callback_and_timeout_and_arguments_0(
                            &closure.as_ref().unchecked_ref(),
                            2000,
                        );
                        closure.forget();
                    }
                }
                #[cfg(not(target_arch = "wasm32"))]
                {
                    npub_copied.set(true);
                }
            }
        }
    };

    // Navigate to publish view
    let go_to_publish = move |_| {
        set_view.set(MarketplaceView::Publish);
    };

    // Navigate to listing detail
    let on_select_listing = {
        let set_view = set_view.clone();
        move |listing: GameListing| {
            set_view.set(MarketplaceView::Detail(listing));
        }
    };

    view! {
        <div class="profile-page">
            // Section 1: Profile header
            {move || {
                match profile() {
                    None => {
                        // Loading skeleton
                        view! {
                            <div class="profile-header">
                                <div class="profile-avatar-placeholder-lg"></div>
                                <div style="flex: 1;">
                                    <div class="skeleton-line" style="width: 200px; height: 22px; background: #333; border-radius: 4px; margin-bottom: 8px;"></div>
                                    <div class="skeleton-line" style="width: 140px; height: 14px; background: #333; border-radius: 4px;"></div>
                                </div>
                            </div>
                        }.into_any()
                    }
                    Some(p) => {
                        let display_name = p.display();
                        let avatar_letter = display_name.chars().next().map(|c| c.to_uppercase().to_string()).unwrap_or_else(|| "?".to_string());

                        view! {
                            <div class="profile-header">
                                {if let Some(url) = p.picture.clone() {
                                    Some(view! {
                                        <img src={url} class="profile-avatar-lg" alt="avatar" />
                                    }.into_any())
                                } else {
                                    Some(view! {
                                        <div class="profile-avatar-placeholder-lg">{avatar_letter}</div>
                                    }.into_any())
                                }}
                                <div style="flex: 1;">
                                    // Name row
                                    <h1 class="profile-display-name">{display_name}</h1>
                                    {if let (Some(display_name), Some(name)) = (p.display_name.clone(), p.name.clone()) {
                                        if display_name != name {
                                            Some(view! {
                                                <p class="profile-username">{"@"}{name}</p>
                                            }.into_any())
                                        } else {
                                            None
                                        }
                                    } else {
                                        None
                                    }}

                                    // NIP-05 row
                                    {if let Some(nip05) = p.nip05.clone() {
                                        Some(view! {
                                            <p class="profile-nip05">
                                                {if p.nip05_verified {
                                                    Some(view! {
                                                        <span class="verified">{"✓ "}</span>
                                                    }.into_any())
                                                } else {
                                                    Some(view! {
                                                        <span title="NIP-05 not verified">{"? "}</span>
                                                    }.into_any())
                                                }}
                                                {nip05}
                                            </p>
                                        }.into_any())
                                    } else {
                                        None
                                    }}

                                    // About
                                    {if let Some(about) = p.about.clone() {
                                        if !about.is_empty() {
                                            Some(view! {
                                                <p class="profile-about">{about}</p>
                                            }.into_any())
                                        } else {
                                            None
                                        }
                                    } else {
                                        None
                                    }}

                                    // Website
                                    {if let Some(website) = p.website.clone() {
                                        if !website.is_empty() {
                                            let website_url = website.clone();
                                            Some(view! {
                                                <a href={website_url} class="profile-link" target="_blank" rel="noopener">
                                                    {"🌐 "}{website}
                                                </a>
                                            }.into_any())
                                        } else {
                                            None
                                        }
                                    } else {
                                        None
                                    }}

                                    // Lightning address
                                    {if let Some(lud16) = p.lud16.clone() {
                                        if !lud16.is_empty() {
                                            Some(view! {
                                                <span class="profile-link">{"⚡ "}{lud16}</span>
                                            }.into_any())
                                        } else {
                                            None
                                        }
                                    } else {
                                        None
                                    }}

                                    // npub row with copy button
                                    <div class="profile-npub-row">
                                        <code class="profile-npub">{p.npub.clone()}</code>
                                        <button class="copy-btn" on:click={on_copy_npub}>
                                            {if npub_copied.get() { "✓ Copied" } else { "📋 Copy" }}
                                        </button>
                                    </div>
                                </div>
                            </div>
                        }.into_any()
                    }
                }
            }}

            <hr class="profile-divider" />

            // Section 2: My Listings
            <div class="my-listings-header">
                <h2 class="my-listings-title">"My Listings"</h2>
                <span class="my-listings-count">
                    {move || {
                        let count = my_listings.get().len();
                        format!("({})", count)
                    }}
                </span>
            </div>

            {move || {
                if listings_loading.get() {
                    Some(view! {
                        <p class="empty-listings">"Fetching your listings..."</p>
                    }.into_any())
                } else if let Some(err) = listings_error.get() {
                    Some(view! {
                        <p class="error-message">{err}</p>
                    }.into_any())
                } else if my_listings.get().is_empty() {
                    Some(view! {
                        <div class="empty-listings">
                            <p>"You haven't published any listings yet."</p>
                            <button class="empty-listings-btn" on:click={go_to_publish}>
                                "Go to Publish →"
                            </button>
                        </div>
                    }.into_any())
                } else {
                    Some(view! {
                        <div class="listings-grid">
                            {my_listings.get().into_iter().map(|listing| {
                                let on_select_listing = on_select_listing.clone();
                                view! {
                                    <ListingCard
                                        listing={listing}
                                        on_select={Callback::new(move |l: GameListing| on_select_listing(l))}
                                    />
                                }
                            }).collect::<Vec<_>>()}
                        </div>
                    }.into_any())
                }
            }}
        </div>
    }
}
