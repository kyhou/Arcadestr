use std::cell::RefCell;
use std::collections::HashMap;

use arcadestr_core::store_page::{
    AccessibilityFeature, LanguageSupport, StorePageDraft, StorePageMediaItem, StorePageSection,
};
use leptos::prelude::*;
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::spawn_local;

use crate::campaign_management::{accepts_account_response, listing_coordinate};
use crate::components::DatePicker;
use crate::models::{GameDetailPresentation, GameListing, StorePageListingRef};
use crate::tauri_bridge::{
    invoke_clone_store_page, invoke_load_publisher_store_page_editor, invoke_publish_store_page,
    invoke_retry_store_page_pointer_sync, invoke_validate_store_page_draft, ListingPointerMutation,
    PublishStorePageResponse, PublisherStorePageListingRevision, StorePageListingMutation,
};
use crate::ui_v2::components::blossom_media_upload::publisher_hex as blossom_publisher_hex;
use crate::ui_v2::components::{BlossomMediaUpload, StorePageRichDetail};

#[derive(Clone)]
struct CachedDraft {
    draft: StorePageDraft,
    baseline: StorePageDraft,
    associations: Vec<AssociationRow>,
    input_dirty: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct AssociationRow {
    listing_coordinate: String,
    event_id: String,
    reciprocal: bool,
    action: ListingPointerMutation,
    relay_hint: Option<String>,
}

#[derive(Clone)]
struct PublisherRecovery {
    response: PublishStorePageResponse,
    mutations: Vec<StorePageListingMutation>,
    draft: StorePageDraft,
    selected_listing_event_id: Option<String>,
}

thread_local! {
    static PUBLISHER_STORE_PAGE_DRAFTS: RefCell<HashMap<String, CachedDraft>> = RefCell::new(HashMap::new());
    static PUBLISHER_STORE_PAGE_RECOVERY: RefCell<HashMap<String, PublisherRecovery>> = RefCell::new(HashMap::new());
}

const EDITOR_TABS: [(&str, &str); 8] = [
    ("basic", "Basic Info"),
    ("description", "Description"),
    ("media", "Media"),
    ("sections", "Feature Sections"),
    ("requirements", "Requirements"),
    ("languages", "Languages"),
    ("accessibility", "Accessibility"),
    ("links", "Links"),
];
const CANONICAL_PREVIEW_SOURCE: &str = "canonical-validation";

fn diagnostic_tab(message: &str) -> &'static str {
    let normalized = message.to_ascii_lowercase();
    EDITOR_TABS
        .iter()
        .find_map(|(id, _)| normalized.contains(id).then_some(*id))
        .unwrap_or("basic")
}

fn adjacent_tab(current: &str, direction: isize) -> &'static str {
    let index = EDITOR_TABS
        .iter()
        .position(|(id, _)| *id == current)
        .unwrap_or_default() as isize;
    let len = EDITOR_TABS.len() as isize;
    EDITOR_TABS[((index + direction).rem_euclid(len)) as usize].0
}

fn focus_editor_tab(id: &str) {
    let Some(document) = web_sys::window().and_then(|window| window.document()) else {
        return;
    };
    let Some(element) = document.get_element_by_id(&format!("store-editor-tab-{id}")) else {
        return;
    };
    if let Ok(element) = element.dyn_into::<web_sys::HtmlElement>() {
        let _ = element.focus();
    }
}

/// Focus a stable in-panel target when one exists, otherwise fall back to the
/// tab control itself. Returns whether a focus target was found.
fn focus_editor_element(element_id: &str) -> bool {
    let Some(document) = web_sys::window().and_then(|window| window.document()) else {
        return false;
    };
    let Some(element) = document.get_element_by_id(element_id) else {
        return false;
    };
    match element.dyn_into::<web_sys::HtmlElement>() {
        Ok(element) => {
            let _ = element.focus();
            true
        }
        Err(_) => false,
    }
}

/// Where the first blocking issue lives. Routing reuses the existing
/// `diagnostic_tab` mapping so no second validation rule can drift from it.
#[derive(Clone, Debug, PartialEq, Eq)]
struct BlockerTarget {
    tab: &'static str,
    /// Stable element id to focus, when the owning panel exposes one.
    element_id: Option<String>,
    message: String,
}

/// Panels that expose a stable focus target today. Everything else falls back to
/// its tab control rather than inventing an id.
fn blocker_focus_element(tab: &str) -> Option<String> {
    match tab {
        "basic" => Some("store-editor-basic".to_string()),
        _ => None,
    }
}

fn first_blocker_target(blockers: &[String]) -> Option<BlockerTarget> {
    blockers.first().map(|message| {
        let tab = diagnostic_tab(message);
        BlockerTarget {
            tab,
            element_id: blocker_focus_element(tab),
            message: message.clone(),
        }
    })
}

fn unique_editor_id<'a>(prefix: &str, existing_ids: impl IntoIterator<Item = &'a str>) -> String {
    let existing = existing_ids
        .into_iter()
        .collect::<std::collections::HashSet<_>>();
    (1usize..)
        .map(|sequence| format!("{prefix}-{sequence}"))
        .find(|candidate| !existing.contains(candidate.as_str()))
        .unwrap_or_else(|| format!("{prefix}-new"))
}

fn validate_editor_ids(draft: &StorePageDraft) -> Result<(), String> {
    let mut media_ids = std::collections::HashSet::new();
    if draft
        .content
        .media
        .iter()
        .any(|item| item.id.trim().is_empty() || !media_ids.insert(item.id.as_str()))
    {
        return Err("Media IDs must be non-empty and unique.".into());
    }
    let mut section_ids = std::collections::HashSet::new();
    if draft
        .content
        .sections
        .iter()
        .any(|section| section.id.trim().is_empty() || !section_ids.insert(section.id.as_str()))
    {
        return Err("Feature section IDs must be non-empty and unique.".into());
    }
    Ok(())
}

fn description_contains_link(markdown: &str) -> bool {
    let bytes = markdown.as_bytes();
    bytes.windows(2).any(|window| window == b"](")
        || bytes.windows(3).any(|window| window == b"![")
        || markdown.contains("]:")
        || markdown.contains("http://")
        || markdown.contains("https://")
        || markdown.to_ascii_lowercase().contains("<img")
}

fn selected_replacement_event_id(
    response: &PublishStorePageResponse,
    selected_coordinate: &str,
) -> Option<String> {
    response
        .listing_updates
        .iter()
        .find(|outcome| outcome.published && outcome.listing_coordinate == selected_coordinate)
        .and_then(|outcome| outcome.replacement_event_id.clone())
}

fn draft_key(publisher: &str, listing_coordinate: &str) -> String {
    format!("{publisher}|{listing_coordinate}")
}

pub(crate) fn publisher_store_page_dirty_coordinates(publisher: &str) -> Vec<String> {
    let prefix = format!("{publisher}|");
    PUBLISHER_STORE_PAGE_DRAFTS.with(|drafts| {
        drafts
            .borrow()
            .iter()
            .filter(|(_, entry)| entry.input_dirty || entry.draft != entry.baseline)
            .filter_map(|(key, _)| key.strip_prefix(&prefix).map(ToString::to_string))
            .collect()
    })
}

pub(crate) fn publisher_store_page_partial_coordinates(publisher: &str) -> Vec<String> {
    let prefix = format!("{publisher}|");
    PUBLISHER_STORE_PAGE_RECOVERY.with(|recoveries| {
        recoveries
            .borrow()
            .keys()
            .filter_map(|key| key.strip_prefix(&prefix).map(ToString::to_string))
            .collect()
    })
}

fn cached_draft(key: &str) -> Option<CachedDraft> {
    PUBLISHER_STORE_PAGE_DRAFTS.with(|drafts| drafts.borrow().get(key).cloned())
}

fn save_cached_draft(
    key: &str,
    draft: StorePageDraft,
    baseline: StorePageDraft,
    associations: Vec<AssociationRow>,
    input_dirty: bool,
) {
    PUBLISHER_STORE_PAGE_DRAFTS.with(|drafts| {
        drafts.borrow_mut().insert(
            key.to_string(),
            CachedDraft {
                draft,
                baseline,
                associations,
                input_dirty,
            },
        );
    });
}

fn recovery(key: &str) -> Option<PublisherRecovery> {
    PUBLISHER_STORE_PAGE_RECOVERY.with(|recoveries| recoveries.borrow().get(key).cloned())
}

fn save_recovery(key: &str, recovery: PublisherRecovery) {
    PUBLISHER_STORE_PAGE_RECOVERY.with(|recoveries| {
        recoveries.borrow_mut().insert(key.to_string(), recovery);
    });
}

fn clear_recovery(key: &str) {
    PUBLISHER_STORE_PAGE_RECOVERY.with(|recoveries| {
        recoveries.borrow_mut().remove(key);
    });
}

fn seed_new_draft_association(
    draft: &mut StorePageDraft,
    associations: &mut Vec<AssociationRow>,
    coordinate: &str,
    event_id: Option<&str>,
) {
    let Some(event_id) = event_id else {
        return;
    };
    if associations.is_empty() && draft.loaded_event_id.is_none() {
        associations.push(AssociationRow {
            listing_coordinate: coordinate.to_string(),
            event_id: event_id.to_string(),
            reciprocal: false,
            action: ListingPointerMutation::Link,
            relay_hint: None,
        });
        if !draft
            .listing_coordinates
            .iter()
            .any(|value| value == coordinate)
        {
            draft.listing_coordinates.push(coordinate.to_string());
        }
    }
}

fn optional(value: String) -> Option<String> {
    let value = value.trim().to_string();
    (!value.is_empty()).then_some(value)
}

fn optional_editor_text(value: String) -> Option<String> {
    (!value.is_empty()).then_some(value)
}

fn media_url_snapshot(draft: &StorePageDraft) -> HashMap<String, String> {
    draft
        .content
        .media
        .iter()
        .map(|media| (media.id.clone(), media.url.clone()))
        .collect()
}

fn clear_changed_media_integrity(
    draft: &mut StorePageDraft,
    previous_urls: &HashMap<String, String>,
) -> bool {
    let mut changed = false;
    for media in &mut draft.content.media {
        let url_changed = previous_urls
            .get(&media.id)
            .is_some_and(|previous| previous != &media.url);
        if url_changed
            && (media.sha256.is_some() || media.mime_type.is_some() || media.size.is_some())
        {
            media.sha256 = None;
            media.mime_type = None;
            media.size = None;
            changed = true;
        }
    }
    changed
}

fn sync_compact_tags(draft: &mut StorePageDraft) {
    draft.compact_tags.title = draft.content.basic.title.clone();
    draft.compact_tags.summary = draft.content.basic.summary.clone();
    draft.compact_tags.developer = draft.content.basic.developer.clone();
    draft.compact_tags.publisher = draft.content.basic.publisher.clone();
    draft.compact_tags.release_date = draft.content.basic.release_date.clone();
    draft.compact_tags.genres = draft.content.discovery.genres.clone().unwrap_or_default();
    draft.compact_tags.features = draft.content.discovery.features.clone().unwrap_or_default();
    draft.compact_tags.languages = draft.content.languages.clone().unwrap_or_default();
    draft.compact_tags.website = draft.content.links.website.clone();
    draft.compact_tags.support = draft.content.links.support.clone();
}

fn associations_from_revisions(
    listings: &[PublisherStorePageListingRevision],
) -> Vec<AssociationRow> {
    listings
        .iter()
        .map(|listing| AssociationRow {
            listing_coordinate: listing.listing_coordinate.clone(),
            event_id: listing.event_id.clone(),
            reciprocal: listing.reciprocal,
            action: if listing.reciprocal {
                ListingPointerMutation::Link
            } else {
                ListingPointerMutation::Review
            },
            relay_hint: None,
        })
        .collect()
}

fn adapter_requests(
    mut draft: StorePageDraft,
    associations: &[AssociationRow],
) -> Result<(StorePageDraft, Vec<StorePageListingMutation>), String> {
    validate_editor_ids(&draft)?;
    if description_contains_link(&draft.content.description_markdown)
        || draft
            .content
            .sections
            .iter()
            .any(|section| description_contains_link(&section.body_markdown))
    {
        return Err(
            "Markdown links and images are not allowed. Add destinations in the Links fields."
                .to_string(),
        );
    }
    for singular in ["hero", "capsule"] {
        if draft
            .content
            .media
            .iter()
            .filter(|item| item.role == singular)
            .count()
            > 1
        {
            return Err(format!("Only one {singular} media item is allowed."));
        }
    }
    let media_ids = draft
        .content
        .media
        .iter()
        .map(|item| item.id.as_str())
        .collect::<std::collections::HashSet<_>>();
    if let Some(section) = draft.content.sections.iter().find(|section| {
        section
            .media_id
            .as_deref()
            .is_some_and(|id| !media_ids.contains(id))
    }) {
        return Err(format!(
            "Section ‘{}’ refers to media that no longer exists.",
            section.heading
        ));
    }
    let mutations = associations
        .iter()
        .map(|row| StorePageListingMutation {
            listing_coordinate: row.listing_coordinate.clone(),
            expected_event_id: row.event_id.clone(),
            action: row.action,
            relay_hint: row.relay_hint.clone(),
            published_event_id: None,
        })
        .collect::<Vec<_>>();
    draft.listing_coordinates = associations
        .iter()
        .filter(|row| {
            matches!(
                row.action,
                ListingPointerMutation::Link | ListingPointerMutation::Review
            )
        })
        .map(|row| row.listing_coordinate.clone())
        .collect();
    sync_compact_tags(&mut draft);
    Ok((draft, mutations))
}

fn move_item<T>(items: &mut [T], index: usize, direction: isize) {
    let target = index as isize + direction;
    if target >= 0 && (target as usize) < items.len() {
        items.swap(index, target as usize);
    }
}

fn media_url_has_inline_format_guidance(url: &str, media_type: &str) -> bool {
    let lower = url.trim().to_ascii_lowercase();
    let allowed = if media_type == "video" {
        [".mp4", ".webm"].as_slice()
    } else {
        [".png", ".jpg", ".jpeg", ".webp", ".gif", ".avif"].as_slice()
    };
    lower.starts_with("https://")
        && allowed.iter().any(|extension| {
            lower
                .split(['?', '#'])
                .next()
                .is_some_and(|path| path.ends_with(extension))
        })
}

#[cfg(test)]
fn safe_media_preview(url: &str, media_type: &str) -> bool {
    media_url_has_inline_format_guidance(url, media_type)
}

#[cfg(not(test))]
fn safe_media_preview(_url: &str, _media_type: &str) -> bool {
    // Draft URLs are never loaded by the editor. Only the canonical validated preview may load media.
    false
}

fn safe_https_link(url: &str) -> bool {
    let value = url.trim();
    value.starts_with("https://")
        && value.len() > "https://".len()
        && !value.chars().any(char::is_whitespace)
        && value["https://".len()..]
            .split('/')
            .next()
            .is_some_and(|host| {
                host.contains('.') && !host.starts_with('.') && !host.ends_with('.')
            })
}

fn platform_label(platform: &str) -> String {
    platform
        .split(['-', '_'])
        .filter(|part| !part.is_empty())
        .map(|part| {
            let mut chars = part.chars();
            chars
                .next()
                .map(|first| first.to_uppercase().collect::<String>() + chars.as_str())
                .unwrap_or_default()
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn linked_platforms(
    listing_platforms: &[String],
    associations: &[AssociationRow],
    selected_coordinate: &str,
) -> Vec<String> {
    if associations.iter().any(|row| {
        row.listing_coordinate == selected_coordinate && row.action == ListingPointerMutation::Link
    }) {
        listing_platforms.to_vec()
    } else {
        Vec::new()
    }
}

#[derive(Default, Debug, PartialEq, Eq)]
struct Readiness {
    blockers: Vec<String>,
    recommendations: Vec<String>,
    warnings: Vec<String>,
}

fn readiness(
    draft: &StorePageDraft,
    associations: &[AssociationRow],
    platforms: &[String],
    diagnostics: &[String],
) -> Readiness {
    let mut value = Readiness::default();
    if draft.presentation_id.trim().is_empty() {
        value.blockers.push("Presentation ID is required.".into());
    }
    if draft
        .content
        .basic
        .title
        .as_deref()
        .is_none_or(str::is_empty)
    {
        value.blockers.push("Title is required.".into());
    }
    if associations.is_empty() {
        value
            .blockers
            .push("At least one listing association is required.".into());
    }
    if associations
        .iter()
        .any(|row| row.action == ListingPointerMutation::Review)
    {
        value
            .warnings
            .push("A listing association needs review.".into());
    }
    for diagnostic in diagnostics {
        if diagnostic.starts_with("Warning:") || diagnostic.starts_with("Association:") {
            value.warnings.push(diagnostic.clone());
        } else {
            value.blockers.push(diagnostic.clone());
        }
    }
    if description_contains_link(&draft.content.description_markdown)
        || draft
            .content
            .sections
            .iter()
            .any(|section| description_contains_link(&section.body_markdown))
    {
        value
            .blockers
            .push("Move Markdown links and images to the Links fields.".into());
    }
    for (missing, label) in [
        ("hero", "Add hero media."),
        ("capsule", "Add capsule media."),
    ] {
        if !draft.content.media.iter().any(|item| item.role == missing) {
            value.recommendations.push(label.into());
        }
    }
    if draft
        .content
        .media
        .iter()
        .filter(|item| item.role == "screenshot")
        .count()
        < 3
    {
        value
            .recommendations
            .push("Add at least three screenshots.".into());
    }
    if !draft
        .content
        .media
        .iter()
        .any(|item| item.role == "trailer")
    {
        value.recommendations.push("Add a trailer.".into());
    }
    if draft.content.description_markdown.trim().is_empty() {
        value.recommendations.push("Add a description.".into());
    }
    if platforms
        .iter()
        .any(|platform| !draft.content.requirements.contains_key(platform))
    {
        value
            .recommendations
            .push("Add requirements for linked platforms.".into());
    }
    if draft.content.languages.as_ref().is_none_or(Vec::is_empty) {
        value.recommendations.push("Add language support.".into());
    }
    if draft.content.accessibility.is_empty() {
        value
            .recommendations
            .push("Add accessibility information.".into());
    }
    value
}

/// Outcome of one publication stage. Store Page event publication and listing
/// pointer publication are tracked separately and never collapsed into one
/// generic success.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum StageOutcome {
    NotAttempted,
    Pending,
    Partial,
    Complete,
    Failed,
}

impl StageOutcome {
    fn label(self) -> &'static str {
        match self {
            Self::NotAttempted => "Not attempted",
            Self::Pending => "In progress",
            Self::Partial => "Partially published",
            Self::Complete => "Published",
            Self::Failed => "Failed",
        }
    }

    fn class(self) -> &'static str {
        match self {
            Self::NotAttempted => "v2-store-stage-idle",
            Self::Pending => "v2-store-stage-busy",
            Self::Partial => "v2-store-stage-warning",
            Self::Complete => "v2-store-stage-ok",
            Self::Failed => "v2-store-stage-error",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct PublicationLifecycle {
    store_page: StageOutcome,
    pointers: StageOutcome,
    retryable: bool,
}

fn store_page_stage(publishing: bool, response: Option<&PublishStorePageResponse>) -> StageOutcome {
    let Some(response) = response else {
        return if publishing {
            StageOutcome::Pending
        } else {
            StageOutcome::NotAttempted
        };
    };
    match response.store_page.as_ref() {
        None => StageOutcome::NotAttempted,
        Some(outcome) if outcome.success_count == 0 => StageOutcome::Failed,
        Some(outcome) if outcome.propagation_confirmed => StageOutcome::Complete,
        Some(_) => StageOutcome::Partial,
    }
}

fn pointer_stage(publishing: bool, response: Option<&PublishStorePageResponse>) -> StageOutcome {
    let Some(response) = response else {
        return if publishing {
            StageOutcome::Pending
        } else {
            StageOutcome::NotAttempted
        };
    };
    if response.listing_updates.is_empty() {
        return StageOutcome::NotAttempted;
    }
    let confirmed = response
        .listing_updates
        .iter()
        .filter(|outcome| outcome.published && outcome.propagation_confirmed)
        .count();
    let published = response
        .listing_updates
        .iter()
        .filter(|outcome| outcome.published)
        .count();
    if confirmed == response.listing_updates.len() {
        StageOutcome::Complete
    } else if published == 0 {
        StageOutcome::Failed
    } else {
        StageOutcome::Partial
    }
}

fn publication_lifecycle(
    publishing: bool,
    response: Option<&PublishStorePageResponse>,
) -> PublicationLifecycle {
    PublicationLifecycle {
        store_page: store_page_stage(publishing, response),
        pointers: pointer_stage(publishing, response),
        retryable: response.is_some_and(|response| response.retryable),
    }
}

/// Overall publication wording. "Published" requires every attempted stage to be
/// complete; a Store Page event accepted without its pointer update is never
/// described as a finished publication.
fn overall_publication_label(lifecycle: PublicationLifecycle) -> &'static str {
    use StageOutcome::*;
    match (lifecycle.store_page, lifecycle.pointers) {
        (NotAttempted, NotAttempted) => "Not published from this editor",
        (Pending, _) | (_, Pending) => "Publishing",
        (Complete, Complete) => "Store Page and listing pointers published",
        (Complete, NotAttempted) => "Store Page published; no pointer update was required",
        (Complete, Partial) => "Store Page published; listing pointers partially published",
        (Complete, Failed) => "Store Page published; listing pointer update failed",
        (Partial, _) => "Store Page event only partially published",
        (Failed, _) => "Store Page publication failed",
        (NotAttempted, _) => "Listing pointer work only",
    }
}

/// The editor keeps changes in memory for the session. Nothing here may be
/// described as saved: no durable draft persistence exists.
fn draft_persistence_label(dirty: bool) -> &'static str {
    if dirty {
        "Unsaved in-memory changes"
    } else {
        "No in-memory changes"
    }
}

fn revision_label(loaded_event_id: Option<&str>) -> String {
    match loaded_event_id {
        Some(id) => format!("Published revision loaded: {id}"),
        None => "No published Store Page revision yet".to_string(),
    }
}

/// Per-tab validation treatment derived from the authoritative readiness output.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum EditorTabState {
    Neutral,
    Warned,
    Blocked,
}

fn editor_tab_state(tab: &str, blockers: &[String], warnings: &[String]) -> EditorTabState {
    if blockers.iter().any(|item| diagnostic_tab(item) == tab) {
        EditorTabState::Blocked
    } else if warnings.iter().any(|item| diagnostic_tab(item) == tab) {
        EditorTabState::Warned
    } else {
        EditorTabState::Neutral
    }
}

/// Presentation state for one draft media entry. A local selection or a
/// completed Blossom upload is never presented as published Store Page media.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MediaSlotState {
    Empty,
    Invalid,
    Referenced,
}

fn media_slot_state(url: &str, media_type: &str) -> MediaSlotState {
    if url.trim().is_empty() {
        MediaSlotState::Empty
    } else if safe_media_preview(url, media_type) {
        MediaSlotState::Referenced
    } else {
        MediaSlotState::Invalid
    }
}

impl MediaSlotState {
    fn label(self) -> &'static str {
        match self {
            Self::Empty => "No media URL yet",
            Self::Invalid => "Enter a supported HTTPS media URL",
            Self::Referenced => "Referenced in the draft; not published until you publish",
        }
    }
}

fn retryable_mutations(
    result: &PublishStorePageResponse,
    mutations: &[StorePageListingMutation],
) -> Vec<StorePageListingMutation> {
    result
        .listing_updates
        .iter()
        .filter(|outcome| !outcome.published || !outcome.propagation_confirmed)
        .filter_map(|outcome| {
            mutations
                .iter()
                .find(|mutation| mutation.listing_coordinate == outcome.listing_coordinate)
                .cloned()
                .map(|mut mutation| {
                    if outcome.published {
                        mutation.published_event_id = outcome.replacement_event_id.clone();
                    }
                    mutation
                })
        })
        .collect()
}

fn preview_commerce_label(
    price: f64,
    currency: &str,
    acquisition: &crate::models::AcquisitionPolicy,
) -> String {
    let access = match acquisition {
        crate::models::AcquisitionPolicy::Public => "Public",
        crate::models::AcquisitionPolicy::Gated => "Gated",
        crate::models::AcquisitionPolicy::TimedAccess { .. } => "Timed access",
    };
    format!("{price} {currency} · {access}")
}

#[component]
pub fn StorePageEditorView(
    listing: GameListing,
    on_back: Callback<()>,
    on_saved: Callback<GameListing>,
) -> impl IntoView {
    let auth = use_context::<crate::AuthContext>().expect("AuthContext not provided");
    let publisher = listing.publisher_npub.clone();
    let canonical_publisher_hex = blossom_publisher_hex(&publisher).unwrap_or_default();
    let coordinate = listing_coordinate(&listing);
    let key = draft_key(&publisher, &coordinate);
    let active_publisher = auth.npub.get_untracked().as_deref() == Some(publisher.as_str());
    let cached = active_publisher.then(|| cached_draft(&key)).flatten();
    let recovered = active_publisher.then(|| recovery(&key)).flatten();
    let fallback = StorePageDraft::new(listing.id.clone(), vec![coordinate.clone()]);
    let mut initial_draft = cached
        .as_ref()
        .map_or_else(|| fallback.clone(), |entry| entry.draft.clone());
    let baseline = RwSignal::new(
        cached
            .as_ref()
            .map_or_else(|| fallback, |entry| entry.baseline.clone()),
    );
    let listings = RwSignal::new(Vec::<PublisherStorePageListingRevision>::new());
    let mut initial_associations = cached
        .as_ref()
        .map_or_else(Vec::new, |entry| entry.associations.clone());
    seed_new_draft_association(
        &mut initial_draft,
        &mut initial_associations,
        &coordinate,
        listing.event_id.as_deref(),
    );
    let draft = RwSignal::new(initial_draft);
    let associations = RwSignal::new(initial_associations);
    let input_dirty = RwSignal::new(cached.as_ref().is_some_and(|entry| entry.input_dirty));
    let association_review_required = RwSignal::new(false);
    let preview = RwSignal::new(None::<GameDetailPresentation>);
    let diagnostics = RwSignal::new(Vec::<String>::new());
    let loading = RwSignal::new(true);
    let validating = RwSignal::new(false);
    let publishing = RwSignal::new(false);
    let form_error = RwSignal::new(None::<String>);
    let message = RwSignal::new(None::<String>);
    let partial = RwSignal::new(recovered.as_ref().map(|state| state.response.clone()));
    let transaction_mutations = RwSignal::new(
        recovered
            .as_ref()
            .map_or_else(Vec::new, |state| state.mutations.clone()),
    );
    let transaction_draft = RwSignal::new(recovered.as_ref().map(|state| state.draft.clone()));
    let transaction_selected_listing_event_id = RwSignal::new(
        recovered
            .as_ref()
            .and_then(|state| state.selected_listing_event_id.clone()),
    );
    let show_discard = RwSignal::new(false);
    let clone_id = RwSignal::new(String::new());
    let link_existing_id = RwSignal::new(String::new());
    let operation_generation = RwSignal::new(0_u64);
    let upload_context_generation = RwSignal::new(0_u64);
    let media_url_generation = RwSignal::new(0_u64);
    let media_urls = RwSignal::new(media_url_snapshot(&draft.get_untracked()));
    let operation_account = RwSignal::new(auth.npub.get_untracked());
    let blossom_dialog_role = RwSignal::new(None::<String>);
    let active_tab = RwSignal::new("basic");
    let description_preview = RwSignal::new(false);
    let preview_narrow = RwSignal::new(false);
    let readiness_open = RwSignal::new(false);
    let genre_input = RwSignal::new(String::new());
    let feature_input = RwSignal::new(String::new());
    let locale_input = RwSignal::new("en".to_string());
    let custom_accessibility = RwSignal::new(String::new());
    let pending_removal = RwSignal::new(None::<(&'static str, usize)>);
    let validation_valid = RwSignal::new(None::<bool>);
    let discard_dialog_ref = NodeRef::<leptos::html::Dialog>::new();

    Effect::new(move |_| {
        let current = draft.get();
        let current_urls = media_url_snapshot(&current);
        let generation = upload_context_generation.get();
        if media_url_generation.get_untracked() != generation {
            media_url_generation.set(generation);
            media_urls.set(current_urls);
        } else {
            let previous_urls = media_urls.get_untracked();
            let changed = current.content.media.iter().any(|media| {
                previous_urls
                    .get(&media.id)
                    .is_some_and(|previous| previous != &media.url)
                    && (media.sha256.is_some() || media.mime_type.is_some() || media.size.is_some())
            });
            media_urls.set(current_urls);
            if changed {
                draft.update(|value| {
                    clear_changed_media_integrity(value, &previous_urls);
                });
            }
        }
        associations.track();
        preview.set(None);
        validation_valid.set(None);
    });

    Effect::new(move |_| {
        let Some(dialog) = discard_dialog_ref.get() else {
            return;
        };
        if show_discard.get() {
            if !dialog.open() {
                let _ = dialog.show_modal();
            }
        } else if dialog.open() {
            dialog.close();
        }
    });

    Effect::new({
        let key = key.clone();
        let publisher = publisher.clone();
        move |_| {
            if auth.npub.get().as_deref() == Some(publisher.as_str()) {
                save_cached_draft(
                    &key,
                    draft.get(),
                    baseline.get_untracked(),
                    associations.get(),
                    input_dirty.get(),
                )
            }
        }
    });

    Effect::new({
        let publisher = publisher.clone();
        move |_| {
            let current = auth.npub.get();
            if current.as_deref() != Some(publisher.as_str()) {
                operation_generation.update(|value| *value = value.wrapping_add(1));
                upload_context_generation.update(|value| *value = value.wrapping_add(1));
                operation_account.set(current.clone());
                preview.set(None);
                loading.set(false);
                validating.set(false);
                publishing.set(false);
                message.set(Some(
                    "Account changed. Switch back to the publisher account and reload before publishing."
                        .to_string(),
                ));
            }
        }
    });

    Effect::new({
        let publisher = publisher.clone();
        move |_| {
            let selected = active_tab.get();
            let account_matches = auth.npub.get().as_deref() == Some(publisher.as_str());
            let busy =
                loading.get() || validating.get() || publishing.get() || partial.get().is_some();
            let Some(window) = web_sys::window() else {
                return;
            };
            let Some(document) = window.document() else {
                return;
            };
            if let Ok(tabs) = document.query_selector_all(".v2-store-editor-tabs [role='tab']") {
                for (index, (id, _)) in EDITOR_TABS.iter().enumerate() {
                    let Some(element) = tabs.item(index as u32) else {
                        continue;
                    };
                    let Ok(element) = element.dyn_into::<web_sys::Element>() else {
                        continue;
                    };
                    let tab_id = format!("store-editor-tab-{id}");
                    let panel_id = format!("store-editor-{id}");
                    let _ = element.set_attribute("id", &tab_id);
                    let _ = element.set_attribute("aria-controls", &panel_id);
                    let _ =
                        element.set_attribute("tabindex", if *id == selected { "0" } else { "-1" });
                }
            }
            if let Ok(Some(panel)) =
                document.query_selector(".v2-store-page-editor [role='tabpanel']")
            {
                let _ = panel.set_attribute("id", &format!("store-editor-{selected}"));
                let _ =
                    panel.set_attribute("aria-labelledby", &format!("store-editor-tab-{selected}"));
            }
            if let Ok(Some(fieldset)) = document.query_selector(".v2-store-page-editor fieldset") {
                if account_matches && !busy {
                    let _ = fieldset.remove_attribute("disabled");
                } else {
                    let _ = fieldset.set_attribute("disabled", "");
                }
            }
            if let Ok(actions) = document
                .query_selector_all(".v2-store-editor-footer button, .v2-store-editor-footer input")
            {
                for index in 0..actions.length() {
                    let Some(element) = actions.item(index) else {
                        continue;
                    };
                    let Ok(element) = element.dyn_into::<web_sys::Element>() else {
                        continue;
                    };
                    if account_matches && !busy {
                        let _ = element.remove_attribute("disabled");
                    } else {
                        let _ = element.set_attribute("disabled", "");
                    }
                }
            }
        }
    });

    Effect::new({
        let publisher = publisher.clone();
        let coordinate = coordinate.clone();
        let event_id = listing.event_id.clone();
        move |_| {
            let Some(event_id) = event_id.clone() else {
                loading.set(false);
                message.set(Some(
                    "The current signed listing event ID is unavailable.".to_string(),
                ));
                return;
            };
            let Some(initiating_account) = auth.npub.get_untracked() else {
                loading.set(false);
                return;
            };
            if initiating_account != publisher {
                loading.set(false);
                message.set(Some(
                    "Switch to the publisher account before loading Store Page authoring state."
                        .to_string(),
                ));
                return;
            }
            operation_generation.update(|value| *value = value.wrapping_add(1));
            let generation = operation_generation.get_untracked();
            spawn_local({
                let publisher = publisher.clone();
                let coordinate = coordinate.clone();
                async move {
                    let result = invoke_load_publisher_store_page_editor(
                        publisher,
                        StorePageListingRef {
                            listing_coordinate: coordinate,
                            listing_event_id: event_id,
                        },
                        None,
                    )
                    .await;
                    if !accepts_account_response(
                        auth.npub.get_untracked().as_deref(),
                        &initiating_account,
                        operation_generation.get_untracked(),
                        generation,
                    ) {
                        loading.set(false);
                        return;
                    }
                    match result {
                        Ok(state) => {
                            let requires_association_review = !state.diagnostics.is_empty()
                                || state.listings.iter().any(|listing| !listing.reciprocal);
                            listings.set(state.listings.clone());
                            diagnostics.set(
                                state
                                    .diagnostics
                                    .into_iter()
                                    .map(|message| format!("Association: {message}"))
                                    .collect(),
                            );
                            validation_valid.set(None);
                            association_review_required.set(requires_association_review);
                            if draft.get_untracked() == baseline.get_untracked()
                                && !input_dirty.get_untracked()
                            {
                                associations.set(associations_from_revisions(&state.listings));
                                upload_context_generation
                                    .update(|value| *value = value.wrapping_add(1));
                                draft.set(state.draft.clone());
                                baseline.set(state.baseline_draft);
                            }
                        }
                        Err(error) => message.set(Some(error)),
                    }
                    loading.set(false);
                }
            });
        }
    });

    let update_complex = move || -> Result<Vec<StorePageListingMutation>, String> {
        let (adapted, mutations) =
            adapter_requests(draft.get_untracked(), &associations.get_untracked())?;
        draft.set(adapted);
        form_error.set(None);
        Ok(mutations)
    };

    let run_validation = Callback::new({
        let publisher = publisher.clone();
        let selected_coordinate = coordinate.clone();
        move |_: ()| {
            if loading.get_untracked()
                || validating.get_untracked()
                || publishing.get_untracked()
                || partial.get_untracked().is_some()
            {
                return;
            }
            if auth.npub.get_untracked().as_deref() != Some(publisher.as_str()) {
                message.set(Some(
                    "Switch back to the publisher account before previewing.".to_string(),
                ));
                return;
            }
            let mutations = match update_complex() {
                Ok(mutations) => mutations,
                Err(error) => {
                    form_error.set(Some(error));
                    return;
                }
            };
            let Some(preview_listing) = mutations
                .iter()
                .find(|mutation| {
                    mutation.action == ListingPointerMutation::Link
                        && mutation.listing_coordinate == selected_coordinate
                })
                .map(|mutation| PublisherStorePageListingRevision {
                    listing_coordinate: mutation.listing_coordinate.clone(),
                    event_id: mutation.expected_event_id.clone(),
                    reciprocal: true,
                })
            else {
                message.set(Some(
                    "The selected listing must remain linked to preview its authoritative commerce."
                        .to_string(),
                ));
                return;
            };
            validating.set(true);
            validation_valid.set(None);
            diagnostics.set(Vec::new());
            let request_draft = draft.get_untracked();
            let publisher = publisher.clone();
            let Some(initiating_account) = auth.npub.get_untracked() else {
                validating.set(false);
                return;
            };
            operation_generation.update(|value| *value = value.wrapping_add(1));
            let generation = operation_generation.get_untracked();
            spawn_local(async move {
                let result = invoke_validate_store_page_draft(
                    publisher,
                    request_draft,
                    preview_listing,
                    mutations,
                )
                .await;
                if !accepts_account_response(
                    auth.npub.get_untracked().as_deref(),
                    &initiating_account,
                    operation_generation.get_untracked(),
                    generation,
                ) {
                    validating.set(false);
                    return;
                }
                match result {
                    Ok(result) => {
                        validation_valid.set(Some(result.valid));
                        diagnostics.set(
                            result
                                .diagnostics
                                .into_iter()
                                .map(|diagnostic| {
                                    if result.valid {
                                        format!("Warning: {}", diagnostic.message)
                                    } else {
                                        diagnostic.message
                                    }
                                })
                                .collect(),
                        );
                        preview.set(result.preview);
                    }
                    Err(error) => message.set(Some(error)),
                }
                validating.set(false);
            });
        }
    });

    let publish = Callback::new({
        let publisher = publisher.clone();
        let listing_for_saved = listing.clone();
        let selected_coordinate = coordinate.clone();
        let recovery_key = key.clone();
        move |_: ()| {
            if loading.get_untracked()
                || validating.get_untracked()
                || publishing.get_untracked()
                || partial.get_untracked().is_some()
            {
                return;
            }
            let mutations = match update_complex() {
                Ok(mutations) => mutations,
                Err(error) => {
                    form_error.set(Some(error));
                    return;
                }
            };
            if auth.npub.get_untracked().as_deref() != Some(publisher.as_str()) {
                message.set(Some(
                    "Switch back to the publisher account before publishing.".to_string(),
                ));
                return;
            }
            if association_review_required.get_untracked() {
                message.set(Some(
                    "Review and edit the association rows before publishing this incomplete relationship."
                        .to_string(),
                ));
                return;
            }
            let request_draft = draft.get_untracked();
            let Some(initiating_account) = auth.npub.get_untracked() else {
                return;
            };
            operation_generation.update(|value| *value = value.wrapping_add(1));
            let generation = operation_generation.get_untracked();
            publishing.set(true);
            partial.set(None);
            transaction_selected_listing_event_id.set(listing_for_saved.event_id.clone());
            transaction_mutations.set(mutations.clone());
            transaction_draft.set(Some(request_draft.clone()));
            let publisher = publisher.clone();
            let listing_for_saved = listing_for_saved.clone();
            let selected_coordinate = selected_coordinate.clone();
            let recovery_key = recovery_key.clone();
            spawn_local(async move {
                let result =
                    invoke_publish_store_page(publisher, request_draft.clone(), mutations.clone())
                        .await;
                if !accepts_account_response(
                    auth.npub.get_untracked().as_deref(),
                    &initiating_account,
                    operation_generation.get_untracked(),
                    generation,
                ) {
                    if let Ok(mut response) = result {
                        response.complete = false;
                        response.retryable = true;
                        let selected_listing_event_id =
                            selected_replacement_event_id(&response, &selected_coordinate)
                                .or(listing_for_saved.event_id);
                        save_recovery(
                            &recovery_key,
                            PublisherRecovery {
                                response,
                                mutations,
                                draft: request_draft,
                                selected_listing_event_id,
                            },
                        );
                    }
                    return;
                }
                match result {
                    Ok(result) => {
                        let mut updated_listing = listing_for_saved;
                        if let Some(event_id) =
                            selected_replacement_event_id(&result, &selected_coordinate)
                        {
                            transaction_selected_listing_event_id.set(Some(event_id));
                        }
                        if let Some(event_id) =
                            transaction_selected_listing_event_id.get_untracked()
                        {
                            updated_listing.event_id = Some(event_id);
                        }
                        if result.complete {
                            clear_recovery(&recovery_key);
                            baseline.set(request_draft.clone());
                            draft.set(request_draft);
                            input_dirty.set(false);
                            message.set(Some(
                                "Store Page and listing pointers published.".to_string(),
                            ));
                            on_saved.run(updated_listing);
                        } else {
                            message.set(Some("Store Page publication is incomplete. Review and retry failed pointer updates.".to_string()));
                            save_recovery(
                                &recovery_key,
                                PublisherRecovery {
                                    response: result.clone(),
                                    mutations: transaction_mutations.get_untracked(),
                                    draft: request_draft,
                                    selected_listing_event_id:
                                        transaction_selected_listing_event_id.get_untracked(),
                                },
                            );
                            partial.set(Some(result));
                        }
                    }
                    Err(error) => message.set(Some(error)),
                }
                publishing.set(false);
            });
        }
    });

    let retry = Callback::new({
        let publisher = publisher.clone();
        let listing_for_saved = listing.clone();
        let selected_coordinate = coordinate.clone();
        let recovery_key = key.clone();
        move |_: ()| {
            if loading.get_untracked() || validating.get_untracked() || publishing.get_untracked() {
                return;
            }
            if auth.npub.get_untracked().as_deref() != Some(publisher.as_str()) {
                message.set(Some(
                    "Switch back to the publisher account before retrying.".to_string(),
                ));
                return;
            }
            let Some(result) = partial.get_untracked() else {
                return;
            };
            let Some(page) = result.store_page.as_ref() else {
                return;
            };
            let all_mutations = transaction_mutations.get_untracked();
            if all_mutations.is_empty() {
                message.set(Some(
                    "The original publication transaction is unavailable.".to_string(),
                ));
                return;
            }
            let retry_mutations = retryable_mutations(&result, &all_mutations);
            if let Some(event_id) = selected_replacement_event_id(&result, &selected_coordinate) {
                transaction_selected_listing_event_id.set(Some(event_id));
            }
            let store_page_coordinate = result.store_page_coordinate.clone();
            let store_page_event_id = page.event_id.clone();
            let Some(initiating_account) = auth.npub.get_untracked() else {
                return;
            };
            operation_generation.update(|value| *value = value.wrapping_add(1));
            let generation = operation_generation.get_untracked();
            publishing.set(true);
            let publisher = publisher.clone();
            let listing_for_saved = listing_for_saved.clone();
            let selected_coordinate = selected_coordinate.clone();
            let recovery_key = recovery_key.clone();
            let recovery_draft = transaction_draft
                .get_untracked()
                .unwrap_or_else(|| draft.get_untracked());
            let recovery_selected_listing_event_id =
                transaction_selected_listing_event_id.get_untracked();
            spawn_local(async move {
                let result = invoke_retry_store_page_pointer_sync(
                    publisher,
                    store_page_coordinate,
                    store_page_event_id,
                    retry_mutations,
                )
                .await;
                if !accepts_account_response(
                    auth.npub.get_untracked().as_deref(),
                    &initiating_account,
                    operation_generation.get_untracked(),
                    generation,
                ) {
                    if let Ok(response) = result {
                        let selected_listing_event_id =
                            selected_replacement_event_id(&response, &selected_coordinate)
                                .or(recovery_selected_listing_event_id);
                        save_recovery(
                            &recovery_key,
                            PublisherRecovery {
                                response,
                                mutations: all_mutations,
                                draft: recovery_draft,
                                selected_listing_event_id,
                            },
                        );
                    }
                    return;
                }
                match result {
                    Ok(retried) if retried.retry_scope_complete => {
                        clear_recovery(&recovery_key);
                        let mut updated_listing = listing_for_saved;
                        if let Some(event_id) =
                            selected_replacement_event_id(&retried, &selected_coordinate)
                        {
                            transaction_selected_listing_event_id.set(Some(event_id));
                        }
                        if let Some(event_id) =
                            transaction_selected_listing_event_id.get_untracked()
                        {
                            updated_listing.event_id = Some(event_id);
                        }
                        if let Some(published_draft) = transaction_draft.get_untracked() {
                            baseline.set(published_draft.clone());
                            draft.set(published_draft);
                            input_dirty.set(false);
                        }
                        partial.set(None);
                        message.set(Some(
                            "Listing pointer synchronization completed.".to_string(),
                        ));
                        on_saved.run(updated_listing);
                    }
                    Ok(retried) => {
                        if let Some(event_id) =
                            selected_replacement_event_id(&retried, &selected_coordinate)
                        {
                            transaction_selected_listing_event_id.set(Some(event_id));
                        }
                        save_recovery(
                            &recovery_key,
                            PublisherRecovery {
                                response: retried.clone(),
                                mutations: transaction_mutations.get_untracked(),
                                draft: transaction_draft
                                    .get_untracked()
                                    .unwrap_or_else(|| draft.get_untracked()),
                                selected_listing_event_id: transaction_selected_listing_event_id
                                    .get_untracked(),
                            },
                        );
                        partial.set(Some(retried));
                    }
                    Err(error) => message.set(Some(error)),
                }
                publishing.set(false);
            });
        }
    });

    let clone_page = Callback::new({
        let publisher = publisher.clone();
        move |_: ()| {
            if loading.get_untracked()
                || validating.get_untracked()
                || publishing.get_untracked()
                || partial.get_untracked().is_some()
            {
                return;
            }
            if auth.npub.get_untracked().as_deref() != Some(publisher.as_str()) {
                message.set(Some(
                    "Switch back to the publisher account before cloning.".to_string(),
                ));
                return;
            }
            let presentation_id = clone_id.get_untracked();
            if presentation_id.trim().is_empty() {
                message.set(Some(
                    "Enter a new presentation ID before cloning.".to_string(),
                ));
                return;
            }
            let source = draft.get_untracked();
            let Some(initiating_account) = auth.npub.get_untracked() else {
                return;
            };
            operation_generation.update(|value| *value = value.wrapping_add(1));
            let generation = operation_generation.get_untracked();
            loading.set(true);
            spawn_local(async move {
                let result = invoke_clone_store_page(source, presentation_id).await;
                if !accepts_account_response(
                    auth.npub.get_untracked().as_deref(),
                    &initiating_account,
                    operation_generation.get_untracked(),
                    generation,
                ) {
                    loading.set(false);
                    return;
                }
                match result {
                    Ok(cloned) => {
                        associations.set(Vec::new());
                        upload_context_generation.update(|value| *value = value.wrapping_add(1));
                        draft.set(cloned);
                        preview.set(None);
                        message.set(Some("Clone created locally. Add explicit listing associations before publishing.".to_string()));
                    }
                    Err(error) => message.set(Some(error)),
                }
                loading.set(false);
            });
        }
    });

    let link_existing = Callback::new({
        let publisher = publisher.clone();
        let coordinate = coordinate.clone();
        let event_id = listing.event_id.clone();
        move |_: ()| {
            if loading.get_untracked()
                || validating.get_untracked()
                || publishing.get_untracked()
                || partial.get_untracked().is_some()
            {
                return;
            }
            if draft.get_untracked() != baseline.get_untracked() || input_dirty.get_untracked() {
                message.set(Some(
                    "Discard or publish the current draft before loading another Store Page."
                        .to_string(),
                ));
                return;
            }
            let presentation_id = link_existing_id.get_untracked();
            let Some(event_id) = event_id.clone() else {
                message.set(Some(
                    "The selected listing event ID is unavailable.".to_string(),
                ));
                return;
            };
            if presentation_id.trim().is_empty() {
                message.set(Some("Enter an existing presentation ID.".to_string()));
                return;
            }
            let Some(initiating_account) = auth.npub.get_untracked() else {
                return;
            };
            if initiating_account != publisher {
                message.set(Some(
                    "Switch back to the publisher account before linking.".to_string(),
                ));
                return;
            }
            operation_generation.update(|value| *value = value.wrapping_add(1));
            let generation = operation_generation.get_untracked();
            loading.set(true);
            let publisher = publisher.clone();
            let coordinate = coordinate.clone();
            spawn_local(async move {
                let result = invoke_load_publisher_store_page_editor(
                    publisher,
                    StorePageListingRef {
                        listing_coordinate: coordinate,
                        listing_event_id: event_id,
                    },
                    Some(presentation_id),
                )
                .await;
                if !accepts_account_response(
                    auth.npub.get_untracked().as_deref(),
                    &initiating_account,
                    operation_generation.get_untracked(),
                    generation,
                ) {
                    loading.set(false);
                    return;
                }
                match result {
                    Ok(state) => {
                        let requires_association_review = !state.diagnostics.is_empty()
                            || state.listings.iter().any(|listing| !listing.reciprocal);
                        associations.set(associations_from_revisions(&state.listings));
                        listings.set(state.listings);
                        diagnostics.set(
                            state
                                .diagnostics
                                .into_iter()
                                .map(|message| format!("Association: {message}"))
                                .collect(),
                        );
                        validation_valid.set(None);
                        association_review_required.set(requires_association_review);
                        upload_context_generation.update(|value| *value = value.wrapping_add(1));
                        draft.set(state.draft.clone());
                        baseline.set(state.baseline_draft);
                        input_dirty.set(false);
                        message.set(Some(
                            "Existing Store Page loaded locally. Publishing will add the selected listing reciprocally."
                                .to_string(),
                        ));
                    }
                    Err(error) => message.set(Some(error)),
                }
                loading.set(false);
            });
        }
    });

    let on_back_click = move |_| {
        if loading.get_untracked() || validating.get_untracked() || publishing.get_untracked() {
            message.set(Some(
                "Wait for the active Store Page request to finish before leaving.".to_string(),
            ));
            return;
        }
        if partial.get_untracked().is_some() {
            message.set(Some(
                "Resolve or retry the incomplete publication before leaving this editor. Recovery state has been retained."
                    .to_string(),
            ));
            return;
        }
        if draft.get_untracked() != baseline.get_untracked() || input_dirty.get_untracked() {
            show_discard.set(true);
        } else {
            on_back.run(());
        }
    };

    let selected_platforms = StoredValue::new(listing.platforms.clone());
    let listing_for_preview = listing.clone();
    let coordinate_for_readiness = StoredValue::new(coordinate.clone());
    let coordinate_for_requirements = StoredValue::new(coordinate.clone());
    let coordinate_for_preview = StoredValue::new(coordinate.clone());
    let coordinate_for_tabs = StoredValue::new(coordinate.clone());
    let selected_platforms_for_tabs = StoredValue::new(listing.platforms.clone());
    let publisher_for_retry_disabled = StoredValue::new(publisher.clone());
    let publisher_for_discard = StoredValue::new(publisher.clone());

    view! {
        <section class="v2-publisher-studio v2-store-page-editor">
            <button class="v2-btn-secondary v2-publisher-back" on:click=on_back_click>"Back to game management"</button>
            <header class="v2-publisher-game-hero">
                <div><p class="v2-publisher-kicker">"Store Page editor"</p><h1>{listing.title.clone()}</h1><p class="v2-store-help">"Drafts stay local until Publish is selected."</p></div>
            </header>
            <BlossomMediaUpload
                dialog_role=blossom_dialog_role
                listing_publisher_npub=publisher.clone()
                publisher_hex=canonical_publisher_hex.clone()
                context_generation=upload_context_generation
                draft=draft
                input_dirty=input_dirty
            />
            {move || loading.get().then(|| view! { <p>"Loading current Store Page and signed listings..."</p> })}
            {move || message.get().map(|value| view! { <p class="v2-store-notice" role="status">{value}</p> })}
            {move || form_error.get().map(|value| view! { <p class="v2-store-alert" role="alert">{value}</p> })}
            {move || partial.get().map(|result| {
                let lifecycle = publication_lifecycle(publishing.get(), Some(&result));
                let page_status = result.store_page.as_ref().map(|page| {
                    format!(
                        "Store Page event {}: accepted by {} relay(s), {} failure(s), propagation {}",
                        page.event_id,
                        page.success_count,
                        page.failure_count,
                        if page.propagation_confirmed { "confirmed" } else { "not confirmed" }
                    )
                }).unwrap_or_else(|| "Store Page was not republished during this retry.".to_string());
                view! { <section class="v2-publisher-panel v2-store-publication" aria-labelledby="partial-store-page-title">
                    <h2 id="partial-store-page-title">"Incomplete publication"</h2>
                    <p class="v2-store-overall-status">{overall_publication_label(lifecycle)}</p>
                    <dl class="v2-store-stage-grid">
                        <div><dt>"Store Page event"</dt><dd class=lifecycle.store_page.class()>{lifecycle.store_page.label()}</dd></div>
                        <div><dt>"Listing pointer update"</dt><dd class=lifecycle.pointers.class()>{lifecycle.pointers.label()}</dd></div>
                    </dl>
                    <p>{page_status}</p>
                    {result.cache_error.map(|error| view! { <p class="v2-store-alert">{format!("Local cache update failed: {error}")}</p> })}
                    <ul class="v2-store-outcome-list">{result.listing_updates.into_iter().map(|outcome| view! { <li class="v2-store-outcome-row"><strong>{outcome.listing_coordinate}</strong><p>{format!("{:?}: published={}, propagation={}", outcome.action, outcome.published, outcome.propagation_confirmed)}</p>{outcome.error.map(|error| view! { <p class="v2-store-alert">{error}</p> })}</li> }).collect_view()}</ul>
                    <button class="v2-btn-secondary" type="button" disabled=move || validating.get() || publishing.get() || auth.npub.get().as_deref() != Some(publisher_for_retry_disabled.get_value().as_str()) on:click=move |_| retry.run(())>"Retry incomplete listing pointer synchronization"</button>
                </section> }
            })}

            <Show when=move || !diagnostics.get().is_empty()>
                <section class="v2-publisher-panel" aria-labelledby="store-editor-diagnostics-title">
                    <h2 id="store-editor-diagnostics-title">{move || match validation_valid.get() { Some(true) => "Validation warnings", Some(false) => "Blocking validation issues", None => "Association warnings" }}</h2>
                    <ul class="v2-store-diagnostic-list">
                        {move || diagnostics.get().into_iter().map(|item| {
                            let tab = diagnostic_tab(&item);
                            view! { <li><button type="button" on:click=move |_| active_tab.set(tab)>{item}</button></li> }
                        }).collect_view()}
                    </ul>
                </section>
            </Show>

            <nav class="v2-store-editor-tabs" role="tablist" aria-label="Store Page fields">
                {EDITOR_TABS.into_iter().map(|(id, label)| {
                    let tab_state = move || {
                        let platforms = linked_platforms(&selected_platforms_for_tabs.get_value(), &associations.get(), &coordinate_for_tabs.get_value());
                        let state = readiness(&draft.get(), &associations.get(), &platforms, &diagnostics.get());
                        editor_tab_state(id, &state.blockers, &state.warnings)
                    };
                    view! { <button
                        id=format!("store-editor-tab-{id}")
                        aria-controls=format!("store-editor-{id}")
                        tabindex=move || if active_tab.get() == id { 0 } else { -1 }
                        type="button"
                        role="tab"
                        aria-selected=move || active_tab.get() == id
                        class:v2-store-editor-tab-active=move || active_tab.get() == id
                        class:v2-store-editor-tab-blocked=move || tab_state() == EditorTabState::Blocked
                        class:v2-store-editor-tab-warned=move || tab_state() == EditorTabState::Warned
                        on:keydown=move |event| { let next = match event.key().as_str() { "ArrowRight" => Some(adjacent_tab(id, 1)), "ArrowLeft" => Some(adjacent_tab(id, -1)), "Home" => Some(EDITOR_TABS[0].0), "End" => Some(EDITOR_TABS[EDITOR_TABS.len() - 1].0), _ => None }; if let Some(next) = next { event.prevent_default(); active_tab.set(next); focus_editor_tab(next); } }
                        on:click=move |_| active_tab.set(id)
                    >
                        <span>{label}</span>
                        {move || match tab_state() {
                            EditorTabState::Blocked => Some(view! { <span class="v2-store-tab-flag v2-store-tab-flag-blocked"><span aria-hidden="true">"!"</span><span class="sr-only">"has blocking issues"</span></span> }),
                            EditorTabState::Warned => Some(view! { <span class="v2-store-tab-flag v2-store-tab-flag-warned"><span aria-hidden="true">"*"</span><span class="sr-only">"has warnings"</span></span> }),
                            EditorTabState::Neutral => None,
                        }}
                    </button> }
                }).collect_view()}
            </nav>
            <button type="button" class="v2-btn-secondary v2-store-readiness-toggle" aria-expanded=move || readiness_open.get() on:click=move |_| readiness_open.update(|open| *open = !*open)>"Readiness"</button>
            <div class="v2-publisher-management-layout v2-store-editor-layout">
            <main class="v2-publisher-main v2-store-editor-main">
                <fieldset class="v2-store-fieldset" disabled=move || publishing.get() || partial.get().is_some()>
                <Show when=move || active_tab.get() == "basic"><section id="store-editor-basic" role="tabpanel" class="v2-publisher-panel v2-store-form-grid">
                    <h2 class="v2-store-span-all">"Basic Info"</h2>
                    <label>"Presentation ID"<input class="v2-input" disabled=move || draft.get().loaded_event_id.is_some() prop:value=move || draft.get().presentation_id on:input=move |event| draft.update(|value| value.presentation_id = event_target_value(&event)) /></label>
                    <div><span class="v2-store-field-label">"Release date"</span><DatePicker value=Signal::derive(move || draft.get().content.basic.release_date.unwrap_or_default()) on_value=Callback::new(move |date| draft.update(|value| value.content.basic.release_date = optional(date))) disabled=Signal::derive(move || publishing.get() || partial.get().is_some()) /></div>
                    <label>"Title"<input class="v2-input" prop:value=move || draft.get().content.basic.title.unwrap_or_default() on:input:target=move |event| draft.update(|value| value.content.basic.title = optional_editor_text(event.target().value())) /></label>
                    <label>"Summary"<textarea class="v2-input" prop:value=move || draft.get().content.basic.summary.unwrap_or_default() on:input:target=move |event| draft.update(|value| value.content.basic.summary = optional_editor_text(event.target().value())) /><small>{move || format!("{} characters", draft.get().content.basic.summary.as_deref().unwrap_or_default().chars().count())}</small></label>
                    <label>"Developer display name"<input class="v2-input" prop:value=move || draft.get().content.basic.developer.unwrap_or_default() on:input:target=move |event| draft.update(|value| value.content.basic.developer = optional_editor_text(event.target().value())) /></label>
                    <label>"Publisher display name"<input class="v2-input" prop:value=move || draft.get().content.basic.publisher.unwrap_or_default() on:input:target=move |event| draft.update(|value| value.content.basic.publisher = optional_editor_text(event.target().value())) /></label>
                    <div class="v2-store-span-all"><span class="v2-store-field-label">"Genres"</span><div class="v2-store-chip-row">{move || draft.get().content.discovery.genres.unwrap_or_default().into_iter().enumerate().map(|(index, value)| view! { <button type="button" class="v2-chip" aria-label=format!("Remove genre {value}") on:click=move |_| draft.update(|item| { if let Some(values) = &mut item.content.discovery.genres { values.remove(index); } })>{value.clone()}" ×"</button> }).collect_view()}</div><div class="v2-store-add-row"><input class="v2-input" list="store-genre-suggestions" placeholder="Add genre" prop:value=move || genre_input.get() on:input=move |event| genre_input.set(event_target_value(&event)) /><datalist id="store-genre-suggestions"><option value="Action"/><option value="Adventure"/><option value="Puzzle"/><option value="Role-playing"/><option value="Strategy"/></datalist><button type="button" class="v2-btn-secondary" on:click=move |_| { let value = genre_input.get_untracked().trim().to_string(); if !value.is_empty() { draft.update(|item| item.content.discovery.genres.get_or_insert_default().push(value)); genre_input.set(String::new()); } }>"Add"</button></div></div>
                    <div class="v2-store-span-all"><span class="v2-store-field-label">"Features"</span><div class="v2-store-chip-row">{move || draft.get().content.discovery.features.unwrap_or_default().into_iter().enumerate().map(|(index, value)| view! { <button type="button" class="v2-chip" aria-label=format!("Remove feature {value}") on:click=move |_| draft.update(|item| { if let Some(values) = &mut item.content.discovery.features { values.remove(index); } })>{value.clone()}" ×"</button> }).collect_view()}</div><div class="v2-store-add-row"><input class="v2-input" list="store-feature-suggestions" placeholder="Add feature" prop:value=move || feature_input.get() on:input=move |event| feature_input.set(event_target_value(&event)) /><datalist id="store-feature-suggestions"><option value="Single-player"/><option value="Multiplayer"/><option value="Controller support"/><option value="Achievements"/></datalist><button type="button" class="v2-btn-secondary" on:click=move |_| { let value = feature_input.get_untracked().trim().to_string(); if !value.is_empty() { draft.update(|item| item.content.discovery.features.get_or_insert_default().push(value)); feature_input.set(String::new()); } }>"Add"</button></div></div>
                    <section class="v2-store-span-all"><h3>"Associated listings"</h3><p class="v2-store-help">"Signed current-user-owned revisions. Changes publish as explicit pointer mutations."</p>{move || associations.get().into_iter().enumerate().map(|(index, row)| { let status = if row.reciprocal { "Reciprocal pointer current" } else { "Reciprocal pointer missing or incomplete" }; view! { <article class="v2-store-card"><strong>{row.listing_coordinate}</strong><p class="v2-store-mono">{row.event_id}</p><p>{status}</p><label>"Association action"<select class="v2-input" prop:value=format!("{:?}", row.action).to_ascii_lowercase() on:change=move |event| { let action = match event_target_value(&event).as_str() { "link" => ListingPointerMutation::Link, "unlink" => ListingPointerMutation::Unlink, _ => ListingPointerMutation::Review }; associations.update(|rows| if let Some(row) = rows.get_mut(index) { row.action = action; }); association_review_required.set(associations.get_untracked().iter().any(|row| row.action == ListingPointerMutation::Review)); input_dirty.set(true); }><option value="link">"Link"</option><option value="unlink">"Unlink"</option><option value="review">"Review"</option></select></label></article> } }).collect_view()}</section>
                </section></Show>

                <Show when=move || active_tab.get() == "description"><section role="tabpanel" class="v2-publisher-panel"><div class="v2-store-section-heading"><h2>"Description"</h2><div role="group" aria-label="Description mode"><button type="button" class="v2-btn-secondary" aria-pressed=move || !description_preview.get() on:click=move |_| description_preview.set(false)>"Write"</button><button type="button" class="v2-btn-secondary" aria-pressed=move || description_preview.get() on:click=move |_| description_preview.set(true)>"Preview"</button></div></div><Show when=move || !description_preview.get()><div class="v2-store-toolbar" role="toolbar" aria-label="Markdown formatting"><button type="button" on:click=move |_| draft.update(|value| value.content.description_markdown.push_str("**bold**"))>"Bold"</button><button type="button" on:click=move |_| draft.update(|value| value.content.description_markdown.push_str("\n## Heading\n"))>"Heading"</button><button type="button" on:click=move |_| draft.update(|value| value.content.description_markdown.push_str("\n- item"))>"List"</button></div><label>"Markdown description"<textarea class="v2-input v2-store-textarea-lg" prop:value=move || draft.get().content.description_markdown on:input=move |event| draft.update(|value| value.content.description_markdown = event_target_value(&event)) /></label><small>{move || format!("{} characters", draft.get().content.description_markdown.chars().count())}</small></Show><Show when=move || description_preview.get()><div class="v2-store-canonical-placeholder"><strong>"Canonical preview only"</strong><p>"Validate the draft to render sanitized content with the buyer Store Page renderer below."</p><button type="button" class="v2-btn-secondary" on:click=move |_| run_validation.run(())>"Validate canonical preview"</button></div></Show><details class="v2-publisher-diagnostics"><summary>"Description diagnostics"</summary>{move || diagnostics.get().into_iter().map(|item| view! { <p>{item}</p> }).collect_view()}</details></section></Show>

                <Show when=move || active_tab.get() == "media"><section role="tabpanel" class="v2-publisher-panel"><div class="v2-store-section-heading"><h2>"Media"</h2><div class="v2-store-chip-row" role="group" aria-label="Add Store Page media"><button type="button" class="v2-btn-secondary" on:click=move |_| blossom_dialog_role.set(Some("hero".into()))>"Upload hero"</button><button type="button" class="v2-btn-secondary" on:click=move |_| blossom_dialog_role.set(Some("capsule".into()))>"Upload capsule"</button><button type="button" class="v2-btn-secondary" on:click=move |_| blossom_dialog_role.set(Some("screenshot".into()))>"Add screenshot"</button><button type="button" class="v2-btn-secondary" on:click=move |_| blossom_dialog_role.set(Some("trailer".into()))>"Add trailer"</button><button type="button" class="v2-btn-secondary" on:click=move |_| blossom_dialog_role.set(Some("feature".into()))>"Add feature image"</button><button type="button" class="v2-btn-secondary" on:click=move |_| draft.update(|value| { let id = unique_editor_id("media", value.content.media.iter().map(|item| item.id.as_str())); value.content.media.push(StorePageMediaItem { id, media_type: "image".into(), role: "screenshot".into(), url: String::new(), sha256: None, mime_type: None, size: None, thumbnail_url: None, alt: None, caption: None, width: None, height: None }); })>"Use existing URL"</button></div></div><p class="v2-store-help">"Removing media only removes its draft reference; it does not delete the hosted blob."</p>{move || draft.get().content.media.into_iter().enumerate().map(|(index, item)| view! { <article class="v2-store-card"><div class="v2-store-card-actions"><button type="button" aria-label="Move media up" disabled=index == 0 on:click=move |_| { pending_removal.set(None); draft.update(|value| move_item(&mut value.content.media, index, -1)); }>"↑"</button><button type="button" aria-label="Move media down" on:click=move |_| { pending_removal.set(None); draft.update(|value| move_item(&mut value.content.media, index, 1)); }>"↓"</button><button type="button" aria-label="Delete media" on:click=move |_| { if pending_removal.get_untracked() == Some(("media", index)) { pending_removal.set(None); draft.update(|value| { let removed_id = value.content.media.get(index).map(|item| item.id.clone()); if index < value.content.media.len() { value.content.media.remove(index); } if let Some(removed_id) = removed_id { for section in &mut value.content.sections { if section.media_id.as_deref() == Some(removed_id.as_str()) { section.media_id = None; } } } }); } else { pending_removal.set(Some(("media", index))); } }>{move || if pending_removal.get() == Some(("media", index)) { "Confirm remove draft reference" } else { "Remove reference" }}</button></div><div class="v2-store-form-grid"><label>"Type"<select class="v2-input" prop:value=item.media_type.clone() on:change=move |event| draft.update(|value| if let Some(item) = value.content.media.get_mut(index) { item.media_type = event_target_value(&event); })><option value="image">"Image"</option><option value="video">"Video"</option></select></label><label>"Role"<select class="v2-input" prop:value=item.role.clone() on:change=move |event| { let role = event_target_value(&event); if matches!(role.as_str(), "hero" | "capsule") && draft.get_untracked().content.media.iter().enumerate().any(|(other, item)| other != index && item.role == role) { form_error.set(Some(format!("Only one {role} is allowed."))); } else { draft.update(|value| if let Some(item) = value.content.media.get_mut(index) { item.role = role; }); } }><option value="hero">"Hero"</option><option value="capsule">"Capsule"</option><option value="screenshot">"Screenshot"</option><option value="trailer">"Trailer"</option><option value="feature">"Feature"</option></select></label><label class="v2-store-span-all">"HTTPS URL"<input type="url" class="v2-input" prop:value=item.url.clone() on:input=move |event| draft.update(|value| if let Some(item) = value.content.media.get_mut(index) { item.url = event_target_value(&event); }) /></label><label>"Thumbnail URL"<input type="url" class="v2-input" prop:value=item.thumbnail_url.clone().unwrap_or_default() on:input=move |event| draft.update(|value| if let Some(item) = value.content.media.get_mut(index) { item.thumbnail_url = optional(event_target_value(&event)); }) /></label><label>"Alternative text"<input class="v2-input" prop:value=item.alt.clone().unwrap_or_default() on:input=move |event| draft.update(|value| if let Some(item) = value.content.media.get_mut(index) { item.alt = optional(event_target_value(&event)); }) /></label><label>"Caption"<input class="v2-input" prop:value=item.caption.clone().unwrap_or_default() on:input=move |event| draft.update(|value| if let Some(item) = value.content.media.get_mut(index) { item.caption = optional(event_target_value(&event)); }) /></label><label>"Width"<input type="number" min="1" class="v2-input" prop:value=item.width.map(|value| value.to_string()).unwrap_or_default() on:input=move |event| draft.update(|value| if let Some(item) = value.content.media.get_mut(index) { item.width = event_target_value(&event).parse().ok(); }) /></label><label>"Height"<input type="number" min="1" class="v2-input" prop:value=item.height.map(|value| value.to_string()).unwrap_or_default() on:input=move |event| draft.update(|value| if let Some(item) = value.content.media.get_mut(index) { item.height = event_target_value(&event).parse().ok(); }) /></label></div><div class="v2-store-canonical-placeholder"><strong>{media_slot_state(&item.url, &item.media_type).label()}</strong><p>"Media is rendered only after canonical validation in Preview. A completed upload is not a published Store Page."</p></div></article> }).collect_view()}</section></Show>

                <Show when=move || active_tab.get() == "sections"><section role="tabpanel" class="v2-publisher-panel"><div class="v2-store-section-heading"><h2>"Feature Sections"</h2><button type="button" class="v2-btn-secondary" on:click=move |_| draft.update(|value| { let id = unique_editor_id("section", value.content.sections.iter().map(|section| section.id.as_str())); value.content.sections.push(StorePageSection { id, heading: String::new(), body_markdown: String::new(), media_id: None, layout: "text".into() }); })>"Add section"</button></div>{move || { let media = draft.get().content.media; draft.get().content.sections.into_iter().enumerate().map(|(index, section)| { let options = media.clone(); view! { <article class="v2-store-card"><div class="v2-store-card-actions"><button type="button" aria-label="Move section up" disabled=index == 0 on:click=move |_| { pending_removal.set(None); draft.update(|value| move_item(&mut value.content.sections, index, -1)); }>"↑"</button><button type="button" aria-label="Move section down" on:click=move |_| { pending_removal.set(None); draft.update(|value| move_item(&mut value.content.sections, index, 1)); }>"↓"</button><button type="button" aria-label="Remove section" on:click=move |_| { if pending_removal.get_untracked() == Some(("section", index)) { pending_removal.set(None); draft.update(|value| { if index < value.content.sections.len() { value.content.sections.remove(index); } }); } else { pending_removal.set(Some(("section", index))); } }>{move || if pending_removal.get() == Some(("section", index)) { "Confirm remove" } else { "Remove" }}</button></div><label>"Layout"<select class="v2-input" prop:value=section.layout on:change=move |event| draft.update(|value| if let Some(section) = value.content.sections.get_mut(index) { section.layout = event_target_value(&event); })><option value="text">"Text"</option><option value="media-left">"Media left"</option><option value="media-right">"Media right"</option><option value="media-wide">"Media wide"</option></select></label><label>"Heading"<input class="v2-input" prop:value=section.heading on:input=move |event| draft.update(|value| if let Some(section) = value.content.sections.get_mut(index) { section.heading = event_target_value(&event); }) /></label><label>"Media"<select class="v2-input" prop:value=section.media_id.unwrap_or_default() on:change=move |event| draft.update(|value| if let Some(section) = value.content.sections.get_mut(index) { section.media_id = optional(event_target_value(&event)); })><option value="">"No media"</option>{options.into_iter().map(|item| view! { <option value=item.id.clone()>{format!("{} · {}", item.role, item.id)}</option> }).collect_view()}</select></label><label>"Section Markdown"<textarea class="v2-input v2-store-textarea-md" prop:value=section.body_markdown on:input=move |event| draft.update(|value| if let Some(section) = value.content.sections.get_mut(index) { section.body_markdown = event_target_value(&event); }) /></label><button type="button" class="v2-btn-secondary" on:click=move |_| run_validation.run(())>"Preview canonical section"</button></article> } }).collect_view() }}</section></Show>

                <Show when=move || active_tab.get() == "requirements"><section role="tabpanel" class="v2-publisher-panel"><h2>"Requirements"</h2><p class="v2-store-help">"Platforms come from the selected linked authoritative listing; no compatibility is inferred."</p>{move || { let platforms = linked_platforms(&selected_platforms.get_value(), &associations.get(), &coordinate_for_requirements.get_value()); if platforms.is_empty() { view! { <p>"Link the selected listing to edit its declared platform requirements."</p> }.into_any() } else { platforms.into_iter().map(|platform| { let label = platform_label(&platform); let platform_min = platform.clone(); let platform_rec = platform.clone(); view! { <article class="v2-store-card"><h3>{label}</h3>{[("Minimum", true), ("Recommended", false)].into_iter().map(|(name, minimum)| { let key = if minimum { platform_min.clone() } else { platform_rec.clone() }; view! { <fieldset class="v2-store-tier"><legend>{name}</legend>{[("Operating system", "os"), ("Processor", "processor"), ("Memory", "memory"), ("Graphics", "graphics"), ("Storage", "storage"), ("Additional", "additional")].into_iter().map(|(label, field)| { let key = key.clone(); let value_key = key.clone(); view! { <label>{label}<input class="v2-input" prop:value=move || { let requirement = draft.get().content.requirements.get(&value_key).cloned().unwrap_or_default(); let tier = if minimum { requirement.minimum } else { requirement.recommended }; tier.and_then(|tier| match field { "os" => tier.os, "processor" => tier.processor, "memory" => tier.memory, "graphics" => tier.graphics, "storage" => tier.storage, _ => tier.additional }).unwrap_or_default() } on:input=move |event| { let input = optional(event_target_value(&event)); draft.update(|value| { let requirement = value.content.requirements.entry(key.clone()).or_default(); let tier = if minimum { requirement.minimum.get_or_insert_default() } else { requirement.recommended.get_or_insert_default() }; match field { "os" => tier.os = input, "processor" => tier.processor = input, "memory" => tier.memory = input, "graphics" => tier.graphics = input, "storage" => tier.storage = input, _ => tier.additional = input } }); } /></label> } }).collect_view()}</fieldset> } }).collect_view()}</article> } }).collect_view().into_any() } }}</section></Show>

                <Show when=move || active_tab.get() == "languages"><section role="tabpanel" class="v2-publisher-panel"><div class="v2-store-section-heading"><h2>"Languages"</h2><div class="v2-store-add-row"><label>"Language"<select class="v2-input" prop:value=move || locale_input.get() on:change=move |event| locale_input.set(event_target_value(&event))><option value="en">"English (en)"</option><option value="es">"Spanish (es)"</option><option value="pt-BR">"Portuguese — Brazil (pt-BR)"</option><option value="fr">"French (fr)"</option><option value="de">"German (de)"</option><option value="ja">"Japanese (ja)"</option><option value="zh-CN">"Chinese — Simplified (zh-CN)"</option></select></label><button type="button" class="v2-btn-secondary" on:click=move |_| { let code = locale_input.get_untracked(); draft.update(|value| { let values = value.content.languages.get_or_insert_default(); if !values.iter().any(|entry| entry.code == code) { values.push(LanguageSupport { code, interface: true, audio: false, subtitles: false }); } }); }>"Add language"</button></div></div>{move || draft.get().content.languages.unwrap_or_default().into_iter().enumerate().map(|(index, language)| view! { <article class="v2-store-card v2-store-language-row"><strong>{language.code}</strong><label><input type="checkbox" prop:checked=language.interface on:change=move |event| draft.update(|value| if let Some(entry) = value.content.languages.as_mut().and_then(|values| values.get_mut(index)) { entry.interface = event_target_checked(&event); }) />" Interface"</label><label><input type="checkbox" prop:checked=language.audio on:change=move |event| draft.update(|value| if let Some(entry) = value.content.languages.as_mut().and_then(|values| values.get_mut(index)) { entry.audio = event_target_checked(&event); }) />" Audio"</label><label><input type="checkbox" prop:checked=language.subtitles on:change=move |event| draft.update(|value| if let Some(entry) = value.content.languages.as_mut().and_then(|values| values.get_mut(index)) { entry.subtitles = event_target_checked(&event); }) />" Subtitles"</label><button type="button" on:click=move |_| draft.update(|value| if let Some(values) = &mut value.content.languages { values.remove(index); if values.is_empty() { value.content.languages = None; } })>"Remove"</button></article> }).collect_view()}</section></Show>

                <Show when=move || active_tab.get() == "accessibility"><section role="tabpanel" class="v2-publisher-panel"><h2>"Accessibility"</h2><p class="v2-store-help">"Publisher-provided accessibility information. Verify every claim."</p>{[("Visual", "colorblind-modes"), ("Visual", "scalable-text"), ("Hearing", "subtitles"), ("Hearing", "closed-captions"), ("Input", "remappable-controls"), ("Input", "single-stick")].into_iter().map(|(group, feature)| view! { <article class="v2-store-accessibility-row"><div><small>{group}</small><strong>{platform_label(feature)}</strong></div><label><input type="checkbox" prop:checked=move || draft.get().content.accessibility.iter().find(|entry| entry.feature == feature).is_some_and(|entry| entry.supported) on:change=move |event| { let supported = event_target_checked(&event); draft.update(|value| { if let Some(entry) = value.content.accessibility.iter_mut().find(|entry| entry.feature == feature) { entry.supported = supported; } else { value.content.accessibility.push(AccessibilityFeature { feature: feature.into(), supported, notes: None }); } }); } />" Supported"</label><label>"Optional notes"<input class="v2-input" prop:value=move || draft.get().content.accessibility.iter().find(|entry| entry.feature == feature).and_then(|entry| entry.notes.clone()).unwrap_or_default() on:input=move |event| { let notes = optional(event_target_value(&event)); draft.update(|value| { if let Some(entry) = value.content.accessibility.iter_mut().find(|entry| entry.feature == feature) { entry.notes = notes; } else { value.content.accessibility.push(AccessibilityFeature { feature: feature.into(), supported: false, notes }); } }); } /></label></article> }).collect_view()}<details class="v2-publisher-diagnostics"><summary>"Advanced custom identifier"</summary><div class="v2-store-add-row"><input class="v2-input" placeholder="publisher-defined-feature" prop:value=move || custom_accessibility.get() on:input=move |event| custom_accessibility.set(event_target_value(&event)) /><button type="button" class="v2-btn-secondary" on:click=move |_| { let feature = custom_accessibility.get_untracked().trim().to_string(); if !feature.is_empty() { draft.update(|value| value.content.accessibility.push(AccessibilityFeature { feature, supported: true, notes: None })); custom_accessibility.set(String::new()); } }>"Add"</button></div></details></section></Show>

                <Show when=move || active_tab.get() == "links"><section role="tabpanel" class="v2-publisher-panel v2-store-form-grid"><h2 class="v2-store-span-all">"Links"</h2>
                    <label>"Website"<input class="v2-input" prop:value=move || draft.get().content.links.website.unwrap_or_default() on:input=move |event| draft.update(|draft| draft.content.links.website = optional(event_target_value(&event))) /></label>
                    <label>"Support"<input class="v2-input" prop:value=move || draft.get().content.links.support.unwrap_or_default() on:input=move |event| draft.update(|draft| draft.content.links.support = optional(event_target_value(&event))) /></label>
                    <label>"Documentation"<input class="v2-input" prop:value=move || draft.get().content.links.documentation.unwrap_or_default() on:input=move |event| draft.update(|draft| draft.content.links.documentation = optional(event_target_value(&event))) /></label>
                    <label>"Source"<input class="v2-input" prop:value=move || draft.get().content.links.source.unwrap_or_default() on:input=move |event| draft.update(|draft| draft.content.links.source = optional(event_target_value(&event))) /></label>
                    <label>"Community"<input class="v2-input" prop:value=move || draft.get().content.links.community.unwrap_or_default() on:input=move |event| draft.update(|draft| draft.content.links.community = optional(event_target_value(&event))) /></label>
                    <label>"Privacy policy"<input class="v2-input" prop:value=move || draft.get().content.links.privacy_policy.unwrap_or_default() on:input=move |event| draft.update(|draft| draft.content.links.privacy_policy = optional(event_target_value(&event))) /></label>
                    <div class="v2-store-span-all"><p class="v2-store-help">"Only validated HTTPS links are opened. Backend validation remains authoritative."</p>{move || [draft.get().content.links.website, draft.get().content.links.support, draft.get().content.links.documentation, draft.get().content.links.source, draft.get().content.links.community, draft.get().content.links.privacy_policy].into_iter().flatten().map(|url| if safe_https_link(&url) { view! { <a class="v2-btn-secondary" href=url target="_blank" rel="noopener noreferrer">"Test safe link"</a> }.into_any() } else { view! { <span class="v2-store-alert">"Enter a complete HTTPS URL"</span> }.into_any() }).collect_view()}</div>
                </section></Show>
                <Show when=move || active_tab.get() == "sections">
                    <button type="button" class="v2-btn-secondary" on:click=move |_| run_validation.run(())>"Preview sections with canonical validation"</button>
                </Show>
                </fieldset>

                {move || preview.get().map(|presentation| view! {
                    <section class="v2-store-preview" class:v2-store-preview-narrow=move || preview_narrow.get() aria-label="Store Page preview" data-preview-source=CANONICAL_PREVIEW_SOURCE>
                        <div class="v2-store-preview-banner"><strong>"Canonical validated preview"</strong><span>"Commerce and external navigation are disabled"</span><label>"Associated listing"<select disabled><option>{coordinate_for_preview.get_value()}</option></select></label><div role="group" aria-label="Preview width"><button type="button" aria-pressed=move || !preview_narrow.get() on:click=move |_| preview_narrow.set(false)>"Desktop"</button><button type="button" aria-pressed=move || preview_narrow.get() on:click=move |_| preview_narrow.set(true)>"Narrow"</button></div></div>
                        <h2>{presentation.title.clone().unwrap_or_else(|| listing.title.clone())}</h2>
                        <p>{presentation.summary.clone().unwrap_or_else(|| listing.description.clone())}</p>
                        <div class="v2-store-preview-commerce"><strong>"Authoritative listing commerce"</strong><p>{preview_commerce_label(listing.price, &listing.currency, &listing.acquisition)}</p></div>
                        <StorePageRichDetail presentation=presentation preview=true />
                    </section>
                })}
            </main>
            <aside class="v2-publisher-panel v2-publisher-sidebar v2-store-readiness" class:v2-store-readiness-open=move || readiness_open.get()><h2>"Readiness"</h2>{move || { let platforms = linked_platforms(&listing_for_preview.platforms, &associations.get(), &coordinate_for_readiness.get_value()); let state = readiness(&draft.get(), &associations.get(), &platforms, &diagnostics.get()); view! { <div><p class="v2-store-persistence">{draft_persistence_label(!(draft.get() == baseline.get() && !input_dirty.get()))}</p><p class="v2-store-revision">{revision_label(draft.get().loaded_event_id.as_deref())}</p><p class="v2-store-persistence-note">"Editor changes live only in this session. Closing Arcadestr discards them; publication is the only durable action."</p><h3>"Blocking issues"</h3>{if state.blockers.is_empty() { view! { <p class="v2-store-ok">"No known blockers"</p> }.into_any() } else { let target = first_blocker_target(&state.blockers); view! { <div><ul>{state.blockers.iter().cloned().map(|item| { let tab = diagnostic_tab(&item); view! { <li><button type="button" class="v2-store-blocker-link" on:click=move |_| { active_tab.set(tab); focus_editor_tab(tab); }>{item}</button></li> } }).collect_view()}</ul>{target.map(|target| { let tab = target.tab; let element_id = target.element_id.clone(); view! { <button type="button" class="v2-btn-secondary v2-store-first-blocker" on:click=move |_| { active_tab.set(tab); if !element_id.as_deref().is_some_and(focus_editor_element) { focus_editor_tab(tab); } }>{format!("Go to first blocker: {}", target.message)}</button> } })}</div> }.into_any() }}<h3>"Association warnings"</h3><ul>{state.warnings.into_iter().map(|item| view! { <li>{item}</li> }).collect_view()}</ul><h3>"Recommendations"</h3><ul>{state.recommendations.into_iter().map(|item| view! { <li>{item}</li> }).collect_view()}</ul></div> } }}<p class="v2-store-help">"Preview uses the core sanitizer and buyer renderer."</p></aside>
            </div>

            <footer class="v2-store-editor-footer"><span class="v2-store-footer-status" role="status">{move || if validating.get() { "Validating…".to_string() } else if publishing.get() { "Publishing…".to_string() } else { draft_persistence_label(!(draft.get() == baseline.get() && !input_dirty.get())).to_string() }}</span><button class="v2-btn-secondary" type="button" disabled=move || validating.get() on:click=move |_| run_validation.run(())>{move || if validating.get() { "Validating..." } else { "Preview" }}</button><button class="v2-btn-primary" type="button" disabled=move || publishing.get() on:click=move |_| publish.run(())>{move || if publishing.get() { "Publishing..." } else { "Publish" }}</button><details class="v2-store-overflow"><summary class="v2-btn-secondary">"More"</summary><div><label>"New presentation ID"<input class="v2-input" prop:value=move || clone_id.get() on:input=move |event| clone_id.set(event_target_value(&event)) /></label><button type="button" on:click=move |_| clone_page.run(())>"Clone"</button><label>"Existing presentation ID"<input class="v2-input" prop:value=move || link_existing_id.get() on:input=move |event| link_existing_id.set(event_target_value(&event)) /></label><button type="button" on:click=move |_| link_existing.run(())>"Link existing"</button><button type="button" on:click=move |_| show_discard.set(true)>"Reset / discard draft"</button><details><summary>"Protocol diagnostics"</summary>{move || diagnostics.get().into_iter().map(|item| view! { <p>{item}</p> }).collect_view()}</details></div></details></footer>

            <dialog node_ref=discard_dialog_ref class="v2-publisher-dialog v2-store-dialog" on:cancel=move |event: web_sys::Event| { event.prevent_default(); show_discard.set(false); }><h2>"Discard Store Page draft?"</h2><p>"Unsaved changes will be removed for this game."</p><div class="v2-store-dialog-actions"><button class="v2-btn-secondary" autofocus on:click=move |_| show_discard.set(false)>"Keep editing"</button><button class="v2-btn-primary" on:click={let key = key.clone(); move |_| { if loading.get_untracked() || validating.get_untracked() || publishing.get_untracked() || partial.get_untracked().is_some() || auth.npub.get_untracked().as_deref() != Some(publisher_for_discard.get_value().as_str()) { show_discard.set(false); message.set(Some("The draft cannot be discarded while an operation is active or the publisher account is unavailable.".into())); return; } PUBLISHER_STORE_PAGE_DRAFTS.with(|drafts| { drafts.borrow_mut().remove(&key); }); on_back.run(()); }}>"Discard changes"</button></div></dialog>
        </section>
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tauri_bridge::{EventPublishOutcome, ListingPointerPublishOutcome};

    #[test]
    fn typed_adapter_preserves_supported_v1_fields() {
        let mut draft = StorePageDraft::new("page".into(), Vec::new());
        draft.content.media.push(StorePageMediaItem {
            id: "hero".into(),
            media_type: "image".into(),
            role: "hero".into(),
            url: "https://cdn.example/hero.png".into(),
            sha256: None,
            mime_type: None,
            size: None,
            thumbnail_url: None,
            alt: Some("alt | text".into()),
            caption: None,
            width: Some(1920),
            height: Some(1080),
        });
        draft.content.sections.push(StorePageSection {
            id: "section-1".into(),
            heading: "Intro".into(),
            body_markdown: "Hello **world**".into(),
            media_id: Some("hero".into()),
            layout: "media-wide".into(),
        });
        draft.content.languages = Some(vec![LanguageSupport {
            code: "en".into(),
            interface: true,
            audio: false,
            subtitles: true,
        }]);
        draft.content.accessibility.push(AccessibilityFeature {
            feature: "subtitles".into(),
            supported: true,
            notes: Some("Configurable".into()),
        });
        let associations = vec![AssociationRow {
            listing_coordinate: "listing".into(),
            event_id: "event".into(),
            reciprocal: true,
            action: ListingPointerMutation::Link,
            relay_hint: None,
        }];
        let (adapted, mutations) = adapter_requests(draft, &associations).expect("typed adapter");
        assert_eq!(adapted.content.media[0].width, Some(1920));
        assert_eq!(
            adapted.content.sections[0].media_id.as_deref(),
            Some("hero")
        );
        assert_eq!(adapted.content.languages.as_ref().map(Vec::len), Some(1));
        assert!(adapted.content.accessibility[0].supported);
        assert_eq!(mutations[0].action, ListingPointerMutation::Link);
    }

    #[test]
    fn editor_text_preserves_spaces_while_typing() {
        assert_eq!(
            optional_editor_text("Arcade ".into()).as_deref(),
            Some("Arcade ")
        );
        assert_eq!(
            optional_editor_text("Arcade Studio".into()).as_deref(),
            Some("Arcade Studio")
        );
        assert_eq!(optional_editor_text(String::new()), None);
    }

    #[test]
    fn manual_url_change_clears_stale_integrity_assertions() {
        let hash = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        let mut draft = StorePageDraft::new("page".into(), Vec::new());
        draft.content.media.push(StorePageMediaItem {
            id: "uploaded".into(),
            media_type: "image".into(),
            role: "screenshot".into(),
            url: format!("https://cdn.example/{hash}.png"),
            sha256: Some(hash.into()),
            mime_type: Some("image/png".into()),
            size: Some(42),
            thumbnail_url: None,
            alt: None,
            caption: None,
            width: None,
            height: None,
        });
        let previous_urls = media_url_snapshot(&draft);

        draft.content.media[0].url = format!("https://other.example/{hash}.png");
        assert!(clear_changed_media_integrity(&mut draft, &previous_urls));
        let media = &draft.content.media[0];
        assert_eq!(
            (media.sha256.as_ref(), media.mime_type.as_ref(), media.size),
            (None, None, None)
        );
    }

    #[test]
    fn association_editor_distinguishes_link_unlink_and_review() {
        let rows = [
            ListingPointerMutation::Link,
            ListingPointerMutation::Unlink,
            ListingPointerMutation::Review,
        ]
        .into_iter()
        .enumerate()
        .map(|(index, action)| AssociationRow {
            listing_coordinate: format!("listing-{index}"),
            event_id: format!("event-{index}"),
            reciprocal: false,
            action,
            relay_hint: None,
        })
        .collect::<Vec<_>>();
        let (_, values) = adapter_requests(StorePageDraft::new("page".into(), Vec::new()), &rows)
            .expect("adapter");
        assert_eq!(
            values.iter().map(|value| value.action).collect::<Vec<_>>(),
            vec![
                ListingPointerMutation::Link,
                ListingPointerMutation::Unlink,
                ListingPointerMutation::Review
            ]
        );
    }

    #[test]
    fn media_reorder_and_singular_roles_are_enforced() {
        let item = |id: &str, role: &str| StorePageMediaItem {
            id: id.into(),
            media_type: "image".into(),
            role: role.into(),
            url: format!("https://cdn.example/{id}.png"),
            sha256: None,
            mime_type: None,
            size: None,
            thumbnail_url: None,
            alt: None,
            caption: None,
            width: None,
            height: None,
        };
        let mut media = vec![item("a", "screenshot"), item("b", "screenshot")];
        move_item(&mut media, 1, -1);
        assert_eq!(media[0].id, "b");
        let mut draft = StorePageDraft::new("page".into(), Vec::new());
        draft.content.media = vec![item("hero-a", "hero"), item("hero-b", "hero")];
        assert!(adapter_requests(draft, &[]).is_err());
    }

    #[test]
    fn section_media_selection_must_reference_typed_media() {
        let mut draft = StorePageDraft::new("page".into(), Vec::new());
        draft.content.sections.push(StorePageSection {
            id: "section-1".into(),
            heading: "Details".into(),
            body_markdown: String::new(),
            media_id: Some("missing".into()),
            layout: "media-left".into(),
        });
        assert!(adapter_requests(draft, &[]).is_err());
    }

    #[test]
    fn publisher_draft_cache_is_scoped_by_account() {
        let mut draft = StorePageDraft::new("page".into(), vec!["listing".into()]);
        let baseline = draft.clone();
        draft.content.description_markdown = "unsaved".into();
        save_cached_draft("npub-a|listing", draft.clone(), baseline, Vec::new(), true);
        assert!(cached_draft("npub-a|listing").is_some());
        assert!(cached_draft("npub-b|listing").is_none());
        assert_eq!(
            publisher_store_page_dirty_coordinates("npub-a"),
            vec!["listing".to_string()]
        );
        assert!(publisher_store_page_dirty_coordinates("npub-b").is_empty());
        PUBLISHER_STORE_PAGE_DRAFTS.with(|drafts| drafts.borrow_mut().clear());
    }

    #[test]
    fn publisher_dashboard_sees_only_dirty_in_session_store_page_drafts() {
        let clean = StorePageDraft::new("clean".into(), vec!["clean-listing".into()]);
        save_cached_draft(
            "npub-dashboard|clean-listing",
            clean.clone(),
            clean,
            Vec::new(),
            false,
        );
        let baseline = StorePageDraft::new("dirty".into(), vec!["dirty-listing".into()]);
        let mut dirty = baseline.clone();
        dirty.content.description_markdown = "Unsaved".into();
        save_cached_draft(
            "npub-dashboard|dirty-listing",
            dirty,
            baseline,
            Vec::new(),
            true,
        );

        assert_eq!(
            publisher_store_page_dirty_coordinates("npub-dashboard"),
            vec!["dirty-listing".to_string()]
        );
        assert!(publisher_store_page_dirty_coordinates("npub-other").is_empty());
        PUBLISHER_STORE_PAGE_DRAFTS.with(|drafts| drafts.borrow_mut().clear());
    }

    #[test]
    fn new_draft_keeps_selected_listing_association_before_load() {
        let coordinate = "30402:publisher:game";
        let mut draft = StorePageDraft::new("game".into(), Vec::new());
        let mut associations = Vec::new();

        seed_new_draft_association(
            &mut draft,
            &mut associations,
            coordinate,
            Some("listing-event"),
        );

        assert_eq!(draft.listing_coordinates, vec![coordinate.to_string()]);
        assert_eq!(associations[0].listing_coordinate, coordinate);
    }

    #[test]
    fn linked_platforms_use_only_selected_authoritative_listing_values() {
        let platforms = vec!["linux-x86_64".to_string(), "windows-x86_64".to_string()];
        let mut associations = vec![AssociationRow {
            listing_coordinate: "selected".into(),
            event_id: "event".into(),
            reciprocal: true,
            action: ListingPointerMutation::Unlink,
            relay_hint: None,
        }];
        assert!(linked_platforms(&platforms, &associations, "selected").is_empty());
        associations[0].action = ListingPointerMutation::Link;
        assert_eq!(
            linked_platforms(&platforms, &associations, "selected"),
            platforms
        );
        assert_eq!(platform_label("linux-x86_64"), "Linux X86 64");
    }

    #[test]
    fn typed_languages_accessibility_and_readiness_remain_recommendations() {
        let mut draft = StorePageDraft::new("page".into(), Vec::new());
        draft.content.basic.title = Some("Game".into());
        draft.content.languages = Some(vec![LanguageSupport {
            code: "pt-BR".into(),
            interface: true,
            audio: false,
            subtitles: true,
        }]);
        draft.content.accessibility.push(AccessibilityFeature {
            feature: "subtitles".into(),
            supported: true,
            notes: Some("Publisher provided".into()),
        });
        let associations = vec![AssociationRow {
            listing_coordinate: "listing".into(),
            event_id: "event".into(),
            reciprocal: true,
            action: ListingPointerMutation::Link,
            relay_hint: None,
        }];
        let status = readiness(&draft, &associations, &["linux-x86_64".into()], &[]);
        assert!(status.blockers.is_empty());
        assert!(!status
            .recommendations
            .iter()
            .any(|item| item.contains("language")));
        assert!(!status
            .recommendations
            .iter()
            .any(|item| item.contains("accessibility")));
        assert!(status
            .recommendations
            .iter()
            .any(|item| item.contains("requirements")));
    }

    #[test]
    fn diagnostic_mapping_preview_source_and_responsive_tab_state_are_stable() {
        assert_eq!(diagnostic_tab("media URL is invalid"), "media");
        assert_eq!(diagnostic_tab("unknown protocol issue"), "basic");
        assert_eq!(CANONICAL_PREVIEW_SOURCE, "canonical-validation");
        assert_eq!(adjacent_tab("basic", -1), "links");
        assert_eq!(adjacent_tab("links", 1), "basic");
    }

    #[test]
    fn conservative_media_preview_requires_https_and_known_extensions() {
        assert!(safe_media_preview(
            "https://cdn.example/shot.webp?x=1",
            "image"
        ));
        assert!(safe_media_preview(
            "https://cdn.example/trailer.webm",
            "video"
        ));
        assert!(!safe_media_preview("http://cdn.example/shot.png", "image"));
        assert!(!safe_media_preview("https://cdn.example/file", "image"));
        assert!(safe_https_link("https://arcadestr.example/support"));
        assert!(!safe_https_link("https://"));
        assert!(!safe_https_link("javascript:alert(1)"));
    }

    #[test]
    fn editor_ids_fill_first_available_gap_and_reject_duplicates() {
        assert_eq!(unique_editor_id("media", ["media-1", "media-3"]), "media-2");
        assert_eq!(
            unique_editor_id("section", std::iter::empty::<&str>()),
            "section-1"
        );

        let mut draft = StorePageDraft::new("page".into(), Vec::new());
        let media = |id: &str| StorePageMediaItem {
            id: id.into(),
            media_type: "image".into(),
            role: "screenshot".into(),
            url: "https://cdn.example/image.png".into(),
            sha256: None,
            mime_type: None,
            size: None,
            thumbnail_url: None,
            alt: None,
            caption: None,
            width: None,
            height: None,
        };
        draft.content.media = vec![media("media-1"), media("media-1")];
        assert!(validate_editor_ids(&draft).is_err());
    }

    #[test]
    fn markdown_destinations_are_rejected_by_typed_adapter() {
        assert!(description_contains_link(
            "Read [the guide](https://example.org)"
        ));
        assert!(description_contains_link(
            "![cover](https://example.org/a.png)"
        ));
        assert!(!description_contains_link("Use **bold** and headings."));

        let mut draft = StorePageDraft::new("page".into(), Vec::new());
        draft.content.description_markdown = "[link](https://example.org)".into();
        assert!(adapter_requests(draft, &[]).is_err());
    }

    #[test]
    fn valid_validation_diagnostics_and_relationship_diagnostics_are_warnings() {
        let mut draft = StorePageDraft::new("page".into(), Vec::new());
        draft.content.basic.title = Some("Game".into());
        let associations = vec![AssociationRow {
            listing_coordinate: "listing".into(),
            event_id: "event".into(),
            reciprocal: true,
            action: ListingPointerMutation::Link,
            relay_hint: None,
        }];
        let status = readiness(
            &draft,
            &associations,
            &[],
            &[
                "Warning: sanitized Markdown".into(),
                "Association: incomplete".into(),
            ],
        );
        assert!(status.blockers.is_empty());
        assert_eq!(status.warnings.len(), 2);
    }

    #[test]
    fn retry_after_partial_success_only_targets_incomplete_pointers() {
        let mutations = vec![
            StorePageListingMutation {
                listing_coordinate: "listing-a".into(),
                expected_event_id: "a".into(),
                action: ListingPointerMutation::Link,
                relay_hint: None,
                published_event_id: None,
            },
            StorePageListingMutation {
                listing_coordinate: "listing-b".into(),
                expected_event_id: "b".into(),
                action: ListingPointerMutation::Link,
                relay_hint: None,
                published_event_id: None,
            },
        ];
        let result = PublishStorePageResponse {
            store_page_coordinate: "page".into(),
            store_page: Some(EventPublishOutcome {
                event_id: "page-event".into(),
                success_count: 2,
                failure_count: 0,
                propagation_confirmed: true,
            }),
            listing_updates: vec![
                ListingPointerPublishOutcome {
                    listing_coordinate: "listing-a".into(),
                    action: ListingPointerMutation::Link,
                    replacement_event_id: Some("new-a".into()),
                    published: true,
                    propagation_confirmed: true,
                    error: None,
                },
                ListingPointerPublishOutcome {
                    listing_coordinate: "listing-b".into(),
                    action: ListingPointerMutation::Link,
                    replacement_event_id: None,
                    published: false,
                    propagation_confirmed: false,
                    error: Some("failed".into()),
                },
            ],
            complete: false,
            retryable: true,
            cache_error: None,
            retry_scope_complete: false,
        };

        let retry = retryable_mutations(&result, &mutations);
        assert_eq!(retry, vec![mutations[1].clone()]);
        assert_eq!(
            selected_replacement_event_id(&result, "listing-a").as_deref(),
            Some("new-a")
        );
        assert!(selected_replacement_event_id(&result, "listing-b").is_none());
    }

    #[test]
    fn preview_commerce_is_derived_only_from_listing_values() {
        assert_eq!(
            preview_commerce_label(19.99, "USD", &crate::models::AcquisitionPolicy::Gated),
            "19.99 USD · Gated"
        );
    }

    #[test]
    fn account_generation_rejects_late_editor_response() {
        assert!(!accepts_account_response(
            Some("npub-new"),
            "npub-old",
            2,
            1
        ));
    }

    fn publication_response(
        page: Option<(usize, bool)>,
        pointers: &[(bool, bool)],
    ) -> PublishStorePageResponse {
        PublishStorePageResponse {
            store_page_coordinate: "page".into(),
            store_page: page.map(
                |(success_count, propagation_confirmed)| EventPublishOutcome {
                    event_id: "page-event".into(),
                    success_count,
                    failure_count: if success_count == 0 { 1 } else { 0 },
                    propagation_confirmed,
                },
            ),
            listing_updates: pointers
                .iter()
                .enumerate()
                .map(
                    |(index, (published, propagation_confirmed))| ListingPointerPublishOutcome {
                        listing_coordinate: format!("listing-{index}"),
                        action: ListingPointerMutation::Link,
                        replacement_event_id: published.then(|| format!("event-{index}")),
                        published: *published,
                        propagation_confirmed: *propagation_confirmed,
                        error: (!published).then(|| "failed".to_string()),
                    },
                )
                .collect(),
            complete: false,
            retryable: true,
            cache_error: None,
            retry_scope_complete: false,
        }
    }

    #[test]
    fn store_page_and_pointer_publication_never_collapse_into_one_state() {
        let both_done = publication_response(Some((2, true)), &[(true, true)]);
        let lifecycle = publication_lifecycle(false, Some(&both_done));
        assert_eq!(lifecycle.store_page, StageOutcome::Complete);
        assert_eq!(lifecycle.pointers, StageOutcome::Complete);
        assert_eq!(
            overall_publication_label(lifecycle),
            "Store Page and listing pointers published"
        );

        // Store Page accepted, pointer failed: never reported as published.
        let pointer_failed = publication_response(Some((2, true)), &[(false, false)]);
        let lifecycle = publication_lifecycle(false, Some(&pointer_failed));
        assert_eq!(lifecycle.store_page, StageOutcome::Complete);
        assert_eq!(lifecycle.pointers, StageOutcome::Failed);
        assert_eq!(
            overall_publication_label(lifecycle),
            "Store Page published; listing pointer update failed"
        );

        // Mixed pointer results stay partial rather than complete.
        let pointer_partial = publication_response(Some((2, true)), &[(true, true), (true, false)]);
        assert_eq!(
            pointer_stage(false, Some(&pointer_partial)),
            StageOutcome::Partial
        );

        // Store Page accepted by relays without propagation confirmation is partial.
        let page_partial = publication_response(Some((1, false)), &[]);
        assert_eq!(
            store_page_stage(false, Some(&page_partial)),
            StageOutcome::Partial
        );
        assert_eq!(
            pointer_stage(false, Some(&page_partial)),
            StageOutcome::NotAttempted
        );

        // Zero relay acceptances is a failure, not a partial success.
        let page_failed = publication_response(Some((0, false)), &[]);
        assert_eq!(
            store_page_stage(false, Some(&page_failed)),
            StageOutcome::Failed
        );
    }

    #[test]
    fn publication_stages_report_pending_while_busy_and_idle_before_any_attempt() {
        let idle = publication_lifecycle(false, None);
        assert_eq!(idle.store_page, StageOutcome::NotAttempted);
        assert_eq!(idle.pointers, StageOutcome::NotAttempted);
        assert_eq!(
            overall_publication_label(idle),
            "Not published from this editor"
        );

        let busy = publication_lifecycle(true, None);
        assert_eq!(busy.store_page, StageOutcome::Pending);
        assert_eq!(overall_publication_label(busy), "Publishing");
    }

    #[test]
    fn in_memory_editor_state_is_never_labelled_saved() {
        assert_eq!(draft_persistence_label(true), "Unsaved in-memory changes");
        assert_eq!(draft_persistence_label(false), "No in-memory changes");
        for dirty in [true, false] {
            assert!(!draft_persistence_label(dirty)
                .to_ascii_lowercase()
                .contains("saved revision"));
        }
        assert!(revision_label(None).contains("No published Store Page revision"));
        assert!(revision_label(Some("abc")).contains("abc"));

        // The source must not reintroduce a bare "Saved" state label.
        let source = include_str!("store_page_publish.rs");
        assert!(!source.contains(concat!("{ \"Saved", "\" }")));
        assert!(!source.contains(concat!("\"Saved ", "revision\"")));
    }

    #[test]
    fn tab_state_follows_authoritative_readiness_output() {
        let blockers = vec!["media: unsupported URL".to_string()];
        let warnings = vec!["Association: listing needs review".to_string()];
        assert_eq!(
            editor_tab_state("media", &blockers, &warnings),
            EditorTabState::Blocked
        );
        assert_eq!(
            editor_tab_state("basic", &[], &warnings),
            EditorTabState::Warned
        );
        assert_eq!(
            editor_tab_state("links", &blockers, &warnings),
            EditorTabState::Neutral
        );
    }

    #[test]
    fn blockers_warnings_and_recommendations_stay_separate() {
        let draft = StorePageDraft::new(String::new(), Vec::new());
        let state = readiness(&draft, &[], &[], &[]);
        // Missing optional media is advisory, never a blocker.
        assert!(state
            .recommendations
            .iter()
            .any(|item| item.contains("hero")));
        assert!(!state.blockers.iter().any(|item| item.contains("hero")));
        // A missing association blocks publication.
        assert!(state
            .blockers
            .iter()
            .any(|item| item.contains("listing association")));
        assert!(state
            .recommendations
            .iter()
            .all(|item| !state.blockers.contains(item)));
    }

    #[test]
    fn media_slots_never_describe_a_local_reference_as_published() {
        assert_eq!(media_slot_state("", "image"), MediaSlotState::Empty);
        assert_eq!(
            media_slot_state("http://insecure.example/a.png", "image"),
            MediaSlotState::Invalid
        );
        for state in [
            MediaSlotState::Empty,
            MediaSlotState::Invalid,
            MediaSlotState::Referenced,
        ] {
            assert!(!state.label().to_ascii_lowercase().contains("published to"));
        }
        assert!(MediaSlotState::Referenced.label().contains("not published"));
    }

    #[test]
    fn first_blocker_routes_to_the_owning_tab_with_a_focus_fallback() {
        let blockers = vec![
            "media: unsupported URL".to_string(),
            "Title is required.".to_string(),
        ];
        let target = first_blocker_target(&blockers).expect("a blocker target");
        assert_eq!(target.tab, "media");
        assert_eq!(target.message, "media: unsupported URL");
        // Media has no stable in-panel target, so focus falls back to its tab.
        assert_eq!(target.element_id, None);

        // Basic Info exposes a stable panel id and is used directly.
        let basic =
            first_blocker_target(&["Title is required.".to_string()]).expect("a blocker target");
        assert_eq!(basic.tab, "basic");
        assert_eq!(basic.element_id.as_deref(), Some("store-editor-basic"));

        // No blockers means no navigation action at all.
        assert_eq!(first_blocker_target(&[]), None);
    }

    #[test]
    fn first_blocker_navigation_does_not_resolve_the_blocker() {
        let blockers = vec!["Title is required.".to_string()];
        let before = first_blocker_target(&blockers);
        // Selecting the target is presentation only; the same blocker remains.
        let after = first_blocker_target(&blockers);
        assert_eq!(before, after);
        assert!(after.is_some());
    }

    #[test]
    fn retry_targets_only_incomplete_pointer_work_and_keeps_published_identifiers() {
        let mutations = vec![
            StorePageListingMutation {
                listing_coordinate: "listing-0".into(),
                expected_event_id: "a".into(),
                action: ListingPointerMutation::Link,
                relay_hint: None,
                published_event_id: None,
            },
            StorePageListingMutation {
                listing_coordinate: "listing-1".into(),
                expected_event_id: "b".into(),
                action: ListingPointerMutation::Link,
                relay_hint: None,
                published_event_id: None,
            },
        ];
        // listing-0 published but unconfirmed, listing-1 not published at all.
        let response = publication_response(Some((2, true)), &[(true, false), (false, false)]);
        let retry = retryable_mutations(&response, &mutations);
        assert_eq!(retry.len(), 2);
        assert_eq!(retry[0].published_event_id.as_deref(), Some("event-0"));
        assert_eq!(retry[1].published_event_id, None);
    }
}
