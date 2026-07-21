use leptos::prelude::*;
use std::collections::HashMap;
use wasm_bindgen_futures::spawn_local;

use crate::campaign_management::{
    accepts_account_response, apply_campaign_pointer_mutation,
    apply_campaign_response_pointer_mutation, build_campaign_request, build_cancel_request,
    campaign_pointer_failure_retryable, campaign_pointer_update_plan, campaign_status,
    can_request_campaign_confirmation, current_user_listings, generated_campaign_id,
    listing_coordinate, validate_campaign_form, CampaignForm, CampaignPointerUpdatePlan,
    CampaignValidationError,
};
#[cfg(not(feature = "web"))]
use crate::components::PublishView;
use crate::invoke_fetch_marketplace_stream;
use crate::models::{AcquisitionPolicy, GameListing};
use crate::tauri_bridge::{
    invoke_discover_campaign_summaries, invoke_discover_campaigns, invoke_publish_campaign,
    invoke_update_campaign_pointer, CampaignPointerInput, CampaignSummaryListingInput,
    DiscoverCampaignSummariesRequest, DiscoverCampaignsRequest, DiscoveredCampaign,
    UpdateCampaignPointerRequest,
};
use crate::ui_v2::views::{use_fallback_cover, valid_cover_url};

#[derive(Clone, PartialEq)]
pub enum PublishViewState {
    Games,
    NewPublication,
    EditPublication(GameListing),
    Game(GameListing),
    Campaign {
        listing: GameListing,
        campaign: Option<DiscoveredCampaign>,
    },
}

#[component]
#[cfg(feature = "web")]
pub fn PublishV2View(
    state: PublishViewState,
    on_navigate: Callback<PublishViewState>,
) -> impl IntoView {
    let _ = (state, on_navigate);
    view! {
        <section class="v2-publisher-studio v2-publisher-unavailable" aria-labelledby="publisher-unavailable-title">
            <p class="v2-publisher-kicker">"Publisher studio"</p>
            <h1 id="publisher-unavailable-title">"Publishing unavailable on the web"</h1>
            <p>"Network publication and Promotion management require the Arcadestr desktop app. This standalone web build does not provide nonfunctional publishing controls."</p>
        </section>
    }
}

#[component]
#[cfg(not(feature = "web"))]
pub fn PublishV2View(
    state: PublishViewState,
    on_navigate: Callback<PublishViewState>,
) -> impl IntoView {
    view! {
        {match state {
            PublishViewState::Games => view! {
                <PublishedGamesView on_navigate={on_navigate.clone()} />
            }.into_any(),
            PublishViewState::NewPublication => view! { <PublishView /> }.into_any(),
            PublishViewState::EditPublication(listing) => {
                let listing_for_back = listing.clone();
                view! {
                    <div class="v2-publisher-studio"><button class="v2-btn-secondary" on:click={let on_navigate = on_navigate.clone(); move |_| on_navigate.run(PublishViewState::Game(listing_for_back.clone()))}>"Back to Game page"</button></div>
                    <PublishView listing=listing />
                }.into_any()
            }
            PublishViewState::Game(listing) => view! {
                <GameManagementView
                    listing=listing
                    on_back=Callback::new({ let on_navigate = on_navigate.clone(); move |_| on_navigate.run(PublishViewState::Games) })
                    on_navigate={on_navigate.clone()}
                />
            }.into_any(),
            PublishViewState::Campaign { listing, campaign } => view! {
                <CampaignEditorView
                    listing=listing.clone()
                    campaign=campaign
                    on_back=Callback::new({ let on_navigate = on_navigate.clone(); let listing = listing.clone(); move |_| on_navigate.run(PublishViewState::Game(listing.clone())) })
                    on_saved=Callback::new({ let on_navigate = on_navigate.clone(); move |listing| on_navigate.run(PublishViewState::Game(listing)) })
                />
            }.into_any(),
        }}
    }
}

#[component]
fn PublishedGamesView(on_navigate: Callback<PublishViewState>) -> impl IntoView {
    let auth = use_context::<crate::AuthContext>().expect("AuthContext not provided");
    let listings = RwSignal::new(Vec::<GameListing>::new());
    let loading = RwSignal::new(true);
    let error = RwSignal::new(None::<String>);
    let campaign_counts = RwSignal::new(HashMap::<String, (usize, usize)>::new());
    let campaign_counts_loading = RwSignal::new(false);
    let campaign_counts_error = RwSignal::new(false);
    let listing_generation = RwSignal::new(0_u64);
    let listing_request = RwSignal::new(None::<(String, u64)>);
    let summary_generation = RwSignal::new(0_u64);
    let summary_fingerprint = RwSignal::new(String::new());

    let auth_for_refresh = auth.clone();
    let refresh = Callback::new(move |()| {
        let Some(publisher) = auth_for_refresh.npub.get() else {
            listing_generation.update(|value| *value = value.wrapping_add(1));
            listing_request.set(None);
            listings.set(Vec::new());
            campaign_counts.set(HashMap::new());
            loading.set(false);
            error.set(Some("Authenticate as the publisher to manage games".into()));
            return;
        };
        if listing_request
            .get_untracked()
            .as_ref()
            .is_some_and(|(request_npub, _)| request_npub == &publisher)
        {
            return;
        }
        listing_generation.update(|value| *value = value.wrapping_add(1));
        let request_generation = listing_generation.get_untracked();
        listing_request.set(Some((publisher.clone(), request_generation)));
        loading.set(true);
        error.set(None);
        let listings_signal = listings;
        let auth_for_request = auth_for_refresh.clone();
        spawn_local(async move {
            let received = RwSignal::new(Vec::<GameListing>::new());
            let received_for_listing = received;
            let received_for_complete = received;
            let publisher_for_complete = publisher.clone();
            let auth_for_complete = auth_for_request.clone();
            match invoke_fetch_marketplace_stream(
                100,
                Some(3650),
                None,
                move |listing| received_for_listing.update(|items| items.push(listing)),
                Some(move || {
                    if !accepts_account_response(
                        auth_for_complete.npub.get_untracked().as_deref(),
                        &publisher_for_complete,
                        listing_generation.get_untracked(),
                        request_generation,
                    ) {
                        return;
                    }
                    let items = received_for_complete.get_untracked();
                    listings_signal.set(current_user_listings(items, &publisher_for_complete));
                    loading.set(false);
                    listing_request.set(None);
                }),
            )
            .await
            {
                Ok((product_cleanup, completion_cleanup)) => {
                    product_cleanup();
                    completion_cleanup();
                    if !accepts_account_response(
                        auth_for_request.npub.get_untracked().as_deref(),
                        &publisher,
                        listing_generation.get_untracked(),
                        request_generation,
                    ) {
                        return;
                    }
                    if loading.get_untracked() {
                        listings_signal
                            .set(current_user_listings(received.get_untracked(), &publisher));
                        loading.set(false);
                    }
                    listing_request.set(None);
                }
                Err(fetch_error) => {
                    if !accepts_account_response(
                        auth_for_request.npub.get_untracked().as_deref(),
                        &publisher,
                        listing_generation.get_untracked(),
                        request_generation,
                    ) {
                        return;
                    }
                    error.set(Some(fetch_error));
                    loading.set(false);
                    listing_request.set(None);
                }
            }
        });
    });

    Effect::new(move |_| refresh.run(()));
    Effect::new(move |_| {
        let items = listings.get();
        let Some(publisher_npub) = auth.npub.get() else {
            summary_generation.update(|value| *value = value.wrapping_add(1));
            summary_fingerprint.set(String::new());
            campaign_counts.set(HashMap::new());
            campaign_counts_loading.set(false);
            return;
        };
        if items.is_empty() {
            campaign_counts.set(HashMap::new());
            campaign_counts_loading.set(false);
            return;
        }
        let fingerprint = format!(
            "{}:{}",
            publisher_npub,
            items
                .iter()
                .map(|listing| listing.id.as_str())
                .collect::<Vec<_>>()
                .join("|")
        );
        if fingerprint == summary_fingerprint.get_untracked() {
            return;
        }
        summary_fingerprint.set(fingerprint);
        summary_generation.update(|value| *value = value.wrapping_add(1));
        let request_generation = summary_generation.get_untracked();
        campaign_counts_loading.set(true);
        campaign_counts_error.set(false);
        let request = DiscoverCampaignSummariesRequest {
            publisher_npub: publisher_npub.clone(),
            listings: items
                .into_iter()
                .map(|listing| CampaignSummaryListingInput {
                    listing_id: listing.id,
                })
                .collect(),
        };
        let auth_for_request = auth.clone();
        spawn_local(async move {
            let result = invoke_discover_campaign_summaries(request).await;
            if !accepts_account_response(
                auth_for_request.npub.get_untracked().as_deref(),
                &publisher_npub,
                summary_generation.get_untracked(),
                request_generation,
            ) {
                return;
            }
            match result {
                Ok(summaries) => campaign_counts.set(
                    summaries
                        .into_iter()
                        .map(|summary| (summary.listing_id, (summary.active, summary.upcoming)))
                        .collect(),
                ),
                Err(_) => {
                    campaign_counts_error.set(true);
                    summary_fingerprint.set(String::new());
                }
            }
            campaign_counts_loading.set(false);
        });
    });

    view! {
        <section class="v2-publisher-studio">
            <header class="v2-publisher-header">
                <div>
                    <p class="v2-publisher-kicker">"Publisher studio"</p>
                    <h1>"Published games"</h1>
                    <p>"Manage each Game page, Network publication, and Claim and keep Promotion."</p>
                </div>
                <div class="v2-publisher-actions">
                    <button class="v2-btn-secondary" on:click=move |_| refresh.run(()) disabled=move || loading.get()>"Refresh"</button>
                    <button class="v2-btn-primary" on:click=move |_| on_navigate.run(PublishViewState::NewPublication)>"New Network publication"</button>
                </div>
            </header>
            {move || error.get().map(|message| view! { <div class="v2-panel border border-error/40 text-error">{message}</div> })}
            {move || if loading.get() {
                view! { <div class="v2-panel text-on-surface-variant">"Loading your published games..."</div> }.into_any()
            } else if listings.get().is_empty() {
                view! {
                    <div class="v2-panel text-center space-y-4">
                        <h2 class="text-2xl font-headline font-bold">"No published games yet"</h2>
                        <p class="text-on-surface-variant">"Create a Network publication first, then manage Promotions from its Game page."</p>
                        <button class="v2-btn-primary" on:click=move |_| on_navigate.run(PublishViewState::NewPublication)>"Open publishing form"</button>
                    </div>
                }.into_any()
            } else {
                view! {
                    <div class="v2-publisher-game-grid">
                        {move || listings.get().into_iter().map(|listing| {
                            let selected = listing.clone();
                            let cover = valid_cover_url(&listing.images);
                            let listing_id_for_counts = listing.id.clone();
                            view! {
                                <article class="v2-publisher-game-card">
                                    {cover.map(|url| view! { <img src=url alt="cover" class="h-28 w-24 rounded-xl object-cover" on:error=use_fallback_cover /> }.into_any()).unwrap_or_else(|| view! { <div class="h-28 w-24 rounded-xl bg-surface-container-highest flex items-center justify-center text-2xl">"🎮"</div> }.into_any())}
                                    <div class="min-w-0 flex-1">
                                        <h2 class="text-xl font-headline font-bold truncate">{listing.title.clone()}</h2>
                                        <p class="text-xs text-on-surface-variant truncate">{listing_coordinate(&listing)}</p>
                                        <div class="mt-3 grid grid-cols-2 gap-2 text-sm">
                                            <span>{format!("Price: {} sats", listing.price_sats)}</span>
                                            <span>{access_label(&listing.acquisition)}</span>
                                            <span>{version_label(&listing)}</span>
                                            <span>{fulfillment_label(&listing)}</span>
                                        </div>
                                        <p class="mt-2 text-sm text-on-surface-variant">
                                            {move || if let Some((active, upcoming)) = campaign_counts.get().get(&listing_id_for_counts).copied() {
                                                 format!("Promotions: {active} Active · {upcoming} Upcoming")
                                            } else if campaign_counts_error.get() {
                                                 "Promotion status: chain unavailable".into()
                                            } else if campaign_counts_loading.get() {
                                                 "Promotions: checking Network publication...".into()
                                            } else {
                                                 "Promotions: 0 Active · 0 Upcoming".into()
                                            }}
                                        </p>
                                        <button class="v2-btn-primary mt-4" on:click=move |_| on_navigate.run(PublishViewState::Game(selected.clone()))>"Open Game page"</button>
                                    </div>
                                </article>
                            }
                        }).collect_view()}
                    </div>
                }.into_any()
            }}
        </section>
    }
}

#[component]
fn GameManagementView(
    listing: GameListing,
    on_back: Callback<()>,
    on_navigate: Callback<PublishViewState>,
) -> impl IntoView {
    let auth = use_context::<crate::AuthContext>().expect("AuthContext not provided");
    let campaigns = RwSignal::new(Vec::<DiscoveredCampaign>::new());
    let loading = RwSignal::new(true);
    let error = RwSignal::new(None::<String>);
    let publisher = listing.publisher_npub.clone();
    let listing_id = listing.id.clone();
    let pointers = listing
        .campaigns
        .iter()
        .map(|pointer| CampaignPointerInput {
            root_event_id: pointer.root_event_id.clone(),
            relay_hint: pointer.relay_hint.clone(),
        })
        .collect::<Vec<_>>();
    let listing_for_effect = listing.clone();
    let listing_for_button = listing_for_effect.clone();
    let listing_for_edit = listing.clone();
    let on_navigate_for_edit = on_navigate.clone();
    let discovery_generation = RwSignal::new(0_u64);
    let discovery_account = RwSignal::new(None::<String>);
    Effect::new(move |_| {
        let Some(initiating_npub) = auth.npub.get() else {
            discovery_generation.update(|value| *value = value.wrapping_add(1));
            discovery_account.set(None);
            campaigns.set(Vec::new());
            error.set(Some(
                "Authenticate as the developer to manage this Game page".into(),
            ));
            loading.set(false);
            return;
        };
        if initiating_npub != publisher {
            discovery_generation.update(|value| *value = value.wrapping_add(1));
            discovery_account.set(None);
            campaigns.set(Vec::new());
            error.set(Some(
                "Switch to the developer account that published this game".into(),
            ));
            loading.set(false);
            return;
        }
        if discovery_account.get_untracked().as_deref() == Some(initiating_npub.as_str()) {
            return;
        }
        discovery_generation.update(|value| *value = value.wrapping_add(1));
        let request_generation = discovery_generation.get_untracked();
        discovery_account.set(Some(initiating_npub.clone()));
        loading.set(true);
        error.set(None);
        let request = DiscoverCampaignsRequest {
            publisher_npub: publisher.clone(),
            listing_id: listing_id.clone(),
            pointers: pointers.clone(),
        };
        let auth_for_request = auth.clone();
        spawn_local(async move {
            let result = invoke_discover_campaigns(request).await;
            if !accepts_account_response(
                auth_for_request.npub.get_untracked().as_deref(),
                &initiating_npub,
                discovery_generation.get_untracked(),
                request_generation,
            ) {
                return;
            }
            match result {
                Ok(found) => campaigns.set(found),
                Err(message) => error.set(Some(message)),
            }
            loading.set(false);
            discovery_account.set(None);
        });
    });

    view! {
        <section class="v2-publisher-studio">
            <button class="v2-btn-secondary v2-publisher-back" on:click=move |_| on_back.run(())>"Back to Published games"</button>
            <header class="v2-publisher-game-hero">
                {valid_cover_url(&listing.images).map(|url| view! { <img src=url alt="cover" class="h-32 w-24 rounded-xl object-cover" on:error=use_fallback_cover /> }.into_any()).unwrap_or_else(|| view! { <div class="h-32 w-24 rounded-xl bg-surface-container-highest flex items-center justify-center text-3xl">"🎮"</div> }.into_any())}
                <div class="flex-1">
                    <p class="v2-publisher-kicker">"Game page"</p>
                    <h1>{listing.title.clone()}</h1>
                    <p class="text-sm text-on-surface-variant break-all">{listing_coordinate(&listing)}</p>
                    <div class="mt-4 flex flex-wrap gap-2 text-sm">
                        <span class="v2-chip">{format!("{} sats", listing.price_sats)}</span>
                        <span class="v2-chip">{access_label(&listing.acquisition)}</span>
                        <span class="v2-chip">{version_label(&listing)}</span>
                    </div>
                </div>
                <button class="v2-btn-secondary" on:click=move |_| on_navigate_for_edit.run(PublishViewState::EditPublication(listing_for_edit.clone()))>"Edit Network publication"</button>
            </header>
            <div class="v2-publisher-management-layout">
            <main class="v2-publisher-main">
            <section class="v2-publisher-panel">
                <h2>"Network publication"</h2>
                <dl class="grid gap-4 sm:grid-cols-2 lg:grid-cols-3 text-sm">
                    <div><dt class="text-on-surface-variant">"Published"</dt><dd>{format_unix(listing.created_at)}</dd></div>
                    <div><dt class="text-on-surface-variant">"Platforms"</dt><dd>{if listing.platforms.is_empty() { "Unspecified".into() } else { listing.platforms.join(", ") }}</dd></div>
                    <div><dt class="text-on-surface-variant">"ADP fulfillment"</dt><dd>{fulfillment_label(&listing)}</dd></div>
                    <div><dt class="text-on-surface-variant">"ADP server"</dt><dd class="break-all">{adp_server_label(&listing)}</dd></div>
                    <div><dt class="text-on-surface-variant">"Promotion links"</dt><dd>{listing.campaigns.len()}</dd></div>
                </dl>
                <details class="v2-publisher-diagnostics"><summary>"Network diagnostics"</summary><p class="break-all">{format!("Listing event: {}", listing.event_id.clone().unwrap_or_else(|| "Unavailable".into()))}</p></details>
            </section>
            <section class="v2-publisher-panel">
                <div class="flex flex-col gap-3 md:flex-row md:items-center md:justify-between">
                    <div><h2>"Promotions"</h2><p class="text-sm text-on-surface-variant">"Claim and keep creates durable access. A Promotion link is an advisory discovery hint, never validity."</p></div>
                    <button class="v2-btn-primary" on:click=move |_| on_navigate.run(PublishViewState::Campaign { listing: listing_for_button.clone(), campaign: None })>"New Promotion"</button>
                </div>
                {move || error.get().map(|message| view! { <p class="text-error" role="alert">{format!("Promotion chain unavailable: {message}")}</p> })}
                {move || if loading.get() { view! { <p class="text-on-surface-variant">"Discovering Promotions..."</p> }.into_any() } else if campaigns.get().is_empty() { view! { <p class="text-on-surface-variant">"No valid Promotions found. Discovery checks Promotion links and relay search."</p> }.into_any() } else { let selected_listing = listing_for_effect.clone(); let navigate = on_navigate.clone(); view! { <div class="v2-publisher-promotion-list">{campaigns.get().into_iter().map(|campaign| campaign_row(campaign, selected_listing.clone(), navigate.clone())).collect_view()}</div> }.into_any() }}
            </section>
            </main>
            <aside class="v2-publisher-panel v2-publisher-sidebar">
                <h2>"Distribution"</h2>
                <div><h3 class="font-bold">"Platforms"</h3><p class="text-sm text-on-surface-variant">{if listing.platforms.is_empty() { "Unspecified".into() } else { listing.platforms.join(", ") }}</p></div>
                <div><h3 class="font-bold">"Acquisition policy"</h3><p class="text-sm text-on-surface-variant">{access_label(&listing.acquisition)}</p><p class="text-xs text-on-surface-variant mt-1">"Timed access is configured on the Game page, not as a Claim and keep Promotion."</p></div>
            </aside>
            </div>
        </section>
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CampaignConfirmation {
    CancelCampaign,
    RemovePointer,
    DiscardChanges,
}

impl CampaignConfirmation {
    fn title(self) -> &'static str {
        match self {
            Self::CancelCampaign => "Cancel Promotion?",
            Self::RemovePointer => "Remove Promotion link?",
            Self::DiscardChanges => "Discard unsaved changes?",
        }
    }

    fn message(self) -> &'static str {
        match self {
            Self::CancelCampaign => {
                "New claims stop immediately. Prior claims remain valid; campaign cancellation does not revoke prior claims."
            }
            Self::RemovePointer => {
                "Cancellation remains authoritative either way. Remove its advisory Promotion link from the Game page too?"
            }
            Self::DiscardChanges => "Your unsaved Promotion changes will be lost.",
        }
    }

    fn reject_label(self) -> &'static str {
        match self {
            Self::CancelCampaign => "Keep Promotion",
            Self::RemovePointer => "Keep Promotion link",
            Self::DiscardChanges => "Keep editing",
        }
    }

    fn accept_label(self) -> &'static str {
        match self {
            Self::CancelCampaign => "Cancel Promotion",
            Self::RemovePointer => "Remove Promotion link",
            Self::DiscardChanges => "Discard changes",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ConfirmationOutcome {
    Close,
    PromptRemovePointer,
    CancelCampaign(bool),
    DiscardChanges,
}

fn resolve_confirmation(
    confirmation: CampaignConfirmation,
    accepted: Option<bool>,
    points_here: bool,
) -> ConfirmationOutcome {
    match (confirmation, accepted) {
        (_, None) => ConfirmationOutcome::Close,
        (CampaignConfirmation::DiscardChanges, Some(true)) => ConfirmationOutcome::DiscardChanges,
        (CampaignConfirmation::DiscardChanges, Some(false))
        | (CampaignConfirmation::CancelCampaign, Some(false)) => ConfirmationOutcome::Close,
        (CampaignConfirmation::CancelCampaign, Some(true)) if points_here => {
            ConfirmationOutcome::PromptRemovePointer
        }
        (CampaignConfirmation::CancelCampaign, Some(true)) => {
            ConfirmationOutcome::CancelCampaign(false)
        }
        (CampaignConfirmation::RemovePointer, Some(remove_pointer)) => {
            ConfirmationOutcome::CancelCampaign(remove_pointer)
        }
    }
}

#[component]
fn CampaignConfirmationDialog(
    confirmation: RwSignal<Option<CampaignConfirmation>>,
    on_decision: Callback<Option<bool>>,
) -> impl IntoView {
    let dialog_ref = NodeRef::<leptos::html::Dialog>::new();
    let reject_ref = NodeRef::<leptos::html::Button>::new();
    Effect::new(move |_| {
        let Some(dialog) = dialog_ref.get() else {
            return;
        };
        if confirmation.get().is_some() {
            if dialog.open() {
                dialog.close();
            }
            let _ = dialog.show_modal();
            if let Some(reject) = reject_ref.get() {
                let _ = reject.focus();
            }
        } else if dialog.open() {
            dialog.close();
        }
    });

    view! {
        <dialog
            node_ref=dialog_ref
            class="v2-publisher-dialog"
            aria-label=move || confirmation.get().map(CampaignConfirmation::title).unwrap_or_default()
            aria-description=move || confirmation.get().map(CampaignConfirmation::message).unwrap_or_default()
            on:cancel=move |event: web_sys::Event| {
                event.prevent_default();
                on_decision.run(None);
            }
            on:click=move |_| on_decision.run(None)
        >
            <section
                class="v2-publisher-dialog-card"
                on:click=move |event| event.stop_propagation()
            >
                <div class="space-y-2">
                    <h2 class="text-xl font-headline font-bold">
                        {move || confirmation.get().map(CampaignConfirmation::title).unwrap_or_default()}
                    </h2>
                    <p class="text-sm text-on-surface-variant">
                        {move || confirmation.get().map(CampaignConfirmation::message).unwrap_or_default()}
                    </p>
                </div>
                <div class="flex flex-wrap justify-end gap-3">
                    <button node_ref=reject_ref class="v2-btn-secondary" on:click=move |_| on_decision.run(Some(false))>
                        {move || confirmation.get().map(CampaignConfirmation::reject_label).unwrap_or_default()}
                    </button>
                    <button class="v2-btn-primary" on:click=move |_| on_decision.run(Some(true))>
                        {move || confirmation.get().map(CampaignConfirmation::accept_label).unwrap_or_default()}
                    </button>
                </div>
            </section>
        </dialog>
    }
}

fn campaign_row(
    campaign: DiscoveredCampaign,
    listing: GameListing,
    on_navigate: Callback<PublishViewState>,
) -> impl IntoView {
    let auth = use_context::<crate::AuthContext>().expect("AuthContext not provided");
    let campaign_for_edit = campaign.clone();
    let campaign_for_view = campaign.clone();
    let listing_for_view = listing.clone();
    let navigate_for_view = on_navigate.clone();
    let status = campaign_status(&campaign.classification);
    let points_here = listing
        .campaigns
        .iter()
        .any(|pointer| pointer.root_event_id == campaign.root_event_id);
    let is_upcoming = campaign.classification == "upcoming";
    let is_active = campaign.classification == "active";
    let cancel_message = RwSignal::new(None::<String>);
    let pointer_message = RwSignal::new(None::<String>);
    let pointer_cleanup_retry = RwSignal::new(false);
    let action_in_progress = RwSignal::new(false);
    let action_completed = RwSignal::new(false);
    let action_generation = RwSignal::new(0_u64);
    let action_account = RwSignal::new(auth.npub.get_untracked());
    let auth_for_epoch = auth.clone();
    Effect::new(move |_| {
        let current = auth_for_epoch.npub.get();
        if current != action_account.get_untracked() {
            action_account.set(current);
            action_generation.update(|value| *value = value.wrapping_add(1));
        }
    });
    let confirmation = RwSignal::new(None::<CampaignConfirmation>);
    let pointer_campaign = campaign.clone();
    let pointer_listing = listing.clone();
    let pointer_auth = auth.clone();
    let pointer_navigate = on_navigate.clone();
    let on_pointer_update = Callback::new(move |remove: bool| {
        if action_in_progress.get_untracked() || action_completed.get_untracked() {
            return;
        }
        let Some(publisher_npub) = pointer_auth.npub.get() else {
            pointer_message.set(Some("Authenticate as the publisher first".into()));
            return;
        };
        if publisher_npub != pointer_listing.publisher_npub {
            pointer_message.set(Some(
                "Switch to the developer account that published this game".into(),
            ));
            return;
        }
        action_generation.update(|value| *value = value.wrapping_add(1));
        let request_generation = action_generation.get_untracked();
        let request = UpdateCampaignPointerRequest {
            publisher_npub: publisher_npub.clone(),
            listing_id: pointer_listing.id.clone(),
            campaign_root_id: pointer_campaign.root_event_id.clone(),
            remove,
        };
        let listing = pointer_listing.clone();
        let root_event_id = pointer_campaign.root_event_id.clone();
        let navigate = pointer_navigate.clone();
        let auth_for_request = pointer_auth.clone();
        action_in_progress.set(true);
        spawn_local(async move {
            let result = invoke_update_campaign_pointer(request).await;
            if !accepts_account_response(
                auth_for_request.npub.get_untracked().as_deref(),
                &publisher_npub,
                action_generation.get_untracked(),
                request_generation,
            ) {
                action_in_progress.set(false);
                action_completed.set(true);
                pointer_message.set(Some("Account changed while the Promotion link was updating. The stale response was ignored; refresh this Game page before another change.".into()));
                return;
            }
            match result {
                Ok(listing_event_id) => {
                    action_completed.set(true);
                    let updated = apply_campaign_pointer_mutation(
                        &listing,
                        &root_event_id,
                        &listing_event_id,
                        remove,
                    );
                    navigate.run(PublishViewState::Game(updated));
                }
                Err(error) => {
                    pointer_message.set(Some(error));
                    action_in_progress.set(false);
                }
            }
        });
    });
    let cancel_campaign = campaign.clone();
    let cancel_listing = listing.clone();
    let cancel_navigate = on_navigate.clone();
    let cancel_with_pointer = Callback::new(move |remove_pointer: bool| {
        if action_in_progress.get_untracked() || action_completed.get_untracked() {
            return;
        }
        let Some(publisher_npub) = auth.npub.get() else {
            cancel_message.set(Some("Authenticate as the publisher first".into()));
            return;
        };
        if publisher_npub != cancel_listing.publisher_npub {
            cancel_message.set(Some(
                "Switch to the developer account that published this game".into(),
            ));
            return;
        }
        let Some(predecessor) = cancel_campaign.event_id.clone() else {
            cancel_message.set(Some("The Promotion update reference is unavailable".into()));
            return;
        };
        action_generation.update(|value| *value = value.wrapping_add(1));
        let request_generation = action_generation.get_untracked();
        let request = build_cancel_request(
            publisher_npub.clone(),
            cancel_listing.id.clone(),
            cancel_campaign.campaign_id.clone(),
            predecessor,
            remove_pointer,
        );
        let listing = cancel_listing.clone();
        let navigate = cancel_navigate.clone();
        let auth_for_request = auth.clone();
        action_in_progress.set(true);
        spawn_local(async move {
            let result = invoke_publish_campaign(request).await;
            if !accepts_account_response(
                auth_for_request.npub.get_untracked().as_deref(),
                &publisher_npub,
                action_generation.get_untracked(),
                request_generation,
            ) {
                action_in_progress.set(false);
                action_completed.set(true);
                cancel_message.set(Some("Account changed while cancellation was being signed. The stale response was ignored; refresh this Game page to reconcile the Promotion state.".into()));
                return;
            }
            match result {
                Ok(response) => {
                    let pointer_failed = response.pointer_update_error.is_some();
                    action_completed.set(!pointer_failed);
                    action_in_progress.set(false);
                    pointer_cleanup_retry.set(pointer_failed && remove_pointer);
                    let updated = apply_campaign_response_pointer_mutation(
                        &listing,
                        &response,
                        true,
                        remove_pointer,
                    );
                    cancel_message.set(Some(
                        response
                            .pointer_update_error
                            .map(|error| format!("Promotion cancelled, but Promotion link cleanup failed: {error}. Cancellation remains authoritative; retry cleanup here."))
                            .unwrap_or_else(|| {
                                "Promotion cancelled. New claims stop; prior claims remain valid because campaign cancellation does not revoke them.".into()
                            }),
                    ));
                    if !pointer_failed {
                        navigate.run(PublishViewState::Game(updated.unwrap_or(listing)));
                    }
                }
                Err(error) => {
                    cancel_message.set(Some(error));
                    action_in_progress.set(false);
                }
            }
        });
    });
    let cancel_after_confirmation = cancel_with_pointer.clone();
    let on_confirmation_decision = Callback::new(move |accepted: Option<bool>| {
        let Some(current) = confirmation.get_untracked() else {
            return;
        };
        match resolve_confirmation(current, accepted, points_here) {
            ConfirmationOutcome::Close => confirmation.set(None),
            ConfirmationOutcome::PromptRemovePointer => {
                confirmation.set(Some(CampaignConfirmation::RemovePointer));
            }
            ConfirmationOutcome::CancelCampaign(remove_pointer) => {
                confirmation.set(None);
                cancel_after_confirmation.run(remove_pointer);
            }
            ConfirmationOutcome::DiscardChanges => confirmation.set(None),
        }
    });
    let on_cancel = move |_| {
        if can_request_campaign_confirmation(
            action_in_progress.get_untracked(),
            action_completed.get_untracked(),
        ) {
            confirmation.set(Some(CampaignConfirmation::CancelCampaign));
        }
    };
    let pointer_for_add = on_pointer_update.clone();
    let pointer_for_remove = on_pointer_update.clone();
    let pointer_for_retry = on_pointer_update.clone();
    view! {
        <article class="v2-publisher-promotion-row">
            <div>
                <div class="flex flex-wrap items-center gap-2"><strong>{campaign.campaign_id.clone()}</strong><span class="v2-chip">{status}</span>{if points_here && (is_upcoming || is_active) { view! { <span class="v2-chip">"Promotion link present"</span> }.into_any() } else if points_here { view! { <span class="v2-chip">"Promotion link stale"</span> }.into_any() } else { view! { <span class="text-xs text-on-surface-variant">"Promotion link missing"</span> }.into_any() }}</div>
                <p class="text-sm text-on-surface-variant mt-2">{format!("{} to {}", format_unix(campaign.starts_at), format_unix(campaign.ends_at))}</p>
                <details class="v2-publisher-diagnostics"><summary>"Network diagnostics"</summary><p class="break-all mt-1">{format!("Campaign root event: {} | current event: {}", campaign.root_event_id, campaign.event_id.clone().unwrap_or_default())}</p></details>
            </div>
            <div class="flex flex-wrap gap-2">
                {if is_upcoming { view! { <button class="v2-btn-secondary" disabled=move || action_in_progress.get() || action_completed.get() on:click={move |_| {
                    if action_in_progress.get_untracked() || action_completed.get_untracked() {
                        return;
                    }
                    on_navigate.run(PublishViewState::Campaign { listing: listing.clone(), campaign: Some(campaign_for_edit.clone()) });
                }}>"Edit"</button> }.into_any() } else { view! { <button class="v2-btn-secondary" disabled=move || action_in_progress.get() || action_completed.get() on:click=move |_| {
                    if action_in_progress.get_untracked() || action_completed.get_untracked() {
                        return;
                    }
                    navigate_for_view.run(PublishViewState::Campaign { listing: listing_for_view.clone(), campaign: Some(campaign_for_view.clone()) });
                }>"View details"</button> }.into_any() }}
                {if is_upcoming || is_active { view! { <button class="v2-btn-secondary" disabled=move || action_in_progress.get() || action_completed.get() on:click=on_cancel>"Cancel"</button> }.into_any() } else { view! { <></> }.into_any() }}
                {if !points_here && (is_upcoming || is_active) { view! { <button class="v2-btn-secondary" disabled=move || action_in_progress.get() || action_completed.get() on:click=move |_| pointer_for_add.run(false)>"Add Promotion link"</button> }.into_any() } else if points_here && !is_upcoming && !is_active { view! { <button class="v2-btn-secondary" disabled=move || action_in_progress.get() || action_completed.get() on:click=move |_| pointer_for_remove.run(true)>"Remove stale Promotion link"</button> }.into_any() } else { view! { <></> }.into_any() }}
                {move || pointer_cleanup_retry.get().then(|| { let retry = pointer_for_retry.clone(); view! { <button class="v2-btn-secondary" disabled=move || action_in_progress.get() on:click=move |_| retry.run(true)>"Retry Promotion link cleanup"</button> } })}
            </div>
            {move || action_completed.get().then(|| view! { <p class="text-sm text-secondary">"Completed"</p> })}
            {move || cancel_message.get().map(|message| view! { <p class="text-sm text-secondary">{message}</p> })}
            {move || pointer_message.get().map(|message| view! { <p class="text-sm text-secondary">{message}</p> })}
            <CampaignConfirmationDialog confirmation=confirmation on_decision=on_confirmation_decision />
        </article>
    }
}

#[component]
fn CampaignEditorView(
    listing: GameListing,
    campaign: Option<DiscoveredCampaign>,
    on_back: Callback<()>,
    on_saved: Callback<GameListing>,
) -> impl IntoView {
    let auth = use_context::<crate::AuthContext>().expect("AuthContext not provided");
    let editing = campaign.is_some();
    let now = current_unix_secs();
    let campaign_id = campaign
        .as_ref()
        .map(|item| item.campaign_id.clone())
        .unwrap_or_else(|| generated_campaign_id(now, &random_campaign_suffix()));
    let starts = campaign
        .as_ref()
        .map(|item| datetime_local(item.starts_at))
        .unwrap_or_else(|| datetime_local(now.saturating_add(60)));
    let ends = campaign
        .as_ref()
        .map(|item| datetime_local(item.ends_at))
        .unwrap_or_else(|| datetime_local(now.saturating_add(86_460)));
    let initially_points_to_campaign = campaign.as_ref().is_some_and(|item| {
        listing
            .campaigns
            .iter()
            .any(|pointer| pointer.root_event_id == item.root_event_id)
    });
    let mut initial_form = CampaignForm::new(campaign_id);
    initial_form.starts_at = starts;
    initial_form.ends_at = ends;
    if editing {
        initial_form.update_listing_pointer = initially_points_to_campaign;
    }
    let initial_snapshot = initial_form.clone();
    let form = RwSignal::new(initial_form);
    let live_validation = Memo::new(move |_| {
        let current = form.get();
        validate_campaign_form(&current)
            .err()
            .map(validation_message)
    });
    let submitting = RwSignal::new(false);
    let completed = RwSignal::new(false);
    let message = RwSignal::new(None::<String>);
    let error = RwSignal::new(None::<String>);
    let operation_generation = RwSignal::new(0_u64);
    let pointer_retry = RwSignal::new(None::<(String, bool)>);
    let operation_account = RwSignal::new(auth.npub.get_untracked());
    let auth_for_epoch = auth.clone();
    Effect::new(move |_| {
        let current = auth_for_epoch.npub.get();
        if current != operation_account.get_untracked() {
            operation_account.set(current);
            operation_generation.update(|value| *value = value.wrapping_add(1));
            submitting.set(false);
        }
    });
    let confirmation = RwSignal::new(None::<CampaignConfirmation>);
    let active = campaign
        .as_ref()
        .is_some_and(|item| item.classification == "active");
    let upcoming = campaign
        .as_ref()
        .is_some_and(|item| item.classification == "upcoming");
    let terms_read_only = editing && !upcoming;
    let cancellable = active || upcoming;
    let predecessor = campaign.as_ref().and_then(|item| item.event_id.clone());
    let campaign_for_cancel = campaign.clone();
    let listing_for_cancel = listing.clone();
    let auth_for_cancel = auth.clone();
    let auth_for_retry = auth.clone();
    let on_saved_for_cancel = on_saved.clone();
    let on_saved_for_retry = on_saved.clone();
    let listing_for_retry = listing.clone();
    let on_back_now = on_back.clone();
    let back = Callback::new(move |()| {
        if !completed.get_untracked() && form.get_untracked() != initial_snapshot {
            confirmation.set(Some(CampaignConfirmation::DiscardChanges));
            return;
        }
        on_back_now.run(());
    });
    let listing_for_save = listing.clone();
    let on_saved_for_save = on_saved.clone();
    let save = move |_| {
        if terms_read_only || submitting.get_untracked() || completed.get_untracked() {
            return;
        }
        let Some(publisher_npub) = auth.npub.get() else {
            error.set(Some("Authenticate as the publisher first".into()));
            return;
        };
        if publisher_npub != listing.publisher_npub {
            error.set(Some(
                "Switch to the developer account that published this game".into(),
            ));
            return;
        }
        let current = form.get();
        let pointer_plan = campaign_pointer_update_plan(
            initially_points_to_campaign,
            current.update_listing_pointer,
        );
        let request = match build_campaign_request(
            publisher_npub.clone(),
            listing.id.clone(),
            &current,
            predecessor.clone(),
        ) {
            Ok(request) => request,
            Err(validation) => {
                error.set(Some(validation_message(validation)));
                return;
            }
        };
        let pointer_update_requested = request.update_listing_pointer;
        let publisher_for_pointer_removal = request.publisher_npub.clone();
        submitting.set(true);
        error.set(None);
        message.set(None);
        let listing_after_save = listing_for_save.clone();
        let on_saved = on_saved_for_save.clone();
        operation_generation.update(|value| *value = value.wrapping_add(1));
        let request_generation = operation_generation.get_untracked();
        let auth_for_request = auth.clone();
        spawn_local(async move {
            let result = invoke_publish_campaign(request).await;
            if !accepts_account_response(
                auth_for_request.npub.get_untracked().as_deref(),
                &publisher_npub,
                operation_generation.get_untracked(),
                request_generation,
            ) {
                submitting.set(false);
                completed.set(true);
                error.set(Some("Account changed during Promotion publication. The stale response was ignored; return to the Game page and refresh before retrying.".into()));
                return;
            }
            match result {
                Ok(response) => {
                    completed.set(true);
                    if let Some(pointer_error) = response.pointer_update_error.as_ref() {
                        if campaign_pointer_failure_retryable(&response) {
                            pointer_retry.set(Some((response.root_event_id.clone(), false)));
                        }
                        message.set(Some(format!("Promotion published, but its Promotion link could not be updated: {pointer_error}. The Promotion remains valid and discoverable through relay search; retry the link without republishing.")));
                        submitting.set(false);
                        return;
                    }

                    if matches!(
                        pointer_plan,
                        CampaignPointerUpdatePlan::RemoveAfterCampaignPublish
                    ) {
                        let root_event_id = response.root_event_id.clone();
                        let removal_request = UpdateCampaignPointerRequest {
                            publisher_npub: publisher_for_pointer_removal.clone(),
                            listing_id: listing_after_save.id.clone(),
                            campaign_root_id: root_event_id.clone(),
                            remove: true,
                        };
                        let removal_result = invoke_update_campaign_pointer(removal_request).await;
                        if !accepts_account_response(
                            auth_for_request.npub.get_untracked().as_deref(),
                            &publisher_for_pointer_removal,
                            operation_generation.get_untracked(),
                            request_generation,
                        ) {
                            submitting.set(false);
                            completed.set(true);
                            error.set(Some("Account changed while the Promotion link was updating. The stale response was ignored; return to the Game page and refresh.".into()));
                            return;
                        }
                        match removal_result {
                            Ok(listing_event_id) => {
                                let updated = apply_campaign_pointer_mutation(
                                    &listing_after_save,
                                    &root_event_id,
                                    &listing_event_id,
                                    true,
                                );
                                message.set(Some(
                                    "Promotion published and its Promotion link was removed."
                                        .into(),
                                ));
                                submitting.set(false);
                                on_saved.run(updated);
                            }
                            Err(problem) => {
                                pointer_retry.set(Some((root_event_id, true)));
                                message.set(Some(format!("Promotion published, but its Promotion link could not be removed: {problem}. The Promotion remains authoritative; retry the link without republishing.")));
                                submitting.set(false);
                            }
                        }
                        return;
                    }

                    let updated = apply_campaign_response_pointer_mutation(
                        &listing_after_save,
                        &response,
                        false,
                        pointer_update_requested,
                    );
                    message.set(Some("Promotion published successfully.".into()));
                    submitting.set(false);
                    if matches!(
                        pointer_plan,
                        CampaignPointerUpdatePlan::AddWithCampaignPublish
                    ) {
                        on_saved.run(updated.unwrap_or(listing_after_save));
                    } else {
                        on_saved.run(listing_after_save);
                    }
                }
                Err(problem) => {
                    error.set(Some(problem));
                    submitting.set(false);
                }
            }
        });
    };
    let cancel_with_pointer = Callback::new(move |remove_pointer: bool| {
        if submitting.get_untracked() || completed.get_untracked() {
            return;
        }
        let Some(campaign) = campaign_for_cancel.clone() else {
            return;
        };
        let Some(publisher_npub) = auth_for_cancel.npub.get() else {
            error.set(Some("Authenticate as the publisher first".into()));
            return;
        };
        if publisher_npub != listing_for_cancel.publisher_npub {
            error.set(Some(
                "Switch to the developer account that published this game".into(),
            ));
            return;
        }
        let Some(tip) = campaign.event_id else {
            error.set(Some("The Promotion update reference is unavailable".into()));
            return;
        };
        let request = build_cancel_request(
            publisher_npub.clone(),
            listing_for_cancel.id.clone(),
            campaign.campaign_id,
            tip,
            remove_pointer,
        );
        submitting.set(true);
        error.set(None);
        let listing = listing_for_cancel.clone();
        let on_saved = on_saved_for_cancel.clone();
        operation_generation.update(|value| *value = value.wrapping_add(1));
        let request_generation = operation_generation.get_untracked();
        let auth_for_request = auth_for_cancel.clone();
        spawn_local(async move {
            let result = invoke_publish_campaign(request).await;
            if !accepts_account_response(
                auth_for_request.npub.get_untracked().as_deref(),
                &publisher_npub,
                operation_generation.get_untracked(),
                request_generation,
            ) {
                submitting.set(false);
                completed.set(true);
                error.set(Some("Account changed during Promotion cancellation. The stale response was ignored; return to the Game page and refresh before another change.".into()));
                return;
            }
            match result {
                Ok(response) => {
                    completed.set(true);
                    let pointer_failed = response.pointer_update_error.is_some();
                    let updated = apply_campaign_response_pointer_mutation(
                        &listing,
                        &response,
                        true,
                        remove_pointer,
                    );
                    if pointer_failed && !response.root_event_id.trim().is_empty() {
                        pointer_retry.set(Some((response.root_event_id.clone(), true)));
                    }
                    message.set(Some(response.pointer_update_error.map(|problem| format!("Promotion cancelled, but Promotion link cleanup failed: {problem}. Cancellation remains authoritative and the link can be retried.")).unwrap_or_else(|| "Promotion cancelled. New claims stop; prior claims remain valid because campaign cancellation does not revoke them.".into())));
                    submitting.set(false);
                    if !pointer_failed {
                        on_saved.run(updated.unwrap_or(listing));
                    }
                }
                Err(problem) => {
                    error.set(Some(problem));
                    submitting.set(false);
                }
            }
        });
    });
    let retry_pointer = Callback::new(move |()| {
        if submitting.get_untracked() {
            return;
        }
        let Some((root_event_id, remove)) = pointer_retry.get_untracked() else {
            return;
        };
        let Some(publisher_npub) = auth_for_retry.npub.get() else {
            error.set(Some("Authenticate as the developer first".into()));
            return;
        };
        if publisher_npub != listing_for_retry.publisher_npub {
            error.set(Some(
                "Switch to the developer account that published this game".into(),
            ));
            return;
        }
        operation_generation.update(|value| *value = value.wrapping_add(1));
        let request_generation = operation_generation.get_untracked();
        let request = UpdateCampaignPointerRequest {
            publisher_npub: publisher_npub.clone(),
            listing_id: listing_for_retry.id.clone(),
            campaign_root_id: root_event_id.clone(),
            remove,
        };
        let auth_for_request = auth_for_retry.clone();
        let listing = listing_for_retry.clone();
        let on_saved = on_saved_for_retry.clone();
        submitting.set(true);
        error.set(None);
        spawn_local(async move {
            let result = invoke_update_campaign_pointer(request).await;
            if !accepts_account_response(
                auth_for_request.npub.get_untracked().as_deref(),
                &publisher_npub,
                operation_generation.get_untracked(),
                request_generation,
            ) {
                submitting.set(false);
                completed.set(true);
                error.set(Some("Account changed while the Promotion link was updating. The stale response was ignored; return to the Game page and refresh.".into()));
                return;
            }
            match result {
                Ok(listing_event_id) => {
                    pointer_retry.set(None);
                    submitting.set(false);
                    message.set(Some(
                        "Promotion link updated without republishing the Promotion.".into(),
                    ));
                    on_saved.run(apply_campaign_pointer_mutation(
                        &listing,
                        &root_event_id,
                        &listing_event_id,
                        remove,
                    ));
                }
                Err(problem) => {
                    submitting.set(false);
                    message.set(Some(format!("Promotion link retry failed: {problem}. The Promotion remains valid; retry again from this page.")));
                }
            }
        });
    });
    let cancel_after_confirmation = cancel_with_pointer.clone();
    let back_after_confirmation = on_back.clone();
    let on_confirmation_decision = Callback::new(move |accepted: Option<bool>| {
        let Some(current) = confirmation.get_untracked() else {
            return;
        };
        match resolve_confirmation(current, accepted, initially_points_to_campaign) {
            ConfirmationOutcome::Close => confirmation.set(None),
            ConfirmationOutcome::PromptRemovePointer => {
                confirmation.set(Some(CampaignConfirmation::RemovePointer));
            }
            ConfirmationOutcome::CancelCampaign(remove_pointer) => {
                confirmation.set(None);
                cancel_after_confirmation.run(remove_pointer);
            }
            ConfirmationOutcome::DiscardChanges => {
                confirmation.set(None);
                back_after_confirmation.run(());
            }
        }
    });
    let cancel = move |_| {
        if can_request_campaign_confirmation(submitting.get_untracked(), completed.get_untracked())
        {
            confirmation.set(Some(CampaignConfirmation::CancelCampaign));
        }
    };
    view! {
        <section class="v2-publisher-studio v2-publisher-editor">
            <button class="v2-btn-secondary v2-publisher-back" on:click=move |_| back.run(())>"Back to Game page"</button>
            <header class="v2-publisher-game-hero">
                {valid_cover_url(&listing.images).map(|url| view! { <img src=url alt="cover" class="h-20 w-16 rounded-lg object-cover" on:error=use_fallback_cover /> }.into_any()).unwrap_or_else(|| view! { <div class="h-20 w-16 rounded-lg bg-surface-container-highest flex items-center justify-center text-2xl">"🎮"</div> }.into_any())}
                <div><p class="v2-publisher-kicker">{if editing { "Promotion details" } else { "New Promotion" }}</p><h1>{format!("{} for {}", if editing { "Promotion" } else { "Create a Promotion" }, listing.title)}</h1>{campaign.as_ref().map(|item| view! { <span class="v2-chip">{campaign_status(&item.classification)}</span> })}</div>
            </header>
            <div class="v2-publisher-management-layout">
            <main class="v2-publisher-main">
            <section class="v2-publisher-panel v2-publisher-form">
                <div class="v2-publisher-authority" role="note"><strong>"Developer-only authority"</strong><p>"Only the developer account that published this game can create, edit, or cancel a Promotion. A fulfillment provider cannot perform these actions."</p></div>
                {terms_read_only.then(|| view! { <p class="v2-publisher-readonly" role="status">"Active, Ended, and Cancelled Promotion terms are immutable. This view is read-only."</p> })}
                <div><label for="campaign-id">"Promotion ID"</label><input id="campaign-id" class="v2-input" readonly=true prop:value=move || form.get().campaign_id /></div>
                <div><h2>"Claim and keep"</h2><div class="v2-publisher-option"><strong>"Free Claim and keep"</strong><p>"People may claim before the exclusive end time and keep durable access permanently."</p></div><p class="text-sm text-on-surface-variant">"Timed access belongs to the Game page acquisition policy, not this Promotion."</p></div>
                <div class="v2-publisher-date-grid"><div><label for="campaign-start">"Start date and time (required)"</label><input id="campaign-start" required=true class="v2-input" type="datetime-local" disabled=move || terms_read_only || completed.get() prop:value=move || form.get().starts_at on:input:target=move |event| form.update(|current| current.starts_at = event.target().value()) /></div><div><label for="campaign-end">"Exclusive end date and time (required)"</label><input id="campaign-end" required=true class="v2-input" type="datetime-local" disabled=move || terms_read_only || completed.get() prop:value=move || form.get().ends_at on:input:target=move |event| form.update(|current| current.ends_at = event.target().value()) /></div></div>
                <p class="text-xs text-on-surface-variant">"Times use your local timezone. Claims at or after the end are not accepted. Local timezone: "{timezone_label()}</p>
                {move || live_validation.get().map(|text| view! { <p class="text-sm text-error" role="alert">{text}</p> })}
                <div class="v2-publisher-link-option"><label><input type="checkbox" disabled=move || terms_read_only || completed.get() prop:checked=move || form.get().update_listing_pointer on:change:target=move |event| form.update(|current| current.update_listing_pointer = event.target().checked()) /><span><strong>"Add a Promotion link to the Game page"</strong><span>"Recommended advisory discovery hint. Promotion validity never depends on this link."</span></span></label></div>
                {move || error.get().map(|text| view! { <p class="text-error" role="alert">{text}</p> })}
                {move || message.get().map(|text| view! { <p class="text-secondary" role="status">{text}</p> })}
                <div class="v2-publisher-actions v2-publisher-actions-end"><button class="v2-btn-secondary" on:click=move |_| back.run(())>{move || if terms_read_only { "Close" } else if completed.get() { "Back to Game page" } else { "Discard changes" }}</button>{move || completed.get().then(|| view! { <span class="v2-chip">"Completed"</span> })}{move || { let retry = retry_pointer.clone(); pointer_retry.get().is_some().then(move || view! { <button class="v2-btn-secondary" disabled=move || submitting.get() on:click=move |_| retry.run(())>"Retry Promotion link"</button> }) }}{if cancellable { view! { <button class="v2-btn-secondary" disabled=move || submitting.get() || completed.get() on:click=cancel>"Cancel Promotion"</button> }.into_any() } else { view! { <></> }.into_any() }}{if !terms_read_only { view! { <button class="v2-btn-primary" disabled=move || submitting.get() || completed.get() || live_validation.get().is_some() on:click=save>{move || if completed.get() { "Completed" } else if submitting.get() { "Publishing..." } else { "Publish Promotion" }}</button> }.into_any() } else { view! { <></> }.into_any() }}</div>
            </section>
            </main>
            <aside class="v2-publisher-panel v2-publisher-sidebar"><h2>"Promotion policy"</h2><ul><li>"Developer account controls publication and cancellation."</li><li>"End time is exclusive and shown in your local timezone."</li><li>"Claims create durable access."</li><li>"Campaign cancellation stops new claims without revoking prior claims."</li><li>"Promotion links are advisory and retryable."</li></ul><details class="v2-publisher-diagnostics"><summary>"Protocol diagnostics"</summary><p>"Campaign chain events are validated independently from listing pointer events."</p></details></aside>
            </div>
            <CampaignConfirmationDialog confirmation=confirmation on_decision=on_confirmation_decision />
        </section>
    }
}

fn access_label(policy: &AcquisitionPolicy) -> String {
    match policy {
        AcquisitionPolicy::Public => "Access: Public".into(),
        AcquisitionPolicy::Gated => "Access: Paid/gated".into(),
        AcquisitionPolicy::TimedAccess { .. } => "Access: Timed".into(),
    }
}
fn version_label(listing: &GameListing) -> String {
    listing
        .specs
        .iter()
        .find(|(key, _)| key == "version")
        .map(|(_, value)| format!("Version: {value}"))
        .unwrap_or_else(|| "Version: Unspecified".into())
}
fn fulfillment_label(listing: &GameListing) -> String {
    let fulfillment_key = listing
        .specs
        .iter()
        .find(|(key, _)| key == "fulfillment_pubkey")
        .map(|(_, value)| value);
    match fulfillment_key {
        Some(key) if publisher_hex(&listing.publisher_npub).as_deref() == Some(key.as_str()) => {
            "ADP: Direct fulfillment".into()
        }
        Some(_) => "ADP: Delegated fulfillment".into(),
        None => "ADP: Not configured".into(),
    }
}
fn adp_server_label(listing: &GameListing) -> String {
    let spec_server = listing
        .specs
        .iter()
        .find(|(key, _)| key == "server")
        .map(|(_, value)| value.as_str());
    let value = spec_server.unwrap_or(listing.download_url.trim());
    if value.is_empty() {
        return "Not configured".into();
    }
    value
        .split_once("://")
        .and_then(|(scheme, rest)| {
            rest.split('/')
                .next()
                .map(|host| format!("{scheme}://{host}"))
        })
        .unwrap_or_else(|| value.to_string())
}
fn publisher_hex(npub: &str) -> Option<String> {
    use nostr::nips::nip19::FromBech32;

    nostr::PublicKey::from_bech32(npub)
        .ok()
        .map(|key| key.to_hex())
}
fn validation_message(error: CampaignValidationError) -> String {
    match error {
        CampaignValidationError::MissingCampaignId => "Promotion ID is required".into(),
        CampaignValidationError::MissingStart => "Choose a start date and time".into(),
        CampaignValidationError::MissingEnd => "Choose an end date and time".into(),
        CampaignValidationError::InvalidStart => "Start date is invalid".into(),
        CampaignValidationError::InvalidEnd => "End date is invalid".into(),
        CampaignValidationError::EndMustFollowStart => {
            "End date must be after the start date".into()
        }
        CampaignValidationError::UnsupportedCampaignType => {
            "This Promotion type is not currently supported".into()
        }
    }
}
fn current_unix_secs() -> u64 {
    (js_sys::Date::now() / 1000.0) as u64
}
fn random_campaign_suffix() -> String {
    format!("{:06x}", (js_sys::Math::random() * 16_777_215.0) as u32)
}
fn format_unix(value: u64) -> String {
    datetime_local(value).replace('T', " ")
}
fn datetime_local(value: u64) -> String {
    let date = js_sys::Date::new(&(value as f64 * 1000.0).into());
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}",
        date.get_full_year(),
        date.get_month() + 1,
        date.get_date(),
        date.get_hours(),
        date.get_minutes()
    )
}
fn timezone_label() -> String {
    js_sys::Date::new_0().to_string().into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cancellation_with_pointer_prompts_for_cleanup() {
        assert_eq!(
            resolve_confirmation(CampaignConfirmation::CancelCampaign, Some(true), true,),
            ConfirmationOutcome::PromptRemovePointer
        );
    }

    #[test]
    fn declining_pointer_cleanup_still_cancels_campaign() {
        assert_eq!(
            resolve_confirmation(CampaignConfirmation::RemovePointer, Some(false), true,),
            ConfirmationOutcome::CancelCampaign(false)
        );
    }

    #[test]
    fn dismissing_confirmation_does_not_take_action() {
        assert_eq!(
            resolve_confirmation(CampaignConfirmation::DiscardChanges, None, false),
            ConfirmationOutcome::Close
        );
    }

    #[test]
    fn accepted_discard_leaves_editor() {
        assert_eq!(
            resolve_confirmation(CampaignConfirmation::DiscardChanges, Some(true), false,),
            ConfirmationOutcome::DiscardChanges
        );
    }

    #[test]
    fn cancellation_without_pointer_does_not_prompt_for_cleanup() {
        assert_eq!(
            resolve_confirmation(CampaignConfirmation::CancelCampaign, Some(true), false,),
            ConfirmationOutcome::CancelCampaign(false)
        );
    }

    #[test]
    fn cancellation_confirmation_explains_claim_durability() {
        let message = CampaignConfirmation::CancelCampaign.message();
        assert!(message.contains("New claims stop"));
        assert!(message.contains("Prior claims remain valid"));
        assert!(message.contains("does not revoke prior claims"));
    }

    #[test]
    fn publisher_studio_never_uses_window_confirm() {
        let source = include_str!("publish.rs");
        assert!(!source.contains(&["window.", "confirm"].concat()));
        assert!(!source.contains(&["window().", "confirm"].concat()));
    }
}
