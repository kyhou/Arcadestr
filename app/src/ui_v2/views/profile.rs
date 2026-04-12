use leptos::prelude::*;
use wasm_bindgen_futures::spawn_local;

use crate::models::GameListing;
use crate::models::{Nip05Status, Nip49ExportResult};
use crate::{invoke_fetch_marketplace, AuthContext};

#[path = "../../components/nip05_badge.rs"]
mod nip05_badge;
#[path = "../../components/nip49_modal.rs"]
mod nip49_modal;
#[path = "../../tauri_bridge.rs"]
mod tauri_bridge;

use nip05_badge::Nip05Badge;
use nip49_modal::Nip49Modal;

fn normalize_nip05_identifier(identifier: &str) -> Option<(String, String, String)> {
    let trimmed = identifier.trim();
    let (local_part, domain) = trimmed.split_once('@')?;
    let normalized_local = local_part.trim().to_lowercase();
    let normalized_domain = domain.trim().to_lowercase();

    if normalized_local.is_empty() || normalized_domain.is_empty() {
        return None;
    }

    let normalized_identifier = format!("{}@{}", normalized_local, normalized_domain);
    Some((normalized_identifier, normalized_local, normalized_domain))
}

fn default_nip05_status(identifier: Option<String>) -> Nip05Status {
    let raw_identifier = identifier.unwrap_or_default();
    if let Some((normalized_identifier, local_part, domain)) =
        normalize_nip05_identifier(&raw_identifier)
    {
        Nip05Status {
            identifier: raw_identifier,
            normalized_identifier,
            local_part,
            domain,
            verified: false,
            status: "unverified".to_string(),
            message: "Verification pending".to_string(),
        }
    } else {
        Nip05Status {
            identifier: raw_identifier,
            normalized_identifier: String::new(),
            local_part: String::new(),
            domain: String::new(),
            verified: false,
            status: "unverified".to_string(),
            message: "Set a valid NIP-05 identifier to verify.".to_string(),
        }
    }
}

#[component]
pub fn ProfileV2View(
    on_open_publish: Callback<()>,
    on_open_listing: Callback<GameListing>,
) -> impl IntoView {
    let auth = use_context::<AuthContext>().expect("AuthContext not provided");

    let my_listings: RwSignal<Vec<GameListing>> = RwSignal::new(vec![]);
    let is_loading = RwSignal::new(true);
    let error = RwSignal::new(None::<String>);
    let show_nip49_modal = RwSignal::new(false);
    let last_export_result = RwSignal::new(None::<Nip49ExportResult>);
    let nip05_status = RwSignal::new(default_nip05_status(
        auth.profile.get().and_then(|profile| profile.nip05),
    ));

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

    let is_own_profile = Signal::derive(move || {
        let active_npub = auth.active_account.get().map(|account| account.npub);
        match (auth.npub.get(), active_npub) {
            (Some(current_npub), Some(active_npub)) => current_npub == active_npub,
            (Some(_), None) => true,
            _ => false,
        }
    });

    let current_npub = Signal::derive(move || auth.npub.get().unwrap_or_default());

    Effect::new(move |_| {
        let profile_nip05 = auth.profile.get().and_then(|profile| profile.nip05);
        nip05_status.set(default_nip05_status(profile_nip05));
    });

    let on_verify_nip05 = Callback::new(move |identifier: String| {
        let mut verifying = nip05_status.get();
        verifying.status = "verifying".to_string();
        verifying.message = "Checking identifier…".to_string();
        nip05_status.set(verifying);

        spawn_local(async move {
            match tauri_bridge::invoke_verify_nip05(identifier.clone()).await {
                Ok(status) => nip05_status.set(status),
                Err(bridge_error) => {
                    let fallback = default_nip05_status(Some(identifier));
                    nip05_status.set(Nip05Status {
                        status: "failed".to_string(),
                        message: format!("Verification failed: {}", bridge_error),
                        ..fallback
                    });
                }
            }
        });
    });

    let on_nip49_export = Callback::new(move |result: Nip49ExportResult| {
        last_export_result.set(Some(result));
        show_nip49_modal.set(false);
    });

    let on_nip49_cancel = Callback::new(move |_| {
        show_nip49_modal.set(false);
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
                <Show when=move || is_own_profile.get()>
                    <Nip05Badge status=nip05_status.into() on_verify=on_verify_nip05 />
                </Show>
            </header>

            <Show when=move || is_own_profile.get()>
                <div class="v2-panel v2-profile-listings">
                    <div class="v2-profile-listings-header">
                        <h3>"Account Management"</h3>
                        <button
                            class="v2-btn-primary"
                            on:click=move |_| show_nip49_modal.set(true)
                            disabled=move || current_npub.get().is_empty()
                        >
                            "Export NIP-49 backup"
                        </button>
                    </div>

                    <p>
                        "Generate a portable encrypted ncryptsec backup for your current account."
                    </p>

                    <Show when=move || last_export_result.get().is_some()>
                        <p>
                            {move || {
                                last_export_result
                                    .get()
                                    .map(|result| result.message)
                                    .unwrap_or_default()
                            }}
                        </p>
                    </Show>

                    <Nip49Modal
                        show=show_nip49_modal.into()
                        npub=current_npub.get()
                        on_export=on_nip49_export
                        on_cancel=on_nip49_cancel
                    />
                </div>
            </Show>

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
