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

use crate::campaign_management::datetime_local_to_unix;
use crate::models::{AcquisitionPolicy, CampaignPointer, ListingSource};
use crate::publish_state::{
    partial_retry_payload, prepare_publish, preserve_edited_platforms, ArchiveSelectionState,
    AuthoritativeEditModel, AuthoritativeEditState, DefectReviewState, LaunchMode, PreparedPublish,
    ProviderBuildFields, PublishAttempt, PublishDirtySnapshot, PublishIdentityOverlay,
    PublishState, Transition,
};
use crate::tauri_bridge::{
    invoke_add_blossom_server, invoke_check_adp_server, invoke_discard_archive_selection,
    invoke_discard_blossom_media_selection, invoke_discover_adp_servers,
    invoke_get_blossom_server_settings, invoke_hash_selected_archive, invoke_inspect_archive,
    invoke_load_authoritative_adp_edit_metadata, invoke_publish_adp_listing_with_contract,
    invoke_resolve_adp_operator, invoke_select_archive_build, invoke_select_blossom_media_file,
    invoke_start_blossom_upload, listen_publish_progress, AddBlossomServerRequest,
    AdpServerAnnouncement, ArchiveAccountFailure, ArchiveRequestId, BlossomMediaSelectionDto,
    BuildSelectionRef, CampaignPointerInput, DiscardArchiveSelectionRequest,
    DiscardBlossomMediaRequest, ExpectedBlossomPublisherRequest, FulfillmentMode,
    HashSelectedArchiveRequest, InspectArchiveRequest, LoadAuthoritativeAdpEditMetadataRequest,
    PublishAdpListingRequest, PublishAdpListingResult, PublishProgressPayload,
    ResolveAdpOperatorRequest, SelectArchiveRequest, StartBlossomUploadRequest,
};
use crate::ui_v2::components::aria::aria_bool;
use crate::ui_v2::components::blossom_media_upload::{
    fresh_request_id, preferred_candidate, publisher_hex, stable_error_message,
};
use crate::ui_v2::components::{
    Button, ButtonVariant, Dialog, DialogCloseAction, DialogClosePolicy, DialogCloseRequest,
    DialogDismissal, DialogInitialFocus, DialogTone, DialogWidth,
};
use crate::ui_v2::views::publish::{
    publisher_signer_state, signer_can_publish, PublisherSignerState,
};
use crate::ui_v2::views::use_fallback_cover;
use crate::{AuthContext, GameListing};

use super::date_time_picker::DateTimeRangePicker;

const DEFAULT_ADP_SERVER_URL: &str = match option_env!("ARCADESTR_DEFAULT_ADP_SERVER_URL") {
    Some(url) => url,
    None => "http://localhost:9099",
};

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum PublishStage {
    Details,
    Pricing,
    Builds,
    Review,
}

impl PublishStage {
    const ALL: [Self; 4] = [Self::Details, Self::Pricing, Self::Builds, Self::Review];

    fn index(self) -> usize {
        match self {
            Self::Details => 0,
            Self::Pricing => 1,
            Self::Builds => 2,
            Self::Review => 3,
        }
    }

    fn title(self) -> &'static str {
        match self {
            Self::Details => "Game details",
            Self::Pricing => "Pricing and access",
            Self::Builds => "Builds and distribution",
            Self::Review => "Review and publish",
        }
    }

    fn next(self) -> Option<Self> {
        Self::ALL.get(self.index() + 1).copied()
    }

    fn previous(self) -> Option<Self> {
        self.index()
            .checked_sub(1)
            .and_then(|index| Self::ALL.get(index).copied())
    }
}

/// Drafts are explicitly saved to account-scoped native storage, not localStorage.
const SUPPORTS_DRAFTS: bool = true;

/// Truthful state of a single stage in the indicator.
///
/// `Complete` is never inferred from "was visited": a passed stage whose fields
/// were later broken reports `NeedsAttention`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StageStatus {
    Complete,
    NeedsAttention,
    Current,
    Upcoming,
}

fn stage_status(stage: PublishStage, current: PublishStage, valid: bool) -> StageStatus {
    if stage == current {
        StageStatus::Current
    } else if stage.index() < current.index() {
        if valid {
            StageStatus::Complete
        } else {
            StageStatus::NeedsAttention
        }
    } else {
        StageStatus::Upcoming
    }
}

/// Forward jumping stays blocked: a stage is only reachable once the workflow has
/// already validated its way there.
fn stage_reachable(stage: PublishStage, current: PublishStage) -> bool {
    stage.index() <= current.index()
}

impl StageStatus {
    fn class(self) -> &'static str {
        match self {
            Self::Complete => "v2-create-stage-complete",
            Self::NeedsAttention => "v2-create-stage-attention",
            Self::Current => "v2-create-stage-current",
            Self::Upcoming => "v2-create-stage-upcoming",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Complete => "Complete",
            Self::NeedsAttention => "Needs attention",
            Self::Current => "Current",
            Self::Upcoming => "Not started",
        }
    }
}

/// Blocking issues gathered across stages for the review summary, in stage order.
fn blocking_stage_errors(
    details: &Result<(), String>,
    pricing: &Result<(), String>,
    builds: &Result<(), String>,
) -> Vec<(PublishStage, String)> {
    [
        (PublishStage::Details, details),
        (PublishStage::Pricing, pricing),
        (PublishStage::Builds, builds),
    ]
    .into_iter()
    .filter_map(|(stage, result)| result.as_ref().err().map(|error| (stage, error.clone())))
    .collect()
}

/// Where the workflow stands for the author, kept separate from publication
/// outcome so a local edit is never described as a published listing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AuthoringState {
    NotStarted,
    UnsavedChanges,
    Blocked,
    ReadyToPublish,
}

fn authoring_state(editing: bool, dirty: bool, blocking_issues: usize) -> AuthoringState {
    if blocking_issues == 0 {
        AuthoringState::ReadyToPublish
    } else if dirty {
        AuthoringState::UnsavedChanges
    } else if editing {
        AuthoringState::Blocked
    } else {
        AuthoringState::NotStarted
    }
}

impl AuthoringState {
    fn label(self) -> &'static str {
        match self {
            Self::NotStarted => "Nothing entered yet",
            Self::UnsavedChanges => "Unsaved changes · not publishable yet",
            Self::Blocked => "Required information is missing",
            Self::ReadyToPublish => "Ready to publish",
        }
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
        PublisherSignerState::Available => {
            "Network publication is authorized by the active account signer."
        }
        PublisherSignerState::Connecting => {
            "Network publication is unavailable while the remote signer connects."
        }
        PublisherSignerState::Unavailable => {
            "Network publication requires an available signer for the active account."
        }
        PublisherSignerState::Unknown => {
            "Network publication stays unavailable until signer availability is resolved."
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
    images: Vec<String>,
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
            _ => Err("Commercial review is stale; retrieve exact signed terms again".into()),
        },
        "replace" => {
            Err("Review the signed terms and explicitly approve their allocation first".into())
        }
        _ => {
            Err("Operator retirement is unavailable pending an agreed prospective workflow".into())
        }
    }
}

fn fee_policy_label(policy: &FeePolicy) -> String {
    match policy {
        FeePolicy::None => "No operator fee".into(),
        FeePolicy::Fixed { amount_msat } => format!("Fixed {amount_msat} msat"),
        FeePolicy::BasisPoints {
            basis_points,
            minimum_msat,
            maximum_msat,
        } => {
            let minimum = minimum_msat
                .map(|amount| format!("{amount} msat"))
                .unwrap_or_else(|| "none".into());
            let maximum = maximum_msat
                .map(|amount| format!("{amount} msat"))
                .unwrap_or_else(|| "none".into());
            format!("{basis_points} basis points · minimum {minimum} · maximum {maximum}")
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

fn listing_image_mime(mime: &str) -> bool {
    matches!(mime, "image/jpeg" | "image/png" | "image/webp")
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ListingImageDraft {
    Hosted(String),
    Pending(BlossomMediaSelectionDto),
}

fn hosted_image_urls(images: &[ListingImageDraft]) -> Result<Vec<String>, String> {
    let hosted = images
        .iter()
        .filter_map(|image| match image {
            ListingImageDraft::Hosted(url) => Some(url.clone()),
            ListingImageDraft::Pending(_) => None,
        })
        .collect::<Vec<_>>();
    validate_http_urls(&hosted.join(","), "Image URL")
}

fn screenshot_slot_count(image_count: usize) -> usize {
    let screenshot_count = image_count.saturating_sub(1);
    4.max(screenshot_count.saturating_add(1))
}

fn has_pending_images(images: &[ListingImageDraft]) -> bool {
    images
        .iter()
        .any(|image| matches!(image, ListingImageDraft::Pending(_)))
}

fn image_server_ready(images: &[ListingImageDraft], server: Option<&str>) -> bool {
    !has_pending_images(images) || server.is_some()
}

fn human_image_size(size: u64) -> String {
    if size >= 1_048_576 {
        format!("{:.1} MiB", size as f64 / 1_048_576.0)
    } else if size >= 1_024 {
        format!("{:.1} KiB", size as f64 / 1_024.0)
    } else {
        format!("{size} bytes")
    }
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
            "The game page exists on the network, but automated installation is incomplete because an upload failed. Targeted upload retry is not supported by the current backend."
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

fn publication_completed(
    mut state: PublicationState,
    result: Result<(String, bool), String>,
) -> PublicationState {
    match result {
        Ok((_event_id, true)) => {
            state.listing_published = true;
            state.outcome = PublicationOutcome::Partial;
            state.message = Some(
                "The game page exists on the network, but automated installation is incomplete because an upload failed. Targeted upload retry is not supported by the current backend."
                    .to_string(),
            );
        }
        Ok((event_id, false)) => {
            state.listing_published = true;
            state.outcome = PublicationOutcome::Complete;
            state.message = Some(format!("Network publication completed: {event_id}"));
        }
        Err(error) if state.listing_published => {
            state.outcome = PublicationOutcome::Partial;
            state.message = Some(format!(
                "The game page exists on the network, but the remaining publication work failed: {error}. Targeted upload retry is not supported by the current backend."
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
        "hash-file" => "Build verification",
        "check-operator" => "Distribution provider check",
        "provision" => "Publishing authorization",
        "publish-listing" => "Game page publication",
        "confirm-propagation" => "Network publication confirmation",
        "upload" => "Build upload",
        _ => "Publication",
    };
    let status = match event.status.as_str() {
        "pending" => "in progress",
        "ok" => "completed",
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
        event
            .message
            .as_ref()
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

fn advance_stage(
    current: PublishStage,
    validation: Result<(), String>,
) -> Result<PublishStage, String> {
    validation?;
    current
        .next()
        .ok_or_else(|| "Already at the review stage".to_string())
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

fn publish_error_message(error: &PublishAdpListingError) -> String {
    match error {
        PublishAdpListingError::StaleExpectedEventId { .. } => {
            "A concurrent listing revision was detected. Your local work is preserved and publication is blocked. Leave this workflow, return to Published games, and reopen the latest revision before publishing."
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

pub(super) fn archive_error_message(error: &ArchiveSelectionError) -> String {
    publish_error_message(&PublishAdpListingError::Archive {
        error: error.clone(),
    })
}

fn build_capability_message() -> &'static str {
    #[cfg(feature = "web")]
    {
        "Build file selection, hashing, and network publication require the desktop app."
    }
    #[cfg(not(feature = "web"))]
    {
        "This desktop app can select, hash, and publish build files."
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
        return Err("Description is required".to_string());
    }
    if description.len() > 2000 {
        return Err("Description must be 2000 characters or less".to_string());
    }
    if price_sats > 0 && lud16.is_empty() {
        return Err("Lightning address (lud16) is required for priced listings".to_string());
    }
    if !lud16.is_empty() {
        let parts: Vec<&str> = lud16.split('@').collect();
        if parts.len() != 2 || parts[0].is_empty() || parts[1].is_empty() {
            return Err("Lightning address must look like name@example.com".to_string());
        }
    }
    if fulfillment_enabled {
        if servers.is_empty() {
            return Err("Add at least one distribution server for fulfillment".to_string());
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
                return Err(
                    "Select a build file and wait for its hash before publishing fulfillment"
                        .to_string(),
                );
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
            return Err("Version is required for fulfillment".to_string());
        }
        match fulfillment_mode {
            FulfillmentMode::None => {
                return Err("Choose a fulfillment signing mode".to_string());
            }
            FulfillmentMode::Direct => {}
            FulfillmentMode::Delegate => {
                // Metadata edits preserve signed authority without a local operator row.
                // The backend requires an operator when explicit new provisioning is needed.
                if !operator_url.trim().is_empty() && !is_http_url(operator_url) {
                    return Err("Operator URL must start with http:// or https://".to_string());
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
        warnings.push("No build will be uploaded; this publishes game-page metadata only.".into());
    }
    if servers
        .iter()
        .any(|server| server.reachability == ServerStatus::Failed)
    {
        warnings.push("At least one distribution provider could not be reached.".into());
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

fn initial_operator_url(editing_delegated_listing: bool) -> String {
    if editing_delegated_listing {
        String::new()
    } else {
        DEFAULT_ADP_SERVER_URL.to_string()
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

#[cfg(test)]
mod tests {
    use super::*;

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

        assert!(message.contains("local work is preserved"));
        assert!(message.contains("publication is blocked"));
        assert!(message.contains("reopen the latest revision"));
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
        assert_eq!(initial_operator_url(true), "");

        let request = operator_resolution_request(&listing)
            .expect("delegated edit should request an exact local operator lookup");
        assert_eq!(request.publisher_npub, listing.publisher_npub);
        assert_eq!(request.fulfillment_pubkey, delegate.public_key().to_hex());
        assert_eq!(request.scope, "managed-game");
    }

    #[test]
    fn new_publications_use_the_configured_distribution_provider() {
        assert_eq!(initial_operator_url(false), DEFAULT_ADP_SERVER_URL);
        assert!(is_http_url(DEFAULT_ADP_SERVER_URL));
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
    fn stage_navigation_validates_forward_and_keeps_form_values_external() {
        let title = String::from("Preserved game");
        assert_eq!(
            advance_stage(PublishStage::Details, Err("missing".into())),
            Err("missing".into())
        );
        assert_eq!(
            advance_stage(PublishStage::Details, Ok(())),
            Ok(PublishStage::Pricing)
        );
        assert_eq!(title, "Preserved game");
    }

    #[test]
    fn drafts_use_explicit_account_scoped_database_actions() {
        assert!(SUPPORTS_DRAFTS);
        assert!(include_str!("publish.rs").contains("invoke_save_publisher_draft"));
        assert!(include_str!("publish.rs").contains("RESTORE SAVED DRAFT (REPLACES FORM)"));
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
    fn deferred_listing_images_accept_only_supported_image_types() {
        assert!(listing_image_mime("image/jpeg"));
        assert!(listing_image_mime("image/png"));
        assert!(listing_image_mime("image/webp"));
        assert!(!listing_image_mime("image/gif"));
        assert!(!listing_image_mime("video/mp4"));
    }

    #[test]
    fn hosted_listing_image_urls_preserve_visual_order() {
        let images = vec![
            ListingImageDraft::Hosted("https://cdn.example/cover.png".into()),
            ListingImageDraft::Pending(BlossomMediaSelectionDto {
                selection_id: "pending".into(),
                filename: "shot.png".into(),
                detected_mime: "image/png".into(),
                size: 42,
                width: Some(1),
                height: Some(1),
                preview_data_url: Some("data:image/png;base64,AA==".into()),
            }),
            ListingImageDraft::Hosted("https://cdn.example/shot-2.png".into()),
        ];
        assert_eq!(
            hosted_image_urls(&images),
            Ok(vec![
                "https://cdn.example/cover.png".into(),
                "https://cdn.example/shot-2.png".into()
            ])
        );
        assert!(has_pending_images(&images));
        assert!(!image_server_ready(&images, None));
        assert!(image_server_ready(&images, Some("https://blossom.example")));
        let mut completed = images;
        completed[1] = ListingImageDraft::Hosted("https://cdn.example/shot-1.png".into());
        assert!(!has_pending_images(&completed));
        assert!(image_server_ready(&completed, None));
        assert_eq!(
            hosted_image_urls(&completed),
            Ok(vec![
                "https://cdn.example/cover.png".into(),
                "https://cdn.example/shot-1.png".into(),
                "https://cdn.example/shot-2.png".into()
            ])
        );
    }

    #[test]
    fn screenshot_slots_start_at_four_and_keep_one_empty_slot() {
        assert_eq!(screenshot_slot_count(0), 4);
        assert_eq!(screenshot_slot_count(1), 4);
        assert_eq!(screenshot_slot_count(4), 4);
        assert_eq!(screenshot_slot_count(5), 5);
        assert_eq!(screenshot_slot_count(8), 8);
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
            .any(|warning| warning.contains("could not be reached")));
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
            .is_some_and(|message| message.contains("game page exists")));

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
    fn capability_message_matches_the_build_target() {
        #[cfg(feature = "web")]
        assert!(build_capability_message().contains("require the desktop app"));
        #[cfg(not(feature = "web"))]
        assert!(build_capability_message().contains("desktop app can"));
    }

    #[test]
    fn four_production_stages_keep_their_real_names_and_order() {
        assert_eq!(PublishStage::ALL.len(), 4);
        assert_eq!(
            PublishStage::ALL.map(PublishStage::title),
            [
                "Game details",
                "Pricing and access",
                "Builds and distribution",
                "Review and publish",
            ]
        );
        assert_eq!(PublishStage::Details.next(), Some(PublishStage::Pricing));
        assert_eq!(PublishStage::Review.next(), None);
        assert_eq!(PublishStage::Details.previous(), None);
    }

    #[test]
    fn visiting_a_stage_never_marks_it_complete_on_its_own() {
        // Behind the current stage, completeness follows live validation only.
        assert_eq!(
            stage_status(PublishStage::Details, PublishStage::Builds, true),
            StageStatus::Complete
        );
        assert_eq!(
            stage_status(PublishStage::Details, PublishStage::Builds, false),
            StageStatus::NeedsAttention
        );
        assert_eq!(
            stage_status(PublishStage::Builds, PublishStage::Builds, false),
            StageStatus::Current
        );
        assert_eq!(
            stage_status(PublishStage::Review, PublishStage::Builds, true),
            StageStatus::Upcoming
        );
    }

    #[test]
    fn forward_jumping_stays_blocked_while_backward_navigation_is_safe() {
        assert!(stage_reachable(PublishStage::Details, PublishStage::Builds));
        assert!(stage_reachable(PublishStage::Builds, PublishStage::Builds));
        assert!(!stage_reachable(PublishStage::Review, PublishStage::Builds));
        assert!(advance_stage(PublishStage::Details, Err("blocked".into())).is_err());
        assert_eq!(
            advance_stage(PublishStage::Details, Ok(())),
            Ok(PublishStage::Pricing)
        );
    }

    #[test]
    fn blocking_issues_are_reported_in_stage_order_with_actionable_text() {
        let issues = blocking_stage_errors(
            &Err("Title is required".to_string()),
            &Ok(()),
            &Err("Version is required for fulfillment".to_string()),
        );
        assert_eq!(
            issues,
            vec![
                (PublishStage::Details, "Title is required".to_string()),
                (
                    PublishStage::Builds,
                    "Version is required for fulfillment".to_string()
                ),
            ]
        );
        assert!(blocking_stage_errors(&Ok(()), &Ok(()), &Ok(())).is_empty());
    }

    #[test]
    fn authoring_state_separates_untouched_dirty_blocked_and_ready() {
        assert_eq!(authoring_state(false, false, 2), AuthoringState::NotStarted);
        assert_eq!(authoring_state(true, false, 2), AuthoringState::Blocked);
        assert_eq!(
            authoring_state(false, true, 2),
            AuthoringState::UnsavedChanges
        );
        assert_eq!(
            authoring_state(false, true, 0),
            AuthoringState::ReadyToPublish
        );
        // Nothing in this workflow may describe itself as a saved draft.
        for state in [
            AuthoringState::NotStarted,
            AuthoringState::UnsavedChanges,
            AuthoringState::Blocked,
            AuthoringState::ReadyToPublish,
        ] {
            assert!(!state.label().to_ascii_lowercase().contains("draft"));
            assert!(!state.label().to_ascii_lowercase().contains("published"));
        }
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
    fn media_presentation_reuses_existing_selection_semantics() {
        let hosted = ListingImageDraft::Hosted("https://cdn.example.com/cover.png".to_string());
        assert!(!has_pending_images(std::slice::from_ref(&hosted)));
        assert!(image_server_ready(std::slice::from_ref(&hosted), None));
        assert_eq!(
            hosted_image_urls(std::slice::from_ref(&hosted)),
            Ok(vec!["https://cdn.example.com/cover.png".to_string()])
        );
        assert!(listing_image_mime("image/png"));
        assert!(!listing_image_mime("image/gif"));
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
        ] {
            assert!(!source.contains(&absent), "unsupported control rendered");
        }
        assert!(source.contains("does not store Create Game drafts"));
    }
}

/// Publish view component - form for creating NIP-99 listings with optional ADP fulfillment.
#[component]
pub fn PublishView(
    #[prop(optional)] listing: Option<GameListing>,
    #[prop(optional)] on_published: Option<Callback<GameListing>>,
    #[prop(optional)] on_back: Option<Callback<()>>,
) -> impl IntoView {
    let auth = use_context::<AuthContext>().expect("AuthContext not provided");
    let editing = listing.is_some();
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
    let durable_draft_id = StoredValue::new(format!(
        "listing:{}",
        listing
            .as_ref()
            .map(|listing| listing.id.as_str())
            .unwrap_or("new")
    ));
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
    let image_drafts = RwSignal::new(
        listing
            .as_ref()
            .map(|item| {
                item.images
                    .iter()
                    .cloned()
                    .map(ListingImageDraft::Hosted)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default(),
    );
    let selected_image_publisher = RwSignal::new(None::<String>);
    let is_selecting_image = RwSignal::new(false);
    let image_upload_status = RwSignal::new(None::<String>);
    let image_upload_server = RwSignal::new(None::<String>);
    let image_server_origin = RwSignal::new(String::new());
    let image_server_error = RwSignal::new(None::<String>);
    let is_saving_image_server = RwSignal::new(false);
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
            .unwrap_or_else(|| "No accepted roots; legacy fee-free configuration".into()),
    );
    let discovered_servers = RwSignal::new(Vec::<AdpServerAnnouncement>::new());
    let discovery_error = RwSignal::new(None::<String>);
    let initial_servers = if use_default_provider {
        vec![ServerEntry {
            url: DEFAULT_ADP_SERVER_URL.to_string(),
            label: "Arcadestr distribution provider".into(),
            reachability: ServerStatus::Pending,
            upload: ServerStatus::Idle,
            auto_operator: true,
        }]
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
    let initial_operator_url = initial_operator_url(editing_delegated_listing);
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
    let operator_auto_added = RwSignal::new(use_default_provider.then_some(initial_operator_url));
    let file_hash = RwSignal::new(existing_file_hash);
    let version = RwSignal::new(
        listing
            .as_ref()
            .and_then(|item| listing_spec(item, "version"))
            .unwrap_or_default(),
    );

    let current_stage = RwSignal::new(PublishStage::Details);
    let stage_error = RwSignal::new(None::<String>);
    let can_exit = on_back.is_some();

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
        add_server(new_url, "Distribution provider".to_string(), true);
    };

    Effect::new(move |_| {
        spawn_local(async move {
            match invoke_discover_adp_servers().await {
                Ok(found) => {
                    discovered_servers.set(found);
                    discovery_error.set(None);
                }
                Err(err) => {
                    discovery_error.set(Some(format!("Couldn't reach relays for discovery: {err}")))
                }
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
                error_message.set(Some("The reselected archive does not match the saved draft digest. The saved artifact reference was not changed.".into()));
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

    let editing_publisher_for_images = editing_publisher.clone();
    let on_select_image = Callback::new(move |target_index: usize| {
        if !can_dispatch(
            is_selecting_image.get_untracked(),
            is_publishing.get_untracked(),
        ) {
            return;
        }
        let Some(account) = auth.npub.get_untracked() else {
            error_message.set(Some("Sign in before selecting an image.".into()));
            return;
        };
        if !publication_account_allowed(editing_publisher_for_images.as_deref(), &account) {
            error_message.set(Some(
                "Switch to the developer account that published this Game page before adding images."
                    .into(),
            ));
            return;
        }
        let Some(account_hex) = publisher_hex(&account) else {
            error_message.set(Some("The active publisher key is invalid.".into()));
            return;
        };
        if has_pending_images(&image_drafts.get_untracked())
            && selected_image_publisher.get_untracked().as_deref() != Some(account_hex.as_str())
        {
            error_message.set(Some(
                "Remove images selected by the previous account before adding more.".into(),
            ));
            return;
        }
        is_selecting_image.set(true);
        error_message.set(None);
        spawn_local(async move {
            match invoke_get_blossom_server_settings(ExpectedBlossomPublisherRequest {
                expected_publisher_hex: account_hex.clone(),
            })
            .await
            {
                Ok(settings) if settings.publisher_pubkey == account_hex => {
                    image_upload_server.set(preferred_candidate(&settings));
                    image_server_error.set(None);
                }
                Ok(_) => {
                    image_upload_server.set(None);
                    image_server_error.set(Some(
                        "The Blossom settings belong to a different account.".into(),
                    ));
                }
                Err(command_error) => {
                    image_upload_server.set(None);
                    image_server_error.set(Some(stable_error_message(&command_error.code).into()));
                }
            }
            let picked = invoke_select_blossom_media_file(ExpectedBlossomPublisherRequest {
                expected_publisher_hex: account_hex.clone(),
            })
            .await;
            if auth.npub.get_untracked().as_deref() != Some(account.as_str()) {
                if let Ok(Some(stale)) = picked {
                    let _ = invoke_discard_blossom_media_selection(DiscardBlossomMediaRequest {
                        selection_id: stale.selection_id,
                        expected_publisher_hex: account_hex,
                    })
                    .await;
                }
                error_message.set(Some(
                    "The active account changed while selecting the image.".into(),
                ));
                is_selecting_image.set(false);
                return;
            }
            match picked {
                Ok(Some(selection)) if listing_image_mime(&selection.detected_mime) => {
                    selected_image_publisher.set(Some(account_hex.clone()));
                    let replaced = image_drafts
                        .try_update(|images| {
                            let selected = ListingImageDraft::Pending(selection);
                            if target_index < images.len() {
                                Some(std::mem::replace(&mut images[target_index], selected))
                            } else {
                                images.push(selected);
                                None
                            }
                        })
                        .flatten();
                    if let Some(ListingImageDraft::Pending(replaced)) = replaced {
                        let _ =
                            invoke_discard_blossom_media_selection(DiscardBlossomMediaRequest {
                                selection_id: replaced.selection_id,
                                expected_publisher_hex: account_hex,
                            })
                            .await;
                    }
                }
                Ok(Some(selection)) => {
                    let _ = invoke_discard_blossom_media_selection(DiscardBlossomMediaRequest {
                        selection_id: selection.selection_id,
                        expected_publisher_hex: account_hex,
                    })
                    .await;
                    error_message.set(Some("Choose a JPEG, PNG, or WebP image.".into()));
                }
                Ok(None) => {}
                Err(command_error) => {
                    error_message.set(Some(stable_error_message(&command_error.code).to_string()))
                }
            }
            is_selecting_image.set(false);
        });
    });

    let remove_image = Callback::new(move |index: usize| {
        if is_publishing.get_untracked() {
            return;
        }
        let removed = image_drafts
            .try_update(|images| (index < images.len()).then(|| images.remove(index)))
            .flatten();
        if let (Some(ListingImageDraft::Pending(selection)), Some(expected_publisher_hex)) =
            (removed, selected_image_publisher.get_untracked())
        {
            spawn_local(async move {
                let _ = invoke_discard_blossom_media_selection(DiscardBlossomMediaRequest {
                    selection_id: selection.selection_id,
                    expected_publisher_hex,
                })
                .await;
            });
        }
    });

    let save_image_server = Callback::new(move |_: ()| {
        if is_saving_image_server.get_untracked() || is_publishing.get_untracked() {
            return;
        }
        let origin = image_server_origin.get_untracked().trim().to_string();
        if origin.is_empty() {
            image_server_error.set(Some("Enter a Blossom server URL.".into()));
            return;
        }
        let Some(account) = auth.npub.get_untracked() else {
            image_server_error.set(Some("Sign in before adding a Blossom server.".into()));
            return;
        };
        let Some(account_hex) = publisher_hex(&account) else {
            image_server_error.set(Some("The active publisher key is invalid.".into()));
            return;
        };
        if selected_image_publisher.get_untracked().as_deref() != Some(account_hex.as_str()) {
            image_server_error.set(Some(
                "Switch back to the account that selected these images.".into(),
            ));
            return;
        }
        is_saving_image_server.set(true);
        image_server_error.set(None);
        spawn_local(async move {
            match invoke_add_blossom_server(AddBlossomServerRequest {
                expected_publisher_hex: account_hex.clone(),
                origin,
                label: Some("Game publishing".into()),
            })
            .await
            {
                Ok(settings) if settings.publisher_pubkey == account_hex => {
                    if let Some(server) = preferred_candidate(&settings) {
                        image_upload_server.set(Some(server));
                        image_server_origin.set(String::new());
                    } else {
                        image_server_error.set(Some(
                            "The Blossom server was added but is not enabled.".into(),
                        ));
                    }
                }
                Ok(_) => image_server_error.set(Some(
                    "The Blossom settings belong to a different account.".into(),
                )),
                Err(command_error) => {
                    image_server_error.set(Some(stable_error_message(&command_error.code).into()))
                }
            }
            is_saving_image_server.set(false);
        });
    });

    // Single source of stage validation, reused by forward navigation, the stage
    // indicator, and the review summary so no surface can disagree about whether a
    // stage is actually satisfied.
    let validate_stage = move |stage: PublishStage| -> Result<(), String> {
        match stage {
            PublishStage::Details => validate_listing(
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
            .and_then(|_| hosted_image_urls(&image_drafts.get()).map(|_| ()))
            .and_then(|_| {
                if image_server_ready(&image_drafts.get(), image_upload_server.get().as_deref()) {
                    Ok(())
                } else {
                    Err("Add a Blossom upload server below before continuing.".into())
                }
            }),
            PublishStage::Pricing => {
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
            PublishStage::Builds => {
                let draft = publish_draft.get();
                if matches!(draft.launch, LaunchMode::Launchable(_)) && !fulfillment_enabled.get() {
                    return Err(
                        "Enable automated installation, or explicitly choose download-only."
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
                        "Complete the launchable tier, or explicitly choose download-only."
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
            }
            PublishStage::Review => Err("Already at the review stage".to_string()),
        }
    };

    let blocking_issues = move || {
        let mut issues = blocking_stage_errors(
            &validate_stage(PublishStage::Details),
            &validate_stage(PublishStage::Pricing),
            &validate_stage(PublishStage::Builds),
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
                PublishStage::Builds,
                "Authoritative launch metadata must load before publishing this edit.".to_string(),
            ));
        }
        issues
    };

    let on_next = move |_| {
        let stage = current_stage.get_untracked();
        let validation = validate_stage(stage);
        if stage == PublishStage::Details
            && validation.is_err()
            && !image_server_ready(
                &image_drafts.get_untracked(),
                image_upload_server.get_untracked().as_deref(),
            )
        {
            image_server_error.set(Some(
                "Add an enabled Blossom server before continuing.".into(),
            ));
        }
        match advance_stage(stage, validation) {
            Ok(next) => {
                current_stage.set(next);
                stage_error.set(None);
            }
            Err(error) => stage_error.set(Some(error)),
        }
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
        images: image_drafts
            .get()
            .into_iter()
            .map(|image| match image {
                ListingImageDraft::Hosted(url) => format!("hosted:{url}"),
                ListingImageDraft::Pending(selection) => {
                    format!("pending:{}", selection.selection_id)
                }
            })
            .collect(),
        fulfillment_enabled: fulfillment_enabled.get(),
        commercial: format!(
            "{}:{:?}:{}",
            commercial_mode.get(),
            commercial_review.get(),
            commercial_approved.get()
        ),
    };
    let save_auth = auth.clone();
    let save_snapshot = full_form_snapshot.clone();
    let on_save_draft = Callback::new(move |_: ()| {
        let Some(publisher) = save_auth.npub.get_untracked() else {
            return;
        };
        if durable_draft_owner.get_value().as_deref() != Some(publisher.as_str()) {
            draft_message.set(Some(
                "Switch back to this editor's publisher before saving its draft".into(),
            ));
            return;
        }
        if draft_busy.get_untracked() || is_publishing.get_untracked() {
            return;
        }
        let epoch = save_auth.current_auth_flow();
        let response_auth = save_auth.clone();
        let mut form = save_snapshot();
        form.images.retain(|url| url.starts_with("hosted:"));
        let snapshot = DurableListingDraft {
            server_roles: server_roles.get_untracked(),
            form,
            identity: published_identity.get_untracked(),
            launch: publish_draft.get_untracked().launch,
            servers: servers
                .get_untracked()
                .into_iter()
                .map(|server| server.url)
                .collect(),
            mode: fulfillment_mode.get_untracked(),
            acquisition: acquisition_kind.get_untracked(),
            artifact_hash: file_hash.get_untracked(),
            commercial_intent: commercial_mode.get_untracked(),
            commercial_review: commercial_review.get_untracked(),
            accepted_terms: durable_accepted_terms.get_value(),
            authorization_references: durable_authorizations.get_value(),
        };
        let payload = match serde_json::to_value(snapshot) {
            Ok(payload) => payload,
            Err(_) => {
                draft_message.set(Some("Draft could not be encoded".into()));
                return;
            }
        };
        let draft = arcadestr_core::publisher_recovery::PublisherDraft {
            draft_id: durable_draft_id.get_value(),
            kind: "listing".into(),
            payload,
            revision: draft_revision.get_untracked().unwrap_or(0),
        };
        draft_busy.set(true);
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
            draft_busy.set(false);
            match result {
                Ok(revision) => {
                    draft_revision.set(Some(revision));
                    draft_message.set(Some("Draft saved to this account's database. Archive and pending media selections must be reselected after restart.".into()));
                }
                Err(error) => draft_message.set(Some(error)),
            }
        });
    });
    let restore_auth = auth.clone();
    let on_restore_draft = Callback::new(move |_: ()| {
        let Some(publisher) = restore_auth.npub.get_untracked() else {
            return;
        };
        if durable_draft_owner.get_value().as_deref() != Some(publisher.as_str()) {
            draft_message.set(Some(
                "Switch back to this editor's publisher before restoring its draft".into(),
            ));
            return;
        }
        if draft_busy.get_untracked() || is_publishing.get_untracked() || is_hashing.get_untracked()
        {
            return;
        }
        let epoch = restore_auth.current_auth_flow();
        let response_auth = restore_auth.clone();
        draft_busy.set(true);
        spawn_local(async move {
            let result = crate::tauri_bridge::invoke_publisher_recovery_state(publisher).await;
            if !response_auth.is_current_auth_flow(epoch) {
                return;
            }
            draft_busy.set(false);
            let stored = match result {
                Ok(state) => state
                    .drafts
                    .into_iter()
                    .find(|draft| draft.draft_id == durable_draft_id.get_value()),
                Err(error) => {
                    draft_message.set(Some(error));
                    return;
                }
            };
            let Some(stored) = stored else {
                draft_message.set(Some("No saved draft for this editor and publisher".into()));
                return;
            };
            let snapshot: DurableListingDraft = match serde_json::from_value(stored.payload) {
                Ok(snapshot) => snapshot,
                Err(_) => {
                    draft_message.set(Some("Saved draft is invalid".into()));
                    return;
                }
            };
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
            image_drafts.set(
                snapshot
                    .form
                    .images
                    .into_iter()
                    .filter_map(|url| {
                        url.strip_prefix("hosted:")
                            .map(|url| ListingImageDraft::Hosted(url.to_string()))
                    })
                    .collect(),
            );
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
            draft_revision.set(Some(stored.revision));
            draft_message.set(Some("Draft restored locally, not published. Reselect the pinned archive, revalidate launch paths and review commercial changes before publishing.".into()));
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
                        error_message.set(Some("An earlier operation is incomplete. Save edits as a draft, then use Publisher recovery to reconcile and resume its exact signed events.".into()));
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
                            "The Game page was published before the active account changed, but later results were ignored. Switch to the initiating account and refresh Published games to reconcile uploads."
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
                if let (Some(on_published), Some(listing)) = (on_published, published) {
                    on_published.run(listing);
                }
            });
        },
    );
    let submit_auth = auth.clone();
    let on_submit = Callback::new(move |()| {
        if is_selecting_image.get_untracked() {
            error_message.set(Some("Finish selecting the image before publishing.".into()));
            return;
        }
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
                "Switch to the developer account that published this Game page before updating it."
                    .to_string(),
            ));
            return;
        }

        let title_val = title.get().trim().to_string();
        let description_val = description.get().trim().to_string();
        let lud16_val = lud16.get().trim().to_string();
        let acquisition_kind_val = acquisition_kind.get();
        let price_val =
            match price_for_acquisition(acquisition_kind_val, &price_input.get(), &lud16_val) {
                Ok(price) => price,
                Err(msg) => {
                    error_message.set(Some(msg));
                    return;
                }
            };
        let servers_val = servers.get();
        let file_hash_val = file_hash.get();
        let version_val = version.get();
        let fulfillment_enabled_val = fulfillment_enabled.get();
        let fulfillment_mode_val = if fulfillment_enabled_val {
            fulfillment_mode.get()
        } else {
            FulfillmentMode::None
        };
        let operator_url_val = operator_url.get();
        let acquisition = match acquisition_policy_from_form(
            acquisition_kind_val,
            &acquisition_starts_at.get(),
            &acquisition_ends_at.get(),
        ) {
            Ok(policy) => policy,
            Err(msg) => {
                error_message.set(Some(msg));
                return;
            }
        };

        if let Err(msg) = validate_listing(
            &title_val,
            &description_val,
            price_val,
            &lud16_val,
            fulfillment_enabled_val,
            &servers_val,
            &None,
            &file_hash_val,
            &version_val,
            &fulfillment_mode_val,
            &operator_url_val,
        ) {
            error_message.set(Some(msg));
            return;
        }

        let platforms = match parse_platform_tags(&platforms_input.get()) {
            Ok(platforms) => platforms,
            Err(msg) => {
                error_message.set(Some(msg));
                return;
            }
        };
        let draft_images = image_drafts.get();
        let images = match hosted_image_urls(&draft_images) {
            Ok(images) => images,
            Err(msg) => {
                error_message.set(Some(msg));
                return;
            }
        };
        let pending_images = draft_images
            .iter()
            .filter_map(|image| match image {
                ListingImageDraft::Pending(selection) => Some(selection.clone()),
                ListingImageDraft::Hosted(_) => None,
            })
            .collect::<Vec<_>>();
        let image_publisher_hex = match publisher_hex(&initiating_npub) {
            Some(value) => value,
            None => {
                error_message.set(Some("The active publisher key is invalid.".into()));
                return;
            }
        };
        if !pending_images.is_empty()
            && selected_image_publisher.get_untracked().as_deref()
                != Some(image_publisher_hex.as_str())
        {
            error_message.set(Some(
                "The selected images belong to a different publisher account. Remove and select them again."
                    .into(),
            ));
            return;
        }

        let draft_state = publish_draft.get_untracked();
        let selected_build = fulfillment_enabled_val
            .then(|| draft_state.archive.selected_build.clone())
            .flatten();
        let selection_id = selected_build.as_ref().map(|build| &build.selection_id);
        let launch_tier = match draft_state.launch.to_wire_result(selection_id) {
            Ok(launch_tier) => launch_tier,
            Err(violation) => {
                let error = PublishAdpListingError::CoreValidation {
                    violation,
                    message: "Complete every launch-tier field before publishing.".into(),
                };
                publish_attempt.update(|attempt| attempt.fail(error.clone()));
                error_message.set(Some(publish_error_message(&error)));
                return;
            }
        };
        let extension = PublishAdpListingRequestExtension {
            authorize_operators: authorize_operators.get_untracked(),
            server_roles: servers_val
                .iter()
                .map(|server| {
                    server_roles
                        .get_untracked()
                        .into_iter()
                        .find(|role| role.server_url == server.url)
                        .unwrap_or(arcadestr_core::publisher_management::ServerRoleIntent {
                            server_url: server.url.clone(),
                            issue_receipt: server.url == operator_url.get_untracked(),
                            issue_grant: server.url == operator_url.get_untracked(),
                        })
                })
                .collect(),
            operation_id: None,
            commercial_intent: match reviewed_commercial_intent(
                &commercial_mode.get_untracked(),
                &PublisherTermsContext {
                    publisher_pubkey: image_publisher_hex.clone(),
                    servers: if fulfillment_enabled_val {
                        servers_val.iter().map(|s| s.url.clone()).collect()
                    } else {
                        Vec::new()
                    },
                    price_sats: price_val,
                    developer_destination: if acquisition_kind_val == AcquisitionKind::Gated {
                        lud16_val.clone()
                    } else {
                        String::new()
                    },
                },
                commercial_review.get_untracked(),
                commercial_approved.get_untracked(),
            ) {
                Ok(intent) => intent,
                Err(error) => {
                    error_message.set(Some(error));
                    return;
                }
            },
            launch_tier,
            acknowledged_removed_profile_ids: acknowledged_removed_profile_ids.get_untracked(),
        };
        let mut request = PublishAdpListingRequest {
            expected_publisher_npub: initiating_npub.clone(),
            existing_event_id: published_identity.get_untracked().event_id,
            existing_d_tag: published_identity.get_untracked().d_tag,
            pending_d_tag: published_identity.get_untracked().pending_d_tag,
            title: title_val,
            description: description_val,
            price_sats: price_val,
            lud16: (acquisition_kind_val == AcquisitionKind::Gated).then_some(lud16_val),
            tags: parse_csv_values(&tag_input.get()),
            images,
            fulfillment_mode: fulfillment_mode_val,
            operator_url: when_fulfillment_enabled(
                fulfillment_enabled_val,
                (!operator_url_val.trim().is_empty()).then_some(operator_url_val),
            )
            .flatten(),
            servers: when_fulfillment_enabled(
                fulfillment_enabled_val,
                servers_val.into_iter().map(|entry| entry.url).collect(),
            )
            .unwrap_or_default(),
            accepted_terms: Vec::new(),
            file_path: None,
            selected_build: selected_build.clone(),
            existing_file_hash: (fulfillment_enabled_val && selected_build.is_none())
                .then_some(file_hash_val)
                .flatten(),
            existing_fulfillment_pubkey: when_fulfillment_enabled(
                fulfillment_enabled_val,
                existing_fulfillment_key_for_submit.clone(),
            )
            .flatten(),
            version: when_fulfillment_enabled(
                fulfillment_enabled_val,
                (!version_val.trim().is_empty()).then_some(version_val),
            )
            .flatten(),
            acquisition,
            platforms,
            campaigns: existing_campaigns_for_submit.clone(),
            nip94_event_id: existing_nip94_for_submit.clone(),
        };
        let prepared = match prepare_publish(
            request,
            extension,
            selected_build,
            fulfillment_enabled_val,
            authoritative_file_hash_for_submit.as_deref(),
        ) {
            Ok(prepared) => prepared,
            Err(error) => {
                let typed = PublishAdpListingError::Archive { error };
                publish_attempt.update(|attempt| attempt.fail(typed.clone()));
                error_message.set(Some(publish_error_message(&typed)));
                return;
            }
        };
        let PreparedPublish {
            request: next_request,
            extension,
        } = prepared;
        request = next_request;

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
                    image_upload_status.set(None);
                    return true;
                }
                false
            };
            if stop_stale_preparation() {
                return;
            }
            if !pending_images.is_empty() {
                image_upload_status.set(Some(format!(
                    "Preparing {} image upload(s)",
                    pending_images.len()
                )));
                let settings_result =
                    invoke_get_blossom_server_settings(ExpectedBlossomPublisherRequest {
                        expected_publisher_hex: image_publisher_hex.clone(),
                    })
                    .await;
                if stop_stale_preparation() {
                    return;
                }
                let settings = match settings_result {
                    Ok(settings) if settings.publisher_pubkey == image_publisher_hex => settings,
                    Ok(_) => {
                        publication_state.set(PublicationState {
                            outcome: PublicationOutcome::Failed,
                            listing_published: false,
                            message: Some(
                                "Image upload stopped because the publisher account changed. No game event was published."
                                    .into(),
                            ),
                        });
                        image_upload_status.set(None);
                        is_publishing.set(false);
                        initiating_account.set(None);
                        return;
                    }
                    Err(command_error) => {
                        publication_state.set(PublicationState {
                            outcome: PublicationOutcome::Failed,
                            listing_published: false,
                            message: Some(format!(
                                "Image upload failed: {} No game event was published.",
                                stable_error_message(&command_error.code)
                            )),
                        });
                        image_upload_status.set(None);
                        is_publishing.set(false);
                        initiating_account.set(None);
                        return;
                    }
                };
                let Some(server) = preferred_candidate(&settings) else {
                    image_upload_server.set(None);
                    image_server_error.set(Some(
                        "Add an enabled Blossom server before publishing.".into(),
                    ));
                    current_stage.set(PublishStage::Details);
                    publication_state.set(PublicationState {
                        outcome: PublicationOutcome::Failed,
                        listing_published: false,
                        message: Some(
                            "Image upload paused: add a Blossom server in Cover & screenshots. No game event was published."
                                .into(),
                        ),
                    });
                    image_upload_status.set(None);
                    is_publishing.set(false);
                    initiating_account.set(None);
                    return;
                };
                let upload_started = js_sys::Date::now().max(0.0) as u64;
                for (index, selection) in pending_images.iter().enumerate() {
                    if !response_auth.is_current_auth_flow(submit_epoch)
                        || auth.npub.get_untracked().as_deref() != Some(initiating_npub.as_str())
                    {
                        publication_state.set(PublicationState {
                            outcome: PublicationOutcome::Failed,
                            listing_published: false,
                            message: Some(
                                "The active account changed during image upload. No game event was published."
                                    .into(),
                            ),
                        });
                        image_upload_status.set(None);
                        is_publishing.set(false);
                        initiating_account.set(None);
                        return;
                    }
                    image_upload_status.set(Some(format!(
                        "Uploading image {} of {}: {}",
                        index + 1,
                        pending_images.len(),
                        selection.filename
                    )));
                    let response = invoke_start_blossom_upload(StartBlossomUploadRequest {
                        selection_id: selection.selection_id.clone(),
                        expected_publisher_hex: image_publisher_hex.clone(),
                        selected_server: Some(server.clone()),
                        preflight: true,
                        request_id: fresh_request_id(upload_started, index as u64 + 1),
                    })
                    .await;
                    if stop_stale_preparation() {
                        return;
                    }
                    let response = match response {
                        Ok(response)
                            if response.mime_type == selection.detected_mime
                                && listing_image_mime(&response.mime_type)
                                && is_http_url(&response.url) =>
                        {
                            response
                        }
                        Ok(_) => {
                            publication_state.set(PublicationState {
                                outcome: PublicationOutcome::Failed,
                                listing_published: false,
                                message: Some(
                                    "Image upload returned invalid media metadata. No game event was published."
                                        .into(),
                                ),
                            });
                            image_upload_status.set(None);
                            is_publishing.set(false);
                            initiating_account.set(None);
                            return;
                        }
                        Err(command_error) => {
                            publication_state.set(PublicationState {
                                outcome: PublicationOutcome::Failed,
                                listing_published: false,
                                message: Some(format!(
                                    "Image upload failed: {} No game event was published.",
                                    stable_error_message(&command_error.code)
                                )),
                            });
                            image_upload_status.set(None);
                            is_publishing.set(false);
                            initiating_account.set(None);
                            return;
                        }
                    };
                    image_drafts.update(|images| {
                        if let Some(image) = images.iter_mut().find(|image| {
                            matches!(image, ListingImageDraft::Pending(pending) if pending.selection_id == selection.selection_id)
                        }) {
                            *image = ListingImageDraft::Hosted(response.url.clone());
                        }
                    });
                }
                image_upload_status.set(None);
            }
            request.images = match hosted_image_urls(&image_drafts.get_untracked()) {
                Ok(images) => images,
                Err(error) => {
                    publication_state.set(PublicationState {
                        outcome: PublicationOutcome::Failed,
                        listing_published: false,
                        message: Some(format!(
                            "Image upload returned an invalid URL: {error}. No game event was published."
                        )),
                    });
                    is_publishing.set(false);
                    initiating_account.set(None);
                    return;
                }
            };
            if has_pending_images(&image_drafts.get_untracked()) {
                publication_state.set(PublicationState {
                    outcome: PublicationOutcome::Failed,
                    listing_published: false,
                    message: Some(
                        "An image selection changed during publication. No game event was published."
                            .into(),
                    ),
                });
                is_publishing.set(false);
                initiating_account.set(None);
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
                "The form, archive, distribution settings, account, or authoritative revision changed after the backend check. The frozen confirmation was voided; submit again."
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
                "The form, archive, distribution settings, account, or authoritative revision changed after the backend check. The frozen confirmation was voided; submit again."
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
        let selections = image_drafts
            .get_untracked()
            .into_iter()
            .filter_map(|image| match image {
                ListingImageDraft::Pending(selection) => Some(selection),
                ListingImageDraft::Hosted(_) => None,
            })
            .collect::<Vec<_>>();
        let publisher = selected_image_publisher.get_untracked();
        if let Some(expected_publisher_hex) = publisher {
            spawn_local(async move {
                for selection in selections {
                    let _ = invoke_discard_blossom_media_selection(DiscardBlossomMediaRequest {
                        selection_id: selection.selection_id,
                        expected_publisher_hex: expected_publisher_hex.clone(),
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
            &hosted_image_urls(&image_drafts.get())
                .unwrap_or_default()
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
    let is_dirty = move || {
        form_snapshot() != initial_form_snapshot.get_value()
            || distribution_snapshot_for_dirty() != initial_distribution_snapshot.get()
    };

    // Publish Game has no draft persistence. Share the complete create/edit
    // dirty snapshot with the shell and clear it when this form unmounts.
    let is_dirty_for_guard = is_dirty.clone();
    Effect::new(move |_| {
        crate::ui_v2::components::set_publish_game_dirty(is_dirty_for_guard());
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
    let workflow_kind = if editing {
        "Update game page"
    } else {
        "Create game"
    };

    view! {
        <main class="v2-publisher-studio v2-create-workflow">
            <header class="v2-create-header">
                <h1 class="v2-create-title">{move || format!("{workflow_kind} · Step {} of {} — {}", current_stage.get().index() + 1, PublishStage::ALL.len(), current_stage.get().title())}</h1>
                <p class="v2-create-subtitle">{move || match current_stage.get() {
                    PublishStage::Details => "This builds the buyer-facing game page metadata published in the listing.",
                    PublishStage::Pricing => "A zero price never enables public access on its own — choose the access model explicitly.",
                    PublishStage::Builds => "Build verification is computed from the selected file; there is no freeform hash field.",
                    PublishStage::Review => "Review the authored values and active-account authorization before network publication.",
                }}</p>
            </header>

            <nav class="v2-create-stages" aria-label="Create Game stages">
                <ol>
                    {PublishStage::ALL.into_iter().map(|stage| {
                        let status = move || stage_status(stage, current_stage.get(), validate_stage(stage).is_ok());
                        view! {
                            <li>
                                <button
                                    type="button"
                                    class=move || format!("v2-create-stage {}", status().class())
                                    aria-current=move || (current_stage.get() == stage).then_some("step")
                                    disabled=move || !stage_reachable(stage, current_stage.get()) || is_publishing.get()
                                    title=move || (!stage_reachable(stage, current_stage.get())).then_some("Complete the current stage first.")
                                    on:click=move |_| { current_stage.set(stage); stage_error.set(None); }
                                >
                                    <span class="v2-create-stage-index">{format!("Step {}", stage.index() + 1)}</span>
                                    <span class="v2-create-stage-title">{stage.title()}</span>
                                    <span class="v2-create-stage-status">{move || status().label()}</span>
                                </button>
                            </li>
                        }
                    }).collect_view()}
                </ol>
            </nav>

            {move || stage_error.get().map(|msg| view! { <div id="publish-stage-error" role="alert" class="v2-create-alert v2-create-alert-error">{msg}</div> })}
            {move || error_message.get().map(|msg| view! { <div role="alert" class="v2-create-alert v2-create-alert-error">{msg}</div> })}
            <section style="padding:14px;border:1px solid oklch(0.32 0.01 60);background:#121215;font:400 12.5px/1.6 ui-monospace,monospace;color:#fff">
                <button type="button" style="padding:10px;color:oklch(0.8 0.13 210);background:transparent;border:1px solid oklch(0.5 0.13 210);font-family:ui-monospace,monospace" disabled=move || draft_busy.get() || is_publishing.get() on:click=move |_| on_save_draft.run(())>"SAVE DRAFT"</button>
                <button type="button" style="padding:10px;color:oklch(0.8 0.13 210);background:transparent;border:1px solid oklch(0.5 0.13 210);font-family:ui-monospace,monospace" disabled=move || draft_busy.get() || is_publishing.get() || is_hashing.get() on:click=move |_| on_restore_draft.run(())>"RESTORE SAVED DRAFT (REPLACES FORM)"</button>
                {move || draft_message.get().map(|message| view! { <p role="status">{message}</p> })}
            </section>
            {(!SUPPORTS_DRAFTS).then(|| view! {
                <p class="v2-create-persistence-note">"Arcadestr does not store Create Game drafts. Nothing here is saved locally or on a relay until network publication succeeds; leaving this workflow discards it."</p>
            })}

            <Dialog
                id="publish-profile-removal-confirmation"
                open=Signal::derive(move || pending_removal_confirmation.get().is_some())
                title=Signal::derive(move || pending_removal_confirmation.get().map(|pending| {
                    (if pending.abort_count > 1 {
                        "Removal set changed — confirm again"
                    } else {
                        "Confirm removed launch profiles"
                    })
                    .to_string()
                }).unwrap_or_default())
                kicker="Publication blocked"
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
                    >"Confirm removals and publish"</Button>
                }
            >
                <div role="alert" class="v2-create-alert v2-create-alert-error">
                    <p>"The backend stopped before publication. Confirming resubmits the exact frozen review payload and only acknowledges these removals:"</p>
                    <ul>
                        {move || pending_removal_confirmation.get().map(|pending| pending.removed_profile_ids.into_iter().map(|id| view! {
                            <li><strong>{id.as_str().to_string()}</strong>": installed clients may have this profile ID pinned and will fall back after publication."</li>
                        }).collect_view())}
                    </ul>
                    <p>"Closing this dialog voids this confirmation. Any form, archive, profile, distribution, account, or revision change requires a new submit and backend check."</p>
                </div>
            </Dialog>

            <div class="v2-create-surface">
                    <Show when=move || current_stage.get() == PublishStage::Details>
                    <section class="v2-create-stage-panel" aria-labelledby="publish-details-title">
                        <h2 id="publish-details-title" class="v2-create-section-title">"Game details"</h2>
                        <div class="v2-create-fields">
                            <div>
                                <label for="publish-title" class="v2-create-field-label">"Title (required)"</label>
                                <input id="publish-title" required=true class="v2-input" placeholder="Neon Drifter" prop:value={move || title.get()} on:input:target=move |ev| title.set(ev.target().value()) disabled={move || is_publishing.get()} />
                            </div>
                            <div>
                                <label for="publish-description" class="v2-create-field-label">"Description (required)"</label>
                                <textarea id="publish-description" required=true class="v2-input" rows=5 placeholder="Tell players about your game..." prop:value={move || description.get()} on:input:target=move |ev| description.set(ev.target().value()) disabled={move || is_publishing.get()} />
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

                    <section class="v2-create-stage-panel" aria-labelledby="publish-images-title">
                        <div class="v2-create-panel-heading">
                            <span class="material-symbols-outlined flex h-9 w-9 items-center justify-center rounded-lg bg-secondary/10 text-xl text-secondary" aria-hidden="true">"image"</span>
                            <h2 id="publish-images-title" class="v2-create-section-title">"Cover & screenshots"</h2>
                        </div>
                        {move || {
                            match image_drafts.get().first().cloned() {
                                Some(image) => {
                                    let (src, title, detail) = match image {
                                        ListingImageDraft::Hosted(url) => (url, "Hosted cover".to_string(), "Published image".to_string()),
                                        ListingImageDraft::Pending(selection) => (
                                            selection.preview_data_url.unwrap_or_default(),
                                            selection.filename,
                                            format!("{} · Local preview", human_image_size(selection.size)),
                                        ),
                                    };
                                    view! {
                                        <article class="v2-create-cover">
                                            <img class="v2-create-media-image" src=src alt="Game cover preview" on:error=use_fallback_cover />
                                            <div class="v2-create-cover-overlay">
                                                <div class="v2-create-media-meta"><p class="v2-create-media-name">{title}</p><p class="v2-create-media-detail">{detail}</p></div>
                                                <div class="v2-create-media-actions">
                                                    <button type="button" class="v2-create-media-button" on:click=move |_| on_select_image.run(0) disabled=move || is_selecting_image.get() || is_publishing.get()>"Replace"</button>
                                                    <button type="button" class="v2-create-media-button v2-create-media-remove" on:click=move |_| remove_image.run(0) disabled=move || is_publishing.get()>"Remove"</button>
                                                </div>
                                            </div>
                                        </article>
                                    }.into_any()
                                }
                                None => view! {
                                    <button type="button" class="v2-create-cover v2-create-cover-empty" on:click=move |_| on_select_image.run(0) disabled=move || is_selecting_image.get() || is_publishing.get()>
                                        <span><span class="material-symbols-outlined v2-create-drop-icon" aria-hidden="true">"arrow_upward"</span><span class="v2-create-drop-title">{move || if is_selecting_image.get() { "Selecting image..." } else { "Hero banner upload" }}</span><span class="v2-create-help">"Recommended: 1920×1080px"</span></span>
                                    </button>
                                }.into_any(),
                            }
                        }}
                        <div class="v2-create-shot-grid">
                            {move || {
                                let images = image_drafts.get();
                                let screenshots_disabled = images.is_empty();
                                (0..screenshot_slot_count(images.len())).map(|slot| {
                                    let image_index = slot + 1;
                                    match images.get(image_index).cloned() {
                                        Some(image) => {
                                            let (src, title) = match image {
                                                ListingImageDraft::Hosted(url) => (url, "Hosted screenshot".to_string()),
                                                ListingImageDraft::Pending(selection) => (selection.preview_data_url.unwrap_or_default(), selection.filename),
                                            };
                                            view! {
                                                <article class="v2-create-shot">
                                                    <img class="v2-create-media-image" src=src alt=format!("Game screenshot {}", image_index) on:error=use_fallback_cover />
                                                    <div class="v2-create-shot-overlay">
                                                        <p class="v2-create-media-name">{title}</p>
                                                        <div class="v2-create-media-actions"><button type="button" class="v2-create-media-link" on:click=move |_| on_select_image.run(image_index) disabled=move || is_selecting_image.get() || is_publishing.get()>"Replace"</button><button type="button" class="v2-create-media-link v2-create-media-remove" on:click=move |_| remove_image.run(image_index) disabled=move || is_publishing.get()>"Remove"</button></div>
                                                    </div>
                                                </article>
                                            }.into_any()
                                        }
                                        None => view! {
                                            <button type="button" class="v2-create-shot v2-create-shot-empty" aria-label="Add screenshot" on:click=move |_| on_select_image.run(image_index) disabled=move || screenshots_disabled || is_selecting_image.get() || is_publishing.get()>
                                                <span class="material-symbols-outlined" aria-hidden="true">"add"</span>
                                            </button>
                                        }.into_any(),
                                    }
                                }).collect_view()
                            }}
                        </div>
                        <Show when=move || has_pending_images(&image_drafts.get()) && image_upload_server.get().is_none()>
                            <section class="v2-create-subpanel" aria-labelledby="publish-blossom-server-title">
                                <div class="v2-create-subpanel-row">
                                    <span class="material-symbols-outlined v2-create-subpanel-icon" aria-hidden="true">"cloud_upload"</span>
                                    <div class="v2-create-subpanel-body">
                                        <h3 id="publish-blossom-server-title" class="v2-create-subsection-title">"Add a Blossom upload server"</h3>
                                        <p class="v2-create-help">"Your images need a Blossom server. Add one here; it will also be saved to your Store Page media settings."</p>
                                        <div class="v2-create-inline-row">
                                            <label for="publish-blossom-server" class="sr-only">"Blossom server URL"</label>
                                            <input id="publish-blossom-server" type="url" class="v2-input" placeholder="https://blossom.example" prop:value=move || image_server_origin.get() on:input:target=move |event| { image_server_origin.set(event.target().value()); image_server_error.set(None); } disabled=move || is_saving_image_server.get() || is_publishing.get() />
                                            <button type="button" class="v2-btn-secondary" on:click=move |_| save_image_server.run(()) disabled=move || image_server_origin.get().trim().is_empty() || is_saving_image_server.get() || is_publishing.get()>{move || if is_saving_image_server.get() { "Saving…" } else { "Save server" }}</button>
                                        </div>
                                        {move || image_server_error.get().map(|error| view! { <p class="v2-create-inline-error" role="alert">{error}</p> })}
                                    </div>
                                </div>
                            </section>
                        </Show>
                        {move || image_upload_server.get().filter(|_| has_pending_images(&image_drafts.get())).map(|server| view! {
                            <p class="v2-create-inline-note"><span class="material-symbols-outlined v2-create-inline-icon" aria-hidden="true">"cloud_done"</span><span>"Images will upload to "<span class="v2-create-mono-inline">{server}</span>" when you publish."</span></p>
                        })}
                        <p class="v2-create-help">"JPEG, PNG, or WebP. Images stay local until you publish."</p>
                    </section>
                    </Show>

                    <Show when=move || current_stage.get() == PublishStage::Builds>
                    <section class="v2-create-stage-panel" aria-labelledby="publish-builds-title">
                        <h2 id="publish-builds-title" class="v2-create-section-title">"Builds and distribution"</h2>
                        <p class="v2-create-help">"Enable automated installation only when you have a build and distribution provider. Otherwise this remains a metadata-only game page."</p>
                        <p class="v2-create-help">{build_capability_message()}</p>
                        <label class="v2-create-toggle">
                             <input type="checkbox" checked={move || fulfillment_enabled.get()} disabled=move || is_publishing.get() || existing_fulfillment_locked on:change:target=move |ev| {
                                 let enabled = ev.target().checked();
                                 fulfillment_enabled.set(enabled);
                                  if enabled && fulfillment_mode.get_untracked() == FulfillmentMode::None {
                                     fulfillment_mode.set(FulfillmentMode::Delegate);
                                  } else if !enabled {
                                     fulfillment_mode.set(FulfillmentMode::None);
                                  }
                                  distribution_user_revision.update(|revision| *revision = revision.wrapping_add(1));
                             } />
                            <span class="v2-create-toggle-label">"Enable automated installation"</span>
                        </label>
                        {existing_fulfillment_locked.then(|| view! { <p class="v2-create-help">"This existing publishing authorization cannot be removed by an ordinary Game page update. Keep the current mode to reuse its key, or deliberately choose another mode to replace the authorization."</p> })}

                        <Show when=move || fulfillment_enabled.get()>
                            <div class="v2-create-fields">
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
                                        <span class=move || if fulfillment_mode.get() == FulfillmentMode::Delegate { "v2-create-mode-title v2-create-mode-title-active" } else { "v2-create-mode-title" }>"Authorize a distribution provider"</span>
                                        <span class=move || if fulfillment_mode.get() == FulfillmentMode::Delegate { "v2-create-selected-badge" } else { "v2-create-selected-badge v2-create-selected-badge-idle" }>
                                            {move || if fulfillment_mode.get() == FulfillmentMode::Delegate { "Selected" } else { "Switch to provider" }}
                                        </span>
                                    </span>
                                    <span class="v2-create-help">"Recommended. The provider manages fulfillment authorization and build distribution for this game."</span>
                                    <Show when=move || fulfillment_mode.get() == FulfillmentMode::Delegate>
                                        <span class="v2-create-mono-value">{move || operator_url.get()}</span>
                                        <Show when=move || operator_auto_added.get().is_some()>
                                            <span class="v2-create-help">"This provider is also included as a distribution server."</span>
                                        </Show>
                                    </Show>
                                </button>

                                <details class="v2-create-disclosure">
                                    <summary class="v2-create-disclosure-summary">
                                        {move || if fulfillment_mode.get() == FulfillmentMode::Direct { "Advanced distribution options - direct account signing selected" } else { "Advanced distribution options" }}
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
                                                <span class=move || if fulfillment_mode.get() == FulfillmentMode::Direct { "v2-create-mode-title v2-create-mode-title-active" } else { "v2-create-mode-title" }>"Use my active account instead"</span>
                                                <Show when=move || fulfillment_mode.get() == FulfillmentMode::Direct>
                                                    <span class="v2-create-selected-badge">"Selected"</span>
                                                </Show>
                                            </span>
                                            <span class="v2-create-help">"Sign fulfillment directly without provisioning a provider key."</span>
                                        </button>

                                        <Show when=move || matches!(fulfillment_mode.get(), FulfillmentMode::Delegate)>
                                            <div class="v2-create-subpanel v2-create-fields">
                                                <label for="publish-provider-url" class="v2-create-field-label">"Use a different distribution provider"</label>
                                                <div class="v2-create-inline-row">
                                                    <input id="publish-provider-url" class="v2-input" placeholder="https://provider.example.com" prop:value={move || operator_url.get()} on:input:target=move |ev| {
                                                         let next = ev.target().value();
                                                         operator_url.set(next.clone());
                                                         distribution_user_revision.update(|revision| *revision = revision.wrapping_add(1));
                                                        if operator_auto_added.get_untracked().is_some() { sync_operator_server(next); }
                                                    } />
                                                    <select aria-label="Copy provider URL from a selected server" class="v2-input" on:change:target=move |ev| {
                                                        let selected = ev.target().value();
                                                        if !selected.is_empty() {
                                                             operator_url.set(selected.clone());
                                                             distribution_user_revision.update(|revision| *revision = revision.wrapping_add(1));
                                                            if operator_auto_added.get_untracked().is_some() { sync_operator_server(selected); }
                                                        }
                                                    }>
                                                        <option value="">"Copy from server"</option>
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
                                                    "Also add this provider as a distribution server"
                                                </label>
                                            </div>
                                        </Show>

                                <div class="v2-create-subpanel v2-create-fields">
                                    <div class="v2-create-row-between">
                                         <h3 class="v2-create-subsection-title">"Discovered distribution providers"</h3>
                                        <span class="v2-create-help">"Live relay query; manual entry still works if discovery fails."</span>
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
                                                    <span class="v2-create-help">{server.supported_adp.unwrap_or_default()}</span>
                                                </label>
                                            }
                                        }).collect_view()}
                                    </div>
                                    <div class="v2-create-inline-row">
                                        <label for="publish-custom-server" class="sr-only">"Custom distribution server URL"</label>
                                        <input id="publish-custom-server" class="v2-input" placeholder="Add custom server URL" prop:value={move || custom_server.get()} on:input:target=move |ev| custom_server.set(ev.target().value()) />
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
                                                         <p class={reachability.class()}>{format!("Provider check: {}", reachability.label())}</p>
                                                         <p class={upload.class()}>{format!("Build upload: {}", upload.label())}</p>
                                                    </div>
                                                    <button type="button" class="v2-create-media-link v2-create-media-remove" on:click=move |_| remove_server(url.clone())>"Remove"</button>
                                                </div>
                                            }
                                        }).collect_view()}
                                    </div>
                                    <Show when=move || fulfillment_mode.get() == FulfillmentMode::Delegate>
                                        <div style="padding:14px;background:#121215;border:1px solid oklch(0.32 0.01 60);font:400 12.5px/1.6 ui-monospace,monospace;color:#fff">
                                            <p style="color:oklch(0.8 0.16 60);font-weight:800">"OPERATOR ISSUING ROLES"</p>
                                            <p>"Every selected operator gets upload-build only unless you authorize receipt or grant issuance below. Aliases of the same operator must use identical roles. Metadata edits preserve existing authority; use management to cancel it."</p>
                                            <label><input type="checkbox" prop:checked=move || authorize_operators.get() on:change:target=move |event| authorize_operators.set(event.target().checked()) />" AUTHORIZE NEW OPERATORS ON THIS EDIT (does not renew cancelled authority or expand existing capabilities)"</label>
                                            {move || servers.get().into_iter().map(|server| {
                                                let url = server.url;
                                                let display = url.clone();
                                                let receipt_url = url.clone(); let grant_url = url.clone();
                                                let receipt_checked = url.clone(); let grant_checked = url.clone();
                                                view! { <div style="border-top:1px solid oklch(0.32 0.01 60);padding:10px;overflow-wrap:anywhere">
                                                    <p>{display}</p>
                                                    <label><input type="checkbox" prop:checked=move || server_roles.get().iter().find(|role| role.server_url == receipt_checked).map(|role| role.issue_receipt).unwrap_or_else(|| receipt_checked == operator_url.get()) on:change:target=move |event| { let checked=event.target().checked(); server_roles.update(|roles| { if let Some(role)=roles.iter_mut().find(|role| role.server_url == receipt_url) { role.issue_receipt=checked; } else { roles.push(arcadestr_core::publisher_management::ServerRoleIntent { server_url:receipt_url.clone(),issue_receipt:checked,issue_grant:receipt_url==operator_url.get_untracked() }); } }); } />" ISSUE RECEIPTS "</label>
                                                    <label><input type="checkbox" prop:checked=move || server_roles.get().iter().find(|role| role.server_url == grant_checked).map(|role| role.issue_grant).unwrap_or_else(|| grant_checked == operator_url.get()) on:change:target=move |event| { let checked=event.target().checked(); server_roles.update(|roles| { if let Some(role)=roles.iter_mut().find(|role| role.server_url == grant_url) { role.issue_grant=checked; } else { roles.push(arcadestr_core::publisher_management::ServerRoleIntent { server_url:grant_url.clone(),issue_receipt:grant_url==operator_url.get_untracked(),issue_grant:checked }); } }); } />" ISSUE GRANTS"</label>
                                                </div> }
                                            }).collect_view()}
                                        </div>
                                    </Show>
                                    <div style="clip-path:polygon(0 0,calc(100% - 12px) 0,100% 12px,100% 100%,12px 100%,0 calc(100% - 12px));border:1px solid oklch(0.32 0.01 60);background:#121215;padding:18px;display:grid;gap:12px;font-family:ui-monospace,monospace">
                                        <div style="display:flex;align-items:center;gap:10px">
                                            <span style="font:800 12px ui-monospace,monospace;letter-spacing:1.5px;color:oklch(0.8 0.16 60)">"◆ SERVICE FEE CONSENT"</span>
                                            <span style="flex:1;height:1px;background:repeating-linear-gradient(90deg,oklch(0.4 0.03 60 / .6) 0 4px,transparent 4px 8px)"></span>
                                        </div>
                                        <label style="color:#fff;font:600 12.5px/1.6 ui-monospace,monospace">"COMMERCIAL INTENTION"
                                            <select aria-label="Commercial intention" style="display:block;width:100%;background:#141417;color:#fff;border:1px solid oklch(0.32 0.01 60);padding:10px;font-family:ui-monospace,monospace" prop:value=move || commercial_mode.get() disabled=move || is_publishing.get() on:change:target=move |event| {
                                                commercial_mode.set(event.target().value());
                                                commercial_review.set(None);
                                                commercial_approved.set(false);
                                                commercial_loading.set(false);
                                                commercial_generation.update(|value| *value = value.wrapping_add(1));
                                            }>
                                                <option value="preserve">"PRESERVE EXACT ACCEPTANCE"</option>
                                                <option value="replace">"REPLACE AFTER REVIEW"</option>
                                                <option value="retire" disabled=true>"RETIRE — UNAVAILABLE"</option>
                                            </select>
                                        </label>
                                        <p style="margin:0;color:oklch(0.68 0.01 60);font:400 12.5px/1.6 ui-monospace,monospace;overflow-wrap:anywhere">{move || format!("Existing acceptance: {}", preserved_terms_label.get_value())}</p>
                                        <Show when=move || commercial_mode.get() == "replace">
                                            <button type="button" style="padding:10px;background:transparent;border:1px solid oklch(0.5 0.13 210);color:oklch(0.8 0.13 210);font:800 12px ui-monospace,monospace" disabled=move || commercial_loading.get() || is_publishing.get() on:click=move |_| on_review_terms.run(())>{move || if commercial_loading.get() { "VERIFYING TERMS…" } else { "RETRIEVE SIGNED TERMS >" }}</button>
                                            {move || commercial_review.get().map(|review| view! {
                                                <div style="display:grid;gap:10px;color:oklch(0.68 0.01 60);font:400 12.5px/1.6 ui-monospace,monospace;overflow-wrap:anywhere" aria-live="polite">
                                                    <p>{format!("Gross: {} msat", review.context.price_sats.saturating_mul(1000))}</p>
                                                    {review.offers.into_iter().zip(review.accepted_terms).map(|(offer, reference)| view! {
                                                        <div style="border-top:1px solid oklch(0.4 0.03 60 / .3);padding-top:8px">
                                                            <p>{format!("Operator {} · terms {} · settlement v{}", offer.operator_pubkey, reference.terms_event_id, offer.version)}</p>
                                                            <p>{format!("{} · destination {}", fee_policy_label(&offer.fee), offer.payment_destination)}</p>
                                                            <p>{format!("Operator limitations: {}", if offer.limitations.is_empty() { "None advertised".into() } else { offer.limitations.join("; ") })}</p>
                                                        </div>
                                                    }).collect_view()}
                                                    {review.legs.into_iter().map(|leg| view! { <p>{format!("{:?}: {} → {} · {} msat", leg.role, leg.recipient_pubkey, leg.destination, leg.amount_msat)}</p> }).collect_view()}
                                                </div>
                                            })}
                                            <label style="display:flex;align-items:flex-start;gap:10px;color:#fff;font:600 12.5px/1.6 ui-monospace,monospace">
                                                <input type="checkbox" disabled=move || commercial_review.get().is_none() || is_publishing.get() prop:checked=move || commercial_approved.get() on:change:target=move |event| commercial_approved.set(event.target().checked()) />
                                                <span>"I approve these exact signed terms, recipients and allocation for this listing. Changes require a new review."</span>
                                            </label>
                                        </Show>
                                        {move || commercial_error.get().map(|error| view! { <p role="alert" style="color:oklch(0.65 0.18 25);font:400 12.5px/1.6 ui-monospace,monospace">{error}</p> })}
                                        <p style="margin:0;color:oklch(0.68 0.01 60);font:400 12.5px/1.6 ui-monospace,monospace">"Preserve never accepts new advertisements. Withdrawal checks cover available configured-relay evidence, not global history. Historical sales and receipts are unchanged. Operator retirement is unavailable pending an agreed prospective workflow."</p>
                                    </div>
                                </div>
                                    </div>
                                </details>

                                 <div class="v2-create-field-pair">
                                    <div>
                                        <span id="publish-build-label" class="v2-create-field-label">"Build file (required for automated installation)"</span>
                                        <button type="button" aria-labelledby="publish-build-label" aria-describedby="publish-hash-status" class="v2-create-file-button" on:click={on_select_file} disabled={move || is_hashing.get() || is_publishing.get()}>
                                            {move || if is_hashing.get() { "Inspecting and hashing…".to_string() } else { publish_draft.get().archive.selection.map(|selection| selection.safe_filename.as_str().to_string()).unwrap_or_else(|| if file_hash.get().is_some() { "Using existing build — select replacement".to_string() } else { "Select archive".to_string() }) }}
                                        </button>
                                        <p id="publish-hash-status" aria-live="polite" class="v2-create-help">{move || if is_hashing.get() { "Inspecting the backend-owned archive and computing SHA-256…".to_string() } else { file_hash.get().map(|hash| format!("Read-only SHA-256: {}", format_sha256(&hash))).unwrap_or_else(|| "Hash appears after archive inspection.".to_string()) }}</p>
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
                    </section>
                    </Show>

                    <Show when=move || current_stage.get() == PublishStage::Pricing>
                    <section class="v2-create-stage-panel" aria-labelledby="publish-pricing-title">
                        <h2 id="publish-pricing-title" class="v2-create-section-title">"Pricing and access"</h2>
                        <p class="v2-create-help">"Paid access is the default. Select public or timed access only when the game should be available without an entitlement."</p>
                        <div class="v2-create-fields">
                            <div>
                                <label class="v2-create-field-label" for="acquisition-policy">"Access model"</label>
                                <select id="acquisition-policy" class="v2-input" on:change:target=move |ev| acquisition_kind.set(AcquisitionKind::from_value(&ev.target().value())) disabled=move || is_publishing.get()>
                                    <option value="gated" selected=move || acquisition_kind.get() == AcquisitionKind::Gated>"Paid access (default)"</option>
                                    <option value="public" selected=move || acquisition_kind.get() == AcquisitionKind::Public>"Public access"</option>
                                    <option value="timed-access" selected=move || acquisition_kind.get() == AcquisitionKind::TimedAccess>"Timed public access"</option>
                                </select>
                                <p class="v2-create-help">{move || match acquisition_kind.get() {
                                    AcquisitionKind::Gated => "A positive price and Lightning address are required.",
                                    AcquisitionKind::Public => "Anyone can access the game. Its published price is 0 sats.",
                                    AcquisitionKind::TimedAccess => "Anyone can access the game during the selected window. Its published price is 0 sats.",
                                }}</p>
                            </div>
                            <Show when=move || acquisition_kind.get() == AcquisitionKind::Gated>
                                <div>
                                    <label for="publish-price" class="v2-create-field-label">"Price in sats (required)"</label>
                                    <input id="publish-price" required=true class="v2-input" type="number" min=1 step=1 prop:value={move || price_input.get()} on:input:target=move |ev| price_input.set(ev.target().value()) />
                                </div>
                                <div>
                                    <label for="publish-lud16" class="v2-create-field-label">"Lightning address (required)"</label>
                                    <input id="publish-lud16" required=true class="v2-input" placeholder="you@example.com" prop:value={move || lud16.get()} on:input:target=move |ev| lud16.set(ev.target().value()) />
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

                    <Show when=move || current_stage.get() == PublishStage::Review>
                    <section class="v2-create-stage-panel" aria-labelledby="publish-review-title">
                        <h2 id="publish-review-title" class="v2-create-section-title">"Review and publish"</h2>

                        <article class="v2-create-summary-card">
                            <p class="v2-create-summary-title">{move || { let value = title.get(); if value.trim().is_empty() { "Untitled game page".to_string() } else { value } }}</p>
                            <p class="v2-create-summary-meta">{move || format!(
                                "{} · {} · {}",
                                match acquisition_kind.get() {
                                    AcquisitionKind::Gated => format!("{} sats", price_input.get()),
                                    AcquisitionKind::Public => "Public access".to_string(),
                                    AcquisitionKind::TimedAccess => "Timed public access".to_string(),
                                },
                                if version.get().trim().is_empty() { "no version declared".to_string() } else { format!("v{}", version.get().trim()) },
                                platform_summary(&platforms_input.get()),
                            )}</p>
                        </article>

                        <dl class="v2-create-review-facts">
                            {move || {
                                let identity = published_identity.get();
                                identity.d_tag.or(identity.pending_d_tag).map(|d_tag| view! {
                                    <div><dt>"Identifier"</dt><dd>{d_tag}</dd></div>
                                })
                            }}
                            <div><dt>"Metadata"</dt><dd>{move || format!("{} tags · {} images", parse_csv_values(&tag_input.get()).len(), image_drafts.get().len())}</dd></div>
                            <div><dt>"Description"</dt><dd class="v2-create-review-prewrap">{move || description.get()}</dd></div>
                            <div><dt>"Pricing"</dt><dd>{move || match acquisition_kind.get() { AcquisitionKind::Gated => format!("{} sats via {}", price_input.get(), lud16.get()), AcquisitionKind::Public | AcquisitionKind::TimedAccess => "Not for sale (0 sats)".to_string() }}</dd></div>
                            <div><dt>"Current access"</dt><dd>{move || match acquisition_kind.get() { AcquisitionKind::Gated => "Paid".to_string(), AcquisitionKind::Public => "Public".to_string(), AcquisitionKind::TimedAccess => format!("Timed from {} to {} (local)", acquisition_starts_at.get(), acquisition_ends_at.get()) }}</dd></div>
                            <div><dt>"Platforms / version"</dt><dd>{move || format!("{} / {}", platform_summary(&platforms_input.get()), version.get())}</dd></div>
                            <div><dt>"Distribution provider/server"</dt><dd>{move || if fulfillment_enabled.get() { format!("{} server(s); {}", servers.get().len(), operator_url.get()) } else { "Metadata only".to_string() }}</dd></div>
                            <div><dt>"Build archive / hash"</dt><dd>{move || format!("{} / {}", publish_draft.get().archive.selection.map(|selection| selection.safe_filename.as_str().to_string()).unwrap_or_else(|| "existing or none".into()), file_hash.get().map(|hash| format_sha256(&hash)).unwrap_or_else(|| "none".into()))}</dd></div>
                            <div><dt>"Authorization"</dt><dd>{move || match fulfillment_mode.get() { FulfillmentMode::None => "Game page only", FulfillmentMode::Direct => "Active account", FulfillmentMode::Delegate => "Distribution provider" }}</dd></div>
                            <div><dt>"Promotion links"</dt><dd>{format!("{} preserved", existing_campaigns.len())}</dd></div>
                            <div><dt>"Publishing authorization key"</dt><dd>{existing_fulfillment_pubkey.clone().unwrap_or_else(|| "Prepared during publication when required".into())}</dd></div>
                        </dl>

                        <super::publish_launch::PublishLaunchReview
                            state=publish_draft
                            artifact_enabled=Signal::derive(move || fulfillment_enabled.get())
                            existing_artifact_digest=Signal::derive(move || {
                                fulfillment_enabled.get().then(|| file_hash.get()).flatten()
                            })
                            surfaced_warnings=surfaced_archive_warnings
                        />

                        <div class="v2-create-authorization">
                            <p class="v2-create-field-label">"Publishing authorization"</p>
                            <p class="v2-create-account">{move || auth.npub.get().unwrap_or_else(|| "Not signed in".to_string())}</p>
                            <p class="v2-create-help" id="publish-signer-requirement">{move || signer_requirement_message(signer_state.get())}</p>
                            {move || (!signed_in.get()).then(|| view! { <p class="v2-create-alert v2-create-alert-error" role="alert">"Sign in before starting network publication."</p> })}
                        </div>

                        <p class="v2-create-kicker">"Readiness checklist"</p>
                        <ul class="v2-create-checklist">
                            <li class=move || if checklist().metadata { "v2-create-check-ok" } else { "v2-create-check-blocked" }>{move || if checklist().metadata { "Game details complete" } else { "Game details need attention" }}</li>
                            <li class=move || if checklist().pricing_and_access { "v2-create-check-ok" } else { "v2-create-check-blocked" }>{move || if checklist().pricing_and_access { "Pricing and access complete" } else { "Pricing or access needs attention" }}</li>
                            <li class=move || if checklist().distribution { "v2-create-check-ok" } else { "v2-create-check-blocked" }>{move || if checklist().distribution { "Builds and distribution complete" } else { "Builds or distribution needs attention" }}</li>
                            <li class=move || if checklist().authorization { "v2-create-check-ok" } else { "v2-create-check-blocked" }>{move || if checklist().authorization { "Publishing authorization complete" } else { "Publishing authorization needs attention" }}</li>
                        </ul>
                        <ul class="v2-create-warnings">{move || checklist().warnings.into_iter().map(|warning| view! { <li>{format!("Warning: {warning}")}</li> }).collect_view()}</ul>

                        {move || {
                            let issues = blocking_issues();
                            (!issues.is_empty()).then(|| view! {
                                <div class="v2-create-alert v2-create-alert-error" role="alert">
                                    <strong>{format!("{} blocking issue(s) must be resolved before publication", issues.len())}</strong>
                                    <ul>{issues.into_iter().map(|(stage, error)| view! {
                                        <li><button type="button" class="v2-create-issue-link" on:click=move |_| { current_stage.set(stage); stage_error.set(None); }>{format!("{}: {}", stage.title(), error)}</button></li>
                                    }).collect_view()}</ul>
                                </div>
                            })
                        }}
                    </section>
                    </Show>

                    <Show when=move || current_stage.get() == PublishStage::Review || phase() != PublicationPhase::Idle>
                    <section class="v2-create-stage-panel" aria-labelledby="publication-status-title">
                        <div class="v2-create-status-heading">
                            <h2 id="publication-status-title" class="v2-create-section-title">"Publication status"</h2>
                            <span role="status" aria-live="polite" aria-atomic="true" class=move || format!("v2-create-phase {}", phase().class())>{move || phase().label()}</span>
                        </div>
                        <p class="v2-create-help">{move || match phase() {
                            PublicationPhase::Idle => "Nothing has been sent to the network from this workflow yet.",
                            PublicationPhase::Publishing => "The publication command is running. Relay acceptance is only reported once the command returns.",
                            PublicationPhase::Partial => "The game page exists on the network, but the remaining publication work did not complete.",
                            PublicationPhase::Published => "The listing event was accepted; the workflow can continue into management.",
                            PublicationPhase::Failed => "Publication failed. Nothing partial is claimed for this attempt.",
                            PublicationPhase::StaleAccount => "The active account changed during publication. Results from the previous account are rejected.",
                        }}</p>
                        <div aria-live="polite" aria-atomic="true">
                        {move || publication_state.get().message.map(|msg| {
                            let class = match publication_state.get().outcome { PublicationOutcome::Complete => "v2-create-status-ok", PublicationOutcome::Partial | PublicationOutcome::Failed => "v2-create-status-error", _ => "v2-create-status-neutral" };
                            view! { <p class={class}>{msg}</p> }
                        })}
                        {move || image_upload_status.get().map(|status| view! { <p class="v2-create-status-busy" role="status">{status}</p> })}
                        {move || upload_progress.get().and_then(|event| {
                            let bytes_uploaded = event.bytes_uploaded?;
                            let total_bytes = event.total_bytes?;
                            let percent = progress_percent(bytes_uploaded, total_bytes);
                            let server = event.server_url.unwrap_or_else(|| "distribution server".to_string());
                            Some(view! {
                                <div class="v2-create-progress" role="progressbar" aria-label="Build upload progress" aria-valuemin="0" aria-valuemax="100" aria-valuenow=percent>
                                    <div class="v2-create-progress-heading">
                                        <span>{format!("Uploading to {server}")}</span>
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
            </div>

            <div class="v2-create-actions">
                <div class="v2-create-actions-state">
                    <span class="v2-create-authoring">{move || authoring().label()}</span>
                    {move || is_dirty().then(|| view! { <span class="v2-create-authoring-dirty">"Not saved anywhere"</span> })}
                </div>
                <div class="v2-create-actions-buttons">
                    <button type="button" class="v2-btn-secondary" disabled=move || (!can_exit && current_stage.get() == PublishStage::Details) || is_publishing.get() on:click=move |_| {
                        if let Some(previous) = current_stage.get_untracked().previous() {
                            current_stage.set(previous);
                            stage_error.set(None);
                        } else if let Some(on_back) = on_back {
                            on_back.run(());
                        }
                    }>{move || if current_stage.get() == PublishStage::Details { "Leave without publishing" } else { "Back" }}</button>
                    <Show when=move || current_stage.get() != PublishStage::Review>
                        <button type="button" class="v2-btn-primary" on:click=on_next disabled=move || is_publishing.get()>"Continue >"</button>
                    </Show>
                    <Show when=move || current_stage.get() == PublishStage::Review>
                        <button type="button" class="v2-btn-primary" aria-describedby="publish-signer-requirement" on:click=move |_| on_submit.run(()) disabled=move || is_hashing.get() || is_selecting_image.get() || is_publishing.get() || !can_publish_now.get()>
                            {move || if is_publishing.get() { "Publishing to network…".to_string() } else { match (editing, fulfillment_enabled.get()) { (true, true) => "Update game page and distribution", (true, false) => "Update game page", (false, true) => "Publish game page and build", (false, false) => "Publish game page" }.to_string() }}
                        </button>
                    </Show>
                </div>
            </div>
        </main>
    }
}
