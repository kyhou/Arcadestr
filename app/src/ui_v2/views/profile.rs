use leptos::prelude::*;
use wasm_bindgen_futures::spawn_local;

use crate::models::GameListing;
use crate::{invoke_fetch_marketplace, AuthContext};

#[component]
pub fn ProfileV2View(
    on_open_publish: Callback<()>,
    on_open_listing: Callback<GameListing>,
) -> impl IntoView {
    let auth = use_context::<AuthContext>().expect("AuthContext not provided");

    let my_listings: RwSignal<Vec<GameListing>> = RwSignal::new(vec![]);
    let is_loading = RwSignal::new(true);
    let error = RwSignal::new(None::<String>);

    Effect::new(move |_| {
        let npub = match auth.npub.get() {
            Some(value) => value,
            None => return,
        };

        spawn_local(async move {
            is_loading.set(true);
            error.set(None);
            match invoke_fetch_marketplace(50, Some(30), None).await {
                Ok(all) => {
                    let filtered = all
                        .into_iter()
                        .filter(|listing| listing.publisher_npub == npub)
                        .collect::<Vec<_>>();
                    my_listings.set(filtered);
                }
                Err(fetch_error) => error.set(Some(fetch_error)),
            }
            is_loading.set(false);
        });
    });

    let display_name = Signal::derive(move || {
        auth.profile
            .get()
            .map(|profile| profile.display())
            .or_else(|| auth.npub.get())
            .unwrap_or_else(|| "Unknown".to_string())
    });

    view! {
        <section class="v2-profile-grid">
            <header class="v2-panel-glass v2-profile-hero">
                <h1 class="v2-display">{move || display_name.get()}</h1>
                <p>
                    {move || {
                        auth.npub
                            .get()
                            .map(|npub| format!("npub: {}", npub))
                            .unwrap_or_else(|| "No active account".to_string())
                    }}
                </p>
            </header>

            <div class="v2-panel v2-profile-listings">
                <div class="v2-profile-listings-header">
                    <h3>"My Listings"</h3>
                    <button class="v2-btn-primary" on:click=move |_| on_open_publish.run(())>
                        "Go to Publish"
                    </button>
                </div>

                {move || {
                    if is_loading.get() {
                        view! { <p>"Loading listings..."</p> }.into_any()
                    } else if let Some(err) = error.get() {
                        view! { <p>{err}</p> }.into_any()
                    } else if my_listings.get().is_empty() {
                        view! { <p>"No listings yet."</p> }.into_any()
                    } else {
                        view! {
                            <div class="v2-profile-list">
                                {my_listings
                                    .get()
                                    .into_iter()
                                    .map(|listing| {
                                        let selected_listing = listing.clone();
                                        view! {
                                            <button
                                                class="v2-btn-ghost v2-profile-list-item"
                                                on:click=move |_| on_open_listing.run(selected_listing.clone())
                                            >
                                                <span>{listing.title.clone()}</span>
                                                <span>{format!("{} {}", listing.price, listing.currency)}</span>
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
        </section>
    }
}
