use leptos::prelude::*;
use std::collections::HashMap;
use wasm_bindgen_futures::spawn_local;

use crate::campaign_management::{
    apply_campaign_pointer_mutation, apply_campaign_response_pointer_mutation,
    build_campaign_request, build_cancel_request, campaign_pointer_update_plan, campaign_status,
    current_user_listings, generated_campaign_id, listing_coordinate, validate_campaign_form,
    CampaignForm, CampaignPointerUpdatePlan, CampaignValidationError,
};
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
                    <button class="v2-btn-secondary" on:click={let on_navigate = on_navigate.clone(); move |_| on_navigate.run(PublishViewState::Game(listing_for_back.clone()))}>"Back to game"</button>
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

    let refresh = move || {
        let Some(publisher) = auth.npub.get() else {
            listings.set(Vec::new());
            loading.set(false);
            error.set(Some("Authenticate as the publisher to manage games".into()));
            return;
        };
        loading.set(true);
        error.set(None);
        let listings_signal = listings;
        spawn_local(async move {
            let received = RwSignal::new(Vec::<GameListing>::new());
            let received_for_listing = received;
            let received_for_complete = received;
            let publisher_for_complete = publisher.clone();
            match invoke_fetch_marketplace_stream(
                100,
                Some(3650),
                None,
                move |listing| received_for_listing.update(|items| items.push(listing)),
                Some(move || {
                    let items = received_for_complete.get_untracked();
                    listings_signal.set(current_user_listings(items, &publisher_for_complete));
                    loading.set(false);
                }),
            )
            .await
            {
                Ok((product_cleanup, completion_cleanup)) => {
                    product_cleanup();
                    completion_cleanup();
                    if loading.get_untracked() {
                        listings_signal
                            .set(current_user_listings(received.get_untracked(), &publisher));
                        loading.set(false);
                    }
                }
                Err(fetch_error) => {
                    error.set(Some(fetch_error));
                    loading.set(false);
                }
            }
        });
    };

    Effect::new(move |_| refresh());
    Effect::new(move |_| {
        let items = listings.get();
        let Some(publisher_npub) = auth.npub.get() else {
            return;
        };
        if items.is_empty() {
            return;
        }
        campaign_counts_loading.set(true);
        campaign_counts_error.set(false);
        let request = DiscoverCampaignSummariesRequest {
            publisher_npub,
            listings: items
                .into_iter()
                .map(|listing| CampaignSummaryListingInput {
                    listing_id: listing.id,
                })
                .collect(),
        };
        spawn_local(async move {
            match invoke_discover_campaign_summaries(request).await {
                Ok(summaries) => campaign_counts.set(
                    summaries
                        .into_iter()
                        .map(|summary| (summary.listing_id, (summary.active, summary.upcoming)))
                        .collect(),
                ),
                Err(_) => campaign_counts_error.set(true),
            }
            campaign_counts_loading.set(false);
        });
    });

    view! {
        <section class="space-y-8">
            <header class="flex flex-col gap-4 md:flex-row md:items-end md:justify-between">
                <div>
                    <p class="text-xs font-bold uppercase tracking-[0.25em] text-primary">"Publisher studio"</p>
                    <h1 class="text-4xl font-headline font-bold tracking-tight">"Published games"</h1>
                    <p class="text-on-surface-variant mt-2">"Manage your listings, distribution, and claim campaigns."</p>
                </div>
                <div class="flex gap-3">
                    <button class="v2-btn-secondary" on:click=move |_| refresh() disabled=move || loading.get()>"Refresh"</button>
                    <button class="v2-btn-primary" on:click=move |_| on_navigate.run(PublishViewState::NewPublication)>"New publication"</button>
                </div>
            </header>
            {move || error.get().map(|message| view! { <div class="v2-panel border border-error/40 text-error">{message}</div> })}
            {move || if loading.get() {
                view! { <div class="v2-panel text-on-surface-variant">"Loading your published games..."</div> }.into_any()
            } else if listings.get().is_empty() {
                view! {
                    <div class="v2-panel text-center space-y-4">
                        <h2 class="text-2xl font-headline font-bold">"No published games yet"</h2>
                        <p class="text-on-surface-variant">"Publish a listing first, then campaigns can be managed from that game."</p>
                        <button class="v2-btn-primary" on:click=move |_| on_navigate.run(PublishViewState::NewPublication)>"Open publishing form"</button>
                    </div>
                }.into_any()
            } else {
                view! {
                    <div class="grid gap-5 lg:grid-cols-2">
                        {move || listings.get().into_iter().map(|listing| {
                            let selected = listing.clone();
                            let cover = valid_cover_url(&listing.images);
                            let listing_id_for_counts = listing.id.clone();
                            view! {
                                <article class="v2-panel flex min-w-0 max-w-full gap-5 overflow-hidden">
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
                                                format!("Campaigns: {active} active · {upcoming} upcoming")
                                            } else if campaign_counts_error.get() {
                                                "Campaign counts unavailable".into()
                                            } else if campaign_counts_loading.get() {
                                                "Campaigns: checking...".into()
                                            } else {
                                                "Campaigns: 0 active · 0 upcoming".into()
                                            }}
                                        </p>
                                        <button class="v2-btn-primary mt-4" on:click=move |_| on_navigate.run(PublishViewState::Game(selected.clone()))>"Manage publication"</button>
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
    Effect::new(move |_| {
        let request = DiscoverCampaignsRequest {
            publisher_npub: publisher.clone(),
            listing_id: listing_id.clone(),
            pointers: pointers.clone(),
        };
        spawn_local(async move {
            match invoke_discover_campaigns(request).await {
                Ok(found) => campaigns.set(found),
                Err(message) => error.set(Some(message)),
            }
            loading.set(false);
        });
    });

    view! {
        <section class="space-y-8">
            <button class="v2-btn-secondary" on:click=move |_| on_back.run(())>"Back to published games"</button>
            <header class="v2-panel flex flex-col gap-5 md:flex-row md:items-center">
                {valid_cover_url(&listing.images).map(|url| view! { <img src=url alt="cover" class="h-32 w-24 rounded-xl object-cover" on:error=use_fallback_cover /> }.into_any()).unwrap_or_else(|| view! { <div class="h-32 w-24 rounded-xl bg-surface-container-highest flex items-center justify-center text-3xl">"🎮"</div> }.into_any())}
                <div class="flex-1">
                    <p class="text-xs uppercase tracking-[0.2em] text-primary">"Manage publication"</p>
                    <h1 class="text-3xl font-headline font-bold">{listing.title.clone()}</h1>
                    <p class="text-sm text-on-surface-variant break-all">{listing_coordinate(&listing)}</p>
                    <div class="mt-4 flex flex-wrap gap-2 text-sm">
                        <span class="v2-chip">{format!("{} sats", listing.price_sats)}</span>
                        <span class="v2-chip">{access_label(&listing.acquisition)}</span>
                        <span class="v2-chip">{version_label(&listing)}</span>
                    </div>
                </div>
                <button class="v2-btn-secondary" on:click=move |_| on_navigate_for_edit.run(PublishViewState::EditPublication(listing_for_edit.clone()))>"Edit publication"</button>
            </header>
            <section class="v2-panel">
                <h2 class="text-xl font-headline font-bold mb-4">"Publication details"</h2>
                <dl class="grid gap-4 sm:grid-cols-2 lg:grid-cols-3 text-sm">
                    <div><dt class="text-on-surface-variant">"Published"</dt><dd>{format_unix(listing.created_at)}</dd></div>
                    <div><dt class="text-on-surface-variant">"Platforms"</dt><dd>{if listing.platforms.is_empty() { "Unspecified".into() } else { listing.platforms.join(", ") }}</dd></div>
                    <div><dt class="text-on-surface-variant">"ADP fulfillment"</dt><dd>{fulfillment_label(&listing)}</dd></div>
                    <div><dt class="text-on-surface-variant">"ADP server"</dt><dd class="break-all">{adp_server_label(&listing)}</dd></div>
                    <div><dt class="text-on-surface-variant">"Campaign pointers"</dt><dd>{listing.campaigns.len()}</dd></div>
                    <div><dt class="text-on-surface-variant">"Listing event"</dt><dd class="break-all">{listing.event_id.clone().unwrap_or_else(|| "Unavailable".into())}</dd></div>
                </dl>
            </section>
            <section class="v2-panel space-y-4">
                <div class="flex flex-col gap-3 md:flex-row md:items-center md:justify-between">
                    <div><h2 class="text-2xl font-headline font-bold">"Campaigns"</h2><p class="text-sm text-on-surface-variant">"Free claims create permanent grants. Listing pointers are advisory discovery hints."</p></div>
                    <button class="v2-btn-primary" on:click=move |_| on_navigate.run(PublishViewState::Campaign { listing: listing_for_button.clone(), campaign: None })>"New campaign"</button>
                </div>
                {move || error.get().map(|message| view! { <p class="text-error">{format!("Campaign discovery failed: {message}")}</p> })}
                {move || if loading.get() { view! { <p class="text-on-surface-variant">"Discovering campaigns..."</p> }.into_any() } else if campaigns.get().is_empty() { view! { <p class="text-on-surface-variant">"No valid campaigns found. Discovery checks listing pointers and relay search."</p> }.into_any() } else { let selected_listing = listing_for_effect.clone(); let navigate = on_navigate.clone(); view! { <div class="space-y-3">{campaigns.get().into_iter().map(|campaign| campaign_row(campaign, selected_listing.clone(), navigate.clone())).collect_view()}</div> }.into_any() }}
            </section>
            <section class="v2-panel grid gap-3 md:grid-cols-2">
                <div><h3 class="font-bold">"Distribution"</h3><p class="text-sm text-on-surface-variant">{format!("Platforms: {}", if listing.platforms.is_empty() { "Unspecified".into() } else { listing.platforms.join(", ") })}</p></div>
                <div><h3 class="font-bold">"Acquisition policy"</h3><p class="text-sm text-on-surface-variant">{access_label(&listing.acquisition)}</p><p class="text-xs text-on-surface-variant mt-1">"Timed access is configured here as a listing policy, not a claim campaign."</p></div>
            </section>
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
            Self::CancelCampaign => "Cancel campaign?",
            Self::RemovePointer => "Remove listing pointer?",
            Self::DiscardChanges => "Discard unsaved changes?",
        }
    }

    fn message(self) -> &'static str {
        match self {
            Self::CancelCampaign => {
                "This prevents new claims immediately. Entitlements already issued remain valid."
            }
            Self::RemovePointer => {
                "The campaign cancellation remains authoritative either way. Remove its advisory pointer from the game listing too?"
            }
            Self::DiscardChanges => "Your unsaved campaign changes will be lost.",
        }
    }

    fn reject_label(self) -> &'static str {
        match self {
            Self::CancelCampaign => "Keep campaign",
            Self::RemovePointer => "Keep pointer",
            Self::DiscardChanges => "Keep editing",
        }
    }

    fn accept_label(self) -> &'static str {
        match self {
            Self::CancelCampaign => "Cancel campaign",
            Self::RemovePointer => "Remove pointer",
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
    Effect::new(move |_| {
        let Some(dialog) = dialog_ref.get() else {
            return;
        };
        if confirmation.get().is_some() {
            if dialog.open() {
                dialog.close();
            }
            let _ = dialog.show_modal();
        } else if dialog.open() {
            dialog.close();
        }
    });

    view! {
        <dialog
            node_ref=dialog_ref
            class="m-auto w-full max-w-md bg-transparent p-4 text-on-surface backdrop:bg-black/70"
            aria-label=move || confirmation.get().map(CampaignConfirmation::title).unwrap_or_default()
            aria-description=move || confirmation.get().map(CampaignConfirmation::message).unwrap_or_default()
            on:cancel=move |event: web_sys::Event| {
                event.prevent_default();
                on_decision.run(None);
            }
            on:click=move |_| on_decision.run(None)
        >
            <section
                class="v2-panel space-y-5 border border-outline-variant/40 shadow-2xl"
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
                    <button class="v2-btn-secondary" on:click=move |_| on_decision.run(Some(false))>
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
    let action_in_progress = RwSignal::new(false);
    let action_completed = RwSignal::new(false);
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
        let request = UpdateCampaignPointerRequest {
            publisher_npub,
            listing_id: pointer_listing.id.clone(),
            campaign_root_id: pointer_campaign.root_event_id.clone(),
            remove,
        };
        let listing = pointer_listing.clone();
        let root_event_id = pointer_campaign.root_event_id.clone();
        let navigate = pointer_navigate.clone();
        action_in_progress.set(true);
        spawn_local(async move {
            match invoke_update_campaign_pointer(request).await {
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
        let Some(predecessor) = cancel_campaign.event_id.clone() else {
            cancel_message.set(Some("Campaign tip is unavailable".into()));
            return;
        };
        let request = build_cancel_request(
            publisher_npub,
            cancel_listing.id.clone(),
            cancel_campaign.campaign_id.clone(),
            predecessor,
            remove_pointer,
        );
        let listing = cancel_listing.clone();
        let navigate = cancel_navigate.clone();
        action_in_progress.set(true);
        spawn_local(async move {
            match invoke_publish_campaign(request).await {
                Ok(response) => {
                    action_completed.set(true);
                    let updated = apply_campaign_response_pointer_mutation(
                        &listing,
                        &response,
                        true,
                        remove_pointer,
                    );
                    cancel_message.set(Some(
                        response
                            .pointer_update_error
                            .map(|error| format!("Cancelled, but listing cleanup failed: {error}"))
                            .unwrap_or_else(|| {
                                "Campaign cancelled. Existing grants remain valid.".into()
                            }),
                    ));
                    if let Some(updated) = updated {
                        navigate.run(PublishViewState::Game(updated));
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
    let on_cancel = move |_| confirmation.set(Some(CampaignConfirmation::CancelCampaign));
    view! {
        <article class="rounded-2xl bg-surface-container-highest/70 p-4 flex flex-col gap-4 md:flex-row md:items-center md:justify-between">
            <div>
                <div class="flex flex-wrap items-center gap-2"><strong>{campaign.campaign_id.clone()}</strong><span class="v2-chip">{status}</span>{if points_here { view! { <span class="v2-chip">"Listing pointer"</span> }.into_any() } else { view! { <span class="text-xs text-on-surface-variant">"No listing pointer"</span> }.into_any() }}</div>
                <p class="text-sm text-on-surface-variant mt-2">{format!("{} to {}", format_unix(campaign.starts_at), format_unix(campaign.ends_at))}</p>
                <details class="mt-2 text-xs text-on-surface-variant"><summary>"Technical details"</summary><p class="break-all mt-1">{format!("root: {} | event: {}", campaign.root_event_id, campaign.event_id.clone().unwrap_or_default())}</p></details>
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
                {if !points_here && (is_upcoming || is_active) { view! { <button class="v2-btn-secondary" disabled=move || action_in_progress.get() || action_completed.get() on:click=move |_| on_pointer_update.run(false)>"Add listing pointer"</button> }.into_any() } else if points_here && !is_upcoming && !is_active { view! { <button class="v2-btn-secondary" disabled=move || action_in_progress.get() || action_completed.get() on:click=move |_| on_pointer_update.run(true)>"Remove stale pointer"</button> }.into_any() } else { view! { <></> }.into_any() }}
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
        if current.starts_at.is_empty() || current.ends_at.is_empty() {
            None
        } else {
            validate_campaign_form(&current)
                .err()
                .map(validation_message)
        }
    });
    let submitting = RwSignal::new(false);
    let completed = RwSignal::new(false);
    let message = RwSignal::new(None::<String>);
    let error = RwSignal::new(None::<String>);
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
    let on_saved_for_cancel = on_saved.clone();
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
        let current = form.get();
        let pointer_plan = campaign_pointer_update_plan(
            initially_points_to_campaign,
            current.update_listing_pointer,
        );
        let request = match build_campaign_request(
            publisher_npub,
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
        spawn_local(async move {
            match invoke_publish_campaign(request).await {
                Ok(response) => {
                    completed.set(true);
                    if let Some(pointer_error) = response.pointer_update_error.as_ref() {
                        message.set(Some(format!("Campaign published successfully, but the listing pointer could not be updated: {pointer_error}. The campaign remains valid and discoverable through relay search.")));
                        submitting.set(false);
                        return;
                    }

                    if matches!(
                        pointer_plan,
                        CampaignPointerUpdatePlan::RemoveAfterCampaignPublish
                    ) {
                        let root_event_id = response.root_event_id.clone();
                        let removal_request = UpdateCampaignPointerRequest {
                            publisher_npub: publisher_for_pointer_removal,
                            listing_id: listing_after_save.id.clone(),
                            campaign_root_id: root_event_id.clone(),
                            remove: true,
                        };
                        match invoke_update_campaign_pointer(removal_request).await {
                            Ok(listing_event_id) => {
                                let updated = apply_campaign_pointer_mutation(
                                    &listing_after_save,
                                    &root_event_id,
                                    &listing_event_id,
                                    true,
                                );
                                message.set(Some("Campaign published successfully and the listing pointer was removed.".into()));
                                submitting.set(false);
                                on_saved.run(updated);
                            }
                            Err(problem) => {
                                message.set(Some(format!("Campaign published successfully, but the listing pointer could not be removed: {problem}. The campaign update remains authoritative; use Back to reconcile from relays.")));
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
                    message.set(Some("Campaign published successfully.".into()));
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
        let Some(tip) = campaign.event_id else {
            error.set(Some("Campaign tip is unavailable".into()));
            return;
        };
        let request = build_cancel_request(
            publisher_npub,
            listing_for_cancel.id.clone(),
            campaign.campaign_id,
            tip,
            remove_pointer,
        );
        submitting.set(true);
        error.set(None);
        let listing = listing_for_cancel.clone();
        let on_saved = on_saved_for_cancel.clone();
        spawn_local(async move {
            match invoke_publish_campaign(request).await {
                Ok(response) => {
                    completed.set(true);
                    let pointer_failed = response.pointer_update_error.is_some();
                    let updated = apply_campaign_response_pointer_mutation(
                        &listing,
                        &response,
                        true,
                        remove_pointer,
                    );
                    message.set(Some(response.pointer_update_error.map(|problem| format!("Campaign cancelled, but listing pointer cleanup failed: {problem}. Cancellation remains authoritative.")).unwrap_or_else(|| "Campaign cancelled. Existing grants remain valid.".into())));
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
    let cancel = move |_| confirmation.set(Some(CampaignConfirmation::CancelCampaign));
    view! {
        <section class="max-w-3xl mx-auto space-y-6">
            <button class="v2-btn-secondary" on:click=move |_| back.run(())>"Back to game"</button>
            <header class="v2-panel flex gap-4 items-center">
                {valid_cover_url(&listing.images).map(|url| view! { <img src=url alt="cover" class="h-20 w-16 rounded-lg object-cover" on:error=use_fallback_cover /> }.into_any()).unwrap_or_else(|| view! { <div class="h-20 w-16 rounded-lg bg-surface-container-highest flex items-center justify-center text-2xl">"🎮"</div> }.into_any())}
                <div><p class="text-xs uppercase tracking-[0.2em] text-primary">{if editing { "Edit campaign" } else { "New campaign" }}</p><h1 class="text-3xl font-headline font-bold">{format!("{} for {}", if editing { "Edit campaign" } else { "New campaign" }, listing.title)}</h1></div>
            </header>
            <section class="v2-panel space-y-6">
                <div><label class="block text-sm font-bold mb-2" for="campaign-id">"Campaign ID"</label><input id="campaign-id" class="v2-input" readonly=true prop:value=move || form.get().campaign_id /></div>
                <div><h2 class="text-lg font-bold mb-3">"Campaign type"</h2><div class="rounded-xl border border-primary/40 bg-primary/10 p-4"><strong>"Free claim"</strong><p class="text-sm text-on-surface-variant">"Users can claim during the campaign and keep the game permanently."</p></div><div class="mt-3 rounded-xl border border-outline-variant/30 p-4 opacity-60"><strong>"Discounted price"</strong><p class="text-sm text-on-surface-variant">"Not supported by the current campaign protocol."</p></div><p class="text-xs text-on-surface-variant mt-2">"Timed access is a listing acquisition policy, not a claim-campaign type."</p></div>
                <div class="grid gap-4 md:grid-cols-2"><div><label class="block text-sm font-bold mb-2" for="campaign-start">"Start date and time"</label><input id="campaign-start" class="v2-input" type="datetime-local" disabled=move || terms_read_only || completed.get() prop:value=move || form.get().starts_at on:input:target=move |event| form.update(|current| current.starts_at = event.target().value()) /></div><div><label class="block text-sm font-bold mb-2" for="campaign-end">"End date and time"</label><input id="campaign-end" class="v2-input" type="datetime-local" disabled=move || terms_read_only || completed.get() prop:value=move || form.get().ends_at on:input:target=move |event| form.update(|current| current.ends_at = event.target().value()) /></div></div>
                <p class="text-xs text-on-surface-variant">"Times use your local timezone: "{timezone_label()}</p>
                {move || live_validation.get().map(|text| view! { <p class="text-sm text-error" role="alert">{text}</p> })}
                <div class="rounded-xl bg-surface-container-highest p-4"><label class="flex gap-3 items-start"><input type="checkbox" disabled=move || terms_read_only || completed.get() prop:checked=move || form.get().update_listing_pointer on:change:target=move |event| form.update(|current| current.update_listing_pointer = event.target().checked()) /><span><strong>"Add this campaign to the game listing"</strong><span class="block text-sm text-on-surface-variant">"Recommended discovery hint. Campaign validity does not depend on this pointer."</span></span></label></div>
                {move || error.get().map(|text| view! { <p class="text-error">{text}</p> })}
                {move || message.get().map(|text| view! { <p class="text-secondary">{text}</p> })}
                <div class="flex flex-wrap gap-3 justify-end"><button class="v2-btn-secondary" on:click=move |_| back.run(())>{move || if terms_read_only { "Close" } else if completed.get() { "Back to game" } else { "Discard changes" }}</button>{move || completed.get().then(|| view! { <span class="v2-chip">"Completed"</span> })}{if cancellable { view! { <button class="v2-btn-secondary" disabled=move || submitting.get() || completed.get() on:click=cancel>"Cancel campaign"</button> }.into_any() } else { view! { <></> }.into_any() }}{if !terms_read_only { view! { <button class="v2-btn-primary" disabled=move || submitting.get() || completed.get() || live_validation.get().is_some() on:click=save>{move || if completed.get() { "Completed" } else if submitting.get() { "Publishing..." } else { "Save campaign" }}</button> }.into_any() } else { view! { <></> }.into_any() }}</div>
            </section>
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
        CampaignValidationError::MissingCampaignId => "Campaign ID is required".into(),
        CampaignValidationError::MissingStart => "Choose a start date and time".into(),
        CampaignValidationError::MissingEnd => "Choose an end date and time".into(),
        CampaignValidationError::InvalidStart => "Start date is invalid".into(),
        CampaignValidationError::InvalidEnd => "End date is invalid".into(),
        CampaignValidationError::EndMustFollowStart => {
            "End date must be after the start date".into()
        }
        CampaignValidationError::UnsupportedCampaignType => {
            "This campaign type is not supported by the current protocol".into()
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
}
