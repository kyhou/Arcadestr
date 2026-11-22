// ADP publish view component.

use leptos::prelude::*;
use wasm_bindgen_futures::spawn_local;

use arcadestr_core::adp_publish_contract::{
    PublishAdpListingError, PublishAdpListingRequestExtension,
};
use arcadestr_core::adp_settlement::{
    FeePolicy, PublisherCommercialIntent, PublisherTermsContext, PublisherTermsReview,
};
use arcadestr_core::archive_inspection_contract::ArchiveSelectionError;
use arcadestr_core::distribution::ProfileId;
use arcadestr_core::is_sha256_hex;
use arcadestr_core::store_page::StorePageDraft;

use crate::campaign_management::datetime_local_to_unix;
use crate::models::{AcquisitionPolicy, CampaignPointer, ListingSource, StorePageListingRef};
use crate::publish_state::{
    partial_retry_payload, prepare_publish, preserve_edited_platforms, ArchiveSelectionState,
    AuthoritativeEditModel, AuthoritativeEditState, DefectReviewState, LaunchMode, PreparedPublish,
    ProviderBuildFields, PublishAttempt, PublishDirtySnapshot, PublishIdentityOverlay,
    PublishState, Transition,
};
use crate::tauri_bridge::{
    invoke_check_adp_server, invoke_discard_archive_selection, invoke_discover_adp_servers,
    invoke_hash_selected_archive, invoke_inspect_archive,
    invoke_load_authoritative_adp_edit_metadata, invoke_load_publisher_store_page_editor,
    invoke_publish_adp_listing_with_contract, invoke_resolve_adp_operator,
    invoke_select_archive_build, listen_publish_progress, AdpServerAnnouncement,
    ArchiveAccountFailure, ArchiveRequestId, BuildSelectionRef, CampaignPointerInput,
    DiscardArchiveSelectionRequest, FulfillmentMode, HashSelectedArchiveRequest,
    InspectArchiveRequest, LoadAuthoritativeAdpEditMetadataRequest, PublishAdpListingRequest,
    PublishAdpListingResult, PublishProgressPayload, ResolveAdpOperatorRequest,
    SelectArchiveRequest,
};
use crate::ui_v2::components::aria::aria_bool;
use crate::ui_v2::components::blossom_media_upload::publisher_hex;
use crate::ui_v2::components::{
    Button, ButtonVariant, Dialog, DialogCloseAction, DialogClosePolicy, DialogCloseRequest,
    DialogDismissal, DialogInitialFocus, DialogTone, DialogWidth, GameCard, GameCardPresentation,
    PlatformCompatibility, StatusChip, StatusChipSize, StorePageRichDetail,
};
use crate::ui_v2::views::publish::{
    publisher_signer_state, signer_can_publish, PublisherSignerState,
};
use crate::ui_v2::views::store_page_publish::{embedded_readiness, StorePageContentEditor};
use crate::{AuthContext, GameListing};

use super::date_time_picker::DateTimeRangePicker;

/// Distribution provider pre-filled for new publications. Only a build that
/// explicitly configures one gets a default; otherwise the publisher must
/// choose a provider, so no build silently targets a local development server.
const DEFAULT_ADP_SERVER_URL: Option<&str> = option_env!("ARCADESTR_DEFAULT_ADP_SERVER_URL");

fn configured_default_provider(configured: Option<&'static str>) -> Option<&'static str> {
    configured.map(str::trim).filter(|url| !url.is_empty())
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ServerEntry {
    url: String,
    label: String,
    reachability: ServerStatus,
    upload: ServerStatus,
    auto_operator: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ServerStatus {
    Idle,
    Pending,
    Ok,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
enum AcquisitionKind {
    Gated,
    Public,
    TimedAccess,
}

/// A part of the game hub. `Overview` is the hub itself; the other sections are
/// the independently editable parts a game is assembled from, opened in any order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum HubSection {
    #[default]
    Overview,
    Basics,
    StorePage,
    PriceAccess,
    Delivery,
}

impl HubSection {
    pub(crate) const EDITABLE: [Self; 4] = [
        Self::Basics,
        Self::StorePage,
        Self::PriceAccess,
        Self::Delivery,
    ];

    /// The section that owns a Store page field group.
    fn for_store_page_group(group: &str) -> Self {
        match group {
            "basic" => Self::Basics,
            _ => Self::StorePage,
        }
    }

    pub(crate) fn title(self) -> &'static str {
        match self {
            Self::Overview => "Overview",
            Self::Basics => "Basics",
            Self::StorePage => "Store page",
            Self::PriceAccess => "Price & access",
            Self::Delivery => "Delivery & builds",
        }
    }

    fn description(self) -> &'static str {
        match self {
            Self::Overview => "Everything your game needs, in any order.",
            Self::Basics => "The title, summary and tags players see on game cards.",
            Self::StorePage => {
                "Your store page: description, media, feature sections, system requirements, languages, accessibility and links."
            }
            Self::PriceAccess => "Whether players pay for the game or get it for free.",
            Self::Delivery => "How players get your game, and the build they receive.",
        }
    }

    fn dom_id(self) -> &'static str {
        match self {
            Self::Overview => "game-hub-overview",
            Self::Basics => "game-hub-basics",
            Self::StorePage => "game-hub-store-page",
            Self::PriceAccess => "game-hub-price",
            Self::Delivery => "game-hub-delivery",
        }
    }
}

/// Tabs of the Store page section, each with the field groups it renders.
const STORE_PAGE_TABS: [(&str, &str, &[&str]); 7] = [
    ("description", "Description", &["description"]),
    ("media", "Media", &["media"]),
    ("sections", "Feature sections", &["sections"]),
    ("requirements", "System requirements", &["requirements"]),
    ("languages", "Languages", &["languages"]),
    ("accessibility", "Accessibility", &["accessibility"]),
    ("links", "Links", &["links"]),
];

/// Truthful state of one checklist row. A section is only `Ready` when its
/// current values validate; having been visited never counts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SectionStatus {
    Ready,
    /// Publishable, but missing things players benefit from.
    Suggestions,
    NeedsAttention,
}

/// Blocking problems win; otherwise outstanding suggestions keep a section
/// from claiming it is finished.
fn section_status(result: &Result<(), String>, suggestions: usize) -> SectionStatus {
    match (result, suggestions) {
        (Err(_), _) => SectionStatus::NeedsAttention,
        (Ok(()), 0) => SectionStatus::Ready,
        (Ok(()), _) => SectionStatus::Suggestions,
    }
}

impl SectionStatus {
    fn class(self) -> &'static str {
        match self {
            Self::Ready => "v2-hub-row-ready",
            Self::Suggestions => "v2-hub-row-suggestions",
            Self::NeedsAttention => "v2-hub-row-attention",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Ready => "Ready",
            Self::Suggestions => "Can be improved",
            Self::NeedsAttention => "Needs attention",
        }
    }

    fn icon(self) -> &'static str {
        match self {
            Self::Ready => "✓",
            Self::Suggestions => "○",
            Self::NeedsAttention => "!",
        }
    }

    fn action(self) -> &'static str {
        match self {
            Self::Ready => "Edit >",
            Self::Suggestions => "Improve >",
            Self::NeedsAttention => "Fix >",
        }
    }
}

/// Store page suggestions, most useful first: an empty description and the
/// images on game cards matter more than a trailer.
fn prioritized_suggestions(recommendations: &[String]) -> Vec<String> {
    let rank = |item: &String| {
        if item.starts_with("Add a description") {
            0
        } else if item.starts_with("Add a capsule") {
            1
        } else if item.starts_with("Add a hero") {
            2
        } else {
            3
        }
    };
    let mut items = recommendations.to_vec();
    items.sort_by_key(rank);
    items
}

/// Blocking issues gathered across sections, in checklist order.
fn blocking_section_errors(
    results: &[(HubSection, Result<(), String>)],
) -> Vec<(HubSection, String)> {
    results
        .iter()
        .filter_map(|(section, result)| {
            result.as_ref().err().map(|error| (*section, error.clone()))
        })
        .collect()
}

/// How players get the game. One explicit choice replaces the separate
/// "automated installation" toggle and download-only/launchable switch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DeliveryChoice {
    Install,
    DownloadOnly,
    StorePageOnly,
}

impl DeliveryChoice {
    const ALL: [Self; 3] = [Self::Install, Self::DownloadOnly, Self::StorePageOnly];

    fn title(self) -> &'static str {
        match self {
            Self::Install => "Arcadestr installs it",
            Self::DownloadOnly => "Download only",
            Self::StorePageOnly => "Store page only",
        }
    }

    fn description(self) -> &'static str {
        match self {
            Self::Install => {
                "Recommended. Upload your game; players install it and press Play in their library."
            }
            Self::DownloadOnly => "Upload your game; players download the file and run it themselves.",
            Self::StorePageOnly => {
                "No build yet. Publish the store page now (for example, coming soon) and add a build later."
            }
        }
    }

    fn uses_build(self) -> bool {
        !matches!(self, Self::StorePageOnly)
    }
}

/// The delivery choice the current form expresses, or `None` until a new game
/// has made one. Existing games always have a choice.
fn delivery_choice(chosen: bool, build_enabled: bool, launchable: bool) -> Option<DeliveryChoice> {
    if !chosen {
        None
    } else if !build_enabled {
        Some(DeliveryChoice::StorePageOnly)
    } else if launchable {
        Some(DeliveryChoice::Install)
    } else {
        Some(DeliveryChoice::DownloadOnly)
    }
}

/// Apply a delivery choice to the settings the publish request is built from.
/// Returns whether the distribution settings and the launch mode changed.
fn apply_delivery_choice(
    choice: DeliveryChoice,
    fulfillment_enabled: &mut bool,
    fulfillment_mode: &mut FulfillmentMode,
    state: &mut PublishState,
    platforms: &str,
) -> (bool, bool) {
    let enable = choice.uses_build();
    let distribution_changed = enable != *fulfillment_enabled;
    if distribution_changed {
        *fulfillment_enabled = enable;
        if enable && *fulfillment_mode == FulfillmentMode::None {
            *fulfillment_mode = FulfillmentMode::Delegate;
        } else if !enable {
            *fulfillment_mode = FulfillmentMode::None;
        }
    }
    let launch_changed = super::publish_launch::apply_launch_choice(
        state,
        choice == DeliveryChoice::Install,
        platforms,
    );
    (distribution_changed, launch_changed)
}

/// What publishing a live game's changes has to send.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UpdateScope {
    /// The whole game: listing, store page, build and distribution.
    FullGame,
    /// Only the store page; the game itself is unchanged.
    StorePageOnly,
}

/// Choose the smallest publish that carries every change. Anything the game
/// itself publishes (basics, price, delivery, or the card and screenshot
/// images, which are also the game's images) needs the full publish, as does
/// a store page the game doesn't point to yet. When unsure, publish it all.
fn update_scope(
    editing: bool,
    distribution_settled: bool,
    game_fields_changed: bool,
    distribution_changed: bool,
    game_images_changed: bool,
    store_page_changed: bool,
    store_page_linked: bool,
) -> UpdateScope {
    if editing
        && distribution_settled
        && store_page_linked
        && store_page_changed
        && !game_fields_changed
        && !distribution_changed
        && !game_images_changed
    {
        UpdateScope::StorePageOnly
    } else {
        UpdateScope::FullGame
    }
}

/// Where the game stands for the author, kept separate from publication
/// outcome so a local edit is never described as published.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AuthoringState {
    DraftIncomplete,
    DraftReady,
    Live,
    LiveChanged,
    LiveChangedBlocked,
}

fn authoring_state(editing: bool, dirty: bool, blocking_issues: usize) -> AuthoringState {
    match (editing, dirty, blocking_issues == 0) {
        (false, _, false) => AuthoringState::DraftIncomplete,
        (false, _, true) => AuthoringState::DraftReady,
        (true, false, _) => AuthoringState::Live,
        (true, true, true) => AuthoringState::LiveChanged,
        (true, true, false) => AuthoringState::LiveChangedBlocked,
    }
}

impl AuthoringState {
    fn label(self) -> &'static str {
        match self {
            Self::DraftIncomplete => "Draft · not ready to publish",
            Self::DraftReady => "Draft · ready to publish",
            Self::Live => "Live",
            Self::LiveChanged => "Live · changes not published yet",
            Self::LiveChangedBlocked => "Live · changes need attention",
        }
    }

    fn chip(self) -> crate::ui_v2::components::StatusChipVariant {
        use crate::ui_v2::components::StatusChipVariant;
        match self {
            Self::DraftIncomplete | Self::DraftReady => StatusChipVariant::Draft,
            Self::Live => StatusChipVariant::Published,
            Self::LiveChanged => StatusChipVariant::Draft,
            Self::LiveChangedBlocked => StatusChipVariant::Warning,
        }
    }
}

/// Autosave state of the game's local draft.
#[derive(Debug, Clone, PartialEq, Eq)]
enum DraftSaveStatus {
    Idle,
    Pending,
    Saving,
    Saved,
    Failed(String),
    Conflict,
}

impl DraftSaveStatus {
    fn label(&self) -> Option<String> {
        match self {
            Self::Idle => None,
            Self::Pending | Self::Saving => Some("Saving draft…".into()),
            Self::Saved => Some("Draft saved on this device".into()),
            Self::Failed(error) => Some(format!("Couldn't save draft: {error}")),
            Self::Conflict => {
                Some("This draft was changed in another window. Reload it to keep editing.".into())
            }
        }
    }

    /// Leaving would lose edits that are not yet in the local draft.
    fn unsaved(&self) -> bool {
        matches!(
            self,
            Self::Pending | Self::Saving | Self::Failed(_) | Self::Conflict
        )
    }
}

/// Local draft key of a game: the published identifier when there is one,
/// otherwise the id minted when the draft was created.
pub(crate) fn game_draft_id(published_d_tag: Option<&str>, local_id: Option<&str>) -> String {
    match (published_d_tag, local_id) {
        (Some(d_tag), _) => format!("listing:{d_tag}"),
        (None, Some(local)) if local.starts_with("local:") => local.to_string(),
        (None, Some(local)) => format!("local:{local}"),
        (None, None) => format!("local:{}", uuid::Uuid::new_v4()),
    }
}

/// Run `callback` once after `delay_ms` in the browser event loop.
pub(crate) fn after_delay(delay_ms: i32, callback: impl FnOnce() + 'static) {
    use wasm_bindgen::JsCast;
    let closure = wasm_bindgen::closure::Closure::once_into_js(callback);
    if let Some(window) = web_sys::window() {
        let _ = window.set_timeout_with_callback_and_timeout_and_arguments_0(
            closure.unchecked_ref(),
            delay_ms,
        );
    }
}

/// Publication presentation derived from authoritative command state. A local
/// command completing is never enough to claim relay publication succeeded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PublicationPhase {
    Idle,
    Publishing,
    Partial,
    Published,
    Failed,
    StaleAccount,
}

fn publication_phase(
    is_publishing: bool,
    account_stale: bool,
    outcome: PublicationOutcome,
) -> PublicationPhase {
    if account_stale {
        return PublicationPhase::StaleAccount;
    }
    match outcome {
        PublicationOutcome::Publishing => PublicationPhase::Publishing,
        PublicationOutcome::Partial => PublicationPhase::Partial,
        PublicationOutcome::Complete => PublicationPhase::Published,
        PublicationOutcome::Failed => PublicationPhase::Failed,
        PublicationOutcome::Idle if is_publishing => PublicationPhase::Publishing,
        PublicationOutcome::Idle => PublicationPhase::Idle,
    }
}

impl PublicationPhase {
    fn label(self) -> &'static str {
        match self {
            Self::Idle => "Not published",
            Self::Publishing => "Publishing to the network",
            Self::Partial => "Partially published",
            Self::Published => "Published",
            Self::Failed => "Publication failed",
            Self::StaleAccount => "Account changed during publication",
        }
    }

    fn class(self) -> &'static str {
        match self {
            Self::Idle => "v2-create-phase-idle",
            Self::Publishing => "v2-create-phase-busy",
            Self::Partial | Self::StaleAccount => "v2-create-phase-warning",
            Self::Published => "v2-create-phase-ok",
            Self::Failed => "v2-create-phase-error",
        }
    }
}

/// Signer presentation for the publication action, mapped from the shared
/// publisher signer state so both publisher surfaces gate identically.
fn signer_requirement_message(state: PublisherSignerState) -> &'static str {
    match state {
        PublisherSignerState::Available => "Publishing is signed by your active account.",
        PublisherSignerState::Connecting => {
            "Publishing is unavailable while your signing app connects. You can keep editing."
        }
        PublisherSignerState::Unavailable => {
            "Reconnect your signing app in Settings to publish. You can keep editing."
        }
        PublisherSignerState::Unknown => {
            "Publishing is unavailable until your signing app is checked. You can keep editing."
        }
    }
}

/// Snapshot of every author-editable field, used only to tell pristine from dirty.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct FormSnapshot {
    title: String,
    description: String,
    tags: String,
    price: String,
    lud16: String,
    platforms: String,
    version: String,
    operator_url: String,
    acquisition: String,
    starts_at: String,
    ends_at: String,
    fulfillment_enabled: bool,
    commercial: String,
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
struct DurableListingDraft {
    #[serde(default)]
    server_roles: Vec<arcadestr_core::publisher_management::ServerRoleIntent>,
    form: FormSnapshot,
    identity: PublishedIdentity,
    launch: LaunchMode,
    servers: Vec<String>,
    mode: FulfillmentMode,
    acquisition: AcquisitionKind,
    artifact_hash: Option<String>,
    commercial_intent: String,
    commercial_review: Option<PublisherTermsReview>,
    #[serde(default)]
    accepted_terms: Vec<(String, String)>,
    #[serde(default)]
    authorization_references: Vec<(String, String)>,
    /// The Store Page edited with the game, including its media.
    #[serde(default)]
    store_page: Option<StorePageDraft>,
    /// Whether the author has chosen how players get the game.
    #[serde(default)]
    delivery_chosen: bool,
}

fn reviewed_commercial_intent(
    mode: &str,
    context: &PublisherTermsContext,
    review: Option<PublisherTermsReview>,
    approved: bool,
) -> Result<PublisherCommercialIntent, String> {
    match mode {
        "preserve" => Ok(PublisherCommercialIntent::Preserve),
        "replace" if approved => match review {
            Some(review) if &review.context == context => {
                Ok(PublisherCommercialIntent::ReplaceAfterReview { review })
            }
            _ => Err(
                "The price or services changed since you reviewed the fee terms. Get the service's terms again."
                    .into(),
            ),
        },
        "replace" => Err("Review and accept the distribution service's fee terms first.".into()),
        _ => Err("Stopping a distribution service isn't available yet.".into()),
    }
}

/// Millisatoshis as sats, keeping sub-sat precision only when it exists.
fn msat_text(msat: u64) -> String {
    if msat % 1000 == 0 {
        format!("{} sats", msat / 1000)
    } else {
        let fraction = format!("{:03}", msat % 1000);
        format!("{}.{} sats", msat / 1000, fraction.trim_end_matches('0'))
    }
}

fn fee_policy_label(policy: &FeePolicy) -> String {
    match policy {
        FeePolicy::None => "No service fee".into(),
        FeePolicy::Fixed { amount_msat } => format!("{} per sale", msat_text(*amount_msat)),
        FeePolicy::BasisPoints {
            basis_points,
            minimum_msat,
            maximum_msat,
        } => {
            let percent = if basis_points % 100 == 0 {
                format!("{}%", basis_points / 100)
            } else {
                let fraction = format!("{:02}", basis_points % 100);
                format!("{}.{}%", basis_points / 100, fraction.trim_end_matches('0'))
            };
            let mut label = format!("{percent} of each sale");
            if let Some(minimum) = minimum_msat {
                label.push_str(&format!(" · at least {}", msat_text(*minimum)));
            }
            if let Some(maximum) = maximum_msat {
                label.push_str(&format!(" · at most {}", msat_text(*maximum)));
            }
            label
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RemovalConfirmationIdentity {
    account_npub: Option<String>,
    publication: PublishedIdentity,
    authoritative_state: Option<AuthoritativeEditState>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PendingRemovalConfirmation {
    prepared: PreparedPublish,
    removed_profile_ids: Vec<ProfileId>,
    form: FormSnapshot,
    dirty: PublishDirtySnapshot,
    identity: RemovalConfirmationIdentity,
    generation: u64,
    abort_count: u64,
}

fn removal_confirmation_prepared(
    frozen: &PreparedPublish,
    removed_profile_ids: &[ProfileId],
) -> PreparedPublish {
    let mut confirmed = frozen.clone();
    confirmed.extension.acknowledged_removed_profile_ids = removed_profile_ids.to_vec();
    confirmed
}

fn removal_confirmation_is_fresh(
    pending: &PendingRemovalConfirmation,
    form: &FormSnapshot,
    dirty: &PublishDirtySnapshot,
    identity: &RemovalConfirmationIdentity,
    generation: u64,
) -> bool {
    pending.form == *form
        && pending.dirty == *dirty
        && pending.identity == *identity
        && pending.generation == generation
}

fn removal_confirmation_copy(ids: &[ProfileId], abort_count: u64) -> String {
    let names = ids
        .iter()
        .map(|id| id.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    if abort_count > 1 {
        format!(
            "The backend rechecked the latest revision and returned a fresh complete removal set. Current profile IDs: {names}. Review every ID and confirm again."
        )
    } else {
        format!(
            "Publication would remove these launch profile IDs: {names}. Review every ID before confirming."
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PublicationOutcome {
    Idle,
    Publishing,
    Complete,
    Partial,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PublicationState {
    outcome: PublicationOutcome,
    listing_published: bool,
    message: Option<String>,
}

impl Default for PublicationState {
    fn default() -> Self {
        Self {
            outcome: PublicationOutcome::Idle,
            listing_published: false,
            message: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ReadinessChecklist {
    metadata: bool,
    pricing_and_access: bool,
    distribution: bool,
    authorization: bool,
    warnings: Vec<String>,
}

impl AcquisitionKind {
    fn from_value(value: &str) -> Self {
        match value {
            "public" => Self::Public,
            "timed-access" => Self::TimedAccess,
            _ => Self::Gated,
        }
    }
}

fn acquisition_kind(policy: &AcquisitionPolicy) -> AcquisitionKind {
    match policy {
        AcquisitionPolicy::Gated => AcquisitionKind::Gated,
        AcquisitionPolicy::Public => AcquisitionKind::Public,
        AcquisitionPolicy::TimedAccess { .. } => AcquisitionKind::TimedAccess,
    }
}

fn acquisition_policy_from_form(
    kind: AcquisitionKind,
    starts_at: &str,
    ends_at: &str,
) -> Result<AcquisitionPolicy, String> {
    match kind {
        AcquisitionKind::Gated => Ok(AcquisitionPolicy::Gated),
        AcquisitionKind::Public => Ok(AcquisitionPolicy::Public),
        AcquisitionKind::TimedAccess => {
            if starts_at.trim().is_empty() {
                return Err("Choose when timed access starts".to_string());
            }
            if ends_at.trim().is_empty() {
                return Err("Choose when timed access ends".to_string());
            }
            let starts_at = datetime_local_to_unix(starts_at)
                .ok_or_else(|| "Timed access start is invalid".to_string())?;
            let ends_at = datetime_local_to_unix(ends_at)
                .ok_or_else(|| "Timed access end is invalid".to_string())?;
            if starts_at >= ends_at {
                return Err("Timed access must end after it starts".to_string());
            }
            Ok(AcquisitionPolicy::TimedAccess { starts_at, ends_at })
        }
    }
}

fn datetime_local_value(value: u64) -> String {
    let date = js_sys::Date::new(&(value as f64 * 1000.0).into());
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}",
        date.get_full_year(),
        date.get_month() + 1,
        date.get_date(),
        date.get_hours(),
        date.get_minutes(),
        date.get_seconds()
    )
}

fn price_for_acquisition(kind: AcquisitionKind, input: &str, lud16: &str) -> Result<u64, String> {
    if kind != AcquisitionKind::Gated {
        return Ok(0);
    }
    let price = input
        .trim()
        .parse::<u64>()
        .map_err(|_| "Enter a valid whole-number price in sats".to_string())?;
    if price == 0 {
        return Err("Paid purchase requires a price greater than zero sats".to_string());
    }
    if lud16.trim().is_empty() {
        return Err("Lightning address is required when paid purchase is enabled".to_string());
    }
    let mut address_parts = lud16.trim().split('@');
    let user = address_parts.next().unwrap_or_default();
    let domain = address_parts.next().unwrap_or_default();
    if user.is_empty() || domain.is_empty() || address_parts.next().is_some() {
        return Err("Lightning address must look like name@example.com".to_string());
    }
    Ok(price)
}

fn is_http_url(value: &str) -> bool {
    url::Url::parse(value).is_ok_and(|parsed| {
        matches!(parsed.scheme(), "http" | "https")
            && parsed.host_str().is_some()
            && !value
                .strip_prefix("https://")
                .or_else(|| value.strip_prefix("http://"))
                .is_some_and(|remainder| remainder.starts_with('/'))
    })
}

fn validate_http_urls(input: &str, label: &str) -> Result<Vec<String>, String> {
    let values = parse_csv_values(input);
    for value in &values {
        if !is_http_url(value) {
            return Err(format!("{label} is not a valid HTTP(S) URL: {value}"));
        }
    }
    Ok(values)
}

fn publication_progress(
    mut state: PublicationState,
    event: &PublishProgressPayload,
) -> PublicationState {
    if state.outcome != PublicationOutcome::Partial {
        state.outcome = PublicationOutcome::Publishing;
    }
    if event.step == "publish-listing" && event.status == "ok" {
        state.listing_published = true;
    }
    if state.listing_published && event.step == "upload" && event.status == "error" {
        state.outcome = PublicationOutcome::Partial;
        state.message = Some(
            "Your game is live, but uploading the build didn't finish. Select Finish publishing below to retry the upload."
                .to_string(),
        );
    }
    state
}

/// Identity of the listing this form is responsible for, once one exists.
///
/// Empty for a first publication: Arcadestr mints the identifier in the
/// backend, and the form only learns it afterwards. Publication is not
/// transactional, so this is filled in as soon as the listing event is on a
/// relay — a retry after a later failure must replace that listing rather than
/// create a second coordinate.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct PublishedIdentity {
    event_id: Option<String>,
    d_tag: Option<String>,
    coordinate: Option<String>,
    /// Identity the backend minted for a first publication, held from the
    /// moment it is minted rather than from the moment a listing exists.
    ///
    /// A publication that fails before its listing — while provisioning a
    /// fulfillment key, say — leaves this draft owning a coordinate nothing
    /// was published under. Keeping it means the retry resumes that
    /// publication under the same identity, and so under the same provisioning
    /// scope, instead of asking the operator for a second key.
    pending_d_tag: Option<String>,
}

/// Fold a publish progress event into the published identity.
fn publication_identity_progress(
    mut identity: PublishedIdentity,
    event: &PublishProgressPayload,
) -> PublishedIdentity {
    if event.status != "ok" {
        return identity;
    }
    match event.step.as_str() {
        "publish-listing" => {
            if let Some(event_id) = event.message.clone() {
                identity.event_id = Some(event_id);
            }
        }
        "listing-coordinate" => {
            if let Some(coordinate) = event.message.clone() {
                if let Some(d_tag) = d_tag_from_coordinate(&coordinate) {
                    identity.d_tag = Some(d_tag);
                    identity.coordinate = Some(coordinate);
                    // The listing exists; this identity is no longer pending.
                    identity.pending_d_tag = None;
                }
            }
        }
        "pending-coordinate" => {
            // Nothing is published under this identity yet, and the attempt
            // carrying it may still fail. Hold it for the retry.
            if identity.d_tag.is_none() {
                if let Some(d_tag) = event
                    .message
                    .as_deref()
                    .and_then(|coordinate| d_tag_from_coordinate(coordinate))
                {
                    identity.pending_d_tag = Some(d_tag);
                }
            }
        }
        _ => {}
    }
    identity
}

/// The `d` tag inside a `30402:<publisher-hex>:<d-tag>` coordinate.
///
/// Legacy identifiers may contain colons, so everything after the second one
/// belongs to the tag.
fn d_tag_from_coordinate(coordinate: &str) -> Option<String> {
    let mut parts = coordinate.splitn(3, ':');
    let kind = parts.next()?;
    let _publisher = parts.next()?;
    let d_tag = parts.next()?;
    (kind == "30402" && !d_tag.is_empty()).then(|| d_tag.to_string())
}

/// The Store Page as it is bundled with the listing: aligned with the game
/// details the form owns (title, genres from tags, starting copy).
fn synced_store_page(
    mut page: StorePageDraft,
    title: &str,
    description: &str,
    tags_input: &str,
) -> StorePageDraft {
    let tags = parse_csv_values(tags_input);
    arcadestr_core::store_page_seed::sync_with_listing(
        &mut page,
        arcadestr_core::store_page_seed::ListingPresentation {
            title: title.trim(),
            description: description.trim(),
            tags: &tags,
            images: &[],
        },
    );
    page
}

/// Problems the backend would reject the bundled Store Page for, checked
/// locally with the core validator. A page not published yet gets a
/// stand-in identity, since the real one is settled by the publication.
fn store_page_problems(page: &StorePageDraft, publisher_hex: &str) -> Vec<String> {
    let Ok(publisher) = nostr::PublicKey::from_hex(publisher_hex) else {
        return vec!["The active publisher key is invalid.".to_string()];
    };
    let mut page = page.clone();
    if page.presentation_id.trim().is_empty() {
        page.presentation_id = "preview".to_string();
    }
    if page.listing_coordinates.is_empty() {
        page.listing_coordinates = vec![format!("30402:{publisher_hex}:{}", page.presentation_id)];
    }
    page.sync_compact_tags();
    match arcadestr_core::store_page::validate_store_page_draft(&page.build_params(publisher)) {
        Ok(validated) => validated
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.blocks_authoring())
            .map(|diagnostic| diagnostic.describe())
            .collect(),
        Err(error) => vec![error.to_string()],
    }
}

/// The store page exactly as buyers will see it: the draft synced with the
/// game's basics, validated and sanitized by the same core rules publishing
/// uses. Requirements only show for platforms the game declares, as on the
/// published page.
fn buyer_preview_page(
    page: &StorePageDraft,
    publisher_hex: &str,
    platforms: &[String],
) -> Result<crate::models::GameDetailPresentation, String> {
    use crate::models::{
        GameDetailPresentation, SafeStorePageHtml, StorePageAccessibility, StorePageDetailSection,
        StorePageLanguage, StorePageLinks, StorePageMedia, StorePagePlatformRequirements,
        StorePageRequirementTier,
    };
    let publisher = nostr::PublicKey::from_hex(publisher_hex)
        .map_err(|_| "The active publisher key is invalid.".to_string())?;
    let mut page = page.clone();
    if page.presentation_id.trim().is_empty() {
        page.presentation_id = "preview".to_string();
    }
    if page.listing_coordinates.is_empty() {
        page.listing_coordinates = vec![format!("30402:{publisher_hex}:{}", page.presentation_id)];
    }
    page.sync_compact_tags();
    let params = page.build_params(publisher);
    let validated = arcadestr_core::store_page::validate_store_page_draft(&params)
        .map_err(|error| error.to_string())?;
    if let Some(problem) = validated
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.blocks_authoring())
    {
        return Err(problem.describe());
    }
    let content = &params.content;
    let normalized = &validated.normalized;
    let sanitized = validated.sanitized_content();
    let tier = |tier: &arcadestr_core::store_page::RequirementTier| StorePageRequirementTier {
        os: tier.os.clone(),
        processor: tier.processor.clone(),
        memory: tier.memory.clone(),
        graphics: tier.graphics.clone(),
        storage: tier.storage.clone(),
        additional: tier.additional.clone(),
    };
    Ok(GameDetailPresentation {
        listing_coordinate: page.listing_coordinates[0].clone(),
        listing_event_id: String::new(),
        store_page_coordinate: String::new(),
        event_id: "draft-preview".to_string(),
        title: normalized.title.clone(),
        summary: normalized.summary.clone(),
        description_html: (!sanitized.description_html.as_str().is_empty()).then(|| {
            SafeStorePageHtml::from_backend(sanitized.description_html.as_str().to_string())
        }),
        media: sanitized
            .media
            .iter()
            .map(|item| StorePageMedia {
                id: item.id.clone(),
                media_type: item.media_type.clone(),
                role: item.role.clone(),
                url: item.url.clone(),
                thumbnail_url: item.thumbnail_url.clone(),
                alt: item.alt.clone(),
                caption: item.caption.clone(),
            })
            .collect(),
        sections: sanitized
            .sections
            .iter()
            .map(|section| StorePageDetailSection {
                id: section.id.clone(),
                heading: section.heading.clone(),
                body_html: SafeStorePageHtml::from_backend(section.body_html.as_str().to_string()),
                media_id: section.media_id.clone(),
                layout: section.layout.clone(),
            })
            .collect(),
        genres: normalized.genres.clone(),
        features: normalized.features.clone(),
        languages: content
            .languages
            .clone()
            .unwrap_or_default()
            .into_iter()
            .map(|language| StorePageLanguage {
                code: language.code,
                interface: language.interface,
                audio: language.audio,
                subtitles: language.subtitles,
            })
            .collect(),
        requirements: content
            .requirements
            .iter()
            .filter(|(platform, _)| platforms.contains(platform))
            .map(|(platform, requirement)| StorePagePlatformRequirements {
                platform: platform.clone(),
                minimum: requirement.minimum.as_ref().map(tier),
                recommended: requirement.recommended.as_ref().map(tier),
            })
            .collect(),
        accessibility: content
            .accessibility
            .iter()
            .map(|entry| StorePageAccessibility {
                feature: entry.feature.clone(),
                supported: entry.supported,
                notes: entry.notes.clone(),
            })
            .collect(),
        links: StorePageLinks {
            website: sanitized.links.website.clone(),
            support: sanitized.links.support.clone(),
            documentation: sanitized.links.documentation.clone(),
            source: sanitized.links.source.clone(),
            community: sanitized.links.community.clone(),
            privacy_policy: sanitized.links.privacy_policy.clone(),
        },
        developer: normalized.developer.clone(),
        publisher: normalized.publisher.clone(),
        release_date: normalized.release_date.clone(),
    })
}

/// The game card buyers see in the store and search results, built from the
/// same listing values and store page the request will publish.
fn buyer_preview_card(
    publisher_npub: &str,
    title: &str,
    description: &str,
    price_sats: u64,
    acquisition: &AcquisitionPolicy,
    page: &crate::models::GameDetailPresentation,
    images: Vec<String>,
    platforms: &[String],
) -> Option<(GameListing, crate::models::StorePageCardPresentation)> {
    let listing: GameListing = serde_json::from_value(serde_json::json!({
        "id": "draft-preview",
        "source": "nip99_listing",
        "title": title.trim(),
        "description": description.trim(),
        "publisher_npub": publisher_npub,
        "created_at": 0,
        "price_sats": price_sats,
        "price": price_sats as f64,
        "currency": "SATS",
        "images": images,
        "platforms": platforms,
        "acquisition": serde_json::to_value(acquisition).ok()?,
    }))
    .ok()?;
    let media_url = |role: &str| {
        page.media
            .iter()
            .find(|item| item.role == role && item.media_type == "image")
            .map(|item| item.url.clone())
    };
    let card = crate::models::StorePageCardPresentation {
        listing_coordinate: page.listing_coordinate.clone(),
        store_page_coordinate: String::new(),
        event_id: page.event_id.clone(),
        title: page.title.clone(),
        summary: page.summary.clone(),
        capsule_url: media_url("capsule"),
        hero_url: media_url("hero"),
        genres: page.genres.clone(),
        features: page.features.clone(),
        release_date: page.release_date.clone(),
    };
    Some((listing, card))
}

/// Once the bundled Store Page is published the form edits that revision, so
/// a retry after a failed listing replaces it instead of conflicting with it.
/// A new page takes the game's identity, exactly as the backend assigned it.
fn store_page_after_progress(
    page: &StorePageDraft,
    event: &PublishProgressPayload,
) -> Option<StorePageDraft> {
    if event.step != "publish-store-page" || event.status != "ok" {
        return None;
    }
    let event_id = event.message.clone()?;
    let mut next = page.clone();
    if next.loaded_event_id.is_none() {
        next.presentation_id = d_tag_from_coordinate(&event.game_coordinate)?;
        if !next.listing_coordinates.contains(&event.game_coordinate) {
            next.listing_coordinates.push(event.game_coordinate.clone());
        }
    }
    next.loaded_event_id = Some(event_id);
    Some(next)
}

fn publication_completed(
    mut state: PublicationState,
    result: Result<(String, bool), String>,
) -> PublicationState {
    match result {
        Ok((_event_id, true)) => {
            state.listing_published = true;
            state.outcome = PublicationOutcome::Partial;
            state.message = Some(
                "Your game is live, but uploading the build didn't finish. Select Finish publishing below to retry the upload."
                    .to_string(),
            );
        }
        Ok((event_id, false)) => {
            state.listing_published = true;
            state.outcome = PublicationOutcome::Complete;
            state.message = Some(format!("Your game is published. (event {event_id})"));
        }
        Err(error) if state.listing_published => {
            state.outcome = PublicationOutcome::Partial;
            state.message = Some(format!(
                "Your game is live, but the rest of publishing failed: {error} Select Finish publishing below to retry."
            ));
        }
        Err(error) => {
            state.outcome = PublicationOutcome::Failed;
            state.message = Some(error);
        }
    }
    state
}

/// Steps that hand this form the identity of what it is publishing.
///
/// They carry no publication outcome, so they are folded into the identity and
/// then dropped rather than shown as stages the publisher is waiting on.
fn is_recovery_channel(step: &str) -> bool {
    matches!(step, "listing-coordinate" | "pending-coordinate")
}

fn accepts_publication_progress(
    operation: &str,
    publisher: &str,
    coordinate: &mut Option<String>,
    event: &PublishProgressPayload,
) -> bool {
    if event.operation_id != operation
        || event.publisher_npub != publisher
        || event.game_coordinate.is_empty()
        || coordinate
            .as_ref()
            .is_some_and(|expected| expected != &event.game_coordinate)
    {
        return false;
    }
    if coordinate.is_none() {
        *coordinate = Some(event.game_coordinate.clone());
    }
    true
}

fn progress_label(event: &PublishProgressPayload) -> String {
    let stage = match event.step.as_str() {
        "hash-file" => "Checking your build",
        "check-operator" => "Contacting the distribution service",
        "provision" => "Setting up the distribution service",
        "publish-listing" => "Publishing your game",
        "publish-store-page" => "Publishing your store page",
        "confirm-propagation" => "Confirming it reached the network",
        "upload" => "Uploading your build",
        _ => "Publishing",
    };
    let status = match event.status.as_str() {
        "pending" => "in progress",
        "ok" => "done",
        "error" => "failed",
        other => other,
    };
    format!(
        "{stage}{}: {status}{}",
        event
            .server_url
            .as_ref()
            .map(|url| format!(" for {url}"))
            .unwrap_or_default(),
        // Successful steps report identifiers, not something to read.
        event
            .message
            .as_ref()
            .filter(|_| event.status != "ok")
            .map(|message| format!(" — {message}"))
            .unwrap_or_default()
    )
}

fn progress_percent(completed_bytes: u64, total_bytes: u64) -> u64 {
    if total_bytes == 0 {
        return 100;
    }
    (completed_bytes.saturating_mul(100) / total_bytes).min(100)
}

fn publication_account_matches(initiating_npub: &str, active_npub: Option<&str>) -> bool {
    active_npub == Some(initiating_npub)
}

fn publication_account_allowed(editing_publisher: Option<&str>, active_npub: &str) -> bool {
    editing_publisher.is_none_or(|publisher| publisher == active_npub)
}

fn can_dispatch(is_hashing: bool, is_publishing: bool) -> bool {
    !is_hashing && !is_publishing
}

fn when_fulfillment_enabled<T>(enabled: bool, value: T) -> Option<T> {
    enabled.then_some(value)
}

pub(super) fn archive_request_id() -> Result<ArchiveRequestId, ArchiveSelectionError> {
    ArchiveRequestId::parse(&uuid::Uuid::new_v4().to_string())
        .map_err(|_| ArchiveSelectionError::ContractMismatch)
}

#[cfg(test)]
fn file_selection_changed(new_path: Option<String>) -> (Option<String>, Option<String>) {
    match new_path {
        Some(path) => (Some(path), None),
        None => (None, None),
    }
}

pub(super) fn publish_error_message(error: &PublishAdpListingError) -> String {
    match error {
        PublishAdpListingError::Archive { error } => archive_error_message(error),
        PublishAdpListingError::PartialPublication { cause, successful_servers } => format!(
            "Publishing didn't finish ({} server(s) accepted it): {} Use Finish publishing on the game instead of publishing again.",
            successful_servers.len(),
            publish_error_message(cause)
        ),
        PublishAdpListingError::StaleExpectedEventId { .. } => {
            "This game was updated somewhere else since you opened it. Your changes are saved as a draft. Go back to Your games, refresh, and open the game again before publishing."
                .to_string()
        }
        _ => error.to_string(),
    }
}

fn publish_archive_error(error: &PublishAdpListingError) -> Option<&ArchiveSelectionError> {
    match error {
        PublishAdpListingError::Archive { error } => Some(error),
        PublishAdpListingError::PartialPublication { cause, .. } => publish_archive_error(cause),
        _ => None,
    }
}

fn should_rebase_edit_distribution(
    authority_ready: bool,
    operator_initialized: bool,
    user_revision: u64,
) -> bool {
    authority_ready && operator_initialized && user_revision == 0
}

/// One message per archive failure category, so an expired selection, a
/// rejected archive, local storage and transport problems are never reported
/// as each other (or as an unrelated launch-profile problem).
pub(super) fn archive_error_message(error: &ArchiveSelectionError) -> String {
    use ArchiveSelectionError as E;
    match error {
        E::Account { .. } => "This build selection belongs to a different or signed-out account. Select the build again with the active account.".into(),
        E::Expired { .. } => "The build selection expired after inactivity. Select the build file again.".into(),
        E::Handle { .. } => "The staged build is missing or was discarded. Select the build file again.".into(),
        E::Busy { .. } => "Another build-file operation is still running. Wait for it to finish, then retry.".into(),
        E::Mutation { .. } => "The selected build file changed after it was selected. Select it again so the published hash matches the file.".into(),
        E::NonRegularFile => "The selection is not a regular file. Choose a build archive file.".into(),
        E::PickerFailed => "The file picker could not open. Retry the selection.".into(),
        E::SelectedBuildRequired => "Select a build file before continuing.".into(),
        E::InsufficientStorage { required, available } => format!(
            "Not enough local storage to stage the build ({required} bytes needed, {available} available). Free space and select it again."
        ),
        E::Storage { .. } => "A local storage operation failed while staging the build. Check disk access and select the build again.".into(),
        E::Detection { .. } => "The file is not a supported game archive (ZIP, tar.gz or tar.zst).".into(),
        E::DigestMismatch { .. } => "The staged build no longer matches its recorded hash. Select the build file again.".into(),
        E::Pagination { .. } => "The archive listing request was invalid. Reload the archive contents.".into(),
        E::InvalidPlatform { .. } | E::UnsupportedPlatform => "The selected platform is not supported for this archive.".into(),
        E::Inspection { .. } => "The archive could not be inspected: it is malformed or exceeds supported limits.".into(),
        E::BlockingHazards { .. } => "The archive contains unsafe entries (such as links or path escapes) and cannot be published.".into(),
        E::Registry { .. } => "The local build-selection registry is unavailable. Restart the desktop app and select the build again.".into(),
        E::Cancellation { .. } => "The build selection could not be cancelled cleanly. Select the build again.".into(),
        E::Transport | E::ContractMismatch => "The desktop backend did not return a valid response for the build selection. Restart the desktop app and retry.".into(),
    }
}

impl ServerStatus {
    fn label(self) -> &'static str {
        match self {
            ServerStatus::Idle => "not checked",
            ServerStatus::Pending => "pending",
            ServerStatus::Ok => "ok",
            ServerStatus::Failed => "failed",
        }
    }

    fn class(self) -> &'static str {
        match self {
            ServerStatus::Idle => "v2-create-status-neutral",
            ServerStatus::Pending => "v2-create-status-busy",
            ServerStatus::Ok => "v2-create-status-ok",
            ServerStatus::Failed => "v2-create-status-error",
        }
    }
}

fn validate_listing(
    title: &str,
    description: &str,
    price_sats: u64,
    lud16: &str,
    fulfillment_enabled: bool,
    servers: &[ServerEntry],
    _file_path: &Option<String>,
    file_hash: &Option<String>,
    version: &str,
    fulfillment_mode: &FulfillmentMode,
    operator_url: &str,
) -> Result<(), String> {
    let title = title.trim();
    let description = description.trim();
    let lud16 = lud16.trim();
    if title.is_empty() {
        return Err("Title is required".to_string());
    }
    if title.len() > 100 {
        return Err("Title must be 100 characters or less".to_string());
    }
    if description.is_empty() {
        return Err("Summary is required".to_string());
    }
    if description.len() > 2000 {
        return Err("Summary must be 2000 characters or less".to_string());
    }
    if price_sats > 0 && lud16.is_empty() {
        return Err("Add a Lightning address so you can be paid.".to_string());
    }
    if !lud16.is_empty() {
        let parts: Vec<&str> = lud16.split('@').collect();
        if parts.len() != 2 || parts[0].is_empty() || parts[1].is_empty() {
            return Err("Lightning address must look like name@example.com".to_string());
        }
    }
    if fulfillment_enabled {
        if servers.is_empty() {
            return Err("Add at least one upload server under Advanced options.".to_string());
        }
        if let Some(bad_url) = servers
            .iter()
            .map(|server| server.url.as_str())
            .find(|url| !is_http_url(url))
        {
            return Err(format!(
                "Server URL must start with http:// or https://: {bad_url}"
            ));
        }
        match file_hash.as_deref() {
            None => {
                return Err("Choose your build file and wait for it to be checked.".to_string());
            }
            Some(hash) if !is_sha256_hex(hash) => {
                return Err(
                    "Existing SHA-256 metadata is invalid; select a replacement build file"
                        .to_string(),
                );
            }
            Some(_) => {}
        }
        if version.trim().is_empty() {
            return Err("Enter a version for your build.".to_string());
        }
        match fulfillment_mode {
            FulfillmentMode::None => {
                return Err(
                    "Choose a distribution service, or sign with your own account.".to_string(),
                );
            }
            FulfillmentMode::Direct => {}
            FulfillmentMode::Delegate => {
                // Metadata edits preserve signed authority without a local operator row.
                // The backend requires an operator when explicit new provisioning is needed.
                if !operator_url.trim().is_empty() && !is_http_url(operator_url) {
                    return Err(
                        "The distribution service address must start with http:// or https://"
                            .to_string(),
                    );
                }
            }
        }
    }
    Ok(())
}

fn parse_csv_values(input: &str) -> Vec<String> {
    input
        .split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
        .collect()
}

fn editable_listing_tags(tags: &[String]) -> String {
    let mut seen = std::collections::HashSet::new();
    tags.iter()
        .map(String::as_str)
        .map(str::trim)
        .filter(|tag| !tag.is_empty() && !tag.eq_ignore_ascii_case("game"))
        .filter(|tag| seen.insert(tag.to_ascii_lowercase()))
        .collect::<Vec<_>>()
        .join(", ")
}

const AVAILABLE_GAME_TAGS: [&str; 18] = [
    "Action",
    "Adventure",
    "Arcade",
    "Casual",
    "Fighting",
    "Horror",
    "Multiplayer",
    "Platformer",
    "Puzzle",
    "Racing",
    "RPG",
    "Shooter",
    "Simulation",
    "Sports",
    "Strategy",
    "Survival",
    "Visual Novel",
    "VR",
];

fn add_game_tag(input: &str, tag: &str) -> String {
    let mut tags = parse_csv_values(input);
    if !tag.is_empty()
        && !tags
            .iter()
            .any(|existing| existing.eq_ignore_ascii_case(tag))
    {
        tags.push(tag.to_string());
    }
    tags.join(", ")
}

fn remove_game_tag(input: &str, tag: &str) -> String {
    parse_csv_values(input)
        .into_iter()
        .filter(|existing| !existing.eq_ignore_ascii_case(tag))
        .collect::<Vec<_>>()
        .join(", ")
}

fn format_sha256(hash: &str) -> String {
    if !is_sha256_hex(hash) {
        return "Invalid SHA-256 metadata".to_string();
    }

    format!("{}...{}", &hash[..12], &hash[52..])
}

fn parse_platform_tags(input: &str) -> Result<Vec<String>, String> {
    let tags = input
        .split(',')
        .map(str::trim)
        .filter(|tag| !tag.is_empty())
        .map(|tag| {
            if tag.chars().any(char::is_whitespace) {
                return Err("Platform tags cannot contain whitespace".to_string());
            }
            let mut parts = tag.split('-');
            let os = parts.next().unwrap_or_default();
            let arch = parts.next().unwrap_or_default();
            if os.is_empty()
                || arch.is_empty()
                || parts.next().is_some()
                || !os.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
                || !arch.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
            {
                return Err("Platform tags must look like <os>-<arch>".to_string());
            }
            Ok(tag.to_string())
        })
        .collect::<Result<Vec<_>, _>>()?;
    let mut unique = std::collections::HashSet::new();
    if let Some(duplicate) = tags.iter().find(|tag| !unique.insert(tag.as_str())) {
        return Err(format!("Duplicate platform tag: {duplicate}"));
    }
    Ok(tags)
}

const AVAILABLE_PLATFORMS: [(&str, &str); 6] = [
    ("linux-x86_64", "Linux (x86_64)"),
    ("linux-aarch64", "Linux (ARM64)"),
    ("windows-x86_64", "Windows (x86_64)"),
    ("windows-aarch64", "Windows (ARM64)"),
    ("macos-x86_64", "macOS (Intel)"),
    ("macos-aarch64", "macOS (Apple silicon)"),
];

fn add_platform(input: &str, platform: &str) -> String {
    let mut platforms = parse_csv_values(input);
    if !platform.is_empty() && !platforms.iter().any(|existing| existing == platform) {
        platforms.push(platform.to_string());
    }
    platforms.join(", ")
}

fn remove_platform(input: &str, platform: &str) -> String {
    parse_csv_values(input)
        .into_iter()
        .filter(|existing| existing != platform)
        .collect::<Vec<_>>()
        .join(", ")
}

fn platform_summary(input: &str) -> String {
    let platforms = parse_csv_values(input);
    if platforms.is_empty() {
        "All platforms".to_string()
    } else {
        platforms.join(", ")
    }
}

#[allow(clippy::too_many_arguments)]
fn readiness_checklist(
    title: &str,
    description: &str,
    image_input: &str,
    acquisition_kind: AcquisitionKind,
    price_input: &str,
    lud16: &str,
    acquisition: Result<AcquisitionPolicy, String>,
    platforms_input: &str,
    fulfillment_enabled: bool,
    servers: &[ServerEntry],
    file_hash: Option<&str>,
    version: &str,
    fulfillment_mode: &FulfillmentMode,
    operator_url: &str,
) -> ReadinessChecklist {
    let metadata = validate_listing(
        title,
        description,
        0,
        "",
        false,
        &[],
        &None,
        &None,
        "",
        &FulfillmentMode::None,
        "",
    )
    .is_ok()
        && validate_http_urls(image_input, "Image URL").is_ok();
    let pricing_and_access =
        price_for_acquisition(acquisition_kind, price_input, lud16).is_ok() && acquisition.is_ok();
    let platforms_ok = parse_platform_tags(platforms_input).is_ok();
    let distribution = platforms_ok
        && (!fulfillment_enabled
            || (!servers.is_empty()
                && file_hash.is_some_and(is_sha256_hex)
                && !version.trim().is_empty()
                && !matches!(fulfillment_mode, FulfillmentMode::None)));
    let authorization = !fulfillment_enabled
        || matches!(fulfillment_mode, FulfillmentMode::Direct)
        || (matches!(fulfillment_mode, FulfillmentMode::Delegate)
            && !operator_url.trim().is_empty());
    let mut warnings = Vec::new();
    if !fulfillment_enabled {
        warnings.push(
            "No build: players can see the store page but can't get the game until you add one."
                .into(),
        );
    }
    if servers
        .iter()
        .any(|server| server.reachability == ServerStatus::Failed)
    {
        warnings.push("One of your upload servers couldn't be reached.".into());
    }
    ReadinessChecklist {
        metadata,
        pricing_and_access,
        distribution,
        authorization,
        warnings,
    }
}

fn listing_spec(listing: &GameListing, key: &str) -> Option<String> {
    listing
        .specs
        .iter()
        .find(|(name, _)| name == key)
        .map(|(_, value)| value.clone())
}

fn captured_file_hash(listing: &GameListing) -> Option<String> {
    listing_spec(listing, "file_hash").map(|hash| hash.to_ascii_lowercase())
}

fn listing_servers(listing: &GameListing) -> Vec<String> {
    listing
        .specs
        .iter()
        .filter(|(name, value)| name == "server" && !value.is_empty())
        .map(|(_, value)| value.clone())
        .collect()
}

fn listing_authorizations(listing: &GameListing) -> Vec<(nostr::EventId, String)> {
    let mut authorizations = listing
        .specs
        .iter()
        .filter(|(name, _)| name == "fulfillment_authorization")
        .filter_map(|(_, value)| {
            let value: serde_json::Value = serde_json::from_str(value).ok()?;
            Some((
                nostr::EventId::from_hex(value.get("root_event_id")?.as_str()?).ok()?,
                value.get("fulfillment_pubkey")?.as_str()?.to_string(),
            ))
        })
        .collect::<Vec<_>>();
    authorizations.sort_by_key(|(root, _)| *root);
    authorizations
}

fn listing_fulfillment_mode(listing: &GameListing) -> FulfillmentMode {
    if !listing_authorizations(listing).is_empty() {
        FulfillmentMode::Delegate
    } else if !listing_servers(listing).is_empty() && listing_spec(listing, "file_hash").is_some() {
        FulfillmentMode::Direct
    } else {
        FulfillmentMode::None
    }
}

fn listing_fulfillment_pubkey(listing: &GameListing) -> Option<String> {
    let references = listing_authorizations(listing);
    let (_, key) = references.first()?;
    references
        .iter()
        .all(|(_, candidate)| candidate == key)
        .then(|| key.clone())
}

fn published_listing(
    existing: Option<GameListing>,
    request: &PublishAdpListingRequest,
    result: &PublishAdpListingResult,
    commercial: &PublisherCommercialIntent,
) -> GameListing {
    let mut specs = request
        .servers
        .iter()
        .map(|server| ("server".to_string(), server.clone()))
        .collect::<Vec<_>>();
    if let Some(existing) = &existing {
        specs.extend(
            existing
                .specs
                .iter()
                .filter(|(name, _)| {
                    name == "fulfillment_authorization"
                        || (matches!(commercial, PublisherCommercialIntent::Preserve)
                            && name.starts_with("adp_terms:"))
                })
                .cloned(),
        );
    }
    if let PublisherCommercialIntent::ReplaceAfterReview { review } = commercial {
        specs.extend(review.accepted_terms.iter().map(|reference| {
            (
                format!("adp_terms:{}", reference.operator_pubkey),
                reference.terms_event_id.clone(),
            )
        }));
    }
    if let Some(file_hash) = result
        .file_hash
        .clone()
        .or_else(|| request.existing_file_hash.clone())
    {
        specs.push(("file_hash".into(), file_hash));
    }
    if let Some(version) = request.version.clone() {
        specs.push(("version".into(), version));
    }
    if let (Some(root_event_id), Some(fulfillment_pubkey)) = (
        result.acceptance_event_id.clone(),
        result.fulfillment_pubkey.clone(),
    ) {
        specs.push((
            "fulfillment_authorization".into(),
            serde_json::json!({
                "root_event_id": root_event_id,
                "fulfillment_pubkey": fulfillment_pubkey,
                "relay_hint": null,
            })
            .to_string(),
        ));
    }

    let created_at = existing
        .as_ref()
        .map(|listing| listing.created_at)
        .unwrap_or_else(|| (js_sys::Date::now() / 1000.0) as u64);
    GameListing {
        // The identifier is whatever the backend actually published, never a
        // form value: a first publication only learns it from the result.
        id: result.d_tag.clone(),
        source: ListingSource::Nip99Listing,
        title: request.title.clone(),
        description: request.description.clone(),
        images: request.images.clone(),
        download_url: request.images.first().cloned().unwrap_or_default(),
        price: request.price_sats as f64,
        currency: "SATS".into(),
        price_sats: request.price_sats,
        quantity: None,
        tags: request.tags.clone(),
        specs,
        publisher_npub: request.expected_publisher_npub.clone(),
        stall_id: String::new(),
        stall_name: None,
        lud16: request.lud16.clone().unwrap_or_default(),
        event_id: Some(result.event_id.clone()),
        created_at,
        platforms: request.platforms.clone(),
        nip94_event_id: request.nip94_event_id.clone(),
        acquisition: request.acquisition.clone(),
        campaigns: request
            .campaigns
            .iter()
            .map(|campaign| CampaignPointer {
                root_event_id: campaign.root_event_id.clone(),
                relay_hint: campaign.relay_hint.clone(),
            })
            .collect(),
        is_owned: false,
        #[cfg(debug_assertions)]
        nip99_raw_event_json: None,
    }
}

fn initial_operator_url(editing_delegated_listing: bool, default: Option<&str>) -> String {
    if editing_delegated_listing {
        String::new()
    } else {
        default.unwrap_or_default().to_string()
    }
}

fn operator_resolution_request(listing: &GameListing) -> Option<ResolveAdpOperatorRequest> {
    if !matches!(listing_fulfillment_mode(listing), FulfillmentMode::Delegate) {
        return None;
    }
    let fulfillment_pubkey = listing_fulfillment_pubkey(listing)?;
    Some(ResolveAdpOperatorRequest {
        publisher_npub: listing.publisher_npub.clone(),
        fulfillment_pubkey,
        scope: listing.id.clone(),
    })
}

fn operator_prefill_update(
    current_operator_url: &str,
    resolution: Result<Option<String>, String>,
) -> Option<String> {
    if !current_operator_url.is_empty() {
        return None;
    }
    match resolution {
        Ok(Some(url)) => Some(url),
        Ok(None) | Err(_) => None,
    }
}

/// Everything the editor holds when Publish is pressed. Plain values, so the
/// request built from them can be tested without the reactive form.
#[derive(Clone, Debug)]
struct PublishFormInput {
    initiating_npub: String,
    title: String,
    description: String,
    tags: String,
    lud16: String,
    acquisition: AcquisitionKind,
    price: String,
    starts_at: String,
    ends_at: String,
    fulfillment_enabled: bool,
    fulfillment_mode: FulfillmentMode,
    servers: Vec<ServerEntry>,
    operator_url: String,
    server_roles: Vec<arcadestr_core::publisher_management::ServerRoleIntent>,
    authorize_operators: bool,
    file_hash: Option<String>,
    version: String,
    platforms: String,
    publish_state: PublishState,
    store_page_loaded: bool,
    store_page: StorePageDraft,
    commercial_mode: String,
    commercial_review: Option<PublisherTermsReview>,
    commercial_approved: bool,
    acknowledged_removed_profile_ids: Vec<ProfileId>,
    identity: PublishedIdentity,
    existing_fulfillment_pubkey: Option<String>,
    existing_campaigns: Vec<CampaignPointerInput>,
    existing_nip94_event_id: Option<String>,
    authoritative_file_hash: Option<String>,
}

/// Why Publish was refused before anything was sent.
#[derive(Clone, Debug, PartialEq)]
enum PublishBlocked {
    Message(String),
    /// A Store page problem; the editor opens the field that owns it.
    StorePage(String),
    /// A typed failure the publish attempt records, so a retry sees it.
    Attempt(PublishAdpListingError),
}

/// Build the exact request the backend signs and publishes. Pure: the same
/// input always produces the same request, whichever screen collected it.
fn build_publish_request(input: PublishFormInput) -> Result<PreparedPublish, PublishBlocked> {
    use PublishBlocked::{Attempt, Message};
    let title = input.title.trim().to_string();
    let description = input.description.trim().to_string();
    let lud16 = input.lud16.trim().to_string();
    let price = price_for_acquisition(input.acquisition, &input.price, &lud16).map_err(Message)?;
    let fulfillment_mode = if input.fulfillment_enabled {
        input.fulfillment_mode
    } else {
        FulfillmentMode::None
    };
    let acquisition =
        acquisition_policy_from_form(input.acquisition, &input.starts_at, &input.ends_at)
            .map_err(Message)?;
    validate_listing(
        &title,
        &description,
        price,
        &lud16,
        input.fulfillment_enabled,
        &input.servers,
        &None,
        &input.file_hash,
        &input.version,
        &fulfillment_mode,
        &input.operator_url,
    )
    .map_err(Message)?;
    let platforms = parse_platform_tags(&input.platforms).map_err(Message)?;
    let publisher_hex = publisher_hex(&input.initiating_npub)
        .ok_or_else(|| Message("The active publisher key is invalid.".into()))?;
    if !input.store_page_loaded {
        return Err(Message(
            "Wait for the store page to load before publishing.".into(),
        ));
    }
    let store_page = synced_store_page(input.store_page, &title, &description, &input.tags);
    if let Some(problem) = store_page_problems(&store_page, &publisher_hex)
        .into_iter()
        .next()
    {
        return Err(PublishBlocked::StorePage(problem));
    }
    // The cover and screenshots on the Store Page are the listing images.
    let images =
        arcadestr_core::store_page_seed::listing_images_from_media(&store_page.content.media);

    let selected_build = input
        .fulfillment_enabled
        .then(|| input.publish_state.archive.selected_build.clone())
        .flatten();
    let selection_id = selected_build.as_ref().map(|build| &build.selection_id);
    let launch_tier = input
        .publish_state
        .launch
        .to_wire_result(selection_id)
        .map_err(|violation| {
            Attempt(PublishAdpListingError::CoreValidation {
                violation,
                message: "Finish the launch settings before publishing.".into(),
            })
        })?;
    let commercial_intent = reviewed_commercial_intent(
        &input.commercial_mode,
        &PublisherTermsContext {
            publisher_pubkey: publisher_hex,
            servers: if input.fulfillment_enabled {
                input.servers.iter().map(|s| s.url.clone()).collect()
            } else {
                Vec::new()
            },
            price_sats: price,
            developer_destination: if input.acquisition == AcquisitionKind::Gated {
                lud16.clone()
            } else {
                String::new()
            },
        },
        input.commercial_review,
        input.commercial_approved,
    )
    .map_err(Message)?;
    let extension = PublishAdpListingRequestExtension {
        store_page: Some(store_page),
        authorize_operators: input.authorize_operators,
        server_roles: input
            .servers
            .iter()
            .map(|server| {
                input
                    .server_roles
                    .iter()
                    .find(|role| role.server_url == server.url)
                    .cloned()
                    .unwrap_or(arcadestr_core::publisher_management::ServerRoleIntent {
                        server_url: server.url.clone(),
                        issue_receipt: server.url == input.operator_url,
                        issue_grant: server.url == input.operator_url,
                    })
            })
            .collect(),
        operation_id: None,
        commercial_intent,
        launch_tier,
        acknowledged_removed_profile_ids: input.acknowledged_removed_profile_ids,
    };
    let enabled = input.fulfillment_enabled;
    let operator_url = input.operator_url;
    let version = input.version;
    let request = PublishAdpListingRequest {
        expected_publisher_npub: input.initiating_npub,
        existing_event_id: input.identity.event_id,
        existing_d_tag: input.identity.d_tag,
        pending_d_tag: input.identity.pending_d_tag,
        title,
        description,
        price_sats: price,
        lud16: (input.acquisition == AcquisitionKind::Gated).then_some(lud16),
        tags: parse_csv_values(&input.tags),
        images,
        fulfillment_mode,
        operator_url: when_fulfillment_enabled(
            enabled,
            (!operator_url.trim().is_empty()).then_some(operator_url),
        )
        .flatten(),
        servers: when_fulfillment_enabled(
            enabled,
            input.servers.into_iter().map(|entry| entry.url).collect(),
        )
        .unwrap_or_default(),
        accepted_terms: Vec::new(),
        file_path: None,
        selected_build: selected_build.clone(),
        existing_file_hash: (enabled && selected_build.is_none())
            .then_some(input.file_hash)
            .flatten(),
        existing_fulfillment_pubkey: when_fulfillment_enabled(
            enabled,
            input.existing_fulfillment_pubkey,
        )
        .flatten(),
        version: when_fulfillment_enabled(enabled, (!version.trim().is_empty()).then_some(version))
            .flatten(),
        acquisition,
        platforms,
        campaigns: input.existing_campaigns,
        nip94_event_id: input.existing_nip94_event_id,
    };
    prepare_publish(
        request,
        extension,
        selected_build,
        enabled,
        input.authoritative_file_hash.as_deref(),
    )
    .map_err(|error| Attempt(PublishAdpListingError::Archive { error }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn archive_errors_keep_their_precise_cause() {
        use arcadestr_core::archive_inspection_contract::{
            ArchiveDetectionFailure, ArchiveHandleFailure, ArchiveMutationFailure,
            ArchiveSelectionId, ArchiveStorageOperation,
        };
        let id = ArchiveSelectionId::parse("2f9a1c34-5b6d-4e7f-8a9b-0c1d2e3f4a5b").expect("id");
        let cases = [
            (
                ArchiveSelectionError::Expired {
                    selection_id: id.clone(),
                },
                "expired",
            ),
            (
                ArchiveSelectionError::Handle {
                    selection_id: id.clone(),
                    failure: ArchiveHandleFailure::MissingStaging,
                },
                "staged build is missing",
            ),
            (
                ArchiveSelectionError::Mutation {
                    failure: ArchiveMutationFailure::TimestampChanged,
                },
                "changed after",
            ),
            (
                ArchiveSelectionError::InsufficientStorage {
                    required: 10,
                    available: 1,
                },
                "storage",
            ),
            (
                ArchiveSelectionError::Storage {
                    operation: ArchiveStorageOperation::CopySource,
                },
                "local storage",
            ),
            (
                ArchiveSelectionError::Detection {
                    failure: ArchiveDetectionFailure::UnknownFormat,
                },
                "supported game archive",
            ),
            (ArchiveSelectionError::Transport, "desktop"),
        ];
        for (error, expected) in cases {
            let error = PublishAdpListingError::Archive { error };
            let wire = serde_json::to_value(&error).expect("encode IPC rejection");
            let decoded = serde_json::from_value(wire).expect("decode IPC rejection");
            assert_eq!(error, decoded);
            let message = publish_error_message(&decoded);
            assert!(
                message.to_lowercase().contains(expected),
                "{error:?}: {message}"
            );
            assert_ne!(message, "archive selection rejected publication");
            assert!(!message.to_lowercase().contains("launch profile"));
        }
        let partial = PublishAdpListingError::PartialPublication {
            cause: Box::new(PublishAdpListingError::Archive {
                error: ArchiveSelectionError::Expired { selection_id: id },
            }),
            successful_servers: vec!["https://a".into()],
        };
        assert!(publish_error_message(&partial).contains("expired"));
    }

    #[test]
    fn commercial_consent_defaults_to_preserve_and_requires_fresh_explicit_review() {
        let context = PublisherTermsContext {
            publisher_pubkey: "publisher".into(),
            servers: vec!["https://operator.test".into()],
            price_sats: 100,
            developer_destination: "developer@example.test".into(),
        };
        let review = PublisherTermsReview {
            context: context.clone(),
            accepted_terms: vec![arcadestr_core::adp_settlement::AcceptedTermsRef {
                operator_pubkey: "operator".into(),
                terms_event_id: "old-root".into(),
            }],
            offers: Vec::new(),
            legs: Vec::new(),
        };
        assert_eq!(
            reviewed_commercial_intent("preserve", &context, Some(review.clone()), false),
            Ok(PublisherCommercialIntent::Preserve)
        );
        assert!(
            reviewed_commercial_intent("replace", &context, Some(review.clone()), false).is_err()
        );
        assert!(reviewed_commercial_intent("replace", &context, None, true).is_err());
        assert!(matches!(
            reviewed_commercial_intent("replace", &context, Some(review.clone()), true),
            Ok(PublisherCommercialIntent::ReplaceAfterReview { .. })
        ));
        let mut changed = context.clone();
        changed.price_sats += 1;
        assert!(
            reviewed_commercial_intent("replace", &changed, Some(review.clone()), true).is_err()
        );
        let mut changed = context.clone();
        changed.servers.push("https://second.test".into());
        assert!(
            reviewed_commercial_intent("replace", &changed, Some(review.clone()), true).is_err()
        );
        let mut changed = context.clone();
        changed.publisher_pubkey = "another publisher".into();
        assert!(
            reviewed_commercial_intent("replace", &changed, Some(review.clone()), true).is_err()
        );
        assert!(reviewed_commercial_intent("retire", &context, Some(review), true).is_err());
    }

    const VALID_SHA256: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    fn frozen_prepared() -> PreparedPublish {
        PreparedPublish {
            request: PublishAdpListingRequest {
                expected_publisher_npub: "npub-test".into(),
                existing_event_id: Some("event-before-check".into()),
                existing_d_tag: Some("game".into()),
                pending_d_tag: None,
                title: "Frozen title".into(),
                description: "Frozen description".into(),
                price_sats: 0,
                lud16: None,
                tags: vec!["action".into()],
                images: vec!["https://example.com/image.png".into()],
                fulfillment_mode: FulfillmentMode::None,
                operator_url: None,
                servers: Vec::new(),
                accepted_terms: Vec::new(),
                file_path: None,
                selected_build: None,
                existing_file_hash: None,
                existing_fulfillment_pubkey: None,
                version: None,
                acquisition: AcquisitionPolicy::Public,
                platforms: Vec::new(),
                campaigns: Vec::new(),
                nip94_event_id: None,
            },
            extension: PublishAdpListingRequestExtension::default(),
        }
    }

    fn profile_id(value: &str) -> ProfileId {
        ProfileId::parse(value).expect("test profile ID must be valid")
    }

    #[test]
    fn removal_confirmation_mutates_only_exact_acknowledgement_ids() {
        let frozen = frozen_prepared();
        let ids = vec![profile_id("legacy"), profile_id("old-vulkan")];
        let confirmed = removal_confirmation_prepared(&frozen, &ids);

        assert_eq!(confirmed.request, frozen.request);
        assert_eq!(
            confirmed.extension.launch_tier,
            frozen.extension.launch_tier
        );
        assert_eq!(confirmed.extension.acknowledged_removed_profile_ids, ids);
        assert!(frozen.extension.acknowledged_removed_profile_ids.is_empty());
    }

    #[test]
    fn removal_confirmation_freshness_covers_form_dirty_identity_and_generation() {
        let form = FormSnapshot::default();
        let dirty = PublishDirtySnapshot::capture(
            &ArchiveSelectionState::new("npub-test".into()),
            &LaunchMode::DownloadOnly,
            ProviderBuildFields::default(),
            Vec::new(),
            DefectReviewState::default(),
        );
        let identity = RemovalConfirmationIdentity {
            account_npub: Some("npub-test".into()),
            publication: PublishedIdentity::default(),
            authoritative_state: None,
        };
        let pending = PendingRemovalConfirmation {
            prepared: frozen_prepared(),
            removed_profile_ids: vec![profile_id("legacy")],
            form: form.clone(),
            dirty: dirty.clone(),
            identity: identity.clone(),
            generation: 7,
            abort_count: 1,
        };

        assert!(removal_confirmation_is_fresh(
            &pending, &form, &dirty, &identity, 7
        ));
        let mut changed = form.clone();
        changed.title = "changed".into();
        assert!(!removal_confirmation_is_fresh(
            &pending, &changed, &dirty, &identity, 7
        ));
        let mut changed_dirty = dirty.clone();
        changed_dirty.defect_review.reviewed = true;
        assert!(!removal_confirmation_is_fresh(
            &pending,
            &form,
            &changed_dirty,
            &identity,
            7
        ));
        let mut changed_identity = identity.clone();
        changed_identity.account_npub = Some("npub-other".into());
        assert!(!removal_confirmation_is_fresh(
            &pending,
            &form,
            &dirty,
            &changed_identity,
            7
        ));
        let mut changed_revision = identity.clone();
        changed_revision.publication.event_id = Some("new-revision".into());
        assert!(!removal_confirmation_is_fresh(
            &pending,
            &form,
            &dirty,
            &changed_revision,
            7
        ));
        let mut stale_authority = identity.clone();
        stale_authority.authoritative_state = Some(AuthoritativeEditState::Stale(
            PublishAdpListingError::StaleExpectedEventId {
                expected_event_id: "expected".into(),
                actual_event_id: "actual".into(),
            },
        ));
        assert!(!removal_confirmation_is_fresh(
            &pending,
            &form,
            &dirty,
            &stale_authority,
            7
        ));
        assert!(!removal_confirmation_is_fresh(
            &pending, &form, &dirty, &identity, 8
        ));
    }

    #[test]
    fn second_removal_abort_names_every_new_id_and_requires_fresh_confirmation() {
        let copy =
            removal_confirmation_copy(&[profile_id("old-main"), profile_id("old-config")], 2);
        assert!(copy.contains("backend rechecked the latest revision"));
        assert!(copy.contains("fresh complete removal set"));
        assert!(copy.contains("old-main"));
        assert!(copy.contains("old-config"));
        assert!(copy.contains("confirm again"));
    }

    #[test]
    fn stale_revision_message_preserves_work_and_blocks_overwrite() {
        let message = publish_error_message(&PublishAdpListingError::StaleExpectedEventId {
            expected_event_id: "expected".into(),
            actual_event_id: "actual".into(),
        });

        assert!(message.contains("changes are saved as a draft"));
        assert!(message.contains("before publishing"));
        assert!(message.contains("open the game again"));
    }

    #[test]
    fn format_sha256_abbreviates_valid_ascii_hex() {
        let hash = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

        assert_eq!(format_sha256(hash), "0123456789ab...456789abcdef");
    }

    #[test]
    fn format_sha256_rejects_malformed_metadata() {
        for hash in [
            "",
            "abc123",
            "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcde",
            "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef0",
            "g123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
            "é123456789abcdef0123456789abcdef0123456789abcdef0123456789abcde",
        ] {
            assert_eq!(format_sha256(hash), "Invalid SHA-256 metadata");
        }
    }

    fn managed_listing(publisher_npub: String, fulfillment_pubkey: String) -> GameListing {
        GameListing {
            id: "managed-game".into(),
            source: crate::models::ListingSource::Nip99Listing,
            title: "Managed Game".into(),
            description: "Description".into(),
            images: Vec::new(),
            download_url: String::new(),
            price: 0.0,
            currency: "SATS".into(),
            price_sats: 0,
            quantity: None,
            tags: Vec::new(),
            specs: vec![
                ("server".into(), "http://localhost:9099".into()),
                ("version".into(), "1.4.2".into()),
                ("file_hash".into(), VALID_SHA256.into()),
                (
                    "fulfillment_authorization".into(),
                    serde_json::json!({
                        "root_event_id": "11".repeat(32),
                        "fulfillment_pubkey": fulfillment_pubkey,
                        "relay_hint": null,
                    })
                    .to_string(),
                ),
            ],
            publisher_npub,
            stall_id: String::new(),
            stall_name: None,
            lud16: String::new(),
            event_id: Some("event-id".into()),
            created_at: 1,
            platforms: Vec::new(),
            nip94_event_id: None,
            acquisition: crate::models::AcquisitionPolicy::Gated,
            campaigns: Vec::new(),
            is_owned: false,
            #[cfg(debug_assertions)]
            nip99_raw_event_json: None,
        }
    }

    #[test]
    fn prior_revision_file_hash_is_lowercased_at_capture() {
        let mut listing = managed_listing("publisher".to_owned(), "fulfillment".to_owned());
        let uppercase = VALID_SHA256.to_ascii_uppercase();
        listing
            .specs
            .iter_mut()
            .find(|(name, _)| name == "file_hash")
            .expect("file_hash fixture exists")
            .1 = uppercase;

        assert_eq!(captured_file_hash(&listing), Some(VALID_SHA256.to_owned()));
    }

    #[test]
    fn parse_platform_tags_trims_values_and_discards_empty_entries() {
        let platforms = parse_platform_tags(" linux-x86_64, ,windows-x86_64, macos-aarch64 ")
            .expect("valid platform tags should parse");

        assert_eq!(
            platforms,
            vec!["linux-x86_64", "windows-x86_64", "macos-aarch64"]
        );
    }

    #[test]
    fn platform_selection_adds_unique_targets_and_removes_them() {
        let selected = add_platform("linux-x86_64", "windows-aarch64");
        assert_eq!(selected, "linux-x86_64, windows-aarch64");
        assert_eq!(add_platform(&selected, "linux-x86_64"), selected);
        assert_eq!(
            remove_platform(&selected, "linux-x86_64"),
            "windows-aarch64"
        );
    }

    #[test]
    fn empty_platform_selection_means_all_platforms() {
        assert_eq!(platform_summary(""), "All platforms");
    }

    #[test]
    fn editable_tags_hide_implicit_game_and_remove_duplicates() {
        let tags = vec![
            "game".to_string(),
            " game ".to_string(),
            "test".to_string(),
            "TEST".to_string(),
        ];

        assert_eq!(editable_listing_tags(&tags), "test");
    }

    #[test]
    fn game_tag_selection_adds_unique_tags_and_removes_them_case_insensitively() {
        let selected = add_game_tag("Action", "Multiplayer");
        assert_eq!(selected, "Action, Multiplayer");
        assert_eq!(add_game_tag(&selected, "action"), selected);
        assert_eq!(remove_game_tag(&selected, "ACTION"), "Multiplayer");
    }

    #[test]
    fn parse_platform_tags_rejects_whitespace_inside_tag() {
        let err = parse_platform_tags("linux x86_64")
            .expect_err("platform tags with whitespace should be rejected");

        assert_eq!(err, "Platform tags cannot contain whitespace");
    }

    #[test]
    fn parse_platform_tags_rejects_tags_without_os_arch_separator() {
        let err =
            parse_platform_tags("linux").expect_err("platform tags without '-' should be rejected");

        assert_eq!(err, "Platform tags must look like <os>-<arch>");
    }

    #[test]
    fn existing_fulfillment_hash_does_not_require_reselecting_build_file() {
        let result = validate_listing(
            "Managed Game",
            "Description",
            0,
            "",
            true,
            &[ServerEntry {
                url: "http://localhost:9099".into(),
                label: "Published server".into(),
                reachability: ServerStatus::Ok,
                upload: ServerStatus::Idle,
                auto_operator: false,
            }],
            &None,
            &Some(VALID_SHA256.into()),
            "1.4.2",
            &FulfillmentMode::Delegate,
            "http://localhost:9099",
        );

        assert_eq!(result, Ok(()));
    }

    #[test]
    fn malformed_existing_fulfillment_hash_requires_replacement_file() {
        let result = validate_listing(
            "Managed Game",
            "Description",
            0,
            "",
            true,
            &[ServerEntry {
                url: "http://localhost:9099".into(),
                label: "Published server".into(),
                reachability: ServerStatus::Ok,
                upload: ServerStatus::Idle,
                auto_operator: false,
            }],
            &None,
            &Some("abc123".into()),
            "1.4.2",
            &FulfillmentMode::Delegate,
            "http://localhost:9099",
        );

        assert_eq!(
            result,
            Err("Existing SHA-256 metadata is invalid; select a replacement build file".into())
        );
    }

    #[test]
    fn delegated_publication_defaults_are_recovered_from_listing_specs() {
        use nostr::nips::nip19::ToBech32;

        let publisher = nostr::Keys::generate();
        let delegate = nostr::Keys::generate();
        let listing = managed_listing(
            publisher
                .public_key()
                .to_bech32()
                .expect("publisher npub should encode"),
            delegate.public_key().to_hex(),
        );

        assert_eq!(listing_servers(&listing), vec!["http://localhost:9099"]);
        assert_eq!(listing_spec(&listing, "version").as_deref(), Some("1.4.2"));
        assert_eq!(
            listing_spec(&listing, "file_hash").as_deref(),
            Some(VALID_SHA256)
        );
        assert_eq!(
            listing_fulfillment_mode(&listing),
            FulfillmentMode::Delegate
        );
        assert_eq!(
            listing_fulfillment_pubkey(&listing),
            Some(delegate.public_key().to_hex())
        );
        assert_eq!(
            initial_operator_url(true, Some("https://provider.example")),
            ""
        );

        let request = operator_resolution_request(&listing)
            .expect("delegated edit should request an exact local operator lookup");
        assert_eq!(request.publisher_npub, listing.publisher_npub);
        assert_eq!(request.fulfillment_pubkey, delegate.public_key().to_hex());
        assert_eq!(request.scope, "managed-game");
    }

    #[test]
    fn new_publications_use_the_configured_distribution_provider() {
        let configured = configured_default_provider(Some(" https://provider.example "));
        assert_eq!(configured, Some("https://provider.example"));
        assert_eq!(
            initial_operator_url(false, configured),
            "https://provider.example"
        );
    }

    #[test]
    fn unconfigured_builds_require_an_explicit_distribution_provider() {
        assert_eq!(configured_default_provider(None), None);
        assert_eq!(configured_default_provider(Some("  ")), None);
        assert_eq!(initial_operator_url(false, None), "");
    }

    #[test]
    fn multioperator_metadata_does_not_choose_a_first_key_or_drop_sibling_references() {
        use nostr::nips::nip19::ToBech32;
        let publisher = nostr::Keys::generate()
            .public_key()
            .to_bech32()
            .expect("npub");
        let mut listing = managed_listing(
            publisher.clone(),
            nostr::Keys::generate().public_key().to_hex(),
        );
        listing.specs.push((
            "fulfillment_authorization".into(),
            serde_json::json!({
                "root_event_id": "22".repeat(32),
                "fulfillment_pubkey": nostr::Keys::generate().public_key().to_hex(),
                "relay_hint": "wss://sibling.example"
            })
            .to_string(),
        ));
        listing.specs.push((
            "adp_terms:operator-one".into(),
            "exact-original-root".into(),
        ));
        let original = listing
            .specs
            .iter()
            .filter(|(name, _)| name == "fulfillment_authorization")
            .cloned()
            .collect::<Vec<_>>();
        assert_eq!(listing_fulfillment_pubkey(&listing), None);
        assert!(operator_resolution_request(&listing).is_none());
        let mut request = frozen_prepared().request;
        request.expected_publisher_npub = publisher;
        let result = PublishAdpListingResult {
            operation_id: "metadata-operation".into(),
            event_id: "new-event".into(),
            game_id: None,
            d_tag: listing.id.clone(),
            game_coordinate: "coordinate".into(),
            acceptance_event_id: None,
            fulfillment_pubkey: None,
            file_hash: None,
            uploads: Vec::new(),
            store_page: None,
        };
        let updated = published_listing(
            Some(listing),
            &request,
            &result,
            &PublisherCommercialIntent::Preserve,
        );
        assert!(updated.specs.contains(&(
            "adp_terms:operator-one".into(),
            "exact-original-root".into()
        )));
        assert_eq!(
            updated
                .specs
                .into_iter()
                .filter(|(name, _)| name == "fulfillment_authorization")
                .collect::<Vec<_>>(),
            original
        );
    }

    #[test]
    fn operator_prefill_only_applies_unique_success_to_empty_input() {
        assert_eq!(
            operator_prefill_update("", Ok(Some("https://operator.example.com".to_string())))
                .as_deref(),
            Some("https://operator.example.com")
        );
        assert_eq!(
            operator_prefill_update(
                "https://manual.example.com",
                Ok(Some("https://operator.example.com".to_string()))
            ),
            None
        );
        assert_eq!(operator_prefill_update("", Ok(None)), None);
        assert_eq!(
            operator_prefill_update("", Err("lookup failed".to_string())),
            None
        );
    }

    #[test]
    fn acquisition_kind_recovers_existing_policy() {
        assert_eq!(
            acquisition_kind(&AcquisitionPolicy::TimedAccess {
                starts_at: 100,
                ends_at: 200,
            }),
            AcquisitionKind::TimedAccess
        );
        assert_eq!(
            acquisition_kind(&AcquisitionPolicy::Public),
            AcquisitionKind::Public
        );
        assert_eq!(
            acquisition_kind(&AcquisitionPolicy::Gated),
            AcquisitionKind::Gated
        );
    }

    #[test]
    fn timed_acquisition_form_requires_ordered_dates() {
        let policy = acquisition_policy_from_form(
            AcquisitionKind::TimedAccess,
            "2026-07-18T12:30",
            "2026-07-18T13:30",
        )
        .expect("ordered dates should be accepted");
        assert!(matches!(
            policy,
            AcquisitionPolicy::TimedAccess { starts_at, ends_at } if starts_at < ends_at
        ));

        let error = acquisition_policy_from_form(
            AcquisitionKind::TimedAccess,
            "2026-07-18T13:30",
            "2026-07-18T12:30",
        )
        .expect_err("reversed dates should be rejected");
        assert_eq!(error, "Timed access must end after it starts");
    }

    #[test]
    fn drafts_are_autosaved_to_the_account_scoped_database() {
        let source = include_str!("publish.rs");
        assert!(source.contains("invoke_save_publisher_draft"));
        assert!(source.contains("invoke_delete_publisher_draft"));
        // Manual save/restore controls were replaced by autosave.
        assert!(!source.contains(concat!("SAVE ", "DRAFT\"")));
        assert!(!source.contains(concat!("RESTORE SAVED ", "DRAFT")));
    }

    #[test]
    fn paid_access_requires_positive_parseable_sats_and_lud16() {
        assert!(
            price_for_acquisition(AcquisitionKind::Gated, "abc", "seller@example.com").is_err()
        );
        assert!(price_for_acquisition(AcquisitionKind::Gated, "0", "seller@example.com").is_err());
        assert!(price_for_acquisition(AcquisitionKind::Gated, "25", "").is_err());
        assert_eq!(
            price_for_acquisition(AcquisitionKind::Gated, "25", "seller@example.com"),
            Ok(25)
        );
    }

    #[test]
    fn public_and_timed_access_force_zero_price() {
        assert_eq!(
            price_for_acquisition(AcquisitionKind::Public, "invalid", ""),
            Ok(0)
        );
        assert_eq!(
            price_for_acquisition(AcquisitionKind::TimedAccess, "25", "seller@example.com"),
            Ok(0)
        );
        assert!(matches!(
            acquisition_policy_from_form(AcquisitionKind::Public, "", ""),
            Ok(AcquisitionPolicy::Public)
        ));
    }

    #[test]
    fn duplicate_and_malformed_platform_tags_are_rejected() {
        assert_eq!(
            parse_platform_tags("linux-x86_64,linux-x86_64"),
            Err("Duplicate platform tag: linux-x86_64".into())
        );
        assert!(parse_platform_tags("linux-x86_64-extra").is_err());
        assert!(parse_platform_tags("-x86_64").is_err());
    }

    #[test]
    fn image_urls_must_be_http_or_https() {
        assert!(validate_http_urls(
            "https://example.com/a.png, http://example.com/b.png",
            "Image URL"
        )
        .is_ok());
        assert!(validate_http_urls("file:///tmp/a.png", "Image URL").is_err());
        assert!(validate_http_urls("https:///missing-host.png", "Image URL").is_err());
    }

    #[test]
    fn selecting_a_different_file_invalidates_the_hash() {
        let (path, hash) = file_selection_changed(Some("/tmp/new.zip".into()));
        assert_eq!(path.as_deref(), Some("/tmp/new.zip"));
        assert_eq!(hash, None);
    }

    #[test]
    fn readiness_reports_provider_reachability_and_existing_hash_reuse() {
        let servers = vec![ServerEntry {
            url: "https://dist.example.com".into(),
            label: "Provider".into(),
            reachability: ServerStatus::Failed,
            upload: ServerStatus::Idle,
            auto_operator: false,
        }];
        let checklist = readiness_checklist(
            "Game",
            "Description",
            "https://example.com/cover.png",
            AcquisitionKind::Public,
            "0",
            "",
            Ok(AcquisitionPolicy::Public),
            "linux-x86_64",
            true,
            &servers,
            Some(VALID_SHA256),
            "1.0.0",
            &FulfillmentMode::Direct,
            "",
        );
        assert!(checklist.metadata);
        assert!(checklist.pricing_and_access);
        assert!(checklist.distribution, "a valid existing hash is reusable");
        assert!(checklist.authorization);
        assert!(checklist
            .warnings
            .iter()
            .any(|warning| warning.contains("couldn't be reached")));
    }

    #[test]
    fn publication_reducer_never_marks_upload_failure_complete() {
        let published = publication_progress(
            PublicationState {
                outcome: PublicationOutcome::Publishing,
                ..Default::default()
            },
            &PublishProgressPayload {
                step: "publish-listing".into(),
                status: "ok".into(),
                server_url: None,
                message: None,
                bytes_uploaded: None,
                total_bytes: None,
                ..Default::default()
            },
        );
        let partial = publication_progress(
            published,
            &PublishProgressPayload {
                step: "upload".into(),
                status: "error".into(),
                server_url: Some("https://dist.example.com".into()),
                message: Some("offline".into()),
                bytes_uploaded: None,
                total_bytes: None,
                ..Default::default()
            },
        );
        assert_eq!(partial.outcome, PublicationOutcome::Partial);
        assert!(partial.listing_published);
        assert!(partial
            .message
            .as_deref()
            .is_some_and(|message| message.contains("Your game is live")));

        let command_failure = publication_completed(partial, Err("upload failed".into()));
        assert_eq!(command_failure.outcome, PublicationOutcome::Partial);
        assert_ne!(command_failure.outcome, PublicationOutcome::Complete);
    }

    #[test]
    fn progress_percentage_is_bounded_and_handles_empty_files() {
        assert_eq!(progress_percent(25, 100), 25);
        assert_eq!(progress_percent(100, 100), 100);
        assert_eq!(progress_percent(0, 0), 100);
    }

    #[test]
    fn stale_account_and_duplicate_dispatch_guards_are_explicit() {
        assert!(publication_account_matches("npub1a", Some("npub1a")));
        assert!(!publication_account_matches("npub1a", Some("npub1b")));
        assert!(!publication_account_matches("npub1a", None));
        assert!(publication_account_allowed(Some("npub1a"), "npub1a"));
        assert!(!publication_account_allowed(Some("npub1a"), "npub1b"));
        assert!(publication_account_allowed(None, "npub1b"));
        assert!(can_dispatch(false, false));
        assert!(!can_dispatch(true, false));
        assert!(!can_dispatch(false, true));
    }

    #[test]
    fn disabling_fulfillment_omits_hidden_configuration() {
        assert_eq!(when_fulfillment_enabled(false, Some("hash")), None);
        assert_eq!(when_fulfillment_enabled(false, vec!["server"]), None);
        assert_eq!(
            when_fulfillment_enabled(true, Some("hash")),
            Some(Some("hash"))
        );
    }

    #[test]
    fn hub_sections_keep_their_names_and_checklist_order() {
        assert_eq!(
            HubSection::EDITABLE.map(HubSection::title),
            [
                "Basics",
                "Store page",
                "Price & access",
                "Delivery & builds"
            ]
        );
        assert_eq!(HubSection::default(), HubSection::Overview);
        // Every Store page group has exactly one owning section; requirements now
        // live with the rest of the store page instead of the build settings.
        assert_eq!(
            HubSection::for_store_page_group("basic"),
            HubSection::Basics
        );
        for group in [
            "description",
            "media",
            "sections",
            "requirements",
            "languages",
            "accessibility",
            "links",
        ] {
            assert_eq!(
                HubSection::for_store_page_group(group),
                HubSection::StorePage,
                "{group}"
            );
        }
    }

    #[test]
    fn store_page_tabs_cover_every_store_page_group_once() {
        let mut groups = STORE_PAGE_TABS
            .iter()
            .flat_map(|(_, _, groups)| groups.iter().copied())
            .collect::<Vec<_>>();
        groups.sort_unstable();
        assert_eq!(
            groups,
            [
                "accessibility",
                "description",
                "languages",
                "links",
                "media",
                "requirements",
                "sections"
            ]
        );
        for (id, _, groups) in STORE_PAGE_TABS {
            assert_eq!(groups, [id]);
        }
    }

    #[test]
    fn section_status_follows_live_validation_only() {
        assert_eq!(section_status(&Ok(()), 0), SectionStatus::Ready);
        // An empty but valid store page is not "Ready".
        assert_eq!(section_status(&Ok(()), 3), SectionStatus::Suggestions);
        // Blocking problems outrank suggestions.
        assert_eq!(
            section_status(&Err("Title is required".into()), 3),
            SectionStatus::NeedsAttention
        );
        assert_ne!(
            SectionStatus::Suggestions.label(),
            SectionStatus::Ready.label()
        );
    }

    #[test]
    fn only_store_page_edits_on_a_linked_live_game_skip_the_game_publish() {
        // editing, settled, game fields, distribution, images, page, linked
        assert_eq!(
            update_scope(true, true, false, false, false, true, true),
            UpdateScope::StorePageOnly
        );
        for (case, scope) in [
            (
                "new game",
                update_scope(false, true, false, false, false, true, true),
            ),
            (
                "launch settings still loading",
                update_scope(true, false, false, false, false, true, true),
            ),
            (
                "basics or price changed",
                update_scope(true, true, true, false, false, true, true),
            ),
            (
                "build or delivery changed",
                update_scope(true, true, false, true, false, true, true),
            ),
            (
                "card or screenshot images changed",
                update_scope(true, true, false, false, true, true, true),
            ),
            (
                "store page not linked to the game yet",
                update_scope(true, true, false, false, false, true, false),
            ),
            (
                "nothing changed on the store page",
                update_scope(true, true, false, false, false, false, true),
            ),
        ] {
            assert_eq!(scope, UpdateScope::FullGame, "{case}");
        }
    }

    #[test]
    fn suggestions_put_the_description_and_card_image_first() {
        let ordered = prioritized_suggestions(&[
            "Add a trailer.".into(),
            "Add a hero image (the banner at the top of the store page).".into(),
            "Add a capsule image (shown on game cards).".into(),
            "Add a description.".into(),
        ]);
        assert_eq!(ordered[0], "Add a description.");
        assert!(ordered[1].starts_with("Add a capsule"));
        assert!(ordered[2].starts_with("Add a hero"));
        assert_eq!(ordered[3], "Add a trailer.");
    }

    #[test]
    fn blocking_issues_are_reported_in_checklist_order_with_actionable_text() {
        let issues = blocking_section_errors(&[
            (HubSection::Basics, Err("Title is required".to_string())),
            (HubSection::StorePage, Ok(())),
            (HubSection::PriceAccess, Ok(())),
            (
                HubSection::Delivery,
                Err("Version is required for fulfillment".to_string()),
            ),
        ]);
        assert_eq!(
            issues,
            vec![
                (HubSection::Basics, "Title is required".to_string()),
                (
                    HubSection::Delivery,
                    "Version is required for fulfillment".to_string()
                ),
            ]
        );
        assert!(blocking_section_errors(&[(HubSection::Basics, Ok(()))]).is_empty());
    }

    #[test]
    fn delivery_choice_is_explicit_for_new_games_and_derived_for_existing_ones() {
        // A new game has no delivery until the author picks one.
        assert_eq!(delivery_choice(false, true, true), None);
        assert_eq!(delivery_choice(false, false, false), None);
        assert_eq!(
            delivery_choice(true, true, true),
            Some(DeliveryChoice::Install)
        );
        assert_eq!(
            delivery_choice(true, true, false),
            Some(DeliveryChoice::DownloadOnly)
        );
        assert_eq!(
            delivery_choice(true, false, false),
            Some(DeliveryChoice::StorePageOnly)
        );
        assert!(DeliveryChoice::Install.uses_build());
        assert!(DeliveryChoice::DownloadOnly.uses_build());
        assert!(!DeliveryChoice::StorePageOnly.uses_build());
    }

    #[test]
    fn authoring_state_separates_drafts_from_live_games() {
        assert_eq!(
            authoring_state(false, false, 2),
            AuthoringState::DraftIncomplete
        );
        assert_eq!(authoring_state(false, true, 0), AuthoringState::DraftReady);
        assert_eq!(authoring_state(true, false, 2), AuthoringState::Live);
        assert_eq!(authoring_state(true, true, 0), AuthoringState::LiveChanged);
        assert_eq!(
            authoring_state(true, true, 1),
            AuthoringState::LiveChangedBlocked
        );
        // A draft is never described as live, and local edits never as published.
        for state in [AuthoringState::DraftIncomplete, AuthoringState::DraftReady] {
            assert!(state.label().starts_with("Draft"));
        }
        assert!(AuthoringState::LiveChanged
            .label()
            .contains("not published"));
    }

    #[test]
    fn only_edits_not_yet_in_the_local_draft_block_leaving() {
        assert!(!DraftSaveStatus::Idle.unsaved());
        assert!(!DraftSaveStatus::Saved.unsaved());
        assert!(DraftSaveStatus::Pending.unsaved());
        assert!(DraftSaveStatus::Saving.unsaved());
        assert!(DraftSaveStatus::Failed("disk full".into()).unsaved());
        assert!(DraftSaveStatus::Conflict.unsaved());
        assert_eq!(DraftSaveStatus::Idle.label(), None);
        assert!(DraftSaveStatus::Failed("disk full".into())
            .label()
            .is_some_and(|label| label.contains("disk full")));
    }

    #[test]
    fn draft_ids_follow_the_published_game_or_a_local_id() {
        assert_eq!(game_draft_id(Some("abc"), Some("local:old")), "listing:abc");
        assert_eq!(game_draft_id(None, Some("local:xyz")), "local:xyz");
        assert_eq!(game_draft_id(None, Some("xyz")), "local:xyz");
        let minted = game_draft_id(None, None);
        assert!(minted.starts_with("local:"));
        assert_ne!(minted, game_draft_id(None, None));
    }

    #[test]
    fn fee_labels_use_sats_and_percentages() {
        assert_eq!(fee_policy_label(&FeePolicy::None), "No service fee");
        assert_eq!(
            fee_policy_label(&FeePolicy::Fixed { amount_msat: 5_000 }),
            "5 sats per sale"
        );
        assert_eq!(
            fee_policy_label(&FeePolicy::BasisPoints {
                basis_points: 250,
                minimum_msat: Some(1_500),
                maximum_msat: None,
            }),
            "2.5% of each sale · at least 1.5 sats"
        );
        assert_eq!(
            fee_policy_label(&FeePolicy::BasisPoints {
                basis_points: 1_000,
                minimum_msat: None,
                maximum_msat: Some(100_000),
            }),
            "10% of each sale · at most 100 sats"
        );
        assert_eq!(msat_text(1), "0.001 sats");
    }

    #[test]
    fn dirty_detection_ignores_untouched_forms() {
        let pristine = FormSnapshot::default();
        let mut edited = pristine.clone();
        assert_eq!(pristine, edited);
        edited.title = "Nova".to_string();
        assert_ne!(pristine, edited);
    }

    #[test]
    fn edit_distribution_rebases_only_after_safe_async_initialization() {
        assert!(should_rebase_edit_distribution(true, true, 0));
        assert!(!should_rebase_edit_distribution(false, true, 0));
        assert!(!should_rebase_edit_distribution(true, false, 0));
        assert!(!should_rebase_edit_distribution(true, true, 1));
    }

    #[test]
    fn publication_phase_never_reports_success_before_an_authoritative_result() {
        assert_eq!(
            publication_phase(false, false, PublicationOutcome::Idle),
            PublicationPhase::Idle
        );
        assert_eq!(
            publication_phase(true, false, PublicationOutcome::Idle),
            PublicationPhase::Publishing
        );
        assert_eq!(
            publication_phase(false, false, PublicationOutcome::Publishing),
            PublicationPhase::Publishing
        );
        assert_eq!(
            publication_phase(false, false, PublicationOutcome::Partial),
            PublicationPhase::Partial
        );
        assert_eq!(
            publication_phase(false, false, PublicationOutcome::Failed),
            PublicationPhase::Failed
        );
        assert_eq!(
            publication_phase(false, false, PublicationOutcome::Complete),
            PublicationPhase::Published
        );
        // A stale account outranks any result produced for the previous account.
        assert_eq!(
            publication_phase(false, true, PublicationOutcome::Complete),
            PublicationPhase::StaleAccount
        );
        assert_ne!(
            PublicationPhase::Partial.label(),
            PublicationPhase::Published.label()
        );
    }

    #[test]
    fn the_new_game_form_has_no_editable_identifier_field() {
        let source = include_str!("publish.rs");

        assert!(!source.contains(concat!("publish", "-id")));
        assert!(!source.contains(concat!("Game page ", "identifier")));
        // The identifier may still be shown as a technical detail, but nothing
        // may bind it to an input.
        assert!(!source.contains(concat!("d_tag", ": id_val")));
    }

    #[test]
    fn publish_readiness_no_longer_requires_user_entered_identifier_text() {
        let checklist = readiness_checklist(
            "Game",
            "Description",
            "https://example.com/cover.png",
            AcquisitionKind::Public,
            "0",
            "",
            Ok(AcquisitionPolicy::Public),
            "linux-x86_64",
            false,
            &[],
            None,
            "",
            &FulfillmentMode::None,
            "",
        );

        assert!(
            checklist.metadata,
            "metadata readiness must not depend on an author-entered identifier"
        );
        assert_eq!(
            validate_listing(
                "Game",
                "Description",
                0,
                "",
                false,
                &[],
                &None,
                &None,
                "",
                &FulfillmentMode::None,
                "",
            ),
            Ok(())
        );
    }

    #[test]
    fn a_failure_after_listing_publication_preserves_the_published_coordinate() {
        let identity = PublishedIdentity::default();
        let identity = publication_identity_progress(
            identity,
            &PublishProgressPayload {
                step: "publish-listing".into(),
                status: "ok".into(),
                server_url: None,
                message: Some("listing-event-id".into()),
                bytes_uploaded: None,
                total_bytes: None,
                ..Default::default()
            },
        );
        let identity = publication_identity_progress(
            identity,
            &PublishProgressPayload {
                step: "listing-coordinate".into(),
                status: "ok".into(),
                server_url: None,
                message: Some("30402:publisherhex:2f9a1c34-5b6d-4e7f-8a9b-0c1d2e3f4a5b".into()),
                bytes_uploaded: None,
                total_bytes: None,
                ..Default::default()
            },
        );
        // The upload that follows fails; the identity survives it.
        let identity = publication_identity_progress(
            identity,
            &PublishProgressPayload {
                step: "upload".into(),
                status: "error".into(),
                server_url: Some("https://dist.example.com".into()),
                message: Some("connection refused".into()),
                bytes_uploaded: None,
                total_bytes: None,
                ..Default::default()
            },
        );

        assert_eq!(identity.event_id.as_deref(), Some("listing-event-id"));
        assert_eq!(
            identity.d_tag.as_deref(),
            Some("2f9a1c34-5b6d-4e7f-8a9b-0c1d2e3f4a5b")
        );
    }

    #[test]
    fn retrying_after_partial_publication_republishes_the_same_coordinate() {
        let recovered = PublishedIdentity {
            event_id: Some("listing-event-id".into()),
            d_tag: Some("2f9a1c34-5b6d-4e7f-8a9b-0c1d2e3f4a5b".into()),
            coordinate: Some("30402:publisherhex:2f9a1c34-5b6d-4e7f-8a9b-0c1d2e3f4a5b".into()),
            pending_d_tag: None,
        };

        // A retry sends both halves, which is exactly the replacement-edit
        // contract: the backend preserves the identifier instead of minting one.
        assert!(recovered.event_id.is_some());
        assert!(recovered.d_tag.is_some());

        // Later progress from the retry cannot change the identifier.
        let after_retry = publication_identity_progress(
            recovered.clone(),
            &PublishProgressPayload {
                step: "listing-coordinate".into(),
                status: "ok".into(),
                server_url: None,
                message: Some("30402:publisherhex:2f9a1c34-5b6d-4e7f-8a9b-0c1d2e3f4a5b".into()),
                bytes_uploaded: None,
                total_bytes: None,
                ..Default::default()
            },
        );
        assert_eq!(after_retry, recovered);
    }

    fn progress(step: &str, message: &str) -> PublishProgressPayload {
        PublishProgressPayload {
            step: step.into(),
            status: "ok".into(),
            server_url: None,
            message: Some(message.into()),
            bytes_uploaded: None,
            total_bytes: None,
            ..Default::default()
        }
    }

    #[test]
    fn publication_progress_isolated_by_operation_account_and_coordinate() {
        let mut coordinate = None;
        let mut event = progress("pending-coordinate", "coordinate-a");
        event.operation_id = "operation-a".into();
        event.publisher_npub = "alice".into();
        event.game_coordinate = "coordinate-a".into();
        // Same-account concurrent publication cannot bind or overwrite identity.
        assert!(!accepts_publication_progress(
            "operation-b",
            "alice",
            &mut coordinate,
            &event
        ));
        assert!(coordinate.is_none());
        assert!(!accepts_publication_progress(
            "operation-a",
            "bob",
            &mut coordinate,
            &event
        ));
        assert!(accepts_publication_progress(
            "operation-a",
            "alice",
            &mut coordinate,
            &event
        ));
        event.game_coordinate = "coordinate-b".into();
        assert!(!accepts_publication_progress(
            "operation-a",
            "alice",
            &mut coordinate,
            &event
        ));
        assert_eq!(coordinate.as_deref(), Some("coordinate-a"));
        // A late event from a previous attempt is ignored by the replacement attempt.
        assert!(!accepts_publication_progress(
            "retry-operation",
            "alice",
            &mut coordinate,
            &event
        ));
    }

    #[test]
    fn a_failure_before_listing_publication_keeps_the_minted_identity_for_the_retry() {
        // Provisioning failed: no listing exists, but a coordinate was minted
        // and a fulfillment key was requested for it.
        let identity = publication_identity_progress(
            PublishedIdentity::default(),
            &progress(
                "pending-coordinate",
                "30402:publisherhex:2f9a1c34-5b6d-4e7f-8a9b-0c1d2e3f4a5b",
            ),
        );

        assert_eq!(
            identity.pending_d_tag.as_deref(),
            Some("2f9a1c34-5b6d-4e7f-8a9b-0c1d2e3f4a5b")
        );
        // Nothing was published, so the retry is still a first publication.
        assert_eq!(identity.event_id, None);
        assert_eq!(identity.d_tag, None);
    }

    #[test]
    fn publishing_the_listing_settles_the_pending_identity() {
        let identity = publication_identity_progress(
            PublishedIdentity::default(),
            &progress(
                "pending-coordinate",
                "30402:publisherhex:2f9a1c34-5b6d-4e7f-8a9b-0c1d2e3f4a5b",
            ),
        );
        let identity = publication_identity_progress(
            identity,
            &progress("publish-listing", "listing-event-id"),
        );
        let identity = publication_identity_progress(
            identity,
            &progress(
                "listing-coordinate",
                "30402:publisherhex:2f9a1c34-5b6d-4e7f-8a9b-0c1d2e3f4a5b",
            ),
        );

        assert_eq!(
            identity.d_tag.as_deref(),
            Some("2f9a1c34-5b6d-4e7f-8a9b-0c1d2e3f4a5b")
        );
        assert_eq!(
            identity.pending_d_tag, None,
            "a published listing is edited by event id, not resumed"
        );
    }

    #[test]
    fn a_published_identity_is_never_downgraded_to_a_pending_one() {
        let published = PublishedIdentity {
            event_id: Some("listing-event-id".into()),
            d_tag: Some("2f9a1c34-5b6d-4e7f-8a9b-0c1d2e3f4a5b".into()),
            coordinate: Some("30402:publisherhex:2f9a1c34-5b6d-4e7f-8a9b-0c1d2e3f4a5b".into()),
            pending_d_tag: None,
        };

        // The retry of a partial publication emits its identity too; it must
        // not reopen the pending channel for a listing that already exists.
        let after_retry = publication_identity_progress(
            published.clone(),
            &progress(
                "pending-coordinate",
                "30402:publisherhex:2f9a1c34-5b6d-4e7f-8a9b-0c1d2e3f4a5b",
            ),
        );

        assert_eq!(after_retry, published);
    }

    #[test]
    fn identity_handoffs_are_never_shown_as_publication_stages() {
        assert!(is_recovery_channel("listing-coordinate"));
        assert!(is_recovery_channel("pending-coordinate"));
        for step in [
            "hash-file",
            "check-operator",
            "provision",
            "publish-listing",
            "confirm-propagation",
            "upload",
        ] {
            assert!(
                !is_recovery_channel(step),
                "{step} is a stage the publisher is waiting on"
            );
            assert_ne!(
                progress_label(&progress(step, "detail")),
                progress_label(&progress("unmapped-step", "detail")),
                "{step} should have its own label"
            );
        }
    }

    #[test]
    fn a_malformed_pending_coordinate_leaves_the_identity_untouched() {
        let identity = publication_identity_progress(
            PublishedIdentity::default(),
            &progress("pending-coordinate", "not-a-coordinate"),
        );

        assert_eq!(identity, PublishedIdentity::default());
    }

    #[test]
    fn a_published_store_page_becomes_the_revision_a_retry_edits() {
        let page = StorePageDraft::new(String::new(), Vec::new());
        let coordinate = "30402:publisherhex:game-id".to_string();
        let progress = |step: &str, status: &str| PublishProgressPayload {
            step: step.into(),
            status: status.into(),
            message: Some("page-event".into()),
            game_coordinate: coordinate.clone(),
            ..Default::default()
        };
        assert!(
            store_page_after_progress(&page, &progress("publish-store-page", "pending")).is_none()
        );
        assert!(store_page_after_progress(&page, &progress("publish-listing", "ok")).is_none());

        let adopted = store_page_after_progress(&page, &progress("publish-store-page", "ok"))
            .expect("published page adopted");
        assert_eq!(adopted.presentation_id, "game-id");
        assert_eq!(adopted.listing_coordinates, vec![coordinate.clone()]);
        assert_eq!(adopted.loaded_event_id.as_deref(), Some("page-event"));

        // An existing (possibly shared) page keeps its own identity.
        let mut existing = StorePageDraft::new("shared".into(), vec!["30402:p:other".into()]);
        existing.loaded_event_id = Some("old".into());
        let next = store_page_after_progress(&existing, &progress("publish-store-page", "ok"))
            .expect("revision advanced");
        assert_eq!(next.presentation_id, "shared");
        assert_eq!(next.listing_coordinates, vec!["30402:p:other".to_string()]);
        assert_eq!(next.loaded_event_id.as_deref(), Some("page-event"));
    }

    #[test]
    fn the_bundled_store_page_follows_the_game_details_and_is_checked_locally() {
        let publisher = nostr::Keys::generate().public_key().to_hex();
        let mut page = StorePageDraft::new(String::new(), Vec::new());
        page.content.links.support = Some("https://example.com/help".into());
        let synced = synced_store_page(page, " Neon ", "First.\n\nSecond.", "Racing, Arcade");
        assert_eq!(synced.content.basic.title.as_deref(), Some("Neon"));
        assert_eq!(
            synced.content.discovery.genres,
            Some(vec!["Racing".to_string(), "Arcade".to_string()])
        );
        assert!(store_page_problems(&synced, &publisher).is_empty());

        let mut broken = synced;
        broken.content.links.support = Some("not a url".into());
        let problems = store_page_problems(&broken, &publisher);
        assert!(
            problems.iter().any(|problem| problem.contains("Support")),
            "{problems:?}"
        );
    }

    #[test]
    fn recovery_updates_identity_without_resetting_unsaved_form_values() {
        let mut form = FormSnapshot {
            title: "Renamed after the failure".into(),
            price: "9000".into(),
            platforms: "linux-x86_64".into(),
            acquisition: "Public".into(),
            ..FormSnapshot::default()
        };
        let untouched = form.clone();
        let identity = publication_identity_progress(
            PublishedIdentity::default(),
            &PublishProgressPayload {
                step: "listing-coordinate".into(),
                status: "ok".into(),
                server_url: None,
                message: Some("30402:publisherhex:2f9a1c34-5b6d-4e7f-8a9b-0c1d2e3f4a5b".into()),
                bytes_uploaded: None,
                total_bytes: None,
                ..Default::default()
            },
        );

        // Recovery only ever touches identity: it takes the form snapshot
        // neither as input nor as output, so no unsaved edit can be reset by it.
        assert_eq!(form, untouched);
        assert_eq!(
            identity.coordinate.as_deref(),
            Some("30402:publisherhex:2f9a1c34-5b6d-4e7f-8a9b-0c1d2e3f4a5b")
        );
        assert_eq!(
            identity.d_tag.as_deref(),
            Some("2f9a1c34-5b6d-4e7f-8a9b-0c1d2e3f4a5b")
        );

        // The form remains free to change afterwards; the retry sends the later
        // value, not the half-published one.
        form.price = "12000".into();
        assert_ne!(form, untouched);
    }

    #[test]
    fn legacy_and_uuid_coordinates_both_yield_their_identifier() {
        assert_eq!(
            d_tag_from_coordinate("30402:publisherhex:my-game-v1").as_deref(),
            Some("my-game-v1")
        );
        assert_eq!(
            d_tag_from_coordinate("30402:publisherhex:2f9a1c34-5b6d-4e7f-8a9b-0c1d2e3f4a5b")
                .as_deref(),
            Some("2f9a1c34-5b6d-4e7f-8a9b-0c1d2e3f4a5b")
        );
        assert_eq!(
            d_tag_from_coordinate("30402:publisherhex:legacy:with:colons").as_deref(),
            Some("legacy:with:colons")
        );
        assert_eq!(d_tag_from_coordinate("30023:publisherhex:article"), None);
        assert_eq!(d_tag_from_coordinate("30402:publisherhex:"), None);
        assert_eq!(d_tag_from_coordinate("not-a-coordinate"), None);
    }

    #[test]
    fn signer_requirements_distinguish_every_publisher_signer_state() {
        let messages = [
            PublisherSignerState::Available,
            PublisherSignerState::Connecting,
            PublisherSignerState::Unavailable,
            PublisherSignerState::Unknown,
        ]
        .map(signer_requirement_message);
        let mut unique = messages.to_vec();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(unique.len(), 4);
        assert!(signer_can_publish(PublisherSignerState::Available));
        assert!(!signer_can_publish(PublisherSignerState::Connecting));
        assert!(!signer_can_publish(PublisherSignerState::Unavailable));
        assert!(!signer_can_publish(PublisherSignerState::Unknown));
    }

    #[test]
    fn unsupported_prototype_controls_are_absent_from_the_workflow() {
        let source = include_str!("publish.rs");
        for absent in [
            ["Simulate", " publish failure"].concat(),
            ["Release", " notes"].concat(),
            ["Short", " summary"].concat(),
            ["Live", " on the store now"].concat(),
            ["Step 1 of ", "5"].concat(),
            ["Continue ", ">\""].concat(),
            ["Not saved ", "anywhere"].concat(),
            // Leaving a section is navigation, not completion.
            ["\"Do", "ne\""].concat(),
        ] {
            assert!(!source.contains(&absent), "unsupported control rendered");
        }
    }
}

#[cfg(test)]
mod buyer_preview_tests {
    use super::*;

    const PUBLISHER_HEX: &str = "79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798";

    fn page(description: &str) -> StorePageDraft {
        synced_store_page(
            StorePageDraft::new(String::new(), Vec::new()),
            "Neon Drifter",
            "A racing game.",
            "Racing",
        )
        .tap(|page| page.content.description_markdown = description.into())
    }

    trait Tap: Sized {
        fn tap(mut self, edit: impl FnOnce(&mut Self)) -> Self {
            edit(&mut self);
            self
        }
    }
    impl Tap for StorePageDraft {}

    #[test]
    fn the_preview_shows_the_basics_and_sanitized_description() {
        let preview = buyer_preview_page(
            &page("**Fast** cars <script>alert(1)</script>"),
            PUBLISHER_HEX,
            &[],
        )
        .expect("previewable page");
        assert_eq!(preview.title.as_deref(), Some("Neon Drifter"));
        assert_eq!(preview.summary.as_deref(), Some("A racing game."));
        let html = preview.description_html.expect("description");
        assert!(html.as_str().contains("<strong>Fast</strong>"));
        assert!(!html.as_str().to_ascii_lowercase().contains("<script"));
    }

    #[test]
    fn requirements_only_show_for_platforms_the_game_declares() {
        let mut draft = page("");
        for platform in ["linux-x86_64", "windows-x86_64"] {
            draft
                .content
                .requirements
                .entry(platform.into())
                .or_default()
                .minimum = Some(arcadestr_core::store_page::RequirementTier {
                os: Some(platform.into()),
                ..Default::default()
            });
        }
        let preview = buyer_preview_page(&draft, PUBLISHER_HEX, &["linux-x86_64".to_string()])
            .expect("previewable page");
        assert_eq!(preview.requirements.len(), 1);
        assert_eq!(preview.requirements[0].platform, "linux-x86_64");
    }

    #[test]
    fn the_card_uses_the_published_price_and_store_page_images() {
        use nostr::nips::nip19::ToBech32;
        let npub = nostr::PublicKey::from_hex(PUBLISHER_HEX)
            .expect("key")
            .to_bech32()
            .expect("npub");
        let mut draft = page("");
        draft
            .content
            .media
            .push(arcadestr_core::store_page::StorePageMediaItem {
                id: "capsule".into(),
                media_type: "image".into(),
                role: "capsule".into(),
                url: "https://media.test/capsule.png".into(),
                sha256: None,
                mime_type: None,
                size: None,
                thumbnail_url: None,
                alt: None,
                caption: None,
                width: None,
                height: None,
            });
        let preview = buyer_preview_page(&draft, PUBLISHER_HEX, &[]).expect("page");
        let (listing, card) = buyer_preview_card(
            &npub,
            " Neon Drifter ",
            "A racing game.",
            2100,
            &AcquisitionPolicy::Gated,
            &preview,
            vec!["https://media.test/capsule.png".into()],
            &["linux-x86_64".to_string()],
        )
        .expect("card");
        assert_eq!(listing.title, "Neon Drifter");
        assert_eq!(listing.price_sats, 2100);
        assert_eq!(listing.acquisition, AcquisitionPolicy::Gated);
        assert_eq!(
            card.capsule_url.as_deref(),
            Some("https://media.test/capsule.png")
        );
    }

    #[test]
    fn a_store_page_that_would_be_rejected_has_no_preview() {
        let mut draft = page("");
        draft.content.links.website = Some("http://not-https.test".into());
        assert!(buyer_preview_page(&draft, PUBLISHER_HEX, &[]).is_err());
    }

    #[test]
    fn publishing_always_goes_through_the_preview() {
        let source = include_str!("publish.rs");
        let view = source
            .split(concat!("id=\"publish-buyer-", "preview\""))
            .next()
            .expect("view before the preview dialog");
        // The header's Publish button opens the preview; only the preview's
        // confirm action runs the submit.
        assert!(!view.contains(concat!("on:click=move |_| on_", "submit.run(())")));
        assert!(source.contains("UpdateScope::FullGame => on_submit.run(())"));
        assert!(source.contains("UpdateScope::StorePageOnly => publish_store_page_only.run(())"));
    }
}

/// The request the backend signs, built from the editor's values after each
/// delivery choice, a restored draft, and stale leftover values.
#[cfg(test)]
mod publish_request_tests {
    use super::*;
    use arcadestr_core::adp_settlement::PublisherCommercialIntent;
    use arcadestr_core::archive_inspection_contract::ArchiveSelectionId;
    use arcadestr_core::distribution::{RelativePath, Sha256Digest, WorkingDir};

    const OLD_BUILD: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
    const NEW_BUILD: &str = "fedcba9876543210fedcba9876543210fedcba9876543210fedcba9876543210";
    const SERVICE: &str = "https://service.test";

    fn publisher() -> String {
        use nostr::nips::nip19::ToBech32;
        nostr::PublicKey::from_hex(
            "79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798",
        )
        .expect("fixture key")
        .to_bech32()
        .expect("fixture npub")
    }

    fn server(url: &str) -> ServerEntry {
        ServerEntry {
            url: url.into(),
            label: url.into(),
            reachability: ServerStatus::Ok,
            upload: ServerStatus::Idle,
            auto_operator: false,
        }
    }

    /// A new game with only the basics filled in and no delivery chosen yet.
    fn new_game() -> PublishFormInput {
        let npub = publisher();
        PublishFormInput {
            initiating_npub: npub.clone(),
            title: "  Neon Drifter ".into(),
            description: "A racing game.".into(),
            tags: "Racing, Arcade".into(),
            lud16: String::new(),
            acquisition: AcquisitionKind::Public,
            price: String::new(),
            starts_at: String::new(),
            ends_at: String::new(),
            fulfillment_enabled: false,
            fulfillment_mode: FulfillmentMode::None,
            servers: Vec::new(),
            operator_url: String::new(),
            server_roles: Vec::new(),
            authorize_operators: false,
            file_hash: None,
            version: String::new(),
            platforms: "linux-x86_64".into(),
            publish_state: PublishState {
                archive: ArchiveSelectionState::new(npub),
                launch: LaunchMode::DownloadOnly,
            },
            store_page_loaded: true,
            store_page: StorePageDraft::new(String::new(), Vec::new()),
            commercial_mode: "preserve".into(),
            commercial_review: None,
            commercial_approved: false,
            acknowledged_removed_profile_ids: Vec::new(),
            identity: PublishedIdentity::default(),
            existing_fulfillment_pubkey: None,
            existing_campaigns: Vec::new(),
            existing_nip94_event_id: None,
            authoritative_file_hash: None,
        }
    }

    /// A live game delivered through a service, with a published build and promotion.
    fn live_game() -> PublishFormInput {
        let mut input = new_game();
        input.fulfillment_enabled = true;
        input.fulfillment_mode = FulfillmentMode::Delegate;
        input.servers = vec![server(SERVICE)];
        input.operator_url = SERVICE.into();
        input.file_hash = Some(OLD_BUILD.into());
        input.authoritative_file_hash = Some(OLD_BUILD.into());
        input.version = "1.0".into();
        input.existing_fulfillment_pubkey = Some("22".repeat(32));
        input.existing_campaigns = vec![CampaignPointerInput {
            root_event_id: "33".repeat(32),
            relay_hint: None,
        }];
        input.existing_nip94_event_id = Some("44".repeat(32));
        input.identity = PublishedIdentity {
            event_id: Some("11".repeat(32)),
            d_tag: Some("neon-drifter".into()),
            coordinate: None,
            pending_d_tag: None,
        };
        input
    }

    fn choose(input: &mut PublishFormInput, choice: DeliveryChoice) {
        apply_delivery_choice(
            choice,
            &mut input.fulfillment_enabled,
            &mut input.fulfillment_mode,
            &mut input.publish_state,
            &input.platforms.clone(),
        );
    }

    /// Launch settings that are complete, using paths already published.
    fn complete_launch(input: &mut PublishFormInput) {
        let LaunchMode::Launchable(tier) = &mut input.publish_state.launch else {
            panic!("launch options expected");
        };
        tier.package_format.text = "zip".into();
        let row = tier.add_profile();
        let profile = tier
            .profiles
            .iter_mut()
            .find(|profile| profile.row_id == row)
            .expect("added profile");
        profile.profile_id = "default".into();
        profile.display_name = "Play".into();
        profile.executable.authoritative_value =
            Some(RelativePath::parse("bin/game").expect("executable"));
        profile.working_dir.authoritative_value = Some(WorkingDir::parse("bin").expect("dir"));
        assert!(tier.set_publisher_default_profile(row, true));
    }

    fn select_new_build(input: &mut PublishFormInput) {
        input.publish_state.archive.selected_build = Some(BuildSelectionRef {
            selection_id: ArchiveSelectionId::parse("2f9a1c34-5b6d-4e7f-8a9b-0c1d2e3f4a5b")
                .expect("selection"),
            sha256: Sha256Digest::parse(NEW_BUILD).expect("digest"),
        });
        input.file_hash = Some(NEW_BUILD.into());
    }

    fn built(input: PublishFormInput) -> PreparedPublish {
        build_publish_request(input).expect("publishable input")
    }

    #[test]
    fn store_page_only_sends_no_build_or_distribution_even_with_leftover_values() {
        // A live game whose author switches to store page only: every build and
        // distribution value left in the form must stay out of the request.
        let mut input = live_game();
        choose(&mut input, DeliveryChoice::StorePageOnly);
        assert!(!input.fulfillment_enabled);
        assert_eq!(input.fulfillment_mode, FulfillmentMode::None);
        let PreparedPublish { request, extension } = built(input);
        assert_eq!(request.fulfillment_mode, FulfillmentMode::None);
        assert!(request.servers.is_empty());
        assert_eq!(request.operator_url, None);
        assert_eq!(request.version, None);
        assert_eq!(request.selected_build, None);
        assert_eq!(request.existing_file_hash, None);
        assert_eq!(request.existing_fulfillment_pubkey, None);
        assert_eq!(extension.launch_tier, None);
        // Everything unrelated to delivery is kept.
        assert_eq!(request.title, "Neon Drifter");
        assert_eq!(request.campaigns.len(), 1);
        assert_eq!(request.existing_d_tag.as_deref(), Some("neon-drifter"));
    }

    #[test]
    fn download_only_reuses_the_published_build_without_launch_settings() {
        let mut input = live_game();
        choose(&mut input, DeliveryChoice::DownloadOnly);
        let PreparedPublish { request, extension } = built(input);
        assert_eq!(request.fulfillment_mode, FulfillmentMode::Delegate);
        assert_eq!(request.servers, vec![SERVICE.to_string()]);
        assert_eq!(request.operator_url.as_deref(), Some(SERVICE));
        assert_eq!(request.version.as_deref(), Some("1.0"));
        assert_eq!(request.selected_build, None);
        assert_eq!(request.existing_file_hash.as_deref(), Some(OLD_BUILD));
        assert_eq!(request.existing_fulfillment_pubkey, Some("22".repeat(32)));
        assert_eq!(extension.launch_tier, None);
    }

    #[test]
    fn choosing_a_build_option_on_a_new_game_turns_on_a_distribution_service() {
        for choice in [DeliveryChoice::Install, DeliveryChoice::DownloadOnly] {
            let mut input = new_game();
            choose(&mut input, choice);
            assert!(input.fulfillment_enabled);
            assert_eq!(input.fulfillment_mode, FulfillmentMode::Delegate);
            assert_eq!(
                matches!(input.publish_state.launch, LaunchMode::Launchable(_)),
                choice == DeliveryChoice::Install
            );
        }
        // Switching back keeps the author's signing choice.
        let mut input = live_game();
        input.fulfillment_mode = FulfillmentMode::Direct;
        choose(&mut input, DeliveryChoice::Install);
        assert_eq!(input.fulfillment_mode, FulfillmentMode::Direct);
    }

    #[test]
    fn a_build_option_without_a_build_file_is_refused() {
        for choice in [DeliveryChoice::Install, DeliveryChoice::DownloadOnly] {
            let mut input = new_game();
            choose(&mut input, choice);
            input.servers = vec![server(SERVICE)];
            input.operator_url = SERVICE.into();
            input.version = "1.0".into();
            assert!(
                matches!(build_publish_request(input), Err(PublishBlocked::Message(message)) if message.contains("build file")),
                "{choice:?}"
            );
        }
    }

    #[test]
    fn incomplete_launch_settings_are_refused_and_recorded_for_retry() {
        let mut input = live_game();
        choose(&mut input, DeliveryChoice::Install);
        assert!(matches!(
            build_publish_request(input),
            Err(PublishBlocked::Attempt(
                PublishAdpListingError::CoreValidation { .. }
            ))
        ));
    }

    #[test]
    fn install_publishes_complete_launch_settings_with_the_build() {
        let mut input = live_game();
        choose(&mut input, DeliveryChoice::Install);
        complete_launch(&mut input);
        let PreparedPublish { request, extension } = built(input);
        let tier = extension.launch_tier.expect("launch settings published");
        assert_eq!(tier.launch_profiles[0].id.as_str(), "default");
        assert_eq!(tier.launch_profiles[0].executable.as_str(), "bin/game");
        assert_eq!(request.existing_file_hash.as_deref(), Some(OLD_BUILD));
        assert_eq!(request.fulfillment_mode, FulfillmentMode::Delegate);
    }

    #[test]
    fn a_newly_selected_build_replaces_the_published_one() {
        let mut input = live_game();
        choose(&mut input, DeliveryChoice::DownloadOnly);
        select_new_build(&mut input);
        let PreparedPublish { request, .. } = built(input);
        assert_eq!(
            request
                .selected_build
                .as_ref()
                .map(|build| build.sha256.as_str()),
            Some(NEW_BUILD)
        );
        assert_eq!(request.existing_file_hash, None);
        assert_eq!(request.file_path, None);
    }

    #[test]
    fn a_restored_draft_with_a_new_build_must_reselect_the_file() {
        // Restoring keeps the new build's fingerprint but not the file itself
        // (`consume_success` drops the selection), so publishing must refuse
        // rather than publish the old build under the new fingerprint.
        let mut input = live_game();
        choose(&mut input, DeliveryChoice::DownloadOnly);
        input.file_hash = Some(NEW_BUILD.into());
        input.publish_state.archive.consume_success();
        assert!(matches!(
            build_publish_request(input),
            Err(PublishBlocked::Attempt(PublishAdpListingError::Archive {
                error: ArchiveSelectionError::SelectedBuildRequired
            }))
        ));
    }

    #[test]
    fn a_restored_draft_rebuilds_the_same_request() {
        // The autosaved draft is JSON; values that survive it must yield the
        // same request as the editor they were saved from.
        let mut input = live_game();
        choose(&mut input, DeliveryChoice::DownloadOnly);
        input.acquisition = AcquisitionKind::Gated;
        input.price = "2100".into();
        input.lud16 = "dev@example.com".into();
        let saved = DurableListingDraft {
            server_roles: input.server_roles.clone(),
            form: FormSnapshot {
                title: input.title.clone(),
                description: input.description.clone(),
                tags: input.tags.clone(),
                price: input.price.clone(),
                lud16: input.lud16.clone(),
                platforms: input.platforms.clone(),
                version: input.version.clone(),
                operator_url: input.operator_url.clone(),
                acquisition: format!("{:?}", input.acquisition),
                starts_at: input.starts_at.clone(),
                ends_at: input.ends_at.clone(),
                fulfillment_enabled: input.fulfillment_enabled,
                commercial: String::new(),
            },
            identity: input.identity.clone(),
            launch: input.publish_state.launch.clone(),
            servers: input
                .servers
                .iter()
                .map(|server| server.url.clone())
                .collect(),
            mode: input.fulfillment_mode,
            acquisition: input.acquisition,
            artifact_hash: input.file_hash.clone(),
            commercial_intent: input.commercial_mode.clone(),
            commercial_review: None,
            accepted_terms: Vec::new(),
            authorization_references: Vec::new(),
            store_page: Some(input.store_page.clone()),
            delivery_chosen: true,
        };
        let json = serde_json::to_value(&saved).expect("encode draft");
        let restored: DurableListingDraft = serde_json::from_value(json).expect("decode draft");

        let mut from_draft = new_game();
        from_draft.title = restored.form.title;
        from_draft.description = restored.form.description;
        from_draft.tags = restored.form.tags;
        from_draft.price = restored.form.price;
        from_draft.lud16 = restored.form.lud16;
        from_draft.platforms = restored.form.platforms;
        from_draft.version = restored.form.version;
        from_draft.operator_url = restored.form.operator_url;
        from_draft.fulfillment_enabled = restored.form.fulfillment_enabled;
        from_draft.acquisition = restored.acquisition;
        from_draft.fulfillment_mode = restored.mode;
        from_draft.servers = restored.servers.iter().map(|url| server(url)).collect();
        from_draft.identity = restored.identity;
        from_draft.publish_state.launch = restored.launch;
        from_draft.file_hash = restored.artifact_hash;
        from_draft.store_page = restored.store_page.expect("store page saved");
        from_draft.commercial_mode = restored.commercial_intent;
        // Values that come from the published game, not the draft.
        from_draft.authoritative_file_hash = input.authoritative_file_hash.clone();
        from_draft.existing_fulfillment_pubkey = input.existing_fulfillment_pubkey.clone();
        from_draft.existing_campaigns = input.existing_campaigns.clone();
        from_draft.existing_nip94_event_id = input.existing_nip94_event_id.clone();

        assert_eq!(built(from_draft), built(input));
    }

    #[test]
    fn price_access_and_fee_terms_reach_the_request_exactly() {
        let mut paid = live_game();
        paid.acquisition = AcquisitionKind::Gated;
        paid.price = "2100".into();
        paid.lud16 = " dev@example.com ".into();
        let PreparedPublish { request, extension } = built(paid);
        assert_eq!(request.price_sats, 2100);
        assert_eq!(request.lud16.as_deref(), Some("dev@example.com"));
        assert!(matches!(
            extension.commercial_intent,
            PublisherCommercialIntent::Preserve
        ));

        // A free game never sends a price or payout address left in the form.
        let mut free = live_game();
        free.price = "2100".into();
        free.lud16 = "dev@example.com".into();
        let PreparedPublish { request, .. } = built(free);
        assert_eq!(request.price_sats, 0);
        assert_eq!(request.lud16, None);

        // New fee terms without an approved review are never sent.
        let mut unreviewed = live_game();
        unreviewed.commercial_mode = "replace".into();
        assert!(matches!(
            build_publish_request(unreviewed),
            Err(PublishBlocked::Message(_))
        ));
    }

    #[test]
    fn service_permissions_default_to_the_chosen_service_only() {
        let mut input = live_game();
        input.servers.push(server("https://mirror.test"));
        let PreparedPublish { extension, .. } = built(input);
        let roles = extension.server_roles;
        assert_eq!(roles.len(), 2);
        assert!(roles[0].issue_receipt && roles[0].issue_grant);
        assert!(!roles[1].issue_receipt && !roles[1].issue_grant);
    }

    #[test]
    fn nothing_is_sent_until_the_store_page_has_loaded() {
        let mut input = live_game();
        input.store_page_loaded = false;
        assert!(matches!(
            build_publish_request(input),
            Err(PublishBlocked::Message(message)) if message.contains("store page to load")
        ));
    }
}

/// Game hub: the overview checklist of a draft or live game plus its
/// independently editable sections. Publishes the listing, Store page, build
/// and distribution authorization through one pipeline.
#[component]
pub fn PublishView(
    #[prop(optional)] listing: Option<GameListing>,
    /// Local draft of a game that has not been published yet.
    #[prop(optional)]
    draft_id: Option<String>,
    #[prop(optional)] initial_section: Option<HubSection>,
    /// Extra overview content for a live game (promotions, access management).
    #[prop(optional)]
    overview_extra: Option<ViewFn>,
    #[prop(optional)] on_published: Option<Callback<GameListing>>,
    #[prop(optional)] on_back: Option<Callback<()>>,
) -> impl IntoView {
    let auth = use_context::<AuthContext>().expect("AuthContext not provided");
    let editing = listing.is_some();
    let restoring_draft = draft_id.is_some();
    let editing_publisher = listing.as_ref().map(|item| item.publisher_npub.clone());
    // Identity of the game page under edit. Both stay `None` for a first
    // publication until the backend reports what it minted, after which a retry
    // replaces that listing instead of creating a second coordinate.
    let published_identity = RwSignal::new(PublishedIdentity {
        event_id: listing.as_ref().and_then(|item| item.event_id.clone()),
        d_tag: listing.as_ref().map(|item| item.id.clone()),
        coordinate: listing
            .as_ref()
            .map(|item| format!("30402:{}:{}", item.publisher_npub, item.id)),
        pending_d_tag: None,
    });
    let existing_listing_for_submit = listing.clone();
    let published_servers = listing.as_ref().map(listing_servers).unwrap_or_default();
    let published_fulfillment_mode = listing
        .as_ref()
        .map(listing_fulfillment_mode)
        .unwrap_or(FulfillmentMode::None);
    let has_published_fulfillment = !matches!(published_fulfillment_mode, FulfillmentMode::None);
    let editing_delegated_listing =
        editing && matches!(published_fulfillment_mode, FulfillmentMode::Delegate);
    let use_default_provider = !has_published_fulfillment;
    let existing_file_hash = listing.as_ref().and_then(captured_file_hash);
    let authoritative_file_hash = existing_file_hash.clone();
    let authoritative_file_hash_for_account = authoritative_file_hash.clone();
    let authoritative_file_hash_for_submit = authoritative_file_hash.clone();
    let authoritative_file_hash_for_restore = authoritative_file_hash.clone();
    let existing_fulfillment_pubkey = listing.as_ref().and_then(listing_fulfillment_pubkey);
    let operator_resolution = listing.as_ref().and_then(operator_resolution_request);
    let initial_acquisition = listing
        .as_ref()
        .map(|item| item.acquisition.clone())
        .unwrap_or_default();
    let existing_campaigns = listing
        .as_ref()
        .map(|item| {
            item.campaigns
                .iter()
                .map(|pointer| CampaignPointerInput {
                    root_event_id: pointer.root_event_id.clone(),
                    relay_hint: pointer.relay_hint.clone(),
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let existing_nip94_event_id = listing
        .as_ref()
        .and_then(|item| item.nip94_event_id.clone());
    let existing_fulfillment_locked = existing_fulfillment_pubkey.is_some();

    let draft_revision = RwSignal::new(None::<u64>);
    let draft_message = RwSignal::new(None::<String>);
    let draft_busy = RwSignal::new(false);
    let restored_artifact_hash = RwSignal::new(None::<String>);
    let durable_draft_id = StoredValue::new(game_draft_id(
        listing.as_ref().map(|listing| listing.id.as_str()),
        draft_id.as_deref(),
    ));
    let draft_save_status = RwSignal::new(DraftSaveStatus::Idle);
    // Reloads this game's unfinished publishes after each attempt.
    let recovery_refresh = RwSignal::new(0_u64);
    let last_saved_fingerprint = RwSignal::new(None::<String>);
    let autosave_tick = RwSignal::new(0_u64);
    // Autosave waits until any saved draft has been restored, so an empty form
    // never overwrites it.
    let draft_ready = RwSignal::new(false);
    let durable_draft_owner = StoredValue::new(
        listing
            .as_ref()
            .map(|listing| listing.publisher_npub.clone())
            .or_else(|| auth.npub.get_untracked()),
    );
    let durable_accepted_terms = StoredValue::new(
        listing
            .as_ref()
            .map(|listing| {
                listing
                    .specs
                    .iter()
                    .filter(|(key, _)| key.starts_with("adp_terms:"))
                    .cloned()
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default(),
    );
    let durable_authorizations = StoredValue::new(
        listing
            .as_ref()
            .map(|listing| {
                listing
                    .specs
                    .iter()
                    .filter(|(key, _)| key == "fulfillment_authorization")
                    .cloned()
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default(),
    );
    let draft_epoch_auth = auth.clone();
    Effect::new(move |_| {
        draft_epoch_auth.current_auth_flow();
        draft_busy.set(false);
        draft_revision.set(None);
        draft_message.set(None);
    });
    let title = RwSignal::new(
        listing
            .as_ref()
            .map(|item| item.title.clone())
            .unwrap_or_default(),
    );
    let description = RwSignal::new(
        listing
            .as_ref()
            .map(|item| item.description.clone())
            .unwrap_or_default(),
    );
    // The Store Page edited with the game. Its media is the form's only image
    // editor: the listing images are derived from it at publication, and the
    // page itself is published in the same operation.
    let store_page_draft = RwSignal::new(StorePageDraft::new(String::new(), Vec::new()));
    let store_page_baseline = RwSignal::new(store_page_draft.get_untracked());
    // `None` while an existing game's page loads; publishing waits for it so
    // the listing images are never derived from an empty page.
    let store_page_status = RwSignal::new((!editing).then_some(Ok::<(), String>(())));
    let store_page_note = RwSignal::new(None::<String>);
    let store_page_input_dirty = RwSignal::new(false);
    let store_page_tab = RwSignal::new("description");
    let store_page_diagnostics = RwSignal::new(Vec::<String>::new());
    let store_page_form_error = RwSignal::new(None::<String>);
    let store_page_upload_generation = RwSignal::new(0_u64);
    // The store page is published and the game already points to it, so a
    // store-page-only edit can be published without touching the game.
    let store_page_linked = RwSignal::new(false);
    let load_store_page = {
        let target = listing.as_ref().and_then(|item| {
            item.event_id.clone().map(|event_id| {
                (
                    item.publisher_npub.clone(),
                    crate::campaign_management::listing_coordinate(item),
                    event_id,
                )
            })
        });
        Callback::new(move |_: ()| {
            let Some((publisher, coordinate, event_id)) = target.clone() else {
                return;
            };
            store_page_status.set(None);
            spawn_local(async move {
                let result = invoke_load_publisher_store_page_editor(
                    publisher,
                    StorePageListingRef {
                        listing_coordinate: coordinate.clone(),
                        listing_event_id: event_id,
                    },
                    None,
                )
                .await;
                match result {
                    Ok(state) => {
                        let linked = state.draft.loaded_event_id.is_some()
                            && state.listings.iter().any(|listing| {
                                listing.reciprocal && listing.listing_coordinate == coordinate
                            });
                        let _ = store_page_linked.try_set(linked);
                        let note = state.started_from_listing.map(|started| {
                            let mut note = "This game had no store page yet, so one was started from its details and images. It's published with the game.".to_string();
                            if !started.skipped_images.is_empty() {
                                note.push_str(&format!(
                                    " {} image(s) were left out because the store page can't use them.",
                                    started.skipped_images.len()
                                ));
                            }
                            note
                        });
                        let _ = store_page_note.try_set(note);
                        let _ = store_page_draft.try_set(state.draft);
                        let _ = store_page_baseline.try_set(state.baseline_draft);
                        let _ = store_page_status.try_set(Some(Ok(())));
                    }
                    Err(error) => {
                        let _ = store_page_status.try_set(Some(Err(error)));
                    }
                }
            });
        })
    };
    if let Some(item) = listing.as_ref().filter(|item| item.event_id.is_none()) {
        // Nothing to load against: start from the listing as the backend would.
        let (draft, _) = arcadestr_core::store_page_seed::draft_from_listing(
            item.id.clone(),
            crate::campaign_management::listing_coordinate(item),
            arcadestr_core::store_page_seed::ListingPresentation {
                title: &item.title,
                description: &item.description,
                tags: &item.tags,
                images: &item.images,
            },
        );
        store_page_draft.set(draft.clone());
        store_page_baseline.set(draft);
        store_page_status.set(Some(Ok(())));
    } else {
        load_store_page.run(());
    }
    // Problems reported by the last local check are stale after any edit.
    Effect::new(move |_| {
        store_page_draft.track();
        store_page_diagnostics.set(Vec::new());
    });
    let tag_input = RwSignal::new(
        listing
            .as_ref()
            .map(|item| editable_listing_tags(&item.tags))
            .unwrap_or_default(),
    );
    let tag_choice = RwSignal::new(String::new());
    let initial_price = listing.as_ref().map(|item| item.price_sats);
    let price_input = RwSignal::new(
        initial_price
            .filter(|price| *price > 0)
            .map(|price| price.to_string())
            .unwrap_or_default(),
    );
    let platforms_input = RwSignal::new(
        listing
            .as_ref()
            .map(|item| item.platforms.join(", "))
            .unwrap_or_default(),
    );
    let lud16 = RwSignal::new(
        listing
            .as_ref()
            .map(|item| item.lud16.clone())
            .unwrap_or_default(),
    );
    let acquisition_kind = RwSignal::new(acquisition_kind(&initial_acquisition));
    let acquisition_starts_at = RwSignal::new(match &initial_acquisition {
        AcquisitionPolicy::TimedAccess { starts_at, .. } => datetime_local_value(*starts_at),
        _ => String::new(),
    });
    let acquisition_ends_at = RwSignal::new(match &initial_acquisition {
        AcquisitionPolicy::TimedAccess { ends_at, .. } => datetime_local_value(*ends_at),
        _ => String::new(),
    });
    let fulfillment_enabled = RwSignal::new(!editing || has_published_fulfillment);
    let fulfillment_mode = RwSignal::new(if editing {
        published_fulfillment_mode
    } else {
        FulfillmentMode::Delegate
    });
    // A new game has no delivery until the author picks one, so nothing silently
    // requires a build the author never asked for.
    let delivery_chosen = RwSignal::new(editing);
    let commercial_mode = RwSignal::new("preserve".to_string());
    let commercial_review = RwSignal::new(None::<PublisherTermsReview>);
    let commercial_approved = RwSignal::new(false);
    let commercial_loading = RwSignal::new(false);
    let commercial_error = RwSignal::new(None::<String>);
    let commercial_generation = RwSignal::new(0_u64);
    let preserved_terms_label = StoredValue::new(
        listing
            .as_ref()
            .map(|listing| {
                listing
                    .specs
                    .iter()
                    .filter(|(key, _)| key.starts_with("adp_terms:"))
                    .map(|(operator, root)| format!("{operator} → {root}"))
                    .collect::<Vec<_>>()
                    .join("; ")
            })
            .filter(|text| !text.is_empty())
            .unwrap_or_else(|| "None. No service fee terms have been accepted.".into()),
    );
    let discovered_servers = RwSignal::new(Vec::<AdpServerAnnouncement>::new());
    let discovery_error = RwSignal::new(None::<String>);
    let default_provider = configured_default_provider(DEFAULT_ADP_SERVER_URL);
    let initial_servers = if use_default_provider {
        default_provider
            .map(|url| ServerEntry {
                url: url.to_string(),
                label: "Arcadestr distribution service".into(),
                reachability: ServerStatus::Pending,
                upload: ServerStatus::Idle,
                auto_operator: true,
            })
            .into_iter()
            .collect()
    } else {
        published_servers
            .iter()
            .map(|url| ServerEntry {
                url: url.clone(),
                label: "Published server".into(),
                reachability: ServerStatus::Pending,
                upload: ServerStatus::Idle,
                auto_operator: false,
            })
            .collect::<Vec<_>>()
    };
    let initial_server_urls = initial_servers
        .iter()
        .map(|server| server.url.clone())
        .collect::<Vec<_>>();
    let servers = RwSignal::new(initial_servers);
    let custom_server = RwSignal::new(String::new());
    let initial_operator_url = initial_operator_url(editing_delegated_listing, default_provider);
    let operator_url = RwSignal::new(initial_operator_url.clone());
    let server_roles =
        RwSignal::new(Vec::<arcadestr_core::publisher_management::ServerRoleIntent>::new());
    let authorize_operators = RwSignal::new(false);
    let role_auth = auth.clone();
    Effect::new(move |_| {
        server_roles.track();
        servers.track();
        operator_url.track();
        role_auth.npub.track();
        authorize_operators.set(false);
    });
    let operator_auto_added = RwSignal::new(
        (use_default_provider && !initial_operator_url.is_empty()).then_some(initial_operator_url),
    );
    let file_hash = RwSignal::new(existing_file_hash);
    let version = RwSignal::new(
        listing
            .as_ref()
            .and_then(|item| listing_spec(item, "version"))
            .unwrap_or_default(),
    );

    // A brand-new game opens on its first fields; anything else opens the hub.
    let current_section = RwSignal::new(initial_section.unwrap_or(if editing || restoring_draft {
        HubSection::Overview
    } else {
        HubSection::Basics
    }));
    let can_exit = on_back.is_some();
    let open_section = move |section: HubSection| {
        current_section.set(section);
        if let Some(window) = web_sys::window() {
            window.scroll_to_with_x_and_y(0.0, 0.0);
        }
    };

    let is_publishing = RwSignal::new(false);
    let is_hashing = RwSignal::new(false);
    let error_message = RwSignal::new(None::<String>);
    let progress_events = RwSignal::new(Vec::<PublishProgressPayload>::new());
    let upload_progress = RwSignal::new(None::<PublishProgressPayload>);
    let publication_state = RwSignal::new(PublicationState::default());
    let initiating_account = RwSignal::new(None::<String>);
    let publication_account_stale = RwSignal::new(false);
    let active_publication =
        StoredValue::new_local(std::rc::Rc::new(std::cell::RefCell::new(None::<String>)));
    on_cleanup(move || active_publication.with_value(|active| *active.borrow_mut() = None));
    let publication_view_alive =
        StoredValue::new_local(std::rc::Rc::new(std::cell::Cell::new(true)));
    on_cleanup(move || publication_view_alive.with_value(|alive| alive.set(false)));
    let publish_draft = RwSignal::new(PublishState {
        archive: ArchiveSelectionState::new(auth.npub.get_untracked().unwrap_or_default()),
        launch: LaunchMode::DownloadOnly,
    });
    let authoritative_edit = RwSignal::new(None::<AuthoritativeEditModel>);
    let publish_attempt = RwSignal::new(PublishAttempt::default());
    let acknowledged_removed_profile_ids = RwSignal::new(Vec::<ProfileId>::new());
    let pending_removal_confirmation = RwSignal::new(None::<PendingRemovalConfirmation>);
    let removal_confirmation_generation = RwSignal::new(0_u64);
    let removal_abort_count = RwSignal::new(0_u64);
    let defect_review = RwSignal::new(DefectReviewState::default());
    let surfaced_archive_warnings = RwSignal::new(Vec::<String>::new());
    let distribution_user_revision = RwSignal::new(0_u64);
    let platform_user_revision = RwSignal::new(0_u64);
    let operator_initialized = RwSignal::new(operator_resolution.is_none());

    Effect::new(move |_| {
        if let Some(expected) = initiating_account.get() {
            if !publication_account_matches(&expected, auth.npub.get().as_deref()) {
                publication_account_stale.set(true);
            }
        }
    });

    Effect::new(move |_| {
        let next_owner = auth.npub.get().unwrap_or_default();
        let previous_owner = publish_draft.get_untracked().archive.owner_npub;
        if previous_owner != next_owner {
            publish_draft.update(|state| {
                state
                    .invalidate_account(next_owner.clone(), ArchiveAccountFailure::AccountMismatch);
            });
            file_hash.set(authoritative_file_hash_for_account.clone());
        }
    });

    if let Some(edit_listing) = listing.clone() {
        let mut model = AuthoritativeEditModel::new(
            edit_listing.publisher_npub.clone(),
            LaunchMode::DownloadOnly,
        );
        let operation = model.begin_load(edit_listing.event_id.clone());
        authoritative_edit.set(Some(model));
        if let Some(operation) = operation {
            let request = LoadAuthoritativeAdpEditMetadataRequest {
                expected_publisher_npub: edit_listing.publisher_npub.clone(),
                d_tag: edit_listing.id.clone(),
                expected_event_id: operation.expected_event_id.clone(),
            };
            spawn_local(async move {
                let response = invoke_load_authoritative_adp_edit_metadata(request).await;
                authoritative_edit.update(|current| {
                    let Some(model) = current.as_mut() else {
                        return;
                    };
                    if matches!(
                        model.apply_response(&operation, response),
                        Transition::Applied(())
                    ) {
                        let projected = preserve_edited_platforms(
                            model.draft.clone(),
                            parse_csv_values(&platforms_input.get_untracked()),
                            platform_user_revision.get_untracked(),
                        );
                        if platform_user_revision.get_untracked() == 0 {
                            if let LaunchMode::Launchable(tier) = &projected {
                                platforms_input.set(tier.platforms.join(", "));
                            }
                        }
                        publish_draft.update(|state| state.launch = projected);
                    }
                });
            });
        }
    }

    let add_server = move |url: String, label: String, auto_operator: bool| {
        let trimmed = url.trim().to_string();
        if trimmed.is_empty() {
            return;
        }
        servers.update(|entries| {
            if entries.iter().any(|entry| entry.url == trimmed) {
                return;
            }
            entries.push(ServerEntry {
                url: trimmed.clone(),
                label,
                reachability: ServerStatus::Pending,
                upload: ServerStatus::Idle,
                auto_operator,
            });
        });
        distribution_user_revision.update(|revision| *revision = revision.wrapping_add(1));
        let servers_for_check = servers;
        spawn_local(async move {
            let status = match invoke_check_adp_server(trimmed.clone()).await {
                Ok(_) => ServerStatus::Ok,
                Err(_) => ServerStatus::Failed,
            };
            servers_for_check.update(|entries| {
                if let Some(entry) = entries.iter_mut().find(|entry| entry.url == trimmed) {
                    entry.reachability = status;
                }
            });
        });
    };

    let remove_server = move |url: String| {
        servers.update(|entries| entries.retain(|entry| entry.url != url));
        distribution_user_revision.update(|revision| *revision = revision.wrapping_add(1));
        if operator_auto_added.get_untracked().as_deref() == Some(url.as_str()) {
            operator_auto_added.set(None);
        }
    };

    let sync_operator_server = move |new_url: String| {
        if let Some(old_url) = operator_auto_added.get_untracked() {
            servers.update(|entries| {
                entries.retain(|entry| !(entry.auto_operator && entry.url == old_url))
            });
        }
        if new_url.trim().is_empty() {
            operator_auto_added.set(None);
            return;
        }
        operator_auto_added.set(Some(new_url.clone()));
        add_server(new_url, "Distribution service".to_string(), true);
    };

    Effect::new(move |_| {
        spawn_local(async move {
            match invoke_discover_adp_servers().await {
                Ok(found) => {
                    discovered_servers.set(found);
                    discovery_error.set(None);
                }
                Err(err) => discovery_error.set(Some(format!(
                    "Couldn't look up distribution services: {err}"
                ))),
            }
        });
    });

    Effect::new(move |_| {
        for url in initial_server_urls.clone() {
            spawn_local(async move {
                let status = match invoke_check_adp_server(url.clone()).await {
                    Ok(_) => ServerStatus::Ok,
                    Err(_) => ServerStatus::Failed,
                };
                servers.update(|entries| {
                    if let Some(entry) = entries.iter_mut().find(|entry| entry.url == url) {
                        entry.reachability = status;
                    }
                });
            });
        }
    });

    if let Some(request) = operator_resolution {
        Effect::new(move |_| {
            let request = request.clone();
            spawn_local(async move {
                let resolution = invoke_resolve_adp_operator(request).await;
                if let Some(resolved) =
                    operator_prefill_update(&operator_url.get_untracked(), resolution)
                {
                    operator_url.set(resolved);
                }
                operator_initialized.set(true);
            });
        });
    }

    let on_add_custom_server = move |_| {
        let url = custom_server.get();
        add_server(url.clone(), url.clone(), false);
        custom_server.set(String::new());
    };

    let on_select_file = move |_| {
        if !can_dispatch(is_hashing.get_untracked(), is_publishing.get_untracked()) {
            return;
        }
        let Some(expected_publisher_npub) = auth.npub.get_untracked() else {
            error_message.set(Some("Sign in before selecting an archive.".into()));
            return;
        };
        let request_id = match archive_request_id() {
            Ok(request_id) => request_id,
            Err(error) => {
                error_message.set(Some(archive_error_message(&error)));
                return;
            }
        };
        let operation = publish_draft
            .try_update(|state| state.archive.begin_picker(request_id.clone()))
            .unwrap_or_else(|| {
                let mut fallback = ArchiveSelectionState::new(expected_publisher_npub.clone());
                fallback.begin_picker(request_id.clone())
            });
        is_hashing.set(true);
        error_message.set(None);
        spawn_local(async move {
            let selected = invoke_select_archive_build(SelectArchiveRequest {
                request_id: operation.request_id.clone(),
                expected_publisher_npub: expected_publisher_npub.clone(),
            })
            .await
            .map(|response| response.selection);
            let returned_selection = selected.as_ref().ok().and_then(Clone::clone);
            let selected_new_archive = returned_selection.is_some();
            if auth.npub.get_untracked().as_deref() != Some(expected_publisher_npub.as_str()) {
                if let Some(selection) = returned_selection {
                    if let Ok(request_id) = archive_request_id() {
                        let _ = invoke_discard_archive_selection(DiscardArchiveSelectionRequest {
                            request_id,
                            selection_id: selection.selection_id,
                            expected_publisher_npub,
                        })
                        .await;
                    }
                }
                is_hashing.set(false);
                return;
            }
            let transition = publish_draft
                .try_update(|state| state.apply_picker(&operation, selected))
                .unwrap_or(Transition::Stale);
            let replaced = match transition {
                Transition::Applied(replaced) => replaced,
                Transition::Stale => {
                    if let Some(selection) = returned_selection {
                        if let Ok(request_id) = archive_request_id() {
                            let _ =
                                invoke_discard_archive_selection(DiscardArchiveSelectionRequest {
                                    request_id,
                                    selection_id: selection.selection_id,
                                    expected_publisher_npub,
                                })
                                .await;
                        }
                    }
                    is_hashing.set(false);
                    return;
                }
            };
            if selected_new_archive {
                distribution_user_revision.update(|revision| *revision = revision.wrapping_add(1));
            }
            if let Some(previous) = replaced {
                if let Ok(request_id) = archive_request_id() {
                    let _ = invoke_discard_archive_selection(DiscardArchiveSelectionRequest {
                        request_id,
                        selection_id: previous.selection_id,
                        expected_publisher_npub: expected_publisher_npub.clone(),
                    })
                    .await;
                }
            }
            let Some(selection_id) = publish_draft
                .get_untracked()
                .archive
                .selection
                .map(|selection| selection.selection_id)
            else {
                is_hashing.set(false);
                return;
            };
            file_hash.set(None);

            let inspect_request_id = match archive_request_id() {
                Ok(request_id) => request_id,
                Err(error) => {
                    error_message.set(Some(archive_error_message(&error)));
                    is_hashing.set(false);
                    return;
                }
            };
            let Some(inspect_operation) = publish_draft
                .try_update(|state| {
                    state
                        .archive
                        .begin_selection_operation(inspect_request_id.clone())
                })
                .flatten()
            else {
                is_hashing.set(false);
                return;
            };
            let inspection = invoke_inspect_archive(InspectArchiveRequest {
                request_id: inspect_request_id,
                selection_id: selection_id.clone(),
                expected_publisher_npub: expected_publisher_npub.clone(),
                expected_format: None,
            })
            .await
            .map(|response| response.summary);
            let detected_package = inspection
                .as_ref()
                .ok()
                .map(|summary| match &summary.detected_format {
                    arcadestr_core::distribution::PackageFormat::Zip => "zip",
                    arcadestr_core::distribution::PackageFormat::TarZstd => "tar.zst",
                    arcadestr_core::distribution::PackageFormat::TarGz => "tar.gz",
                })
                .map(str::to_string);
            let safe_to_hash = inspection
                .as_ref()
                .is_ok_and(|summary| !summary.blocking_hazards);
            let inspection_error = inspection.as_ref().err().cloned();
            let summary_transition = publish_draft
                .try_update(|state| state.archive.apply_summary(&inspect_operation, inspection))
                .unwrap_or(Transition::Stale);
            if matches!(summary_transition, Transition::Stale) {
                is_hashing.set(false);
                return;
            }
            if let Some(package_format) = detected_package {
                publish_draft.update(|state| {
                    if let LaunchMode::Launchable(tier) = &mut state.launch {
                        tier.package_format.text = package_format;
                    }
                });
            }
            if let Some(error) = inspection_error {
                error_message.set(Some(archive_error_message(&error)));
            }
            if !safe_to_hash {
                is_hashing.set(false);
                return;
            }

            let hash_request_id = match archive_request_id() {
                Ok(request_id) => request_id,
                Err(error) => {
                    error_message.set(Some(archive_error_message(&error)));
                    is_hashing.set(false);
                    return;
                }
            };
            let Some(hash_operation) = publish_draft
                .try_update(|state| {
                    state
                        .archive
                        .begin_selection_operation(hash_request_id.clone())
                })
                .flatten()
            else {
                is_hashing.set(false);
                return;
            };
            let hashed = invoke_hash_selected_archive(HashSelectedArchiveRequest {
                request_id: hash_request_id,
                selection_id: selection_id.clone(),
                expected_publisher_npub,
            })
            .await
            .map(|response| BuildSelectionRef {
                selection_id: response.selection_id,
                sha256: response.sha256,
            });
            let hash_for_display = hashed
                .as_ref()
                .ok()
                .map(|build| build.sha256.as_str().to_string());
            if restored_artifact_hash
                .get_untracked()
                .as_ref()
                .is_some_and(|expected| {
                    hash_for_display
                        .as_ref()
                        .is_some_and(|actual| actual != expected)
                })
            {
                publish_draft.update(|state| {
                    state.archive.consume_success();
                    state.launch.mark_bindings_stale();
                });
                error_message.set(Some("This isn't the same build file you chose before. Select the original file, or remove the build and choose a new one.".into()));
                is_hashing.set(false);
                return;
            }
            let hash_error = hashed.as_ref().err().cloned();
            let transition = publish_draft
                .try_update(|state| state.archive.apply_hash(&hash_operation, hashed))
                .unwrap_or(Transition::Stale);
            if matches!(transition, Transition::Applied(())) {
                if let Some(hash) = hash_for_display {
                    file_hash.set(Some(hash));
                }
                if let Some(error) = hash_error {
                    error_message.set(Some(archive_error_message(&error)));
                }
            }
            is_hashing.set(false);
        });
    };

    // Single source of section validation, reused by the checklist, the
    // section headers and the publish button, so no surface can disagree about
    // whether a section is actually satisfied.
    // First Store page problem in the fields a section owns.
    let store_page_blocker = move |section: HubSection| -> Result<(), String> {
        let platforms = parse_platform_tags(&platforms_input.get()).unwrap_or_default();
        let state = embedded_readiness(
            &store_page_draft.get(),
            &platforms,
            &store_page_diagnostics.get(),
        );
        match state.blockers().iter().find(|blocker| {
            HubSection::for_store_page_group(
                crate::ui_v2::views::store_page_publish::diagnostic_tab(blocker),
            ) == section
        }) {
            Some(blocker) => Err(blocker.clone()),
            None => Ok(()),
        }
    };
    // Non-blocking improvements; only the Store page has them.
    let section_suggestions = move |section: HubSection| -> Vec<String> {
        if section != HubSection::StorePage || store_page_status.get() != Some(Ok(())) {
            return Vec::new();
        }
        let platforms = parse_platform_tags(&platforms_input.get()).unwrap_or_default();
        prioritized_suggestions(
            embedded_readiness(
                &store_page_draft.get(),
                &platforms,
                &store_page_diagnostics.get(),
            )
            .recommendations(),
        )
    };
    let validate_section = move |section: HubSection| -> Result<(), String> {
        match section {
            HubSection::Basics => validate_listing(
                &title.get(),
                &description.get(),
                0,
                "",
                false,
                &[],
                &None,
                &None,
                "",
                &FulfillmentMode::None,
                "",
            )
            .and_then(|_| store_page_blocker(HubSection::Basics)),
            HubSection::StorePage => match store_page_status.get() {
                None => Err("The store page is still loading.".into()),
                Some(Err(error)) => Err(format!("The store page did not load: {error}")),
                Some(Ok(())) => store_page_blocker(HubSection::StorePage),
            },
            HubSection::PriceAccess => {
                price_for_acquisition(acquisition_kind.get(), &price_input.get(), &lud16.get())
                    .and_then(|_| {
                        acquisition_policy_from_form(
                            acquisition_kind.get(),
                            &acquisition_starts_at.get(),
                            &acquisition_ends_at.get(),
                        )
                        .map(|_| ())
                    })
            }
            HubSection::Delivery => {
                if !delivery_chosen.get() {
                    return Err("Choose how players get your game.".to_string());
                }
                let draft = publish_draft.get();
                if matches!(draft.launch, LaunchMode::Launchable(_)) && !fulfillment_enabled.get() {
                    return Err(
                        "Launch settings need a build. Choose \"Arcadestr installs it\" or \"Download only\"."
                            .to_string(),
                    );
                }
                let selection_id = draft
                    .archive
                    .selected_build
                    .as_ref()
                    .map(|build| &build.selection_id);
                draft
                    .launch
                    .to_wire_result(selection_id)
                    .map(|_| ())
                    .map_err(|_| {
                        "Finish the launch settings: every launch option needs a name and a program to run."
                            .to_string()
                    })
                    .and_then(|_| parse_platform_tags(&platforms_input.get()).map(|_| ()))
                    .and_then(|_| {
                        if !fulfillment_enabled.get() {
                            return Ok(());
                        }
                        validate_listing(
                            &title.get(),
                            &description.get(),
                            0,
                            "",
                            true,
                            &servers.get(),
                            &None,
                            &file_hash.get(),
                            &version.get(),
                            &fulfillment_mode.get(),
                            &operator_url.get(),
                        )
                    })
                    .and_then(|_| {
                        if fulfillment_enabled.get()
                            && commercial_mode.get() == "replace"
                            && !(commercial_review.get().is_some() && commercial_approved.get())
                        {
                            Err("Review and approve the distribution service's fee terms.".into())
                        } else {
                            Ok(())
                        }
                    })
            }
            HubSection::Overview => Ok(()),
        }
    };

    let blocking_issues = move || {
        let mut issues = blocking_section_errors(
            &HubSection::EDITABLE
                .into_iter()
                .map(|section| (section, validate_section(section)))
                .collect::<Vec<_>>(),
        );
        if editing
            && !authoritative_edit.get().is_some_and(|model| {
                matches!(
                    model.state,
                    AuthoritativeEditState::Absent | AuthoritativeEditState::PresentValid(_)
                ) || (matches!(model.state, AuthoritativeEditState::PresentDefective(_))
                    && defect_review.get().reviewed)
            })
        {
            issues.push((
                HubSection::Delivery,
                "Your game's current launch settings must finish loading before you can update it."
                    .to_string(),
            ));
        }
        issues
    };

    let commercial_context_auth = auth.clone();
    let commercial_context = move || -> Result<PublisherTermsContext, String> {
        let publisher_pubkey = commercial_context_auth
            .npub
            .get()
            .as_deref()
            .and_then(publisher_hex)
            .ok_or("Authenticate as the publisher first")?;
        let kind = acquisition_kind.get();
        let destination = lud16.get().trim().to_string();
        Ok(PublisherTermsContext {
            publisher_pubkey,
            servers: if fulfillment_enabled.get() {
                servers.get().iter().map(|s| s.url.clone()).collect()
            } else {
                Vec::new()
            },
            price_sats: price_for_acquisition(kind, &price_input.get(), &destination)?,
            developer_destination: if kind == AcquisitionKind::Gated {
                destination
            } else {
                String::new()
            },
        })
    };
    let last_commercial_context = RwSignal::new(None::<PublisherTermsContext>);
    let last_commercial_epoch = RwSignal::new(None::<u64>);
    let commercial_epoch_auth = auth.clone();
    let commercial_context_for_effect = commercial_context.clone();
    Effect::new(move |_| {
        let context = commercial_context_for_effect().ok();
        let epoch = commercial_epoch_auth.current_auth_flow();
        if last_commercial_context.get_untracked() != context
            || last_commercial_epoch.get_untracked() != Some(epoch)
        {
            last_commercial_context.set(context);
            last_commercial_epoch.set(Some(epoch));
            commercial_review.set(None);
            commercial_approved.set(false);
            commercial_loading.set(false);
            commercial_generation.update(|value| *value = value.wrapping_add(1));
        }
    });
    let review_auth = auth.clone();
    let commercial_context_for_review = commercial_context.clone();
    let on_review_terms = Callback::new(move |_: ()| {
        let context = match commercial_context_for_review() {
            Ok(context) => context,
            Err(error) => {
                commercial_error.set(Some(error));
                return;
            }
        };
        commercial_generation.update(|value| *value = value.wrapping_add(1));
        let generation = commercial_generation.get_untracked();
        let epoch = review_auth.current_auth_flow();
        let response_auth = review_auth.clone();
        commercial_review.set(None);
        commercial_approved.set(false);
        commercial_error.set(None);
        commercial_loading.set(true);
        spawn_local(async move {
            let result = crate::tauri_bridge::invoke_preview_publisher_terms(context).await;
            if commercial_generation.get_untracked() != generation
                || !response_auth.is_current_auth_flow(epoch)
            {
                return;
            }
            commercial_loading.set(false);
            match result {
                Ok(review) => commercial_review.set(Some(review)),
                Err(error) => commercial_error.set(Some(error)),
            }
        });
    });

    let full_form_snapshot = move || FormSnapshot {
        title: title.get(),
        description: description.get(),
        tags: tag_input.get(),
        price: price_input.get(),
        lud16: lud16.get(),
        platforms: platforms_input.get(),
        version: version.get(),
        operator_url: operator_url.get(),
        acquisition: format!("{:?}", acquisition_kind.get()),
        starts_at: acquisition_starts_at.get(),
        ends_at: acquisition_ends_at.get(),
        fulfillment_enabled: fulfillment_enabled.get(),
        commercial: format!(
            "{}:{:?}:{}",
            commercial_mode.get(),
            commercial_review.get(),
            commercial_approved.get()
        ),
    };
    // Everything the local draft stores. Tracked reads, so the autosave effect
    // reruns on any change; callbacks call it outside a reactive context.
    let draft_snapshot = full_form_snapshot.clone();
    let build_durable_draft = move || DurableListingDraft {
        server_roles: server_roles.get(),
        form: draft_snapshot(),
        identity: published_identity.get(),
        launch: publish_draft.get().launch,
        servers: servers.get().into_iter().map(|server| server.url).collect(),
        mode: fulfillment_mode.get(),
        acquisition: acquisition_kind.get(),
        artifact_hash: file_hash.get(),
        commercial_intent: commercial_mode.get(),
        commercial_review: commercial_review.get(),
        accepted_terms: durable_accepted_terms.get_value(),
        authorization_references: durable_authorizations.get_value(),
        store_page: Some(store_page_draft.get()),
        delivery_chosen: delivery_chosen.get(),
    };
    let save_auth = auth.clone();
    let save_build = build_durable_draft.clone();
    let on_save_draft = Callback::new(move |_: ()| {
        let Some(publisher) = save_auth.npub.get_untracked() else {
            draft_save_status.set(DraftSaveStatus::Failed("sign in to save drafts".into()));
            return;
        };
        if durable_draft_owner.get_value().as_deref() != Some(publisher.as_str()) {
            draft_save_status.set(DraftSaveStatus::Failed(
                "switch back to the account that owns this game".into(),
            ));
            return;
        }
        if draft_save_status.get_untracked() == DraftSaveStatus::Conflict {
            return;
        }
        if draft_busy.get_untracked() || is_publishing.get_untracked() {
            // The autosave effect reschedules once the running work finishes.
            return;
        }
        let epoch = save_auth.current_auth_flow();
        let response_auth = save_auth.clone();
        let payload = match serde_json::to_value(untrack(|| save_build())) {
            Ok(payload) => payload,
            Err(_) => {
                draft_save_status.set(DraftSaveStatus::Failed(
                    "the draft could not be encoded".into(),
                ));
                return;
            }
        };
        let saved_fingerprint = payload.to_string();
        let draft = arcadestr_core::publisher_recovery::PublisherDraft {
            draft_id: durable_draft_id.get_value(),
            kind: "listing".into(),
            payload,
            revision: draft_revision.get_untracked().unwrap_or(0),
        };
        draft_busy.set(true);
        draft_save_status.set(DraftSaveStatus::Saving);
        spawn_local(async move {
            let result = crate::tauri_bridge::invoke_save_publisher_draft(
                publisher,
                draft,
                draft_revision.get_untracked(),
            )
            .await;
            if !response_auth.is_current_auth_flow(epoch) {
                return;
            }
            let _ = draft_busy.try_set(false);
            match result {
                Ok(revision) => {
                    let _ = draft_revision.try_set(Some(revision));
                    let _ = last_saved_fingerprint.try_set(Some(saved_fingerprint));
                    let _ = draft_save_status.try_set(DraftSaveStatus::Saved);
                    // Edits made while saving are picked up by the next run.
                    let _ = autosave_tick.try_update(|tick| *tick = tick.wrapping_add(1));
                }
                Err(error) if error.contains("Draft changed") => {
                    let _ = draft_save_status.try_set(DraftSaveStatus::Conflict);
                }
                Err(error) => {
                    let _ = draft_save_status.try_set(DraftSaveStatus::Failed(error));
                }
            }
        });
    });
    let restore_auth = auth.clone();
    let restore_build = build_durable_draft.clone();
    let on_restore_draft = Callback::new(move |_: ()| {
        let Some(publisher) = restore_auth.npub.get_untracked() else {
            draft_ready.set(true);
            return;
        };
        if durable_draft_owner.get_value().as_deref() != Some(publisher.as_str()) {
            draft_ready.set(true);
            return;
        }
        if draft_busy.get_untracked() || is_publishing.get_untracked() || is_hashing.get_untracked()
        {
            return;
        }
        let epoch = restore_auth.current_auth_flow();
        let response_auth = restore_auth.clone();
        let restore_build = restore_build.clone();
        let published_build = authoritative_file_hash_for_restore.clone();
        draft_busy.set(true);
        spawn_local(async move {
            let result = crate::tauri_bridge::invoke_publisher_recovery_state(publisher).await;
            if !response_auth.is_current_auth_flow(epoch) {
                return;
            }
            let _ = draft_busy.try_set(false);
            let _ = draft_ready.try_set(true);
            let stored = match result {
                Ok(state) => state
                    .drafts
                    .into_iter()
                    .find(|draft| draft.draft_id == durable_draft_id.get_value()),
                Err(error) => {
                    let _ = draft_save_status.try_set(DraftSaveStatus::Failed(error));
                    return;
                }
            };
            let Some(stored) = stored else {
                // Nothing saved yet; a later save creates the draft.
                let _ = draft_revision.try_set(None);
                let _ = draft_save_status.try_set(DraftSaveStatus::Idle);
                return;
            };
            let snapshot: DurableListingDraft = match serde_json::from_value(stored.payload) {
                Ok(snapshot) => snapshot,
                Err(_) => {
                    // Keep the revision so the next save replaces the unreadable draft.
                    let _ = draft_revision.try_set(Some(stored.revision));
                    let _ = draft_message.try_set(Some(
                        "The saved draft could not be read and will be replaced by your next edit."
                            .into(),
                    ));
                    return;
                }
            };
            let had_build =
                snapshot.artifact_hash.is_some() && snapshot.artifact_hash != published_build;
            title.set(snapshot.form.title);
            description.set(snapshot.form.description);
            tag_input.set(snapshot.form.tags);
            price_input.set(snapshot.form.price);
            lud16.set(snapshot.form.lud16);
            platforms_input.set(snapshot.form.platforms);
            version.set(snapshot.form.version);
            operator_url.set(snapshot.form.operator_url);
            acquisition_starts_at.set(snapshot.form.starts_at);
            acquisition_ends_at.set(snapshot.form.ends_at);
            fulfillment_enabled.set(snapshot.form.fulfillment_enabled);
            acquisition_kind.set(snapshot.acquisition);
            fulfillment_mode.set(snapshot.mode);
            server_roles.set(snapshot.server_roles);
            authorize_operators.set(false);
            if let Some(page) = snapshot.store_page {
                store_page_draft.set(page);
            }
            servers.set(
                snapshot
                    .servers
                    .into_iter()
                    .map(|url| ServerEntry {
                        label: url.clone(),
                        url,
                        reachability: ServerStatus::Idle,
                        upload: ServerStatus::Idle,
                        auto_operator: false,
                    })
                    .collect(),
            );
            published_identity.set(snapshot.identity);
            restored_artifact_hash.set(snapshot.artifact_hash.clone());
            file_hash.set(snapshot.artifact_hash);
            publish_draft.update(|state| {
                state.archive.consume_success();
                state.launch = snapshot.launch;
                state.launch.mark_bindings_stale();
            });
            commercial_mode.set(snapshot.commercial_intent);
            commercial_review.set(snapshot.commercial_review);
            commercial_approved.set(false);
            durable_accepted_terms.set_value(snapshot.accepted_terms);
            durable_authorizations.set_value(snapshot.authorization_references);
            delivery_chosen.set(snapshot.delivery_chosen || editing);
            draft_revision.set(Some(stored.revision));
            last_saved_fingerprint.set(
                serde_json::to_value(untrack(|| restore_build()))
                    .ok()
                    .map(|value| value.to_string()),
            );
            draft_save_status.set(DraftSaveStatus::Saved);
            draft_message.set(Some(if had_build {
                "Restored your unpublished changes. Select your build file again before publishing; it must match the one you chose before.".into()
            } else {
                "Restored your unpublished changes.".into()
            }));
        });
    });
    let form_snapshot = {
        let full_form_snapshot = full_form_snapshot.clone();
        move || {
            let mut snapshot = full_form_snapshot();
            // These fields have an asynchronously rebased distribution baseline.
            snapshot.platforms.clear();
            snapshot.version.clear();
            snapshot.operator_url.clear();
            snapshot
        }
    };
    let fulfillment_key_for_snapshot = existing_fulfillment_pubkey.clone();
    let nip94_for_snapshot = existing_nip94_event_id.clone();
    let distribution_snapshot = move || {
        let draft = publish_draft.get();
        PublishDirtySnapshot::capture(
            &draft.archive,
            &draft.launch,
            ProviderBuildFields {
                fulfillment_mode: format!("{:?}", fulfillment_mode.get()),
                operator_url: (!operator_url.get().trim().is_empty()).then(|| operator_url.get()),
                servers: servers.get().into_iter().map(|server| server.url).collect(),
                existing_file_hash: draft
                    .archive
                    .selected_build
                    .is_none()
                    .then(|| file_hash.get())
                    .flatten(),
                existing_fulfillment_pubkey: fulfillment_key_for_snapshot.clone(),
                version: (!version.get().trim().is_empty()).then(|| version.get()),
                nip94_event_id: nip94_for_snapshot.clone(),
                platforms: parse_csv_values(&platforms_input.get()),
            },
            acknowledged_removed_profile_ids.get(),
            defect_review.get(),
        )
    };

    let existing_campaigns_for_submit = existing_campaigns.clone();
    let existing_nip94_for_submit = existing_nip94_event_id.clone();
    let existing_fulfillment_key_for_submit = existing_fulfillment_pubkey.clone();
    let editing_publisher_for_submit = editing_publisher.clone();
    let existing_listing_for_dispatch = existing_listing_for_submit.clone();
    let form_snapshot_for_dispatch = full_form_snapshot.clone();
    let distribution_snapshot_for_dispatch = distribution_snapshot.clone();
    let publication_auth = auth.clone();
    let dispatch_prepared = Callback::new(
        move |(prepared, initiating_npub, confirmed_removed_ids): (
            PreparedPublish,
            String,
            Option<Vec<ProfileId>>,
        )| {
            if active_publication.with_value(|active| active.borrow().is_some()) {
                return;
            }
            let mut prepared = prepared;
            let prior_operation = publish_attempt
                .get_untracked()
                .prepared
                .as_ref()
                .and_then(|previous| previous.extension.operation_id.clone());
            let operation_id = uuid::Uuid::new_v4().to_string();
            let auth_epoch = publication_auth.current_auth_flow();
            let response_auth = publication_auth.clone();
            prepared.extension.operation_id = Some(operation_id.clone());
            let active_operation = active_publication.get_value();
            *active_operation.borrow_mut() = Some(operation_id.clone());
            let published_request = prepared.request.clone();
            let published_commercial_intent = prepared.extension.commercial_intent.clone();
            let existing_listing = existing_listing_for_dispatch.clone();
            is_publishing.set(true);
            initiating_account.set(Some(initiating_npub.clone()));
            publication_account_stale.set(false);
            error_message.set(None);
            progress_events.set(Vec::new());
            upload_progress.set(None);
            publication_state.set(PublicationState {
                outcome: PublicationOutcome::Publishing,
                ..PublicationState::default()
            });
            servers.update(|entries| {
                for entry in entries {
                    entry.upload = ServerStatus::Idle;
                }
            });

            let form_at_dispatch = form_snapshot_for_dispatch.clone();
            let dirty_at_dispatch = distribution_snapshot_for_dispatch.clone();
            spawn_local(async move {
                let progress_account = initiating_npub.clone();
                let progress_operation = operation_id.clone();
                let progress_active = active_operation.clone();
                let progress_auth = response_auth.clone();
                let mut progress_coordinate =
                    prepared.request.existing_d_tag.as_ref().and_then(|d| {
                        publisher_hex(&progress_account)
                            .map(|publisher| format!("30402:{publisher}:{d}"))
                    });
                let listener_cleanup = listen_publish_progress(move |payload| {
                    if progress_active.borrow().as_deref() != Some(progress_operation.as_str())
                        || !progress_auth.is_current_auth_flow(auth_epoch)
                        || publication_account_stale.get_untracked()
                        || !publication_account_matches(
                            &progress_account,
                            auth.npub.get_untracked().as_deref(),
                        )
                    {
                        return;
                    }
                    if !accepts_publication_progress(
                        &progress_operation,
                        &progress_account,
                        &mut progress_coordinate,
                        &payload,
                    ) {
                        return;
                    }
                    published_identity.update(|identity| {
                        *identity = publication_identity_progress(identity.clone(), &payload)
                    });
                    if let Some(page) =
                        store_page_after_progress(&store_page_draft.get_untracked(), &payload)
                    {
                        store_page_draft.set(page.clone());
                        store_page_baseline.set(page);
                    }
                    if is_recovery_channel(&payload.step) {
                        return;
                    }
                    if payload.step == "upload" {
                        if payload.bytes_uploaded.is_some() && payload.total_bytes.is_some() {
                            upload_progress.set(Some(payload.clone()));
                        }
                        if let Some(server_url) = payload.server_url.clone() {
                            let status = match payload.status.as_str() {
                                "pending" | "progress" => ServerStatus::Pending,
                                "ok" => ServerStatus::Ok,
                                "error" => ServerStatus::Failed,
                                _ => ServerStatus::Idle,
                            };
                            servers.update(|entries| {
                                if let Some(entry) =
                                    entries.iter_mut().find(|entry| entry.url == server_url)
                                {
                                    entry.upload = status;
                                }
                            });
                        }
                    }
                    publication_state
                        .update(|state| *state = publication_progress(state.clone(), &payload));
                    if payload.status != "progress" {
                        progress_events.update(|events| events.push(payload));
                    }
                })
                .await;
                if active_operation.borrow().as_deref() != Some(operation_id.as_str()) {
                    if let Ok(cleanup) = listener_cleanup {
                        cleanup();
                    }
                    return;
                }
                let listener_cleanup = match listener_cleanup {
                    Ok(cleanup) => cleanup,
                    Err(error) => {
                        *active_operation.borrow_mut() = None;
                        publication_state.set(PublicationState {
                            outcome: PublicationOutcome::Failed,
                            listing_published: false,
                            message: Some(format!(
                                "Publication did not start because progress monitoring is unavailable: {error}"
                            )),
                        });
                        is_publishing.set(false);
                        initiating_account.set(None);
                        return;
                    }
                };

                if !response_auth.is_current_auth_flow(auth_epoch) {
                    listener_cleanup();
                    *active_operation.borrow_mut() = None;
                    is_publishing.set(false);
                    initiating_account.set(None);
                    return;
                }

                if let Some(prior) = prior_operation {
                    let recovery = crate::tauri_bridge::invoke_publisher_recovery_state(
                        initiating_npub.clone(),
                    )
                    .await;
                    if !response_auth.is_current_auth_flow(auth_epoch) {
                        listener_cleanup();
                        return;
                    }
                    if recovery.as_ref().is_err()
                        || recovery.ok().is_some_and(|state| {
                            state.operations.iter().any(|operation| {
                                operation.operation_id == prior && operation.state != "complete"
                            })
                        })
                    {
                        listener_cleanup();
                        *active_operation.borrow_mut() = None;
                        is_publishing.set(false);
                        initiating_account.set(None);
                        error_message.set(Some("An earlier publish of this game didn't finish. Your changes are saved as a draft. Select Finish publishing above to complete it, then publish your changes.".into()));
                        return;
                    }
                }
                if let Some(removed_profile_ids) = confirmed_removed_ids {
                    acknowledged_removed_profile_ids.set(removed_profile_ids);
                }
                publish_attempt.update(|attempt| attempt.start(prepared.clone()));
                let publish_result = invoke_publish_adp_listing_with_contract(
                    prepared.request.clone(),
                    prepared.extension.clone(),
                )
                .await;
                listener_cleanup();
                if active_operation.borrow().as_deref() != Some(operation_id.as_str()) {
                    return;
                }
                *active_operation.borrow_mut() = None;
                if publication_account_stale.get_untracked()
                    || !response_auth.is_current_auth_flow(auth_epoch)
                    || !publication_account_matches(
                        &initiating_npub,
                        auth.npub.get_untracked().as_deref(),
                    )
                {
                    let current = publication_state.get_untracked();
                    publication_state.set(PublicationState {
                        outcome: if current.listing_published {
                            PublicationOutcome::Partial
                        } else {
                            PublicationOutcome::Failed
                        },
                        listing_published: current.listing_published,
                        message: Some(if current.listing_published {
                            "Your game was published, but you switched accounts before publishing finished. Switch back and open the game from Your games to check it."
                                .into()
                        } else {
                            "The active account changed during publication. The stale response was ignored; check Published games with the initiating account before retrying."
                                .into()
                        }),
                    });
                    is_publishing.set(false);
                    initiating_account.set(None);
                    return;
                }
                let published = match publish_result {
                    Ok(result) => {
                        if result.operation_id != operation_id {
                            is_publishing.set(false);
                            initiating_account.set(None);
                            error_message.set(Some("Publication response did not match the active operation; refresh Published games.".into()));
                            return;
                        }
                        pending_removal_confirmation.set(None);
                        publish_attempt.update(|attempt| attempt.succeed(result.clone()));
                        publish_draft.update(PublishState::consume_selected_archive);
                        published_identity.set(PublishedIdentity {
                            event_id: Some(result.event_id.clone()),
                            d_tag: Some(result.d_tag.clone()),
                            coordinate: Some(result.game_coordinate.clone()),
                            pending_d_tag: None,
                        });
                        let uploads_failed =
                            result.uploads.iter().any(|upload| upload.status != "ok");
                        publication_state.update(|state| {
                            *state = publication_completed(
                                state.clone(),
                                Ok((result.event_id.clone(), uploads_failed)),
                            )
                        });
                        (!uploads_failed).then(|| {
                            published_listing(
                                existing_listing,
                                &published_request,
                                &result,
                                &published_commercial_intent,
                            )
                        })
                    }
                    Err(err) => {
                        publish_attempt.update(|attempt| attempt.fail(err.clone()));
                        if let PublishAdpListingError::ProfileRemovalAcknowledgementRequired {
                            removed_profile_ids,
                        } = &err
                        {
                            let Some(frozen) = publish_attempt.get_untracked().prepared else {
                                publication_state.update(|state| {
                                    *state = publication_completed(
                                        state.clone(),
                                        Err("Publication removal confirmation could not retain its frozen payload. Submit again.".into()),
                                    )
                                });
                                is_publishing.set(false);
                                initiating_account.set(None);
                                return;
                            };
                            removal_confirmation_generation
                                .update(|generation| *generation = generation.wrapping_add(1));
                            removal_abort_count.update(|count| *count = count.wrapping_add(1));
                            pending_removal_confirmation.set(Some(PendingRemovalConfirmation {
                                prepared: frozen,
                                removed_profile_ids: removed_profile_ids.clone(),
                                form: form_at_dispatch(),
                                dirty: dirty_at_dispatch(),
                                identity: RemovalConfirmationIdentity {
                                    account_npub: auth.npub.get_untracked(),
                                    publication: published_identity.get_untracked(),
                                    authoritative_state: authoritative_edit
                                        .get_untracked()
                                        .map(|model| model.state),
                                },
                                generation: removal_confirmation_generation.get_untracked(),
                                abort_count: removal_abort_count.get_untracked(),
                            }));
                        }
                        if let Some(archive_error) = publish_archive_error(&err).cloned() {
                            publish_draft.update(|state| state.apply_archive_error(archive_error));
                        }
                        if matches!(err, PublishAdpListingError::StaleExpectedEventId { .. }) {
                            pending_removal_confirmation.set(None);
                            authoritative_edit.update(|state| {
                                if let Some(state) = state {
                                    state.state = AuthoritativeEditState::Stale(err.clone());
                                }
                            });
                        }
                        publication_state.update(|state| {
                            *state = publication_completed(
                                state.clone(),
                                Err(publish_error_message(&err)),
                            )
                        });
                        None
                    }
                };
                is_publishing.set(false);
                initiating_account.set(None);
                recovery_refresh.update(|value| *value = value.wrapping_add(1));
                if published.is_some() {
                    // Everything in the draft is now live; drop the local copy.
                    if let Some(revision) = draft_revision.get_untracked() {
                        let draft_id = durable_draft_id.get_value();
                        let publisher = initiating_npub.clone();
                        spawn_local(async move {
                            let _ = crate::tauri_bridge::invoke_delete_publisher_draft(
                                publisher,
                                draft_id,
                                "listing".into(),
                                revision,
                            )
                            .await;
                        });
                    }
                    draft_revision.set(None);
                    last_saved_fingerprint.set(None);
                    draft_save_status.set(DraftSaveStatus::Idle);
                }
                if let (Some(on_published), Some(listing)) = (on_published, published) {
                    on_published.run(listing);
                }
            });
        },
    );
    let submit_auth = auth.clone();
    let on_submit = Callback::new(move |()| {
        if !can_dispatch(is_hashing.get_untracked(), is_publishing.get_untracked()) {
            return;
        }
        let Some(initiating_npub) = auth.npub.get() else {
            error_message.set(Some(
                "Sign in before starting network publication".to_string(),
            ));
            return;
        };
        if !publication_account_allowed(editing_publisher_for_submit.as_deref(), &initiating_npub) {
            error_message.set(Some(
                "Switch to the account that published this game before updating it.".to_string(),
            ));
            return;
        }

        let input = PublishFormInput {
            initiating_npub: initiating_npub.clone(),
            title: title.get_untracked(),
            description: description.get_untracked(),
            tags: tag_input.get_untracked(),
            lud16: lud16.get_untracked(),
            acquisition: acquisition_kind.get_untracked(),
            price: price_input.get_untracked(),
            starts_at: acquisition_starts_at.get_untracked(),
            ends_at: acquisition_ends_at.get_untracked(),
            fulfillment_enabled: fulfillment_enabled.get_untracked(),
            fulfillment_mode: fulfillment_mode.get_untracked(),
            servers: servers.get_untracked(),
            operator_url: operator_url.get_untracked(),
            server_roles: server_roles.get_untracked(),
            authorize_operators: authorize_operators.get_untracked(),
            file_hash: file_hash.get_untracked(),
            version: version.get_untracked(),
            platforms: platforms_input.get_untracked(),
            publish_state: publish_draft.get_untracked(),
            store_page_loaded: store_page_status.get_untracked() == Some(Ok(())),
            store_page: store_page_draft.get_untracked(),
            commercial_mode: commercial_mode.get_untracked(),
            commercial_review: commercial_review.get_untracked(),
            commercial_approved: commercial_approved.get_untracked(),
            acknowledged_removed_profile_ids: acknowledged_removed_profile_ids.get_untracked(),
            identity: published_identity.get_untracked(),
            existing_fulfillment_pubkey: existing_fulfillment_key_for_submit.clone(),
            existing_campaigns: existing_campaigns_for_submit.clone(),
            existing_nip94_event_id: existing_nip94_for_submit.clone(),
            authoritative_file_hash: authoritative_file_hash_for_submit.clone(),
        };
        let PreparedPublish { request, extension } = match build_publish_request(input) {
            Ok(prepared) => prepared,
            Err(PublishBlocked::Message(message)) => {
                error_message.set(Some(message));
                return;
            }
            Err(PublishBlocked::StorePage(problem)) => {
                let tab = crate::ui_v2::views::store_page_publish::diagnostic_tab(&problem);
                let section = HubSection::for_store_page_group(tab);
                if section == HubSection::StorePage {
                    store_page_tab.set(tab);
                }
                current_section.set(section);
                error_message.set(Some(format!("Store page: {problem}")));
                return;
            }
            Err(PublishBlocked::Attempt(error)) => {
                publish_attempt.update(|attempt| attempt.fail(error.clone()));
                error_message.set(Some(publish_error_message(&error)));
                return;
            }
        };

        is_publishing.set(true);
        initiating_account.set(Some(initiating_npub.clone()));
        publication_account_stale.set(false);
        error_message.set(None);
        progress_events.set(Vec::new());
        upload_progress.set(None);
        publication_state.set(PublicationState {
            outcome: PublicationOutcome::Publishing,
            ..PublicationState::default()
        });
        servers.update(|entries| {
            for entry in entries {
                entry.upload = ServerStatus::Idle;
            }
        });

        let submit_epoch = submit_auth.current_auth_flow();
        let response_auth = submit_auth.clone();
        let view_alive = publication_view_alive.get_value();
        spawn_local(async move {
            let stop_stale_preparation = || {
                if !view_alive.get() {
                    return true;
                }
                if !response_auth.is_current_auth_flow(submit_epoch) {
                    is_publishing.set(false);
                    initiating_account.set(None);
                    return true;
                }
                false
            };
            if stop_stale_preparation() {
                return;
            }
            let current_prepared = PreparedPublish { request, extension };
            let retry_overlay = published_identity.get_untracked();
            let prepared = partial_retry_payload(
                &publish_attempt.get_untracked(),
                current_prepared,
                PublishIdentityOverlay {
                    existing_event_id: retry_overlay.event_id.clone(),
                    d_tag: retry_overlay.d_tag.clone(),
                    pending_d_tag: retry_overlay.pending_d_tag.clone(),
                },
            );
            dispatch_prepared.run((prepared, initiating_npub, None));
        });
    });

    let dismiss_removal_confirmation = Callback::new(move |()| {
        removal_confirmation_generation
            .update(|generation| *generation = generation.wrapping_add(1));
        pending_removal_confirmation.set(None);
    });
    let form_snapshot_for_confirmation = full_form_snapshot.clone();
    let distribution_snapshot_for_confirmation = distribution_snapshot.clone();
    let form_snapshot_for_invalidation = full_form_snapshot.clone();
    let distribution_snapshot_for_invalidation = distribution_snapshot.clone();
    Effect::new(move |_| {
        let Some(pending) = pending_removal_confirmation.get() else {
            return;
        };
        let current_identity = RemovalConfirmationIdentity {
            account_npub: auth.npub.get(),
            publication: published_identity.get(),
            authoritative_state: authoritative_edit.get().map(|model| model.state),
        };
        if !removal_confirmation_is_fresh(
            &pending,
            &form_snapshot_for_invalidation(),
            &distribution_snapshot_for_invalidation(),
            &current_identity,
            removal_confirmation_generation.get(),
        ) {
            removal_confirmation_generation
                .update(|generation| *generation = generation.wrapping_add(1));
            pending_removal_confirmation.set(None);
            error_message.set(Some(
                "Something changed after the check, so publishing was cancelled. Publish again to review it."
                    .into(),
            ));
        }
    });
    let confirm_profile_removal = Callback::new(move |()| {
        if is_publishing.get_untracked() {
            return;
        }
        let Some(pending) = pending_removal_confirmation.get_untracked() else {
            return;
        };
        let current_identity = RemovalConfirmationIdentity {
            account_npub: auth.npub.get_untracked(),
            publication: published_identity.get_untracked(),
            authoritative_state: authoritative_edit.get_untracked().map(|model| model.state),
        };
        if !removal_confirmation_is_fresh(
            &pending,
            &form_snapshot_for_confirmation(),
            &distribution_snapshot_for_confirmation(),
            &current_identity,
            removal_confirmation_generation.get_untracked(),
        ) {
            dismiss_removal_confirmation.run(());
            error_message.set(Some(
                "Something changed after the check, so publishing was cancelled. Publish again to review it."
                    .into(),
            ));
            return;
        }

        let confirmed =
            removal_confirmation_prepared(&pending.prepared, &pending.removed_profile_ids);
        let removed_profile_ids = pending.removed_profile_ids;
        pending_removal_confirmation.set(None);
        dispatch_prepared.run((
            confirmed,
            current_identity.account_npub.unwrap_or_default(),
            Some(removed_profile_ids),
        ));
    });

    on_cleanup(move || {
        if let Some(selection) = publish_draft.get_untracked().archive.selection {
            let expected_publisher_npub = publish_draft.get_untracked().archive.owner_npub;
            spawn_local(async move {
                if let Ok(request_id) = archive_request_id() {
                    let _ = invoke_discard_archive_selection(DiscardArchiveSelectionRequest {
                        request_id,
                        selection_id: selection.selection_id,
                        expected_publisher_npub,
                    })
                    .await;
                }
            });
        }
    });

    let checklist = move || {
        readiness_checklist(
            &title.get(),
            &description.get(),
            &arcadestr_core::store_page_seed::listing_images_from_media(
                &store_page_draft.get().content.media,
            )
            .join(","),
            acquisition_kind.get(),
            &price_input.get(),
            &lud16.get(),
            acquisition_policy_from_form(
                acquisition_kind.get(),
                &acquisition_starts_at.get(),
                &acquisition_ends_at.get(),
            ),
            &platforms_input.get(),
            fulfillment_enabled.get(),
            &servers.get(),
            file_hash.get().as_deref(),
            &version.get(),
            &fulfillment_mode.get(),
            &operator_url.get(),
        )
    };

    let initial_form_snapshot = StoredValue::new(form_snapshot());
    let initial_distribution_snapshot = RwSignal::new(distribution_snapshot());
    let authority_rebased = RwSignal::new(!editing);
    let distribution_snapshot_for_baseline = distribution_snapshot.clone();
    Effect::new(move |_| {
        if authority_rebased.get() {
            return;
        }
        let authority_ready = authoritative_edit.get().is_some_and(|model| {
            matches!(
                model.state,
                AuthoritativeEditState::Absent
                    | AuthoritativeEditState::PresentValid(_)
                    | AuthoritativeEditState::PresentDefective(_)
            )
        });
        if should_rebase_edit_distribution(
            authority_ready,
            operator_initialized.get(),
            distribution_user_revision.get(),
        ) {
            initial_distribution_snapshot.set(distribution_snapshot_for_baseline());
            authority_rebased.set(true);
        }
    });
    let distribution_snapshot_for_dirty = distribution_snapshot.clone();
    let form_snapshot_for_scope = form_snapshot.clone();
    let distribution_snapshot_for_scope = distribution_snapshot.clone();
    let publish_scope = move || {
        let images = |page: StorePageDraft| {
            arcadestr_core::store_page_seed::listing_images_from_media(&page.content.media)
        };
        let synced = |page: StorePageDraft| {
            synced_store_page(page, &title.get(), &description.get(), &tag_input.get())
        };
        let current = store_page_draft.get();
        let baseline = store_page_baseline.get();
        update_scope(
            editing,
            authority_rebased.get(),
            form_snapshot_for_scope() != initial_form_snapshot.get_value(),
            distribution_snapshot_for_scope() != initial_distribution_snapshot.get(),
            images(current.clone()) != images(baseline.clone()),
            synced(current) != synced(baseline),
            store_page_linked.get(),
        )
    };
    let is_dirty = move || {
        form_snapshot() != initial_form_snapshot.get_value()
            || distribution_snapshot_for_dirty() != initial_distribution_snapshot.get()
            || store_page_draft.get() != store_page_baseline.get()
            || store_page_input_dirty.get()
    };

    // Restore the saved draft once the published state it edits has loaded, so
    // a late Store page or launch-metadata load cannot overwrite restored edits.
    let restore_started = StoredValue::new(false);
    Effect::new(move |_| {
        if restore_started.get_value() {
            return;
        }
        let page_loaded = store_page_status.get().is_some();
        let authority_settled = !editing
            || (operator_initialized.get()
                && authoritative_edit
                    .get()
                    .is_some_and(|model| !matches!(model.state, AuthoritativeEditState::Loading)));
        if page_loaded && authority_settled {
            restore_started.set_value(true);
            on_restore_draft.run(());
        }
    });

    // Autosave: a short pause after the last edit writes the local draft.
    let autosave_generation = StoredValue::new(0_u64);
    let autosave_build = build_durable_draft.clone();
    let is_dirty_for_autosave = is_dirty.clone();
    Effect::new(move |_| {
        autosave_tick.track();
        let fingerprint = serde_json::to_value(autosave_build())
            .ok()
            .map(|value| value.to_string());
        let wanted = draft_ready.get()
            && !is_publishing.get()
            && (is_dirty_for_autosave() || draft_revision.get_untracked().is_some())
            && fingerprint.is_some()
            && fingerprint != last_saved_fingerprint.get_untracked();
        if !wanted || draft_save_status.get_untracked() == DraftSaveStatus::Conflict {
            return;
        }
        autosave_generation.update_value(|value| *value = value.wrapping_add(1));
        let generation = autosave_generation.get_value();
        draft_save_status.set(DraftSaveStatus::Pending);
        after_delay(800, move || {
            if autosave_generation.try_get_value() == Some(generation) {
                on_save_draft.run(());
            }
        });
    });

    // Leaving is only guarded while edits are not yet in the local draft.
    Effect::new(move |_| {
        crate::ui_v2::components::set_publish_game_dirty(
            is_publishing.get() || draft_save_status.get().unsaved(),
        );
    });
    on_cleanup(|| crate::ui_v2::components::set_publish_game_dirty(false));

    let signer_state = Signal::derive(move || {
        let publisher = auth.npub.get();
        publisher_signer_state(
            publisher.as_deref(),
            auth.active_account.get().as_ref(),
            &auth.connection_status.get(),
        )
    });
    let signed_in = Signal::derive(move || auth.npub.get().is_some());
    let can_publish_now = Signal::derive(move || {
        signed_in.get() && signer_can_publish(signer_state.get()) && blocking_issues().is_empty()
    });
    let is_dirty_for_authoring = is_dirty.clone();
    let authoring =
        move || authoring_state(editing, is_dirty_for_authoring(), blocking_issues().len());
    let phase = move || {
        publication_phase(
            is_publishing.get(),
            publication_account_stale.get(),
            publication_state.get().outcome,
        )
    };
    // Store Page field groups rendered inside a step, as plain sections of it.
    let store_page_step = move |groups: &'static [&'static str]| {
        view! {
            {move || store_page_note.get().filter(|_| groups.contains(&"basic") || groups.contains(&"media")).map(|note| view! { <p class="v2-create-inline-note" role="status">{note}</p> })}
            {move || store_page_form_error.get().map(|error| view! { <p class="v2-create-inline-note" role="status">{error}</p> })}
            {move || match store_page_status.get() {
                None => view! { <p class="v2-create-help" role="status">"Loading the store page…"</p> }.into_any(),
                Some(Err(error)) => view! {
                    <div role="alert" class="v2-create-alert v2-create-alert-error">
                        <p>{format!("The store page didn't load: {error}")}</p>
                        <button type="button" class="v2-btn-secondary" on:click=move |_| load_store_page.run(())>"Retry"</button>
                    </div>
                }.into_any(),
                Some(Ok(())) => match auth.npub.get() {
                    None => view! { <p class="v2-create-help">"Sign in to edit the store page."</p> }.into_any(),
                    Some(npub) => {
                        let hex = publisher_hex(&npub).unwrap_or_default();
                        let check_hex = hex.clone();
                        let check_store_page = Callback::new(move |_: ()| {
                            let page = synced_store_page(
                                store_page_draft.get_untracked(),
                                &title.get_untracked(),
                                &description.get_untracked(),
                                &tag_input.get_untracked(),
                            );
                            let problems = store_page_problems(&page, &check_hex);
                            store_page_form_error.set(Some(if problems.is_empty() {
                                "No problems found.".to_string()
                            } else {
                                format!("Found {} problem(s) to fix.", problems.len())
                            }));
                            store_page_diagnostics.set(problems);
                        });
                        let platforms = Signal::derive(move || parse_platform_tags(&platforms_input.get()).unwrap_or_default());
                        view! {
                            <StorePageContentEditor
                                draft=store_page_draft
                                input_dirty=store_page_input_dirty
                                active_tab=store_page_tab
                                readiness_state=Signal::derive(move || embedded_readiness(&store_page_draft.get(), &platforms.get(), &store_page_diagnostics.get()))
                                linked_platforms=platforms
                                listing_platforms=platforms
                                disabled=Signal::derive(move || is_publishing.get())
                                diagnostics=store_page_diagnostics
                                form_error=store_page_form_error
                                upload_context_generation=store_page_upload_generation
                                listing_publisher_npub=npub
                                publisher_hex=hex
                                on_preview=check_store_page
                                embedded=true
                                sections=groups
                            />
                        }.into_any()
                    }
                },
            }}
        }
    };
    let header_title = move || {
        let value = title.get();
        if value.trim().is_empty() {
            "Untitled game".to_string()
        } else {
            value.trim().to_string()
        }
    };
    let launchable = move || matches!(publish_draft.get().launch, LaunchMode::Launchable(_));
    let current_delivery = move || {
        delivery_choice(
            delivery_chosen.get(),
            fulfillment_enabled.get(),
            launchable(),
        )
    };
    // A published distribution authorization can't be dropped by an ordinary
    // update, so a live game with a build can't switch to store-page-only.
    let delivery_option_locked = move |choice: DeliveryChoice| {
        existing_fulfillment_locked && choice.uses_build() != fulfillment_enabled.get()
    };
    let choose_delivery = move |choice: DeliveryChoice| {
        if is_publishing.get_untracked() || delivery_option_locked(choice) {
            return;
        }
        delivery_chosen.set(true);
        let mut enabled = fulfillment_enabled.get_untracked();
        let mut mode = fulfillment_mode.get_untracked();
        let mut state = publish_draft.get_untracked();
        let platforms = platforms_input.get_untracked();
        let (distribution_changed, launch_changed) =
            apply_delivery_choice(choice, &mut enabled, &mut mode, &mut state, &platforms);
        if distribution_changed {
            fulfillment_enabled.set(enabled);
            fulfillment_mode.set(mode);
            distribution_user_revision.update(|revision| *revision = revision.wrapping_add(1));
        }
        if launch_changed {
            publish_draft.set(state);
            distribution_user_revision.update(|revision| *revision = revision.wrapping_add(1));
            if choice != DeliveryChoice::Install {
                platform_user_revision.update(|revision| *revision = revision.wrapping_add(1));
            }
        }
    };
    let section_summary = move |section: HubSection| -> String {
        match section {
            HubSection::Overview => String::new(),
            HubSection::Basics => {
                let tags = parse_csv_values(&tag_input.get()).len();
                format!(
                    "{} · {}",
                    header_title(),
                    match tags {
                        0 => "no tags".to_string(),
                        1 => "1 tag".to_string(),
                        count => format!("{count} tags"),
                    }
                )
            }
            HubSection::StorePage => {
                let page = store_page_draft.get();
                let media = page.content.media.len();
                let has_description = !page.content.description_markdown.trim().is_empty();
                format!(
                    "{} · {}",
                    if has_description {
                        "Description written"
                    } else {
                        "No description yet"
                    },
                    match media {
                        0 => "no images or videos".to_string(),
                        1 => "1 image or video".to_string(),
                        count => format!("{count} images and videos"),
                    }
                )
            }
            HubSection::PriceAccess => match acquisition_kind.get() {
                AcquisitionKind::Gated => {
                    let price = price_input.get();
                    if price.trim().is_empty() {
                        "Paid · no price set yet".to_string()
                    } else {
                        format!("Paid · {} sats", price.trim())
                    }
                }
                AcquisitionKind::Public => "Free".to_string(),
                AcquisitionKind::TimedAccess => "Free for a limited time".to_string(),
            },
            HubSection::Delivery => match current_delivery() {
                None => "Not chosen yet".to_string(),
                Some(choice) if choice.uses_build() => {
                    let version = version.get();
                    let build = if publish_draft.get().archive.selection.is_some() {
                        "new build selected"
                    } else if file_hash.get().is_some() {
                        "current build"
                    } else {
                        "no build yet"
                    };
                    if version.trim().is_empty() {
                        format!("{} · {build}", choice.title())
                    } else {
                        format!("{} · v{} · {build}", choice.title(), version.trim())
                    }
                }
                Some(choice) => choice.title().to_string(),
            },
        }
    };
    let is_dirty_for_button = is_dirty.clone();
    let is_dirty_for_hint = is_dirty.clone();
    let publish_disabled = Signal::derive(move || {
        is_hashing.get()
            || is_publishing.get()
            || !can_publish_now.get()
            // A live game with nothing new to publish has nothing to update,
            // unless an earlier attempt needs finishing.
            || (editing
                && !is_dirty_for_button()
                && matches!(phase(), PublicationPhase::Idle | PublicationPhase::Published))
    });
    let publish_label = move || {
        if is_publishing.get() {
            "Publishing…"
        } else if editing {
            "Publish changes"
        } else {
            "Publish game"
        }
    };
    // Publishing always goes through a buyer's-view preview first.
    let preview_open = RwSignal::new(false);
    let preview_auth = auth.clone();
    let buyer_preview = move || -> Result<
        (
            crate::models::GameDetailPresentation,
            Option<(GameListing, crate::models::StorePageCardPresentation)>,
            String,
        ),
        String,
    > {
        let npub = preview_auth
            .npub
            .get()
            .ok_or_else(|| "Sign in to preview your game.".to_string())?;
        let hex = publisher_hex(&npub).ok_or_else(|| "The active publisher key is invalid.".to_string())?;
        let platforms = parse_platform_tags(&platforms_input.get()).unwrap_or_default();
        let page = synced_store_page(
            store_page_draft.get(),
            &title.get(),
            &description.get(),
            &tag_input.get(),
        );
        let presentation = buyer_preview_page(&page, &hex, &platforms)?;
        let kind = acquisition_kind.get();
        let price = price_for_acquisition(kind, &price_input.get(), &lud16.get()).unwrap_or(0);
        let policy = acquisition_policy_from_form(
            kind,
            &acquisition_starts_at.get(),
            &acquisition_ends_at.get(),
        )
        .unwrap_or_default();
        let price_text = match kind {
            AcquisitionKind::Gated => format!("{price} sats · players pay once and keep it"),
            AcquisitionKind::Public => "Free".to_string(),
            AcquisitionKind::TimedAccess => {
                format!("Free from {} to {} (your local time)", acquisition_starts_at.get(), acquisition_ends_at.get())
            }
        };
        let card = buyer_preview_card(
            &npub,
            &title.get(),
            &description.get(),
            price,
            &policy,
            &presentation,
            arcadestr_core::store_page_seed::listing_images_from_media(&page.content.media),
            &platforms,
        );
        Ok((presentation, card, price_text))
    };
    // A live game whose only changes are on its store page publishes just the
    // store page: the game, its build and its distribution stay as they are.
    let store_page_publish_auth = auth.clone();
    let publish_store_page_only = Callback::new(move |()| {
        if !can_dispatch(is_hashing.get_untracked(), is_publishing.get_untracked()) {
            return;
        }
        let auth = store_page_publish_auth.clone();
        let Some(npub) = auth.npub.get_untracked() else {
            error_message.set(Some("Sign in before publishing.".into()));
            return;
        };
        if durable_draft_owner.get_value().as_deref() != Some(npub.as_str()) {
            error_message.set(Some(
                "Switch to the account that published this game before updating it.".into(),
            ));
            return;
        }
        let Some(hex) = publisher_hex(&npub) else {
            error_message.set(Some("The active publisher key is invalid.".into()));
            return;
        };
        let page = synced_store_page(
            store_page_draft.get_untracked(),
            &title.get_untracked(),
            &description.get_untracked(),
            &tag_input.get_untracked(),
        );
        if let Some(problem) = store_page_problems(&page, &hex).into_iter().next() {
            let tab = crate::ui_v2::views::store_page_publish::diagnostic_tab(&problem);
            store_page_tab.set(tab);
            current_section.set(HubSection::StorePage);
            error_message.set(Some(format!("Store page: {problem}")));
            return;
        }
        let epoch = auth.current_auth_flow();
        is_publishing.set(true);
        initiating_account.set(Some(npub.clone()));
        publication_account_stale.set(false);
        error_message.set(None);
        progress_events.set(Vec::new());
        upload_progress.set(None);
        publication_state.set(PublicationState {
            outcome: PublicationOutcome::Publishing,
            ..PublicationState::default()
        });
        spawn_local(async move {
            let result = crate::tauri_bridge::invoke_publish_store_page(
                npub.clone(),
                page.clone(),
                Vec::new(),
            )
            .await;
            if !auth.is_current_auth_flow(epoch)
                || !publication_account_matches(&npub, auth.npub.get_untracked().as_deref())
            {
                let _ = is_publishing.try_set(false);
                let _ = initiating_account.try_set(None);
                let _ = publication_account_stale.try_set(true);
                return;
            }
            let _ = is_publishing.try_set(false);
            let _ = initiating_account.try_set(None);
            recovery_refresh.update(|value| *value = value.wrapping_add(1));
            let published_page = |event_id: &str| {
                let mut next = page.clone();
                next.loaded_event_id = Some(event_id.to_string());
                next
            };
            match result {
                Ok(response) => {
                    // Later edits revise the revision that was just published.
                    if let Some(outcome) = response.store_page.as_ref() {
                        let next = published_page(&outcome.event_id);
                        store_page_draft.set(next.clone());
                        store_page_baseline.set(next);
                    }
                    if response.complete {
                        publication_state.set(PublicationState {
                            outcome: PublicationOutcome::Complete,
                            listing_published: true,
                            message: Some(
                                "Your store page is updated. The rest of your game didn't change."
                                    .into(),
                            ),
                        });
                        if let Some(revision) = draft_revision.get_untracked() {
                            let draft_id = durable_draft_id.get_value();
                            spawn_local(async move {
                                let _ = crate::tauri_bridge::invoke_delete_publisher_draft(
                                    npub,
                                    draft_id,
                                    "listing".into(),
                                    revision,
                                )
                                .await;
                            });
                        }
                        draft_revision.set(None);
                        last_saved_fingerprint.set(None);
                        draft_save_status.set(DraftSaveStatus::Idle);
                    } else {
                        publication_state.set(PublicationState {
                            outcome: PublicationOutcome::Partial,
                            listing_published: true,
                            message: Some(
                                "Your store page was published but hasn't reached every server yet. Select Finish publishing below to retry."
                                    .into(),
                            ),
                        });
                    }
                }
                Err(error) => publication_state.set(PublicationState {
                    outcome: PublicationOutcome::Failed,
                    listing_published: false,
                    message: Some(error),
                }),
            }
        });
    });
    let publish_scope_for_confirm = publish_scope.clone();
    let publish_scope_for_preview = publish_scope.clone();
    let confirm_publish = Callback::new(move |()| {
        preview_open.set(false);
        match untrack(|| publish_scope_for_confirm()) {
            UpdateScope::StorePageOnly => publish_store_page_only.run(()),
            UpdateScope::FullGame => on_submit.run(()),
        }
    });
    let leave = move || {
        if let Some(on_back) = on_back {
            on_back.run(());
        }
    };
    let section_header = move |section: HubSection| {
        let status = move || {
            section_status(
                &validate_section(section),
                section_suggestions(section).len(),
            )
        };
        view! {
            <header class="v2-hub-section-header">
                <div>
                    <h2 id=format!("{}-title", section.dom_id()) class="v2-hub-section-title">{section.title()}</h2>
                    <p class="v2-create-help">{section.description()}</p>
                </div>
                <span class=move || format!("v2-hub-section-status {}", status().class())>{move || status().label()}</span>
            </header>
            {move || validate_section(section).err().map(|issue| view! {
                <p class="v2-hub-section-issue" role="status">{issue}</p>
            })}
            {move || {
                let suggestions = section_suggestions(section);
                (validate_section(section).is_ok() && !suggestions.is_empty()).then(|| view! {
                    <div class="v2-hub-section-suggestions" role="note">
                        <p>"Optional, but these help players decide:"</p>
                        <ul>{suggestions.into_iter().map(|item| view! { <li>{item}</li> }).collect_view()}</ul>
                    </div>
                })
            }}
        }
    };

    view! {
        <main class="v2-publisher-studio v2-create-workflow v2-game-hub">
            <nav class="v2-hub-breadcrumb" aria-label="Breadcrumb">
                <ol>
                    <li><button type="button" class="v2-hub-crumb" disabled=move || !can_exit on:click=move |_| leave()>"Publish"</button></li>
                    <li>
                        <button
                            type="button"
                            class="v2-hub-crumb"
                            aria-current=move || (current_section.get() == HubSection::Overview).then_some("page")
                            on:click=move |_| open_section(HubSection::Overview)
                        >{header_title}</button>
                    </li>
                    {move || (current_section.get() != HubSection::Overview).then(|| view! {
                        <li><span class="v2-hub-crumb-current" aria-current="page">{current_section.get().title()}</span></li>
                    })}
                </ol>
            </nav>

            <header class="v2-hub-header">
                <div class="v2-hub-identity">
                    <p class="v2-publisher-kicker">{if editing { "Live game" } else { "Draft game" }}</p>
                    <h1 class="v2-create-title">{header_title}</h1>
                    <div class="v2-hub-header-status">
                        {move || {
                            let state = authoring();
                            view! { <StatusChip label=state.label() variant=state.chip() icon=None size=StatusChipSize::Compact /> }
                        }}
                        <span class="v2-hub-save-status" role="status" aria-live="polite">{move || draft_save_status.get().label()}</span>
                        {move || matches!(draft_save_status.get(), DraftSaveStatus::Failed(_)).then(|| view! {
                            <button type="button" class="v2-hub-link" on:click=move |_| on_save_draft.run(())>"Retry"</button>
                        })}
                        {move || (draft_save_status.get() == DraftSaveStatus::Conflict).then(|| view! {
                            <button type="button" class="v2-hub-link" on:click=move |_| {
                                draft_save_status.set(DraftSaveStatus::Idle);
                                on_restore_draft.run(());
                            }>"Reload draft"</button>
                        })}
                    </div>
                </div>
                <div class="v2-hub-header-actions">
                    <button type="button" class="v2-btn-primary" aria-describedby="publish-signer-requirement" on:click=move |_| preview_open.set(true) disabled=move || publish_disabled.get()>
                        {publish_label}
                    </button>
                    <p class="v2-create-help" id="publish-signer-requirement">{move || {
                        let issues = blocking_issues().len();
                        if !signed_in.get() {
                            "Sign in to publish.".to_string()
                        } else if !signer_can_publish(signer_state.get()) {
                            signer_requirement_message(signer_state.get()).to_string()
                        } else if issues > 0 {
                            if issues == 1 { "1 thing to finish before publishing.".to_string() } else { format!("{issues} things to finish before publishing.") }
                        } else if editing && !is_dirty_for_hint() {
                            "No unpublished changes.".to_string()
                        } else {
                            signer_requirement_message(signer_state.get()).to_string()
                        }
                    }}</p>
                </div>
            </header>

            {move || draft_message.get().map(|message| view! {
                <p class="v2-create-inline-note" role="status">
                    {message}
                    " "
                    <button type="button" class="v2-hub-link" on:click=move |_| draft_message.set(None)>"Dismiss"</button>
                </p>
            })}
            {move || error_message.get().map(|msg| view! { <div role="alert" class="v2-create-alert v2-create-alert-error">{msg}</div> })}

            <Show when=move || phase() != PublicationPhase::Idle>
            <section class="v2-hub-card v2-hub-publication" aria-labelledby="publication-status-title">
                <div class="v2-create-status-heading">
                    <h2 id="publication-status-title" class="v2-create-section-title">"Publishing"</h2>
                    <span role="status" aria-live="polite" aria-atomic="true" class=move || format!("v2-create-phase {}", phase().class())>{move || phase().label()}</span>
                </div>
                <p class="v2-create-help">{move || match phase() {
                    PublicationPhase::Idle => "",
                    PublicationPhase::Publishing => "Publishing your game. Keep Arcadestr open until this finishes.",
                    PublicationPhase::Partial => "Your game is live, but some of the remaining work didn't finish. Use Finish publishing below.",
                    PublicationPhase::Published => "Your game is live.",
                    PublicationPhase::Failed => "Nothing was published. Fix the problem below and try again.",
                    PublicationPhase::StaleAccount => "You switched accounts while publishing, so the result was ignored. Switch back to check whether the game was published.",
                }}</p>
                <div aria-live="polite" aria-atomic="true">
                {move || publication_state.get().message.map(|msg| {
                    let class = match publication_state.get().outcome { PublicationOutcome::Complete => "v2-create-status-ok", PublicationOutcome::Partial | PublicationOutcome::Failed => "v2-create-status-error", _ => "v2-create-status-neutral" };
                    view! { <p class={class}>{msg}</p> }
                })}
                {move || upload_progress.get().and_then(|event| {
                    let bytes_uploaded = event.bytes_uploaded?;
                    let total_bytes = event.total_bytes?;
                    let percent = progress_percent(bytes_uploaded, total_bytes);
                    let server = event.server_url.unwrap_or_else(|| "the upload server".to_string());
                    Some(view! {
                        <div class="v2-create-progress" role="progressbar" aria-label="Build upload progress" aria-valuemin="0" aria-valuemax="100" aria-valuenow=percent>
                            <div class="v2-create-progress-heading">
                                <span>{format!("Uploading your build to {server}")}</span>
                                <span>{format!("{percent}%")}</span>
                            </div>
                            <div class="v2-create-progress-track">
                                <div class="v2-create-progress-fill" style=format!("width: {percent}%")></div>
                            </div>
                        </div>
                    })
                })}
                <ul class="v2-create-progress-log">
                    {move || progress_events.get().into_iter().map(|event| view! {
                        <li>{progress_label(&event)}</li>
                    }).collect_view()}
                </ul>
                </div>
            </section>
            </Show>

            <crate::components::publisher_recovery::PublisherRecoveryPanel
                game_d_tag=Signal::derive(move || {
                    let identity = published_identity.get();
                    identity.d_tag.or(identity.pending_d_tag)
                })
                refresh_key=Signal::derive(move || recovery_refresh.get())
                current_build=Signal::derive(move || publish_draft.get().archive.selected_build)
                on_finished=Callback::new(move |()| {
                    publication_state.set(PublicationState::default());
                    error_message.set(None);
                    draft_message.set(Some(if editing {
                        "Publishing finished. Go back to Your games and open the game again to see its latest version.".into()
                    } else {
                        "Publishing finished. Your game is live; find it under Your games.".into()
                    }));
                })
            />

            <Dialog
                id="publish-buyer-preview"
                open=Signal::derive(move || preview_open.get())
                title=Signal::derive(move || format!("Preview · {}", header_title()))
                kicker="Before publishing"
                description=Signal::derive(move || if editing {
                    "This is how your game will look to buyers after these changes. Nothing is published until you confirm.".to_string()
                } else {
                    "This is how your game will look to buyers. Nothing is published until you confirm.".to_string()
                })
                width=DialogWidth::Wide
                policy=DialogClosePolicy::Dismissible
                dismissal=DialogDismissal::keyboard_and_button()
                initial_focus=DialogInitialFocus::CloseButton
                close_label="Keep editing"
                on_close=UnsyncCallback::new(move |_: DialogCloseRequest| preview_open.set(false))
                actions=move || view! {
                    <Button
                        variant=ButtonVariant::Secondary
                        on_click=Callback::new(move |_| preview_open.set(false))
                    >"Keep editing"</Button>
                    <Button
                        variant=ButtonVariant::Primary
                        disabled=Signal::derive(move || publish_disabled.get() || buyer_preview().is_err())
                        on_click=Callback::new(move |_| confirm_publish.run(()))
                    >{move || if editing { "Confirm and publish changes" } else { "Confirm and publish" }}</Button>
                }
            >
                {move || preview_open.get().then(|| match buyer_preview() {
                    Err(problem) => view! {
                        <div role="alert" class="v2-create-alert v2-create-alert-error">
                            <p>{format!("The store page can't be previewed yet: {problem}")}</p>
                            <p>"Fix it in the Store page section, then preview again."</p>
                        </div>
                    }.into_any(),
                    Ok((presentation, card, price_text)) => {
                        let delivery = match current_delivery() {
                            Some(DeliveryChoice::StorePageOnly) | None => "Store page only: players can't get the game until you add a build.".to_string(),
                            Some(choice) => {
                                let version = version.get();
                                if version.trim().is_empty() { choice.title().to_string() } else { format!("{} · version {}", choice.title(), version.trim()) }
                            }
                        };
                        let now = (js_sys::Date::now() / 1000.0) as u64;
                        view! {
                            <div class="v2-buyer-preview" data-testid="buyer-preview">
                                <section class="v2-buyer-preview-card" aria-labelledby="buyer-preview-card-title">
                                    <h3 id="buyer-preview-card-title" class="v2-hub-kicker">"Game card"</h3>
                                    <p class="v2-create-help">"Shown in the store, search and the library."</p>
                                    {card.map(|(listing, store_page)| {
                                        let presentation = GameCardPresentation::from_listing(&listing, now, PlatformCompatibility::Unknown, false);
                                        view! {
                                            <div class="v2-buyer-preview-card-frame">
                                                <GameCard listing=listing presentation=presentation on_open=Callback::new(|_| {}) store_page=Some(store_page) />
                                            </div>
                                        }
                                    })}
                                    <dl class="v2-create-review-facts">
                                        <div><dt>"Price & access"</dt><dd>{price_text}</dd></div>
                                        <div><dt>"How players get it"</dt><dd>{delivery}</dd></div>
                                        {editing.then(|| view! {
                                            <div><dt>"What will be published"</dt><dd data-testid="publish-scope">{match publish_scope_for_preview() {
                                                UpdateScope::StorePageOnly => "Only your store page. The price, build and game details stay as they are.",
                                                UpdateScope::FullGame => "Your game and its store page.",
                                            }}</dd></div>
                                        })}
                                    </dl>
                                </section>
                                <section class="v2-buyer-preview-page" aria-labelledby="buyer-preview-page-title">
                                    <h3 id="buyer-preview-page-title" class="v2-hub-kicker">"Store page"</h3>
                                    <h2 class="v2-hub-section-title">{presentation.title.clone().unwrap_or_else(header_title)}</h2>
                                    {presentation.summary.clone().map(|summary| view! { <p class="v2-create-help">{summary}</p> })}
                                    <StorePageRichDetail presentation=presentation preview=true />
                                </section>
                            </div>
                        }.into_any()
                    }
                })}
            </Dialog>

            <Dialog
                id="publish-profile-removal-confirmation"
                open=Signal::derive(move || pending_removal_confirmation.get().is_some())
                title=Signal::derive(move || pending_removal_confirmation.get().map(|pending| {
                    (if pending.abort_count > 1 {
                        "Removed launch options changed — confirm again"
                    } else {
                        "Remove launch options?"
                    })
                    .to_string()
                }).unwrap_or_default())
                kicker="Before publishing"
                description=Signal::derive(move || pending_removal_confirmation.get().map(|pending| removal_confirmation_copy(&pending.removed_profile_ids, pending.abort_count)).unwrap_or_default())
                width=DialogWidth::Compact
                tone=DialogTone::Destructive
                policy=DialogClosePolicy::Dismissible
                dismissal=DialogDismissal::keyboard_and_button()
                initial_focus=DialogInitialFocus::CloseButton
                close_label="Keep editing"
                on_close=UnsyncCallback::new(move |request: DialogCloseRequest| {
                    if request.action == DialogCloseAction::Dismiss {
                        dismiss_removal_confirmation.run(());
                    }
                })
                actions=move || view! {
                    <Button
                        variant=ButtonVariant::Secondary
                        on_click=Callback::new(move |_| dismiss_removal_confirmation.run(()))
                    >"Keep editing"</Button>
                    <Button
                        variant=ButtonVariant::Danger
                        disabled=Signal::derive(move || is_publishing.get())
                        on_click=Callback::new(move |_| confirm_profile_removal.run(()))
                    >"Remove and publish"</Button>
                }
            >
                <div role="alert" class="v2-create-alert v2-create-alert-error">
                    <p>"This update removes launch options players may already use. Nothing has been published yet. Players who chose one of these will switch to the default launch option:"</p>
                    <ul>
                        {move || pending_removal_confirmation.get().map(|pending| pending.removed_profile_ids.into_iter().map(|id| view! {
                            <li><strong>{id.as_str().to_string()}</strong></li>
                        }).collect_view())}
                    </ul>
                    <p>"Closing this dialog cancels publishing. If you change anything else, you'll be asked again."</p>
                </div>
            </Dialog>

            <div class="v2-create-surface">
                    <Show when=move || current_section.get() == HubSection::Overview>
                    <section class="v2-hub-overview" aria-labelledby="game-hub-checklist-title">
                        <h2 id="game-hub-checklist-title" class="v2-hub-kicker">{if editing { "Your game" } else { "Before you publish" }}</h2>
                        <ol class="v2-hub-checklist">
                            {HubSection::EDITABLE.into_iter().map(|section| {
                                let status = move || section_status(&validate_section(section), section_suggestions(section).len());
                                view! {
                                    <li class=move || format!("v2-hub-row {}", status().class())>
                                        <button type="button" class="v2-hub-row-button" on:click=move |_| open_section(section)>
                                            <span class="v2-hub-row-icon" aria-hidden="true">{move || status().icon()}</span>
                                            <span class="v2-hub-row-body">
                                                <span class="v2-hub-row-title">{section.title()}</span>
                                                <span class="v2-hub-row-summary">{move || section_summary(section)}</span>
                                                {move || match validate_section(section) {
                                                    Err(issue) => Some(view! { <span class="v2-hub-row-issue">{issue}</span> }.into_any()),
                                                    Ok(()) => {
                                                        let suggestions = section_suggestions(section);
                                                        suggestions.first().map(|first| view! {
                                                            <span class="v2-hub-row-suggestion">{match suggestions.len() {
                                                                1 => format!("Suggestion: {first}"),
                                                                count => format!("{count} suggestions · {first}"),
                                                            }}</span>
                                                        }.into_any())
                                                    }
                                                }}
                                            </span>
                                            <span class="v2-hub-row-status">{move || status().label()}</span>
                                            <span class="v2-hub-row-action" aria-hidden="true">{move || status().action()}</span>
                                        </button>
                                    </li>
                                }
                            }).collect_view()}
                        </ol>
                    </section>
                    {overview_extra.clone().map(|extra| extra.run())}
                    </Show>

                    <Show when=move || current_section.get() == HubSection::Basics>
                    <section class="v2-create-stage-panel" id=HubSection::Basics.dom_id() aria-labelledby="game-hub-basics-title">
                        {section_header(HubSection::Basics)}
                        <div class="v2-create-fields">
                            <div>
                                <label for="publish-title" class="v2-create-field-label">"Title (required)"</label>
                                <input id="publish-title" required=true class="v2-input" placeholder="Neon Drifter" prop:value={move || title.get()} on:input:target=move |ev| title.set(ev.target().value()) disabled={move || is_publishing.get()} />
                            </div>
                            <div>
                                <label for="publish-description" class="v2-create-field-label">"Summary (required)"</label>
                                <textarea id="publish-description" required=true class="v2-input" rows=3 aria-describedby="publish-description-help" placeholder="One or two sentences about your game." prop:value={move || description.get()} on:input:target=move |ev| description.set(ev.target().value()) disabled={move || is_publishing.get()} />
                                <p id="publish-description-help" class="v2-create-help">"Shown on game cards and at the top of your store page."</p>
                            </div>
                            <div>
                                    <label for="publish-tag-choice" class="v2-create-field-label">"Tags"</label>
                                    <select id="publish-tag-choice" aria-describedby="publish-tags-help" class="v2-input" prop:value=move || tag_choice.get() on:change:target=move |ev| {
                                        let selected = ev.target().value();
                                        tag_input.update(|input| *input = add_game_tag(input, &selected));
                                        tag_choice.set(String::new());
                                    } disabled=move || is_publishing.get()>
                                        <option value="">"Add a tag..."</option>
                                        {AVAILABLE_GAME_TAGS.into_iter().map(|tag| view! {
                                            <option value=tag disabled=move || parse_csv_values(&tag_input.get()).iter().any(|selected| selected.eq_ignore_ascii_case(tag))>{tag}</option>
                                        }).collect_view()}
                                    </select>
                                    <div class="v2-create-chip-row">
                                        {move || parse_csv_values(&tag_input.get()).into_iter().map(|tag| {
                                            let tag_to_remove = tag.clone();
                                            view! {
                                                <span class="v2-create-chip">
                                                    {tag}
                                                    <button type="button" class="v2-create-chip-remove" aria-label=format!("Remove {tag_to_remove}") on:click=move |_| tag_input.update(|input| *input = remove_game_tag(input, &tag_to_remove)) disabled=move || is_publishing.get()>"Remove"</button>
                                                </span>
                                            }
                                        }).collect_view()}
                                    </div>
                                    <label for="publish-tags" class="v2-create-field-label">"Custom tags"</label>
                                    <input id="publish-tags" aria-describedby="publish-tags-help" class="v2-input" placeholder="Add comma-separated custom tags" prop:value={move || tag_input.get()} on:input:target=move |ev| tag_input.set(ev.target().value()) disabled={move || is_publishing.get()} />
                                    <p id="publish-tags-help" class="v2-create-help">"Choose common tags above or enter additional comma-separated tags."</p>
                            </div>
                        </div>
                    </section>
                    {store_page_step(&["basic"])}
                    </Show>

                    <Show when=move || current_section.get() == HubSection::StorePage>
                    <section class="v2-create-stage-panel" id=HubSection::StorePage.dom_id() aria-labelledby="game-hub-store-page-title">
                        {section_header(HubSection::StorePage)}
                        <nav class="v2-hub-tabs" role="tablist" aria-label="Store page parts">
                            {STORE_PAGE_TABS.into_iter().map(|(id, label, _)| {
                                let blocked = move || {
                                    let platforms = parse_platform_tags(&platforms_input.get()).unwrap_or_default();
                                    embedded_readiness(&store_page_draft.get(), &platforms, &store_page_diagnostics.get())
                                        .blockers()
                                        .iter()
                                        .any(|blocker| crate::ui_v2::views::store_page_publish::diagnostic_tab(blocker) == id)
                                };
                                view! {
                                    <button
                                        type="button"
                                        role="tab"
                                        aria-selected=move || aria_bool(store_page_tab.get() == id)
                                        class=move || format!(
                                            "v2-hub-tab{}{}",
                                            if store_page_tab.get() == id { " v2-hub-tab-active" } else { "" },
                                            if blocked() { " v2-hub-tab-attention" } else { "" },
                                        )
                                        on:click=move |_| store_page_tab.set(id)
                                    >
                                        {label}
                                        {move || blocked().then(|| view! { <span class="v2-hub-tab-flag" aria-hidden="true">" !"</span><span class="sr-only">" needs attention"</span> })}
                                    </button>
                                }
                            }).collect_view()}
                        </nav>
                        {STORE_PAGE_TABS.into_iter().map(|(id, _, groups)| view! {
                            <Show when=move || store_page_tab.get() == id>
                                <div class="v2-create-store-step" role="tabpanel">{store_page_step(groups)}</div>
                            </Show>
                        }).collect_view()}
                        // Platforms are set with the build; say so where requirements are edited.
                        <Show when=move || store_page_tab.get() == "requirements">
                            <div class="v2-create-inline-note v2-create-row-between" role="note">
                                <span>{move || {
                                    let platforms = parse_platform_tags(&platforms_input.get()).unwrap_or_default();
                                    if platforms.is_empty() {
                                        "Platforms are set in Delivery & builds.".to_string()
                                    } else {
                                        format!("Platforms: {} · set in Delivery & builds.", platform_summary(&platforms_input.get()))
                                    }
                                }}</span>
                                <button type="button" class="v2-btn-secondary" on:click=move |_| open_section(HubSection::Delivery)>"Edit platforms >"</button>
                            </div>
                        </Show>
                    </section>
                    </Show>

                    <Show when=move || current_section.get() == HubSection::Delivery>
                    <section class="v2-create-stage-panel" id=HubSection::Delivery.dom_id() aria-labelledby="game-hub-delivery-title">
                        {section_header(HubSection::Delivery)}
                        <div class="v2-hub-choice-grid" role="radiogroup" aria-label="How players get your game">
                            {DeliveryChoice::ALL.into_iter().map(|choice| {
                                let selected = move || current_delivery() == Some(choice);
                                view! {
                                    <button
                                        type="button"
                                        role="radio"
                                        aria-checked=move || aria_bool(selected())
                                        class=move || if selected() { "v2-create-mode-option v2-create-mode-selected" } else { "v2-create-mode-option" }
                                        disabled=move || is_publishing.get() || delivery_option_locked(choice)
                                        on:click=move |_| choose_delivery(choice)
                                    >
                                        <span class="v2-create-row-between">
                                            <span class=move || if selected() { "v2-create-mode-title v2-create-mode-title-active" } else { "v2-create-mode-title" }>{choice.title()}</span>
                                            <Show when=selected>
                                                <span class="v2-create-selected-badge">"Selected"</span>
                                            </Show>
                                        </span>
                                        <span class="v2-create-help">{choice.description()}</span>
                                    </button>
                                }
                            }).collect_view()}
                        </div>
                        {existing_fulfillment_locked.then(|| view! { <p class="v2-create-help">"This game already has a distribution service authorized, so it can't switch back to store page only."</p> })}

                        <Show when=move || fulfillment_enabled.get() && delivery_chosen.get()>
                            <div class="v2-create-fields">
                                <h3 class="v2-create-subsection-title">"Your build"</h3>
                                <div class="v2-create-field-pair">
                                    <div>
                                        <span id="publish-build-label" class="v2-create-field-label">"Build file (required)"</span>
                                        <button type="button" aria-labelledby="publish-build-label" aria-describedby="publish-hash-status" class="v2-create-file-button" on:click={on_select_file} disabled={move || is_hashing.get() || is_publishing.get()}>
                                            {move || if is_hashing.get() { "Checking the file…".to_string() } else { publish_draft.get().archive.selection.map(|selection| selection.safe_filename.as_str().to_string()).unwrap_or_else(|| if file_hash.get().is_some() { "Current build · choose a new file".to_string() } else { "Choose file (ZIP, tar.gz or tar.zst)".to_string() }) }}
                                        </button>
                                        <p id="publish-hash-status" aria-live="polite" class="v2-create-help">{move || if is_hashing.get() { "Checking the archive's contents and fingerprint…".to_string() } else if file_hash.get().is_some() { "Build checked.".to_string() } else { "Choose the archive players will receive.".to_string() }}</p>
                                    </div>
                                    <div>
                                        <label for="publish-version" class="v2-create-field-label">"Version (required)"</label>
                                        <input id="publish-version" required=true class="v2-input" placeholder="1.0.0" prop:value={move || version.get()} on:input:target=move |ev| {
                                            version.set(ev.target().value());
                                            distribution_user_revision.update(|revision| *revision = revision.wrapping_add(1));
                                        } />
                                    </div>
                                </div>
                            </div>
                        </Show>

                        <Show when=move || delivery_chosen.get()>
                            <super::publish_launch::PublishLaunchEditor
                                state=publish_draft
                                authoritative=authoritative_edit
                                defect_review=defect_review
                                platforms_input=platforms_input
                                distribution_revision=distribution_user_revision
                                platform_revision=platform_user_revision
                                surfaced_warnings=surfaced_archive_warnings
                                disabled=Signal::derive(move || is_publishing.get() || is_hashing.get() || (editing && !operator_initialized.get()))
                            />
                        </Show>

                        <Show when=move || fulfillment_enabled.get() && delivery_chosen.get()>
                            <div class="v2-create-fields">
                                <div>
                                    <h3 class="v2-create-subsection-title">"Distribution service"</h3>
                                    <p class="v2-create-help">"A distribution service stores your build and gives copies to players who have access to the game."</p>
                                </div>
                                <button type="button" aria-pressed=move || aria_bool(fulfillment_mode.get() == FulfillmentMode::Delegate) class=move || if fulfillment_mode.get() == FulfillmentMode::Delegate {
                                    "v2-create-mode-option v2-create-mode-selected"
                                } else {
                                    "v2-create-mode-option"
                                } on:click=move |_| {
                                    if fulfillment_mode.get_untracked() != FulfillmentMode::Delegate {
                                         fulfillment_mode.set(FulfillmentMode::Delegate);
                                         distribution_user_revision.update(|revision| *revision = revision.wrapping_add(1));
                                        if operator_auto_added.get_untracked().is_none() {
                                            sync_operator_server(operator_url.get_untracked());
                                        }
                                    }
                                }>
                                    <span class="v2-create-row-between">
                                        <span class=move || if fulfillment_mode.get() == FulfillmentMode::Delegate { "v2-create-mode-title v2-create-mode-title-active" } else { "v2-create-mode-title" }>"Use a distribution service"</span>
                                        <span class=move || if fulfillment_mode.get() == FulfillmentMode::Delegate { "v2-create-selected-badge" } else { "v2-create-selected-badge v2-create-selected-badge-idle" }>
                                            {move || if fulfillment_mode.get() == FulfillmentMode::Delegate { "Selected" } else { "Use a service" }}
                                        </span>
                                    </span>
                                    <span class="v2-create-help">"Recommended. The service handles downloads and access for this game, so your own account doesn't need to be online."</span>
                                    <Show when=move || fulfillment_mode.get() == FulfillmentMode::Delegate>
                                        <span class="v2-create-mono-value">{move || {
                                            let url = operator_url.get();
                                            if url.trim().is_empty() { "No service chosen yet. Pick one under Advanced options below.".to_string() } else { url }
                                        }}</span>
                                    </Show>
                                </button>

                                <section class="v2-hub-card" aria-labelledby="publish-fee-terms-title">
                                    <h3 id="publish-fee-terms-title" class="v2-create-subsection-title">"Service fee terms"</h3>
                                    <p class="v2-create-help">"Distribution services may take a fee from each sale. Keep the terms you already accepted, or review and accept new ones."</p>
                                    <div>
                                        <label for="publish-fee-terms" class="v2-create-field-label">"Fee terms"</label>
                                        <select id="publish-fee-terms" class="v2-input" prop:value=move || commercial_mode.get() disabled=move || is_publishing.get() on:change:target=move |event| {
                                            commercial_mode.set(event.target().value());
                                            commercial_review.set(None);
                                            commercial_approved.set(false);
                                            commercial_loading.set(false);
                                            commercial_generation.update(|value| *value = value.wrapping_add(1));
                                        }>
                                            <option value="preserve">"Keep the terms already accepted"</option>
                                            <option value="replace">"Review and accept new terms"</option>
                                            <option value="retire" disabled=true>"Stop using a service (not available yet)"</option>
                                        </select>
                                    </div>
                                    <Show when=move || commercial_mode.get() == "replace">
                                        <button type="button" class="v2-btn-secondary" disabled=move || commercial_loading.get() || is_publishing.get() on:click=move |_| on_review_terms.run(())>{move || if commercial_loading.get() { "Checking the service's terms…" } else { "Get the service's terms" }}</button>
                                        {move || commercial_review.get().map(|review| view! {
                                            <div class="v2-create-fields" aria-live="polite">
                                                <dl class="v2-create-review-facts">
                                                    <div><dt>"Sale price"</dt><dd>{format!("{} sats", review.context.price_sats)}</dd></div>
                                                    {review.offers.iter().map(|offer| view! {
                                                        <div><dt>"Service fee"</dt><dd>{fee_policy_label(&offer.fee)}</dd></div>
                                                        <div><dt>"Limitations"</dt><dd>{if offer.limitations.is_empty() { "None".to_string() } else { offer.limitations.join("; ") }}</dd></div>
                                                    }).collect_view()}
                                                    {review.legs.iter().map(|leg| view! {
                                                        <div><dt>{format!("Payout · {:?}", leg.role)}</dt><dd>{format!("{} to {}", msat_text(leg.amount_msat), leg.destination)}</dd></div>
                                                    }).collect_view()}
                                                </dl>
                                                <details class="v2-create-disclosure">
                                                    <summary class="v2-create-disclosure-summary">"Technical details"</summary>
                                                    {review.offers.into_iter().zip(review.accepted_terms).map(|(offer, reference)| view! {
                                                        <p class="v2-create-mono-value">{format!("Service {} · terms {} · settlement v{} · paid to {}", offer.operator_pubkey, reference.terms_event_id, offer.version, offer.payment_destination)}</p>
                                                    }).collect_view()}
                                                    {review.legs.into_iter().map(|leg| view! { <p class="v2-create-mono-value">{format!("{:?}: {} → {} · {} msat", leg.role, leg.recipient_pubkey, leg.destination, leg.amount_msat)}</p> }).collect_view()}
                                                </details>
                                            </div>
                                        })}
                                        <label class="v2-create-checkbox-row">
                                            <input type="checkbox" disabled=move || commercial_review.get().is_none() || is_publishing.get() prop:checked=move || commercial_approved.get() on:change:target=move |event| commercial_approved.set(event.target().checked()) />
                                            "I accept these fee terms and this payout split for this game. Any later change needs a new review."
                                        </label>
                                    </Show>
                                    {move || commercial_error.get().map(|error| view! { <p role="alert" class="v2-hub-section-issue">{error}</p> })}
                                    <p class="v2-create-help">"Keeping your current terms never accepts new ones. Past sales and purchases are not affected."</p>
                                    <details class="v2-create-disclosure">
                                        <summary class="v2-create-disclosure-summary">"Currently accepted terms"</summary>
                                        <p class="v2-create-mono-value">{move || preserved_terms_label.get_value()}</p>
                                    </details>
                                </section>

                                <details class="v2-create-disclosure">
                                    <summary class="v2-create-disclosure-summary">
                                        {move || if fulfillment_mode.get() == FulfillmentMode::Direct { "Advanced options · signing with your own account" } else { "Advanced options" }}
                                    </summary>
                                    <div class="v2-create-fields">
                                        <button type="button" aria-pressed=move || aria_bool(fulfillment_mode.get() == FulfillmentMode::Direct) class=move || if fulfillment_mode.get() == FulfillmentMode::Direct {
                                            "v2-create-mode-option v2-create-mode-selected"
                                        } else {
                                            "v2-create-mode-option"
                                        } on:click=move |_| {
                                            fulfillment_mode.set(FulfillmentMode::Direct);
                                            distribution_user_revision.update(|revision| *revision = revision.wrapping_add(1));
                                        }>
                                            <span class="v2-create-row-between">
                                                <span class=move || if fulfillment_mode.get() == FulfillmentMode::Direct { "v2-create-mode-title v2-create-mode-title-active" } else { "v2-create-mode-title" }>"Sign with my own account instead"</span>
                                                <Show when=move || fulfillment_mode.get() == FulfillmentMode::Direct>
                                                    <span class="v2-create-selected-badge">"Selected"</span>
                                                </Show>
                                            </span>
                                            <span class="v2-create-help">"Your account approves each player's access itself instead of a distribution service."</span>
                                        </button>

                                        <Show when=move || matches!(fulfillment_mode.get(), FulfillmentMode::Delegate)>
                                            <div class="v2-create-subpanel v2-create-fields">
                                                <label for="publish-provider-url" class="v2-create-field-label">"Use a different distribution service"</label>
                                                <div class="v2-create-inline-row">
                                                    <input id="publish-provider-url" class="v2-input" placeholder="https://service.example.com" prop:value={move || operator_url.get()} on:input:target=move |ev| {
                                                         let next = ev.target().value();
                                                         operator_url.set(next.clone());
                                                         distribution_user_revision.update(|revision| *revision = revision.wrapping_add(1));
                                                        if operator_auto_added.get_untracked().is_some() { sync_operator_server(next); }
                                                    } />
                                                    <select aria-label="Use the address of a listed server" class="v2-input" on:change:target=move |ev| {
                                                        let selected = ev.target().value();
                                                        if !selected.is_empty() {
                                                             operator_url.set(selected.clone());
                                                             distribution_user_revision.update(|revision| *revision = revision.wrapping_add(1));
                                                            if operator_auto_added.get_untracked().is_some() { sync_operator_server(selected); }
                                                        }
                                                    }>
                                                        <option value="">"Copy from a server below"</option>
                                                        {move || servers.get().into_iter().map(|server| {
                                                            let url = server.url.clone();
                                                            let text = url.clone();
                                                            view! { <option value={url}>{text}</option> }
                                                        }).collect_view()}
                                                    </select>
                                                </div>
                                                <label class="v2-create-checkbox-row">
                                                    <input type="checkbox" checked={move || operator_auto_added.get().is_some()} on:change:target=move |ev| {
                                                        if ev.target().checked() {
                                                            sync_operator_server(operator_url.get());
                                                        } else if let Some(old_url) = operator_auto_added.get_untracked() {
                                                             servers.update(|entries| entries.retain(|entry| !(entry.auto_operator && entry.url == old_url)));
                                                             operator_auto_added.set(None);
                                                             distribution_user_revision.update(|revision| *revision = revision.wrapping_add(1));
                                                        }
                                                    } />
                                                    "Also upload the build to this service"
                                                </label>
                                            </div>
                                        </Show>

                                        <div class="v2-create-subpanel v2-create-fields">
                                            <div class="v2-create-row-between">
                                                <h3 class="v2-create-subsection-title">"Upload servers"</h3>
                                                <span class="v2-create-help">"Your build is uploaded to every server listed here."</span>
                                            </div>
                                            {move || discovery_error.get().map(|msg| view! { <div class="v2-create-alert v2-create-alert-error">{msg}</div> })}
                                            <div class="v2-create-list">
                                                {move || discovered_servers.get().into_iter().map(|server| {
                                                    let checked_url = server.url.clone();
                                                    let label = server.name.clone().unwrap_or_else(|| server.url.clone());
                                                    let url_for_checked = checked_url.clone();
                                                    let url_for_change = checked_url.clone();
                                                    let label_for_change = label.clone();
                                                    let label_display = label.clone();
                                                    view! {
                                                        <label class="v2-create-server-row">
                                                            <span><input type="checkbox" class="v2-create-checkbox" checked={move || servers.get().iter().any(|entry| entry.url == url_for_checked)} on:change:target=move |ev| {
                                                                if ev.target().checked() { add_server(url_for_change.clone(), label_for_change.clone(), false); }
                                                                else { remove_server(url_for_change.clone()); }
                                                            } />{label_display}</span>
                                                            <span class="v2-create-help">{server.url.clone()}</span>
                                                        </label>
                                                    }
                                                }).collect_view()}
                                            </div>
                                            <div class="v2-create-inline-row">
                                                <label for="publish-custom-server" class="sr-only">"Server address"</label>
                                                <input id="publish-custom-server" class="v2-input" placeholder="Add a server address" prop:value={move || custom_server.get()} on:input:target=move |ev| custom_server.set(ev.target().value()) />
                                                <button type="button" class="v2-btn-secondary" on:click={on_add_custom_server}>"Add"</button>
                                            </div>
                                            <div class="v2-create-list">
                                                {move || servers.get().into_iter().map(|server| {
                                                    let reachability = server.reachability;
                                                    let upload = server.upload;
                                                    let url = server.url.clone();
                                                    view! {
                                                        <div class="v2-create-server-row">
                                                            <div>
                                                                <p class="v2-create-server-name">{server.label}</p>
                                                                <p class="v2-create-help">{server.url}</p>
                                                            </div>
                                                            <div class="v2-create-server-status">
                                                                 <p class={reachability.class()}>{format!("Connection: {}", reachability.label())}</p>
                                                                 <p class={upload.class()}>{format!("Upload: {}", upload.label())}</p>
                                                            </div>
                                                            <button type="button" class="v2-create-media-link v2-create-media-remove" on:click=move |_| remove_server(url.clone())>"Remove"</button>
                                                        </div>
                                                    }
                                                }).collect_view()}
                                            </div>
                                        </div>

                                        <Show when=move || fulfillment_mode.get() == FulfillmentMode::Delegate>
                                            <div class="v2-create-subpanel v2-create-fields">
                                                <h3 class="v2-create-subsection-title">"What each service may do"</h3>
                                                <p class="v2-create-help">"By default a service can only receive your build. Allow it to record purchases or give players access below. Updating the game keeps permissions a service already has; to take them away, use Revoke access on the game overview."</p>
                                                <label class="v2-create-checkbox-row"><input type="checkbox" prop:checked=move || authorize_operators.get() on:change:target=move |event| authorize_operators.set(event.target().checked()) />"Give newly added services these permissions when I publish"</label>
                                                {move || servers.get().into_iter().map(|server| {
                                                    let url = server.url;
                                                    let display = url.clone();
                                                    let receipt_url = url.clone(); let grant_url = url.clone();
                                                    let receipt_checked = url.clone(); let grant_checked = url.clone();
                                                    view! { <div class="v2-hub-role-row">
                                                        <p class="v2-create-mono-value">{display}</p>
                                                        <label class="v2-create-checkbox-row"><input type="checkbox" prop:checked=move || server_roles.get().iter().find(|role| role.server_url == receipt_checked).map(|role| role.issue_receipt).unwrap_or_else(|| receipt_checked == operator_url.get()) on:change:target=move |event| { let checked=event.target().checked(); server_roles.update(|roles| { if let Some(role)=roles.iter_mut().find(|role| role.server_url == receipt_url) { role.issue_receipt=checked; } else { roles.push(arcadestr_core::publisher_management::ServerRoleIntent { server_url:receipt_url.clone(),issue_receipt:checked,issue_grant:receipt_url==operator_url.get_untracked() }); } }); } />"Record purchases"</label>
                                                        <label class="v2-create-checkbox-row"><input type="checkbox" prop:checked=move || server_roles.get().iter().find(|role| role.server_url == grant_checked).map(|role| role.issue_grant).unwrap_or_else(|| grant_checked == operator_url.get()) on:change:target=move |event| { let checked=event.target().checked(); server_roles.update(|roles| { if let Some(role)=roles.iter_mut().find(|role| role.server_url == grant_url) { role.issue_grant=checked; } else { roles.push(arcadestr_core::publisher_management::ServerRoleIntent { server_url:grant_url.clone(),issue_receipt:grant_url==operator_url.get_untracked(),issue_grant:checked }); } }); } />"Give players access"</label>
                                                    </div> }
                                                }).collect_view()}
                                            </div>
                                        </Show>
                                    </div>
                                </details>
                            </div>
                        </Show>
                    </section>
                    </Show>

                    <Show when=move || current_section.get() == HubSection::PriceAccess>
                    <section class="v2-create-stage-panel" id=HubSection::PriceAccess.dom_id() aria-labelledby="game-hub-price-title">
                        {section_header(HubSection::PriceAccess)}
                        <div class="v2-create-fields">
                            <div>
                                <label class="v2-create-field-label" for="acquisition-policy">"How players get access"</label>
                                <select id="acquisition-policy" class="v2-input" on:change:target=move |ev| acquisition_kind.set(AcquisitionKind::from_value(&ev.target().value())) disabled=move || is_publishing.get()>
                                    <option value="gated" selected=move || acquisition_kind.get() == AcquisitionKind::Gated>"Paid"</option>
                                    <option value="public" selected=move || acquisition_kind.get() == AcquisitionKind::Public>"Free"</option>
                                    <option value="timed-access" selected=move || acquisition_kind.get() == AcquisitionKind::TimedAccess>"Free for a limited time"</option>
                                </select>
                                <p class="v2-create-help">{move || match acquisition_kind.get() {
                                    AcquisitionKind::Gated => "Players pay once to own the game. Payments go to your Lightning address.",
                                    AcquisitionKind::Public => "Anyone can get the game at no cost.",
                                    AcquisitionKind::TimedAccess => "Anyone can get the game at no cost between the times you choose.",
                                }}</p>
                            </div>
                            <Show when=move || acquisition_kind.get() == AcquisitionKind::Gated>
                                <div>
                                    <label for="publish-price" class="v2-create-field-label">"Price in sats (required)"</label>
                                    <input id="publish-price" required=true class="v2-input" type="number" min=1 step=1 prop:value={move || price_input.get()} on:input:target=move |ev| price_input.set(ev.target().value()) disabled=move || is_publishing.get() />
                                </div>
                                <div>
                                    <label for="publish-lud16" class="v2-create-field-label">"Lightning address (required)"</label>
                                    <input id="publish-lud16" required=true class="v2-input" placeholder="you@example.com" aria-describedby="publish-lud16-help" prop:value={move || lud16.get()} on:input:target=move |ev| lud16.set(ev.target().value()) disabled=move || is_publishing.get() />
                                    <p id="publish-lud16-help" class="v2-create-help">"Where payments for this game are sent."</p>
                                </div>
                            </Show>
                            <Show when=move || matches!(acquisition_kind.get(), AcquisitionKind::TimedAccess)>
                                <div class="v2-create-fields">
                                    <DateTimeRangePicker
                                        starts_at=Signal::derive(move || acquisition_starts_at.get())
                                        ends_at=Signal::derive(move || acquisition_ends_at.get())
                                        on_starts_at=Callback::new(move |value| acquisition_starts_at.set(value))
                                        on_ends_at=Callback::new(move |value| acquisition_ends_at.set(value))
                                        disabled=Signal::derive(move || is_publishing.get())
                                    />
                                    <p class="v2-create-help">"Times use your local timezone."</p>
                                </div>
                            </Show>
                        </div>
                    </section>
                    </Show>

                    <Show when=move || current_section.get() == HubSection::Overview>
                    {move || {
                        let warnings = if delivery_chosen.get() { checklist().warnings } else { Vec::new() };
                        (!warnings.is_empty()).then(|| view! {
                            <ul class="v2-create-warnings">{warnings.into_iter().map(|warning| view! { <li>{warning}</li> }).collect_view()}</ul>
                        })
                    }}
                    <details class="v2-create-disclosure v2-hub-technical">
                        <summary class="v2-create-disclosure-summary">"Technical details"</summary>
                        <dl class="v2-create-review-facts">
                            {move || {
                                let identity = published_identity.get();
                                identity.coordinate.or(identity.d_tag.or(identity.pending_d_tag)).map(|id| view! {
                                    <div><dt>"Game address"</dt><dd class="v2-create-mono-value">{id}</dd></div>
                                })
                            }}
                            {move || published_identity.get().event_id.map(|id| view! {
                                <div><dt>"Current game event"</dt><dd class="v2-create-mono-value">{id}</dd></div>
                            })}
                            <div><dt>"Publishing account"</dt><dd class="v2-create-mono-value">{move || auth.npub.get().unwrap_or_else(|| "Not signed in".to_string())}</dd></div>
                            <div><dt>"Platforms / version"</dt><dd>{move || format!("{} / {}", platform_summary(&platforms_input.get()), if version.get().trim().is_empty() { "none".to_string() } else { version.get() })}</dd></div>
                            <div><dt>"Distribution"</dt><dd class="v2-create-mono-value">{move || if fulfillment_enabled.get() { format!("{} upload server(s) · service {}", servers.get().len(), if operator_url.get().trim().is_empty() { "none".to_string() } else { operator_url.get() }) } else { "None (store page only)".to_string() }}</dd></div>
                            <div><dt>"Access signed by"</dt><dd>{move || match fulfillment_mode.get() { FulfillmentMode::None => "Nobody (no build)", FulfillmentMode::Direct => "Your account", FulfillmentMode::Delegate => "Distribution service" }}</dd></div>
                            <div><dt>"Build fingerprint (SHA-256)"</dt><dd class="v2-create-mono-value">{move || file_hash.get().map(|hash| format_sha256(&hash)).unwrap_or_else(|| "none".into())}</dd></div>
                            <div><dt>"Distribution key"</dt><dd class="v2-create-mono-value">{existing_fulfillment_pubkey.clone().unwrap_or_else(|| "Created when you publish, if needed".into())}</dd></div>
                            <div><dt>"Promotion links kept"</dt><dd>{existing_campaigns.len()}</dd></div>
                        </dl>
                        <super::publish_launch::PublishLaunchReview
                            state=publish_draft
                            artifact_enabled=Signal::derive(move || fulfillment_enabled.get())
                            existing_artifact_digest=Signal::derive(move || {
                                fulfillment_enabled.get().then(|| file_hash.get()).flatten()
                            })
                            surfaced_warnings=surfaced_archive_warnings
                        />
                    </details>
                    </Show>
            </div>

            <Show when=move || current_section.get() != HubSection::Overview>
                // Leaving a section is only navigation: nothing is finalized,
                // and the section can be reopened any time.
                <div class="v2-create-actions">
                    <div class="v2-create-actions-buttons">
                        <button type="button" class="v2-btn-secondary" on:click=move |_| open_section(HubSection::Overview)>"< Back to overview"</button>
                    </div>
                    <div class="v2-create-actions-state">
                        <span class="v2-create-authoring" role="status">{move || draft_save_status.get().label().unwrap_or_else(|| "Changes are saved as you type.".to_string())}</span>
                        <span class="v2-create-help">{if editing {
                            "They go live only when you select Publish changes."
                        } else {
                            "You can come back to any section at any time."
                        }}</span>
                    </div>
                </div>
            </Show>
        </main>
    }
}
