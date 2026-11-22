use arcadestr_core::distribution::Sha256Digest;
use leptos::prelude::*;
use std::collections::{HashMap, HashSet};
use tracing::warn;
use wasm_bindgen_futures::spawn_local;

#[cfg(not(feature = "web"))]
use crate::campaign_management::listing_coordinate;
use crate::campaign_management::{
    accepts_account_response, apply_campaign_pointer_mutation,
    apply_campaign_response_pointer_mutation, build_campaign_request, build_cancel_request,
    campaign_pointer_failure_retryable, campaign_pointer_update_plan, campaign_status,
    can_request_campaign_confirmation, current_user_listings, generated_campaign_id,
    validate_campaign_form, CampaignForm, CampaignPointerUpdatePlan, CampaignValidationError,
};
use crate::components::DateTimeRangePicker;
#[cfg(not(feature = "web"))]
use crate::components::PublishView;
use crate::invoke_fetch_marketplace_stream;
use crate::models::{
    AcquisitionPolicy, GameListing, ListingSource, StorePageEnrichmentRequest,
    StorePageEnrichmentState, StorePageListingRef,
};
use crate::tauri_bridge::{
    invoke_discover_campaigns, invoke_enrich_store_pages, invoke_publish_campaign,
    invoke_update_campaign_pointer, CampaignPointerInput, DiscoverCampaignsRequest,
    DiscoveredCampaign, UpdateCampaignPointerRequest,
};
use crate::ui_v2::components::{
    artwork_state_from_url, ArtworkRole, Dialog, DialogCloseAction, DialogClosePolicy,
    DialogCloseRequest, DialogDismissal, DialogInitialFocus, DialogTone, DialogWidth,
    ErrorSeverity, ErrorState, GameArtwork, StatusChip, StatusChipSize, StatusChipVariant,
};
use crate::ui_v2::views::marketplace_loader::canonical_listing_coordinate;
use crate::ui_v2::views::store_page_publish::{
    publisher_store_page_dirty_coordinates, publisher_store_page_partial_coordinates,
};
#[cfg(not(feature = "web"))]
use crate::ui_v2::views::StorePageEditorView;
use crate::ui_v2::views::{use_fallback_cover, valid_cover_url};

pub use crate::components::HubSection;

/// Publisher routes. A game is either a local draft (`NewGame`) or a live game
/// (`Game`); both open the same hub, optionally on one of its sections.
#[derive(Clone, PartialEq)]
pub enum PublishViewState {
    Games,
    /// A game that has not been published yet. `None` starts a new draft.
    NewGame {
        draft_id: Option<String>,
    },
    Game {
        listing: GameListing,
        section: HubSection,
    },
    /// Advanced Store page tools (clone, link an existing page, associations).
    StorePage(GameListing),
    Campaign {
        listing: GameListing,
        campaign: Option<DiscoveredCampaign>,
    },
}

impl PublishViewState {
    /// The overview of a live game.
    pub fn game(listing: GameListing) -> Self {
        Self::Game {
            listing,
            section: HubSection::Overview,
        }
    }
}

/// A game draft that exists only on this device, listed on the dashboard.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct LocalGameDraft {
    pub draft_id: String,
    pub title: String,
    pub revision: u64,
}

/// Unpublished game drafts from the account's local drafts. Drafts of live
/// games (`listing:` ids) belong to that game, not to this list.
pub(crate) fn local_game_drafts(
    drafts: &[arcadestr_core::publisher_recovery::PublisherDraft],
) -> Vec<LocalGameDraft> {
    drafts
        .iter()
        .filter(|draft| draft.kind == "listing" && draft.draft_id.starts_with("local:"))
        .map(|draft| LocalGameDraft {
            draft_id: draft.draft_id.clone(),
            title: draft
                .payload
                .pointer("/form/title")
                .and_then(serde_json::Value::as_str)
                .map(str::trim)
                .filter(|title| !title.is_empty())
                .unwrap_or("Untitled game")
                .to_string(),
            revision: draft.revision,
        })
        .collect()
}

/// Live games with unpublished local changes, by listing id.
pub(crate) fn games_with_local_changes(
    drafts: &[arcadestr_core::publisher_recovery::PublisherDraft],
) -> HashSet<String> {
    drafts
        .iter()
        .filter(|draft| draft.kind == "listing")
        .filter_map(|draft| draft.draft_id.strip_prefix("listing:"))
        .map(str::to_string)
        .collect()
}

/// Signer availability for the active publisher account. Shared with the Create
/// Game workflow so both surfaces gate publication on the same authoritative state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PublisherSignerState {
    Available,
    Connecting,
    Unavailable,
    Unknown,
}

pub(crate) fn publisher_signer_state(
    publisher_npub: Option<&str>,
    active_account: Option<&crate::StoredAccount>,
    connection_status: &str,
) -> PublisherSignerState {
    let Some(publisher_npub) = publisher_npub else {
        return PublisherSignerState::Unavailable;
    };
    let Some(account) = active_account.filter(|account| account.npub == publisher_npub) else {
        return PublisherSignerState::Unknown;
    };
    match account.signing_mode.to_ascii_lowercase().as_str() {
        "local" | "nsec" | "debug" => PublisherSignerState::Available,
        "nip46" | "remote" if connection_status.eq_ignore_ascii_case("connected") => {
            PublisherSignerState::Available
        }
        "nip46" | "remote" if connection_status.eq_ignore_ascii_case("connecting") => {
            PublisherSignerState::Connecting
        }
        "nip46" | "remote" | "readonly" | "read_only" | "nip07" => {
            PublisherSignerState::Unavailable
        }
        _ => PublisherSignerState::Unknown,
    }
}

pub(crate) fn signer_can_publish(state: PublisherSignerState) -> bool {
    state == PublisherSignerState::Available
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PublisherDashboardState {
    SignedOut,
    Loading,
    Empty,
    Error,
    Partial,
    Ready,
}

fn publisher_dashboard_state(
    account_loading: bool,
    signed_in: bool,
    loading: bool,
    has_listings: bool,
    has_error: bool,
) -> PublisherDashboardState {
    if account_loading && !signed_in {
        PublisherDashboardState::Loading
    } else if !signed_in {
        PublisherDashboardState::SignedOut
    } else if loading && !has_listings {
        PublisherDashboardState::Loading
    } else if has_error && has_listings {
        PublisherDashboardState::Partial
    } else if has_error {
        PublisherDashboardState::Error
    } else if has_listings {
        PublisherDashboardState::Ready
    } else {
        PublisherDashboardState::Empty
    }
}

/// User-facing title and explanation for a failed publisher listing load.
/// Partial relay coverage never reaches here: it is a successful refresh.
fn publisher_load_error_copy(
    error: &arcadestr_core::marketplace_fetch::MarketplaceFetchError,
) -> (&'static str, &'static str) {
    if error.is_unreachable() {
        (
            "No relays responded",
            "Arcadestr could not reach any relay. Check your internet connection, then try again.",
        )
    } else {
        (
            "Your published games could not be loaded",
            "Check the network status at the top of the page, then try again.",
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum ListingValidityState {
    Valid,
    Invalid(Vec<String>),
}

fn is_valid_sha256(value: &str) -> bool {
    Sha256Digest::parse(value).is_ok()
}

fn listing_validity(listing: &GameListing) -> ListingValidityState {
    let mut reasons = Vec::new();
    let id = listing.id.trim();
    if id.is_empty()
        || id.len() > 64
        || !id.chars().all(|character| {
            character.is_ascii_lowercase() || character.is_ascii_digit() || character == '-'
        })
    {
        reasons.push("Game identifier is missing or invalid.".to_string());
    }
    if listing.title.trim().is_empty() || listing.title.chars().count() > 100 {
        reasons.push("Title is missing or exceeds the supported length.".to_string());
    }
    if listing.description.trim().is_empty() || listing.description.chars().count() > 2000 {
        reasons.push("Description is missing or exceeds the supported length.".to_string());
    }
    if listing.source != ListingSource::Nip99Listing {
        reasons.push(
            "This game's data is in an old or unsupported format. Publish it again to fix this."
                .to_string(),
        );
    }
    if publisher_hex(&listing.publisher_npub).is_none() {
        reasons.push("The signed publisher public key is invalid.".to_string());
    }
    if listing
        .event_id
        .as_deref()
        .map(str::is_empty)
        .unwrap_or(true)
    {
        reasons.push(
            "The game hasn't been confirmed on the network yet. Refresh to check again."
                .to_string(),
        );
    }
    if let AcquisitionPolicy::TimedAccess { starts_at, ends_at } = listing.acquisition {
        if starts_at >= ends_at {
            reasons.push("Timed access must end after it starts.".to_string());
        }
    }
    if listing.has_declared_price() {
        let parts = listing.lud16.split('@').collect::<Vec<_>>();
        if parts.len() != 2 || parts.iter().any(|part| part.trim().is_empty()) {
            reasons.push("Priced access requires a valid Lightning address.".to_string());
        }
    }
    if reasons.is_empty() {
        ListingValidityState::Valid
    } else {
        ListingValidityState::Invalid(reasons)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum ReleaseSummaryState {
    NotConfigured,
    Current { version: String },
    Invalid(Vec<String>),
}

fn release_summary_state(listing: &GameListing) -> ReleaseSummaryState {
    let version = listing
        .specs
        .iter()
        .find(|(key, _)| key == "version")
        .map(|(_, value)| value.trim().to_string())
        .filter(|value| !value.is_empty());
    let file_hash = listing
        .specs
        .iter()
        .find(|(key, _)| key == "file_hash")
        .map(|(_, value)| value.trim().to_string())
        .filter(|value| !value.is_empty());
    let has_distribution = listing.nip94_event_id.is_some()
        || listing.specs.iter().any(|(key, _)| {
            matches!(
                key.as_str(),
                "server" | "fulfillment_authorization" | "file_hash" | "version"
            )
        });
    if !has_distribution {
        return ReleaseSummaryState::NotConfigured;
    }
    let mut reasons = Vec::new();
    if version.is_none() {
        reasons.push("Managed distribution is missing a current version.".to_string());
    }
    match file_hash.as_deref() {
        Some(value) if is_valid_sha256(value) => {}
        Some(_) => reasons.push("The current build SHA-256 is invalid.".to_string()),
        None => {
            reasons.push("Managed distribution is missing a current build SHA-256.".to_string())
        }
    }
    if reasons.is_empty() {
        ReleaseSummaryState::Current {
            version: version.unwrap_or_default(),
        }
    } else {
        ReleaseSummaryState::Invalid(reasons)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum StorePageSummaryState {
    Loading,
    Published,
    NotAssociated,
    NotFound,
    Invalid,
    Unavailable,
}

fn store_page_summary_state(
    state: Option<&StorePageEnrichmentState>,
    loading: bool,
) -> StorePageSummaryState {
    match state {
        Some(StorePageEnrichmentState::Enriched(_)) => StorePageSummaryState::Published,
        Some(StorePageEnrichmentState::NotAssociated) => StorePageSummaryState::NotAssociated,
        Some(StorePageEnrichmentState::NotFound) => StorePageSummaryState::NotFound,
        Some(StorePageEnrichmentState::Invalid) => StorePageSummaryState::Invalid,
        Some(StorePageEnrichmentState::Unavailable) => StorePageSummaryState::Unavailable,
        None if loading => StorePageSummaryState::Loading,
        None => StorePageSummaryState::Unavailable,
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum CampaignDashboardState {
    Loading,
    Resolved(Vec<DiscoveredCampaign>),
    Unavailable,
}

fn campaign_summary_label(state: &CampaignDashboardState) -> String {
    match state {
        CampaignDashboardState::Loading => "Promotions: loading…".to_string(),
        CampaignDashboardState::Unavailable => "Promotions: couldn't be loaded".to_string(),
        CampaignDashboardState::Resolved(campaigns) if campaigns.is_empty() => {
            "Promotions: none".to_string()
        }
        CampaignDashboardState::Resolved(campaigns) => {
            let active = campaigns
                .iter()
                .filter(|campaign| campaign.classification == "active")
                .count();
            let upcoming = campaigns
                .iter()
                .filter(|campaign| campaign.classification == "upcoming")
                .count();
            let ended = campaigns
                .iter()
                .filter(|campaign| campaign.classification == "ended")
                .count();
            let cancelled = campaigns
                .iter()
                .filter(|campaign| campaign.classification == "cancelled")
                .count();
            format!("Promotions: {active} active · {upcoming} upcoming · {ended} ended · {cancelled} cancelled")
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct PublisherAttentionItem {
    listing_id: Option<String>,
    game_title: Option<String>,
    reason: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct PublisherAttentionGroup {
    listing_id: Option<String>,
    title: String,
    reasons: Vec<String>,
}

fn group_attention_items(items: Vec<PublisherAttentionItem>) -> Vec<PublisherAttentionGroup> {
    let mut groups = Vec::<PublisherAttentionGroup>::new();
    for item in items {
        if let Some(group) = groups
            .iter_mut()
            .find(|group| group.listing_id == item.listing_id)
        {
            group.reasons.push(item.reason);
        } else {
            groups.push(PublisherAttentionGroup {
                listing_id: item.listing_id,
                title: item
                    .game_title
                    .unwrap_or_else(|| "Publisher account".to_string()),
                reasons: vec![item.reason],
            });
        }
    }
    groups
}

fn listing_attention_items(
    listing: &GameListing,
    store_page: StorePageSummaryState,
    release: &ReleaseSummaryState,
    campaign: &CampaignDashboardState,
    has_local_partial: bool,
) -> Vec<PublisherAttentionItem> {
    let mut reasons = match listing_validity(listing) {
        ListingValidityState::Valid => Vec::new(),
        ListingValidityState::Invalid(reasons) => reasons,
    };
    if !listing.images.is_empty() && valid_cover_url(&listing.images).is_none() {
        reasons.push("Configured artwork cannot be displayed from a supported URL.".to_string());
    }
    match store_page {
        StorePageSummaryState::NotAssociated => {
            reasons.push("This game has no store page yet.".to_string())
        }
        StorePageSummaryState::NotFound => {
            reasons.push("The store page couldn't be found.".to_string())
        }
        StorePageSummaryState::Invalid => reasons.push("The store page has problems.".to_string()),
        StorePageSummaryState::Loading
        | StorePageSummaryState::Published
        | StorePageSummaryState::Unavailable => {}
    }
    if let ReleaseSummaryState::Invalid(release_reasons) = release {
        reasons.extend(release_reasons.iter().cloned());
    }
    if let CampaignDashboardState::Resolved(campaigns) = campaign {
        if campaigns
            .iter()
            .any(|campaign| campaign.classification == "invalid")
        {
            reasons.push("A promotion is invalid or incomplete.".to_string());
        }
        if campaigns
            .iter()
            .any(|campaign| !matches!(campaign.mode.as_str(), "" | "claim" | "claim_and_keep"))
        {
            reasons.push("A promotion uses settings this version doesn't support.".to_string());
        }
        if campaigns.iter().any(|campaign| campaign.event_id.is_none()) {
            reasons.push("A promotion hasn't been confirmed on the network yet.".to_string());
        }
    }
    if has_local_partial {
        reasons.push(
            "A store page update didn't finish. Open Advanced store page tools to retry."
                .to_string(),
        );
    }
    reasons
        .into_iter()
        .map(|reason| PublisherAttentionItem {
            listing_id: Some(listing.id.clone()),
            game_title: Some(listing.title.clone()),
            reason,
        })
        .collect()
}

fn short_identifier(value: &str) -> String {
    let characters = value.chars().collect::<Vec<_>>();
    if characters.len() <= 24 {
        value.to_string()
    } else {
        format!(
            "{}…{}",
            characters[..12].iter().collect::<String>(),
            characters[characters.len() - 8..]
                .iter()
                .collect::<String>()
        )
    }
}

fn store_page_label(state: &StorePageSummaryState) -> &'static str {
    match state {
        StorePageSummaryState::Loading => "Store page loading",
        StorePageSummaryState::Published => "Store page live",
        StorePageSummaryState::NotAssociated => "No store page",
        StorePageSummaryState::NotFound => "Store page missing",
        StorePageSummaryState::Invalid => "Store page has problems",
        StorePageSummaryState::Unavailable => "Store page status unknown",
    }
}

fn release_label(state: &ReleaseSummaryState) -> String {
    match state {
        ReleaseSummaryState::NotConfigured => "No build".to_string(),
        ReleaseSummaryState::Current { version } => format!("Build {version}"),
        ReleaseSummaryState::Invalid(_) => "Build needs attention".to_string(),
    }
}

fn campaign_mode_label(mode: &str) -> String {
    match mode {
        "" | "claim" | "claim_and_keep" => "Claim and keep".to_string(),
        value => format!("Unsupported configuration: {}", short_identifier(value)),
    }
}

fn render_campaign_dashboard_summaries(
    listings: Vec<GameListing>,
    states: HashMap<String, CampaignDashboardState>,
) -> AnyView {
    let loading = states
        .values()
        .any(|state| matches!(state, CampaignDashboardState::Loading));
    let unavailable = states
        .values()
        .any(|state| matches!(state, CampaignDashboardState::Unavailable));
    let mut rows = Vec::new();
    for listing in listings {
        let Some(CampaignDashboardState::Resolved(campaigns)) = states.get(&listing.id) else {
            continue;
        };
        for campaign in campaigns {
            let pointer_state = if listing
                .campaigns
                .iter()
                .any(|pointer| pointer.root_event_id == campaign.root_event_id)
            {
                "Shown on store page"
            } else {
                "Not shown on store page"
            };
            rows.push(view! {
                <article class="v2-publisher-campaign-summary">
                    <div><strong>{short_identifier(&campaign.campaign_id)}</strong><span>{listing.title.clone()}</span></div>
                    <dl>
                        <div><dt>"Type"</dt><dd>{campaign_mode_label(&campaign.mode)}</dd></div>
                        <div><dt>"State"</dt><dd>{campaign_status(&campaign.classification)}</dd></div>
                        <div><dt>"Start"</dt><dd>{format_unix(campaign.starts_at)}</dd></div>
                        <div><dt>"End"</dt><dd>{format_unix(campaign.ends_at)}</dd></div>
                        <div><dt>"Store page"</dt><dd>{pointer_state}</dd></div>
                    </dl>
                </article>
            });
        }
    }
    if rows.is_empty() && loading {
        return view! { <p class="v2-publisher-summary-state" role="status">"Loading promotions…"</p> }.into_any();
    }
    if rows.is_empty() && unavailable {
        return view! { <p class="v2-publisher-summary-state v2-publisher-summary-error" role="alert">"Promotions couldn't be loaded."</p> }.into_any();
    }
    if rows.is_empty() {
        return view! { <p class="v2-publisher-summary-state">"No promotions yet."</p> }.into_any();
    }
    view! {
        <div>
            {(loading || unavailable).then(|| view! { <p class="v2-publisher-scope-warning" role="status">"Some promotions couldn't be loaded, so this list may be incomplete."</p> })}
            <div class="v2-publisher-campaign-summary-list">{rows}</div>
        </div>
    }
    .into_any()
}

#[component]
#[cfg(feature = "web")]
pub fn PublishV2View(
    state: PublishViewState,
    on_navigate: Callback<PublishViewState>,
    on_open_listing: Callback<GameListing>,
) -> impl IntoView {
    let _ = (state, on_navigate, on_open_listing);
    view! {
        <section class="arc-publish-web-page">
            <crate::ui_v2::components::PageHeader
                title="Publish".to_string()
                eyebrow="Publisher studio".to_string()
                description="Publish games and manage promotions from the Arcadestr desktop app.".to_string()
            />
            <crate::ui_v2::components::EmptyState
                title="Publishing unavailable on the web"
                description="Network publication and promotion management need the Arcadestr desktop app. Open it on your computer to publish and manage your games."
                icon="desktop_windows"
                layout=crate::ui_v2::components::FeedbackLayout::Panel
            />
        </section>
    }
}

#[component]
#[cfg(not(feature = "web"))]
pub fn PublishV2View(
    state: PublishViewState,
    on_navigate: Callback<PublishViewState>,
    on_open_listing: Callback<GameListing>,
) -> impl IntoView {
    let to_dashboard = Callback::new(move |_: ()| on_navigate.run(PublishViewState::Games));
    let to_game =
        Callback::new(move |listing: GameListing| on_navigate.run(PublishViewState::game(listing)));

    view! {
        <div class="arc-publisher-shell">
            {match state {
                PublishViewState::Games => view! {
                    <PublishedGamesView
                        on_navigate=on_navigate
                        on_open_listing=on_open_listing
                    />
                }.into_any(),
                PublishViewState::NewGame { draft_id } => match draft_id {
                    Some(draft_id) => view! {
                        <PublishView draft_id=draft_id on_back=to_dashboard on_published=to_game />
                    }.into_any(),
                    None => view! {
                        <PublishView on_back=to_dashboard on_published=to_game />
                    }.into_any(),
                },
                PublishViewState::Game { listing, section } => view! {
                    <GameManagementView
                        listing=listing
                        section=section
                        on_back=to_dashboard
                        on_navigate=on_navigate
                        on_open_listing=on_open_listing
                    />
                }.into_any(),
                PublishViewState::StorePage(listing) => view! {
                    <StorePageEditorView
                        listing=listing.clone()
                        on_back=Callback::new({ let listing = listing.clone(); move |_| on_navigate.run(PublishViewState::game(listing.clone())) })
                        on_saved=to_game
                    />
                }.into_any(),
                PublishViewState::Campaign { listing, campaign } => view! {
                    <CampaignEditorView
                        listing=listing.clone()
                        campaign=campaign
                        on_back=Callback::new({ let listing = listing.clone(); move |_| on_navigate.run(PublishViewState::game(listing.clone())) })
                        on_saved=to_game
                    />
                }.into_any(),
            }}
        </div>
    }
}

/// Mount the real publisher routing/components against allowlisted mock IPC.
/// No separate authoring implementation or production route is introduced.
#[cfg(all(debug_assertions, not(feature = "web")))]
#[component]
pub fn PublisherConsolePreview() -> impl IntoView {
    use nostr::nips::nip19::ToBech32;
    let public = nostr::PublicKey::from_hex(
        "79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798",
    )
    .expect("public fixture");
    let publisher = public.to_bech32().expect("npub fixture");
    let listing:GameListing=serde_json::from_value(serde_json::json!({"id":"fixture","source":"nip99_listing","title":"Publisher lifecycle fixture","description":"Existing interface integration fixture","publisher_npub":publisher,"created_at":100,"price_sats":1000,"price":1000,"currency":"SATS","lud16":"dev@publisher.invalid","event_id":"11".repeat(32),"images":["https://media.invalid/fixture.png"],"platforms":["linux-x86_64"],"specs":[["server","https://one.test"],["server","https://two.test"],["version","1.0"],["file_hash","ab".repeat(32)],["fulfillment_authorization",serde_json::json!({"root_event_id":"aa".repeat(32),"fulfillment_pubkey":"22".repeat(32),"relay_hint":null}).to_string()]],"campaigns":[{"root_event_id":"33".repeat(32),"relay_hint":null}]})).expect("projection fixture");
    let auth = crate::AuthContext::new();
    auth.npub.set(Some(publisher.clone()));
    auth.begin_auth_flow();
    auth.active_account.set(Some(crate::StoredAccount {
        id: "synthetic".into(),
        npub: publisher,
        name: None,
        signing_mode: "local".into(),
        last_used: 0,
        is_current: true,
        picture: None,
        display_name: None,
        username: None,
        nip05: None,
        about: None,
    }));
    provide_context(auth.clone());
    let state = RwSignal::new(PublishViewState::game(listing.clone()));
    let navigate = Callback::new(move |next| state.set(next));
    let edit = listing.clone();
    let manage = listing.clone();
    view! {<div style="max-width:1440px;margin:0 auto;font-family:ui-monospace,monospace">
        <button type="button" data-testid="fixture-edit" style="padding:10px;background:transparent;color:oklch(0.78 0.13 210);border:1px solid oklch(0.5 0.13 210)" on:click=move |_|state.set(PublishViewState::Game { listing: edit.clone(), section: HubSection::Delivery })>"OPEN FIXTURE EDITOR"</button>
        <button type="button" data-testid="fixture-manage" style="padding:10px;background:transparent;color:oklch(0.78 0.13 210);border:1px solid oklch(0.5 0.13 210)" on:click=move |_|state.set(PublishViewState::game(manage.clone()))>"OPEN FIXTURE MANAGEMENT"</button>
        <button type="button" data-testid="fixture-new-game" style="padding:10px;background:transparent;color:oklch(0.78 0.13 210);border:1px solid oklch(0.5 0.13 210)" on:click=move |_|state.set(PublishViewState::NewGame { draft_id: None })>"OPEN FIXTURE NEW GAME"</button>
        <button type="button" data-testid="fixture-account-switch" style="padding:10px;background:transparent;color:oklch(0.78 0.13 210);border:1px solid oklch(0.5 0.13 210)" on:click=move |_|{auth.begin_auth_flow();auth.npub.set(None);auth.active_account.set(None);}>"MOCK ACCOUNT SWITCH"</button>
        {move ||view!{<PublishV2View state=state.get() on_navigate=navigate on_open_listing=Callback::new(|_|{})/>}}
    </div>}
}

#[component]
fn PublishedGamesView(
    on_navigate: Callback<PublishViewState>,
    on_open_listing: Callback<GameListing>,
) -> impl IntoView {
    let auth = use_context::<crate::AuthContext>().expect("AuthContext not provided");
    let listings = RwSignal::new(Vec::<GameListing>::new());
    let loading = RwSignal::new(true);
    let refreshing = RwSignal::new(false);
    let load_error = RwSignal::new(false);
    let load_error_message =
        RwSignal::new(None::<arcadestr_core::marketplace_fetch::MarketplaceFetchError>);
    let loaded_publisher = RwSignal::new(None::<String>);
    let listing_generation = RwSignal::new(0_u64);
    let listing_request = RwSignal::new(None::<(String, u64)>);
    let campaign_states = RwSignal::new(HashMap::<String, CampaignDashboardState>::new());
    let campaign_generation = RwSignal::new(0_u64);
    let campaign_fingerprint = RwSignal::new(String::new());
    let store_page_states = RwSignal::new(HashMap::<String, StorePageEnrichmentState>::new());
    // Start resolving so an unresolved Store Page is never rendered as "unavailable"
    // before the enrichment request has had a chance to run.
    let store_page_loading = RwSignal::new(true);
    let store_page_generation = RwSignal::new(0_u64);
    let store_page_fingerprint = RwSignal::new(String::new());
    let dirty_store_page_coordinates = RwSignal::new(HashSet::<String>::new());
    let partial_store_page_coordinates = RwSignal::new(HashSet::<String>::new());

    let auth_for_refresh = auth.clone();
    let refresh = Callback::new(move |()| {
        let Some(publisher) = auth_for_refresh.npub.get() else {
            listing_generation.update(|value| *value = value.wrapping_add(1));
            listing_request.set(None);
            loaded_publisher.set(None);
            listings.set(Vec::new());
            campaign_states.set(HashMap::new());
            store_page_states.set(HashMap::new());
            dirty_store_page_coordinates.set(HashSet::new());
            partial_store_page_coordinates.set(HashSet::new());
            loading.set(false);
            refreshing.set(false);
            load_error.set(false);
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
        let account_changed = loaded_publisher.get_untracked().as_deref() != Some(&publisher);
        if account_changed {
            loaded_publisher.set(Some(publisher.clone()));
            listings.set(Vec::new());
            campaign_states.set(HashMap::new());
            store_page_states.set(HashMap::new());
            dirty_store_page_coordinates.set(HashSet::new());
            partial_store_page_coordinates.set(HashSet::new());
        }
        let initial_load = listings.get_untracked().is_empty();
        loading.set(initial_load);
        refreshing.set(!initial_load);
        load_error.set(false);
        load_error_message.set(None);
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
                        listing_generation.try_get_untracked(),
                        request_generation,
                    ) {
                        return;
                    }
                    let items = received_for_complete.get_untracked();
                    listings_signal.set(current_user_listings(items, &publisher_for_complete));
                    loading.set(false);
                    refreshing.set(false);
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
                        listing_generation.try_get_untracked(),
                        request_generation,
                    ) {
                        return;
                    }
                    if loading.get_untracked() {
                        listings_signal
                            .set(current_user_listings(received.get_untracked(), &publisher));
                        loading.set(false);
                    }
                    refreshing.set(false);
                    listing_request.set(None);
                }
                Err(fetch_error) => {
                    if !accepts_account_response(
                        auth_for_request.npub.get_untracked().as_deref(),
                        &publisher,
                        listing_generation.try_get_untracked(),
                        request_generation,
                    ) {
                        return;
                    }
                    warn!("publisher listing refresh failed: {}", fetch_error);
                    // Retain listings received before another relay failed.
                    let partial = current_user_listings(received.get_untracked(), &publisher);
                    listings_signal.update(|items| {
                        for listing in partial {
                            super::marketplace_loader::upsert_latest_listing(items, listing);
                        }
                    });
                    load_error_message.set(Some(fetch_error));
                    load_error.set(true);
                    loading.set(false);
                    refreshing.set(false);
                    listing_request.set(None);
                }
            }
        });
    });

    Effect::new(move |_| refresh.run(()));

    // Local drafts: unpublished games, and unpublished changes to live games.
    let saved_drafts =
        RwSignal::new(Vec::<arcadestr_core::publisher_recovery::PublisherDraft>::new());
    let drafts_generation = RwSignal::new(0_u64);
    let pending_draft_delete = RwSignal::new(None::<String>);
    let auth_for_drafts = auth.clone();
    let load_drafts = Callback::new(move |()| {
        drafts_generation.update(|value| *value = value.wrapping_add(1));
        let request_generation = drafts_generation.get_untracked();
        let Some(publisher) = auth_for_drafts.npub.get_untracked() else {
            saved_drafts.set(Vec::new());
            return;
        };
        let auth_for_request = auth_for_drafts.clone();
        spawn_local(async move {
            let result =
                crate::tauri_bridge::invoke_publisher_recovery_state(publisher.clone()).await;
            if !accepts_account_response(
                auth_for_request.npub.get_untracked().as_deref(),
                &publisher,
                drafts_generation.try_get_untracked(),
                request_generation,
            ) {
                return;
            }
            match result {
                Ok(state) => saved_drafts.set(state.drafts),
                Err(error) => warn!("publisher drafts could not be loaded: {}", error),
            }
        });
    });
    let auth_for_draft_account = auth.clone();
    Effect::new(move |_| {
        auth_for_draft_account.npub.track();
        load_drafts.run(());
    });
    let local_drafts = Signal::derive(move || local_game_drafts(&saved_drafts.get()));
    let changed_games = Signal::derive(move || games_with_local_changes(&saved_drafts.get()));
    let auth_for_draft_delete = auth.clone();
    let delete_draft = Callback::new(move |draft: LocalGameDraft| {
        let Some(publisher) = auth_for_draft_delete.npub.get_untracked() else {
            return;
        };
        pending_draft_delete.set(None);
        spawn_local(async move {
            if let Err(error) = crate::tauri_bridge::invoke_delete_publisher_draft(
                publisher,
                draft.draft_id,
                "listing".into(),
                draft.revision,
            )
            .await
            {
                warn!("publisher draft could not be deleted: {}", error);
            }
            load_drafts.run(());
        });
    });

    let auth_for_local_state = auth.clone();
    Effect::new(move |_| {
        let items = listings.get();
        let Some(publisher_npub) = auth_for_local_state.npub.get() else {
            dirty_store_page_coordinates.set(HashSet::new());
            partial_store_page_coordinates.set(HashSet::new());
            return;
        };
        let coordinates = items
            .iter()
            .filter_map(canonical_listing_coordinate)
            .collect::<HashSet<_>>();
        dirty_store_page_coordinates.set(
            publisher_store_page_dirty_coordinates(&publisher_npub)
                .into_iter()
                .filter(|coordinate| coordinates.contains(coordinate))
                .collect(),
        );
        partial_store_page_coordinates.set(
            publisher_store_page_partial_coordinates(&publisher_npub)
                .into_iter()
                .filter(|coordinate| coordinates.contains(coordinate))
                .collect(),
        );
    });

    let auth_for_campaigns = auth.clone();
    Effect::new(move |_| {
        let items = listings.get();
        let Some(publisher_npub) = auth_for_campaigns.npub.get() else {
            campaign_generation.update(|value| *value = value.wrapping_add(1));
            campaign_fingerprint.set(String::new());
            campaign_states.set(HashMap::new());
            return;
        };
        if items.is_empty() {
            campaign_states.set(HashMap::new());
            return;
        }
        let fingerprint = format!(
            "{}:{}",
            publisher_npub,
            items
                .iter()
                .map(|listing| {
                    format!(
                        "{}:{}:{}",
                        listing.id,
                        listing.event_id.as_deref().unwrap_or_default(),
                        listing
                            .campaigns
                            .iter()
                            .map(|pointer| format!(
                                "{}@{}",
                                pointer.root_event_id,
                                pointer.relay_hint.as_deref().unwrap_or_default()
                            ))
                            .collect::<Vec<_>>()
                            .join(",")
                    )
                })
                .collect::<Vec<_>>()
                .join("|")
        );
        if fingerprint == campaign_fingerprint.get_untracked() {
            return;
        }
        campaign_fingerprint.set(fingerprint);
        campaign_generation.update(|value| *value = value.wrapping_add(1));
        let request_generation = campaign_generation.get_untracked();
        campaign_states.set(
            items
                .iter()
                .map(|listing| (listing.id.clone(), CampaignDashboardState::Loading))
                .collect(),
        );
        for listing in items {
            let listing_id = listing.id.clone();
            let request = DiscoverCampaignsRequest {
                publisher_npub: publisher_npub.clone(),
                listing_id: listing_id.clone(),
                pointers: listing
                    .campaigns
                    .into_iter()
                    .map(|pointer| CampaignPointerInput {
                        root_event_id: pointer.root_event_id,
                        relay_hint: pointer.relay_hint,
                    })
                    .collect(),
            };
            let publisher_for_request = publisher_npub.clone();
            let auth_for_request = auth_for_campaigns.clone();
            spawn_local(async move {
                let result = invoke_discover_campaigns(request).await;
                if !accepts_account_response(
                    auth_for_request.npub.get_untracked().as_deref(),
                    &publisher_for_request,
                    campaign_generation.try_get_untracked(),
                    request_generation,
                ) {
                    return;
                }
                campaign_states.update(|states| {
                    states.insert(
                        listing_id,
                        result
                            .map(CampaignDashboardState::Resolved)
                            .unwrap_or(CampaignDashboardState::Unavailable),
                    );
                });
            });
        }
    });

    let auth_for_store_pages = auth.clone();
    Effect::new(move |_| {
        let items = listings.get();
        let Some(publisher_npub) = auth_for_store_pages.npub.get() else {
            store_page_generation.update(|value| *value = value.wrapping_add(1));
            store_page_fingerprint.set(String::new());
            store_page_states.set(HashMap::new());
            store_page_loading.set(false);
            return;
        };
        let references = items
            .iter()
            .filter_map(|listing| {
                Some(StorePageListingRef {
                    listing_coordinate: canonical_listing_coordinate(listing)?,
                    listing_event_id: listing.event_id.clone()?,
                })
            })
            .take(64)
            .collect::<Vec<_>>();
        let fingerprint = format!(
            "{}:{}",
            publisher_npub,
            references
                .iter()
                .map(|listing| format!(
                    "{}:{}",
                    listing.listing_coordinate, listing.listing_event_id
                ))
                .collect::<Vec<_>>()
                .join("|")
        );
        if fingerprint == store_page_fingerprint.get_untracked() {
            return;
        }
        store_page_fingerprint.set(fingerprint);
        store_page_generation.update(|value| *value = value.wrapping_add(1));
        let request_generation = store_page_generation.get_untracked();
        store_page_states.set(HashMap::new());
        if references.is_empty() {
            store_page_loading.set(false);
            return;
        }
        store_page_loading.set(true);
        let expected = references
            .iter()
            .map(|listing| {
                (
                    listing.listing_coordinate.clone(),
                    listing.listing_event_id.clone(),
                )
            })
            .collect::<HashMap<_, _>>();
        let request = StorePageEnrichmentRequest {
            generation: request_generation,
            listings: references,
        };
        let auth_for_request = auth_for_store_pages.clone();
        spawn_local(async move {
            let result = invoke_enrich_store_pages(request).await;
            if !accepts_account_response(
                auth_for_request.npub.get_untracked().as_deref(),
                &publisher_npub,
                store_page_generation.try_get_untracked(),
                request_generation,
            ) {
                return;
            }
            let mut states = expected
                .keys()
                .map(|coordinate| (coordinate.clone(), StorePageEnrichmentState::Unavailable))
                .collect::<HashMap<_, _>>();
            match result {
                Ok(response) if response.generation == request_generation => {
                    for update in response.cached.into_iter().chain(response.refreshed) {
                        if expected.get(&update.listing_coordinate)
                            == Some(&update.listing_event_id)
                        {
                            states.insert(update.listing_coordinate, update.state);
                        }
                    }
                }
                Ok(_) => warn!("publisher Store Page summary returned a stale generation"),
                Err(error) => warn!("publisher Store Page summary failed: {}", error),
            }
            store_page_states.set(states);
            store_page_loading.set(false);
        });
    });

    let auth_for_signer = auth.clone();
    let signer_state = Signal::derive(move || {
        let publisher = auth_for_signer.npub.get();
        publisher_signer_state(
            publisher.as_deref(),
            auth_for_signer.active_account.get().as_ref(),
            &auth_for_signer.connection_status.get(),
        )
    });
    let auth_for_dashboard_state = auth.clone();
    let dashboard_state = Signal::derive(move || {
        publisher_dashboard_state(
            auth_for_dashboard_state.is_loading.get(),
            auth_for_dashboard_state.npub.get().is_some(),
            loading.get(),
            !listings.get().is_empty(),
            load_error.get(),
        )
    });
    let attention_items = Signal::derive(move || {
        let dirty_partial = partial_store_page_coordinates.get();
        let store_pages = store_page_states.get();
        let campaigns = campaign_states.get();
        let mut items = listings
            .get()
            .into_iter()
            .flat_map(|listing| {
                let coordinate = canonical_listing_coordinate(&listing);
                let store_page = store_page_summary_state(
                    coordinate
                        .as_ref()
                        .and_then(|coordinate| store_pages.get(coordinate)),
                    store_page_loading.get(),
                );
                let campaign = campaigns
                    .get(&listing.id)
                    .cloned()
                    .unwrap_or(CampaignDashboardState::Loading);
                let release = release_summary_state(&listing);
                listing_attention_items(
                    &listing,
                    store_page,
                    &release,
                    &campaign,
                    coordinate
                        .as_ref()
                        .is_some_and(|coordinate| dirty_partial.contains(coordinate)),
                )
            })
            .collect::<Vec<_>>();
        match signer_state.get() {
            PublisherSignerState::Available => {}
            PublisherSignerState::Connecting => items.push(PublisherAttentionItem {
                listing_id: None,
                game_title: None,
                reason: "Your signing app is still connecting, so you can't publish yet."
                    .to_string(),
            }),
            PublisherSignerState::Unavailable => items.push(PublisherAttentionItem {
                listing_id: None,
                game_title: None,
                reason: "Reconnect your signing app in Settings to publish.".to_string(),
            }),
            PublisherSignerState::Unknown => items.push(PublisherAttentionItem {
                listing_id: None,
                game_title: None,
                reason: "Checking your signing app…".to_string(),
            }),
        }
        items
    });
    let auth_for_create = auth.clone();
    let can_create = Signal::derive(move || auth_for_create.npub.get().is_some());
    let new_game = move || on_navigate.run(PublishViewState::NewGame { draft_id: None });

    view! {
        <section class="v2-publisher-studio v2-publisher-dashboard">
            <header class="v2-publisher-header v2-publisher-dashboard-header">
                <div><h1>"Your games"</h1></div>
                <div class="v2-publisher-actions">
                    <button type="button" class="v2-btn-secondary" on:click=move |_| { refresh.run(()); load_drafts.run(()); } disabled=move || loading.get() || refreshing.get()>{move || if refreshing.get() { "Refreshing..." } else { "Refresh" }}</button>
                    <button type="button" class="v2-btn-primary" aria-describedby="publisher-create-game-requirement" on:click=move |_| new_game() disabled=move || !can_create.get()>"+ Create game"</button>
                </div>
            </header>
            <p id="publisher-create-game-requirement" class="v2-publisher-action-requirement">{move || match signer_state.get() {
                PublisherSignerState::Available => "Create a game and fill in its sections in any order. Your progress is saved on this device as you go.",
                PublisherSignerState::Connecting => "Your signing app is connecting. You can create and edit games now and publish once it's connected.",
                PublisherSignerState::Unavailable => "You can create and edit games now. Reconnect your signing app in Settings to publish.",
                PublisherSignerState::Unknown => "You can create and edit games while your signing app is checked.",
            }}</p>

            <crate::components::publisher_recovery::PublisherRecoveryPanel />

            {move || {
                let drafts = local_drafts.get();
                (!drafts.is_empty()).then(|| view! {
                    <section class="v2-publisher-drafts" aria-labelledby="publisher-drafts-title">
                        <h2 id="publisher-drafts-title" class="v2-hub-kicker">"Drafts on this device"</h2>
                        <div class="v2-publisher-game-list">
                            {drafts.into_iter().map(|draft| {
                                let open_id = draft.draft_id.clone();
                                let confirm_id = draft.draft_id.clone();
                                let pending_id = draft.draft_id.clone();
                                let title = draft.title.clone();
                                let delete = draft.clone();
                                view! {
                                    <article class="v2-publisher-game-row">
                                        <div class="v2-publisher-row-art"><GameArtwork title=title.clone() state=artwork_state_from_url(None) role=ArtworkRole::Thumbnail /></div>
                                        <div class="v2-publisher-row-copy">
                                            <div class="v2-publisher-row-title"><h2>{title}</h2></div>
                                            <div class="v2-publisher-row-statuses">
                                                <StatusChip label="Draft · not published" variant=StatusChipVariant::Draft icon=None size=StatusChipSize::Compact />
                                            </div>
                                        </div>
                                        <div class="v2-publisher-row-actions">
                                            <button type="button" class="v2-btn-primary" on:click=move |_| on_navigate.run(PublishViewState::NewGame { draft_id: Some(open_id.clone()) })>"Continue"</button>
                                            {move || if pending_draft_delete.get().as_deref() == Some(pending_id.as_str()) {
                                                let delete = delete.clone();
                                                view! {
                                                    <button type="button" class="v2-btn-danger" on:click=move |_| delete_draft.run(delete.clone())>"Delete draft"</button>
                                                    <button type="button" class="v2-btn-secondary" on:click=move |_| pending_draft_delete.set(None)>"Keep"</button>
                                                }.into_any()
                                            } else {
                                                let confirm_id = confirm_id.clone();
                                                view! {
                                                    <button type="button" class="v2-btn-secondary" on:click=move |_| pending_draft_delete.set(Some(confirm_id.clone()))>"Delete…"</button>
                                                }.into_any()
                                            }}
                                        </div>
                                    </article>
                                }
                            }).collect_view()}
                        </div>
                    </section>
                })
            }}

            {move || match dashboard_state.get() {
                PublisherDashboardState::SignedOut => view! { <section class="v2-publisher-feedback" role="status"><span class="material-symbols-outlined" aria-hidden="true">"person_off"</span><div><h2>"Sign in to see your games"</h2><p>"Sign in with the account you publish games with. Only that account's games appear here."</p></div></section> }.into_any(),
                PublisherDashboardState::Loading => view! { <section class="v2-publisher-feedback" role="status"><span class="material-symbols-outlined" aria-hidden="true">"progress_activity"</span><div><h2>"Loading your games"</h2><p>"Finding your published games, store pages, and promotions."</p></div></section> }.into_any(),
                PublisherDashboardState::Error => {
                    let error = load_error_message.get().unwrap_or_else(|| {
                        arcadestr_core::marketplace_fetch::MarketplaceFetchError::failed(
                            "Unknown error",
                        )
                    });
                    let (title, message) = publisher_load_error_copy(&error);
                    view! {
                        <ErrorState
                            title=title
                            message=message
                            technical_detail=error.to_string()
                            on_retry=Callback::new(move |_| refresh.run(()))
                            retry_busy=Signal::derive(move || loading.get() || refreshing.get())
                            severity=ErrorSeverity::Recoverable
                        />
                    }
                    .into_any()
                }
                PublisherDashboardState::Empty if !local_drafts.get().is_empty() => view! {
                    <p class="v2-publisher-summary-state">"No published games yet. Continue a draft above and publish it when it's ready."</p>
                }.into_any(),
                PublisherDashboardState::Empty => view! { <section class="v2-publisher-feedback"><span class="material-symbols-outlined" aria-hidden="true">"sports_esports"</span><div><h2>"No games yet"</h2><p>"Create a game to get started. Fill in its sections in any order; your progress is saved on this device until you publish. Already published a game? Check you're signed in with that account and refresh."</p>{can_create.get().then(|| view! { <button type="button" class="v2-btn-primary" on:click=move |_| new_game()>"Create game"</button> })}</div></section> }.into_any(),
                PublisherDashboardState::Partial | PublisherDashboardState::Ready => view! {
                    <div class="v2-publisher-dashboard-content" aria-live="polite" aria-busy=move || refreshing.get() || store_page_loading.get()>
                        {(dashboard_state.get() == PublisherDashboardState::Partial).then(|| view! { <div class="v2-publisher-scope-warning" role="status"><strong>"Showing games loaded earlier"</strong><span>"The latest refresh couldn't reach the network. Select Refresh to try again."</span></div> })}

                        {move || {
                            let items = attention_items.get();
                            (!items.is_empty()).then(|| view! {
                                <section class="v2-publisher-attention" aria-labelledby="publisher-attention-title">
                                    <h2 id="publisher-attention-title">"Needs attention"</h2>
                                    <ul>{group_attention_items(items).into_iter().map(|group| { let listing = group.listing_id.as_ref().and_then(|id| listings.get().into_iter().find(|listing| &listing.id == id)); view! { <li><div><strong>{group.title}</strong><ul>{group.reasons.into_iter().map(|reason| view! { <li>{reason}</li> }).collect_view()}</ul></div>{listing.map(|listing| { let selected = listing.clone(); view! { <button type="button" class="v2-btn-secondary" on:click=move |_| on_navigate.run(PublishViewState::game(selected.clone()))>"Open"</button> } })}</li> } }).collect_view()}</ul>
                                </section>
                            })
                        }}

                        <h2 class="v2-hub-kicker">"Published"</h2>
                        <div class="v2-publisher-game-list">
                            {listings.get().into_iter().map(|listing| {
                                let manage_listing = listing.clone();
                                let store_listing = listing.clone();
                                let public_listing = listing.clone();
                                let coordinate = canonical_listing_coordinate(&listing);
                                let resolved_store_pages = store_page_states.get();
                                let store_page = store_page_summary_state(coordinate.as_ref().and_then(|coordinate| resolved_store_pages.get(coordinate)), store_page_loading.get());
                                let release = release_summary_state(&listing);
                                let validity = listing_validity(&listing);
                                let campaign = campaign_states.get().get(&listing.id).cloned().unwrap_or(CampaignDashboardState::Loading);
                                let has_partial = coordinate.as_ref().is_some_and(|coordinate| partial_store_page_coordinates.get().contains(coordinate));
                                let has_local_changes = changed_games.get().contains(&listing.id);
                                let artwork = artwork_state_from_url(valid_cover_url(&listing.images));
                                let title = listing.title.clone();
                                view! {
                                    <article class="v2-publisher-game-row">
                                        <div class="v2-publisher-row-art"><GameArtwork title=title.clone() state=artwork role=ArtworkRole::Thumbnail /></div>
                                        <div class="v2-publisher-row-copy">
                                            <div class="v2-publisher-row-title"><h2>{title}</h2><span>{format!("Updated {}", format_unix(listing.created_at))}</span></div>
                                            <div class="v2-publisher-row-statuses">
                                                <StatusChip label=if listing.event_id.is_some() { "Live" } else { "Not confirmed on the network" } variant=if listing.event_id.is_some() { StatusChipVariant::Published } else { StatusChipVariant::Unverified } icon=None size=StatusChipSize::Compact />
                                                {matches!(validity, ListingValidityState::Invalid(_)).then(|| view! { <StatusChip label="Needs attention" variant=StatusChipVariant::Warning icon=None size=StatusChipSize::Compact /> })}
                                                {has_local_changes.then(|| view! { <StatusChip label="Unpublished changes" variant=StatusChipVariant::Draft icon=None size=StatusChipSize::Compact /> })}
                                                {has_partial.then(|| view! { <StatusChip label="Store page update didn't finish" variant=StatusChipVariant::Warning icon=None size=StatusChipSize::Compact /> })}
                                            </div>
                                            <p class="v2-publisher-row-meta">{format!("{} · {} · {}", access_label(&listing.acquisition), store_page_label(&store_page), release_label(&release))}</p>
                                            <p class="v2-publisher-row-campaign">{campaign_summary_label(&campaign)}</p>
                                        </div>
                                        <div class="v2-publisher-row-actions">
                                            <button type="button" class="v2-btn-primary" on:click=move |_| on_navigate.run(PublishViewState::game(manage_listing.clone()))>"Open"</button>
                                            <details class="v2-publisher-row-more"><summary>"More"</summary><div><button type="button" on:click=move |_| on_navigate.run(PublishViewState::Game { listing: store_listing.clone(), section: HubSection::StorePage })>"Edit store page"</button><button type="button" on:click=move |_| on_open_listing.run(public_listing.clone())>"View in store"</button></div></details>
                                        </div>
                                    </article>
                                }
                            }).collect_view()}
                        </div>

                        <details class="v2-publisher-campaigns"><summary><span><span class="v2-publisher-kicker">"Promotions"</span><strong>"All promotions"</strong></span><span>"View"</span></summary><div aria-live="polite">{render_campaign_dashboard_summaries(listings.get(), campaign_states.get())}</div></details>
                    </div>
                }.into_any(),
            }}
        </section>
    }
}

/// A live game's hub: the shared game editor, with promotions, outstanding
/// issues and access management added to its overview.
#[cfg(not(feature = "web"))]
#[component]
fn GameManagementView(
    listing: GameListing,
    section: HubSection,
    on_back: Callback<()>,
    on_navigate: Callback<PublishViewState>,
    on_open_listing: Callback<GameListing>,
) -> impl IntoView {
    let management_publisher = listing.publisher_npub.clone();
    let management_coordinate = listing_coordinate(&listing);
    let management_servers = listing
        .specs
        .iter()
        .filter(|(name, _)| name == "server")
        .map(|(_, url)| url.clone())
        .collect::<Vec<_>>();
    let management_roots = listing
        .specs
        .iter()
        .filter(|(name, _)| name == "fulfillment_authorization")
        .filter_map(|(_, value)| serde_json::from_str::<serde_json::Value>(value).ok())
        .filter_map(|value| value["root_event_id"].as_str().map(str::to_owned))
        .collect::<Vec<_>>();
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
    let discovery_generation = RwSignal::new(0_u64);
    let discovery_account = RwSignal::new(None::<String>);
    let auth_for_discovery = auth.clone();
    Effect::new(move |_| {
        let Some(initiating_npub) = auth_for_discovery.npub.get() else {
            discovery_generation.update(|value| *value = value.wrapping_add(1));
            discovery_account.set(None);
            campaigns.set(Vec::new());
            error.set(Some(
                "Sign in with the account that published this game.".into(),
            ));
            loading.set(false);
            return;
        };
        if initiating_npub != publisher {
            discovery_generation.update(|value| *value = value.wrapping_add(1));
            discovery_account.set(None);
            campaigns.set(Vec::new());
            error.set(Some(
                "Switch to the account that published this game.".into(),
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
        let auth_for_request = auth_for_discovery.clone();
        spawn_local(async move {
            let result = invoke_discover_campaigns(request).await;
            if !accepts_account_response(
                auth_for_request.npub.get_untracked().as_deref(),
                &initiating_npub,
                discovery_generation.try_get_untracked(),
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

    let store_page_state = RwSignal::new(None::<StorePageEnrichmentState>);
    // Start resolving so an unresolved Store Page is never rendered as "unavailable"
    // before the enrichment request has had a chance to run.
    let store_page_loading = RwSignal::new(true);
    let store_page_generation = RwSignal::new(0_u64);
    let store_page_account = RwSignal::new(None::<String>);
    let auth_for_store_page = auth.clone();
    let listing_for_store_page_state = listing.clone();
    Effect::new(move |_| {
        let account = auth_for_store_page.npub.get();
        let matches_publisher = account
            .as_deref()
            .is_some_and(|account| account == listing_for_store_page_state.publisher_npub);
        let Some(account) = account.filter(|_| matches_publisher) else {
            store_page_generation.update(|value| *value = value.wrapping_add(1));
            store_page_account.set(None);
            store_page_state.set(None);
            store_page_loading.set(false);
            return;
        };
        if store_page_account.get_untracked().as_deref() == Some(account.as_str()) {
            return;
        }
        store_page_generation.update(|value| *value = value.wrapping_add(1));
        let request_generation = store_page_generation.get_untracked();
        store_page_account.set(Some(account.clone()));
        store_page_state.set(None);
        let association = canonical_listing_coordinate(&listing_for_store_page_state)
            .zip(listing_for_store_page_state.event_id.clone());
        let Some((coordinate, event_id)) = association else {
            store_page_loading.set(false);
            store_page_state.set(Some(StorePageEnrichmentState::Unavailable));
            return;
        };
        store_page_loading.set(true);
        let request = StorePageEnrichmentRequest {
            generation: request_generation,
            listings: vec![StorePageListingRef {
                listing_coordinate: coordinate.clone(),
                listing_event_id: event_id.clone(),
            }],
        };
        let auth_for_request = auth_for_store_page.clone();
        spawn_local(async move {
            let result = invoke_enrich_store_pages(request).await;
            if !accepts_account_response(
                auth_for_request.npub.get_untracked().as_deref(),
                &account,
                store_page_generation.try_get_untracked(),
                request_generation,
            ) {
                return;
            }
            let resolved = match result {
                Ok(response) if response.generation == request_generation => response
                    .cached
                    .into_iter()
                    .chain(response.refreshed)
                    .find(|update| {
                        update.listing_coordinate == coordinate
                            && update.listing_event_id == event_id
                    })
                    .map(|update| update.state)
                    .unwrap_or(StorePageEnrichmentState::Unavailable),
                Ok(_) => {
                    warn!("managed Store Page summary returned a stale generation");
                    StorePageEnrichmentState::Unavailable
                }
                Err(error) => {
                    warn!("managed Store Page summary failed: {}", error);
                    StorePageEnrichmentState::Unavailable
                }
            };
            store_page_state.set(Some(resolved));
            store_page_loading.set(false);
        });
    });

    let auth_for_signer = auth.clone();
    let signer_state = Signal::derive(move || {
        let publisher = auth_for_signer.npub.get();
        publisher_signer_state(
            publisher.as_deref(),
            auth_for_signer.active_account.get().as_ref(),
            &auth_for_signer.connection_status.get(),
        )
    });
    let auth_for_local_state = auth.clone();
    let listing_for_local_state = listing.clone();
    let store_page_partial = Signal::derive(move || {
        let Some(account) = auth_for_local_state.npub.get() else {
            return false;
        };
        if account != listing_for_local_state.publisher_npub {
            return false;
        }
        canonical_listing_coordinate(&listing_for_local_state).is_some_and(|coordinate| {
            publisher_store_page_partial_coordinates(&account).contains(&coordinate)
        })
    });

    let campaign_state = Signal::derive(move || {
        if loading.get() {
            CampaignDashboardState::Loading
        } else if error.get().is_some() {
            CampaignDashboardState::Unavailable
        } else {
            CampaignDashboardState::Resolved(campaigns.get())
        }
    });
    let summary_store_page = Signal::derive(move || {
        store_page_summary_state(store_page_state.get().as_ref(), store_page_loading.get())
    });
    let listing_for_attention = listing.clone();
    let attention_groups = Signal::derive(move || {
        let release = release_summary_state(&listing_for_attention);
        group_attention_items(listing_attention_items(
            &listing_for_attention,
            summary_store_page.get(),
            &release,
            &campaign_state.get(),
            store_page_partial.get(),
        ))
    });

    let listing_for_extra = listing.clone();
    let overview_extra = ViewFn::from(move || {
        let listing = listing_for_extra.clone();
        let listing_for_button = listing.clone();
        let listing_for_rows = listing.clone();
        let listing_for_tools = listing.clone();
        let listing_for_public = listing.clone();
        let management_publisher = management_publisher.clone();
        let management_coordinate = management_coordinate.clone();
        let management_servers = management_servers.clone();
        let management_roots = management_roots.clone();
        view! {
            {move || {
                let groups = attention_groups.get();
                (!groups.is_empty()).then(|| view! {
                    <section class="v2-publisher-attention" aria-labelledby="publisher-manage-attention-title">
                        <h2 id="publisher-manage-attention-title">"Needs attention"</h2>
                        <ul>{groups.into_iter().map(|group| view! { <li><div><strong>{group.title}</strong><ul>{group.reasons.into_iter().map(|reason| view! { <li>{reason}</li> }).collect_view()}</ul></div></li> }).collect_view()}</ul>
                    </section>
                })
            }}
            {move || store_page_partial.get().then(|| view! {
                <div class="v2-publisher-scope-warning" role="status">
                    <strong>"Store page update didn't finish"</strong>
                    <span>"An update started from the store page tools didn't reach every server. Open them to retry."</span>
                </div>
            })}

            <section class="v2-publisher-panel" aria-labelledby="publisher-promotions-title">
                <div class="v2-publisher-section-heading">
                    <div>
                        <h2 id="publisher-promotions-title">"Promotions"</h2>
                        <p class="v2-publisher-summary-state">"Let players claim the game for free during a time window. Anyone who claims it keeps it."</p>
                    </div>
                    <button
                        type="button"
                        class="v2-btn-secondary"
                        disabled=move || !signer_can_publish(signer_state.get())
                        on:click=move |_| on_navigate.run(PublishViewState::Campaign { listing: listing_for_button.clone(), campaign: None })
                    >"New promotion"</button>
                </div>
                {move || error.get().map(|message| view! { <p class="v2-publisher-summary-state v2-publisher-summary-error" role="alert">{format!("Promotions couldn't be loaded: {message}")}</p> })}
                {move || if loading.get() {
                    view! { <p class="v2-publisher-summary-state" role="status">"Looking for promotions…"</p> }.into_any()
                } else if error.get().is_some() {
                    view! { <></> }.into_any()
                } else if campaigns.get().is_empty() {
                    view! { <p class="v2-publisher-summary-state">"No promotions yet."</p> }.into_any()
                } else {
                    let selected_listing = listing_for_rows.clone();
                    view! { <div class="v2-publisher-promotion-list">{campaigns.get().into_iter().map(|campaign| campaign_row(campaign, selected_listing.clone(), on_navigate)).collect_view()}</div> }.into_any()
                }}
            </section>

            <details class="v2-create-disclosure">
                <summary class="v2-create-disclosure-summary">"Revoke access"</summary>
                <p class="v2-create-help">"Stop a distribution service from giving out new copies, or take away one player's access. Purchases already made are not refunded."</p>
                <crate::components::publisher_management::PublisherManagementPanel publisher=management_publisher coordinate=management_coordinate servers=management_servers authority_roots=management_roots />
            </details>

            <div class="v2-publisher-actions">
                <button type="button" class="v2-btn-secondary" on:click=move |_| on_open_listing.run(listing_for_public.clone())>"View in store"</button>
                <button type="button" class="v2-btn-secondary" on:click=move |_| on_navigate.run(PublishViewState::StorePage(listing_for_tools.clone()))>"Advanced store page tools"</button>
            </div>
        }
    });

    view! {
        <PublishView
            listing=listing
            initial_section=section
            overview_extra=overview_extra
            on_back=on_back
            on_published=Callback::new(move |listing| on_navigate.run(PublishViewState::game(listing)))
        />
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
                "The promotion is cancelled either way. Also remove it from the store page?"
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

    /// Presentation only. Dismissal policy is typed separately and is the same
    /// for every variant.
    fn tone(self) -> DialogTone {
        match self {
            Self::CancelCampaign | Self::DiscardChanges => DialogTone::Destructive,
            Self::RemovePointer => DialogTone::Neutral,
        }
    }

    /// The accept button follows the dialog tone: destructive decisions use
    /// the red destructive button, never the amber primary CTA.
    fn accept_class(self) -> &'static str {
        match self.tone() {
            DialogTone::Destructive => "v2-btn-danger",
            DialogTone::Neutral => "v2-btn-primary",
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

/// Campaign publication has two independent stages: the campaign event chain and
/// the advisory listing pointer. They are never collapsed into one success.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CampaignStage {
    NotAttempted,
    Pending,
    Complete,
    Failed,
}

impl CampaignStage {
    fn label(self) -> &'static str {
        match self {
            Self::NotAttempted => "Not attempted",
            Self::Pending => "In progress",
            Self::Complete => "Published",
            Self::Failed => "Failed",
        }
    }

    fn variant(self) -> StatusChipVariant {
        match self {
            Self::NotAttempted => StatusChipVariant::Neutral,
            Self::Pending => StatusChipVariant::Pending,
            Self::Complete => StatusChipVariant::Verified,
            Self::Failed => StatusChipVariant::Error,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct CampaignPublicationLifecycle {
    event: CampaignStage,
    pointer: CampaignStage,
    pointer_retryable: bool,
}

fn campaign_publication_lifecycle(
    submitting: bool,
    published_root_event_id: Option<&str>,
    pointer_requested: bool,
    pointer_error: Option<&str>,
) -> CampaignPublicationLifecycle {
    let event = match published_root_event_id {
        Some(id) if !id.trim().is_empty() => CampaignStage::Complete,
        Some(_) => CampaignStage::Failed,
        None if submitting => CampaignStage::Pending,
        None => CampaignStage::NotAttempted,
    };
    let pointer = if !pointer_requested {
        CampaignStage::NotAttempted
    } else if pointer_error.is_some() {
        CampaignStage::Failed
    } else if event == CampaignStage::Complete {
        CampaignStage::Complete
    } else if submitting {
        CampaignStage::Pending
    } else {
        CampaignStage::NotAttempted
    };
    CampaignPublicationLifecycle {
        event,
        pointer,
        // Only a published chain can have its advisory link retried.
        pointer_retryable: pointer == CampaignStage::Failed && event == CampaignStage::Complete,
    }
}

/// Overall wording. A published campaign chain whose advisory link failed is
/// never reported as a finished publication.
fn campaign_overall_label(lifecycle: CampaignPublicationLifecycle) -> &'static str {
    use CampaignStage::*;
    match (lifecycle.event, lifecycle.pointer) {
        (NotAttempted, _) => "Not published",
        (Pending, _) | (_, Pending) => "Publishing",
        (Failed, _) => "Promotion publication failed",
        (Complete, NotAttempted) => "Promotion published; not shown on the store page",
        (Complete, Complete) => "Promotion published and shown on the store page",
        (Complete, Failed) => {
            "Promotion published, but showing it on the store page failed. You can retry."
        }
    }
}

fn campaign_status_variant(classification: &str) -> StatusChipVariant {
    match classification {
        "upcoming" => StatusChipVariant::Pending,
        "active" => StatusChipVariant::Active,
        "ended" => StatusChipVariant::Expired,
        "cancelled" => StatusChipVariant::Cancelled,
        _ => StatusChipVariant::Warning,
    }
}

/// Advisory listing-pointer state for one campaign row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CampaignPointerState {
    Present,
    Stale,
    Missing,
}

fn campaign_pointer_state(points_here: bool, live: bool) -> CampaignPointerState {
    match (points_here, live) {
        (true, true) => CampaignPointerState::Present,
        (true, false) => CampaignPointerState::Stale,
        (false, _) => CampaignPointerState::Missing,
    }
}

impl CampaignPointerState {
    fn label(self) -> &'static str {
        match self {
            Self::Present => "Shown on store page",
            Self::Stale => "Store page link out of date",
            Self::Missing => "Not shown on store page",
        }
    }

    fn variant(self) -> StatusChipVariant {
        match self {
            Self::Present => StatusChipVariant::Verified,
            Self::Stale => StatusChipVariant::Warning,
            Self::Missing => StatusChipVariant::Neutral,
        }
    }
}

/// The declared dismissal contract for the campaign confirmation dialog.
///
/// Every dismissal channel means "no decision", which is exactly the signal
/// `on_decision(None)` already carried.
fn campaign_confirmation_contract() -> (DialogClosePolicy, DialogDismissal) {
    (
        DialogClosePolicy::Dismissible,
        DialogDismissal::freely_dismissible(),
    )
}

#[component]
fn CampaignConfirmationDialog(
    confirmation: RwSignal<Option<CampaignConfirmation>>,
    on_decision: Callback<Option<bool>>,
) -> impl IntoView {
    let reject_ref = NodeRef::<leptos::html::Button>::new();

    view! {
        <Dialog
            id="campaign-confirmation"
            open=Signal::derive(move || confirmation.get().is_some())
            title=Signal::derive(move || confirmation.get().map(CampaignConfirmation::title).unwrap_or_default().to_string())
            kicker="Promotion"
            description=Signal::derive(move || confirmation.get().map(CampaignConfirmation::message).unwrap_or_default().to_string())
            width=DialogWidth::Compact
            tone=Signal::derive(move || confirmation.get().map(CampaignConfirmation::tone).unwrap_or_default())
            policy=campaign_confirmation_contract().0
            dismissal=campaign_confirmation_contract().1
            initial_focus=DialogInitialFocus::Button(reject_ref)
            close_label="Close without deciding"
            on_close=UnsyncCallback::new(move |request: DialogCloseRequest| {
                // Unchanged campaign semantics: any dismissal is the same
                // "no decision" signal the native cancel event used to send.
                if request.action == DialogCloseAction::Dismiss {
                    on_decision.run(None);
                }
            })
            actions=move || view! {
                <button
                    node_ref=reject_ref
                    type="button"
                    class="v2-btn-secondary"
                    on:click=move |_| on_decision.run(Some(false))
                >
                    {move || confirmation.get().map(CampaignConfirmation::reject_label).unwrap_or_default()}
                </button>
                <button
                    type="button"
                    class=move || confirmation.get().map(CampaignConfirmation::accept_class).unwrap_or("v2-btn-primary")
                    on:click=move |_| on_decision.run(Some(true))
                >
                    {move || confirmation.get().map(CampaignConfirmation::accept_label).unwrap_or_default()}
                </button>
            }
        />
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
    let action_account = RwSignal::new((auth.npub.get_untracked(), auth.current_auth_flow()));
    let auth_for_epoch = auth.clone();
    Effect::new(move |_| {
        let current = (
            auth_for_epoch.npub.get(),
            auth_for_epoch.current_auth_flow(),
        );
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
                "Switch to the account that published this game".into(),
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
        let auth_epoch = auth_for_request.current_auth_flow();
        action_in_progress.set(true);
        spawn_local(async move {
            let result = invoke_update_campaign_pointer(request).await;
            if action_generation.try_get_untracked().is_none() {
                return;
            }
            if !auth_for_request.is_current_auth_flow(auth_epoch)
                || !accepts_account_response(
                    auth_for_request.npub.get_untracked().as_deref(),
                    &publisher_npub,
                    action_generation.try_get_untracked(),
                    request_generation,
                )
            {
                action_in_progress.set(false);
                action_completed.set(true);
                pointer_message.set(Some("Account changed while the Promotion link was updating. The stale response was ignored; refresh this game before another change.".into()));
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
                    navigate.run(PublishViewState::game(updated));
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
                "Switch to the account that published this game".into(),
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
        let auth_epoch = auth_for_request.current_auth_flow();
        spawn_local(async move {
            let result = invoke_publish_campaign(request).await;
            if action_generation.try_get_untracked().is_none() {
                return;
            }
            if !auth_for_request.is_current_auth_flow(auth_epoch)
                || !accepts_account_response(
                    auth_for_request.npub.get_untracked().as_deref(),
                    &publisher_npub,
                    action_generation.try_get_untracked(),
                    request_generation,
                )
            {
                action_in_progress.set(false);
                action_completed.set(true);
                cancel_message.set(Some("Account changed while cancellation was being signed. The stale response was ignored; refresh this game to reconcile the Promotion state.".into()));
                return;
            }
            match result {
                Ok(response) => {
                    let pointer_failed = response.pointer_update_error.is_some();
                    action_completed.set(true);
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
                        navigate.run(PublishViewState::game(updated.unwrap_or(listing)));
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
                <div class="v2-publisher-row-statuses"><strong class="v2-campaign-id">{short_identifier(&campaign.campaign_id)}</strong><StatusChip label=status variant=campaign_status_variant(&campaign.classification) icon=None size=StatusChipSize::Compact />{ let pointer = campaign_pointer_state(points_here, is_upcoming || is_active); view! { <StatusChip label=pointer.label() variant=pointer.variant() icon=None size=StatusChipSize::Compact /> } }</div>
                <p class="v2-campaign-window">{format!("{} to {}", format_unix(campaign.starts_at), format_unix(campaign.ends_at))}</p>
                <p class="v2-campaign-mode">{campaign_mode_label(&campaign.mode)}</p>
                <details class="v2-publisher-diagnostics"><summary>"Technical details"</summary><p>{format!("Campaign root event: {} | current event: {}", short_identifier(&campaign.root_event_id), short_identifier(campaign.event_id.as_deref().unwrap_or("unresolved")))}</p></details>
            </div>
            <div class="v2-campaign-actions">
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
            {move || action_completed.get().then(|| view! { <p class="v2-campaign-note v2-campaign-note-ok">"Requested operation completed"</p> })}
            {move || cancel_message.get().map(|message| view! { <p class="v2-campaign-note">{message}</p> })}
            {move || pointer_message.get().map(|message| view! { <p class="v2-campaign-note">{message}</p> })}
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
    let published_root_event_id = RwSignal::new(None::<String>);
    let message = RwSignal::new(None::<String>);
    let error = RwSignal::new(None::<String>);
    let operation_generation = RwSignal::new(0_u64);
    let pointer_retry = RwSignal::new(None::<(String, bool)>);
    let operation_account = RwSignal::new((auth.npub.get_untracked(), auth.current_auth_flow()));
    let auth_for_epoch = auth.clone();
    Effect::new(move |_| {
        let current = (
            auth_for_epoch.npub.get(),
            auth_for_epoch.current_auth_flow(),
        );
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
                "Switch to the account that published this game".into(),
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
        let auth_epoch = auth_for_request.current_auth_flow();
        spawn_local(async move {
            let result = invoke_publish_campaign(request).await;
            if operation_generation.try_get_untracked().is_none() {
                return;
            }
            if !auth_for_request.is_current_auth_flow(auth_epoch)
                || !accepts_account_response(
                    auth_for_request.npub.get_untracked().as_deref(),
                    &publisher_npub,
                    operation_generation.try_get_untracked(),
                    request_generation,
                )
            {
                submitting.set(false);
                completed.set(true);
                error.set(Some("Account changed during Promotion publication. The stale response was ignored; return to the game and refresh before retrying.".into()));
                return;
            }
            match result {
                Ok(response) => {
                    completed.set(true);
                    // Presentation bookkeeping only: records which chain event was
                    // accepted so the publication panel can keep the campaign and
                    // pointer stages separate.
                    published_root_event_id.set(Some(response.root_event_id.clone()));
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
                        if operation_generation.try_get_untracked().is_none() {
                            return;
                        }
                        if !auth_for_request.is_current_auth_flow(auth_epoch)
                            || !accepts_account_response(
                                auth_for_request.npub.get_untracked().as_deref(),
                                &publisher_for_pointer_removal,
                                operation_generation.try_get_untracked(),
                                request_generation,
                            )
                        {
                            submitting.set(false);
                            completed.set(true);
                            error.set(Some("Account changed while the Promotion link was updating. The stale response was ignored; return to the game and refresh.".into()));
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
                "Switch to the account that published this game".into(),
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
        let auth_epoch = auth_for_request.current_auth_flow();
        spawn_local(async move {
            let result = invoke_publish_campaign(request).await;
            if operation_generation.try_get_untracked().is_none() {
                return;
            }
            if !auth_for_request.is_current_auth_flow(auth_epoch)
                || !accepts_account_response(
                    auth_for_request.npub.get_untracked().as_deref(),
                    &publisher_npub,
                    operation_generation.try_get_untracked(),
                    request_generation,
                )
            {
                submitting.set(false);
                completed.set(true);
                error.set(Some("Account changed during Promotion cancellation. The stale response was ignored; return to the game and refresh before another change.".into()));
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
            error.set(Some(
                "Sign in with the account that published this game".into(),
            ));
            return;
        };
        if publisher_npub != listing_for_retry.publisher_npub {
            error.set(Some(
                "Switch to the account that published this game".into(),
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
        let auth_epoch = auth_for_request.current_auth_flow();
        let listing = listing_for_retry.clone();
        let on_saved = on_saved_for_retry.clone();
        submitting.set(true);
        error.set(None);
        spawn_local(async move {
            let result = invoke_update_campaign_pointer(request).await;
            if operation_generation.try_get_untracked().is_none() {
                return;
            }
            if !auth_for_request.is_current_auth_flow(auth_epoch)
                || !accepts_account_response(
                    auth_for_request.npub.get_untracked().as_deref(),
                    &publisher_npub,
                    operation_generation.try_get_untracked(),
                    request_generation,
                )
            {
                submitting.set(false);
                completed.set(true);
                error.set(Some("Account changed while the Promotion link was updating. The stale response was ignored; return to the game and refresh.".into()));
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
            <button class="v2-btn-secondary v2-publisher-back" on:click=move |_| back.run(())>"Back to game"</button>
            <header class="v2-publisher-game-hero">
                <div class="v2-publisher-manage-art"><GameArtwork title=listing.title.clone() state=artwork_state_from_url(valid_cover_url(&listing.images)) role=ArtworkRole::Thumbnail /></div>
                <div><p class="v2-publisher-kicker">{if editing { "Promotion details" } else { "New Promotion" }}</p><h1>{format!("{} for {}", if editing { "Promotion" } else { "Create a Promotion" }, listing.title)}</h1>{campaign.as_ref().map(|item| view! { <div class="v2-publisher-row-statuses"><StatusChip label=campaign_status(&item.classification) variant=campaign_status_variant(&item.classification) icon=None size=StatusChipSize::Compact /></div> })}</div>
            </header>
            <div class="v2-publisher-management-layout">
            <main class="v2-publisher-main">
            <section class="v2-publisher-panel v2-publisher-form">
                <div class="v2-publisher-authority" role="note"><strong>"Only you can manage promotions"</strong><p>"Only the account that published this game can create, edit or cancel its promotions."</p></div>
                {terms_read_only.then(|| view! { <p class="v2-publisher-readonly" role="status">"Active, Ended, and Cancelled Promotion terms are immutable. This view is read-only."</p> })}
                <div><label for="campaign-id">"Promotion ID"</label><input id="campaign-id" class="v2-input" readonly=true prop:value=move || form.get().campaign_id /></div>
                <div><h2>"Claim and keep"</h2><div class="v2-publisher-option"><strong>"Free Claim and keep"</strong><p>"People may claim before the exclusive end time and keep durable access permanently."</p></div><p class="v2-campaign-help">"To make the game free for everyone for a while, use Price & access instead."</p></div>
                <DateTimeRangePicker
                    starts_at=Signal::derive(move || form.get().starts_at)
                    ends_at=Signal::derive(move || form.get().ends_at)
                    on_starts_at=Callback::new(move |value| form.update(|current| current.starts_at = value))
                    on_ends_at=Callback::new(move |value| form.update(|current| current.ends_at = value))
                    disabled=Signal::derive(move || terms_read_only || completed.get())
                />
                <p class="v2-campaign-help">"Times use your local timezone. Claims at or after the end are not accepted. Local timezone: "{timezone_label()}</p>
                {move || live_validation.get().map(|text| view! { <p class="v2-campaign-blocker" role="alert">{text}</p> })}
                <div class="v2-publisher-link-option"><label><input type="checkbox" disabled=move || terms_read_only || completed.get() prop:checked=move || form.get().update_listing_pointer on:change:target=move |event| form.update(|current| current.update_listing_pointer = event.target().checked()) /><span><strong>"Show this promotion on the store page"</strong><span>"Recommended. Helps players find the promotion; it works either way."</span></span></label></div>
                {move || error.get().map(|text| view! { <p class="v2-campaign-blocker" role="alert">{text}</p> })}
                {move || message.get().map(|text| view! { <p class="v2-campaign-note" role="status">{text}</p> })}
                {move || {
                    let lifecycle = campaign_publication_lifecycle(
                        submitting.get(),
                        completed.get().then(|| published_root_event_id.get()).flatten().as_deref(),
                        form.get().update_listing_pointer,
                        pointer_retry.get().is_some().then_some("pointer update failed"),
                    );
                    (lifecycle.event != CampaignStage::NotAttempted).then(|| view! {
                        <section class="v2-campaign-publication" aria-labelledby="campaign-publication-title">
                            <h3 id="campaign-publication-title">"Publication status"</h3>
                            <p class="v2-campaign-overall">{campaign_overall_label(lifecycle)}</p>
                            <dl class="v2-campaign-stage-grid">
                                <div><dt>"Promotion chain event"</dt><dd><StatusChip label=lifecycle.event.label() variant=lifecycle.event.variant() icon=None size=StatusChipSize::Compact /></dd></div>
                                <div><dt>"Game page link"</dt><dd><StatusChip label=lifecycle.pointer.label() variant=lifecycle.pointer.variant() icon=None size=StatusChipSize::Compact /></dd></div>
                            </dl>
                            {lifecycle.pointer_retryable.then(|| view! { <p class="v2-campaign-help">"The Promotion itself is authoritative and already published. Only the advisory Game page link needs retrying."</p> })}
                        </section>
                    })
                }}
                <div class="v2-publisher-actions v2-publisher-actions-end"><button class="v2-btn-secondary" on:click=move |_| back.run(())>{move || if terms_read_only { "Close" } else if completed.get() { "Back to game" } else { "Discard changes" }}</button>{move || completed.get().then(|| view! { <StatusChip label="Requested operation completed" variant=StatusChipVariant::Verified icon=None size=StatusChipSize::Compact /> })}{move || { let retry = retry_pointer.clone(); pointer_retry.get().is_some().then(move || view! { <button class="v2-btn-secondary" disabled=move || submitting.get() on:click=move |_| retry.run(())>"Retry Promotion link"</button> }) }}{if cancellable { view! { <button class="v2-btn-secondary" disabled=move || submitting.get() || completed.get() on:click=cancel>"Cancel Promotion"</button> }.into_any() } else { view! { <></> }.into_any() }}{if !terms_read_only { view! { <button class="v2-btn-primary" disabled=move || submitting.get() || completed.get() || live_validation.get().is_some() on:click=save>{move || if completed.get() { "Completed" } else if submitting.get() { "Publishing..." } else { "Publish Promotion" }}</button> }.into_any() } else { view! { <></> }.into_any() }}</div>
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
        AcquisitionPolicy::Public => "Free".into(),
        AcquisitionPolicy::Gated => "Paid".into(),
        AcquisitionPolicy::TimedAccess { .. } => "Free for a limited time".into(),
    }
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
    fn destructive_campaign_decisions_use_the_destructive_button() {
        assert_eq!(
            CampaignConfirmation::CancelCampaign.accept_class(),
            "v2-btn-danger"
        );
        assert_eq!(
            CampaignConfirmation::DiscardChanges.accept_class(),
            "v2-btn-danger"
        );
        assert_eq!(
            CampaignConfirmation::RemovePointer.accept_class(),
            "v2-btn-primary"
        );
    }

    #[test]
    fn unreachable_network_and_other_failures_get_distinct_copy() {
        use arcadestr_core::marketplace_fetch::MarketplaceFetchError;
        let unreachable = publisher_load_error_copy(&MarketplaceFetchError::Unreachable {
            failed: 6,
            timed_out: 3,
        });
        assert_eq!(unreachable.0, "No relays responded");
        assert!(unreachable.1.contains("try again"));
        let failed =
            publisher_load_error_copy(&MarketplaceFetchError::failed("connection refused"));
        assert_eq!(failed.0, "Your published games could not be loaded");
        // A failure whose text merely resembles the unreachable message is not
        // promoted to it.
        let forged = publisher_load_error_copy(&MarketplaceFetchError::failed(
            "No relay responded: 1 failed, 0 timed out",
        ));
        assert_eq!(forged.0, "Your published games could not be loaded");
    }

    #[test]
    fn every_campaign_dismissal_channel_means_no_decision() {
        use crate::ui_v2::components::{resolve_close, DialogCloseAction, DialogCloseSource};

        let (policy, dismissal) = campaign_confirmation_contract();
        for source in [
            DialogCloseSource::Escape,
            DialogCloseSource::Backdrop,
            DialogCloseSource::CloseButton,
        ] {
            assert_eq!(
                resolve_close(policy, dismissal, false, source),
                DialogCloseAction::Dismiss,
                "{source:?} should cancel the decision"
            );
        }
        // A cancelled decision is still no decision: prior claims are never
        // revoked and the campaign is untouched.
        assert_eq!(
            resolve_confirmation(CampaignConfirmation::CancelCampaign, None, true),
            ConfirmationOutcome::Close
        );
    }

    #[test]
    fn the_safe_rejection_action_receives_initial_focus() {
        let source = include_str!("publish.rs");
        assert!(source.contains("initial_focus=DialogInitialFocus::Button(reject_ref)"));
        // The rejection label is the safe one for every variant.
        assert_eq!(
            CampaignConfirmation::CancelCampaign.reject_label(),
            "Keep Promotion"
        );
    }

    #[test]
    fn prospective_campaign_cancellation_wording_is_unchanged() {
        assert_eq!(
            CampaignConfirmation::CancelCampaign.message(),
            "New claims stop immediately. Prior claims remain valid; campaign cancellation does not revoke prior claims."
        );
    }

    const TEST_PUBLISHER: &str = "npub1kvl9ev2wcjdecvyk0xhqacdwa505fqn6zpqwmwpd7vn4p565amyqesnwt4";

    fn sample_listing() -> GameListing {
        let mut listing = serde_json::from_value::<GameListing>(serde_json::json!({
            "id": "sample-game",
            "source": "nip99_listing",
            "title": "Sample Game",
            "description": "A complete signed listing.",
            "publisher_npub": TEST_PUBLISHER,
            "event_id": "event-id",
            "created_at": 100
        }))
        .expect("sample listing");
        listing.acquisition = AcquisitionPolicy::Public;
        listing
    }

    fn sample_campaign(classification: &str) -> DiscoveredCampaign {
        DiscoveredCampaign {
            root_event_id: format!("root-{classification}"),
            campaign_id: format!("campaign-{classification}"),
            starts_at: 10,
            ends_at: 20,
            classification: classification.to_string(),
            event_id: Some(format!("event-{classification}")),
            predecessor_event_id: None,
            mode: "claim_and_keep".to_string(),
        }
    }

    fn sample_account(npub: &str, signing_mode: &str) -> crate::StoredAccount {
        crate::StoredAccount {
            id: "account".to_string(),
            npub: npub.to_string(),
            name: None,
            signing_mode: signing_mode.to_string(),
            last_used: 0,
            is_current: true,
            picture: None,
            display_name: None,
            username: None,
            nip05: None,
            about: None,
        }
    }

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

    #[test]
    fn dashboard_lists_only_unpublished_local_game_drafts() {
        use arcadestr_core::publisher_recovery::PublisherDraft;
        let draft = |id: &str, kind: &str, payload: serde_json::Value| PublisherDraft {
            draft_id: id.into(),
            kind: kind.into(),
            payload,
            revision: 3,
        };
        let drafts = [
            draft(
                "local:one",
                "listing",
                serde_json::json!({"form": {"title": "  Neon Drifter "}}),
            ),
            draft(
                "local:two",
                "listing",
                serde_json::json!({"form": {"title": ""}}),
            ),
            draft("local:three", "listing", serde_json::json!("not an object")),
            draft(
                "listing:live",
                "listing",
                serde_json::json!({"form": {"title": "Live"}}),
            ),
            draft("local:page", "store_page", serde_json::json!({})),
        ];
        assert_eq!(
            local_game_drafts(&drafts),
            vec![
                LocalGameDraft {
                    draft_id: "local:one".into(),
                    title: "Neon Drifter".into(),
                    revision: 3,
                },
                LocalGameDraft {
                    draft_id: "local:two".into(),
                    title: "Untitled game".into(),
                    revision: 3,
                },
                LocalGameDraft {
                    draft_id: "local:three".into(),
                    title: "Untitled game".into(),
                    revision: 3,
                },
            ]
        );
        assert_eq!(
            games_with_local_changes(&drafts),
            HashSet::from(["live".to_string()])
        );
    }

    #[test]
    fn live_games_open_on_their_overview() {
        let listing = sample_listing();
        assert!(matches!(
            PublishViewState::game(listing),
            PublishViewState::Game {
                section: HubSection::Overview,
                ..
            }
        ));
    }

    #[test]
    fn signed_out_loading_empty_error_and_partial_states_are_distinct() {
        assert_eq!(
            publisher_dashboard_state(false, false, false, false, false),
            PublisherDashboardState::SignedOut
        );
        assert_eq!(
            publisher_dashboard_state(true, false, false, false, false),
            PublisherDashboardState::Loading
        );
        assert_eq!(
            publisher_dashboard_state(false, true, true, false, false),
            PublisherDashboardState::Loading
        );
        assert_eq!(
            publisher_dashboard_state(false, true, false, false, true),
            PublisherDashboardState::Error
        );
        assert_eq!(
            publisher_dashboard_state(false, true, true, true, true),
            PublisherDashboardState::Partial
        );
        assert_eq!(
            publisher_dashboard_state(false, true, false, false, false),
            PublisherDashboardState::Empty
        );
    }

    #[test]
    fn signer_availability_is_bound_to_the_active_account() {
        let local = sample_account(TEST_PUBLISHER, "local");
        assert_eq!(
            publisher_signer_state(Some(TEST_PUBLISHER), Some(&local), "disconnected"),
            PublisherSignerState::Available
        );
        let remote = sample_account(TEST_PUBLISHER, "nip46");
        assert_eq!(
            publisher_signer_state(Some(TEST_PUBLISHER), Some(&remote), "connected"),
            PublisherSignerState::Available
        );
        assert_eq!(
            publisher_signer_state(Some(TEST_PUBLISHER), Some(&remote), "disconnected"),
            PublisherSignerState::Unavailable
        );
        assert_eq!(
            publisher_signer_state(Some("npub1other"), Some(&local), "connected"),
            PublisherSignerState::Unknown
        );
        assert!(!signer_can_publish(PublisherSignerState::Connecting));
    }

    #[test]
    fn stale_publisher_responses_are_rejected() {
        assert!(accepts_account_response(
            Some(TEST_PUBLISHER),
            TEST_PUBLISHER,
            Some(4),
            4
        ));
        assert!(!accepts_account_response(
            Some("npub1other"),
            TEST_PUBLISHER,
            Some(4),
            4
        ));
        assert!(!accepts_account_response(
            Some(TEST_PUBLISHER),
            TEST_PUBLISHER,
            Some(5),
            4
        ));
    }

    #[test]
    fn local_store_page_draft_does_not_claim_listing_publication() {
        let listing = sample_listing();
        assert!(listing.event_id.is_some());
        assert_eq!(
            store_page_summary_state(Some(&StorePageEnrichmentState::NotAssociated), false),
            StorePageSummaryState::NotAssociated
        );
    }

    #[test]
    fn listing_validity_reports_exact_actionable_reasons() {
        assert_eq!(
            listing_validity(&sample_listing()),
            ListingValidityState::Valid
        );
        let mut invalid = sample_listing();
        invalid.id = "INVALID ID".to_string();
        invalid.title.clear();
        invalid.event_id = None;
        let ListingValidityState::Invalid(reasons) = listing_validity(&invalid) else {
            panic!("listing should be invalid");
        };
        assert!(reasons.iter().any(|reason| reason.contains("identifier")));
        assert!(reasons.iter().any(|reason| reason.contains("Title")));
        assert!(reasons
            .iter()
            .any(|reason| reason.contains("confirmed on the network")));
    }

    #[test]
    fn store_page_states_do_not_collapse_unavailable_into_missing() {
        assert_eq!(
            store_page_summary_state(None, true),
            StorePageSummaryState::Loading
        );
        assert_eq!(
            store_page_summary_state(None, false),
            StorePageSummaryState::Unavailable
        );
        assert_eq!(
            store_page_summary_state(Some(&StorePageEnrichmentState::NotAssociated), false),
            StorePageSummaryState::NotAssociated
        );
        assert_eq!(
            store_page_summary_state(Some(&StorePageEnrichmentState::NotFound), false),
            StorePageSummaryState::NotFound
        );
        assert_eq!(
            store_page_summary_state(Some(&StorePageEnrichmentState::Invalid), false),
            StorePageSummaryState::Invalid
        );
        assert_eq!(
            store_page_summary_state(Some(&StorePageEnrichmentState::Unavailable), false),
            StorePageSummaryState::Unavailable
        );
    }

    #[test]
    fn release_state_requires_version_and_valid_hash_only_when_configured() {
        assert_eq!(
            release_summary_state(&sample_listing()),
            ReleaseSummaryState::NotConfigured
        );
        let mut configured = sample_listing();
        configured.specs = vec![
            ("server".into(), "https://distribution.example".into()),
            ("version".into(), "1.2.3".into()),
            ("file_hash".into(), "a".repeat(64)),
        ];
        assert_eq!(
            release_summary_state(&configured),
            ReleaseSummaryState::Current {
                version: "1.2.3".into()
            }
        );
        configured
            .specs
            .iter_mut()
            .find(|(key, _)| key == "file_hash")
            .expect("file_hash fixture exists")
            .1 = "A".repeat(64);
        assert!(matches!(
            release_summary_state(&configured),
            ReleaseSummaryState::Invalid(_)
        ));
        configured.specs.retain(|(key, _)| key != "file_hash");
        assert!(matches!(
            release_summary_state(&configured),
            ReleaseSummaryState::Invalid(_)
        ));
    }

    #[test]
    fn campaign_lifecycle_and_unavailable_states_remain_distinct() {
        let campaigns = vec![
            sample_campaign("active"),
            sample_campaign("ended"),
            sample_campaign("cancelled"),
        ];
        let label = campaign_summary_label(&CampaignDashboardState::Resolved(campaigns));
        assert!(label.contains("1 active"));
        assert!(label.contains("1 ended"));
        assert!(label.contains("1 cancelled"));
        assert_eq!(
            campaign_summary_label(&CampaignDashboardState::Resolved(Vec::new())),
            "Promotions: none"
        );
        assert_eq!(
            campaign_summary_label(&CampaignDashboardState::Unavailable),
            "Promotions: couldn't be loaded"
        );
    }

    #[test]
    fn partial_publication_and_invalid_campaign_create_exact_attention() {
        let listing = sample_listing();
        let campaign = DiscoveredCampaign {
            event_id: None,
            mode: "unsupported".into(),
            ..sample_campaign("invalid")
        };
        let items = listing_attention_items(
            &listing,
            StorePageSummaryState::Published,
            &ReleaseSummaryState::NotConfigured,
            &CampaignDashboardState::Resolved(vec![campaign]),
            true,
        );
        assert!(items
            .iter()
            .any(|item| item.reason.contains("store page update didn't finish")));
        assert!(items
            .iter()
            .any(|item| item.reason.contains("invalid or incomplete")));
        assert!(items
            .iter()
            .any(|item| item.reason.contains("doesn't support")));
        assert!(items
            .iter()
            .any(|item| item.reason.contains("promotion hasn't been confirmed")));
        let groups = group_attention_items(items);
        assert_eq!(groups.len(), 1);
        assert!(groups[0].reasons.len() >= 4);
    }

    #[test]
    fn authored_listing_validation_excludes_legacy_and_other_publishers() {
        let authored = sample_listing();
        let mut legacy = authored.clone();
        legacy.id = "legacy".into();
        legacy.source = ListingSource::Legacy;
        let mut other = authored.clone();
        other.id = "other".into();
        other.publisher_npub = "npub1other".into();
        let result = current_user_listings(vec![authored.clone(), legacy, other], TEST_PUBLISHER);
        assert_eq!(result, vec![authored]);
    }

    #[test]
    fn unsupported_metrics_and_fixture_data_are_omitted() {
        let source = include_str!("publish.rs");
        for fabricated in [
            ["Revenue", ": 0"].concat(),
            ["Sales", ": 0"].concat(),
            ["Views", ": 0"].concat(),
            ["Nova", " Choir"].concat(),
            ["Echoes", " of Aster"].concat(),
            ["~340", " claims"].concat(),
        ] {
            assert!(!source.contains(&fabricated));
        }
    }

    #[test]
    fn missing_artwork_uses_the_shared_deterministic_fallback_state() {
        assert_eq!(
            artwork_state_from_url(valid_cover_url(&sample_listing().images)),
            crate::ui_v2::components::ArtworkState::Missing
        );
    }

    #[test]
    fn campaign_event_and_pointer_publication_never_collapse() {
        // Chain published, advisory link published.
        let both = campaign_publication_lifecycle(false, Some("root-1"), true, None);
        assert_eq!(both.event, CampaignStage::Complete);
        assert_eq!(both.pointer, CampaignStage::Complete);
        assert_eq!(
            campaign_overall_label(both),
            "Promotion published and shown on the store page"
        );
        assert!(!both.pointer_retryable);

        // Chain published, link failed: never reported as a finished publication.
        let link_failed = campaign_publication_lifecycle(false, Some("root-1"), true, Some("boom"));
        assert_eq!(link_failed.event, CampaignStage::Complete);
        assert_eq!(link_failed.pointer, CampaignStage::Failed);
        assert!(link_failed.pointer_retryable);
        assert_eq!(
            campaign_overall_label(link_failed),
            "Promotion published, but showing it on the store page failed. You can retry."
        );

        // No link requested is not a link failure.
        let no_link = campaign_publication_lifecycle(false, Some("root-1"), false, None);
        assert_eq!(no_link.pointer, CampaignStage::NotAttempted);
        assert!(!no_link.pointer_retryable);

        // Busy and untouched states stay distinct from any result.
        assert_eq!(
            campaign_publication_lifecycle(true, None, true, None).event,
            CampaignStage::Pending
        );
        assert_eq!(
            campaign_publication_lifecycle(false, None, true, None).event,
            CampaignStage::NotAttempted
        );
        assert_eq!(
            campaign_overall_label(campaign_publication_lifecycle(false, None, false, None)),
            "Not published"
        );

        // An empty chain id is a failure, never a silent success.
        assert_eq!(
            campaign_publication_lifecycle(false, Some("  "), false, None).event,
            CampaignStage::Failed
        );
    }

    #[test]
    fn campaign_lifecycle_states_map_to_distinct_chips() {
        for (classification, expected) in [
            ("upcoming", StatusChipVariant::Pending),
            ("active", StatusChipVariant::Active),
            ("ended", StatusChipVariant::Expired),
            ("cancelled", StatusChipVariant::Cancelled),
            ("garbage", StatusChipVariant::Warning),
        ] {
            assert_eq!(campaign_status_variant(classification), expected);
        }
        // Status text and chip variant must not disagree about lifecycle.
        assert_eq!(campaign_status("cancelled"), "Cancelled");
        assert_eq!(campaign_status("garbage"), "Invalid/incomplete");
    }

    #[test]
    fn advisory_pointer_state_is_separate_from_campaign_validity() {
        assert_eq!(
            campaign_pointer_state(true, true),
            CampaignPointerState::Present
        );
        assert_eq!(
            campaign_pointer_state(true, false),
            CampaignPointerState::Stale
        );
        assert_eq!(
            campaign_pointer_state(false, true),
            CampaignPointerState::Missing
        );
        for state in [
            CampaignPointerState::Present,
            CampaignPointerState::Stale,
            CampaignPointerState::Missing,
        ] {
            // The advisory link never claims to determine campaign validity.
            assert!(!state.label().to_ascii_lowercase().contains("valid"));
        }
    }

    #[test]
    fn cancellation_is_presented_as_prospective_not_deletion() {
        let message = CampaignConfirmation::CancelCampaign.message();
        assert!(message.contains("Prior claims remain valid"));
        assert!(message.contains("does not revoke prior claims"));
        for forbidden in ["delete", "erase", "revoke all", "remove history"] {
            assert!(
                !message.to_ascii_lowercase().contains(forbidden),
                "cancellation wording implies destruction: {forbidden}"
            );
        }
        assert_eq!(
            CampaignConfirmation::CancelCampaign.accept_label(),
            "Cancel Promotion"
        );
    }

    #[test]
    fn publisher_activity_is_never_simulated() {
        let source = include_str!("publish.rs");
        for fabricated in [
            ["No activity", " yet"].concat(),
            ["Recent", " activity"].concat(),
            ["activity", " feed"].concat(),
        ] {
            assert!(!source.contains(&fabricated), "simulated activity surface");
        }
    }

    #[test]
    fn unsupported_campaign_actions_are_absent() {
        let source = include_str!("publish.rs");
        for absent in [
            ["Duplicate", " Promotion"].concat(),
            ["Pause", " Promotion"].concat(),
            ["Resume", " Promotion"].concat(),
            ["Export", " claims"].concat(),
            ["Delete", " history"].concat(),
        ] {
            assert!(!source.contains(&absent), "unsupported action rendered");
        }
    }

    #[test]
    fn managed_game_omits_unsupported_actions_and_gates_publication_on_a_signer() {
        let source = include_str!("publish.rs");
        for absent in [
            ["Unlist", " game"].concat(),
            ["Disable", " public/timed access"].concat(),
            ["Revoke", " individual access"].concat(),
            ["DANGER", " ZONE"].concat(),
        ] {
            assert!(!source.contains(&absent), "unsupported action rendered");
        }
        // Promotions still require a signer; editing a game never does.
        assert!(source.contains("disabled=move || !signer_can_publish(signer_state.get())"));
    }
}
