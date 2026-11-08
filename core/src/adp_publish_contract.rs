//! Shared typed wire contracts for ADP publication and authoritative edits.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::distribution::{LaunchProfile, PackageFormat, PlatformTarget, ProfileId};

/// One publisher-selected game profile and the listing targets where it is the default.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublisherLaunchDefault {
    pub profile_id: ProfileId,
    pub targets: Vec<PlatformTarget>,
}

/// Publisher-selected launch metadata for a listing revision.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdpLaunchTierIntent {
    pub package_format: PackageFormat,
    pub platforms: Vec<PlatformTarget>,
    pub launch_profiles: Vec<LaunchProfile>,
    pub publisher_defaults: Vec<PublisherLaunchDefault>,
}

/// Additive publication fields reserved for the typed publish command.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublishAdpListingRequestExtension {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub server_roles: Vec<crate::publisher_management::ServerRoleIntent>,
    /// Explicit authority setup, never inferred from a metadata edit or retry.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub authorize_operators: bool,
    /// IPC correlation only; this does not grant publisher or purchase authority.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operation_id: Option<String>,
    #[serde(
        default,
        skip_serializing_if = "crate::adp_settlement::PublisherCommercialIntent::is_preserve"
    )]
    pub commercial_intent: crate::adp_settlement::PublisherCommercialIntent,
    pub launch_tier: Option<AdpLaunchTierIntent>,
    pub acknowledged_removed_profile_ids: Vec<ProfileId>,
}

/// Structured core-builder or exact-final-tag publication violation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "code", content = "details", rename_all = "snake_case")]
pub enum AdpPublishValidationViolation {
    MissingLud16,
    IncompleteFulfillmentTier {
        missing: String,
    },
    MalformedServerUrl {
        url: String,
    },
    InvalidTimedAccess,
    MalformedPreservedTag,
    MalformedImageUrl {
        url: String,
    },
    InvalidPlatform {
        platform: String,
    },
    UnsupportedPlatformOs {
        os: String,
    },
    UnsupportedPlatformArch {
        arch: String,
    },
    InvalidLaunchDisplayName {
        profile_id: String,
        reason: DisplayNameRejection,
    },
    InvalidLaunchArguments {
        profile_id: String,
        reason: ArgumentRejection,
    },
    LaunchArgumentsSerialization {
        profile_id: String,
    },
    LaunchPackageCount {
        actual: usize,
    },
    MalformedPackageTag,
    UnsupportedPackageFormat {
        format: String,
    },
    InvalidLaunchTagLength {
        profile_id: String,
        actual: usize,
    },
    MalformedLaunchProfile {
        profile_id: String,
        rejection: AuthoritativeProfileRejection,
    },
    UnsupportedLaunchType {
        profile_id: String,
        launch_type: String,
    },
    DuplicateLaunchProfileId {
        profile_id: ProfileId,
    },
    InvalidLaunchPlatform {
        platform: String,
    },
    NonCanonicalLaunchPlatform {
        platform: String,
        canonical: String,
    },
    MissingLaunchPlatform,
    MissingGameLaunchProfile,
    MissingLaunchDefault,
    EmptyLaunchDefaultTargets {
        profile_id: ProfileId,
    },
    MalformedLaunchDefault,
    DuplicateLaunchDefaultProfile {
        profile_id: ProfileId,
    },
    InvalidLaunchDefaultTarget {
        target: String,
    },
    NonCanonicalLaunchDefaultTarget {
        target: String,
        canonical: String,
    },
    UndeclaredLaunchDefaultTarget {
        target: PlatformTarget,
    },
    OverlappingLaunchDefaultTarget {
        target: PlatformTarget,
    },
    UncoveredLaunchDefaultTarget {
        target: PlatformTarget,
    },
    MixedLaunchDefaultScopes,
    LegacyLaunchDefaultForMultiplePlatforms,
    DanglingLaunchDefault {
        profile_id: ProfileId,
    },
    NonGameLaunchDefault {
        profile_id: ProfileId,
    },
    InvalidFileHash,
    InvalidCampaignPointer,
    InvalidNip94EventId,
    InvalidFulfillmentAuthorization,
}

/// Intent-only request for loading one authoritative listing revision.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoadAuthoritativeAdpEditMetadataRequest {
    pub expected_publisher_npub: String,
    pub d_tag: String,
    pub expected_event_id: String,
}

/// Authoritative launch metadata loaded for a listing edit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", content = "metadata", rename_all = "snake_case")]
pub enum AuthoritativeAdpEditMetadata {
    Absent,
    Present(AuthoritativeAdpEditMetadataPresent),
}

/// Raw authoritative tags and all structured defects for a present tier.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthoritativeAdpEditMetadataPresent {
    pub tags: Vec<AuthoritativeAdpEditTag>,
    pub defects: Vec<AuthoritativeAdpEditDefect>,
    #[serde(default)]
    pub publisher_defaults: Vec<PublisherLaunchDefault>,
}

/// One decoded tag at its original signed-event index.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AuthoritativeAdpEditTag {
    Package {
        event_tag_index: usize,
        raw: Vec<String>,
        package_format: Option<PackageFormat>,
    },
    Platform {
        event_tag_index: usize,
        raw: Vec<String>,
        raw_platform: Option<String>,
        target: Option<PlatformTarget>,
    },
    Launch {
        event_tag_index: usize,
        raw: Vec<String>,
        raw_profile_id: Option<String>,
        valid_profile_id: Option<ProfileId>,
        profile: Option<LaunchProfile>,
        rejection: Option<AuthoritativeProfileRejection>,
    },
    LaunchDefault {
        event_tag_index: usize,
        raw: Vec<String>,
        raw_profile_id: Option<String>,
        valid_profile_id: Option<ProfileId>,
        raw_targets: Vec<String>,
        targets: Vec<PlatformTarget>,
        legacy: bool,
    },
}

/// A structured parse or launch-tier policy defect.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "defect", content = "details", rename_all = "snake_case")]
pub enum AuthoritativeAdpEditDefect {
    PackageCount {
        actual: usize,
        event_tag_indices: Vec<usize>,
    },
    MalformedPackage {
        event_tag_index: usize,
    },
    UnsupportedPackage {
        event_tag_index: usize,
        raw_format: String,
    },
    MissingPlatform,
    MalformedPlatform {
        event_tag_index: usize,
    },
    InvalidPlatform {
        event_tag_index: usize,
        raw_platform: Option<String>,
    },
    NonCanonicalPlatform {
        event_tag_index: usize,
        raw_platform: String,
        canonical: String,
    },
    MalformedLaunch {
        event_tag_index: usize,
        rejection: AuthoritativeProfileRejection,
    },
    DuplicateProfileId {
        profile_id: ProfileId,
        event_tag_indices: Vec<usize>,
    },
    UnknownLaunchType {
        event_tag_index: usize,
        raw_launch_type: String,
    },
    NoValidGameProfile,
    MalformedLaunchDefault {
        event_tag_index: usize,
        raw_profile_id: Option<String>,
    },
    DuplicateLaunchDefaultProfile {
        profile_id: ProfileId,
        event_tag_indices: Vec<usize>,
    },
    InvalidLaunchDefaultTarget {
        event_tag_index: usize,
        raw_target: String,
    },
    NonCanonicalLaunchDefaultTarget {
        event_tag_index: usize,
        raw_target: String,
        canonical: String,
    },
    UndeclaredLaunchDefaultTarget {
        event_tag_index: usize,
        target: PlatformTarget,
    },
    OverlappingLaunchDefaultTarget {
        target: PlatformTarget,
        event_tag_indices: Vec<usize>,
    },
    UncoveredLaunchDefaultTarget {
        target: PlatformTarget,
    },
    MixedLaunchDefaultScopes {
        event_tag_indices: Vec<usize>,
    },
    DanglingLaunchDefault {
        event_tag_index: usize,
        profile_id: ProfileId,
    },
    NonGameLaunchDefault {
        event_tag_index: usize,
        profile_id: ProfileId,
    },
}

/// Exact structured reason an authoritative launch tag was rejected.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "field", content = "reason", rename_all = "snake_case")]
pub enum AuthoritativeProfileRejection {
    MissingRequiredElement,
    ProfileId(ProfileIdRejection),
    Executable(PathRejection),
    WorkingDir(PathRejection),
    Arguments(ArgumentRejection),
    DisplayName(DisplayNameRejection),
}

/// Structured profile-ID rejection reasons.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProfileIdRejection {
    InvalidGrammar,
}

/// Structured portable-path rejection reasons.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PathRejection {
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

/// Structured launch-argument rejection reasons.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArgumentRejection {
    InvalidJsonShape,
    TooMany,
    ElementTooLong,
    ContainsNul,
}

/// Structured launch display-name rejection reasons.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DisplayNameRejection {
    InvalidLength,
    ControlCharacter,
}

/// Stable typed failures returned by publication and edit-metadata commands.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "category", content = "details", rename_all = "snake_case")]
pub enum PublishAdpListingError {
    Archive {
        error: crate::archive_inspection_contract::ArchiveSelectionError,
    },
    ProfileRemovalAcknowledgementRequired {
        removed_profile_ids: Vec<ProfileId>,
    },
    CoreValidation {
        violation: AdpPublishValidationViolation,
        message: String,
    },
    Authority {
        message: String,
    },
    Signer {
        message: String,
    },
    Relay {
        message: String,
    },
    Upload {
        message: String,
    },
    PartialPublication {
        cause: Box<PublishAdpListingError>,
        successful_servers: Vec<String>,
    },
    AuthoritativeRelayRefetchFailure {
        message: String,
    },
    AuthoritativeListingNotFound {
        coordinate: String,
    },
    AuthoritativeEventIdInvalid {
        event_id: String,
    },
    AuthoritativeSignatureInvalid {
        event_id: String,
    },
    AuthoritativePublisherMismatch {
        expected_publisher_npub: String,
        actual_publisher_hex: String,
    },
    AuthoritativeKindMismatch {
        expected: u16,
        actual: u16,
    },
    AuthoritativeCoordinateMismatch {
        expected_d_tag: String,
        actual_d_tags: Vec<String>,
    },
    StaleExpectedEventId {
        expected_event_id: String,
        actual_event_id: String,
    },
    UnsupportedPlatform,
    Transport {
        message: String,
    },
    Other {
        message: String,
    },
}

impl From<String> for PublishAdpListingError {
    fn from(message: String) -> Self {
        Self::Transport { message }
    }
}

impl fmt::Display for PublishAdpListingError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Archive { .. } => formatter.write_str("archive selection rejected publication"),
            Self::ProfileRemovalAcknowledgementRequired {
                removed_profile_ids,
            } => write!(
                formatter,
                "publication requires acknowledgement of {} removed launch profile(s)",
                removed_profile_ids.len()
            ),
            Self::CoreValidation { message, .. } => write!(formatter, "core validation: {message}"),
            Self::Authority { message } => write!(formatter, "authority: {message}"),
            Self::Signer { message } => write!(formatter, "signer: {message}"),
            Self::Relay { message } => write!(formatter, "relay: {message}"),
            Self::Upload { message } => write!(formatter, "upload: {message}"),
            Self::PartialPublication {
                cause,
                successful_servers,
            } => write!(
                formatter,
                "partial publication after {} successful server upload(s): {cause}",
                successful_servers.len()
            ),
            Self::AuthoritativeRelayRefetchFailure { message } => {
                write!(formatter, "authoritative relay refetch failed: {message}")
            }
            Self::AuthoritativeListingNotFound { coordinate } => {
                write!(formatter, "authoritative listing not found: {coordinate}")
            }
            Self::AuthoritativeEventIdInvalid { event_id } => {
                write!(formatter, "authoritative event ID is invalid: {event_id}")
            }
            Self::AuthoritativeSignatureInvalid { event_id } => {
                write!(formatter, "authoritative event signature is invalid: {event_id}")
            }
            Self::AuthoritativePublisherMismatch {
                expected_publisher_npub,
                actual_publisher_hex,
            } => write!(
                formatter,
                "authoritative publisher mismatch: expected {expected_publisher_npub}, got {actual_publisher_hex}"
            ),
            Self::AuthoritativeKindMismatch { expected, actual } => write!(
                formatter,
                "authoritative kind mismatch: expected {expected}, got {actual}"
            ),
            Self::AuthoritativeCoordinateMismatch {
                expected_d_tag,
                actual_d_tags,
            } => write!(
                formatter,
                "authoritative coordinate mismatch: expected d tag {expected_d_tag}, got {actual_d_tags:?}"
            ),
            Self::StaleExpectedEventId {
                expected_event_id,
                actual_event_id,
            } => write!(
                formatter,
                "stale expected event ID: expected {expected_event_id}, got {actual_event_id}"
            ),
            Self::UnsupportedPlatform => {
                formatter.write_str("ADP publishing is unsupported on this platform")
            }
            Self::Transport { message } => write!(formatter, "transport: {message}"),
            Self::Other { message } => formatter.write_str(message),
        }
    }
}

impl std::error::Error for PublishAdpListingError {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::distribution::{LaunchType, RelativePath, WorkingDir};
    use serde_json::json;

    fn profile() -> LaunchProfile {
        LaunchProfile {
            id: ProfileId::parse("play").expect("valid profile ID"),
            display_name: "Play".to_owned(),
            executable: RelativePath::parse("bin/game").expect("valid executable"),
            working_dir: WorkingDir::parse(".").expect("valid working directory"),
            arguments: vec!["--safe".to_owned()],
            launch_type: LaunchType::Game,
        }
    }

    fn extension() -> PublishAdpListingRequestExtension {
        PublishAdpListingRequestExtension {
            authorize_operators: false,
            server_roles: Vec::new(),
            commercial_intent: Default::default(),
            operation_id: None,
            launch_tier: Some(AdpLaunchTierIntent {
                package_format: PackageFormat::Zip,
                platforms: vec![PlatformTarget {
                    os: "linux".to_owned(),
                    arch: "x86_64".to_owned(),
                }],
                launch_profiles: vec![profile()],
                publisher_defaults: vec![PublisherLaunchDefault {
                    profile_id: ProfileId::parse("play").expect("valid default profile ID"),
                    targets: vec![PlatformTarget {
                        os: "linux".to_owned(),
                        arch: "x86_64".to_owned(),
                    }],
                }],
            }),
            acknowledged_removed_profile_ids: vec![
                ProfileId::parse("legacy").expect("valid acknowledged ID")
            ],
        }
    }

    #[test]
    fn publication_intent_has_exact_wire_shape_and_round_trips() {
        let intent = extension();
        let value = serde_json::to_value(&intent).expect("intent serializes");

        assert_eq!(
            value,
            json!({
                "launch_tier": {
                    "package_format": "Zip",
                    "platforms": [{ "os": "linux", "arch": "x86_64" }],
                    "launch_profiles": [{
                        "id": "play",
                        "display_name": "Play",
                        "executable": "bin/game",
                        "working_dir": ".",
                        "arguments": ["--safe"],
                        "launch_type": "Game"
                    }],
                    "publisher_defaults": [{
                        "profile_id": "play",
                        "targets": [{ "os": "linux", "arch": "x86_64" }]
                    }]
                },
                "acknowledged_removed_profile_ids": ["legacy"]
            })
        );
        assert_eq!(
            serde_json::from_value::<PublishAdpListingRequestExtension>(value)
                .expect("intent round-trips"),
            intent
        );
    }

    #[test]
    fn authoritative_load_request_contains_only_edit_intent() {
        let request = LoadAuthoritativeAdpEditMetadataRequest {
            expected_publisher_npub: "npub1publisher".to_owned(),
            d_tag: "game".to_owned(),
            expected_event_id: "11".repeat(32),
        };
        let value = serde_json::to_value(&request).expect("load request serializes");

        assert_eq!(
            value,
            json!({
                "expected_publisher_npub": "npub1publisher",
                "d_tag": "game",
                "expected_event_id": "11".repeat(32),
            })
        );
        assert_eq!(
            serde_json::from_value::<LoadAuthoritativeAdpEditMetadataRequest>(value)
                .expect("load request round-trips"),
            request
        );
    }

    #[test]
    fn hostile_executables_fail_intent_deserialization_through_a2_types() {
        for executable in ["../escape", "."] {
            let mut value = serde_json::to_value(extension()).expect("intent serializes");
            value["launch_tier"]["launch_profiles"][0]["executable"] = json!(executable);

            assert!(
                serde_json::from_value::<PublishAdpListingRequestExtension>(value).is_err(),
                "{executable} must fail RelativePath validation"
            );
        }
    }

    #[test]
    fn profile_ids_revalidate_and_all_profile_fields_round_trip() {
        let intent = extension();
        let value = serde_json::to_value(&intent).expect("intent serializes");
        assert_eq!(
            serde_json::from_value::<PublishAdpListingRequestExtension>(value.clone())
                .expect("all profile fields round-trip"),
            intent
        );

        let mut hostile = value;
        hostile["launch_tier"]["launch_profiles"][0]["id"] = json!("Play");
        assert!(serde_json::from_value::<PublishAdpListingRequestExtension>(hostile).is_err());
    }

    #[test]
    fn removal_acknowledgement_error_has_exact_validated_wire_shape() {
        let error = PublishAdpListingError::ProfileRemovalAcknowledgementRequired {
            removed_profile_ids: vec![ProfileId::parse("old-play").expect("valid profile ID")],
        };
        let value = serde_json::to_value(&error).expect("error serializes");

        assert_eq!(
            value,
            json!({
                "category": "profile_removal_acknowledgement_required",
                "details": { "removed_profile_ids": ["old-play"] }
            })
        );
        assert_eq!(
            serde_json::from_value::<PublishAdpListingError>(value).expect("error round-trips"),
            error
        );

        for malformed in ["Old-play", "not valid"] {
            let hostile = json!({
                "category": "profile_removal_acknowledgement_required",
                "details": { "removed_profile_ids": [malformed] }
            });
            assert!(serde_json::from_value::<PublishAdpListingError>(hostile).is_err());
        }
    }

    #[test]
    fn core_validation_error_has_structured_non_textual_wire_discriminant() {
        let error = PublishAdpListingError::CoreValidation {
            violation: AdpPublishValidationViolation::DuplicateLaunchProfileId {
                profile_id: ProfileId::parse("play").expect("valid profile ID"),
            },
            message: "duplicate launch profile ID".to_owned(),
        };
        let value = serde_json::to_value(&error).expect("error serializes");

        assert_eq!(
            value,
            json!({
                "category": "core_validation",
                "details": {
                    "violation": {
                        "code": "duplicate_launch_profile_id",
                        "details": { "profile_id": "play" }
                    },
                    "message": "duplicate launch profile ID"
                }
            })
        );
        assert_eq!(
            serde_json::from_value::<PublishAdpListingError>(value).expect("error round-trips"),
            error
        );
    }

    #[test]
    fn publication_error_categories_have_distinct_discriminants() {
        let errors = [
            PublishAdpListingError::CoreValidation {
                violation: AdpPublishValidationViolation::MissingLud16,
                message: "x".into(),
            },
            PublishAdpListingError::Authority {
                message: "x".into(),
            },
            PublishAdpListingError::Signer {
                message: "x".into(),
            },
            PublishAdpListingError::Relay {
                message: "x".into(),
            },
            PublishAdpListingError::Upload {
                message: "x".into(),
            },
            PublishAdpListingError::AuthoritativeRelayRefetchFailure {
                message: "x".into(),
            },
            PublishAdpListingError::AuthoritativeListingNotFound {
                coordinate: "30402:publisher:game".into(),
            },
            PublishAdpListingError::AuthoritativeEventIdInvalid {
                event_id: "x".into(),
            },
            PublishAdpListingError::AuthoritativeSignatureInvalid {
                event_id: "x".into(),
            },
            PublishAdpListingError::AuthoritativePublisherMismatch {
                expected_publisher_npub: "npub1expected".into(),
                actual_publisher_hex: "actual".into(),
            },
            PublishAdpListingError::AuthoritativeKindMismatch {
                expected: 30402,
                actual: 1,
            },
            PublishAdpListingError::AuthoritativeCoordinateMismatch {
                expected_d_tag: "game".into(),
                actual_d_tags: vec!["other".into()],
            },
            PublishAdpListingError::StaleExpectedEventId {
                expected_event_id: "expected".into(),
                actual_event_id: "actual".into(),
            },
            PublishAdpListingError::UnsupportedPlatform,
            PublishAdpListingError::Transport {
                message: "x".into(),
            },
            PublishAdpListingError::Other {
                message: "x".into(),
            },
        ];
        let categories = errors
            .iter()
            .map(|error| {
                serde_json::to_value(error).expect("error serializes")["category"]
                    .as_str()
                    .expect("category is a string")
                    .to_owned()
            })
            .collect::<std::collections::HashSet<_>>();

        assert_eq!(categories.len(), errors.len());
    }

    #[cfg(feature = "native")]
    #[test]
    fn authoritative_metadata_conversion_preserves_raw_order_and_typed_profile() {
        use crate::adp_publish::{
            AuthoritativeLaunchMetadata, AuthoritativeLaunchTag, AuthoritativeLaunchTier,
            DecodedLaunchDefaultTag, DecodedLaunchTag, DecodedPackageTag, DecodedPlatformTag,
        };
        use crate::distribution::ParsedLaunchTag;

        let expected_profile = profile();
        let source = AuthoritativeLaunchTier::Present(AuthoritativeLaunchMetadata {
            tags: vec![
                AuthoritativeLaunchTag::Platform(DecodedPlatformTag {
                    event_tag_index: 3,
                    raw: vec!["platform".to_owned(), "LINUX-x64".to_owned()],
                    raw_platform: Some("LINUX-x64".to_owned()),
                    target: Some(PlatformTarget {
                        os: "linux".to_owned(),
                        arch: "x86_64".to_owned(),
                    }),
                }),
                AuthoritativeLaunchTag::Package(DecodedPackageTag {
                    event_tag_index: 5,
                    raw: vec!["package".to_owned(), "zip".to_owned()],
                    package_format: Some(PackageFormat::Zip),
                }),
                AuthoritativeLaunchTag::Launch(DecodedLaunchTag {
                    event_tag_index: 8,
                    raw: vec![
                        "launch".to_owned(),
                        "play".to_owned(),
                        "bin/game".to_owned(),
                        ".".to_owned(),
                        "[\"--safe\"]".to_owned(),
                        "game".to_owned(),
                        "Play".to_owned(),
                    ],
                    parsed: ParsedLaunchTag {
                        raw_profile_id: Some("play".to_owned()),
                        valid_profile_id: Some(expected_profile.id.clone()),
                        profile: Ok(expected_profile.clone()),
                    },
                }),
                AuthoritativeLaunchTag::LaunchDefault(DecodedLaunchDefaultTag {
                    event_tag_index: 9,
                    raw: vec!["launch_default".to_owned(), "play".to_owned()],
                    raw_profile_id: Some("play".to_owned()),
                    valid_profile_id: Some(expected_profile.id.clone()),
                    raw_targets: Vec::new(),
                    targets: Vec::new(),
                    legacy: true,
                }),
            ],
            defects: Vec::new(),
            publisher_defaults: vec![PublisherLaunchDefault {
                profile_id: expected_profile.id.clone(),
                targets: vec![PlatformTarget {
                    os: "linux".to_owned(),
                    arch: "x86_64".to_owned(),
                }],
            }],
        });

        let wire = AuthoritativeAdpEditMetadata::from(source);
        let value = serde_json::to_value(&wire).expect("metadata serializes");
        assert_eq!(
            value,
            json!({
                "state": "present",
                "metadata": {
                    "tags": [
                        {
                            "kind": "platform",
                            "event_tag_index": 3,
                            "raw": ["platform", "LINUX-x64"],
                            "raw_platform": "LINUX-x64",
                            "target": { "os": "linux", "arch": "x86_64" }
                        },
                        {
                            "kind": "package",
                            "event_tag_index": 5,
                            "raw": ["package", "zip"],
                            "package_format": "Zip"
                        },
                        {
                            "kind": "launch",
                            "event_tag_index": 8,
                            "raw": [
                                "launch", "play", "bin/game", ".", "[\"--safe\"]",
                                "game", "Play"
                            ],
                            "raw_profile_id": "play",
                            "valid_profile_id": "play",
                            "profile": {
                                "id": "play",
                                "display_name": "Play",
                                "executable": "bin/game",
                                "working_dir": ".",
                                "arguments": ["--safe"],
                                "launch_type": "Game"
                            },
                            "rejection": null
                        },
                        {
                            "kind": "launch_default",
                            "event_tag_index": 9,
                            "raw": ["launch_default", "play"],
                            "raw_profile_id": "play",
                            "valid_profile_id": "play",
                            "raw_targets": [],
                            "targets": [],
                            "legacy": true
                        }
                    ],
                    "defects": [],
                    "publisher_defaults": [{
                        "profile_id": "play",
                        "targets": [{ "os": "linux", "arch": "x86_64" }]
                    }]
                }
            })
        );
        let round_trip = serde_json::from_value::<AuthoritativeAdpEditMetadata>(value)
            .expect("metadata round-trips through validated A2 fields");

        assert_eq!(round_trip, wire);
        let AuthoritativeAdpEditMetadata::Present(metadata) = round_trip else {
            panic!("source claimed a present launch tier");
        };
        assert_eq!(
            metadata
                .tags
                .iter()
                .map(|tag| match tag {
                    AuthoritativeAdpEditTag::Package {
                        event_tag_index,
                        raw,
                        ..
                    }
                    | AuthoritativeAdpEditTag::Platform {
                        event_tag_index,
                        raw,
                        ..
                    }
                    | AuthoritativeAdpEditTag::Launch {
                        event_tag_index,
                        raw,
                        ..
                    }
                    | AuthoritativeAdpEditTag::LaunchDefault {
                        event_tag_index,
                        raw,
                        ..
                    } => (*event_tag_index, raw.clone()),
                })
                .collect::<Vec<_>>(),
            vec![
                (3, vec!["platform".to_owned(), "LINUX-x64".to_owned()]),
                (5, vec!["package".to_owned(), "zip".to_owned()]),
                (
                    8,
                    vec![
                        "launch".to_owned(),
                        "play".to_owned(),
                        "bin/game".to_owned(),
                        ".".to_owned(),
                        "[\"--safe\"]".to_owned(),
                        "game".to_owned(),
                        "Play".to_owned(),
                    ]
                ),
                (9, vec!["launch_default".to_owned(), "play".to_owned()]),
            ]
        );
        assert_eq!(
            metadata.publisher_defaults,
            vec![PublisherLaunchDefault {
                profile_id: expected_profile.id.clone(),
                targets: vec![PlatformTarget {
                    os: "linux".to_owned(),
                    arch: "x86_64".to_owned(),
                }],
            }]
        );
        assert!(matches!(
            &metadata.tags[2],
            AuthoritativeAdpEditTag::Launch {
                raw_profile_id: Some(raw_profile_id),
                valid_profile_id: Some(profile_id),
                profile: Some(profile),
                rejection: None,
                ..
            } if raw_profile_id == "play"
                && profile_id == &expected_profile.id
                && profile == &expected_profile
        ));
    }
}
