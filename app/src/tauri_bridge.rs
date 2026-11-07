// Typed IPC bridge wrappers for NIP-49 and NIP-05 desktop commands.

use crate::models::{
    DurableAcquisitionRecord, EarnedBadgeSummary, GameDetailPresentation, GameListing, Nip05Status,
    Nip49ExportResult, Nip49ImportRequest, PlatformInfo, ProfileBadgeEntry, SafeStorePageHtml,
    StorePageAccessibility, StorePageBatchEnrichment, StorePageDetailEnrichment,
    StorePageDetailSection, StorePageDetailState, StorePageEnrichmentRequest, StorePageLanguage,
    StorePageLinks, StorePageListingRef, StorePageMedia, StorePagePlatformRequirements,
    StorePageRequirementTier,
};
pub use arcadestr_core::adp_publish_contract::{
    AdpLaunchTierIntent, AdpPublishValidationViolation, ArgumentRejection,
    AuthoritativeAdpEditDefect, AuthoritativeAdpEditMetadata, AuthoritativeAdpEditMetadataPresent,
    AuthoritativeAdpEditTag, AuthoritativeProfileRejection, DisplayNameRejection,
    LoadAuthoritativeAdpEditMetadataRequest, PathRejection, ProfileIdRejection,
    PublishAdpListingError, PublishAdpListingRequestExtension,
};
pub use arcadestr_core::archive_inspection_contract::{
    ArchiveAccountFailure, ArchiveCancellationFailure, ArchiveCandidate,
    ArchiveCandidateConfidence, ArchiveCandidateReason, ArchiveDetectionFailure, ArchiveEntryKind,
    ArchiveFileSelection, ArchiveHandleFailure, ArchiveHazardCategory, ArchiveHazardCounts,
    ArchiveHazardDetail, ArchiveIdError, ArchiveInspectionCounts, ArchiveInspectionFailure,
    ArchiveMutationFailure, ArchivePage, ArchivePageLimitError, ArchivePageRequest,
    ArchivePaginationFailure, ArchivePathRuleCode, ArchivePlatformFailure, ArchiveRegistryFailure,
    ArchiveRequestId, ArchiveSearch, ArchiveSearchError, ArchiveSelectionError, ArchiveSelectionId,
    ArchiveSelectionSummary, ArchiveStorageOperation, ArchiveWarningCategory, ArchiveWarningCounts,
    ArchiveWarningDetail, ArchiveZipPreflightFailure, BuildSelectionRef,
    DiscardArchiveSelectionRequest, DiscardArchiveSelectionResponse, EfsDivergenceWarningDetail,
    ExtensionMismatchWarningDetail, HashSelectedArchiveRequest, HashSelectedArchiveResponse,
    InspectArchiveRequest, InspectArchiveResponse, QueryArchiveCandidatesRequest,
    QueryArchiveCandidatesResponse, QueryArchiveFilesRequest, QueryArchiveFilesResponse,
    QueryArchiveHazardsRequest, QueryArchiveHazardsResponse, QueryArchiveWarningsRequest,
    QueryArchiveWarningsResponse, QueryArchiveWorkingDirectoriesRequest,
    QueryArchiveWorkingDirectoriesResponse, SafeArchiveFilename, SafeArchiveFilenameError,
    SelectArchiveRequest, SelectArchiveResponse, SelectableArchiveFile, SelectableWorkingDirectory,
    ARCHIVE_PAGE_SIZE_DEFAULT, ARCHIVE_PAGE_SIZE_MAX,
};
pub use arcadestr_core::purchase_contract::{
    ApprovePurchasePlanRequest, InvoiceNetwork, LegProgress, PurchasePlanRequest,
    PurchasePlanReview,
};
use arcadestr_core::store_page::{StorePageDraft, StorePageValidationDiagnostic};
use serde::{Deserialize, Serialize};
pub use arcadestr_core::receipt_sync_contract::{ReceiptSyncRequest,ReceiptImportRequest,ReceiptSyncReport,ReceiptVerification};
pub async fn invoke_sync_purchase_receipts(request:ReceiptSyncRequest)->Result<ReceiptSyncReport,String>{
    recovery_invoke("sync_purchase_receipts",serde_json::json!({"request":request})).await
}
pub async fn invoke_import_purchase_receipts(request:ReceiptImportRequest)->Result<ReceiptSyncReport,String>{
    recovery_invoke("import_purchase_receipts",serde_json::json!({"request":request})).await
}

pub async fn invoke_get_purchase_plan(
    request: PurchasePlanRequest,
) -> Result<PurchasePlanReview, String> {
    #[cfg(not(feature = "web"))]
    {
        crate::tauri_invoke::invoke("get_purchase_plan", serde_json::json!({"request":request}))
            .await
    }
    #[cfg(feature = "web")]
    {
        let _ = request;
        Err("Purchase execution is available in the desktop application only".into())
    }
}
pub async fn invoke_approve_purchase_plan(
    request: ApprovePurchasePlanRequest,
) -> Result<PurchasePlanReview, String> {
    #[cfg(not(feature = "web"))]
    {
        crate::tauri_invoke::invoke(
            "approve_purchase_plan",
            serde_json::json!({"request":request}),
        )
        .await
    }
    #[cfg(feature = "web")]
    {
        let _ = request;
        Err("Purchase approval is available in the desktop application only".into())
    }
}

#[cfg(not(feature = "web"))]
pub async fn invoke_enrich_store_pages(
    request: StorePageEnrichmentRequest,
) -> Result<StorePageBatchEnrichment, String> {
    crate::tauri_invoke::invoke(
        "enrich_store_pages",
        serde_json::json!({ "request": request }),
    )
    .await
}

#[derive(Debug, Clone, Deserialize)]
struct RawStorePageMedia {
    id: String,
    media_type: String,
    role: String,
    url: String,
    thumbnail_url: Option<String>,
    alt: Option<String>,
    caption: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct RawStorePageSection {
    id: String,
    heading: String,
    body_html: String,
    media_id: Option<String>,
    layout: String,
}

#[derive(Debug, Clone, Deserialize)]
struct RawStorePageLanguage {
    code: String,
    interface: bool,
    audio: bool,
    subtitles: bool,
}

#[derive(Debug, Clone, Deserialize)]
struct RawRequirementTier {
    os: Option<String>,
    processor: Option<String>,
    memory: Option<String>,
    graphics: Option<String>,
    storage: Option<String>,
    additional: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct RawPlatformRequirements {
    platform: String,
    minimum: Option<RawRequirementTier>,
    recommended: Option<RawRequirementTier>,
}

#[derive(Debug, Clone, Deserialize)]
struct RawAccessibility {
    feature: String,
    supported: bool,
    notes: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct RawLinks {
    website: Option<String>,
    support: Option<String>,
    documentation: Option<String>,
    source: Option<String>,
    community: Option<String>,
    privacy_policy: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct RawDetailPresentation {
    listing_coordinate: String,
    listing_event_id: String,
    store_page_coordinate: String,
    event_id: String,
    title: Option<String>,
    summary: Option<String>,
    description_html: Option<String>,
    media: Vec<RawStorePageMedia>,
    sections: Vec<RawStorePageSection>,
    genres: Vec<String>,
    features: Vec<String>,
    languages: Vec<RawStorePageLanguage>,
    requirements: Vec<RawPlatformRequirements>,
    accessibility: Vec<RawAccessibility>,
    links: RawLinks,
    developer: Option<String>,
    publisher: Option<String>,
    release_date: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "state", content = "presentation", rename_all = "snake_case")]
enum RawDetailState {
    Enriched(RawDetailPresentation),
    NotAssociated,
    NotFound,
    Invalid,
    Unsupported,
    Unavailable,
}

#[derive(Debug, Clone, Deserialize)]
struct RawDetailEnrichment {
    generation: u64,
    listing_event_current: bool,
    cached: Option<RawDetailPresentation>,
    refreshed: RawDetailState,
}

#[cfg(not(feature = "web"))]
pub async fn invoke_enrich_store_page_detail(
    generation: u64,
    listing: StorePageListingRef,
) -> Result<StorePageDetailEnrichment, String> {
    let raw: RawDetailEnrichment = crate::tauri_invoke::invoke(
        "enrich_store_page_detail",
        serde_json::json!({ "request": { "generation": generation, "listing": listing } }),
    )
    .await?;
    Ok(map_detail_enrichment(raw))
}

#[cfg(feature = "web")]
pub async fn invoke_enrich_store_page_detail(
    _generation: u64,
    _listing: StorePageListingRef,
) -> Result<StorePageDetailEnrichment, String> {
    Err("Store Page detail enrichment is unavailable in standalone web builds.".to_string())
}

fn map_detail_enrichment(raw: RawDetailEnrichment) -> StorePageDetailEnrichment {
    StorePageDetailEnrichment {
        generation: raw.generation,
        listing_event_current: raw.listing_event_current,
        cached: raw.cached.map(map_detail_presentation),
        refreshed: match raw.refreshed {
            RawDetailState::Enriched(value) => {
                StorePageDetailState::Enriched(map_detail_presentation(value))
            }
            RawDetailState::NotAssociated => StorePageDetailState::NotAssociated,
            RawDetailState::NotFound => StorePageDetailState::NotFound,
            RawDetailState::Invalid => StorePageDetailState::Invalid,
            RawDetailState::Unsupported => StorePageDetailState::Unsupported,
            RawDetailState::Unavailable => StorePageDetailState::Unavailable,
        },
    }
}

fn map_detail_presentation(raw: RawDetailPresentation) -> GameDetailPresentation {
    GameDetailPresentation {
        listing_coordinate: raw.listing_coordinate,
        listing_event_id: raw.listing_event_id,
        store_page_coordinate: raw.store_page_coordinate,
        event_id: raw.event_id,
        title: raw.title,
        summary: raw.summary,
        description_html: raw.description_html.map(SafeStorePageHtml::from_backend),
        media: raw
            .media
            .into_iter()
            .map(|item| StorePageMedia {
                id: item.id,
                media_type: item.media_type,
                role: item.role,
                url: item.url,
                thumbnail_url: item.thumbnail_url,
                alt: item.alt,
                caption: item.caption,
            })
            .collect(),
        sections: raw
            .sections
            .into_iter()
            .map(|section| StorePageDetailSection {
                id: section.id,
                heading: section.heading,
                body_html: SafeStorePageHtml::from_backend(section.body_html),
                media_id: section.media_id,
                layout: section.layout,
            })
            .collect(),
        genres: raw.genres,
        features: raw.features,
        languages: raw
            .languages
            .into_iter()
            .map(|language| StorePageLanguage {
                code: language.code,
                interface: language.interface,
                audio: language.audio,
                subtitles: language.subtitles,
            })
            .collect(),
        requirements: raw
            .requirements
            .into_iter()
            .map(|requirement| StorePagePlatformRequirements {
                platform: requirement.platform,
                minimum: requirement.minimum.map(map_requirement_tier),
                recommended: requirement.recommended.map(map_requirement_tier),
            })
            .collect(),
        accessibility: raw
            .accessibility
            .into_iter()
            .map(|entry| StorePageAccessibility {
                feature: entry.feature,
                supported: entry.supported,
                notes: entry.notes,
            })
            .collect(),
        links: StorePageLinks {
            website: raw.links.website,
            support: raw.links.support,
            documentation: raw.links.documentation,
            source: raw.links.source,
            community: raw.links.community,
            privacy_policy: raw.links.privacy_policy,
        },
        developer: raw.developer,
        publisher: raw.publisher,
        release_date: raw.release_date,
    }
}

fn map_requirement_tier(raw: RawRequirementTier) -> StorePageRequirementTier {
    StorePageRequirementTier {
        os: raw.os,
        processor: raw.processor,
        memory: raw.memory,
        graphics: raw.graphics,
        storage: raw.storage,
        additional: raw.additional,
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublisherStorePageListingRevision {
    pub listing_coordinate: String,
    pub event_id: String,
    pub reciprocal: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PublisherStorePageEditorState {
    pub draft: StorePageDraft,
    pub baseline_draft: StorePageDraft,
    pub listings: Vec<PublisherStorePageListingRevision>,
    pub diagnostics: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct RawValidateStorePageDraftResponse {
    valid: bool,
    diagnostics: Vec<StorePageValidationDiagnostic>,
    preview: Option<RawDetailPresentation>,
}

#[derive(Debug, Clone)]
pub struct ValidateStorePageDraftResponse {
    pub valid: bool,
    pub diagnostics: Vec<StorePageValidationDiagnostic>,
    pub preview: Option<GameDetailPresentation>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ListingPointerMutation {
    Link,
    Unlink,
    Review,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StorePageListingMutation {
    pub listing_coordinate: String,
    pub expected_event_id: String,
    pub action: ListingPointerMutation,
    pub relay_hint: Option<String>,
    #[serde(default)]
    pub published_event_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct EventPublishOutcome {
    pub event_id: String,
    pub success_count: usize,
    pub failure_count: usize,
    pub propagation_confirmed: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ListingPointerPublishOutcome {
    pub listing_coordinate: String,
    pub action: ListingPointerMutation,
    pub replacement_event_id: Option<String>,
    pub published: bool,
    pub propagation_confirmed: bool,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PublishStorePageResponse {
    pub store_page_coordinate: String,
    pub store_page: Option<EventPublishOutcome>,
    pub listing_updates: Vec<ListingPointerPublishOutcome>,
    pub complete: bool,
    pub retryable: bool,
    pub cache_error: Option<String>,
    pub retry_scope_complete: bool,
}

fn store_page_publishing_unsupported_error() -> String {
    "Store Page publishing is unavailable in standalone web builds.".to_string()
}

#[cfg(not(feature = "web"))]
pub async fn invoke_load_publisher_store_page_editor(
    expected_publisher_npub: String,
    listing: StorePageListingRef,
    presentation_id: Option<String>,
) -> Result<PublisherStorePageEditorState, String> {
    crate::tauri_invoke::invoke(
        "load_publisher_store_page_editor",
        serde_json::json!({
            "request": {
                "expected_publisher_npub": expected_publisher_npub,
                "listing": listing,
                "presentation_id": presentation_id
            }
        }),
    )
    .await
}

#[cfg(feature = "web")]
pub async fn invoke_load_publisher_store_page_editor(
    _expected_publisher_npub: String,
    _listing: StorePageListingRef,
    _presentation_id: Option<String>,
) -> Result<PublisherStorePageEditorState, String> {
    Err(store_page_publishing_unsupported_error())
}

#[cfg(not(feature = "web"))]
pub async fn invoke_validate_store_page_draft(
    expected_publisher_npub: String,
    draft: StorePageDraft,
    preview_listing: PublisherStorePageListingRevision,
    listing_mutations: Vec<StorePageListingMutation>,
) -> Result<ValidateStorePageDraftResponse, String> {
    let raw: RawValidateStorePageDraftResponse = crate::tauri_invoke::invoke(
        "validate_store_page_draft_command",
        serde_json::json!({
            "request": {
                "expected_publisher_npub": expected_publisher_npub,
                "draft": draft,
                "preview_listing": preview_listing
                ,"listing_mutations": listing_mutations
            }
        }),
    )
    .await?;
    Ok(ValidateStorePageDraftResponse {
        valid: raw.valid,
        diagnostics: raw.diagnostics,
        preview: raw.preview.map(map_detail_presentation),
    })
}

#[cfg(feature = "web")]
pub async fn invoke_validate_store_page_draft(
    _expected_publisher_npub: String,
    _draft: StorePageDraft,
    _preview_listing: PublisherStorePageListingRevision,
    _listing_mutations: Vec<StorePageListingMutation>,
) -> Result<ValidateStorePageDraftResponse, String> {
    Err(store_page_publishing_unsupported_error())
}

#[cfg(not(feature = "web"))]
pub async fn invoke_clone_store_page(
    source: StorePageDraft,
    presentation_id: String,
) -> Result<StorePageDraft, String> {
    crate::tauri_invoke::invoke(
        "clone_store_page",
        serde_json::json!({ "request": { "source": source, "presentation_id": presentation_id } }),
    )
    .await
}

#[cfg(feature = "web")]
pub async fn invoke_clone_store_page(
    _source: StorePageDraft,
    _presentation_id: String,
) -> Result<StorePageDraft, String> {
    Err(store_page_publishing_unsupported_error())
}

#[cfg(not(feature = "web"))]
pub async fn invoke_publish_store_page(
    expected_publisher_npub: String,
    draft: StorePageDraft,
    listing_mutations: Vec<StorePageListingMutation>,
) -> Result<PublishStorePageResponse, String> {
    crate::tauri_invoke::invoke(
        "publish_store_page",
        serde_json::json!({
            "request": {
                "expected_publisher_npub": expected_publisher_npub,
                "draft": draft,
                "listing_mutations": listing_mutations
            }
        }),
    )
    .await
}

#[cfg(feature = "web")]
pub async fn invoke_publish_store_page(
    _expected_publisher_npub: String,
    _draft: StorePageDraft,
    _listing_mutations: Vec<StorePageListingMutation>,
) -> Result<PublishStorePageResponse, String> {
    Err(store_page_publishing_unsupported_error())
}

#[cfg(not(feature = "web"))]
pub async fn invoke_retry_store_page_pointer_sync(
    expected_publisher_npub: String,
    store_page_coordinate: String,
    store_page_event_id: String,
    listing_mutations: Vec<StorePageListingMutation>,
) -> Result<PublishStorePageResponse, String> {
    crate::tauri_invoke::invoke(
        "retry_store_page_pointer_sync",
        serde_json::json!({
            "request": {
                "expected_publisher_npub": expected_publisher_npub,
                "store_page_coordinate": store_page_coordinate,
                "store_page_event_id": store_page_event_id,
                "listing_mutations": listing_mutations
            }
        }),
    )
    .await
}

#[cfg(feature = "web")]
pub async fn invoke_retry_store_page_pointer_sync(
    _expected_publisher_npub: String,
    _store_page_coordinate: String,
    _store_page_event_id: String,
    _listing_mutations: Vec<StorePageListingMutation>,
) -> Result<PublishStorePageResponse, String> {
    Err(store_page_publishing_unsupported_error())
}

#[cfg(feature = "web")]
pub async fn invoke_enrich_store_pages(
    _request: StorePageEnrichmentRequest,
) -> Result<StorePageBatchEnrichment, String> {
    Err("Store Page relay enrichment is unavailable in standalone web builds.".to_string())
}

/// Invoke desktop `get_platform_info` command.
#[cfg(not(feature = "web"))]
pub async fn invoke_get_platform_info() -> Result<PlatformInfo, String> {
    crate::tauri_invoke::invoke("get_platform_info", serde_json::json!({})).await
}

/// Web fallback for `get_platform_info`.
#[cfg(feature = "web")]
pub async fn invoke_get_platform_info() -> Result<PlatformInfo, String> {
    Err("Platform detection is only available in desktop builds.".to_string())
}

/// Reconnect the active desktop NIP-46 signer profile.
#[cfg(not(feature = "web"))]
pub async fn invoke_attempt_reconnect() -> Result<serde_json::Value, String> {
    crate::tauri_invoke::invoke("attempt_reconnect", serde_json::json!({})).await
}

/// Web fallback for NIP-46 reconnect.
#[cfg(feature = "web")]
pub async fn invoke_attempt_reconnect() -> Result<serde_json::Value, String> {
    Err("Remote signer reconnect is only available in desktop builds.".to_string())
}

/// Ask the desktop relay pool to reconnect its configured default relays.
#[cfg(not(feature = "web"))]
pub async fn invoke_reconnect_relays() -> Result<String, String> {
    crate::tauri_invoke::invoke("reconnect_relays", serde_json::json!({})).await
}

/// Web fallback for native relay reconnect.
#[cfg(feature = "web")]
pub async fn invoke_reconnect_relays() -> Result<String, String> {
    Err("Native relay reconnect is only available in desktop builds.".to_string())
}

/// Invoke desktop `install_game` command.
#[cfg(not(feature = "web"))]
pub async fn invoke_install_game(
    listing: &GameListing,
) -> Result<(), arcadestr_core::install_contract::InstallCommandError> {
    crate::tauri_invoke::invoke_typed("install_game", serde_json::json!({ "listing": listing }))
        .await
}

/// Web fallback for `install_game`.
#[cfg(feature = "web")]
pub async fn invoke_install_game(
    _listing: &GameListing,
) -> Result<(), arcadestr_core::install_contract::InstallCommandError> {
    Err("Game installation is only available in desktop builds.".into())
}

/// Add a game to the initiating account's library without downloading it.
#[cfg(not(feature = "web"))]
pub async fn invoke_add_game_to_library(
    game_coordinate: String,
    expected_buyer_npub: String,
) -> Result<(), String> {
    crate::tauri_invoke::invoke(
        "add_game_to_library",
        serde_json::json!({
            "gameCoordinate": game_coordinate,
            "expectedBuyerNpub": expected_buyer_npub,
        }),
    )
    .await
}

/// Account libraries are unavailable in standalone web builds.
#[cfg(feature = "web")]
pub async fn invoke_add_game_to_library(
    _game_coordinate: String,
    _expected_buyer_npub: String,
) -> Result<(), String> {
    Err("Game libraries are only available in desktop builds.".to_string())
}

/// Return whether a game is already in the active account's library.
#[cfg(not(feature = "web"))]
pub async fn invoke_is_game_in_library(game_coordinate: String) -> Result<bool, String> {
    crate::tauri_invoke::invoke(
        "is_game_in_library",
        serde_json::json!({ "gameCoordinate": game_coordinate }),
    )
    .await
}

/// Account libraries are unavailable in standalone web builds.
#[cfg(feature = "web")]
pub async fn invoke_is_game_in_library(_game_coordinate: String) -> Result<bool, String> {
    Ok(false)
}

/// A game saved to the active account's library.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct LibraryGame {
    pub game_coordinate: String,
    pub added_at: i64,
}

/// Return games saved to the active account's library.
#[cfg(not(feature = "web"))]
pub async fn invoke_get_library_games() -> Result<Vec<LibraryGame>, String> {
    crate::tauri_invoke::invoke("get_library_games", serde_json::json!({})).await
}

/// Account libraries are unavailable in standalone web builds.
#[cfg(feature = "web")]
pub async fn invoke_get_library_games() -> Result<Vec<LibraryGame>, String> {
    Ok(Vec::new())
}

/// Return whether the active account owns a listing.
#[cfg(not(feature = "web"))]
pub async fn invoke_get_listing_ownership(
    buyer_npub: String,
    publisher_npub: String,
    listing_id: String,
) -> Result<bool, String> {
    crate::tauri_invoke::invoke(
        "get_listing_ownership",
        serde_json::json!({
            "buyerNpub": buyer_npub,
            "publisherNpub": publisher_npub,
            "listingId": listing_id,
        }),
    )
    .await
}

/// Web fallback for account ownership lookup.
#[cfg(feature = "web")]
pub async fn invoke_get_listing_ownership(
    _buyer_npub: String,
    _publisher_npub: String,
    _listing_id: String,
) -> Result<bool, String> {
    Ok(false)
}

/// Installed ADP game returned by `get_installed_games`.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct InstalledGame {
    pub installation_id: String,
    pub game_coordinate: String,
    pub installation_root: String,
    pub package_digest: String,
    pub package_format: String,
    pub state: InstallationState,
    pub recovery_discriminator: Option<InstallationRecoveryDiscriminator>,
    pub retained_package: bool,
    pub launchable: bool,
    pub installed_at: i64,
}

/// Persisted installation lifecycle state returned by desktop commands.
#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum InstallationState {
    Installing,
    Ready,
    Failed,
    Uninstalling,
}

/// Filesystem outcome recorded when recovery marks an installation failed.
#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum InstallationRecoveryDiscriminator {
    FinalRootAbsent,
    FinalRootPresent,
}

/// Non-ready installation returned by `list_installation_lifecycle_issues`.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct InstallationLifecycleIssue {
    pub installation_id: String,
    pub game_coordinate: String,
    pub installation_root: String,
    pub state: InstallationState,
    pub recovery_discriminator: Option<InstallationRecoveryDiscriminator>,
    pub updated_at: i64,
}

/// Result of lazy interrupted-installation recovery.
#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct InstallationRecoveryReport {
    pub examined: usize,
    pub marked_failed: usize,
    pub busy: usize,
    pub deferred_errors: usize,
    pub changed: usize,
}

/// Download retained for a later offline reinstall.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct RetainedPackage {
    pub game_coordinate: String,
    pub package_id: String,
    pub package_path: String,
    pub listing_event_id: Option<String>,
    pub size_bytes: u64,
}

/// Authoritative sizes shown before uninstall.
#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct UninstallSizes {
    pub installation_size_bytes: u64,
    pub retained_package_size_bytes: Option<u64>,
}

fn installation_id_args(installation_id: String) -> serde_json::Value {
    serde_json::json!({ "installationId": installation_id })
}

fn uninstall_game_args(installation_id: String, retain_package: bool) -> serde_json::Value {
    serde_json::json!({
        "installationId": installation_id,
        "retainPackage": retain_package,
    })
}

fn retained_package_identity_args(
    game_coordinate: String,
    expected_package_id: String,
) -> serde_json::Value {
    serde_json::json!({
        "gameCoordinate": game_coordinate,
        "expectedPackageId": expected_package_id,
    })
}

const RESUME_UNINSTALL_COMMAND: &str = "resume_uninstall";
const REINSTALL_RETAINED_PACKAGE_COMMAND: &str = "reinstall_retained_package";

/// Install phase emitted by the desktop installer.
#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum InstallProgressPhase {
    Preparing,
    Downloading,
    Verifying,
    Installing,
    Complete,
}

/// Typed progress emitted by the complete desktop installation pipeline.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct InstallProgressPayload {
    pub game_coordinate: String,
    pub phase: InstallProgressPhase,
    pub bytes: u64,
    pub total: Option<u64>,
}

/// Invoke desktop `get_installed_games` command.
#[cfg(not(feature = "web"))]
pub async fn invoke_get_installed_games() -> Result<Vec<InstalledGame>, String> {
    crate::tauri_invoke::invoke("get_installed_games", serde_json::json!({})).await
}

/// Web fallback for `get_installed_games`.
#[cfg(feature = "web")]
pub async fn invoke_get_installed_games() -> Result<Vec<InstalledGame>, String> {
    Ok(Vec::new())
}

#[cfg(not(feature = "web"))]
pub async fn invoke_get_launch_options(
    installation_id: &str,
) -> Result<
    arcadestr_core::launch_contract::LaunchConfiguration,
    arcadestr_core::launch_contract::LaunchError,
> {
    crate::tauri_invoke::invoke_typed(
        "get_launch_options",
        serde_json::json!({ "installationId": installation_id }),
    )
    .await
}

#[cfg(feature = "web")]
pub async fn invoke_get_launch_options(
    _installation_id: &str,
) -> Result<
    arcadestr_core::launch_contract::LaunchConfiguration,
    arcadestr_core::launch_contract::LaunchError,
> {
    Err(arcadestr_core::launch_contract::LaunchError::Operation(
        "Launch profiles are available only in the desktop application".into(),
    ))
}

#[cfg(not(feature = "web"))]
pub async fn invoke_set_launch_profile_preference(
    installation_id: &str,
    profile_id: Option<&str>,
    preference_updated_at: i64,
) -> Result<
    arcadestr_core::launch_contract::LaunchConfiguration,
    arcadestr_core::launch_contract::LaunchError,
> {
    crate::tauri_invoke::invoke_typed(
        "set_launch_profile_preference",
        serde_json::json!({
            "installationId": installation_id,
            "profileId": profile_id,
            "preferenceUpdatedAt": preference_updated_at,
        }),
    )
    .await
}

#[cfg(feature = "web")]
pub async fn invoke_set_launch_profile_preference(
    _installation_id: &str,
    _profile_id: Option<&str>,
    _preference_updated_at: i64,
) -> Result<
    arcadestr_core::launch_contract::LaunchConfiguration,
    arcadestr_core::launch_contract::LaunchError,
> {
    Err(arcadestr_core::launch_contract::LaunchError::Operation(
        "Launch profile preferences are available only in the desktop application".into(),
    ))
}

#[cfg(not(feature = "web"))]
pub async fn invoke_launch_game(
    installation_id: &str,
    profile_id: Option<&str>,
) -> Result<
    arcadestr_core::launch_contract::LaunchSession,
    arcadestr_core::launch_contract::LaunchError,
> {
    crate::tauri_invoke::invoke_typed(
        "launch_game",
        serde_json::json!({ "installationId": installation_id, "profileId": profile_id }),
    )
    .await
}

#[cfg(feature = "web")]
pub async fn invoke_launch_game(
    _installation_id: &str,
    _profile_id: Option<&str>,
) -> Result<
    arcadestr_core::launch_contract::LaunchSession,
    arcadestr_core::launch_contract::LaunchError,
> {
    Err(arcadestr_core::launch_contract::LaunchError::Operation(
        "Launching is available only in the desktop application".into(),
    ))
}

#[cfg(not(feature = "web"))]
pub async fn invoke_get_launch_status(
    installation_id: &str,
) -> Result<
    Option<arcadestr_core::launch_contract::LaunchSession>,
    arcadestr_core::launch_contract::LaunchError,
> {
    crate::tauri_invoke::invoke_typed(
        "get_launch_status",
        serde_json::json!({ "installationId": installation_id }),
    )
    .await
}

#[cfg(feature = "web")]
pub async fn invoke_get_launch_status(
    _installation_id: &str,
) -> Result<
    Option<arcadestr_core::launch_contract::LaunchSession>,
    arcadestr_core::launch_contract::LaunchError,
> {
    Ok(None)
}

/// Return non-ready installation lifecycle records.
#[cfg(not(feature = "web"))]
pub async fn invoke_list_installation_lifecycle_issues(
) -> Result<Vec<InstallationLifecycleIssue>, String> {
    crate::tauri_invoke::invoke("list_installation_lifecycle_issues", serde_json::json!({})).await
}

/// Native installation lifecycle records are unavailable on web.
#[cfg(feature = "web")]
pub async fn invoke_list_installation_lifecycle_issues(
) -> Result<Vec<InstallationLifecycleIssue>, String> {
    Ok(Vec::new())
}

/// Lazily retry interrupted-installation recovery.
#[cfg(not(feature = "web"))]
pub async fn invoke_retry_installation_recovery() -> Result<InstallationRecoveryReport, String> {
    crate::tauri_invoke::invoke("retry_installation_recovery", serde_json::json!({})).await
}

/// Native installation recovery is unavailable on web.
#[cfg(feature = "web")]
pub async fn invoke_retry_installation_recovery() -> Result<InstallationRecoveryReport, String> {
    Err("Installation recovery is only available in desktop builds.".to_string())
}

/// Return every retained installer package.
#[cfg(not(feature = "web"))]
pub async fn invoke_list_retained_packages() -> Result<Vec<RetainedPackage>, String> {
    crate::tauri_invoke::invoke("list_retained_packages", serde_json::json!({})).await
}

/// Native retained packages are unavailable on web.
#[cfg(feature = "web")]
pub async fn invoke_list_retained_packages() -> Result<Vec<RetainedPackage>, String> {
    Ok(Vec::new())
}

/// Return installation and retained-package sizes before uninstall.
#[cfg(not(feature = "web"))]
pub async fn invoke_get_uninstall_sizes(installation_id: String) -> Result<UninstallSizes, String> {
    crate::tauri_invoke::invoke("get_uninstall_sizes", installation_id_args(installation_id)).await
}

/// Native uninstall sizing is unavailable on web.
#[cfg(feature = "web")]
pub async fn invoke_get_uninstall_sizes(
    _installation_id: String,
) -> Result<UninstallSizes, String> {
    Err("Game uninstall is only available in desktop builds.".to_string())
}

/// Uninstall a ready installation with an explicit package-retention choice.
#[cfg(not(feature = "web"))]
pub async fn invoke_uninstall_game(
    installation_id: String,
    retain_package: bool,
) -> Result<(), String> {
    crate::tauri_invoke::invoke(
        "uninstall_game",
        uninstall_game_args(installation_id, retain_package),
    )
    .await
}

/// Native uninstall is unavailable on web.
#[cfg(feature = "web")]
pub async fn invoke_uninstall_game(
    _installation_id: String,
    _retain_package: bool,
) -> Result<(), String> {
    Err("Game uninstall is only available in desktop builds.".to_string())
}

/// Resume an interrupted uninstall. Desktop conservatively retains its package.
#[cfg(not(feature = "web"))]
pub async fn invoke_resume_uninstall(installation_id: String) -> Result<(), String> {
    crate::tauri_invoke::invoke(
        RESUME_UNINSTALL_COMMAND,
        installation_id_args(installation_id),
    )
    .await
}

/// Native interrupted-uninstall resolution is unavailable on web.
#[cfg(feature = "web")]
pub async fn invoke_resume_uninstall(_installation_id: String) -> Result<(), String> {
    Err("Interrupted uninstall resolution is only available in desktop builds.".to_string())
}

/// Remove a failed installation record and any preserved final tree.
#[cfg(not(feature = "web"))]
pub async fn invoke_remove_failed_installation(installation_id: String) -> Result<(), String> {
    crate::tauri_invoke::invoke(
        "remove_failed_installation",
        installation_id_args(installation_id),
    )
    .await
}

/// Native failed-installation cleanup is unavailable on web.
#[cfg(feature = "web")]
pub async fn invoke_remove_failed_installation(_installation_id: String) -> Result<(), String> {
    Err("Installation cleanup is only available in desktop builds.".to_string())
}

/// Delete one retained package without changing installation records.
#[cfg(not(feature = "web"))]
pub async fn invoke_delete_retained_package(
    game_coordinate: String,
    expected_package_id: String,
) -> Result<(), String> {
    crate::tauri_invoke::invoke(
        "delete_retained_package",
        retained_package_identity_args(game_coordinate, expected_package_id),
    )
    .await
}

/// Native retained-package deletion is unavailable on web.
#[cfg(feature = "web")]
pub async fn invoke_delete_retained_package(
    _game_coordinate: String,
    _expected_package_id: String,
) -> Result<(), String> {
    Err("Retained package deletion is only available in desktop builds.".to_string())
}

/// Reinstall entirely from one retained package without network or account access.
#[cfg(not(feature = "web"))]
pub async fn invoke_reinstall_retained_package(
    game_coordinate: String,
    expected_package_id: String,
) -> Result<(), String> {
    crate::tauri_invoke::invoke(
        REINSTALL_RETAINED_PACKAGE_COMMAND,
        retained_package_identity_args(game_coordinate, expected_package_id),
    )
    .await
}

/// Native retained-package reinstall is unavailable on web.
#[cfg(feature = "web")]
pub async fn invoke_reinstall_retained_package(
    _game_coordinate: String,
    _expected_package_id: String,
) -> Result<(), String> {
    Err("Retained package reinstall is only available in desktop builds.".to_string())
}

/// Invoke desktop `ingest_receipt` command.
#[cfg(not(feature = "web"))]
pub async fn invoke_ingest_receipt(raw_event_json: String) -> Result<(), String> {
    crate::tauri_invoke::invoke(
        "ingest_receipt",
        serde_json::json!({ "rawEventJson": raw_event_json }),
    )
    .await
}

/// Web fallback for `ingest_receipt`.
#[cfg(feature = "web")]
pub async fn invoke_ingest_receipt(_raw_event_json: String) -> Result<(), String> {
    Err("Receipt ingestion is only available in desktop builds.".to_string())
}

/// Return durable purchase and promotion-claim records for the active account.
#[cfg(not(feature = "web"))]
pub async fn invoke_get_purchase_records() -> Result<Vec<DurableAcquisitionRecord>, String> {
    crate::tauri_invoke::invoke("get_purchase_records", serde_json::json!({})).await
}

/// Durable acquisition history is unavailable in standalone web builds.
#[cfg(feature = "web")]
pub async fn invoke_get_purchase_records() -> Result<Vec<DurableAcquisitionRecord>, String> {
    Err("Purchase and access records are only available in desktop builds.".to_string())
}

/// ADP server metadata returned by `check_adp_server`.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct AdpServerInfo {
    pub adp_version: String,
    #[serde(default)]
    pub max_upload_bytes: Option<u64>,
    pub pubkey: String,
    pub name: Option<String>,
    pub url: Option<String>,
}

/// Request payload for the ADP publish command.
#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FulfillmentMode {
    None,
    Direct,
    Delegate,
}

/// Request payload for the ADP publish command.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct PublishAdpListingRequest {
    pub expected_publisher_npub: String,
    pub existing_event_id: Option<String>,
    /// Identifier of the listing being replaced. `None` for a first
    /// publication, where the backend mints a UUID v4 instead.
    pub existing_d_tag: Option<String>,
    /// Identifier the backend minted for this publication on an attempt that
    /// failed before the listing existed. Sending it back resumes that
    /// publication rather than starting a second one.
    pub pending_d_tag: Option<String>,
    pub title: String,
    pub description: String,
    pub price_sats: u64,
    pub lud16: Option<String>,
    pub tags: Vec<String>,
    pub images: Vec<String>,
    pub fulfillment_mode: FulfillmentMode,
    pub operator_url: Option<String>,
    pub servers: Vec<String>,
    #[serde(default)]
    pub accepted_terms: Vec<arcadestr_core::adp_settlement::AcceptedTermsRef>,
    pub file_path: Option<String>,
    /// Opaque backend-selected build reference for the native publication path.
    #[serde(default)]
    pub selected_build: Option<BuildSelectionRef>,
    pub existing_file_hash: Option<String>,
    pub existing_fulfillment_pubkey: Option<String>,
    pub version: Option<String>,
    pub acquisition: crate::models::AcquisitionPolicy,
    pub platforms: Vec<String>,
    pub campaigns: Vec<CampaignPointerInput>,
    pub nip94_event_id: Option<String>,
}

/// Request payload for resolving an edited listing's delegated operator.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct ResolveAdpOperatorRequest {
    pub publisher_npub: String,
    pub fulfillment_pubkey: String,
    pub scope: String,
}

/// Upload response nested in ADP publish results.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct UploadResponse {
    pub game_coordinate: String,
    pub file_hash: String,
    pub download_url: String,
}

/// Per-server upload status returned by ADP publish results.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct PublishServerUploadResult {
    pub server_url: String,
    pub status: String,
    pub error: Option<String>,
    pub upload: Option<UploadResponse>,
}

/// Result returned by the ADP publish command.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct PublishAdpListingResult {
    pub event_id: String,
    /// Set only when the backend minted an identifier for this publication.
    pub game_id: Option<uuid::Uuid>,
    pub d_tag: String,
    pub game_coordinate: String,
    pub acceptance_event_id: Option<String>,
    pub fulfillment_pubkey: Option<String>,
    pub file_hash: Option<String>,
    pub uploads: Vec<PublishServerUploadResult>,
}

/// Progress event payload emitted during ADP publishing.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct PublishProgressPayload {
    pub step: String,
    pub status: String,
    pub server_url: Option<String>,
    pub message: Option<String>,
    pub bytes_uploaded: Option<u64>,
    pub total_bytes: Option<u64>,
}

/// Progress event payload emitted while hashing a selected build file.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct HashProgressPayload {
    pub bytes_hashed: u64,
    pub total_bytes: u64,
}

/// ADP server announcement discovered from relays.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct AdpServerAnnouncement {
    pub pubkey: String,
    pub url: String,
    pub name: Option<String>,
    pub supported_adp: Option<String>,
    pub contact: Option<String>,
    #[serde(default)]
    pub settlement_version: Option<u16>,
    #[serde(default)]
    pub split_enabled: bool,
    #[serde(default)]
    pub terms_event_id: Option<String>,
    #[serde(default)]
    pub legacy_paid_sales: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct HashBuildFileRequest {
    pub file_path: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct RequestLnurlInvoiceRequest {
    pub lud16: String,
    pub amount_sats: u64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct RequestLnurlInvoiceResponse {
    pub bolt11: String,
}

#[derive(Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct ConnectNwcWalletRequest {
    pub expected_account_pubkey: String,
    pub connection_string: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct ConnectNwcWalletResponse {
    pub wallet_pubkey: String,
    pub relays: Vec<String>,
    pub lud16: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct PayNwcInvoiceRequest {
    pub expected_account_pubkey: String,
    pub bolt11: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct PayNwcInvoiceResponse {
    pub preimage: String,
    pub fees_paid_msat: Option<u64>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct ConfirmPurchaseRequest {
    pub publisher_npub: String,
    pub listing_id: String,
    pub server_url: String,
    pub bolt11: String,
    pub preimage: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct ConfirmPurchaseResponse {
    pub receipt: serde_json::Value,
    pub download_token: String,
    pub token_expires_at: i64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct StartSplitPurchaseRequest {
    pub publisher_npub: String,
    pub listing_id: String,
    pub server_url: String,
    pub refund_lud16: String,
    pub purchase_id: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct StartSplitPurchaseResponse {
    pub quote_id: String,
    pub quote_event: serde_json::Value,
    pub quote: arcadestr_core::adp_settlement::SignedQuote,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct SplitQuoteRequest {
    pub server_url: String,
    pub quote_id: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct SplitInvoiceResponse {
    pub quote_id: String,
    pub attempt_id: String,
    pub leg_id: String,
    pub amount_msat: u64,
    pub payment_hash: String,
    pub bolt11: String,
    pub expires_at: u64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct SubmitSplitPaymentRequest {
    pub server_url: String,
    pub quote_id: String,
    pub attempt_id: String,
    pub preimage: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SplitSettlementResponse {
    pub state: String,
    pub settled_leg: Option<String>,
    pub receipt: Option<serde_json::Value>,
    pub download_token: Option<String>,
    pub token_expires_at: Option<i64>,
}

impl SplitSettlementResponse {
    /// Desktop returns receipt material only after its validation/persistence
    /// path succeeds. A server status or conflict error alone is not ownership.
    pub fn acquisition_complete(&self) -> bool {
        self.state == "settled" && self.receipt.as_ref().is_some_and(|r| r.is_object())
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SettlementHistoryItem {
    pub quote_id: String,
    pub processor_url: String,
    pub listing_event_id: String,
    pub state: String,
    pub failure_at: Option<u64>,
    pub receipt: Option<serde_json::Value>,
    #[serde(default)]
    pub recovery: Vec<RecoveryHistoryItem>,
    #[serde(default)]
    pub attempts: Vec<RecoveryAttemptView>,
    #[serde(default)]
    pub recovery_state: Option<String>,
    #[serde(default)]
    pub recovery_message: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RecoveryAttemptView {
    pub quote_id: String,
    pub processor_url: String,
    pub attempt_id: String,
    pub leg_id: String,
    pub state: String,
    pub has_intent: bool,
    pub has_proof: bool,
}
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ResumeSplitPaymentRequest {
    pub server_url: String,
    pub quote_id: String,
    pub attempt_id: String,
}
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RecoveryReport {
    pub recovered: u64,
    pub blocked: u64,
    pub skipped: u64,
    pub message: String,
}
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RefundHistoryItem {
    pub quote_id: String,
    pub leg_id: String,
    pub recipient_pubkey: String,
    pub principal_msat: u64,
    pub due_at: u64,
    pub state: String,
    pub attempt_id: Option<String>,
    pub invoice: Option<String>,
    pub payment_state: String,
}
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RefundPaymentRequest {
    pub server_url: String,
    pub quote_id: String,
    pub leg_id: String,
    pub confirm_payment: bool,
}

async fn recovery_invoke<T: serde::de::DeserializeOwned + 'static>(
    command: &str,
    args: serde_json::Value,
) -> Result<T, String> {
    #[cfg(not(feature = "web"))]
    {
        crate::tauri_invoke::invoke(command, args).await
    }
    #[cfg(feature = "web")]
    {
        let _ = (command, args);
        Err("Settlement recovery is available in the desktop app".into())
    }
}
pub async fn invoke_resume_split_payment(
    request: ResumeSplitPaymentRequest,
) -> Result<SplitSettlementResponse, String> {
    recovery_invoke(
        "resume_split_payment",
        serde_json::json!({"request":request}),
    )
    .await
}
pub async fn invoke_reconcile_pending_settlements() -> Result<RecoveryReport, String> {
    recovery_invoke("reconcile_pending_settlements", serde_json::json!({})).await
}
pub async fn invoke_recipient_refund_history(
    server_url: String,
) -> Result<Vec<RefundHistoryItem>, String> {
    recovery_invoke(
        "recipient_refund_history",
        serde_json::json!({"request":{"server_url":server_url}}),
    )
    .await
}
pub async fn invoke_pay_split_refund(
    request: RefundPaymentRequest,
) -> Result<serde_json::Value, String> {
    recovery_invoke("pay_split_refund", serde_json::json!({"request":request})).await
}
pub async fn invoke_resume_split_refund(
    request: RefundPaymentRequest,
) -> Result<serde_json::Value, String> {
    recovery_invoke(
        "resume_split_refund",
        serde_json::json!({"request":request}),
    )
    .await
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct RecoveryHistoryItem {
    pub leg_id: String,
    pub state: String,
    pub principal_msat: u64,
    pub due_at: u64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct ReplaceSplitRefundDestinationRequest {
    pub server_url: String,
    pub quote_id: String,
    pub refund_lud16: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct SplitRefundRequest {
    pub server_url: String,
    pub quote_id: String,
    pub leg_id: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct ConfirmSplitRefundRequest {
    pub server_url: String,
    pub quote_id: String,
    pub leg_id: String,
    pub attempt_id: String,
    pub preimage: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct SplitRefundInvoiceResponse {
    pub attempt_id: String,
    pub leg_id: String,
    pub amount_msat: u64,
    pub bolt11: String,
    pub expires_at: u64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct ClaimEntitlementRequest {
    pub publisher_npub: String,
    pub listing_id: String,
    pub campaign_event_id: String,
    pub server_url: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct ClaimEntitlementResponse {
    pub grant: serde_json::Value,
    pub download_token: String,
    pub token_expires_at: i64,
    pub already_claimed: bool,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct DiscoverCampaignsRequest {
    pub publisher_npub: String,
    pub listing_id: String,
    pub pointers: Vec<CampaignPointerInput>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct CampaignPointerInput {
    pub root_event_id: String,
    pub relay_hint: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct DiscoverCampaignSummariesRequest {
    pub publisher_npub: String,
    pub listings: Vec<CampaignSummaryListingInput>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct CampaignSummaryListingInput {
    pub listing_id: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct CampaignSummary {
    pub listing_id: String,
    pub active: usize,
    pub upcoming: usize,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct DiscoveredCampaign {
    pub root_event_id: String,
    pub campaign_id: String,
    pub starts_at: u64,
    pub ends_at: u64,
    pub classification: String,
    #[serde(default)]
    pub event_id: Option<String>,
    #[serde(default)]
    pub predecessor_event_id: Option<String>,
    #[serde(default)]
    pub mode: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct PublishCampaignRequest {
    pub publisher_npub: String,
    pub listing_id: String,
    pub campaign_id: String,
    pub starts_at: Option<u64>,
    pub ends_at: Option<u64>,
    pub predecessor_event_id: Option<String>,
    pub cancel: bool,
    pub update_listing_pointer: bool,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct PublishCampaignResponse {
    pub event_id: String,
    pub root_event_id: String,
    pub listing_event_id: Option<String>,
    #[serde(default)]
    pub pointer_update_error: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct UpdateCampaignPointerRequest {
    pub publisher_npub: String,
    pub listing_id: String,
    pub campaign_root_id: String,
    pub remove: bool,
}

/// Invoke desktop `check_adp_server` command.
#[cfg(not(feature = "web"))]
pub async fn invoke_check_adp_server(server_url: String) -> Result<AdpServerInfo, String> {
    crate::tauri_invoke::invoke(
        "check_adp_server",
        serde_json::json!({ "serverUrl": server_url }),
    )
    .await
}

/// Web fallback for `check_adp_server`.
#[cfg(feature = "web")]
pub async fn invoke_check_adp_server(_server_url: String) -> Result<AdpServerInfo, String> {
    Err("ADP server checks are only available in desktop builds.".to_string())
}

/// Invoke desktop `discover_adp_servers` command.
#[cfg(not(feature = "web"))]
pub async fn invoke_discover_adp_servers() -> Result<Vec<AdpServerAnnouncement>, String> {
    crate::tauri_invoke::invoke("discover_adp_servers", serde_json::json!({})).await
}

/// Web fallback for `discover_adp_servers`.
#[cfg(feature = "web")]
pub async fn invoke_discover_adp_servers() -> Result<Vec<AdpServerAnnouncement>, String> {
    Ok(Vec::new())
}

macro_rules! archive_contract_wrapper {
    ($name:ident, $command:literal, $request:ty, $response:ty) => {
        #[cfg(not(feature = "web"))]
        pub async fn $name(request: $request) -> Result<$response, ArchiveSelectionError> {
            crate::tauri_invoke::invoke_archive_typed(
                $command,
                serde_json::json!({ "request": request }),
            )
            .await
        }

        #[cfg(feature = "web")]
        pub async fn $name(_request: $request) -> Result<$response, ArchiveSelectionError> {
            Err(ArchiveSelectionError::UnsupportedPlatform)
        }
    };
}

archive_contract_wrapper!(
    invoke_select_archive_build,
    "select_archive_build",
    SelectArchiveRequest,
    SelectArchiveResponse
);
archive_contract_wrapper!(
    invoke_inspect_archive,
    "inspect_archive",
    InspectArchiveRequest,
    InspectArchiveResponse
);
archive_contract_wrapper!(
    invoke_hash_selected_archive,
    "hash_selected_archive",
    HashSelectedArchiveRequest,
    HashSelectedArchiveResponse
);
archive_contract_wrapper!(
    invoke_query_archive_candidates,
    "query_archive_candidates",
    QueryArchiveCandidatesRequest,
    QueryArchiveCandidatesResponse
);
archive_contract_wrapper!(
    invoke_query_archive_files,
    "query_archive_files",
    QueryArchiveFilesRequest,
    QueryArchiveFilesResponse
);
archive_contract_wrapper!(
    invoke_query_archive_working_directories,
    "query_archive_working_directories",
    QueryArchiveWorkingDirectoriesRequest,
    QueryArchiveWorkingDirectoriesResponse
);
archive_contract_wrapper!(
    invoke_query_archive_hazards,
    "query_archive_hazards",
    QueryArchiveHazardsRequest,
    QueryArchiveHazardsResponse
);
archive_contract_wrapper!(
    invoke_query_archive_warnings,
    "query_archive_warnings",
    QueryArchiveWarningsRequest,
    QueryArchiveWarningsResponse
);
archive_contract_wrapper!(
    invoke_discard_archive_selection,
    "discard_archive_selection",
    DiscardArchiveSelectionRequest,
    DiscardArchiveSelectionResponse
);

/// Invoke desktop `select_build_file` command.
#[cfg(not(feature = "web"))]
pub async fn invoke_select_build_file() -> Result<Option<String>, String> {
    crate::tauri_invoke::invoke("select_build_file", serde_json::json!({})).await
}

/// Web fallback for `select_build_file`.
#[cfg(feature = "web")]
pub async fn invoke_select_build_file() -> Result<Option<String>, String> {
    Err("Build file selection is only available in desktop builds.".to_string())
}

/// Invoke desktop `hash_build_file` command.
#[cfg(not(feature = "web"))]
pub async fn invoke_hash_build_file(request: HashBuildFileRequest) -> Result<String, String> {
    crate::tauri_invoke::invoke("hash_build_file", serde_json::json!({ "request": request })).await
}

/// Web fallback for `hash_build_file`.
#[cfg(feature = "web")]
pub async fn invoke_hash_build_file(_request: HashBuildFileRequest) -> Result<String, String> {
    Err("Build file hashing is only available in desktop builds.".to_string())
}

/// Listen for desktop build hashing progress events.
#[cfg(not(feature = "web"))]
pub async fn listen_hash_progress<F>(mut callback: F) -> Result<Box<dyn FnOnce()>, String>
where
    F: FnMut(HashProgressPayload) + 'static,
{
    let cleanup = crate::tauri_invoke::listen("hash-progress", move |value| {
        if let Ok(payload) = serde_json::from_value::<HashProgressPayload>(value) {
            callback(payload);
        }
    })
    .await?;
    Ok(Box::new(cleanup))
}

/// Web fallback for build hashing progress events.
#[cfg(feature = "web")]
pub async fn listen_hash_progress<F>(_callback: F) -> Result<Box<dyn FnOnce()>, String>
where
    F: FnMut(HashProgressPayload) + 'static,
{
    Err("Build file hashing is only available in desktop builds.".to_string())
}

/// Invoke desktop `publish_adp_listing` command.
#[cfg(not(feature = "web"))]
pub async fn invoke_publish_adp_listing(
    request: PublishAdpListingRequest,
) -> Result<PublishAdpListingResult, String> {
    crate::tauri_invoke::invoke_typed::<_, PublishAdpListingError>(
        "publish_adp_listing",
        serde_json::json!({ "request": request }),
    )
    .await
    .map_err(|error| error.to_string())
}

/// Invoke desktop `resolve_adp_operator` command.
#[cfg(not(feature = "web"))]
pub async fn invoke_resolve_adp_operator(
    request: ResolveAdpOperatorRequest,
) -> Result<Option<String>, String> {
    crate::tauri_invoke::invoke(
        "resolve_adp_operator",
        serde_json::json!({ "request": request }),
    )
    .await
}

/// Web fallback for `resolve_adp_operator`.
#[cfg(feature = "web")]
pub async fn invoke_resolve_adp_operator(
    _request: ResolveAdpOperatorRequest,
) -> Result<Option<String>, String> {
    Ok(None)
}

/// Web fallback for `publish_adp_listing`.
#[cfg(feature = "web")]
pub async fn invoke_publish_adp_listing(
    _request: PublishAdpListingRequest,
) -> Result<PublishAdpListingResult, String> {
    Err("ADP publishing is only available in desktop builds.".to_string())
}

/// Invokes the future typed publish contract without changing the legacy wrapper.
///
/// Subtask 5 owns adoption of the additive `extension` command argument.
#[cfg(not(feature = "web"))]
pub async fn invoke_publish_adp_listing_with_contract(
    request: PublishAdpListingRequest,
    extension: PublishAdpListingRequestExtension,
) -> Result<PublishAdpListingResult, PublishAdpListingError> {
    crate::tauri_invoke::invoke_typed(
        "publish_adp_listing_with_contract",
        serde_json::json!({ "request": request, "extension": extension }),
    )
    .await
}

/// Web fallback for the future typed publish contract.
#[cfg(feature = "web")]
pub async fn invoke_publish_adp_listing_with_contract(
    _request: PublishAdpListingRequest,
    _extension: PublishAdpListingRequestExtension,
) -> Result<PublishAdpListingResult, PublishAdpListingError> {
    Err(PublishAdpListingError::UnsupportedPlatform)
}

/// Loads launch metadata from the authoritative signed listing revision.
#[cfg(not(feature = "web"))]
pub async fn invoke_load_authoritative_adp_edit_metadata(
    request: LoadAuthoritativeAdpEditMetadataRequest,
) -> Result<AuthoritativeAdpEditMetadata, PublishAdpListingError> {
    crate::tauri_invoke::invoke_typed(
        "load_authoritative_adp_edit_metadata",
        serde_json::json!({ "request": request }),
    )
    .await
}

/// Web fallback for authoritative ADP edit metadata.
#[cfg(feature = "web")]
pub async fn invoke_load_authoritative_adp_edit_metadata(
    _request: LoadAuthoritativeAdpEditMetadataRequest,
) -> Result<AuthoritativeAdpEditMetadata, PublishAdpListingError> {
    Err(PublishAdpListingError::UnsupportedPlatform)
}

#[cfg(not(feature = "web"))]
pub async fn invoke_get_nwc_wallet(
    expected_account_pubkey: String,
) -> Result<Option<ConnectNwcWalletResponse>, String> {
    crate::tauri_invoke::invoke(
        "get_nwc_wallet",
        serde_json::json!({ "expectedAccountPubkey": expected_account_pubkey }),
    )
    .await
}
#[cfg(feature = "web")]
pub async fn invoke_get_nwc_wallet(
    _expected_account_pubkey: String,
) -> Result<Option<ConnectNwcWalletResponse>, String> {
    Ok(None)
}
#[cfg(not(feature = "web"))]
pub async fn invoke_disconnect_nwc_wallet(expected_account_pubkey: String) -> Result<(), String> {
    crate::tauri_invoke::invoke(
        "disconnect_nwc_wallet",
        serde_json::json!({ "expectedAccountPubkey": expected_account_pubkey }),
    )
    .await
}
#[cfg(feature = "web")]
pub async fn invoke_disconnect_nwc_wallet(_expected_account_pubkey: String) -> Result<(), String> {
    Err("NWC is only available on desktop.".into())
}
#[cfg(not(feature = "web"))]
pub async fn invoke_verify_nwc_wallet(expected_account_pubkey: String) -> Result<String, String> {
    crate::tauri_invoke::invoke(
        "verify_nwc_wallet",
        serde_json::json!({ "expectedAccountPubkey": expected_account_pubkey }),
    )
    .await
}
#[cfg(feature = "web")]
pub async fn invoke_verify_nwc_wallet(_expected_account_pubkey: String) -> Result<String, String> {
    Err("NWC is only available on desktop.".into())
}

#[cfg(not(feature = "web"))]
pub async fn invoke_connect_nwc_wallet(
    request: ConnectNwcWalletRequest,
) -> Result<ConnectNwcWalletResponse, String> {
    crate::tauri_invoke::invoke(
        "connect_nwc_wallet",
        serde_json::json!({ "request": request }),
    )
    .await
}

#[cfg(feature = "web")]
pub async fn invoke_connect_nwc_wallet(
    _request: ConnectNwcWalletRequest,
) -> Result<ConnectNwcWalletResponse, String> {
    Err("NWC wallet connection is only available in desktop builds.".to_string())
}

#[cfg(not(feature = "web"))]
pub async fn invoke_request_lnurl_invoice(
    request: RequestLnurlInvoiceRequest,
) -> Result<RequestLnurlInvoiceResponse, String> {
    crate::tauri_invoke::invoke(
        "request_lnurl_invoice",
        serde_json::json!({ "request": request }),
    )
    .await
}

#[cfg(feature = "web")]
pub async fn invoke_request_lnurl_invoice(
    _request: RequestLnurlInvoiceRequest,
) -> Result<RequestLnurlInvoiceResponse, String> {
    Err("ADP invoice requests are only available in desktop builds.".to_string())
}

#[cfg(not(feature = "web"))]
pub async fn invoke_pay_nwc_invoice(
    request: PayNwcInvoiceRequest,
) -> Result<PayNwcInvoiceResponse, String> {
    crate::tauri_invoke::invoke("pay_nwc_invoice", serde_json::json!({ "request": request })).await
}

#[cfg(feature = "web")]
pub async fn invoke_pay_nwc_invoice(
    _request: PayNwcInvoiceRequest,
) -> Result<PayNwcInvoiceResponse, String> {
    Err("NWC payments are only available in desktop builds.".to_string())
}

#[cfg(not(feature = "web"))]
pub async fn invoke_confirm_purchase(
    request: ConfirmPurchaseRequest,
) -> Result<ConfirmPurchaseResponse, String> {
    crate::tauri_invoke::invoke(
        "confirm_purchase",
        serde_json::json!({ "request": request }),
    )
    .await
}

#[cfg(feature = "web")]
pub async fn invoke_confirm_purchase(
    _request: ConfirmPurchaseRequest,
) -> Result<ConfirmPurchaseResponse, String> {
    Err("ADP purchase confirmation is only available in desktop builds.".to_string())
}

#[cfg(not(feature = "web"))]
pub async fn invoke_start_split_purchase(
    request: StartSplitPurchaseRequest,
) -> Result<StartSplitPurchaseResponse, String> {
    crate::tauri_invoke::invoke(
        "start_split_purchase",
        serde_json::json!({ "request": request }),
    )
    .await
}

#[cfg(feature = "web")]
pub async fn invoke_start_split_purchase(
    _request: StartSplitPurchaseRequest,
) -> Result<StartSplitPurchaseResponse, String> {
    Err("Split settlement is only available in desktop builds.".to_string())
}

#[cfg(not(feature = "web"))]
pub async fn invoke_next_split_invoice(
    request: SplitQuoteRequest,
) -> Result<SplitInvoiceResponse, String> {
    crate::tauri_invoke::invoke(
        "next_split_invoice",
        serde_json::json!({ "request": request }),
    )
    .await
}

#[cfg(feature = "web")]
pub async fn invoke_next_split_invoice(
    _request: SplitQuoteRequest,
) -> Result<SplitInvoiceResponse, String> {
    Err("Split settlement is only available in desktop builds.".to_string())
}

#[cfg(not(feature = "web"))]
pub async fn invoke_submit_split_payment(
    request: SubmitSplitPaymentRequest,
) -> Result<SplitSettlementResponse, String> {
    crate::tauri_invoke::invoke(
        "submit_split_payment",
        serde_json::json!({ "request": request }),
    )
    .await
}

#[cfg(feature = "web")]
pub async fn invoke_submit_split_payment(
    _request: SubmitSplitPaymentRequest,
) -> Result<SplitSettlementResponse, String> {
    Err("Split settlement is only available in desktop builds.".to_string())
}

#[cfg(not(feature = "web"))]
pub async fn invoke_settlement_history() -> Result<Vec<SettlementHistoryItem>, String> {
    crate::tauri_invoke::invoke("settlement_history", serde_json::json!({})).await
}

#[cfg(feature = "web")]
pub async fn invoke_settlement_history() -> Result<Vec<SettlementHistoryItem>, String> {
    Err("Settlement history is only available in desktop builds.".to_string())
}

#[cfg(not(feature = "web"))]
pub async fn invoke_refresh_settlement(
    request: SplitQuoteRequest,
) -> Result<serde_json::Value, String> {
    crate::tauri_invoke::invoke(
        "refresh_settlement",
        serde_json::json!({ "request": request }),
    )
    .await
}

#[cfg(feature = "web")]
pub async fn invoke_refresh_settlement(
    _request: SplitQuoteRequest,
) -> Result<serde_json::Value, String> {
    Err("Settlement recovery is only available in desktop builds.".to_string())
}

#[cfg(not(feature = "web"))]
pub async fn invoke_replace_split_refund_destination(
    request: ReplaceSplitRefundDestinationRequest,
) -> Result<serde_json::Value, String> {
    crate::tauri_invoke::invoke(
        "replace_split_refund_destination",
        serde_json::json!({ "request": request }),
    )
    .await
}

#[cfg(feature = "web")]
pub async fn invoke_replace_split_refund_destination(
    _request: ReplaceSplitRefundDestinationRequest,
) -> Result<serde_json::Value, String> {
    Err("Settlement recovery is only available in desktop builds.".to_string())
}

#[cfg(not(feature = "web"))]
pub async fn invoke_prepare_split_recovery(
    request: SplitQuoteRequest,
) -> Result<serde_json::Value, String> {
    crate::tauri_invoke::invoke(
        "prepare_split_recovery",
        serde_json::json!({ "request": request }),
    )
    .await
}

#[cfg(feature = "web")]
pub async fn invoke_prepare_split_recovery(
    _request: SplitQuoteRequest,
) -> Result<serde_json::Value, String> {
    Err("Settlement recovery is only available in desktop builds.".to_string())
}

#[cfg(not(feature = "web"))]
pub async fn invoke_prepare_split_refund(
    request: SplitRefundRequest,
) -> Result<SplitRefundInvoiceResponse, String> {
    crate::tauri_invoke::invoke(
        "prepare_split_refund",
        serde_json::json!({ "request": request }),
    )
    .await
}

#[cfg(feature = "web")]
pub async fn invoke_prepare_split_refund(
    _request: SplitRefundRequest,
) -> Result<SplitRefundInvoiceResponse, String> {
    Err("Settlement recovery is only available in desktop builds.".to_string())
}

#[cfg(not(feature = "web"))]
pub async fn invoke_confirm_split_refund(
    request: ConfirmSplitRefundRequest,
) -> Result<serde_json::Value, String> {
    crate::tauri_invoke::invoke(
        "confirm_split_refund",
        serde_json::json!({ "request": request }),
    )
    .await
}

#[cfg(feature = "web")]
pub async fn invoke_confirm_split_refund(
    _request: ConfirmSplitRefundRequest,
) -> Result<serde_json::Value, String> {
    Err("Settlement recovery is only available in desktop builds.".to_string())
}

#[cfg(not(feature = "web"))]
pub async fn invoke_claim_entitlement(
    request: ClaimEntitlementRequest,
) -> Result<ClaimEntitlementResponse, String> {
    crate::tauri_invoke::invoke(
        "claim_entitlement",
        serde_json::json!({ "request": request }),
    )
    .await
}

#[cfg(feature = "web")]
pub async fn invoke_claim_entitlement(
    _request: ClaimEntitlementRequest,
) -> Result<ClaimEntitlementResponse, String> {
    Err("ADP entitlement claims are only available in desktop builds.".to_string())
}

#[cfg(not(feature = "web"))]
pub async fn invoke_discover_campaigns(
    request: DiscoverCampaignsRequest,
) -> Result<Vec<DiscoveredCampaign>, String> {
    crate::tauri_invoke::invoke(
        "discover_campaigns",
        serde_json::json!({ "request": request }),
    )
    .await
}

#[cfg(not(feature = "web"))]
pub async fn invoke_discover_campaign_summaries(
    request: DiscoverCampaignSummariesRequest,
) -> Result<Vec<CampaignSummary>, String> {
    crate::tauri_invoke::invoke(
        "discover_campaign_summaries",
        serde_json::json!({ "request": request }),
    )
    .await
}

#[cfg(feature = "web")]
pub async fn invoke_discover_campaign_summaries(
    _request: DiscoverCampaignSummariesRequest,
) -> Result<Vec<CampaignSummary>, String> {
    Ok(Vec::new())
}

#[cfg(feature = "web")]
pub async fn invoke_discover_campaigns(
    _request: DiscoverCampaignsRequest,
) -> Result<Vec<DiscoveredCampaign>, String> {
    Ok(Vec::new())
}

#[cfg(not(feature = "web"))]
pub async fn invoke_publish_campaign(
    request: PublishCampaignRequest,
) -> Result<PublishCampaignResponse, String> {
    crate::tauri_invoke::invoke(
        "publish_campaign",
        serde_json::json!({ "request": request }),
    )
    .await
}

#[cfg(not(feature = "web"))]
pub async fn invoke_update_campaign_pointer(
    request: UpdateCampaignPointerRequest,
) -> Result<String, String> {
    crate::tauri_invoke::invoke(
        "update_campaign_pointer",
        serde_json::json!({ "request": request }),
    )
    .await
}

#[cfg(feature = "web")]
pub async fn invoke_update_campaign_pointer(
    _request: UpdateCampaignPointerRequest,
) -> Result<String, String> {
    Err("Campaign pointer updates are only available in desktop builds.".to_string())
}

#[cfg(feature = "web")]
pub async fn invoke_publish_campaign(
    _request: PublishCampaignRequest,
) -> Result<PublishCampaignResponse, String> {
    Err("Campaign publishing is only available in desktop builds.".to_string())
}

/// Listen for desktop `publish-progress` events.
#[cfg(not(feature = "web"))]
pub async fn listen_publish_progress<F>(mut callback: F) -> Result<Box<dyn FnOnce()>, String>
where
    F: FnMut(PublishProgressPayload) + 'static,
{
    let cleanup = crate::tauri_invoke::listen("publish-progress", move |value| {
        if let Ok(payload) = serde_json::from_value::<PublishProgressPayload>(value) {
            callback(payload);
        }
    })
    .await?;
    Ok(Box::new(cleanup))
}

/// Web fallback for `publish-progress` events.
#[cfg(feature = "web")]
pub async fn listen_publish_progress<F>(_callback: F) -> Result<Box<dyn FnOnce()>, String>
where
    F: FnMut(PublishProgressPayload) + 'static,
{
    Err("ADP publishing is only available in desktop builds.".to_string())
}

/// Listen for desktop `install-progress` events.
#[cfg(not(feature = "web"))]
pub async fn listen_install_progress<F>(mut callback: F) -> Result<Box<dyn FnOnce()>, String>
where
    F: FnMut(InstallProgressPayload) + 'static,
{
    let cleanup = crate::tauri_invoke::listen("install-progress", move |value| {
        if let Ok(payload) = serde_json::from_value::<InstallProgressPayload>(value) {
            callback(payload);
        }
    })
    .await?;
    Ok(Box::new(cleanup))
}

/// Web fallback for `install-progress` events.
#[cfg(feature = "web")]
pub async fn listen_install_progress<F>(_callback: F) -> Result<Box<dyn FnOnce()>, String>
where
    F: FnMut(InstallProgressPayload) + 'static,
{
    Err("ADP installation events are only available in desktop builds.".to_string())
}

/// Invoke desktop `nip49_import` command.
#[cfg(not(feature = "web"))]
pub async fn invoke_nip49_import(request: Nip49ImportRequest) -> Result<String, String> {
    crate::tauri_invoke::invoke("nip49_import", serde_json::json!({ "request": request })).await
}

/// Web fallback for `nip49_import`.
#[cfg(feature = "web")]
pub async fn invoke_nip49_import(_request: Nip49ImportRequest) -> Result<String, String> {
    Err("nip49_import is only available in desktop builds".to_string())
}

/// Invoke desktop `nip49_export` command.
#[cfg(not(feature = "web"))]
pub async fn invoke_nip49_export(
    npub: String,
    password: String,
) -> Result<Nip49ExportResult, String> {
    crate::tauri_invoke::invoke(
        "nip49_export",
        serde_json::json!({
            "npub": npub,
            "password": password,
        }),
    )
    .await
}

/// Web fallback for `nip49_export`.
#[cfg(feature = "web")]
pub async fn invoke_nip49_export(
    _npub: String,
    _password: String,
) -> Result<Nip49ExportResult, String> {
    Err("nip49_export is only available in desktop builds".to_string())
}

/// Invoke desktop `verify_nip05` command.
#[cfg(not(feature = "web"))]
pub async fn invoke_verify_nip05(
    identifier: String,
    expected_npub: String,
) -> Result<Nip05Status, String> {
    crate::tauri_invoke::invoke(
        "verify_nip05",
        serde_json::json!({ "identifier": identifier, "expectedNpub": expected_npub }),
    )
    .await
}

/// Web fallback for `verify_nip05`.
#[cfg(feature = "web")]
pub async fn invoke_verify_nip05(
    _identifier: String,
    _expected_npub: String,
) -> Result<Nip05Status, String> {
    Err("verify_nip05 is only available in desktop builds".to_string())
}

/// Invoke desktop `get_cached_earned_badges` command.
#[cfg(not(feature = "web"))]
pub async fn get_cached_earned_badges(
    profile_pubkey: String,
) -> Result<Vec<EarnedBadgeSummary>, String> {
    crate::tauri_invoke::invoke(
        "get_cached_earned_badges",
        serde_json::json!({ "profilePubkey": profile_pubkey }),
    )
    .await
}

/// Web fallback for `get_cached_earned_badges`.
#[cfg(feature = "web")]
pub async fn get_cached_earned_badges(
    _profile_pubkey: String,
) -> Result<Vec<EarnedBadgeSummary>, String> {
    Err("Badge relay display is not yet available on the web target.".to_string())
}

/// Invoke desktop `get_cached_profile_badges` command.
#[cfg(not(feature = "web"))]
pub async fn get_cached_profile_badges(
    profile_pubkey: String,
) -> Result<Vec<ProfileBadgeEntry>, String> {
    crate::tauri_invoke::invoke(
        "get_cached_profile_badges",
        serde_json::json!({ "profilePubkey": profile_pubkey }),
    )
    .await
}

/// Web fallback for `get_cached_profile_badges`.
#[cfg(feature = "web")]
pub async fn get_cached_profile_badges(
    _profile_pubkey: String,
) -> Result<Vec<ProfileBadgeEntry>, String> {
    Err("Badge relay display is not yet available on the web target.".to_string())
}

/// Invoke desktop `fetch_earned_badges` command.
#[cfg(not(feature = "web"))]
pub async fn fetch_earned_badges(
    profile_pubkey: String,
) -> Result<Vec<EarnedBadgeSummary>, String> {
    crate::tauri_invoke::invoke(
        "fetch_earned_badges",
        serde_json::json!({ "profilePubkey": profile_pubkey }),
    )
    .await
}

/// Web fallback for `fetch_earned_badges`.
#[cfg(feature = "web")]
pub async fn fetch_earned_badges(
    _profile_pubkey: String,
) -> Result<Vec<EarnedBadgeSummary>, String> {
    Err("Badge relay display is not yet available on the web target.".to_string())
}

/// Invoke desktop `fetch_profile_badges` command.
#[cfg(not(feature = "web"))]
pub async fn fetch_profile_badges(
    profile_pubkey: String,
) -> Result<Vec<ProfileBadgeEntry>, String> {
    crate::tauri_invoke::invoke(
        "fetch_profile_badges",
        serde_json::json!({ "profilePubkey": profile_pubkey }),
    )
    .await
}

/// Web fallback for `fetch_profile_badges`.
#[cfg(feature = "web")]
pub async fn fetch_profile_badges(
    _profile_pubkey: String,
) -> Result<Vec<ProfileBadgeEntry>, String> {
    Err("Badge relay display is not yet available on the web target.".to_string())
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BlossomCommandErrorDto {
    pub code: String,
    pub message: String,
}

impl From<String> for BlossomCommandErrorDto {
    fn from(_message: String) -> Self {
        Self {
            code: "storage_failure".to_string(),
            message: "The desktop command could not be completed.".to_string(),
        }
    }
}

#[cfg(feature = "web")]
fn blossom_desktop_only() -> BlossomCommandErrorDto {
    BlossomCommandErrorDto {
        code: "desktop_only".to_string(),
        message: "Blossom uploads and settings are only available in desktop builds.".to_string(),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExpectedBlossomPublisherRequest {
    pub expected_publisher_hex: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BlossomMediaSelectionDto {
    pub selection_id: String,
    pub filename: String,
    pub detected_mime: String,
    pub size: u64,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub preview_data_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StartBlossomUploadRequest {
    pub selection_id: String,
    pub expected_publisher_hex: String,
    pub selected_server: Option<String>,
    pub preflight: bool,
    pub request_id: String,
}

pub type RetryBlossomUploadRequest = StartBlossomUploadRequest;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BlossomUploadResponse {
    pub upload_id: String,
    pub url: String,
    pub sha256: String,
    pub mime_type: String,
    pub size: u64,
    pub uploaded: u64,
    pub was_existing: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BlossomUploadProgressDto {
    pub upload_id: String,
    pub selection_id: String,
    pub request_id: String,
    pub publisher_pubkey: String,
    pub phase: String,
    pub bytes_completed: u64,
    pub total_bytes: u64,
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CancelBlossomUploadRequest {
    pub upload_id: String,
    pub expected_publisher_hex: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CancelBlossomUploadResponse {
    pub cancelled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DiscardBlossomMediaRequest {
    pub selection_id: String,
    pub expected_publisher_hex: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DiscardBlossomMediaResponse {
    pub cancelled_uploads: usize,
    pub selection_removed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BlossomServerDto {
    pub origin: String,
    pub label: Option<String>,
    pub enabled: bool,
    pub created_at: u64,
    pub updated_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BlossomServerSettingsDto {
    pub publisher_pubkey: String,
    pub servers: Vec<BlossomServerDto>,
    pub preferred_server: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BlossomServerHealthDto {
    pub origin: String,
    pub status: String,
    pub latency_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BlossomServerHealthResponse {
    pub publisher_pubkey: String,
    pub servers: Vec<BlossomServerHealthDto>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BlossomServerInputDto {
    pub origin: String,
    pub label: Option<String>,
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReplaceBlossomServerSettingsRequest {
    pub expected_publisher_hex: String,
    pub servers: Vec<BlossomServerInputDto>,
    pub preferred_server: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AddBlossomServerRequest {
    pub expected_publisher_hex: String,
    pub origin: String,
    pub label: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct UpdateBlossomServerRequest {
    pub expected_publisher_hex: String,
    pub origin: String,
    pub label: Option<String>,
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BlossomServerOriginRequest {
    pub expected_publisher_hex: String,
    pub origin: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReorderBlossomServersRequest {
    pub expected_publisher_hex: String,
    pub ordered_origins: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SetPreferredBlossomServerRequest {
    pub expected_publisher_hex: String,
    pub origin: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ResolveBlossomServerCandidatesRequest {
    pub expected_publisher_hex: String,
    pub explicit_server: Option<String>,
}

#[cfg(not(feature = "web"))]
async fn invoke_blossom<T, R>(command: &str, request: R) -> Result<T, BlossomCommandErrorDto>
where
    T: serde::de::DeserializeOwned + 'static,
    R: Serialize,
{
    crate::tauri_invoke::invoke_typed(command, serde_json::json!({ "request": request })).await
}

macro_rules! blossom_native_wrapper {
    ($name:ident, $command:literal, $request:ty, $response:ty) => {
        #[cfg(not(feature = "web"))]
        pub async fn $name(request: $request) -> Result<$response, BlossomCommandErrorDto> {
            invoke_blossom($command, request).await
        }
        #[cfg(feature = "web")]
        pub async fn $name(_request: $request) -> Result<$response, BlossomCommandErrorDto> {
            Err(blossom_desktop_only())
        }
    };
}

blossom_native_wrapper!(
    invoke_select_blossom_media_file,
    "select_blossom_media_file",
    ExpectedBlossomPublisherRequest,
    Option<BlossomMediaSelectionDto>
);
blossom_native_wrapper!(
    invoke_start_blossom_upload,
    "start_blossom_upload",
    StartBlossomUploadRequest,
    BlossomUploadResponse
);
blossom_native_wrapper!(
    invoke_retry_blossom_upload,
    "retry_blossom_upload",
    RetryBlossomUploadRequest,
    BlossomUploadResponse
);
blossom_native_wrapper!(
    invoke_cancel_blossom_upload,
    "cancel_blossom_upload",
    CancelBlossomUploadRequest,
    CancelBlossomUploadResponse
);
blossom_native_wrapper!(
    invoke_discard_blossom_media_selection,
    "discard_blossom_media_selection",
    DiscardBlossomMediaRequest,
    DiscardBlossomMediaResponse
);
blossom_native_wrapper!(
    invoke_get_blossom_server_settings,
    "get_blossom_server_settings",
    ExpectedBlossomPublisherRequest,
    BlossomServerSettingsDto
);
blossom_native_wrapper!(
    invoke_probe_blossom_server_health,
    "probe_blossom_server_health",
    ExpectedBlossomPublisherRequest,
    BlossomServerHealthResponse
);
blossom_native_wrapper!(
    invoke_replace_blossom_server_settings,
    "replace_blossom_server_settings",
    ReplaceBlossomServerSettingsRequest,
    BlossomServerSettingsDto
);
blossom_native_wrapper!(
    invoke_add_blossom_server,
    "add_blossom_server",
    AddBlossomServerRequest,
    BlossomServerSettingsDto
);
blossom_native_wrapper!(
    invoke_update_blossom_server,
    "update_blossom_server",
    UpdateBlossomServerRequest,
    BlossomServerSettingsDto
);
blossom_native_wrapper!(
    invoke_remove_blossom_server,
    "remove_blossom_server",
    BlossomServerOriginRequest,
    BlossomServerSettingsDto
);
blossom_native_wrapper!(
    invoke_reorder_blossom_servers,
    "reorder_blossom_servers",
    ReorderBlossomServersRequest,
    BlossomServerSettingsDto
);
blossom_native_wrapper!(
    invoke_set_preferred_blossom_server,
    "set_preferred_blossom_server",
    SetPreferredBlossomServerRequest,
    BlossomServerSettingsDto
);
blossom_native_wrapper!(
    invoke_resolve_blossom_server_candidates,
    "resolve_blossom_server_candidates",
    ResolveBlossomServerCandidatesRequest,
    Vec<String>
);

#[cfg(not(feature = "web"))]
pub async fn listen_blossom_upload_progress<F>(
    mut callback: F,
) -> Result<Box<dyn FnOnce()>, BlossomCommandErrorDto>
where
    F: FnMut(BlossomUploadProgressDto) + 'static,
{
    let cleanup = crate::tauri_invoke::listen("blossom-upload-progress", move |value| {
        if let Ok(payload) = serde_json::from_value::<BlossomUploadProgressDto>(value) {
            callback(payload);
        }
    })
    .await
    .map_err(BlossomCommandErrorDto::from)?;
    Ok(Box::new(cleanup))
}

#[cfg(feature = "web")]
pub async fn listen_blossom_upload_progress<F>(
    _callback: F,
) -> Result<Box<dyn FnOnce()>, BlossomCommandErrorDto>
where
    F: FnMut(BlossomUploadProgressDto) + 'static,
{
    Err(blossom_desktop_only())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_status_or_null_receipt_is_never_completed_acquisition() {
        let mut response = SplitSettlementResponse {
            state: "settled".into(),
            settled_leg: Some("operator".into()),
            receipt: None,
            download_token: None,
            token_expires_at: None,
        };
        assert!(!response.acquisition_complete());
        response.receipt = Some(serde_json::Value::Null);
        assert!(!response.acquisition_complete());
        response.receipt = Some(serde_json::json!({"id":"validated-by-desktop"}));
        assert!(response.acquisition_complete());
        response.state = "active".into();
        assert!(!response.acquisition_complete());
    }

    #[test]
    fn blossom_tauri_bridge_request_response_and_progress_are_safe() {
        let request = StartBlossomUploadRequest {
            selection_id: "selection".into(),
            expected_publisher_hex: "ab".repeat(32),
            selected_server: Some("https://cdn.example/".into()),
            preflight: true,
            request_id: "ui-1".into(),
        };
        let request_json = serde_json::to_value(&request).expect("serialize request");
        assert_eq!(request_json["request_id"], "ui-1");
        let response = BlossomUploadResponse {
            upload_id: "upload".into(),
            url: "https://cdn.example/blob".into(),
            sha256: "cd".repeat(32),
            mime_type: "image/png".into(),
            size: 12,
            uploaded: 10,
            was_existing: true,
        };
        let progress = BlossomUploadProgressDto {
            upload_id: response.upload_id.clone(),
            selection_id: request.selection_id,
            request_id: request.request_id,
            publisher_pubkey: request.expected_publisher_hex,
            phase: "upload".into(),
            bytes_completed: 6,
            total_bytes: 12,
            message: None,
        };
        let json = format!(
            "{}{}{}",
            serde_json::to_string(&request_json).expect("request JSON"),
            serde_json::to_string(&response).expect("response JSON"),
            serde_json::to_string(&progress).expect("progress JSON")
        );
        for forbidden in ["file_path", "authorization", "raw_bytes", "secret"] {
            assert!(!json.to_ascii_lowercase().contains(forbidden));
        }
        assert!(response.was_existing);
        assert_eq!(progress.phase, "upload");
    }

    #[cfg(feature = "web")]
    #[test]
    fn blossom_tauri_bridge_web_error_is_typed_desktop_only() {
        let error = blossom_desktop_only();
        assert_eq!(error.code, "desktop_only");
    }

    #[test]
    fn publish_request_serialization_preserves_existing_fulfillment_metadata() {
        let request = PublishAdpListingRequest {
            expected_publisher_npub: "npub1expected".to_string(),
            existing_event_id: Some("existing-listing-event".to_string()),
            existing_d_tag: Some("game".to_string()),
            pending_d_tag: None,
            title: "Game".to_string(),
            description: "Description".to_string(),
            price_sats: 0,
            lud16: None,
            tags: vec![],
            images: vec![],
            fulfillment_mode: FulfillmentMode::Delegate,
            operator_url: Some("https://operator.example.com".to_string()),
            servers: vec!["https://dist.example.com".to_string()],
            accepted_terms: Vec::new(),
            file_path: None,
            selected_build: None,
            existing_file_hash: Some("hash".to_string()),
            existing_fulfillment_pubkey: Some("delegated-key".to_string()),
            version: Some("1.0.0".to_string()),
            acquisition: crate::models::AcquisitionPolicy::TimedAccess {
                starts_at: 100,
                ends_at: 200,
            },
            platforms: vec!["linux-x86_64".to_string()],
            campaigns: vec![CampaignPointerInput {
                root_event_id: "campaign-root".to_string(),
                relay_hint: Some("wss://relay.example.com".to_string()),
            }],
            nip94_event_id: Some("nip94-event".to_string()),
        };

        let value = serde_json::to_value(request).expect("request should serialize");

        assert_eq!(value["expected_publisher_npub"], "npub1expected");
        assert_eq!(value["existing_event_id"], "existing-listing-event");
        assert_eq!(value["existing_d_tag"], "game");
        assert!(
            value.get("d_tag").is_none(),
            "publishers no longer supply a listing identifier"
        );
        assert_eq!(value["existing_fulfillment_pubkey"], "delegated-key");
        assert!(value.get("existing_fulfillment_valid_from").is_none());
        assert!(value.get("existing_fulfillment_revoked_at").is_none());
        assert_eq!(value["acquisition"]["TimedAccess"]["starts_at"], 100);
        assert_eq!(value["acquisition"]["TimedAccess"]["ends_at"], 200);
        assert_eq!(value["campaigns"][0]["root_event_id"], "campaign-root");
        assert_eq!(value["nip94_event_id"], "nip94-event");
    }

    #[test]
    fn typed_publish_ipc_preserves_selected_build_and_nested_partial_error() {
        let selected_build: BuildSelectionRef = serde_json::from_value(serde_json::json!({
            "selection_id": "123e4567-e89b-42d3-a456-426614174001",
            "sha256": "1111111111111111111111111111111111111111111111111111111111111111"
        }))
        .expect("selected build contract");
        let selected_value = serde_json::to_value(&selected_build).expect("selected build JSON");
        assert_eq!(
            selected_value["selection_id"],
            "123e4567-e89b-42d3-a456-426614174001"
        );

        let error = PublishAdpListingError::PartialPublication {
            cause: Box::new(PublishAdpListingError::Archive {
                error: ArchiveSelectionError::DigestMismatch {
                    expected: selected_build.sha256.clone(),
                    actual: arcadestr_core::distribution::Sha256Digest::parse(&"22".repeat(32))
                        .expect("actual digest"),
                },
            }),
            successful_servers: vec!["https://one.example".to_string()],
        };
        let round_trip: PublishAdpListingError =
            serde_json::from_value(serde_json::to_value(&error).expect("typed publish error JSON"))
                .expect("typed publish error round trip");
        assert_eq!(round_trip, error);
    }

    #[test]
    fn archive_ipc_preserves_hostile_pages_for_typed_backend_validation() {
        let request: QueryArchiveFilesRequest = serde_json::from_value(serde_json::json!({
            "request_id": "123e4567-e89b-42d3-a456-426614174000",
            "selection_id": "123e4567-e89b-42d3-a456-426614174001",
            "expected_publisher_npub": "npub1expected",
            "search": "",
            "page": { "offset": 0, "limit": 201 }
        }))
        .expect("numeric hostile page must cross the IPC contract");

        assert_eq!(
            request.page.validate(),
            Err(ArchivePaginationFailure::LimitExceeded {
                requested: 201,
                maximum: ARCHIVE_PAGE_SIZE_MAX,
            })
        );
        let args = serde_json::json!({ "request": request });
        assert_eq!(args["request"]["page"]["limit"], 201);
    }

    #[test]
    fn publish_result_carries_the_generated_identifier_across_the_ipc_boundary() {
        let result: PublishAdpListingResult = serde_json::from_value(serde_json::json!({
            "event_id": "listing-event",
            "game_id": "2f9a1c34-5b6d-4e7f-8a9b-0c1d2e3f4a5b",
            "d_tag": "2f9a1c34-5b6d-4e7f-8a9b-0c1d2e3f4a5b",
            "game_coordinate": "30402:publisherhex:2f9a1c34-5b6d-4e7f-8a9b-0c1d2e3f4a5b",
            "acceptance_event_id": null,
            "fulfillment_pubkey": null,
            "file_hash": null,
            "uploads": []
        }))
        .expect("desktop publish result should deserialize");

        assert_eq!(
            result.game_id.map(|game_id| game_id.to_string()).as_deref(),
            Some("2f9a1c34-5b6d-4e7f-8a9b-0c1d2e3f4a5b")
        );
        assert!(result.game_coordinate.ends_with(&result.d_tag));

        // Edits of legacy listings report no generated identifier and keep a
        // non-UUID `d` tag.
        let legacy: PublishAdpListingResult = serde_json::from_value(serde_json::json!({
            "event_id": "listing-event",
            "game_id": null,
            "d_tag": "my-game-v1",
            "game_coordinate": "30402:publisherhex:my-game-v1",
            "acceptance_event_id": null,
            "fulfillment_pubkey": null,
            "file_hash": null,
            "uploads": []
        }))
        .expect("legacy publish result should deserialize");

        assert_eq!(legacy.game_id, None);
        assert_eq!(legacy.d_tag, "my-game-v1");
    }

    #[test]
    fn store_page_publish_web_error_is_explicit() {
        assert_eq!(
            store_page_publishing_unsupported_error(),
            "Store Page publishing is unavailable in standalone web builds."
        );
    }

    #[test]
    fn install_progress_payload_matches_the_desktop_event_contract() {
        let payload: InstallProgressPayload = serde_json::from_value(serde_json::json!({
            "game_coordinate": "30402:publisher:game",
            "phase": "installing",
            "bytes": 512,
            "total": 1024
        }))
        .expect("desktop progress payload should deserialize");

        assert_eq!(payload.bytes, 512);
        assert_eq!(payload.total, Some(1024));
        assert_eq!(payload.game_coordinate, "30402:publisher:game");
        assert_eq!(payload.phase, InstallProgressPhase::Installing);
    }

    #[test]
    fn installation_lifecycle_contract_matches_desktop_serde_fields() {
        let installed: InstalledGame = serde_json::from_value(serde_json::json!({
            "installation_id": "install-1",
            "game_coordinate": "30402:publisher:game",
            "installation_root": "/managed/install-1",
            "package_digest": "ab",
            "package_format": "zip",
            "state": "ready",
            "recovery_discriminator": null,
            "retained_package": true,
            "launchable": false,
            "installed_at": 42
        }))
        .expect("desktop installed game should deserialize");
        assert_eq!(installed.state, InstallationState::Ready);
        assert!(installed.retained_package);
        assert!(!installed.launchable);

        let issue: InstallationLifecycleIssue = serde_json::from_value(serde_json::json!({
            "installation_id": "install-2",
            "game_coordinate": "30402:publisher:game",
            "installation_root": "/managed/install-2",
            "state": "failed",
            "recovery_discriminator": "final_root_present",
            "updated_at": 43
        }))
        .expect("desktop lifecycle issue should deserialize");
        assert_eq!(
            issue.recovery_discriminator,
            Some(InstallationRecoveryDiscriminator::FinalRootPresent)
        );

        let sizes: UninstallSizes = serde_json::from_value(serde_json::json!({
            "installation_size_bytes": 1024,
            "retained_package_size_bytes": 2048
        }))
        .expect("desktop uninstall sizes should deserialize");
        assert_eq!(sizes.retained_package_size_bytes, Some(2048));
    }

    #[test]
    fn installation_command_payloads_use_exact_tauri_argument_names() {
        assert_eq!(RESUME_UNINSTALL_COMMAND, "resume_uninstall");
        assert_eq!(
            REINSTALL_RETAINED_PACKAGE_COMMAND,
            "reinstall_retained_package"
        );
        assert_eq!(
            installation_id_args("install-1".into()),
            serde_json::json!({ "installationId": "install-1" })
        );
        assert_eq!(
            uninstall_game_args("install-1".into(), true),
            serde_json::json!({
                "installationId": "install-1",
                "retainPackage": true
            })
        );
        assert_eq!(
            retained_package_identity_args("30402:publisher:game".into(), "package-1".into(),),
            serde_json::json!({
                "gameCoordinate": "30402:publisher:game",
                "expectedPackageId": "package-1"
            })
        );
    }
}
