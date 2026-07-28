use std::collections::{BTreeMap, HashSet};
use std::io::{self, Write};

use nostr::{Event, EventBuilder, PublicKey, Tag};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::adp_protocol::{EXPERIMENTAL_STORE_PAGE_KIND, NIP99_LISTING_KIND};
use crate::is_replaceable_event_newer;
use crate::store_page_content_policy::{
    is_allowed_direct_video_url, sanitize_markdown, validate_store_page_url, ContentPolicyError,
    MarkdownDiagnostic, SanitizedMarkdown, MAX_ACCESSIBILITY_ENTRIES, MAX_ASSOCIATED_LISTINGS,
    MAX_EVENT_BYTES, MAX_EVENT_CONTENT_BYTES, MAX_EXTERNAL_LINKS, MAX_FEATURES,
    MAX_FEATURE_SECTIONS, MAX_GENRES, MAX_IDENTIFIER_CHARS, MAX_LANGUAGES,
    MAX_MARKDOWN_OUTPUT_BYTES, MAX_MARKDOWN_SOURCE_BYTES, MAX_MEDIA_ITEMS, MAX_SCREENSHOTS,
    MAX_SECTION_MARKDOWN_OUTPUT_BYTES, MAX_SECTION_MARKDOWN_SOURCE_BYTES, MAX_SUMMARY_CHARS,
    MAX_TEXT_FIELD_CHARS, MAX_TITLE_CHARS, MAX_TRAILERS,
};

pub const STORE_PAGE_SCHEMA: &str = "io.arcadestr.store-page";
pub const STORE_PAGE_SCHEMA_VERSION: u64 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct StorePageBasic {
    pub title: Option<String>,
    pub summary: Option<String>,
    pub developer: Option<String>,
    pub publisher: Option<String>,
    pub release_date: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct StorePageDiscovery {
    pub genres: Option<Vec<String>>,
    pub features: Option<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StorePageMediaItem {
    pub id: String,
    #[serde(rename = "type")]
    pub media_type: String,
    pub role: String,
    pub url: String,
    #[serde(default)]
    pub thumbnail_url: Option<String>,
    #[serde(default)]
    pub alt: Option<String>,
    #[serde(default)]
    pub caption: Option<String>,
    #[serde(default)]
    pub width: Option<u32>,
    #[serde(default)]
    pub height: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StorePageSection {
    pub id: String,
    pub heading: String,
    pub body_markdown: String,
    #[serde(default)]
    pub media_id: Option<String>,
    pub layout: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LanguageSupport {
    pub code: String,
    #[serde(default)]
    pub interface: bool,
    #[serde(default)]
    pub audio: bool,
    #[serde(default)]
    pub subtitles: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct RequirementTier {
    pub os: Option<String>,
    pub processor: Option<String>,
    pub memory: Option<String>,
    pub graphics: Option<String>,
    pub storage: Option<String>,
    pub additional: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct PlatformRequirement {
    pub minimum: Option<RequirementTier>,
    pub recommended: Option<RequirementTier>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccessibilityFeature {
    pub feature: String,
    pub supported: bool,
    #[serde(default)]
    pub notes: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct StorePageLinks {
    pub website: Option<String>,
    pub support: Option<String>,
    pub documentation: Option<String>,
    pub source: Option<String>,
    pub community: Option<String>,
    pub privacy_policy: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StorePageContentV1 {
    pub schema: String,
    pub version: u64,
    #[serde(default)]
    pub basic: StorePageBasic,
    #[serde(default)]
    pub description_markdown: String,
    #[serde(default)]
    pub discovery: StorePageDiscovery,
    #[serde(default)]
    pub media: Vec<StorePageMediaItem>,
    #[serde(default)]
    pub sections: Vec<StorePageSection>,
    #[serde(default)]
    pub languages: Option<Vec<LanguageSupport>>,
    #[serde(default)]
    pub requirements: BTreeMap<String, PlatformRequirement>,
    #[serde(default)]
    pub accessibility: Vec<AccessibilityFeature>,
    #[serde(default)]
    pub links: StorePageLinks,
}

impl Default for StorePageContentV1 {
    fn default() -> Self {
        Self {
            schema: STORE_PAGE_SCHEMA.to_string(),
            version: STORE_PAGE_SCHEMA_VERSION,
            basic: StorePageBasic::default(),
            description_markdown: String::new(),
            discovery: StorePageDiscovery::default(),
            media: Vec::new(),
            sections: Vec::new(),
            languages: None,
            requirements: BTreeMap::new(),
            accessibility: Vec::new(),
            links: StorePageLinks::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct StorePageCompactTags {
    pub title: Option<String>,
    pub summary: Option<String>,
    pub developer: Option<String>,
    pub publisher: Option<String>,
    pub release_date: Option<String>,
    pub genres: Vec<String>,
    pub features: Vec<String>,
    pub languages: Vec<LanguageSupport>,
    pub website: Option<String>,
    pub support: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StorePageBuildParams {
    pub publisher: PublicKey,
    pub presentation_id: String,
    pub listing_coordinates: Vec<String>,
    pub content: StorePageContentV1,
    pub compact_tags: StorePageCompactTags,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StorePageDraft {
    pub presentation_id: String,
    pub listing_coordinates: Vec<String>,
    pub content: StorePageContentV1,
    #[serde(default)]
    pub compact_tags: StorePageCompactTags,
    #[serde(default)]
    pub loaded_event_id: Option<String>,
}

impl StorePageDraft {
    pub fn new(presentation_id: String, listing_coordinates: Vec<String>) -> Self {
        Self {
            presentation_id,
            listing_coordinates,
            content: StorePageContentV1::default(),
            compact_tags: StorePageCompactTags::default(),
            loaded_event_id: None,
        }
    }

    pub fn build_params(&self, publisher: PublicKey) -> StorePageBuildParams {
        StorePageBuildParams {
            publisher,
            presentation_id: self.presentation_id.clone(),
            listing_coordinates: self.listing_coordinates.clone(),
            content: self.content.clone(),
            compact_tags: self.compact_tags.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StorePageValidationDiagnostic {
    pub code: String,
    pub message: String,
}

impl StorePageValidationDiagnostic {
    pub fn from_error(error: &StorePageError) -> Self {
        let code = match error {
            StorePageError::InvalidSignature => "invalid_signature",
            StorePageError::WrongKind { .. } => "wrong_kind",
            StorePageError::MissingTag(_) => "missing_tag",
            StorePageError::DuplicateTag(_) => "duplicate_tag",
            StorePageError::MalformedTag(_) => "malformed_tag",
            StorePageError::WrongPublisher => "wrong_publisher",
            StorePageError::TooManyAssociations => "too_many_associations",
            StorePageError::UnsupportedSchema(_) => "unsupported_schema",
            StorePageError::UnsupportedSchemaVersion(_) => "unsupported_schema_version",
            StorePageError::InvalidContent(_) => "invalid_content",
            StorePageError::ContentPolicy(_) => "content_policy",
        };
        Self {
            code: code.to_string(),
            message: error.to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidatedStorePageDraft {
    pub normalized: NormalizedStorePage,
    pub diagnostics: Vec<StorePageDiagnostic>,
    sanitized_content: SanitizedStorePageContent,
}

impl ValidatedStorePageDraft {
    pub fn sanitized_content(&self) -> &SanitizedStorePageContent {
        &self.sanitized_content
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StorePagePointerAction {
    Link {
        store_page_coordinate: String,
        relay_hint: Option<String>,
    },
    Unlink,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum StorePagePublishError {
    #[error("the Store Page changed since the editor loaded")]
    StaleStorePage,
    #[error("the listing changed since the editor loaded: {0}")]
    StaleListing(String),
    #[error("a Store Page already exists at this presentation ID")]
    StorePageAlreadyExists,
    #[error("listing pointer update requires a valid signed kind:30402 event")]
    InvalidListing,
    #[error("listing is not owned by the active publisher")]
    WrongListingPublisher,
    #[error("invalid Store Page pointer: {0}")]
    InvalidPointer(String),
    #[error(transparent)]
    StorePage(#[from] StorePageError),
}

pub fn clone_store_page_draft(source: &StorePageDraft, presentation_id: String) -> StorePageDraft {
    StorePageDraft {
        presentation_id,
        listing_coordinates: Vec::new(),
        content: source.content.clone(),
        compact_tags: source.compact_tags.clone(),
        loaded_event_id: None,
    }
}

pub fn validate_store_page_revision(
    expected_event_id: Option<&str>,
    current_event_id: Option<&str>,
) -> Result<(), StorePagePublishError> {
    match (expected_event_id, current_event_id) {
        (None, None) => Ok(()),
        (None, Some(_)) => Err(StorePagePublishError::StorePageAlreadyExists),
        (Some(expected), Some(current)) if expected == current => Ok(()),
        _ => Err(StorePagePublishError::StaleStorePage),
    }
}

pub fn validate_listing_revision(
    coordinate: &str,
    expected_event_id: &str,
    current_event_id: &str,
) -> Result<(), StorePagePublishError> {
    if expected_event_id == current_event_id {
        Ok(())
    } else {
        Err(StorePagePublishError::StaleListing(coordinate.to_string()))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SanitizedStorePageSection {
    pub id: String,
    pub heading: String,
    pub body_html: SanitizedMarkdown,
    pub media_id: Option<String>,
    pub layout: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Default)]
pub struct SanitizedStorePageContent {
    pub description_html: SanitizedMarkdown,
    pub media: Vec<StorePageMediaItem>,
    pub sections: Vec<SanitizedStorePageSection>,
    pub links: StorePageLinks,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum OptionalFieldReason {
    InvalidUrl,
    UnsupportedMediaType,
    UnsupportedVideoFormat,
    DuplicateSingletonRole,
    MissingMediaReference,
    UnsupportedLayout,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum StorePageDiagnostic {
    ContentOverridesTag(&'static str),
    OptionalFieldOmitted {
        field: String,
        reason: OptionalFieldReason,
    },
    MarkdownSanitized {
        field: String,
        issue: MarkdownDiagnostic,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct NormalizedStorePage {
    pub title: Option<String>,
    pub summary: Option<String>,
    pub developer: Option<String>,
    pub publisher: Option<String>,
    pub release_date: Option<String>,
    pub genres: Vec<String>,
    pub features: Vec<String>,
    pub languages: Vec<LanguageSupport>,
    pub website: Option<String>,
    pub support: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedStorePage {
    pub event: Event,
    pub presentation_id: String,
    pub listing_coordinates: Vec<String>,
    /// Parsed wire content retained for diagnostics and forward compatibility; never render it.
    pub content: StorePageContentV1,
    pub compact_tags: StorePageCompactTags,
    /// The only Store Page content suitable for future rendering.
    sanitized_content: SanitizedStorePageContent,
    pub normalized: NormalizedStorePage,
    pub diagnostics: Vec<StorePageDiagnostic>,
}

impl ParsedStorePage {
    pub fn sanitized_content(&self) -> &SanitizedStorePageContent {
        &self.sanitized_content
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StorePagePointer {
    pub coordinate: String,
    pub publisher_pubkey: String,
    pub presentation_id: String,
    pub relay_hint: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum StorePagePointerDiagnostic {
    MalformedTag {
        tag_index: usize,
        reason: StorePagePointerMalformedReason,
    },
    DuplicatePointers,
    ConflictingPointers,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum StorePagePointerMalformedReason {
    WrongCardinality,
    MalformedCoordinate,
    WrongKind,
    WrongPublisher,
    EmptyIdentifier,
    IdentifierTooLong,
    InvalidRelayHint,
    NonCanonicalCoordinate,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct StorePagePointerReport {
    pub active: Option<StorePagePointer>,
    pub diagnostics: Vec<StorePagePointerDiagnostic>,
    pub pointer_tag_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StorePageAssociation {
    pub listing_coordinate: String,
    pub pointer: StorePagePointer,
    pub store_page: ParsedStorePage,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum StorePagePointerError {
    #[error("listing event has the wrong kind")]
    WrongListingKind,
    #[error("listing event signature is invalid")]
    InvalidListingSignature,
    #[error("listing identity tag is missing or malformed")]
    InvalidListingIdentifier,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum StorePageAssociationError {
    #[error(transparent)]
    InvalidListing(#[from] StorePagePointerError),
    #[error("listing has no active Store Page pointer")]
    NoActivePointer,
    #[error("Store Page coordinate does not match the listing pointer")]
    PointerMismatch,
    #[error("Store Page publisher does not match the listing publisher")]
    PublisherMismatch,
    #[error("Store Page does not reference the listing coordinate")]
    MissingReciprocalReference,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum StorePageError {
    #[error("invalid nostr event signature")]
    InvalidSignature,
    #[error("wrong event kind: expected {expected}, found {found}")]
    WrongKind { expected: u16, found: u16 },
    #[error("event is missing required tag: {0}")]
    MissingTag(&'static str),
    #[error("event contains duplicate tag: {0}")]
    DuplicateTag(String),
    #[error("event contains malformed tag: {0}")]
    MalformedTag(String),
    #[error("listing association is not owned by the Store Page publisher")]
    WrongPublisher,
    #[error("too many associated listings")]
    TooManyAssociations,
    #[error("unsupported Store Page schema: {0}")]
    UnsupportedSchema(String),
    #[error("unsupported Store Page schema version: {0}")]
    UnsupportedSchemaVersion(u64),
    #[error("invalid Store Page content: {0}")]
    InvalidContent(String),
    #[error(transparent)]
    ContentPolicy(#[from] ContentPolicyError),
}

#[derive(Deserialize)]
struct ContentHeader {
    schema: String,
    version: u64,
}

pub fn build_store_page_event_builder(
    params: &StorePageBuildParams,
) -> Result<EventBuilder, StorePageError> {
    let _ = validate_store_page_draft(params)?;

    let content = serde_json::to_string(&params.content)
        .map_err(|error| StorePageError::InvalidContent(error.to_string()))?;
    let mut tags = vec![parse_tag(["d", params.presentation_id.as_str()])?];
    for coordinate in &params.listing_coordinates {
        tags.push(parse_tag(["a", coordinate.as_str()])?);
    }
    append_compact_tags(&mut tags, &params.compact_tags)?;

    Ok(EventBuilder::new(nostr::Kind::Custom(EXPERIMENTAL_STORE_PAGE_KIND), content).tags(tags))
}

pub fn validate_store_page_draft(
    params: &StorePageBuildParams,
) -> Result<ValidatedStorePageDraft, StorePageError> {
    validate_presentation_id(&params.presentation_id)?;
    validate_associations(&params.listing_coordinates, params.publisher)?;
    validate_content(&params.content)?;
    validate_compact_tags(&params.compact_tags)?;
    let (sanitized_content, policy_diagnostics) = sanitize_content(&params.content)?;

    let content = serde_json::to_string(&params.content)
        .map_err(|error| StorePageError::InvalidContent(error.to_string()))?;
    ensure_byte_limit("event content", content.len(), MAX_EVENT_CONTENT_BYTES)?;
    let (normalized, mut diagnostics) = normalize(&params.content, &params.compact_tags);
    diagnostics.extend(policy_diagnostics);
    Ok(ValidatedStorePageDraft {
        normalized,
        diagnostics,
        sanitized_content,
    })
}

pub fn parse_store_page_event(event: &Event) -> Result<ParsedStorePage, StorePageError> {
    if event.kind.as_u16() != EXPERIMENTAL_STORE_PAGE_KIND {
        return Err(StorePageError::WrongKind {
            expected: EXPERIMENTAL_STORE_PAGE_KIND,
            found: event.kind.as_u16(),
        });
    }
    ensure_byte_limit(
        "event content",
        event.content.len(),
        MAX_EVENT_CONTENT_BYTES,
    )?;
    ensure_serialized_event_limit(event)?;
    event
        .verify()
        .map_err(|_| StorePageError::InvalidSignature)?;

    let presentation_id = singleton_tag(event, "d")?.ok_or(StorePageError::MissingTag("d"))?;
    validate_presentation_id(&presentation_id)?;
    let listing_coordinates = association_tags(event)?;
    validate_associations(&listing_coordinates, event.pubkey)?;

    let header: ContentHeader = serde_json::from_str(&event.content)
        .map_err(|error| StorePageError::InvalidContent(error.to_string()))?;
    if header.schema != STORE_PAGE_SCHEMA {
        return Err(StorePageError::UnsupportedSchema(header.schema));
    }
    if header.version != STORE_PAGE_SCHEMA_VERSION {
        return Err(StorePageError::UnsupportedSchemaVersion(header.version));
    }
    let content: StorePageContentV1 = serde_json::from_str(&event.content)
        .map_err(|error| StorePageError::InvalidContent(error.to_string()))?;
    validate_content(&content)?;

    let compact = parse_compact_tags(event)?;
    validate_compact_tags(&compact)?;
    let (mut normalized, mut diagnostics) = normalize(&content, &compact);
    sanitize_optional_url(
        "normalized.website",
        &mut normalized.website,
        &mut diagnostics,
    );
    sanitize_optional_url(
        "normalized.support",
        &mut normalized.support,
        &mut diagnostics,
    );
    let (sanitized_content, policy_diagnostics) = sanitize_content(&content)?;
    diagnostics.extend(policy_diagnostics);
    Ok(ParsedStorePage {
        event: event.clone(),
        presentation_id,
        listing_coordinates,
        content,
        compact_tags: compact,
        sanitized_content,
        normalized,
        diagnostics,
    })
}

pub fn parse_store_page_pointers(
    listing: &Event,
) -> Result<StorePagePointerReport, StorePagePointerError> {
    if listing.kind.as_u16() != NIP99_LISTING_KIND {
        return Err(StorePagePointerError::WrongListingKind);
    }
    listing
        .verify()
        .map_err(|_| StorePagePointerError::InvalidListingSignature)?;
    let listing_id = listing_identifier(listing)?;
    if listing_id.chars().count() > MAX_IDENTIFIER_CHARS {
        return Err(StorePagePointerError::InvalidListingIdentifier);
    }

    let mut pointers = Vec::new();
    let mut diagnostics = Vec::new();
    let mut pointer_tag_count = 0;
    for (tag_index, tag) in listing.tags.iter().enumerate() {
        let values = tag.clone().to_vec();
        if !values.first().is_some_and(|name| name == "store_page") {
            continue;
        }
        pointer_tag_count += 1;
        match parse_store_page_pointer_values(&values, listing.pubkey) {
            Ok(pointer) => pointers.push(pointer),
            Err(reason) => {
                diagnostics.push(StorePagePointerDiagnostic::MalformedTag { tag_index, reason })
            }
        }
    }

    let active = match pointers.as_slice() {
        [] => None,
        [pointer] if pointer_tag_count == 1 => Some(pointer.clone()),
        [_] => None,
        pointers => {
            let first = &pointers[0].coordinate;
            if pointers.iter().all(|pointer| pointer.coordinate == *first) {
                diagnostics.push(StorePagePointerDiagnostic::DuplicatePointers);
            } else {
                diagnostics.push(StorePagePointerDiagnostic::ConflictingPointers);
            }
            None
        }
    };

    Ok(StorePagePointerReport {
        active,
        diagnostics,
        pointer_tag_count,
    })
}

pub fn validate_store_page_association(
    listing: &Event,
    store_page: &ParsedStorePage,
) -> Result<StorePageAssociation, StorePageAssociationError> {
    let report = parse_store_page_pointers(listing)?;
    let pointer = report
        .active
        .ok_or(StorePageAssociationError::NoActivePointer)?;
    let expected_page_coordinate =
        store_page_coordinate(store_page.event.pubkey, store_page.presentation_id.as_str());
    if pointer.coordinate != expected_page_coordinate {
        return Err(StorePageAssociationError::PointerMismatch);
    }
    if store_page.event.pubkey != listing.pubkey {
        return Err(StorePageAssociationError::PublisherMismatch);
    }
    let listing_coordinate = listing_coordinate(listing)?;
    if !store_page.listing_coordinates.contains(&listing_coordinate) {
        return Err(StorePageAssociationError::MissingReciprocalReference);
    }
    Ok(StorePageAssociation {
        listing_coordinate,
        pointer,
        store_page: store_page.clone(),
    })
}

pub fn store_page_coordinate(publisher: PublicKey, presentation_id: &str) -> String {
    format!(
        "{EXPERIMENTAL_STORE_PAGE_KIND}:{}:{presentation_id}",
        publisher.to_hex()
    )
}

pub fn listing_coordinate(listing: &Event) -> Result<String, StorePagePointerError> {
    if listing.kind.as_u16() != NIP99_LISTING_KIND {
        return Err(StorePagePointerError::WrongListingKind);
    }
    let identifier = listing_identifier(listing)?;
    Ok(format!(
        "{NIP99_LISTING_KIND}:{}:{identifier}",
        listing.pubkey.to_hex()
    ))
}

pub fn build_listing_store_page_replacement(
    current: &Event,
    publisher: PublicKey,
    action: &StorePagePointerAction,
) -> Result<EventBuilder, StorePagePublishError> {
    if current.kind.as_u16() != NIP99_LISTING_KIND || current.verify().is_err() {
        return Err(StorePagePublishError::InvalidListing);
    }
    if current.pubkey != publisher {
        return Err(StorePagePublishError::WrongListingPublisher);
    }
    listing_coordinate(current).map_err(|_| StorePagePublishError::InvalidListing)?;

    let mut tags = current
        .tags
        .iter()
        .filter(|tag| {
            let values = (*tag).clone().to_vec();
            !values.first().is_some_and(|name| name == "store_page")
        })
        .cloned()
        .collect::<Vec<_>>();

    if let StorePagePointerAction::Link {
        store_page_coordinate,
        relay_hint,
    } = action
    {
        let mut values = vec!["store_page".to_string(), store_page_coordinate.clone()];
        if let Some(relay_hint) = relay_hint {
            values.push(relay_hint.clone());
        }
        parse_store_page_pointer_values(&values, publisher)
            .map_err(|reason| StorePagePublishError::InvalidPointer(format!("{reason:?}")))?;
        tags.push(
            Tag::parse(values)
                .map_err(|error| StorePagePublishError::InvalidPointer(error.to_string()))?,
        );
    }

    Ok(EventBuilder::new(current.kind, current.content.clone()).tags(tags))
}

fn listing_identifier(listing: &Event) -> Result<String, StorePagePointerError> {
    let matches = listing
        .tags
        .iter()
        .map(|tag| tag.clone().to_vec())
        .filter(|values| values.first().is_some_and(|name| name == "d"))
        .collect::<Vec<_>>();
    if matches.len() != 1 || matches[0].len() != 2 || matches[0][1].is_empty() {
        return Err(StorePagePointerError::InvalidListingIdentifier);
    }
    Ok(matches[0][1].clone())
}

fn parse_store_page_pointer_values(
    values: &[String],
    listing_publisher: PublicKey,
) -> Result<StorePagePointer, StorePagePointerMalformedReason> {
    if !matches!(values.len(), 2 | 3) {
        return Err(StorePagePointerMalformedReason::WrongCardinality);
    }
    let coordinate = values
        .get(1)
        .ok_or(StorePagePointerMalformedReason::WrongCardinality)?;
    let mut parts = coordinate.splitn(3, ':');
    let kind = parts
        .next()
        .and_then(|value| value.parse::<u16>().ok())
        .ok_or(StorePagePointerMalformedReason::MalformedCoordinate)?;
    if kind != EXPERIMENTAL_STORE_PAGE_KIND {
        return Err(StorePagePointerMalformedReason::WrongKind);
    }
    let publisher = parts
        .next()
        .and_then(|value| PublicKey::from_hex(value).ok())
        .ok_or(StorePagePointerMalformedReason::MalformedCoordinate)?;
    if publisher != listing_publisher {
        return Err(StorePagePointerMalformedReason::WrongPublisher);
    }
    let presentation_id = parts
        .next()
        .ok_or(StorePagePointerMalformedReason::MalformedCoordinate)?;
    if presentation_id.is_empty() {
        return Err(StorePagePointerMalformedReason::EmptyIdentifier);
    }
    if presentation_id.chars().count() > MAX_IDENTIFIER_CHARS {
        return Err(StorePagePointerMalformedReason::IdentifierTooLong);
    }
    let canonical = store_page_coordinate(publisher, presentation_id);
    if coordinate != &canonical {
        return Err(StorePagePointerMalformedReason::NonCanonicalCoordinate);
    }
    let relay_hint = values
        .get(2)
        .map(|value| {
            let relay = url::Url::parse(value)
                .map_err(|_| StorePagePointerMalformedReason::InvalidRelayHint)?;
            if relay.scheme() != "wss"
                || relay.host_str().is_none()
                || !relay.username().is_empty()
                || relay.password().is_some()
                || !relay_host_is_public(&relay)
            {
                return Err(StorePagePointerMalformedReason::InvalidRelayHint);
            }
            Ok(relay.to_string())
        })
        .transpose()?;
    Ok(StorePagePointer {
        coordinate: canonical,
        publisher_pubkey: publisher.to_hex(),
        presentation_id: presentation_id.to_string(),
        relay_hint,
    })
}

fn relay_host_is_public(relay: &url::Url) -> bool {
    match relay.host() {
        Some(url::Host::Domain(host)) => {
            let host = host.to_ascii_lowercase();
            host != "localhost" && !host.ends_with(".localhost") && !host.ends_with(".local")
        }
        Some(url::Host::Ipv4(address)) => {
            !address.is_private()
                && !address.is_loopback()
                && !address.is_link_local()
                && !address.is_unspecified()
                && !address.is_broadcast()
                && !address.is_multicast()
        }
        Some(url::Host::Ipv6(address)) => {
            !address.is_loopback()
                && !address.is_unspecified()
                && !address.is_unique_local()
                && !address.is_unicast_link_local()
                && !address.is_multicast()
                && address.to_ipv4_mapped().is_none_or(|mapped| {
                    !mapped.is_private()
                        && !mapped.is_loopback()
                        && !mapped.is_link_local()
                        && !mapped.is_unspecified()
                })
        }
        None => false,
    }
}

pub fn resolve_store_page_events<'a>(
    events: impl IntoIterator<Item = &'a Event>,
    publisher: PublicKey,
    presentation_id: &str,
) -> Option<ParsedStorePage> {
    let mut current: Option<ParsedStorePage> = None;
    for event in events {
        let Ok(candidate) = parse_store_page_event(event) else {
            continue;
        };
        if candidate.event.pubkey != publisher || candidate.presentation_id != presentation_id {
            continue;
        }
        let should_replace = current.as_ref().map_or(true, |existing| {
            let candidate_id = candidate.event.id.to_hex();
            let current_id = existing.event.id.to_hex();
            is_replaceable_event_newer(
                candidate.event.created_at.as_secs(),
                Some(candidate_id.as_str()),
                existing.event.created_at.as_secs(),
                Some(current_id.as_str()),
            )
        });
        if should_replace {
            current = Some(candidate);
        }
    }
    current
}

fn validate_presentation_id(value: &str) -> Result<(), StorePageError> {
    if value.is_empty() {
        Err(StorePageError::MalformedTag("d".into()))
    } else {
        ensure_char_limit("presentation ID", value, MAX_IDENTIFIER_CHARS)?;
        Ok(())
    }
}

fn validate_associations(
    coordinates: &[String],
    publisher: PublicKey,
) -> Result<(), StorePageError> {
    if coordinates.is_empty() {
        return Err(StorePageError::MissingTag("a"));
    }
    if coordinates.len() > MAX_ASSOCIATED_LISTINGS {
        return Err(StorePageError::TooManyAssociations);
    }
    let mut unique = HashSet::with_capacity(coordinates.len());
    for coordinate in coordinates {
        let (kind, coordinate_publisher, identifier) = parse_coordinate(coordinate)?;
        if kind != NIP99_LISTING_KIND || identifier.is_empty() {
            return Err(StorePageError::MalformedTag("a".into()));
        }
        ensure_char_limit("listing identifier", identifier, MAX_IDENTIFIER_CHARS)?;
        if coordinate_publisher != publisher {
            return Err(StorePageError::WrongPublisher);
        }
        let canonical = format!(
            "{NIP99_LISTING_KIND}:{}:{identifier}",
            coordinate_publisher.to_hex()
        );
        if coordinate != &canonical {
            return Err(StorePageError::MalformedTag("a".into()));
        }
        if !unique.insert(canonical) {
            return Err(StorePageError::DuplicateTag("a".into()));
        }
    }
    Ok(())
}

fn parse_coordinate(value: &str) -> Result<(u16, PublicKey, &str), StorePageError> {
    let mut parts = value.splitn(3, ':');
    let kind = parts
        .next()
        .and_then(|value| value.parse().ok())
        .ok_or_else(|| StorePageError::MalformedTag("a".into()))?;
    let publisher = parts
        .next()
        .and_then(|value| PublicKey::from_hex(value).ok())
        .ok_or_else(|| StorePageError::MalformedTag("a".into()))?;
    let identifier = parts
        .next()
        .ok_or_else(|| StorePageError::MalformedTag("a".into()))?;
    Ok((kind, publisher, identifier))
}

fn validate_content(content: &StorePageContentV1) -> Result<(), StorePageError> {
    if content.schema != STORE_PAGE_SCHEMA {
        return Err(StorePageError::UnsupportedSchema(content.schema.clone()));
    }
    if content.version != STORE_PAGE_SCHEMA_VERSION {
        return Err(StorePageError::UnsupportedSchemaVersion(content.version));
    }
    ensure_count_limit("media items", content.media.len(), MAX_MEDIA_ITEMS)?;
    ensure_count_limit(
        "screenshots",
        content
            .media
            .iter()
            .filter(|item| item.role == "screenshot")
            .count(),
        MAX_SCREENSHOTS,
    )?;
    ensure_count_limit(
        "trailers",
        content
            .media
            .iter()
            .filter(|item| item.role == "trailer")
            .count(),
        MAX_TRAILERS,
    )?;
    ensure_count_limit(
        "feature sections",
        content.sections.len(),
        MAX_FEATURE_SECTIONS,
    )?;
    ensure_count_limit(
        "languages",
        content.languages.as_ref().map_or(0, Vec::len),
        MAX_LANGUAGES,
    )?;
    ensure_count_limit(
        "accessibility entries",
        content.accessibility.len(),
        MAX_ACCESSIBILITY_ENTRIES,
    )?;
    ensure_count_limit(
        "external links",
        [
            &content.links.website,
            &content.links.support,
            &content.links.documentation,
            &content.links.source,
            &content.links.community,
            &content.links.privacy_policy,
        ]
        .into_iter()
        .filter(|value| value.is_some())
        .count(),
        MAX_EXTERNAL_LINKS,
    )?;
    ensure_count_limit(
        "genres",
        content.discovery.genres.as_ref().map_or(0, Vec::len),
        MAX_GENRES,
    )?;
    ensure_count_limit(
        "features",
        content.discovery.features.as_ref().map_or(0, Vec::len),
        MAX_FEATURES,
    )?;
    ensure_optional_char_limit("title", &content.basic.title, MAX_TITLE_CHARS)?;
    ensure_optional_char_limit("summary", &content.basic.summary, MAX_SUMMARY_CHARS)?;
    for (field, value) in [
        ("developer", &content.basic.developer),
        ("publisher", &content.basic.publisher),
        ("release date", &content.basic.release_date),
    ] {
        ensure_optional_char_limit(field, value, MAX_TEXT_FIELD_CHARS)?;
    }
    ensure_byte_limit(
        "description Markdown",
        content.description_markdown.len(),
        MAX_MARKDOWN_SOURCE_BYTES,
    )?;
    let mut media_ids = HashSet::with_capacity(content.media.len());
    for media in &content.media {
        ensure_char_limit("media ID", &media.id, MAX_IDENTIFIER_CHARS)?;
        ensure_char_limit("media type", &media.media_type, MAX_IDENTIFIER_CHARS)?;
        ensure_char_limit("media role", &media.role, MAX_IDENTIFIER_CHARS)?;
        ensure_optional_char_limit("media alt text", &media.alt, MAX_TEXT_FIELD_CHARS)?;
        ensure_optional_char_limit("media caption", &media.caption, MAX_TEXT_FIELD_CHARS)?;
        if media.id.is_empty() || !media_ids.insert(media.id.as_str()) {
            return Err(StorePageError::InvalidContent(
                "media IDs must be non-empty and unique".into(),
            ));
        }
    }
    let mut section_ids = HashSet::with_capacity(content.sections.len());
    for section in &content.sections {
        ensure_char_limit("section ID", &section.id, MAX_IDENTIFIER_CHARS)?;
        ensure_char_limit("section heading", &section.heading, MAX_TEXT_FIELD_CHARS)?;
        ensure_char_limit("section layout", &section.layout, MAX_IDENTIFIER_CHARS)?;
        if let Some(media_id) = &section.media_id {
            ensure_char_limit("section media ID", media_id, MAX_IDENTIFIER_CHARS)?;
        }
        ensure_byte_limit(
            "section Markdown",
            section.body_markdown.len(),
            MAX_SECTION_MARKDOWN_SOURCE_BYTES,
        )?;
        if section.id.is_empty() || !section_ids.insert(section.id.as_str()) {
            return Err(StorePageError::InvalidContent(
                "section IDs must be non-empty and unique".into(),
            ));
        }
    }
    if let Some(languages) = &content.languages {
        for language in languages {
            ensure_char_limit("language code", &language.code, MAX_IDENTIFIER_CHARS)?;
        }
    }
    for (platform, requirement) in &content.requirements {
        ensure_char_limit("requirements platform", platform, MAX_IDENTIFIER_CHARS)?;
        for tier in [&requirement.minimum, &requirement.recommended]
            .into_iter()
            .flatten()
        {
            for value in [
                &tier.os,
                &tier.processor,
                &tier.memory,
                &tier.graphics,
                &tier.storage,
                &tier.additional,
            ] {
                ensure_optional_char_limit("requirement text", value, MAX_TEXT_FIELD_CHARS)?;
            }
        }
    }
    for entry in &content.accessibility {
        ensure_char_limit(
            "accessibility feature",
            &entry.feature,
            MAX_IDENTIFIER_CHARS,
        )?;
        ensure_optional_char_limit("accessibility notes", &entry.notes, MAX_TEXT_FIELD_CHARS)?;
    }
    Ok(())
}

fn sanitize_content(
    content: &StorePageContentV1,
) -> Result<(SanitizedStorePageContent, Vec<StorePageDiagnostic>), StorePageError> {
    let mut diagnostics = Vec::new();
    let (description_html, description_diagnostics) = sanitize_markdown(
        &content.description_markdown,
        MAX_MARKDOWN_SOURCE_BYTES,
        MAX_MARKDOWN_OUTPUT_BYTES,
        "description Markdown",
    )?;
    append_markdown_diagnostics(
        &mut diagnostics,
        "description_markdown",
        description_diagnostics,
    );

    let mut media = Vec::with_capacity(content.media.len());
    let mut singleton_roles = HashSet::new();
    for item in &content.media {
        let field = format!("media[{}]", item.id);
        if !matches!(item.media_type.as_str(), "image" | "video") {
            omit_optional(
                &mut diagnostics,
                field,
                OptionalFieldReason::UnsupportedMediaType,
            );
            continue;
        }
        if (item.role == "trailer" && item.media_type != "video")
            || (matches!(
                item.role.as_str(),
                "hero" | "capsule" | "thumbnail" | "screenshot"
            ) && item.media_type != "image")
        {
            omit_optional(
                &mut diagnostics,
                field,
                OptionalFieldReason::UnsupportedMediaType,
            );
            continue;
        }
        let Ok(url) = validate_store_page_url(&item.url) else {
            omit_optional(
                &mut diagnostics,
                format!("{field}.url"),
                OptionalFieldReason::InvalidUrl,
            );
            continue;
        };
        if item.media_type == "video" && !is_allowed_direct_video_url(&url) {
            omit_optional(
                &mut diagnostics,
                format!("{field}.url"),
                OptionalFieldReason::UnsupportedVideoFormat,
            );
            continue;
        }
        if matches!(item.role.as_str(), "hero" | "capsule" | "thumbnail")
            && !singleton_roles.insert(item.role.as_str())
        {
            omit_optional(
                &mut diagnostics,
                field,
                OptionalFieldReason::DuplicateSingletonRole,
            );
            continue;
        }

        let mut sanitized = item.clone();
        sanitized.url = url;
        if let Some(thumbnail) = &item.thumbnail_url {
            match validate_store_page_url(thumbnail) {
                Ok(url) => sanitized.thumbnail_url = Some(url),
                Err(_) => {
                    sanitized.thumbnail_url = None;
                    omit_optional(
                        &mut diagnostics,
                        format!("{field}.thumbnail_url"),
                        OptionalFieldReason::InvalidUrl,
                    );
                }
            }
        }
        media.push(sanitized);
    }

    let retained_media_ids = media
        .iter()
        .map(|item| item.id.as_str())
        .collect::<HashSet<_>>();
    let mut sections = Vec::with_capacity(content.sections.len());
    for section in &content.sections {
        let field = format!("sections[{}]", section.id);
        let (body_html, markdown_diagnostics) = sanitize_markdown(
            &section.body_markdown,
            MAX_SECTION_MARKDOWN_SOURCE_BYTES,
            MAX_SECTION_MARKDOWN_OUTPUT_BYTES,
            &format!("{field}.body_markdown"),
        )?;
        append_markdown_diagnostics(
            &mut diagnostics,
            &format!("{field}.body_markdown"),
            markdown_diagnostics,
        );
        let media_id = section.media_id.clone().filter(|id| {
            if retained_media_ids.contains(id.as_str()) {
                true
            } else {
                omit_optional(
                    &mut diagnostics,
                    format!("{field}.media_id"),
                    OptionalFieldReason::MissingMediaReference,
                );
                false
            }
        });
        let layout = if matches!(
            section.layout.as_str(),
            "text" | "media-left" | "media-right" | "media-wide"
        ) {
            section.layout.clone()
        } else {
            omit_optional(
                &mut diagnostics,
                format!("{field}.layout"),
                OptionalFieldReason::UnsupportedLayout,
            );
            "text".to_string()
        };
        sections.push(SanitizedStorePageSection {
            id: section.id.clone(),
            heading: section.heading.clone(),
            body_html,
            media_id,
            layout,
        });
    }

    let links = StorePageLinks {
        website: sanitize_link("links.website", &content.links.website, &mut diagnostics),
        support: sanitize_link("links.support", &content.links.support, &mut diagnostics),
        documentation: sanitize_link(
            "links.documentation",
            &content.links.documentation,
            &mut diagnostics,
        ),
        source: sanitize_link("links.source", &content.links.source, &mut diagnostics),
        community: sanitize_link(
            "links.community",
            &content.links.community,
            &mut diagnostics,
        ),
        privacy_policy: sanitize_link(
            "links.privacy_policy",
            &content.links.privacy_policy,
            &mut diagnostics,
        ),
    };

    Ok((
        SanitizedStorePageContent {
            description_html,
            media,
            sections,
            links,
        },
        diagnostics,
    ))
}

fn sanitize_link(
    field: &str,
    value: &Option<String>,
    diagnostics: &mut Vec<StorePageDiagnostic>,
) -> Option<String> {
    let value = value.as_ref()?;
    match validate_store_page_url(value) {
        Ok(url) => Some(url),
        Err(_) => {
            omit_optional(
                diagnostics,
                field.to_string(),
                OptionalFieldReason::InvalidUrl,
            );
            None
        }
    }
}

fn sanitize_optional_url(
    field: &str,
    value: &mut Option<String>,
    diagnostics: &mut Vec<StorePageDiagnostic>,
) {
    let Some(raw) = value.as_ref() else {
        return;
    };
    match validate_store_page_url(raw) {
        Ok(url) => *value = Some(url),
        Err(_) => {
            *value = None;
            omit_optional(
                diagnostics,
                field.to_string(),
                OptionalFieldReason::InvalidUrl,
            );
        }
    }
}

fn append_markdown_diagnostics(
    diagnostics: &mut Vec<StorePageDiagnostic>,
    field: &str,
    markdown_diagnostics: Vec<MarkdownDiagnostic>,
) {
    diagnostics.extend(markdown_diagnostics.into_iter().map(|issue| {
        StorePageDiagnostic::MarkdownSanitized {
            field: field.to_string(),
            issue,
        }
    }));
}

fn omit_optional(
    diagnostics: &mut Vec<StorePageDiagnostic>,
    field: String,
    reason: OptionalFieldReason,
) {
    diagnostics.push(StorePageDiagnostic::OptionalFieldOmitted { field, reason });
}

fn ensure_byte_limit(field: &str, actual: usize, max: usize) -> Result<(), StorePageError> {
    ensure_count_limit(field, actual, max)
}

fn ensure_count_limit(field: &str, actual: usize, max: usize) -> Result<(), StorePageError> {
    if actual > max {
        return Err(ContentPolicyError::LimitExceeded {
            field: field.to_string(),
            max,
        }
        .into());
    }
    Ok(())
}

fn ensure_char_limit(field: &str, value: &str, max: usize) -> Result<(), StorePageError> {
    ensure_count_limit(field, value.chars().count(), max)
}

fn ensure_optional_char_limit(
    field: &str,
    value: &Option<String>,
    max: usize,
) -> Result<(), StorePageError> {
    if let Some(value) = value {
        ensure_char_limit(field, value, max)?;
    }
    Ok(())
}

fn ensure_serialized_event_limit(event: &Event) -> Result<(), StorePageError> {
    let mut writer = LimitedWriter::new(MAX_EVENT_BYTES);
    match serde_json::to_writer(&mut writer, event) {
        Ok(()) => Ok(()),
        Err(_) if writer.exceeded => Err(ContentPolicyError::LimitExceeded {
            field: "serialized event".to_string(),
            max: MAX_EVENT_BYTES,
        }
        .into()),
        Err(error) => Err(StorePageError::InvalidContent(error.to_string())),
    }
}

struct LimitedWriter {
    written: usize,
    max: usize,
    exceeded: bool,
}

impl LimitedWriter {
    fn new(max: usize) -> Self {
        Self {
            written: 0,
            max,
            exceeded: false,
        }
    }
}

impl Write for LimitedWriter {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        let Some(total) = self.written.checked_add(buffer.len()) else {
            self.exceeded = true;
            return Err(io::Error::other("serialized Store Page exceeds limit"));
        };
        if total > self.max {
            self.exceeded = true;
            return Err(io::Error::other("serialized Store Page exceeds limit"));
        }
        self.written = total;
        Ok(buffer.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn association_tags(event: &Event) -> Result<Vec<String>, StorePageError> {
    let mut coordinates = Vec::new();
    for tag in event.tags.iter() {
        let values = tag.clone().to_vec();
        if values.first().is_some_and(|value| value == "a") {
            let coordinate = values
                .get(1)
                .filter(|value| !value.is_empty())
                .ok_or_else(|| StorePageError::MalformedTag("a".into()))?;
            coordinates.push(coordinate.clone());
        }
    }
    Ok(coordinates)
}

fn singleton_tag(event: &Event, name: &'static str) -> Result<Option<String>, StorePageError> {
    let mut found = None;
    for tag in event.tags.iter() {
        let values = tag.clone().to_vec();
        if !values.first().is_some_and(|value| value == name) {
            continue;
        }
        if found.is_some() {
            return Err(StorePageError::DuplicateTag(name.into()));
        }
        if values.len() != 2 || values[1].is_empty() {
            return Err(StorePageError::MalformedTag(name.into()));
        }
        found = Some(values[1].clone());
    }
    Ok(found)
}

fn repeated_tags(event: &Event, name: &'static str) -> Result<Vec<String>, StorePageError> {
    let mut found = Vec::new();
    for tag in event.tags.iter() {
        let values = tag.clone().to_vec();
        if values.first().is_some_and(|value| value == name) {
            if values.len() != 2 || values[1].is_empty() {
                return Err(StorePageError::MalformedTag(name.into()));
            }
            found.push(values[1].clone());
        }
    }
    Ok(found)
}

fn parse_compact_tags(event: &Event) -> Result<StorePageCompactTags, StorePageError> {
    let mut languages = Vec::new();
    for tag in event.tags.iter() {
        let values = tag.clone().to_vec();
        if values.first().is_some_and(|value| value == "language") {
            if values.len() != 3 || values[1].is_empty() {
                return Err(StorePageError::MalformedTag("language".into()));
            }
            let capabilities = if values[2].is_empty() {
                HashSet::new()
            } else {
                values[2].split(',').collect::<HashSet<_>>()
            };
            if capabilities
                .iter()
                .any(|value| !matches!(*value, "interface" | "audio" | "subtitles"))
            {
                return Err(StorePageError::MalformedTag("language".into()));
            }
            languages.push(LanguageSupport {
                code: values[1].clone(),
                interface: capabilities.contains("interface"),
                audio: capabilities.contains("audio"),
                subtitles: capabilities.contains("subtitles"),
            });
        }
    }
    Ok(StorePageCompactTags {
        title: singleton_tag(event, "title")?,
        summary: singleton_tag(event, "summary")?,
        developer: singleton_tag(event, "developer")?,
        publisher: singleton_tag(event, "publisher")?,
        release_date: singleton_tag(event, "release_date")?,
        genres: repeated_tags(event, "genre")?,
        features: repeated_tags(event, "feature")?,
        languages,
        website: singleton_tag(event, "website")?,
        support: singleton_tag(event, "support")?,
    })
}

fn validate_compact_tags(compact: &StorePageCompactTags) -> Result<(), StorePageError> {
    ensure_optional_char_limit("title tag", &compact.title, MAX_TITLE_CHARS)?;
    ensure_optional_char_limit("summary tag", &compact.summary, MAX_SUMMARY_CHARS)?;
    for (field, value) in [
        ("developer tag", &compact.developer),
        ("publisher tag", &compact.publisher),
        ("release date tag", &compact.release_date),
    ] {
        ensure_optional_char_limit(field, value, MAX_TEXT_FIELD_CHARS)?;
    }
    ensure_count_limit("genre tags", compact.genres.len(), MAX_GENRES)?;
    ensure_count_limit("feature tags", compact.features.len(), MAX_FEATURES)?;
    ensure_count_limit("language tags", compact.languages.len(), MAX_LANGUAGES)?;
    for value in compact.genres.iter().chain(&compact.features) {
        ensure_char_limit("compact tag value", value, MAX_IDENTIFIER_CHARS)?;
    }
    for language in &compact.languages {
        ensure_char_limit("language tag", &language.code, MAX_IDENTIFIER_CHARS)?;
    }
    Ok(())
}

fn normalize(
    content: &StorePageContentV1,
    tags: &StorePageCompactTags,
) -> (NormalizedStorePage, Vec<StorePageDiagnostic>) {
    let mut diagnostics = Vec::new();
    macro_rules! preferred {
        ($content:expr, $tag:expr, $name:literal) => {{
            if $content.is_some() && $tag.is_some() && $content != $tag {
                diagnostics.push(StorePageDiagnostic::ContentOverridesTag($name));
            }
            $content.clone().or_else(|| $tag.clone())
        }};
    }
    let genres = preferred!(
        content.discovery.genres,
        (!tags.genres.is_empty()).then(|| tags.genres.clone()),
        "genres"
    )
    .unwrap_or_default();
    let features = preferred!(
        content.discovery.features,
        (!tags.features.is_empty()).then(|| tags.features.clone()),
        "features"
    )
    .unwrap_or_default();
    let languages = preferred!(
        content.languages,
        (!tags.languages.is_empty()).then(|| tags.languages.clone()),
        "languages"
    )
    .unwrap_or_default();
    (
        NormalizedStorePage {
            title: preferred!(content.basic.title, tags.title, "title"),
            summary: preferred!(content.basic.summary, tags.summary, "summary"),
            developer: preferred!(content.basic.developer, tags.developer, "developer"),
            publisher: preferred!(content.basic.publisher, tags.publisher, "publisher"),
            release_date: preferred!(
                content.basic.release_date,
                tags.release_date,
                "release_date"
            ),
            genres,
            features,
            languages,
            website: preferred!(content.links.website, tags.website, "website"),
            support: preferred!(content.links.support, tags.support, "support"),
        },
        diagnostics,
    )
}

fn append_compact_tags(
    tags: &mut Vec<Tag>,
    compact: &StorePageCompactTags,
) -> Result<(), StorePageError> {
    for (name, value) in [
        ("title", &compact.title),
        ("summary", &compact.summary),
        ("developer", &compact.developer),
        ("publisher", &compact.publisher),
        ("release_date", &compact.release_date),
        ("website", &compact.website),
        ("support", &compact.support),
    ] {
        if let Some(value) = value {
            tags.push(parse_tag([name, value.as_str()])?);
        }
    }
    for genre in &compact.genres {
        tags.push(parse_tag(["genre", genre.as_str()])?);
    }
    for feature in &compact.features {
        tags.push(parse_tag(["feature", feature.as_str()])?);
    }
    for language in &compact.languages {
        let mut capabilities = Vec::new();
        if language.interface {
            capabilities.push("interface");
        }
        if language.audio {
            capabilities.push("audio");
        }
        if language.subtitles {
            capabilities.push("subtitles");
        }
        let capabilities = capabilities.join(",");
        tags.push(parse_tag([
            "language",
            language.code.as_str(),
            capabilities.as_str(),
        ])?);
    }
    Ok(())
}

fn parse_tag<const N: usize>(values: [&str; N]) -> Result<Tag, StorePageError> {
    Tag::parse(values).map_err(|error| StorePageError::MalformedTag(error.to_string()))
}

#[cfg(test)]
mod tests {
    use nostr::{Keys, Kind, Timestamp};

    use super::*;

    fn coordinate(keys: &Keys, id: &str) -> String {
        format!(
            "{}:{}:{}",
            NIP99_LISTING_KIND,
            keys.public_key().to_hex(),
            id
        )
    }

    fn content(title: &str) -> StorePageContentV1 {
        StorePageContentV1 {
            basic: StorePageBasic {
                title: Some(title.into()),
                ..StorePageBasic::default()
            },
            ..StorePageContentV1::default()
        }
    }

    fn signed_event(
        keys: &Keys,
        presentation_id: &str,
        coordinates: Vec<String>,
        content: StorePageContentV1,
        created_at: u64,
    ) -> Event {
        build_store_page_event_builder(&StorePageBuildParams {
            publisher: keys.public_key(),
            presentation_id: presentation_id.into(),
            listing_coordinates: coordinates,
            content,
            compact_tags: StorePageCompactTags::default(),
        })
        .expect("valid builder")
        .custom_created_at(Timestamp::from(created_at))
        .sign_with_keys(keys)
        .expect("signed event")
    }

    fn raw_event(keys: &Keys, tags: Vec<Tag>, content: String, created_at: u64) -> Event {
        EventBuilder::new(Kind::Custom(EXPERIMENTAL_STORE_PAGE_KIND), content)
            .tags(tags)
            .custom_created_at(Timestamp::from(created_at))
            .sign_with_keys(keys)
            .expect("signed event")
    }

    #[test]
    fn builder_and_parser_support_multiple_owned_listings_and_content_precedence() {
        let keys = Keys::generate();
        let coordinates = vec![coordinate(&keys, "linux"), coordinate(&keys, "windows")];
        let event = build_store_page_event_builder(&StorePageBuildParams {
            publisher: keys.public_key(),
            presentation_id: "game".into(),
            listing_coordinates: coordinates.clone(),
            content: content("Content title"),
            compact_tags: StorePageCompactTags {
                title: Some("Tag title".into()),
                genres: vec!["tag-genre".into()],
                ..StorePageCompactTags::default()
            },
        })
        .expect("valid builder")
        .sign_with_keys(&keys)
        .expect("signed event");

        let parsed = parse_store_page_event(&event).expect("valid Store Page");
        assert_eq!(parsed.listing_coordinates, coordinates);
        assert_eq!(parsed.normalized.title.as_deref(), Some("Content title"));
        assert_eq!(parsed.normalized.genres, vec!["tag-genre"]);
        assert!(parsed
            .diagnostics
            .contains(&StorePageDiagnostic::ContentOverridesTag("title")));
    }

    #[test]
    fn parser_ignores_unknown_json_fields() {
        let keys = Keys::generate();
        let json =
            format!(r#"{{"schema":"{STORE_PAGE_SCHEMA}","version":1,"future":{{"value":true}}}}"#);
        let event = raw_event(
            &keys,
            vec![
                parse_tag(["d", "game"]).expect("d tag"),
                parse_tag(["a", coordinate(&keys, "linux").as_str()]).expect("a tag"),
            ],
            json,
            1,
        );
        assert!(parse_store_page_event(&event).is_ok());
    }

    #[test]
    fn parser_rejects_missing_or_duplicate_identity_tags() {
        let keys = Keys::generate();
        let json = serde_json::to_string(&content("Title")).expect("content JSON");
        let a = coordinate(&keys, "linux");
        let missing_d = raw_event(
            &keys,
            vec![parse_tag(["a", a.as_str()]).expect("a tag")],
            json.clone(),
            1,
        );
        assert_eq!(
            parse_store_page_event(&missing_d),
            Err(StorePageError::MissingTag("d"))
        );
        let duplicate_d = raw_event(
            &keys,
            vec![
                parse_tag(["d", "one"]).expect("d tag"),
                parse_tag(["d", "two"]).expect("d tag"),
                parse_tag(["a", a.as_str()]).expect("a tag"),
            ],
            json,
            1,
        );
        assert_eq!(
            parse_store_page_event(&duplicate_d),
            Err(StorePageError::DuplicateTag("d".into()))
        );

        let missing_a = raw_event(
            &keys,
            vec![parse_tag(["d", "game"]).expect("d tag")],
            serde_json::to_string(&content("Title")).expect("content JSON"),
            1,
        );
        assert_eq!(
            parse_store_page_event(&missing_a),
            Err(StorePageError::MissingTag("a"))
        );
    }

    #[test]
    fn parser_rejects_wrong_kind_and_invalid_signature() {
        let keys = Keys::generate();
        let association = coordinate(&keys, "game");
        let tags = vec![
            parse_tag(["d", "game"]).expect("d tag"),
            parse_tag(["a", association.as_str()]).expect("a tag"),
        ];
        let json = serde_json::to_string(&content("Title")).expect("content JSON");
        let wrong_kind = EventBuilder::new(Kind::Custom(NIP99_LISTING_KIND), json)
            .tags(tags)
            .sign_with_keys(&keys)
            .expect("signed event");
        assert_eq!(
            parse_store_page_event(&wrong_kind),
            Err(StorePageError::WrongKind {
                expected: EXPERIMENTAL_STORE_PAGE_KIND,
                found: NIP99_LISTING_KIND,
            })
        );

        let mut tampered = signed_event(&keys, "game", vec![association], content("Original"), 1);
        tampered.content = serde_json::to_string(&content("Tampered")).expect("content JSON");
        assert_eq!(
            parse_store_page_event(&tampered),
            Err(StorePageError::InvalidSignature)
        );
    }

    #[test]
    fn parser_rejects_wrong_listing_kind_foreign_author_and_duplicate_associations() {
        let keys = Keys::generate();
        let foreign = Keys::generate();
        let json = serde_json::to_string(&content("Title")).expect("content JSON");
        for (association, expected) in [
            (
                format!("30078:{}:game", keys.public_key().to_hex()),
                StorePageError::MalformedTag("a".into()),
            ),
            (coordinate(&foreign, "game"), StorePageError::WrongPublisher),
        ] {
            let event = raw_event(
                &keys,
                vec![
                    parse_tag(["d", "game"]).expect("d tag"),
                    parse_tag(["a", association.as_str()]).expect("a tag"),
                ],
                json.clone(),
                1,
            );
            assert_eq!(parse_store_page_event(&event), Err(expected));
        }

        let association = coordinate(&keys, "game");
        let duplicate = raw_event(
            &keys,
            vec![
                parse_tag(["d", "game"]).expect("d tag"),
                parse_tag(["a", association.as_str()]).expect("a tag"),
                parse_tag(["a", association.as_str()]).expect("a tag"),
            ],
            json,
            1,
        );
        assert_eq!(
            parse_store_page_event(&duplicate),
            Err(StorePageError::DuplicateTag("a".into()))
        );
    }

    #[test]
    fn unsupported_version_is_presentation_only_failure() {
        let keys = Keys::generate();
        let mut unsupported = content("Future");
        unsupported.version = STORE_PAGE_SCHEMA_VERSION + 1;
        let event = raw_event(
            &keys,
            vec![
                parse_tag(["d", "game"]).expect("d tag"),
                parse_tag(["a", coordinate(&keys, "game").as_str()]).expect("a tag"),
            ],
            serde_json::to_string(&unsupported).expect("content JSON"),
            1,
        );
        assert_eq!(
            parse_store_page_event(&event),
            Err(StorePageError::UnsupportedSchemaVersion(
                STORE_PAGE_SCHEMA_VERSION + 1
            ))
        );
    }

    #[test]
    fn invalid_newer_event_does_not_replace_valid_page() {
        let keys = Keys::generate();
        let association = coordinate(&keys, "game");
        let valid = signed_event(
            &keys,
            "game",
            vec![association.clone()],
            content("Valid"),
            10,
        );
        let mut future = content("Unsupported");
        future.version += 1;
        let invalid = raw_event(
            &keys,
            vec![
                parse_tag(["d", "game"]).expect("d tag"),
                parse_tag(["a", association.as_str()]).expect("a tag"),
            ],
            serde_json::to_string(&future).expect("content JSON"),
            20,
        );

        let resolved = resolve_store_page_events([&valid, &invalid], keys.public_key(), "game")
            .expect("valid page remains");
        assert_eq!(resolved.event.id, valid.id);
    }

    #[test]
    fn resolver_uses_central_equal_timestamp_event_id_tie_break() {
        let keys = Keys::generate();
        let association = coordinate(&keys, "game");
        let first = signed_event(&keys, "game", vec![association.clone()], content("A"), 10);
        let second = signed_event(&keys, "game", vec![association], content("B"), 10);
        let expected = if first.id.to_hex() < second.id.to_hex() {
            first.id
        } else {
            second.id
        };
        let resolved = resolve_store_page_events([&first, &second], keys.public_key(), "game")
            .expect("resolved page");
        assert_eq!(resolved.event.id, expected);
    }

    #[test]
    fn parser_rejects_duplicate_media_ids() {
        let keys = Keys::generate();
        let media = StorePageMediaItem {
            id: "hero".into(),
            media_type: "image".into(),
            role: "hero".into(),
            url: "https://example.com/hero.webp".into(),
            thumbnail_url: None,
            alt: None,
            caption: None,
            width: None,
            height: None,
        };
        let mut duplicate = content("Title");
        duplicate.media = vec![media.clone(), media];
        let event = raw_event(
            &keys,
            vec![
                parse_tag(["d", "game"]).expect("d tag"),
                parse_tag(["a", coordinate(&keys, "game").as_str()]).expect("a tag"),
            ],
            serde_json::to_string(&duplicate).expect("content JSON"),
            1,
        );
        assert!(matches!(
            parse_store_page_event(&event),
            Err(StorePageError::InvalidContent(_))
        ));
    }

    #[test]
    fn parser_omits_invalid_optional_urls_and_unknown_video_formats() {
        let keys = Keys::generate();
        let mut page = content("Title");
        page.description_markdown = concat!(
            "[unsafe](javascript:alert(1)) ",
            "[safe](https://example.com/page)"
        )
        .into();
        page.links.website = Some("https://example.com".into());
        page.links.support = Some("file:///tmp/support".into());
        page.media = vec![
            StorePageMediaItem {
                thumbnail_url: Some("data:image/png;base64,AAAA".into()),
                ..media_item(
                    "image",
                    "image",
                    "hero",
                    "https://cdn.example.com/hero.webp",
                )
            },
            media_item(
                "video",
                "video",
                "trailer",
                "https://cdn.example.com/trailer.webm",
            ),
            media_item(
                "unsafe-image",
                "image",
                "screenshot",
                "data:image/png;base64,AAAA",
            ),
            media_item(
                "unknown-video",
                "video",
                "feature",
                "https://cdn.example.com/trailer.mov",
            ),
        ];
        let event = signed_event(&keys, "game", vec![coordinate(&keys, "game")], page, 1);

        let parsed = parse_store_page_event(&event).expect("valid page with optional omissions");
        assert_eq!(parsed.sanitized_content.media.len(), 2);
        assert_eq!(parsed.sanitized_content.media[0].thumbnail_url, None);
        assert_eq!(
            parsed.sanitized_content.links.website.as_deref(),
            Some("https://example.com/")
        );
        assert_eq!(parsed.sanitized_content.links.support, None);
        assert!(parsed
            .sanitized_content
            .description_html
            .as_str()
            .contains("href=\"https://example.com/page\""));
        assert!(!parsed
            .sanitized_content
            .description_html
            .as_str()
            .to_ascii_lowercase()
            .contains("javascript:"));
        assert!(parsed.diagnostics.iter().any(|diagnostic| matches!(
            diagnostic,
            StorePageDiagnostic::OptionalFieldOmitted {
                reason: OptionalFieldReason::UnsupportedVideoFormat,
                ..
            }
        )));
    }

    #[test]
    fn invalid_content_link_does_not_fall_back_to_compact_tag() {
        let keys = Keys::generate();
        let mut page = content("Title");
        page.links.website = Some("javascript:alert(1)".into());
        let event = build_store_page_event_builder(&StorePageBuildParams {
            publisher: keys.public_key(),
            presentation_id: "game".into(),
            listing_coordinates: vec![coordinate(&keys, "game")],
            content: page,
            compact_tags: StorePageCompactTags {
                website: Some("https://safe.example.com".into()),
                ..StorePageCompactTags::default()
            },
        })
        .expect("optional invalid URL does not reject builder")
        .sign_with_keys(&keys)
        .expect("signed event");

        let parsed = parse_store_page_event(&event).expect("valid page");
        assert_eq!(parsed.normalized.website, None);
        assert_eq!(parsed.sanitized_content.links.website, None);
    }

    #[test]
    fn parser_rejects_oversized_event_content_before_json_parsing() {
        let keys = Keys::generate();
        let event = raw_event(
            &keys,
            vec![
                parse_tag(["d", "game"]).expect("d tag"),
                parse_tag(["a", coordinate(&keys, "game").as_str()]).expect("a tag"),
            ],
            "x".repeat(MAX_EVENT_CONTENT_BYTES + 1),
            1,
        );
        assert!(matches!(
            parse_store_page_event(&event),
            Err(StorePageError::ContentPolicy(
                ContentPolicyError::LimitExceeded { .. }
            ))
        ));
    }

    #[test]
    fn parser_rejects_oversized_serialized_event_without_buffering_it() {
        let keys = Keys::generate();
        let event = raw_event(
            &keys,
            vec![
                parse_tag(["d", "game"]).expect("d tag"),
                parse_tag(["a", coordinate(&keys, "game").as_str()]).expect("a tag"),
                parse_tag(["title", &"x".repeat(MAX_EVENT_BYTES)]).expect("title tag"),
            ],
            serde_json::to_string(&content("Title")).expect("content JSON"),
            1,
        );
        assert!(matches!(
            parse_store_page_event(&event),
            Err(StorePageError::ContentPolicy(
                ContentPolicyError::LimitExceeded { .. }
            ))
        ));
    }

    #[test]
    fn parser_rejects_excessive_media_and_section_counts() {
        let keys = Keys::generate();
        let tags = || {
            vec![
                parse_tag(["d", "game"]).expect("d tag"),
                parse_tag(["a", coordinate(&keys, "game").as_str()]).expect("a tag"),
            ]
        };

        let mut excessive_media = content("Title");
        excessive_media.media = (0..=MAX_MEDIA_ITEMS)
            .map(|index| {
                media_item(
                    &format!("image-{index}"),
                    "image",
                    "feature",
                    "https://cdn.example.com/image.webp",
                )
            })
            .collect();
        let event = raw_event(
            &keys,
            tags(),
            serde_json::to_string(&excessive_media).expect("content JSON"),
            1,
        );
        assert!(matches!(
            parse_store_page_event(&event),
            Err(StorePageError::ContentPolicy(
                ContentPolicyError::LimitExceeded { .. }
            ))
        ));

        let mut excessive_sections = content("Title");
        excessive_sections.sections = (0..=MAX_FEATURE_SECTIONS)
            .map(|index| StorePageSection {
                id: format!("section-{index}"),
                heading: "Heading".into(),
                body_markdown: "Body".into(),
                media_id: None,
                layout: "text".into(),
            })
            .collect();
        let event = raw_event(
            &keys,
            tags(),
            serde_json::to_string(&excessive_sections).expect("content JSON"),
            1,
        );
        assert!(matches!(
            parse_store_page_event(&event),
            Err(StorePageError::ContentPolicy(
                ContentPolicyError::LimitExceeded { .. }
            ))
        ));
    }

    #[test]
    fn builder_rejects_oversized_title() {
        let keys = Keys::generate();
        let mut page = content("Title");
        page.basic.title = Some("x".repeat(MAX_TITLE_CHARS + 1));
        assert!(matches!(
            build_store_page_event_builder(&StorePageBuildParams {
                publisher: keys.public_key(),
                presentation_id: "game".into(),
                listing_coordinates: vec![coordinate(&keys, "game")],
                content: page,
                compact_tags: StorePageCompactTags::default(),
            }),
            Err(StorePageError::ContentPolicy(
                ContentPolicyError::LimitExceeded { .. }
            ))
        ));
    }

    #[test]
    fn listing_pointer_parser_validates_coordinate_publisher_and_relay_hint() {
        let keys = Keys::generate();
        let coordinate = store_page_coordinate(keys.public_key(), "game-page");
        let listing = signed_listing(
            &keys,
            "game",
            vec![vec![
                "store_page".into(),
                coordinate.clone(),
                "wss://relay.example.com".into(),
            ]],
        );
        let report = parse_store_page_pointers(&listing).expect("valid listing");
        let pointer = report.active.expect("active pointer");
        assert_eq!(pointer.coordinate, coordinate);
        assert_eq!(pointer.presentation_id, "game-page");
        assert_eq!(
            pointer.relay_hint.as_deref(),
            Some("wss://relay.example.com/")
        );
        assert!(report.diagnostics.is_empty());
    }

    #[test]
    fn listing_pointer_parser_rejects_unsafe_advisory_relays() {
        let keys = Keys::generate();
        let coordinate = store_page_coordinate(keys.public_key(), "game-page");
        for relay in [
            "ws://relay.example.com",
            "wss://localhost:8080",
            "wss://127.0.0.1",
            "wss://10.0.0.1",
            "wss://[::1]",
        ] {
            let listing = signed_listing(
                &keys,
                "game",
                vec![vec!["store_page".into(), coordinate.clone(), relay.into()]],
            );
            let report = parse_store_page_pointers(&listing).expect("valid signed listing");
            assert_eq!(report.active, None, "unsafe relay accepted: {relay}");
            assert!(report.diagnostics.iter().any(|diagnostic| matches!(
                diagnostic,
                StorePagePointerDiagnostic::MalformedTag {
                    reason: StorePagePointerMalformedReason::InvalidRelayHint,
                    ..
                }
            )));
        }
    }

    #[test]
    fn publisher_draft_supports_create_edit_clone_and_multiple_listings() {
        let keys = Keys::generate();
        let first = coordinate(&keys, "first");
        let second = coordinate(&keys, "second");
        let mut draft = StorePageDraft::new("page".to_string(), vec![first, second]);
        draft.content = content("Arcade One");
        draft.loaded_event_id = Some("loaded-event".to_string());

        let validated = validate_store_page_draft(&draft.build_params(keys.public_key()))
            .expect("publisher draft should validate");
        assert_eq!(validated.normalized.title.as_deref(), Some("Arcade One"));

        let cloned = clone_store_page_draft(&draft, "page-v2".to_string());
        assert_eq!(cloned.presentation_id, "page-v2");
        assert!(cloned.listing_coordinates.is_empty());
        assert_eq!(cloned.loaded_event_id, None);
        assert_eq!(cloned.content, draft.content);
    }

    #[test]
    fn publisher_draft_rejects_listing_owned_by_another_publisher() {
        let publisher = Keys::generate();
        let foreign = Keys::generate();
        let mut draft =
            StorePageDraft::new("page".to_string(), vec![coordinate(&foreign, "foreign")]);
        draft.content = content("Foreign");

        assert_eq!(
            validate_store_page_draft(&draft.build_params(publisher.public_key())),
            Err(StorePageError::WrongPublisher)
        );
    }

    #[test]
    fn pointer_replacement_preserves_listing_and_creates_reciprocal_association() {
        let keys = Keys::generate();
        let page_coordinate = store_page_coordinate(keys.public_key(), "page");
        let listing = EventBuilder::new(Kind::Custom(NIP99_LISTING_KIND), "signed commerce")
            .tags([
                parse_tag(["d", "game"]).expect("d tag"),
                parse_tag(["price", "19.99", "USD"]).expect("price tag"),
                parse_tag(["store_page", "malformed-old-pointer"]).expect("old pointer tag"),
            ])
            .sign_with_keys(&keys)
            .expect("signed listing");
        let replacement = build_listing_store_page_replacement(
            &listing,
            keys.public_key(),
            &StorePagePointerAction::Link {
                store_page_coordinate: page_coordinate,
                relay_hint: None,
            },
        )
        .expect("pointer replacement")
        .sign_with_keys(&keys)
        .expect("signed replacement");
        let listing_coordinate = listing_coordinate(&replacement).expect("listing coordinate");
        let page = build_store_page_event_builder(&StorePageBuildParams {
            publisher: keys.public_key(),
            presentation_id: "page".to_string(),
            listing_coordinates: vec![listing_coordinate],
            content: content("Arcade One"),
            compact_tags: StorePageCompactTags::default(),
        })
        .expect("page builder")
        .sign_with_keys(&keys)
        .expect("signed page");
        let parsed = parse_store_page_event(&page).expect("parsed page");

        validate_store_page_association(&replacement, &parsed).expect("reciprocal association");
        assert_eq!(replacement.content, listing.content);
        assert!(replacement.tags.iter().any(|tag| {
            let values = tag.clone().to_vec();
            values.as_slice() == ["price", "19.99", "USD"]
        }));
        assert_eq!(
            replacement
                .tags
                .iter()
                .filter(|tag| (*tag)
                    .clone()
                    .to_vec()
                    .first()
                    .is_some_and(|name| name == "store_page"))
                .count(),
            1
        );
    }

    #[test]
    fn optimistic_revision_checks_reject_stale_page_and_listing() {
        assert_eq!(
            validate_store_page_revision(Some("loaded"), Some("newer")),
            Err(StorePagePublishError::StaleStorePage)
        );
        assert_eq!(
            validate_store_page_revision(None, Some("existing")),
            Err(StorePagePublishError::StorePageAlreadyExists)
        );
        assert_eq!(
            validate_listing_revision("30402:publisher:game", "loaded", "newer"),
            Err(StorePagePublishError::StaleListing(
                "30402:publisher:game".to_string()
            ))
        );
    }

    #[test]
    fn listing_pointer_parser_reports_malformed_wrong_kind_publisher_and_relay() {
        let keys = Keys::generate();
        let foreign = Keys::generate();
        let tags = vec![
            vec!["store_page".into(), "not-a-coordinate".into()],
            vec![
                "store_page".into(),
                format!("{}:{}:page", NIP99_LISTING_KIND, keys.public_key().to_hex()),
            ],
            vec![
                "store_page".into(),
                store_page_coordinate(foreign.public_key(), "page"),
            ],
            vec![
                "store_page".into(),
                store_page_coordinate(keys.public_key(), "page"),
                "https://not-a-relay.example.com".into(),
            ],
        ];
        let report = parse_store_page_pointers(&signed_listing(&keys, "game", tags))
            .expect("listing remains valid");
        assert_eq!(report.active, None);
        assert_eq!(report.pointer_tag_count, 4);
        for reason in [
            StorePagePointerMalformedReason::MalformedCoordinate,
            StorePagePointerMalformedReason::WrongKind,
            StorePagePointerMalformedReason::WrongPublisher,
            StorePagePointerMalformedReason::InvalidRelayHint,
        ] {
            assert!(report.diagnostics.iter().any(|diagnostic| matches!(
                diagnostic,
                StorePagePointerDiagnostic::MalformedTag {
                    reason: actual,
                    ..
                } if actual == &reason
            )));
        }
    }

    #[test]
    fn duplicate_or_conflicting_valid_pointers_have_no_active_association() {
        let keys = Keys::generate();
        let first = store_page_coordinate(keys.public_key(), "first");
        let duplicate = signed_listing(
            &keys,
            "game",
            vec![
                vec!["store_page".into(), first.clone()],
                vec!["store_page".into(), first.clone()],
            ],
        );
        let report = parse_store_page_pointers(&duplicate).expect("valid listing");
        assert_eq!(report.active, None);
        assert!(report
            .diagnostics
            .contains(&StorePagePointerDiagnostic::DuplicatePointers));

        let conflicting = signed_listing(
            &keys,
            "game",
            vec![
                vec!["store_page".into(), first],
                vec![
                    "store_page".into(),
                    store_page_coordinate(keys.public_key(), "second"),
                ],
            ],
        );
        let report = parse_store_page_pointers(&conflicting).expect("valid listing");
        assert_eq!(report.active, None);
        assert!(report
            .diagnostics
            .contains(&StorePagePointerDiagnostic::ConflictingPointers));

        let mixed = signed_listing(
            &keys,
            "game",
            vec![
                vec![
                    "store_page".into(),
                    store_page_coordinate(keys.public_key(), "valid"),
                ],
                vec!["store_page".into(), "malformed".into()],
            ],
        );
        let report = parse_store_page_pointers(&mixed).expect("valid listing");
        assert_eq!(report.active, None);
        assert!(report.diagnostics.iter().any(|diagnostic| matches!(
            diagnostic,
            StorePagePointerDiagnostic::MalformedTag { .. }
        )));
    }

    #[test]
    fn reciprocal_association_requires_both_sides_and_supports_multiple_listings() {
        let keys = Keys::generate();
        let linux = signed_listing(
            &keys,
            "linux",
            vec![vec![
                "store_page".into(),
                store_page_coordinate(keys.public_key(), "game-page"),
            ]],
        );
        let windows = signed_listing(
            &keys,
            "windows",
            vec![vec![
                "store_page".into(),
                store_page_coordinate(keys.public_key(), "game-page"),
            ]],
        );
        let page = signed_event(
            &keys,
            "game-page",
            vec![
                listing_coordinate(&linux).expect("linux coordinate"),
                listing_coordinate(&windows).expect("windows coordinate"),
            ],
            content("Title"),
            1,
        );
        let parsed = parse_store_page_event(&page).expect("valid page");
        assert!(validate_store_page_association(&linux, &parsed).is_ok());
        assert!(validate_store_page_association(&windows, &parsed).is_ok());

        let no_pointer = signed_listing(&keys, "linux", Vec::new());
        assert_eq!(
            validate_store_page_association(&no_pointer, &parsed),
            Err(StorePageAssociationError::NoActivePointer)
        );

        let other_listing = signed_listing(
            &keys,
            "other",
            vec![vec![
                "store_page".into(),
                store_page_coordinate(keys.public_key(), "game-page"),
            ]],
        );
        assert_eq!(
            validate_store_page_association(&other_listing, &parsed),
            Err(StorePageAssociationError::MissingReciprocalReference)
        );
    }

    #[test]
    fn parser_falls_back_unknown_section_layout_and_missing_media_reference() {
        let keys = Keys::generate();
        let mut page = content("Title");
        page.sections.push(StorePageSection {
            id: "section".into(),
            heading: "Heading".into(),
            body_markdown: "Safe body".into(),
            media_id: Some("missing".into()),
            layout: "publisher-layout".into(),
        });
        let event = signed_event(&keys, "game", vec![coordinate(&keys, "game")], page, 1);
        let parsed = parse_store_page_event(&event).expect("valid page");
        assert_eq!(parsed.sanitized_content.sections[0].layout, "text");
        assert_eq!(parsed.sanitized_content.sections[0].media_id, None);
        assert!(parsed.diagnostics.iter().any(|diagnostic| matches!(
            diagnostic,
            StorePageDiagnostic::OptionalFieldOmitted {
                reason: OptionalFieldReason::UnsupportedLayout,
                ..
            }
        )));
    }

    fn media_item(id: &str, media_type: &str, role: &str, url: &str) -> StorePageMediaItem {
        StorePageMediaItem {
            id: id.into(),
            media_type: media_type.into(),
            role: role.into(),
            url: url.into(),
            thumbnail_url: None,
            alt: None,
            caption: None,
            width: None,
            height: None,
        }
    }

    fn signed_listing(keys: &Keys, id: &str, pointer_tags: Vec<Vec<String>>) -> Event {
        let mut tags = vec![parse_tag(["d", id]).expect("d tag")];
        tags.extend(
            pointer_tags
                .into_iter()
                .map(|values| Tag::parse(values).expect("pointer tag")),
        );
        EventBuilder::new(Kind::Custom(NIP99_LISTING_KIND), "listing")
            .tags(tags)
            .sign_with_keys(keys)
            .expect("signed listing")
    }
}
