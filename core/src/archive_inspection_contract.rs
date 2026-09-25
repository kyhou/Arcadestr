//! WASM-safe contracts for publisher archive inspection commands.
//!
//! These DTOs expose validated archive-relative metadata and opaque handles.
//! They deliberately cannot carry native paths, archive bytes, or an archive
//! index. Native inspection implementations translate into this contract at
//! the IPC boundary.

use std::{fmt, str::FromStr};

use serde::{de, Deserialize, Deserializer, Serialize};

use crate::distribution::{PackageFormat, PlatformTarget, RelativePath, Sha256Digest, WorkingDir};

/// Default number of immutable snapshot records returned by a query.
pub const ARCHIVE_PAGE_SIZE_DEFAULT: u16 = 100;

/// Hard maximum number of immutable snapshot records returned by a query.
pub const ARCHIVE_PAGE_SIZE_MAX: u16 = 200;

/// Maximum Unicode character count accepted by full-path archive search.
pub const ARCHIVE_SEARCH_MAX_CHARACTERS: usize = 200;

/// Failure to parse a canonical opaque archive identifier.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ArchiveIdError;

impl fmt::Display for ArchiveIdError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("identifier must be a canonical lowercase UUID v4")
    }
}

impl std::error::Error for ArchiveIdError {}

macro_rules! archive_id {
    ($name:ident, $description:literal) => {
        #[doc = $description]
        #[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            /// Parses a canonical lowercase UUID v4 identifier.
            pub fn parse(value: &str) -> Result<Self, ArchiveIdError> {
                if is_canonical_uuid_v4(value) {
                    Ok(Self(value.to_owned()))
                } else {
                    Err(ArchiveIdError)
                }
            }

            /// Returns the validated identifier text.
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl FromStr for $name {
            type Err = ArchiveIdError;

            fn from_str(value: &str) -> Result<Self, Self::Err> {
                Self::parse(value)
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: Deserializer<'de>,
            {
                let value = String::deserialize(deserializer)?;
                Self::parse(&value).map_err(de::Error::custom)
            }
        }
    };
}

archive_id!(
    ArchiveSelectionId,
    "An opaque account-bound archive selection handle."
);
archive_id!(
    ArchiveRequestId,
    "An opaque identifier used to correlate one archive operation."
);

/// A display-only archive basename that cannot encode a native path.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct SafeArchiveFilename(String);

impl SafeArchiveFilename {
    /// Parses a non-empty filename without separators or control characters.
    pub fn parse(value: &str) -> Result<Self, SafeArchiveFilenameError> {
        if value.is_empty()
            || value.chars().count() > 255
            || value
                .chars()
                .any(|character| character == '/' || character == '\\' || character.is_control())
        {
            return Err(SafeArchiveFilenameError);
        }
        Ok(Self(value.to_owned()))
    }

    /// Returns the safe display filename.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl<'de> Deserialize<'de> for SafeArchiveFilename {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::parse(&value).map_err(de::Error::custom)
    }
}

/// Failure to parse a display-safe archive filename.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SafeArchiveFilenameError;

impl fmt::Display for SafeArchiveFilenameError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("archive filename must be a safe basename")
    }
}

impl std::error::Error for SafeArchiveFilenameError {}

fn is_canonical_uuid_v4(value: &str) -> bool {
    let bytes = value.as_bytes();
    if bytes.len() != 36
        || bytes[8] != b'-'
        || bytes[13] != b'-'
        || bytes[18] != b'-'
        || bytes[23] != b'-'
        || bytes[14] != b'4'
        || !matches!(bytes[19], b'8' | b'9' | b'a' | b'b')
    {
        return false;
    }

    bytes.iter().enumerate().all(|(position, byte)| {
        matches!(position, 8 | 13 | 18 | 23) || byte.is_ascii_digit() || matches!(byte, b'a'..=b'f')
    })
}

/// Invalid page-size input.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ArchivePageLimitError {
    pub requested: u16,
    pub maximum: u16,
}

impl fmt::Display for ArchivePageLimitError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "page size {} is outside 1..={}",
            self.requested, self.maximum
        )
    }
}

impl std::error::Error for ArchivePageLimitError {}

/// A stable offset into an immutable inspection snapshot.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct ArchivePageRequest {
    offset: u64,
    limit: u16,
}

/// A bounded case-insensitive search over the complete decoded archive path.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct ArchiveSearch(String);

impl ArchiveSearch {
    pub fn parse(value: &str) -> Result<Self, ArchiveSearchError> {
        if value.chars().count() > ARCHIVE_SEARCH_MAX_CHARACTERS {
            return Err(ArchiveSearchError::TooLong);
        }
        if value.chars().any(char::is_control) {
            return Err(ArchiveSearchError::ControlCharacter);
        }
        Ok(Self(value.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl<'de> Deserialize<'de> for ArchiveSearch {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::parse(&value).map_err(de::Error::custom)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArchiveSearchError {
    TooLong,
    ControlCharacter,
}

impl fmt::Display for ArchiveSearchError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooLong => formatter.write_str("archive search exceeds 200 characters"),
            Self::ControlCharacter => {
                formatter.write_str("archive search contains a control character")
            }
        }
    }
}

impl std::error::Error for ArchiveSearchError {}

impl ArchivePageRequest {
    /// Creates a bounded immutable-snapshot page request.
    pub fn new(offset: u64, limit: u16) -> Result<Self, ArchivePageLimitError> {
        let request = Self { offset, limit };
        request.validate().map_err(|_| ArchivePageLimitError {
            requested: limit,
            maximum: ARCHIVE_PAGE_SIZE_MAX,
        })?;
        Ok(request)
    }

    /// Validates a request decoded at a transport boundary.
    pub fn validate(self) -> Result<Self, ArchivePaginationFailure> {
        if self.limit == 0 {
            return Err(ArchivePaginationFailure::ZeroLimit);
        }
        if self.limit > ARCHIVE_PAGE_SIZE_MAX {
            return Err(ArchivePaginationFailure::LimitExceeded {
                requested: self.limit,
                maximum: ARCHIVE_PAGE_SIZE_MAX,
            });
        }
        Ok(self)
    }

    /// Returns the stable zero-based snapshot offset.
    pub fn offset(self) -> u64 {
        self.offset
    }

    /// Returns the page size. Transport-decoded requests must be validated before use.
    pub fn limit(self) -> u16 {
        self.limit
    }
}

impl Default for ArchivePageRequest {
    fn default() -> Self {
        Self {
            offset: 0,
            limit: ARCHIVE_PAGE_SIZE_DEFAULT,
        }
    }
}

impl<'de> Deserialize<'de> for ArchivePageRequest {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct WirePageRequest {
            offset: u64,
            limit: u16,
        }

        let request = WirePageRequest::deserialize(deserializer)?;
        Ok(Self {
            offset: request.offset,
            limit: request.limit,
        })
    }
}

/// One bounded page from an immutable inspection snapshot.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArchivePage<T> {
    pub offset: u64,
    pub total: u64,
    pub next_offset: Option<u64>,
    pub items: Vec<T>,
}

/// Counts available without transferring snapshot records.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArchiveInspectionCounts {
    pub logical_entries: u64,
    pub indexed_entries: u64,
    pub undecodable_entries: u64,
    pub files: u64,
    pub directory_entries: u64,
    pub symlinks: u64,
    pub hardlinks: u64,
    pub other_entry_kinds: u64,
    pub working_directories: u64,
    pub hazards: ArchiveHazardCounts,
    pub warnings: ArchiveWarningCounts,
}

/// Counts for each blocking hazard category.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArchiveHazardCounts {
    pub path_rule: u64,
    pub symlink: u64,
    pub hardlink: u64,
    pub collision: u64,
    pub other: u64,
    pub undecodable: u64,
    pub type_conflict: u64,
    pub zip_count_mismatch: u64,
}

/// Counts for each non-blocking warning category.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArchiveWarningCounts {
    pub efs_divergence: u64,
    pub extension_mismatch: u64,
}

/// Safe metadata available immediately after native file selection.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArchiveFileSelection {
    pub selection_id: ArchiveSelectionId,
    pub safe_filename: SafeArchiveFilename,
    pub file_size: u64,
    pub expires_at_unix: u64,
}

/// Safe publisher-facing metadata after native archive inspection.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArchiveSelectionSummary {
    pub selection: ArchiveFileSelection,
    pub detected_format: PackageFormat,
    pub extension_format: Option<PackageFormat>,
    pub extension_mismatch_warning: Option<ExtensionMismatchWarningDetail>,
    pub counts: ArchiveInspectionCounts,
    pub blocking_hazards: bool,
}

/// Selects an archive and binds its opaque handle to an account.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SelectArchiveRequest {
    pub request_id: ArchiveRequestId,
    pub expected_publisher_npub: String,
}

/// Result of archive selection; cancellation returns no selection.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SelectArchiveResponse {
    pub selection: Option<ArchiveFileSelection>,
}

/// Requests inspection of a previously selected immutable snapshot.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct InspectArchiveRequest {
    pub request_id: ArchiveRequestId,
    pub selection_id: ArchiveSelectionId,
    pub expected_publisher_npub: String,
    pub expected_format: Option<PackageFormat>,
}

/// Returns the current safe summary for an inspected selection.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct InspectArchiveResponse {
    pub summary: ArchiveSelectionSummary,
}

/// Requests SHA-256 hashing of the selected, mutation-checked archive.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HashSelectedArchiveRequest {
    pub request_id: ArchiveRequestId,
    pub selection_id: ArchiveSelectionId,
    pub expected_publisher_npub: String,
}

/// Hash result for the selected immutable archive snapshot.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HashSelectedArchiveResponse {
    pub selection_id: ArchiveSelectionId,
    pub sha256: Sha256Digest,
}

/// A validated regular file selectable as a launch executable.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SelectableArchiveFile {
    pub path: RelativePath,
    pub declared_size: u64,
}

/// A validated working-directory choice, including the archive root.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SelectableWorkingDirectory {
    pub directory: WorkingDir,
}

/// A3.3 candidate confidence copied into the unconditional contract.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArchiveCandidateConfidence {
    High,
    Medium,
    Low,
}

/// A3.3 candidate reason copied into the unconditional contract.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "code", content = "details", rename_all = "snake_case")]
pub enum ArchiveCandidateReason {
    GodotPackageBesideBinary,
    UnityDataDirectoryBesideBinary,
    UnrealWin64Layout,
    WindowsExecutableExtension { archive_root: bool },
    UnixExecutableMode { archive_root: bool },
}

/// An advisory executable candidate with no automatic-selection state.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArchiveCandidate {
    pub path: RelativePath,
    pub confidence: ArchiveCandidateConfidence,
    pub reason: ArchiveCandidateReason,
}

macro_rules! page_query {
    ($request:ident, $response:ident, $item:ty) => {
        #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
        pub struct $request {
            pub request_id: ArchiveRequestId,
            pub selection_id: ArchiveSelectionId,
            pub expected_publisher_npub: String,
            pub search: ArchiveSearch,
            pub page: ArchivePageRequest,
        }

        #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
        pub struct $response {
            pub page: ArchivePage<$item>,
        }
    };
}

page_query!(
    QueryArchiveFilesRequest,
    QueryArchiveFilesResponse,
    SelectableArchiveFile
);
page_query!(
    QueryArchiveWorkingDirectoriesRequest,
    QueryArchiveWorkingDirectoriesResponse,
    SelectableWorkingDirectory
);

/// Queries advisory candidates for exactly one normalized platform target.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueryArchiveCandidatesRequest {
    pub request_id: ArchiveRequestId,
    pub selection_id: ArchiveSelectionId,
    pub expected_publisher_npub: String,
    pub search: ArchiveSearch,
    pub platform: PlatformTarget,
    pub page: ArchivePageRequest,
}

/// A bounded candidate page; an empty page is valid.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueryArchiveCandidatesResponse {
    pub page: ArchivePage<ArchiveCandidate>,
}

/// Blocking archive hazard categories.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArchiveHazardCategory {
    PathRule,
    Symlink,
    Hardlink,
    Collision,
    Other,
    Undecodable,
    TypeConflict,
    ZipCountMismatch,
}

/// Exact portable path-rule code, kept distinct from error categories.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArchivePathRuleCode {
    Rule1Absolute,
    Rule2ParentSegment,
    Rule3Backslash,
    Rule4ColonOrPrefix,
    Rule5ControlCharacter,
    Rule6EmptyOrDotSegment,
    Rule7TrailingSpaceOrPeriod,
    Rule8ReservedDeviceName,
    Rule9LengthExceeded,
}

/// Container entry kind used only in a type/name conflict report.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArchiveEntryKind {
    File,
    Directory,
    Symlink,
    Hardlink,
    Other,
}

/// Structured blocking hazard detail without native or raw-byte fields.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "category", content = "details", rename_all = "snake_case")]
pub enum ArchiveHazardDetail {
    PathRule {
        display_path: String,
        code: ArchivePathRuleCode,
    },
    Symlink {
        display_path: String,
    },
    Hardlink {
        display_path: String,
    },
    Collision {
        display_paths: Vec<String>,
    },
    Other {
        display_path: String,
    },
    Undecodable {
        lossy: String,
        byte_length: u64,
    },
    TypeConflict {
        display_path: String,
        container_kind: ArchiveEntryKind,
    },
    ZipCountMismatch {
        declared: u64,
        parsed: u64,
    },
}

/// Queries one hazard category from the immutable snapshot.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueryArchiveHazardsRequest {
    pub request_id: ArchiveRequestId,
    pub selection_id: ArchiveSelectionId,
    pub expected_publisher_npub: String,
    pub category: ArchiveHazardCategory,
    pub page: ArchivePageRequest,
}

/// A bounded page of structured blocking hazards.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueryArchiveHazardsResponse {
    pub page: ArchivePage<ArchiveHazardDetail>,
}

/// Non-blocking archive warning categories.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArchiveWarningCategory {
    EfsDivergence,
    ExtensionMismatch,
}

/// Safe EFS divergence detail; raw ZIP filename bytes are not exposed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EfsDivergenceWarningDetail {
    pub utf8: String,
    pub cp437: String,
    pub byte_length: u64,
}

/// Filename-extension disagreement with content-based format detection.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExtensionMismatchWarningDetail {
    pub extension_format: Option<PackageFormat>,
    pub detected_format: PackageFormat,
}

/// Structured non-blocking warning detail.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "category", content = "details", rename_all = "snake_case")]
pub enum ArchiveWarningDetail {
    EfsDivergence(EfsDivergenceWarningDetail),
    ExtensionMismatch(ExtensionMismatchWarningDetail),
}

/// Queries one warning category from the immutable snapshot.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueryArchiveWarningsRequest {
    pub request_id: ArchiveRequestId,
    pub selection_id: ArchiveSelectionId,
    pub expected_publisher_npub: String,
    pub category: ArchiveWarningCategory,
    pub page: ArchivePageRequest,
}

/// A bounded page of structured non-blocking warnings.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueryArchiveWarningsResponse {
    pub page: ArchivePage<ArchiveWarningDetail>,
}

/// Explicitly discards an account-bound archive selection.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiscardArchiveSelectionRequest {
    pub request_id: ArchiveRequestId,
    pub selection_id: ArchiveSelectionId,
    pub expected_publisher_npub: String,
}

/// Acknowledges that an archive selection is no longer registered.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiscardArchiveSelectionResponse {
    pub selection_id: ArchiveSelectionId,
    pub discarded: bool,
}

/// Publication reference to a hash-verified, mutation-checked selection.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BuildSelectionRef {
    pub selection_id: ArchiveSelectionId,
    pub sha256: Sha256Digest,
}

/// Account-resolution failures for an archive operation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "code", content = "details", rename_all = "snake_case")]
pub enum ArchiveAccountFailure {
    NotAuthenticated,
    InvalidHandle,
    AccountMismatch,
}

/// Lifecycle state of an opaque selection handle.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArchiveHandleFailure {
    Unknown,
    Discarded,
}

/// Snapshot mutation detected after the file was selected.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "code", content = "details", rename_all = "snake_case")]
pub enum ArchiveMutationFailure {
    SizeChanged { selected: u64, actual: u64 },
    TimestampChanged,
    IdentityChanged,
}

/// Content-based package detection failures.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "code", content = "details", rename_all = "snake_case")]
pub enum ArchiveDetectionFailure {
    UnknownFormat,
    AmbiguousFormat,
    TruncatedSignature,
}

/// Native storage operation that failed without exposing a filesystem path.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArchiveStorageOperation {
    CreateStagingDirectory,
    StagingPreflight,
    CreateStagedFile,
    CopySource,
    SyncStagedFile,
    DeleteStagedFile,
    StartupCleanup,
    Canonicalize,
    Open,
    Metadata,
    Read,
    Seek,
}

/// Invalid pagination supplied at a transport boundary.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "code", content = "details", rename_all = "snake_case")]
pub enum ArchivePaginationFailure {
    ZeroLimit,
    LimitExceeded { requested: u16, maximum: u16 },
    OffsetOutOfRange { offset: u64, total: u64 },
}

/// Reason a requested platform is not one canonical A2 target.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "code", content = "details", rename_all = "snake_case")]
pub enum ArchivePlatformFailure {
    UnsupportedOs { os: String },
    UnsupportedArch { arch: String },
    NonCanonical { canonical: PlatformTarget },
}

/// Registry failures unrelated to the opaque handle's validity.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "code", content = "details", rename_all = "snake_case")]
pub enum ArchiveRegistryFailure {
    CapacityExceeded,
    Unavailable,
}

/// Cancellation failures for request-correlated work.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "code", content = "details", rename_all = "snake_case")]
pub enum ArchiveCancellationFailure {
    RequestNotFound,
    AlreadyCompleted,
    Cancelled,
}

/// Structural ZIP footer failures mirrored from A3.2 without native imports.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArchiveZipPreflightFailure {
    EndOfCentralDirectoryNotFound,
    Zip64LocatorMissing,
    Zip64RecordsMalformed,
    MultiDiskUnsupported,
    ArithmeticOverflow,
}

/// Every A3.2 inspection failure in a WASM-safe wire representation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "code", content = "details", rename_all = "snake_case")]
pub enum ArchiveInspectionFailure {
    UnsupportedCompressionMethod {
        entry: String,
        method: u16,
    },
    EntryLimitExceeded {
        limit: u64,
    },
    EntryCountOverflow,
    ZipCentralDirectoryLimitExceeded {
        declared_size: u64,
        limit: u64,
    },
    ZipPreflightMalformed {
        reason: ArchiveZipPreflightFailure,
    },
    MeasuredInputOverflow,
    MeasuredOutputOverflow,
    ZeroMeasuredInput {
        measured_output: u64,
    },
    CompressionRatioExceeded {
        measured_input: u64,
        measured_output: u64,
        maximum_ratio: u64,
    },
    MeasuredInputCounterRegressed,
    TarMetadataLimitExceeded {
        measured_metadata: u64,
        limit: u64,
    },
    TarDecoder {
        message: String,
    },
    TarParser {
        message: String,
    },
    ZipParser {
        message: String,
    },
    ZipIo {
        message: String,
    },
}

/// Stable typed failures returned by all archive-selection commands.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "category", content = "details", rename_all = "snake_case")]
pub enum ArchiveSelectionError {
    Account {
        failure: ArchiveAccountFailure,
    },
    Handle {
        selection_id: ArchiveSelectionId,
        failure: ArchiveHandleFailure,
    },
    Busy {
        request_id: ArchiveRequestId,
    },
    Expired {
        selection_id: ArchiveSelectionId,
    },
    Mutation {
        failure: ArchiveMutationFailure,
    },
    NonRegularFile,
    PickerFailed,
    SelectedBuildRequired,
    InsufficientStorage {
        required: u64,
        available: u64,
    },
    Storage {
        operation: ArchiveStorageOperation,
    },
    Detection {
        failure: ArchiveDetectionFailure,
    },
    DigestMismatch {
        expected: Sha256Digest,
        actual: Sha256Digest,
    },
    Pagination {
        failure: ArchivePaginationFailure,
    },
    InvalidPlatform {
        platform: PlatformTarget,
        failure: ArchivePlatformFailure,
    },
    Inspection {
        failure: ArchiveInspectionFailure,
    },
    BlockingHazards {
        counts: ArchiveHazardCounts,
    },
    Registry {
        failure: ArchiveRegistryFailure,
    },
    Cancellation {
        failure: ArchiveCancellationFailure,
    },
    UnsupportedPlatform,
    Transport,
    ContractMismatch,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};

    const SELECTION_ID: &str = "2f9a1c34-5b6d-4e7f-8a9b-0c1d2e3f4a5b";
    const REQUEST_ID: &str = "6d4b5a21-812e-49bc-a753-779d6f8cfed0";
    const DIGEST: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    fn selection_id() -> ArchiveSelectionId {
        ArchiveSelectionId::parse(SELECTION_ID).expect("valid fixture selection ID")
    }

    fn request_id() -> ArchiveRequestId {
        ArchiveRequestId::parse(REQUEST_ID).expect("valid fixture request ID")
    }

    #[test]
    fn opaque_ids_accept_only_canonical_lowercase_uuid_v4() {
        assert_eq!(selection_id().as_str(), SELECTION_ID);
        assert_eq!(request_id().as_str(), REQUEST_ID);
        for hostile in [
            "2F9A1C34-5B6D-4E7F-8A9B-0C1D2E3F4A5B",
            "2f9a1c34-5b6d-3e7f-8a9b-0c1d2e3f4a5b",
            "2f9a1c34-5b6d-4e7f-7a9b-0c1d2e3f4a5b",
            "2f9a1c345b6d4e7f8a9b0c1d2e3f4a5b",
        ] {
            assert!(ArchiveSelectionId::parse(hostile).is_err(), "{hostile}");
            assert!(serde_json::from_value::<ArchiveRequestId>(json!(hostile)).is_err());
        }
        assert!(SafeArchiveFilename::parse("game.zip").is_ok());
        for hostile in ["", "/tmp/game.zip", "..\\game.zip", "bad\nname.zip"] {
            assert!(SafeArchiveFilename::parse(hostile).is_err(), "{hostile:?}");
            assert!(serde_json::from_value::<SafeArchiveFilename>(json!(hostile)).is_err());
        }
    }

    #[test]
    fn page_constants_and_validated_shape_are_stable() {
        assert_eq!(ARCHIVE_PAGE_SIZE_DEFAULT, 100);
        assert_eq!(ARCHIVE_PAGE_SIZE_MAX, 200);
        assert_eq!(
            serde_json::to_value(ArchivePageRequest::default()).expect("serialize page"),
            json!({"offset": 0, "limit": 100})
        );
        assert!(ArchivePageRequest::new(0, 0).is_err());
        assert!(ArchivePageRequest::new(0, 201).is_err());

        let zero = serde_json::from_value::<ArchivePageRequest>(json!({
            "offset": 0,
            "limit": 0
        }))
        .expect("wire zero deserializes");
        assert_eq!(zero.validate(), Err(ArchivePaginationFailure::ZeroLimit));

        let excessive = serde_json::from_value::<ArchivePageRequest>(json!({
            "offset": 0,
            "limit": 201
        }))
        .expect("wire excessive limit deserializes");
        assert_eq!(
            excessive.validate(),
            Err(ArchivePaginationFailure::LimitExceeded {
                requested: 201,
                maximum: ARCHIVE_PAGE_SIZE_MAX,
            })
        );

        for malformed in [
            json!({"offset": 0}),
            json!({"offset": "0", "limit": 1}),
            json!({"offset": 0, "limit": 65_536}),
            json!({"offset": 0, "limit": 1, "extra": true}),
        ] {
            assert!(serde_json::from_value::<ArchivePageRequest>(malformed).is_err());
        }
    }

    #[test]
    fn validated_distribution_types_reject_hostile_wire_values() {
        assert!(serde_json::from_value::<SelectableArchiveFile>(json!({
            "path": "../escape.exe",
            "declared_size": 1
        }))
        .is_err());
        assert!(serde_json::from_value::<SelectableWorkingDirectory>(json!({
            "directory": "bin//tools"
        }))
        .is_err());
        assert!(
            serde_json::from_value::<HashSelectedArchiveResponse>(json!({
                "selection_id": SELECTION_ID,
                "sha256": DIGEST.to_ascii_uppercase()
            }))
            .is_err()
        );
    }

    #[test]
    fn candidate_page_is_empty_and_has_no_selected_field() {
        let response = QueryArchiveCandidatesResponse {
            page: ArchivePage {
                offset: 0,
                total: 0,
                next_offset: None,
                items: Vec::new(),
            },
        };
        let value = serde_json::to_value(response).expect("serialize empty candidate page");
        assert_eq!(
            value,
            json!({"page":{"offset":0,"total":0,"next_offset":null,"items":[]}})
        );
        assert!(value.get("selected").is_none());
    }

    #[test]
    fn serde_discriminants_and_nested_details_are_exact() {
        let hazard = ArchiveHazardDetail::PathRule {
            display_path: "../escape".to_owned(),
            code: ArchivePathRuleCode::Rule2ParentSegment,
        };
        assert_eq!(
            serde_json::to_value(hazard).expect("serialize hazard"),
            json!({
                "category": "path_rule",
                "details": {"display_path": "../escape", "code": "rule2_parent_segment"}
            })
        );
        let error = ArchiveSelectionError::Inspection {
            failure: ArchiveInspectionFailure::ZipPreflightMalformed {
                reason: ArchiveZipPreflightFailure::MultiDiskUnsupported,
            },
        };
        assert_eq!(
            serde_json::to_value(error).expect("serialize error"),
            json!({
                "category": "inspection",
                "details": {"failure": {
                    "code": "zip_preflight_malformed",
                    "details": {"reason": "multi_disk_unsupported"}
                }}
            })
        );
    }

    #[test]
    fn path_rule_and_command_error_categories_remain_distinct() {
        let path = serde_json::to_value(ArchiveHazardDetail::PathRule {
            display_path: "/root".to_owned(),
            code: ArchivePathRuleCode::Rule1Absolute,
        })
        .expect("serialize path hazard");
        let error = serde_json::to_value(ArchiveSelectionError::Detection {
            failure: ArchiveDetectionFailure::UnknownFormat,
        })
        .expect("serialize command error");
        assert_eq!(path["category"], "path_rule");
        assert_eq!(error["category"], "detection");
        assert_eq!(path["details"]["code"], "rule1_absolute");
        assert_eq!(error["details"]["failure"]["code"], "unknown_format");
    }

    #[test]
    fn representative_contract_json_has_no_forbidden_keys() {
        let values = vec![
            serde_json::to_value(BuildSelectionRef {
                selection_id: selection_id(),
                sha256: Sha256Digest::parse(DIGEST).expect("valid fixture digest"),
            })
            .expect("serialize build reference"),
            serde_json::to_value(ArchiveHazardDetail::Undecodable {
                lossy: "bad�name".to_owned(),
                byte_length: 8,
            })
            .expect("serialize undecodable detail"),
            serde_json::to_value(ArchiveWarningDetail::EfsDivergence(
                EfsDivergenceWarningDetail {
                    utf8: "é".to_owned(),
                    cp437: "├⌐".to_owned(),
                    byte_length: 2,
                },
            ))
            .expect("serialize warning"),
        ];

        for value in &values {
            reject_forbidden_keys(value);
        }
    }

    fn reject_forbidden_keys(value: &Value) {
        match value {
            Value::Object(object) => {
                for (key, child) in object {
                    assert!(
                        !["file_path", "path_buf", "raw_bytes", "entries", "index"]
                            .contains(&key.as_str()),
                        "forbidden contract key: {key}"
                    );
                    reject_forbidden_keys(child);
                }
            }
            Value::Array(values) => {
                for child in values {
                    reject_forbidden_keys(child);
                }
            }
            _ => {}
        }
    }
}
