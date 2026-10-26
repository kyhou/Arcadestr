//! Pure ADP publish event builders.

use std::collections::HashSet;

use nostr::{EventBuilder, Kind, Tag, TagKind};
use thiserror::Error;

pub use crate::adp_publish_contract::{
    AdpLaunchTierIntent as AdpLaunchTierInput, PublisherLaunchDefault,
};
use crate::adp_publish_contract::{
    AdpPublishValidationViolation, ArgumentRejection, AuthoritativeAdpEditDefect,
    AuthoritativeAdpEditMetadata, AuthoritativeAdpEditMetadataPresent, AuthoritativeAdpEditTag,
    AuthoritativeProfileRejection, DisplayNameRejection, PathRejection, ProfileIdRejection,
    PublishAdpListingError,
};
use crate::authorization::{
    AuthorizationTerms, CAPABILITY_ISSUE_GRANT, CAPABILITY_ISSUE_RECEIPT, CAPABILITY_UPLOAD_BUILD,
    FULFILLMENT_AUTHORIZATION_KIND,
};
use crate::distribution::{LaunchType, PackageFormat, PlatformTarget};
use crate::marketplace::AcquisitionPolicy;

#[cfg(test)]
use crate::distribution::LaunchProfile;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FulfillmentAuthorizationInput {
    pub root_event_id: String,
    pub fulfillment_pubkey: String,
    pub relay_hint: Option<String>,
}

/// Input required to construct an ADP NIP-99 listing event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdpListingInput {
    pub d_tag: String,
    pub title: String,
    pub description: String,
    pub price_sats: u64,
    pub lud16: Option<String>,
    pub tags: Vec<String>,
    pub images: Vec<String>,
    pub servers: Vec<String>,
    pub file_hash: Option<String>,
    pub version: Option<String>,
    pub fulfillment_authorizations: Vec<FulfillmentAuthorizationInput>,
    pub acquisition: AcquisitionPolicy,
    pub platforms: Vec<String>,
    pub campaigns: Vec<(String, Option<String>)>,
    pub nip94_event_id: Option<String>,
    pub preserved_tags: Vec<Vec<String>>,
}

/// Optional typed launch metadata layered over a legacy listing input.
#[derive(Debug, Clone, Copy)]
pub struct AdpListingPublishInput<'a> {
    pub listing: &'a AdpListingInput,
    pub launch_tier: Option<&'a AdpLaunchTierInput>,
}

impl<'a> From<&'a AdpListingInput> for AdpListingPublishInput<'a> {
    fn from(listing: &'a AdpListingInput) -> Self {
        Self {
            listing,
            launch_tier: None,
        }
    }
}

/// Faithful decoding of signed authoritative launch metadata.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AuthoritativeLaunchTier {
    /// No `package`, `launch`, or `launch_default` tag claims the launch tier.
    Absent,
    /// At least one launch-tier tag claims the launch tier.
    Present(AuthoritativeLaunchMetadata),
}

/// Raw authoritative tags and all defects found without repairing their values.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuthoritativeLaunchMetadata {
    /// Package, platform, launch, and launch-default tags in original order.
    pub tags: Vec<AuthoritativeLaunchTag>,
    /// Parse and tier-policy defects. Defects never suppress decoded tags.
    pub defects: Vec<AuthoritativeLaunchDefect>,
    /// Valid profile-to-target mappings recovered from the signed tags.
    pub publisher_defaults: Vec<PublisherLaunchDefault>,
}

/// One authoritative tag retained at its exact signed-event position.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AuthoritativeLaunchTag {
    Package(DecodedPackageTag),
    Platform(DecodedPlatformTag),
    Launch(DecodedLaunchTag),
    LaunchDefault(DecodedLaunchDefaultTag),
}

/// Faithful publisher-default-tag decoding.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DecodedLaunchDefaultTag {
    pub event_tag_index: usize,
    pub raw: Vec<String>,
    pub raw_profile_id: Option<String>,
    pub valid_profile_id: Option<crate::distribution::ProfileId>,
    pub raw_targets: Vec<String>,
    /// Recognized targets normalized through the tolerant platform vocabulary.
    pub targets: Vec<PlatformTarget>,
    /// Two-element tags are the legacy global form.
    pub legacy: bool,
}

/// Faithful package-tag decoding.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DecodedPackageTag {
    pub event_tag_index: usize,
    pub raw: Vec<String>,
    /// Recognized format, even when another positional element makes the tag malformed.
    pub package_format: Option<PackageFormat>,
}

/// Faithful platform-tag decoding.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DecodedPlatformTag {
    pub event_tag_index: usize,
    pub raw: Vec<String>,
    /// Original value spelling, if supplied.
    pub raw_platform: Option<String>,
    /// A2's canonical interpretation; aliases remain unchanged in `raw_platform`.
    pub target: Option<PlatformTarget>,
}

/// Faithful launch-tag decoding through A2's shared parser.
pub struct DecodedLaunchTag {
    pub event_tag_index: usize,
    pub raw: Vec<String>,
    pub parsed: crate::distribution::ParsedLaunchTag,
}

impl Clone for DecodedLaunchTag {
    fn clone(&self) -> Self {
        Self {
            event_tag_index: self.event_tag_index,
            raw: self.raw.clone(),
            parsed: clone_parsed_launch_tag(&self.parsed),
        }
    }
}

impl std::fmt::Debug for DecodedLaunchTag {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("DecodedLaunchTag")
            .field("event_tag_index", &self.event_tag_index)
            .field("raw", &self.raw)
            .field("raw_profile_id", &self.parsed.raw_profile_id)
            .field("valid_profile_id", &self.parsed.valid_profile_id)
            .field("profile", &self.parsed.profile)
            .finish()
    }
}

impl PartialEq for DecodedLaunchTag {
    fn eq(&self, other: &Self) -> bool {
        self.event_tag_index == other.event_tag_index
            && self.raw == other.raw
            && self.parsed.raw_profile_id == other.parsed.raw_profile_id
            && self.parsed.valid_profile_id == other.parsed.valid_profile_id
            && self.parsed.profile == other.parsed.profile
    }
}

impl Eq for DecodedLaunchTag {}

/// A parse or policy defect in claimed authoritative launch metadata.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AuthoritativeLaunchDefect {
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
        rejection: crate::distribution::ProfileRejection,
    },
    DuplicateProfileId {
        profile_id: crate::distribution::ProfileId,
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
        profile_id: crate::distribution::ProfileId,
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
        profile_id: crate::distribution::ProfileId,
    },
    NonGameLaunchDefault {
        event_tag_index: usize,
        profile_id: crate::distribution::ProfileId,
    },
}

fn clone_parsed_launch_tag(
    parsed: &crate::distribution::ParsedLaunchTag,
) -> crate::distribution::ParsedLaunchTag {
    crate::distribution::ParsedLaunchTag {
        raw_profile_id: parsed.raw_profile_id.clone(),
        valid_profile_id: parsed.valid_profile_id.clone(),
        profile: parsed.profile.clone(),
    }
}

/// Decodes authoritative launch metadata directly from a signed event's tags.
///
/// The decoder is evidentiary: it retains every relevant raw tag and reports
/// defects without normalization, repair, deduplication, or top-level failure.
pub fn decode_authoritative_launch_tier(event: &nostr::Event) -> AuthoritativeLaunchTier {
    let claims_tier = event.tags.iter().any(|tag| {
        tag.as_slice()
            .first()
            .is_some_and(|name| name == "package" || name == "launch" || name == "launch_default")
    });
    if !claims_tier {
        return AuthoritativeLaunchTier::Absent;
    }

    let mut tags = Vec::new();
    let mut defects = Vec::new();
    let mut package_indices = Vec::new();
    let mut platform_count = 0;
    let mut declared_targets = Vec::new();
    let mut valid_profile_indices: Vec<(crate::distribution::ProfileId, Vec<usize>)> = Vec::new();
    let mut valid_profiles = Vec::new();
    let mut launch_defaults = Vec::new();
    let mut has_valid_game_profile = false;

    for (event_tag_index, tag) in event.tags.iter().enumerate() {
        let raw = tag.as_slice();
        match raw.first().map(String::as_str) {
            Some("package") => {
                package_indices.push(event_tag_index);
                tags.push(AuthoritativeLaunchTag::Package(decode_package_tag(
                    event_tag_index,
                    raw,
                    &mut defects,
                )));
            }
            Some("platform") => {
                platform_count += 1;
                let decoded = decode_platform_tag(event_tag_index, raw, &mut defects);
                if let Some(target) = decoded.target.clone() {
                    if !declared_targets.contains(&target) {
                        declared_targets.push(target);
                    }
                }
                tags.push(AuthoritativeLaunchTag::Platform(decoded));
            }
            Some("launch") => {
                let decoded = decode_launch_tag(event_tag_index, raw, &mut defects);
                if let Some(profile_id) = decoded.parsed.valid_profile_id.clone() {
                    if let Some((_, indices)) = valid_profile_indices
                        .iter_mut()
                        .find(|(existing, _)| existing == &profile_id)
                    {
                        indices.push(event_tag_index);
                    } else {
                        valid_profile_indices.push((profile_id, vec![event_tag_index]));
                    }
                }
                has_valid_game_profile |= decoded
                    .parsed
                    .profile
                    .as_ref()
                    .is_ok_and(|profile| profile.launch_type == LaunchType::Game);
                if let Ok(profile) = &decoded.parsed.profile {
                    valid_profiles.push(profile.clone());
                }
                tags.push(AuthoritativeLaunchTag::Launch(decoded));
            }
            Some("launch_default") => {
                let decoded = decode_launch_default_tag(event_tag_index, raw, &mut defects);
                launch_defaults.push(decoded.clone());
                tags.push(AuthoritativeLaunchTag::LaunchDefault(decoded));
            }
            _ => {}
        }
    }

    if package_indices.len() != 1 {
        defects.insert(
            0,
            AuthoritativeLaunchDefect::PackageCount {
                actual: package_indices.len(),
                event_tag_indices: package_indices,
            },
        );
    }
    if platform_count == 0 {
        defects.push(AuthoritativeLaunchDefect::MissingPlatform);
    }
    for (profile_id, event_tag_indices) in valid_profile_indices {
        if event_tag_indices.len() > 1 {
            defects.push(AuthoritativeLaunchDefect::DuplicateProfileId {
                profile_id,
                event_tag_indices,
            });
        }
    }
    if !has_valid_game_profile {
        defects.push(AuthoritativeLaunchDefect::NoValidGameProfile);
    }

    let has_legacy = launch_defaults.iter().any(|tag| tag.legacy);
    let has_scoped = launch_defaults.iter().any(|tag| !tag.legacy);
    if has_legacy && has_scoped {
        defects.push(AuthoritativeLaunchDefect::MixedLaunchDefaultScopes {
            event_tag_indices: launch_defaults
                .iter()
                .map(|tag| tag.event_tag_index)
                .collect(),
        });
    }

    let mut profile_claims = Vec::<(crate::distribution::ProfileId, Vec<usize>)>::new();
    let mut target_claims = Vec::<(PlatformTarget, Vec<usize>)>::new();
    let mut publisher_defaults = Vec::new();
    for launch_default in &launch_defaults {
        let Some(profile_id) = launch_default.valid_profile_id.as_ref() else {
            continue;
        };
        if let Some((_, indices)) = profile_claims
            .iter_mut()
            .find(|(existing, _)| existing == profile_id)
        {
            indices.push(launch_default.event_tag_index);
        } else {
            profile_claims.push((profile_id.clone(), vec![launch_default.event_tag_index]));
        }

        let valid_game = match valid_profiles
            .iter()
            .find(|profile| &profile.id == profile_id)
        {
            None => {
                defects.push(AuthoritativeLaunchDefect::DanglingLaunchDefault {
                    event_tag_index: launch_default.event_tag_index,
                    profile_id: profile_id.clone(),
                });
                false
            }
            Some(profile) if profile.launch_type != LaunchType::Game => {
                defects.push(AuthoritativeLaunchDefect::NonGameLaunchDefault {
                    event_tag_index: launch_default.event_tag_index,
                    profile_id: profile_id.clone(),
                });
                false
            }
            Some(_) => true,
        };

        let effective_targets = if launch_default.legacy {
            declared_targets.clone()
        } else {
            launch_default.targets.clone()
        };
        let mut retained_targets = Vec::new();
        for target in effective_targets {
            if !declared_targets.contains(&target) {
                defects.push(AuthoritativeLaunchDefect::UndeclaredLaunchDefaultTarget {
                    event_tag_index: launch_default.event_tag_index,
                    target,
                });
                continue;
            }
            if !retained_targets.contains(&target) {
                retained_targets.push(target.clone());
            }
            if let Some((_, indices)) = target_claims
                .iter_mut()
                .find(|(existing, _)| existing == &target)
            {
                indices.push(launch_default.event_tag_index);
            } else {
                target_claims.push((target, vec![launch_default.event_tag_index]));
            }
        }
        if valid_game && !retained_targets.is_empty() {
            publisher_defaults.push(PublisherLaunchDefault {
                profile_id: profile_id.clone(),
                targets: retained_targets,
            });
        }
    }
    for (profile_id, event_tag_indices) in profile_claims {
        if event_tag_indices.len() > 1 {
            defects.push(AuthoritativeLaunchDefect::DuplicateLaunchDefaultProfile {
                profile_id,
                event_tag_indices,
            });
        }
    }
    for (target, event_tag_indices) in &target_claims {
        if event_tag_indices.len() > 1 {
            defects.push(AuthoritativeLaunchDefect::OverlappingLaunchDefaultTarget {
                target: target.clone(),
                event_tag_indices: event_tag_indices.clone(),
            });
        }
    }
    if !launch_defaults.is_empty() {
        for target in declared_targets {
            if !target_claims.iter().any(|(claimed, _)| claimed == &target) {
                defects.push(AuthoritativeLaunchDefect::UncoveredLaunchDefaultTarget { target });
            }
        }
    }

    AuthoritativeLaunchTier::Present(AuthoritativeLaunchMetadata {
        tags,
        defects,
        publisher_defaults,
    })
}

impl From<AuthoritativeLaunchTier> for AuthoritativeAdpEditMetadata {
    fn from(tier: AuthoritativeLaunchTier) -> Self {
        match tier {
            AuthoritativeLaunchTier::Absent => Self::Absent,
            AuthoritativeLaunchTier::Present(metadata) => {
                Self::Present(AuthoritativeAdpEditMetadataPresent {
                    tags: metadata.tags.into_iter().map(Into::into).collect(),
                    defects: metadata.defects.into_iter().map(Into::into).collect(),
                    publisher_defaults: metadata.publisher_defaults,
                })
            }
        }
    }
}

impl From<AuthoritativeLaunchTag> for AuthoritativeAdpEditTag {
    fn from(tag: AuthoritativeLaunchTag) -> Self {
        match tag {
            AuthoritativeLaunchTag::Package(tag) => Self::Package {
                event_tag_index: tag.event_tag_index,
                raw: tag.raw,
                package_format: tag.package_format,
            },
            AuthoritativeLaunchTag::Platform(tag) => Self::Platform {
                event_tag_index: tag.event_tag_index,
                raw: tag.raw,
                raw_platform: tag.raw_platform,
                target: tag.target,
            },
            AuthoritativeLaunchTag::Launch(tag) => {
                let (profile, rejection) = match tag.parsed.profile {
                    Ok(profile) => (Some(profile), None),
                    Err(rejection) => (None, Some(rejection.into())),
                };
                Self::Launch {
                    event_tag_index: tag.event_tag_index,
                    raw: tag.raw,
                    raw_profile_id: tag.parsed.raw_profile_id,
                    valid_profile_id: tag.parsed.valid_profile_id,
                    profile,
                    rejection,
                }
            }
            AuthoritativeLaunchTag::LaunchDefault(tag) => Self::LaunchDefault {
                event_tag_index: tag.event_tag_index,
                raw: tag.raw,
                raw_profile_id: tag.raw_profile_id,
                valid_profile_id: tag.valid_profile_id,
                raw_targets: tag.raw_targets,
                targets: tag.targets,
                legacy: tag.legacy,
            },
        }
    }
}

impl From<AuthoritativeLaunchDefect> for AuthoritativeAdpEditDefect {
    fn from(defect: AuthoritativeLaunchDefect) -> Self {
        match defect {
            AuthoritativeLaunchDefect::PackageCount {
                actual,
                event_tag_indices,
            } => Self::PackageCount {
                actual,
                event_tag_indices,
            },
            AuthoritativeLaunchDefect::MalformedPackage { event_tag_index } => {
                Self::MalformedPackage { event_tag_index }
            }
            AuthoritativeLaunchDefect::UnsupportedPackage {
                event_tag_index,
                raw_format,
            } => Self::UnsupportedPackage {
                event_tag_index,
                raw_format,
            },
            AuthoritativeLaunchDefect::MissingPlatform => Self::MissingPlatform,
            AuthoritativeLaunchDefect::MalformedPlatform { event_tag_index } => {
                Self::MalformedPlatform { event_tag_index }
            }
            AuthoritativeLaunchDefect::InvalidPlatform {
                event_tag_index,
                raw_platform,
            } => Self::InvalidPlatform {
                event_tag_index,
                raw_platform,
            },
            AuthoritativeLaunchDefect::NonCanonicalPlatform {
                event_tag_index,
                raw_platform,
                canonical,
            } => Self::NonCanonicalPlatform {
                event_tag_index,
                raw_platform,
                canonical,
            },
            AuthoritativeLaunchDefect::MalformedLaunch {
                event_tag_index,
                rejection,
            } => Self::MalformedLaunch {
                event_tag_index,
                rejection: rejection.into(),
            },
            AuthoritativeLaunchDefect::DuplicateProfileId {
                profile_id,
                event_tag_indices,
            } => Self::DuplicateProfileId {
                profile_id,
                event_tag_indices,
            },
            AuthoritativeLaunchDefect::UnknownLaunchType {
                event_tag_index,
                raw_launch_type,
            } => Self::UnknownLaunchType {
                event_tag_index,
                raw_launch_type,
            },
            AuthoritativeLaunchDefect::NoValidGameProfile => Self::NoValidGameProfile,
            AuthoritativeLaunchDefect::MalformedLaunchDefault {
                event_tag_index,
                raw_profile_id,
            } => Self::MalformedLaunchDefault {
                event_tag_index,
                raw_profile_id,
            },
            AuthoritativeLaunchDefect::DuplicateLaunchDefaultProfile {
                profile_id,
                event_tag_indices,
            } => Self::DuplicateLaunchDefaultProfile {
                profile_id,
                event_tag_indices,
            },
            AuthoritativeLaunchDefect::InvalidLaunchDefaultTarget {
                event_tag_index,
                raw_target,
            } => Self::InvalidLaunchDefaultTarget {
                event_tag_index,
                raw_target,
            },
            AuthoritativeLaunchDefect::NonCanonicalLaunchDefaultTarget {
                event_tag_index,
                raw_target,
                canonical,
            } => Self::NonCanonicalLaunchDefaultTarget {
                event_tag_index,
                raw_target,
                canonical,
            },
            AuthoritativeLaunchDefect::UndeclaredLaunchDefaultTarget {
                event_tag_index,
                target,
            } => Self::UndeclaredLaunchDefaultTarget {
                event_tag_index,
                target,
            },
            AuthoritativeLaunchDefect::OverlappingLaunchDefaultTarget {
                target,
                event_tag_indices,
            } => Self::OverlappingLaunchDefaultTarget {
                target,
                event_tag_indices,
            },
            AuthoritativeLaunchDefect::UncoveredLaunchDefaultTarget { target } => {
                Self::UncoveredLaunchDefaultTarget { target }
            }
            AuthoritativeLaunchDefect::MixedLaunchDefaultScopes { event_tag_indices } => {
                Self::MixedLaunchDefaultScopes { event_tag_indices }
            }
            AuthoritativeLaunchDefect::DanglingLaunchDefault {
                event_tag_index,
                profile_id,
            } => Self::DanglingLaunchDefault {
                event_tag_index,
                profile_id,
            },
            AuthoritativeLaunchDefect::NonGameLaunchDefault {
                event_tag_index,
                profile_id,
            } => Self::NonGameLaunchDefault {
                event_tag_index,
                profile_id,
            },
        }
    }
}

impl From<crate::distribution::ProfileRejection> for AuthoritativeProfileRejection {
    fn from(rejection: crate::distribution::ProfileRejection) -> Self {
        match rejection {
            crate::distribution::ProfileRejection::MissingRequiredElement => {
                Self::MissingRequiredElement
            }
            crate::distribution::ProfileRejection::ProfileId(reason) => {
                Self::ProfileId(reason.into())
            }
            crate::distribution::ProfileRejection::Executable(reason) => {
                Self::Executable(reason.into())
            }
            crate::distribution::ProfileRejection::WorkingDir(reason) => {
                Self::WorkingDir(reason.into())
            }
            crate::distribution::ProfileRejection::Arguments(reason) => {
                Self::Arguments(reason.into())
            }
            crate::distribution::ProfileRejection::DisplayName(reason) => {
                Self::DisplayName(reason.into())
            }
        }
    }
}

impl From<crate::distribution::ProfileIdError> for ProfileIdRejection {
    fn from(reason: crate::distribution::ProfileIdError) -> Self {
        match reason {
            crate::distribution::ProfileIdError::InvalidGrammar => Self::InvalidGrammar,
        }
    }
}

impl From<crate::distribution::PathError> for PathRejection {
    fn from(reason: crate::distribution::PathError) -> Self {
        match reason {
            crate::distribution::PathError::Rule1Absolute => Self::Rule1Absolute,
            crate::distribution::PathError::Rule2ParentSegment => Self::Rule2ParentSegment,
            crate::distribution::PathError::Rule3Backslash => Self::Rule3Backslash,
            crate::distribution::PathError::Rule4ColonOrPrefix => Self::Rule4ColonOrPrefix,
            crate::distribution::PathError::Rule5ControlCharacter => Self::Rule5ControlCharacter,
            crate::distribution::PathError::Rule6EmptyOrDotSegment => Self::Rule6EmptyOrDotSegment,
            crate::distribution::PathError::Rule7TrailingSpaceOrPeriod => {
                Self::Rule7TrailingSpaceOrPeriod
            }
            crate::distribution::PathError::Rule8ReservedDeviceName => {
                Self::Rule8ReservedDeviceName
            }
            crate::distribution::PathError::Rule9LengthExceeded => Self::Rule9LengthExceeded,
        }
    }
}

impl From<crate::distribution::ArgumentError> for ArgumentRejection {
    fn from(reason: crate::distribution::ArgumentError) -> Self {
        match reason {
            crate::distribution::ArgumentError::InvalidJsonShape => Self::InvalidJsonShape,
            crate::distribution::ArgumentError::TooMany => Self::TooMany,
            crate::distribution::ArgumentError::ElementTooLong => Self::ElementTooLong,
            crate::distribution::ArgumentError::ContainsNul => Self::ContainsNul,
        }
    }
}

impl From<crate::distribution::DisplayNameError> for DisplayNameRejection {
    fn from(reason: crate::distribution::DisplayNameError) -> Self {
        match reason {
            crate::distribution::DisplayNameError::InvalidLength => Self::InvalidLength,
            crate::distribution::DisplayNameError::ControlCharacter => Self::ControlCharacter,
        }
    }
}

fn decode_package_tag(
    event_tag_index: usize,
    raw: &[String],
    defects: &mut Vec<AuthoritativeLaunchDefect>,
) -> DecodedPackageTag {
    let raw_format = raw.get(1);
    let package_format = raw_format.and_then(|format| match format.as_str() {
        "zip" => Some(PackageFormat::Zip),
        "tar.zst" => Some(PackageFormat::TarZstd),
        "tar.gz" => Some(PackageFormat::TarGz),
        _ => None,
    });
    if raw.len() != 2 {
        defects.push(AuthoritativeLaunchDefect::MalformedPackage { event_tag_index });
    }
    if let Some(format) = raw_format.filter(|_| package_format.is_none()) {
        defects.push(AuthoritativeLaunchDefect::UnsupportedPackage {
            event_tag_index,
            raw_format: format.clone(),
        });
    }
    DecodedPackageTag {
        event_tag_index,
        raw: raw.to_vec(),
        package_format,
    }
}

fn decode_platform_tag(
    event_tag_index: usize,
    raw: &[String],
    defects: &mut Vec<AuthoritativeLaunchDefect>,
) -> DecodedPlatformTag {
    let raw_platform = raw.get(1).cloned();
    let target = raw_platform
        .as_deref()
        .and_then(crate::distribution::parse_platform_tag);
    if raw.len() != 2 {
        defects.push(AuthoritativeLaunchDefect::MalformedPlatform { event_tag_index });
    }
    match (&raw_platform, &target) {
        (Some(raw_platform), Some(target)) => {
            let canonical = format!("{}-{}", target.os, target.arch);
            if raw_platform != &canonical {
                defects.push(AuthoritativeLaunchDefect::NonCanonicalPlatform {
                    event_tag_index,
                    raw_platform: raw_platform.clone(),
                    canonical,
                });
            }
        }
        _ => defects.push(AuthoritativeLaunchDefect::InvalidPlatform {
            event_tag_index,
            raw_platform: raw_platform.clone(),
        }),
    }
    DecodedPlatformTag {
        event_tag_index,
        raw: raw.to_vec(),
        raw_platform,
        target,
    }
}

fn decode_launch_tag(
    event_tag_index: usize,
    raw: &[String],
    defects: &mut Vec<AuthoritativeLaunchDefect>,
) -> DecodedLaunchTag {
    let parsed = crate::distribution::parse_launch_tag(raw);
    if let Err(rejection) = &parsed.profile {
        defects.push(AuthoritativeLaunchDefect::MalformedLaunch {
            event_tag_index,
            rejection: rejection.clone(),
        });
    }
    if let Some(raw_launch_type) = raw.get(5).filter(|value| {
        !matches!(
            value.as_str(),
            "game" | "config" | "server" | "editor" | "tool"
        )
    }) {
        defects.push(AuthoritativeLaunchDefect::UnknownLaunchType {
            event_tag_index,
            raw_launch_type: raw_launch_type.clone(),
        });
    }
    DecodedLaunchTag {
        event_tag_index,
        raw: raw.to_vec(),
        parsed,
    }
}

fn decode_launch_default_tag(
    event_tag_index: usize,
    raw: &[String],
    defects: &mut Vec<AuthoritativeLaunchDefect>,
) -> DecodedLaunchDefaultTag {
    let raw_profile_id = raw.get(1).cloned();
    let valid_profile_id = raw_profile_id
        .as_deref()
        .and_then(|value| crate::distribution::ProfileId::parse(value).ok());
    let legacy = raw.len() == 2;
    if raw.len() < 2 || valid_profile_id.is_none() {
        defects.push(AuthoritativeLaunchDefect::MalformedLaunchDefault {
            event_tag_index,
            raw_profile_id: raw_profile_id.clone(),
        });
    }
    let raw_targets = raw.iter().skip(2).cloned().collect::<Vec<_>>();
    let mut targets = Vec::new();
    for raw_target in &raw_targets {
        let Some(target) = crate::distribution::parse_platform_tag(raw_target) else {
            defects.push(AuthoritativeLaunchDefect::InvalidLaunchDefaultTarget {
                event_tag_index,
                raw_target: raw_target.clone(),
            });
            continue;
        };
        let canonical = format!("{}-{}", target.os, target.arch);
        if raw_target != &canonical {
            defects.push(AuthoritativeLaunchDefect::NonCanonicalLaunchDefaultTarget {
                event_tag_index,
                raw_target: raw_target.clone(),
                canonical,
            });
        }
        targets.push(target);
    }

    DecodedLaunchDefaultTag {
        event_tag_index,
        raw: raw.to_vec(),
        raw_profile_id,
        valid_profile_id,
        raw_targets,
        targets,
        legacy,
    }
}

/// Errors returned while building ADP publish events.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum AdpPublishError {
    /// Priced ADP listings require a listing-level LUD-16 address.
    #[error("priced ADP listing requires lud16 tag")]
    MissingLud16,
    /// Fulfillment fields must be provided together.
    #[error("fulfillment tier is incomplete; missing: {missing}")]
    IncompleteFulfillmentTier { missing: String },
    /// Server tags must be absolute HTTP(S) URLs.
    #[error("malformed ADP server URL: {url}")]
    MalformedServerUrl { url: String },
    /// Timed access requires a non-empty interval.
    #[error("timed access end must be after its start")]
    InvalidTimedAccess,
    /// A tag preserved from the current listing could not be reconstructed.
    #[error("malformed preserved listing tag")]
    MalformedPreservedTag,
    /// Hosted image fields must be absolute HTTP(S) URLs.
    #[error("malformed image URL: {url}")]
    MalformedImageUrl { url: String },
    /// Platform values must be unique `<os>-<arch>` tags.
    #[error("malformed or duplicate platform tag: {platform}")]
    InvalidPlatform { platform: String },
    /// A typed launch target contains an unsupported operating system.
    #[error("unsupported launch platform operating system: {os}")]
    UnsupportedPlatformOs { os: String },
    /// A typed launch target contains an unsupported architecture.
    #[error("unsupported launch platform architecture: {arch}")]
    UnsupportedPlatformArch { arch: String },
    /// A launch profile contains an invalid display name.
    #[error("invalid display name for launch profile {profile_id}: {source}")]
    InvalidLaunchDisplayName {
        profile_id: String,
        source: crate::distribution::DisplayNameError,
    },
    /// A launch profile contains invalid arguments.
    #[error("invalid arguments for launch profile {profile_id}: {source}")]
    InvalidLaunchArguments {
        profile_id: String,
        source: crate::distribution::ArgumentError,
    },
    /// A launch profile's arguments could not be serialized.
    #[error("could not serialize arguments for launch profile {profile_id}")]
    LaunchArgumentsSerialization { profile_id: String },
    /// A claimed launch tier must contain exactly one package tag.
    #[error("launch tier requires exactly one package tag; found {actual}")]
    LaunchPackageCount { actual: usize },
    /// A package tag must contain exactly its name and format.
    #[error("malformed package tag")]
    MalformedPackageTag,
    /// A package format must be one of the canonical archive tokens.
    #[error("unsupported package format: {format}")]
    UnsupportedPackageFormat { format: String },
    /// A launch tag must contain exactly seven positional elements.
    #[error("launch profile {profile_id} has {actual} elements; expected 7")]
    InvalidLaunchTagLength { profile_id: String, actual: usize },
    /// A launch profile failed an A2 field validator.
    #[error("malformed launch profile {profile_id}: {rejection}")]
    MalformedLaunchProfile {
        profile_id: String,
        rejection: crate::distribution::ProfileRejection,
    },
    /// Launch types emitted for publication must be known lowercase tokens.
    #[error("unsupported launch type for profile {profile_id}: {launch_type}")]
    UnsupportedLaunchType {
        profile_id: String,
        launch_type: String,
    },
    /// Syntactically valid profile IDs must be unique in the final tags.
    #[error("duplicate launch profile ID")]
    DuplicateLaunchProfileId {
        profile_id: crate::distribution::ProfileId,
    },
    /// Final launch-tier platform tags must parse to a known target.
    #[error("invalid launch platform tag: {platform}")]
    InvalidLaunchPlatform { platform: String },
    /// Final launch-tier platform tags must already use canonical spelling.
    #[error("noncanonical launch platform tag {platform}; expected {canonical}")]
    NonCanonicalLaunchPlatform { platform: String, canonical: String },
    /// A claimed launch tier must declare at least one canonical platform.
    #[error("launch tier requires at least one platform tag")]
    MissingLaunchPlatform,
    /// A claimed launch tier must contain at least one valid game profile.
    #[error("launch tier requires at least one valid game launch profile")]
    MissingGameLaunchProfile,
    /// Final launchable publications must select publisher defaults for their targets.
    #[error("launch tier requires at least one launch_default tag")]
    MissingLaunchDefault,
    /// Every typed mapping must cover at least one target.
    #[error("launch_default for profile {profile_id:?} has no targets")]
    EmptyLaunchDefaultTargets {
        profile_id: crate::distribution::ProfileId,
    },
    /// A publisher-default tag must contain one valid profile ID.
    #[error("malformed launch_default tag")]
    MalformedLaunchDefault,
    /// A profile may occur in only one publisher-default mapping.
    #[error("launch_default profile appears more than once: {profile_id:?}")]
    DuplicateLaunchDefaultProfile {
        profile_id: crate::distribution::ProfileId,
    },
    /// Scoped target values must use the known platform vocabulary.
    #[error("invalid launch_default target: {target}")]
    InvalidLaunchDefaultTarget { target: String },
    /// Scoped target values must already use canonical spelling.
    #[error("noncanonical launch_default target {target}; expected {canonical}")]
    NonCanonicalLaunchDefaultTarget { target: String, canonical: String },
    /// Scoped targets must also be listing platform targets.
    #[error("launch_default target is not declared by the listing: {target:?}")]
    UndeclaredLaunchDefaultTarget { target: PlatformTarget },
    /// Each listing target must be claimed by exactly one mapping.
    #[error("launch_default target is claimed more than once: {target:?}")]
    OverlappingLaunchDefaultTarget { target: PlatformTarget },
    /// Every listing target must be claimed.
    #[error("launch_default target is uncovered: {target:?}")]
    UncoveredLaunchDefaultTarget { target: PlatformTarget },
    /// Legacy global and scoped launch-default forms cannot coexist.
    #[error("legacy and scoped launch_default tags cannot be mixed")]
    MixedLaunchDefaultScopes,
    /// Multi-target publications must spell out scoped targets.
    #[error("legacy launch_default cannot be used for a multi-target listing")]
    LegacyLaunchDefaultForMultiplePlatforms,
    /// A publisher-default must name a retained launch profile.
    #[error("launch_default references missing profile {profile_id:?}")]
    DanglingLaunchDefault {
        profile_id: crate::distribution::ProfileId,
    },
    /// A publisher-default must name a game profile.
    #[error("launch_default references non-game profile {profile_id:?}")]
    NonGameLaunchDefault {
        profile_id: crate::distribution::ProfileId,
    },
    /// Fulfillment hashes must be SHA-256 hex.
    #[error("malformed SHA-256 file hash")]
    InvalidFileHash,
    /// Campaign pointers must contain a valid event ID and optional relay URL.
    #[error("malformed campaign pointer")]
    InvalidCampaignPointer,
    /// NIP-94 links must contain a valid event ID.
    #[error("malformed NIP-94 event ID")]
    InvalidNip94EventId,
    /// Authorization references must contain a root, key, and optional relay URL.
    #[error("malformed fulfillment authorization reference")]
    InvalidFulfillmentAuthorization,
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

fn is_platform_tag(value: &str) -> bool {
    let Some((os, arch)) = value.split_once('-') else {
        return false;
    };
    !os.is_empty()
        && !arch.is_empty()
        && !arch.contains('-')
        && os
            .chars()
            .chain(arch.chars())
            .all(|character| character.is_ascii_alphanumeric() || character == '_')
}

/// Builds a developer-signed fulfillment authorization root.
pub fn build_fulfillment_authorization_event_builder(
    terms: &AuthorizationTerms,
) -> Result<EventBuilder, AdpPublishError> {
    if terms.authorization_id.is_empty()
        || !terms.capabilities.iter().any(|capability| {
            matches!(
                capability.as_str(),
                CAPABILITY_ISSUE_RECEIPT | CAPABILITY_ISSUE_GRANT | CAPABILITY_UPLOAD_BUILD
            )
        })
    {
        return Err(AdpPublishError::InvalidFulfillmentAuthorization);
    }
    let mut tags = vec![
        Tag::custom(
            TagKind::Custom("d".into()),
            [terms.authorization_id.clone()],
        ),
        Tag::custom(TagKind::Custom("a".into()), [terms.coordinate.clone()]),
        Tag::custom(TagKind::p(), [terms.operator_pubkey.to_hex()]),
        Tag::custom(
            TagKind::Custom("fulfillment_pubkey".into()),
            [terms.fulfillment_pubkey.to_hex()],
        ),
    ];
    for capability in &terms.capabilities {
        tags.push(Tag::custom(
            TagKind::Custom("capability".into()),
            [capability.clone()],
        ));
    }
    tags.extend([
        Tag::custom(
            TagKind::Custom("valid_from".into()),
            [terms.valid_from.to_string()],
        ),
        Tag::custom(TagKind::Custom("status".into()), ["active"]),
    ]);
    Ok(
        EventBuilder::new(Kind::Custom(FULFILLMENT_AUTHORIZATION_KIND), "")
            .tags(tags)
            .custom_created_at(nostr::Timestamp::from_secs(terms.valid_from)),
    )
}

fn package_format_tag_value(package_format: &PackageFormat) -> &'static str {
    match package_format {
        PackageFormat::Zip => "zip",
        PackageFormat::TarZstd => "tar.zst",
        PackageFormat::TarGz => "tar.gz",
    }
}

fn launch_type_tag_value(launch_type: &LaunchType) -> &'static str {
    match launch_type {
        LaunchType::Game => "game",
        LaunchType::Config => "config",
        LaunchType::Server => "server",
        LaunchType::Editor => "editor",
        LaunchType::Tool => "tool",
    }
}

fn canonical_launch_platforms(targets: &[PlatformTarget]) -> Result<Vec<String>, AdpPublishError> {
    let mut seen = HashSet::new();
    targets
        .iter()
        .map(|target| {
            let os = crate::distribution::normalize_os(&target.os).ok_or_else(|| {
                AdpPublishError::UnsupportedPlatformOs {
                    os: target.os.clone(),
                }
            })?;
            let arch = crate::distribution::normalize_arch(&target.arch).ok_or_else(|| {
                AdpPublishError::UnsupportedPlatformArch {
                    arch: target.arch.clone(),
                }
            })?;
            let platform = format!("{os}-{arch}");
            if !seen.insert(platform.clone()) {
                return Err(AdpPublishError::InvalidPlatform { platform });
            }
            Ok(platform)
        })
        .collect()
}

fn build_launch_tier_tags(launch_tier: &AdpLaunchTierInput) -> Result<Vec<Tag>, AdpPublishError> {
    let platforms = canonical_launch_platforms(&launch_tier.platforms)?;
    if platforms.is_empty() {
        return Err(AdpPublishError::MissingLaunchPlatform);
    }
    let mut tags = Vec::with_capacity(
        1 + platforms.len()
            + launch_tier.launch_profiles.len()
            + launch_tier.publisher_defaults.len(),
    );
    tags.push(Tag::custom(
        TagKind::Custom("package".into()),
        [package_format_tag_value(&launch_tier.package_format)],
    ));
    tags.extend(
        platforms
            .iter()
            .cloned()
            .map(|platform| Tag::custom(TagKind::Custom("platform".into()), [platform])),
    );

    let mut launch_profile_ids = HashSet::new();
    for profile in &launch_tier.launch_profiles {
        if !launch_profile_ids.insert(profile.id.as_str()) {
            return Err(AdpPublishError::DuplicateLaunchProfileId {
                profile_id: profile.id.clone(),
            });
        }
        crate::distribution::validation::validate_display_name(&profile.display_name).map_err(
            |source| AdpPublishError::InvalidLaunchDisplayName {
                profile_id: profile.id.as_str().to_owned(),
                source,
            },
        )?;
        crate::distribution::validation::validate_arguments(&profile.arguments).map_err(
            |source| AdpPublishError::InvalidLaunchArguments {
                profile_id: profile.id.as_str().to_owned(),
                source,
            },
        )?;
        let arguments = serde_json::to_string(&profile.arguments).map_err(|_| {
            AdpPublishError::LaunchArgumentsSerialization {
                profile_id: profile.id.as_str().to_owned(),
            }
        })?;
        tags.push(Tag::custom(
            TagKind::Custom("launch".into()),
            [
                profile.id.as_str().to_owned(),
                profile.executable.as_str().to_owned(),
                profile.working_dir.as_str().to_owned(),
                arguments,
                launch_type_tag_value(&profile.launch_type).to_owned(),
                profile.display_name.clone(),
            ],
        ));
    }

    if launch_tier.publisher_defaults.is_empty() {
        return Err(AdpPublishError::MissingLaunchDefault);
    }
    let declared_targets = platforms
        .iter()
        .filter_map(|platform| crate::distribution::parse_platform_tag(platform))
        .collect::<Vec<_>>();
    let mut seen_profiles = HashSet::new();
    let mut covered_targets = Vec::new();
    let mut defaults = Vec::with_capacity(launch_tier.publisher_defaults.len());
    for default in &launch_tier.publisher_defaults {
        if !seen_profiles.insert(default.profile_id.as_str()) {
            return Err(AdpPublishError::DuplicateLaunchDefaultProfile {
                profile_id: default.profile_id.clone(),
            });
        }
        if default.targets.is_empty() {
            return Err(AdpPublishError::EmptyLaunchDefaultTargets {
                profile_id: default.profile_id.clone(),
            });
        }
        match launch_tier
            .launch_profiles
            .iter()
            .find(|profile| profile.id == default.profile_id)
        {
            None => {
                return Err(AdpPublishError::DanglingLaunchDefault {
                    profile_id: default.profile_id.clone(),
                });
            }
            Some(profile) if profile.launch_type != LaunchType::Game => {
                return Err(AdpPublishError::NonGameLaunchDefault {
                    profile_id: default.profile_id.clone(),
                });
            }
            Some(_) => {}
        }

        let mut scoped_targets = Vec::with_capacity(default.targets.len());
        for target in &default.targets {
            let raw = format!("{}-{}", target.os, target.arch);
            let parsed = crate::distribution::parse_platform_tag(&raw).ok_or_else(|| {
                AdpPublishError::InvalidLaunchDefaultTarget {
                    target: raw.clone(),
                }
            })?;
            let canonical = format!("{}-{}", parsed.os, parsed.arch);
            if raw != canonical {
                return Err(AdpPublishError::NonCanonicalLaunchDefaultTarget {
                    target: raw,
                    canonical,
                });
            }
            if !declared_targets.contains(&parsed) {
                return Err(AdpPublishError::UndeclaredLaunchDefaultTarget { target: parsed });
            }
            if covered_targets.contains(&parsed) {
                return Err(AdpPublishError::OverlappingLaunchDefaultTarget { target: parsed });
            }
            covered_targets.push(parsed);
            scoped_targets.push(canonical);
        }
        defaults.push((default.profile_id.as_str().to_owned(), scoped_targets));
    }
    if let Some(target) = declared_targets
        .iter()
        .find(|target| !covered_targets.contains(target))
    {
        return Err(AdpPublishError::UncoveredLaunchDefaultTarget {
            target: target.clone(),
        });
    }

    if platforms.len() == 1 && defaults.len() == 1 {
        tags.push(Tag::custom(
            TagKind::Custom("launch_default".into()),
            [defaults[0].0.clone()],
        ));
    } else {
        for (profile_id, scoped_targets) in defaults {
            let mut values = Vec::with_capacity(1 + scoped_targets.len());
            values.push(profile_id);
            values.extend(scoped_targets);
            tags.push(Tag::custom(
                TagKind::Custom("launch_default".into()),
                values,
            ));
        }
    }

    Ok(tags)
}

/// Validates launch-tier invariants over an unsigned event's exact final tags.
///
/// Platform-only legacy metadata does not claim the launch tier. A `package`,
/// `launch`, or `launch_default` tag activates publication-time launch checks.
///
/// # Errors
///
/// Returns a deterministic [`AdpPublishError`] for the first violated final-tag
/// invariant. Duplicate valid profile IDs are checked before other launch fields.
pub fn validate_adp_listing_unsigned_event(
    event: &nostr::UnsignedEvent,
) -> Result<(), AdpPublishError> {
    let tags = event.tags.iter().map(Tag::as_slice).collect::<Vec<_>>();
    let package_tags = named_final_tags(&tags, "package");
    let launch_tags = named_final_tags(&tags, "launch");
    let launch_default_tags = named_final_tags(&tags, "launch_default");

    if package_tags.is_empty() && launch_tags.is_empty() && launch_default_tags.is_empty() {
        return Ok(());
    }
    if package_tags.len() != 1 {
        return Err(AdpPublishError::LaunchPackageCount {
            actual: package_tags.len(),
        });
    }

    let Some(package_tag) = package_tags.first().copied() else {
        return Err(AdpPublishError::LaunchPackageCount { actual: 0 });
    };
    validate_final_package_tag(package_tag)?;
    let parsed_launch_tags = launch_tags
        .iter()
        .map(|tag| crate::distribution::parse_launch_tag(tag))
        .collect::<Vec<_>>();
    validate_unique_final_profile_ids(&parsed_launch_tags)?;

    let mut has_game_profile = false;
    for (tag, parsed) in launch_tags.into_iter().zip(&parsed_launch_tags) {
        has_game_profile |= validate_final_launch_tag(tag, parsed)?;
    }
    if !has_game_profile {
        return Err(AdpPublishError::MissingGameLaunchProfile);
    }
    let platform_tags = named_final_tags(&tags, "platform");
    if platform_tags.is_empty() {
        return Err(AdpPublishError::MissingLaunchPlatform);
    }
    let mut platforms = Vec::with_capacity(platform_tags.len());
    for tag in platform_tags {
        let target = validate_final_platform_tag(tag)?;
        if platforms.contains(&target) {
            return Err(AdpPublishError::InvalidLaunchPlatform {
                platform: format!("{}-{}", target.os, target.arch),
            });
        }
        platforms.push(target);
    }
    validate_final_launch_defaults(&launch_default_tags, &parsed_launch_tags, &platforms)?;

    Ok(())
}

fn validate_final_launch_defaults(
    tags: &[&[String]],
    launch_tags: &[crate::distribution::ParsedLaunchTag],
    platforms: &[PlatformTarget],
) -> Result<(), AdpPublishError> {
    if tags.is_empty() {
        return Err(AdpPublishError::MissingLaunchDefault);
    }
    let has_legacy = tags.iter().any(|tag| tag.len() == 2);
    let has_scoped = tags.iter().any(|tag| tag.len() > 2);
    if has_legacy && has_scoped {
        return Err(AdpPublishError::MixedLaunchDefaultScopes);
    }
    if has_legacy && platforms.len() > 1 {
        return Err(AdpPublishError::LegacyLaunchDefaultForMultiplePlatforms);
    }

    let mut seen_profiles = HashSet::new();
    let mut covered_targets = Vec::new();
    for tag in tags {
        let Some(raw_profile_id) = tag.get(1) else {
            return Err(AdpPublishError::MalformedLaunchDefault);
        };
        let profile_id = crate::distribution::ProfileId::parse(raw_profile_id)
            .map_err(|_| AdpPublishError::MalformedLaunchDefault)?;
        if !seen_profiles.insert(profile_id.as_str().to_owned()) {
            return Err(AdpPublishError::DuplicateLaunchDefaultProfile { profile_id });
        }
        let profile = launch_tags
            .iter()
            .filter_map(|tag| tag.profile.as_ref().ok())
            .find(|profile| profile.id == profile_id)
            .ok_or_else(|| AdpPublishError::DanglingLaunchDefault {
                profile_id: profile_id.clone(),
            })?;
        if profile.launch_type != LaunchType::Game {
            return Err(AdpPublishError::NonGameLaunchDefault { profile_id });
        }

        let targets = if tag.len() == 2 {
            platforms.to_vec()
        } else {
            let mut targets = Vec::with_capacity(tag.len() - 2);
            for raw_target in tag.iter().skip(2) {
                let target =
                    crate::distribution::parse_platform_tag(raw_target).ok_or_else(|| {
                        AdpPublishError::InvalidLaunchDefaultTarget {
                            target: raw_target.clone(),
                        }
                    })?;
                let canonical = format!("{}-{}", target.os, target.arch);
                if raw_target != &canonical {
                    return Err(AdpPublishError::NonCanonicalLaunchDefaultTarget {
                        target: raw_target.clone(),
                        canonical,
                    });
                }
                targets.push(target);
            }
            targets
        };
        for target in targets {
            if !platforms.contains(&target) {
                return Err(AdpPublishError::UndeclaredLaunchDefaultTarget { target });
            }
            if covered_targets.contains(&target) {
                return Err(AdpPublishError::OverlappingLaunchDefaultTarget { target });
            }
            covered_targets.push(target);
        }
    }
    if let Some(target) = platforms
        .iter()
        .find(|target| !covered_targets.contains(target))
    {
        return Err(AdpPublishError::UncoveredLaunchDefaultTarget {
            target: target.clone(),
        });
    }
    Ok(())
}

fn named_final_tags<'a>(tags: &'a [&'a [String]], name: &str) -> Vec<&'a [String]> {
    tags.iter()
        .copied()
        .filter(|tag| tag.first().is_some_and(|value| value == name))
        .collect()
}

fn validate_final_package_tag(tag: &[String]) -> Result<(), AdpPublishError> {
    let [_, format] = tag else {
        return Err(AdpPublishError::MalformedPackageTag);
    };
    if matches!(format.as_str(), "zip" | "tar.zst" | "tar.gz") {
        Ok(())
    } else {
        Err(AdpPublishError::UnsupportedPackageFormat {
            format: format.clone(),
        })
    }
}

fn validate_unique_final_profile_ids(
    tags: &[crate::distribution::ParsedLaunchTag],
) -> Result<(), AdpPublishError> {
    let mut profile_ids = HashSet::new();
    for tag in tags {
        if let Some(profile_id) = tag.valid_profile_id.as_ref() {
            if !profile_ids.insert(profile_id.as_str()) {
                return Err(AdpPublishError::DuplicateLaunchProfileId {
                    profile_id: profile_id.clone(),
                });
            }
        }
    }
    Ok(())
}

impl From<AdpPublishError> for PublishAdpListingError {
    fn from(error: AdpPublishError) -> Self {
        let message = error.to_string();
        let violation = match error {
            AdpPublishError::MissingLud16 => AdpPublishValidationViolation::MissingLud16,
            AdpPublishError::IncompleteFulfillmentTier { missing } => {
                AdpPublishValidationViolation::IncompleteFulfillmentTier { missing }
            }
            AdpPublishError::MalformedServerUrl { url } => {
                AdpPublishValidationViolation::MalformedServerUrl { url }
            }
            AdpPublishError::InvalidTimedAccess => {
                AdpPublishValidationViolation::InvalidTimedAccess
            }
            AdpPublishError::MalformedPreservedTag => {
                AdpPublishValidationViolation::MalformedPreservedTag
            }
            AdpPublishError::MalformedImageUrl { url } => {
                AdpPublishValidationViolation::MalformedImageUrl { url }
            }
            AdpPublishError::InvalidPlatform { platform } => {
                AdpPublishValidationViolation::InvalidPlatform { platform }
            }
            AdpPublishError::UnsupportedPlatformOs { os } => {
                AdpPublishValidationViolation::UnsupportedPlatformOs { os }
            }
            AdpPublishError::UnsupportedPlatformArch { arch } => {
                AdpPublishValidationViolation::UnsupportedPlatformArch { arch }
            }
            AdpPublishError::InvalidLaunchDisplayName { profile_id, source } => {
                AdpPublishValidationViolation::InvalidLaunchDisplayName {
                    profile_id,
                    reason: source.into(),
                }
            }
            AdpPublishError::InvalidLaunchArguments { profile_id, source } => {
                AdpPublishValidationViolation::InvalidLaunchArguments {
                    profile_id,
                    reason: source.into(),
                }
            }
            AdpPublishError::LaunchArgumentsSerialization { profile_id } => {
                AdpPublishValidationViolation::LaunchArgumentsSerialization { profile_id }
            }
            AdpPublishError::LaunchPackageCount { actual } => {
                AdpPublishValidationViolation::LaunchPackageCount { actual }
            }
            AdpPublishError::MalformedPackageTag => {
                AdpPublishValidationViolation::MalformedPackageTag
            }
            AdpPublishError::UnsupportedPackageFormat { format } => {
                AdpPublishValidationViolation::UnsupportedPackageFormat { format }
            }
            AdpPublishError::InvalidLaunchTagLength { profile_id, actual } => {
                AdpPublishValidationViolation::InvalidLaunchTagLength { profile_id, actual }
            }
            AdpPublishError::MalformedLaunchProfile {
                profile_id,
                rejection,
            } => AdpPublishValidationViolation::MalformedLaunchProfile {
                profile_id,
                rejection: rejection.into(),
            },
            AdpPublishError::UnsupportedLaunchType {
                profile_id,
                launch_type,
            } => AdpPublishValidationViolation::UnsupportedLaunchType {
                profile_id,
                launch_type,
            },
            AdpPublishError::DuplicateLaunchProfileId { profile_id } => {
                AdpPublishValidationViolation::DuplicateLaunchProfileId { profile_id }
            }
            AdpPublishError::InvalidLaunchPlatform { platform } => {
                AdpPublishValidationViolation::InvalidLaunchPlatform { platform }
            }
            AdpPublishError::NonCanonicalLaunchPlatform {
                platform,
                canonical,
            } => AdpPublishValidationViolation::NonCanonicalLaunchPlatform {
                platform,
                canonical,
            },
            AdpPublishError::MissingLaunchPlatform => {
                AdpPublishValidationViolation::MissingLaunchPlatform
            }
            AdpPublishError::MissingGameLaunchProfile => {
                AdpPublishValidationViolation::MissingGameLaunchProfile
            }
            AdpPublishError::MissingLaunchDefault => {
                AdpPublishValidationViolation::MissingLaunchDefault
            }
            AdpPublishError::EmptyLaunchDefaultTargets { profile_id } => {
                AdpPublishValidationViolation::EmptyLaunchDefaultTargets { profile_id }
            }
            AdpPublishError::MalformedLaunchDefault => {
                AdpPublishValidationViolation::MalformedLaunchDefault
            }
            AdpPublishError::DuplicateLaunchDefaultProfile { profile_id } => {
                AdpPublishValidationViolation::DuplicateLaunchDefaultProfile { profile_id }
            }
            AdpPublishError::InvalidLaunchDefaultTarget { target } => {
                AdpPublishValidationViolation::InvalidLaunchDefaultTarget { target }
            }
            AdpPublishError::NonCanonicalLaunchDefaultTarget { target, canonical } => {
                AdpPublishValidationViolation::NonCanonicalLaunchDefaultTarget { target, canonical }
            }
            AdpPublishError::UndeclaredLaunchDefaultTarget { target } => {
                AdpPublishValidationViolation::UndeclaredLaunchDefaultTarget { target }
            }
            AdpPublishError::OverlappingLaunchDefaultTarget { target } => {
                AdpPublishValidationViolation::OverlappingLaunchDefaultTarget { target }
            }
            AdpPublishError::UncoveredLaunchDefaultTarget { target } => {
                AdpPublishValidationViolation::UncoveredLaunchDefaultTarget { target }
            }
            AdpPublishError::MixedLaunchDefaultScopes => {
                AdpPublishValidationViolation::MixedLaunchDefaultScopes
            }
            AdpPublishError::LegacyLaunchDefaultForMultiplePlatforms => {
                AdpPublishValidationViolation::LegacyLaunchDefaultForMultiplePlatforms
            }
            AdpPublishError::DanglingLaunchDefault { profile_id } => {
                AdpPublishValidationViolation::DanglingLaunchDefault { profile_id }
            }
            AdpPublishError::NonGameLaunchDefault { profile_id } => {
                AdpPublishValidationViolation::NonGameLaunchDefault { profile_id }
            }
            AdpPublishError::InvalidFileHash => AdpPublishValidationViolation::InvalidFileHash,
            AdpPublishError::InvalidCampaignPointer => {
                AdpPublishValidationViolation::InvalidCampaignPointer
            }
            AdpPublishError::InvalidNip94EventId => {
                AdpPublishValidationViolation::InvalidNip94EventId
            }
            AdpPublishError::InvalidFulfillmentAuthorization => {
                AdpPublishValidationViolation::InvalidFulfillmentAuthorization
            }
        };
        PublishAdpListingError::CoreValidation { violation, message }
    }
}

fn validate_final_launch_tag(
    tag: &[String],
    parsed: &crate::distribution::ParsedLaunchTag,
) -> Result<bool, AdpPublishError> {
    let raw_profile_id = parsed.raw_profile_id.clone().unwrap_or_default();
    let [_, profile_id, _, _, _, launch_type, _] = tag else {
        return Err(AdpPublishError::InvalidLaunchTagLength {
            profile_id: raw_profile_id,
            actual: tag.len(),
        });
    };

    if let Err(rejection) = &parsed.profile {
        if matches!(
            rejection,
            crate::distribution::ProfileRejection::DisplayName(_)
        ) && !matches!(
            launch_type.as_str(),
            "game" | "config" | "server" | "editor" | "tool"
        ) {
            return Err(AdpPublishError::UnsupportedLaunchType {
                profile_id: profile_id.clone(),
                launch_type: launch_type.clone(),
            });
        }
        return Err(AdpPublishError::MalformedLaunchProfile {
            profile_id: profile_id.clone(),
            rejection: rejection.clone(),
        });
    }

    if !matches!(
        launch_type.as_str(),
        "game" | "config" | "server" | "editor" | "tool"
    ) {
        return Err(AdpPublishError::UnsupportedLaunchType {
            profile_id: profile_id.clone(),
            launch_type: launch_type.clone(),
        });
    }

    Ok(launch_type == "game")
}

fn validate_final_platform_tag(tag: &[String]) -> Result<PlatformTarget, AdpPublishError> {
    let [_, platform] = tag else {
        return Err(AdpPublishError::InvalidLaunchPlatform {
            platform: tag.get(1).cloned().unwrap_or_default(),
        });
    };
    let target = crate::distribution::parse_platform_tag(platform).ok_or_else(|| {
        AdpPublishError::InvalidLaunchPlatform {
            platform: platform.clone(),
        }
    })?;
    let canonical = format!("{}-{}", target.os, target.arch);
    if platform != &canonical {
        return Err(AdpPublishError::NonCanonicalLaunchPlatform {
            platform: platform.clone(),
            canonical,
        });
    }
    Ok(target)
}

/// Replaces only managed `adp_terms` tags after explicit publisher acceptance.
/// Unrelated extension tags are retained across authoritative replacements/retries.
pub fn apply_managed_adp_terms(
    input: &mut AdpListingInput,
    servers: &[crate::adp_settlement::ServerOperator],
    accepted: &[crate::adp_settlement::AcceptedTermsRef],
) -> Result<(), crate::adp_settlement::SettlementError> {
    let accepted = crate::adp_settlement::validate_terms_coverage(servers, accepted)?;
    for reference in &accepted {
        if nostr::PublicKey::from_hex(&reference.operator_pubkey).is_err()
            || nostr::EventId::from_hex(&reference.terms_event_id).is_err()
        {
            return Err(crate::adp_settlement::SettlementError::TermsCoverage(
                reference.operator_pubkey.clone(),
            ));
        }
    }
    input
        .preserved_tags
        .retain(|tag| !tag.first().is_some_and(|name| name == "adp_terms"));
    input
        .preserved_tags
        .extend(accepted.into_iter().map(|reference| {
            vec![
                "adp_terms".to_string(),
                reference.operator_pubkey,
                reference.terms_event_id,
            ]
        }));
    Ok(())
}

/// Builds the developer-signed `kind:30402` listing event.
///
/// # Errors
/// Returns [`AdpPublishError::MissingLud16`] when `price_sats > 0` and no LUD-16 is provided.
/// Returns [`AdpPublishError::IncompleteFulfillmentTier`] when only part of the fulfillment tier is present.
/// Returns [`AdpPublishError::MalformedServerUrl`] when a declared server is not an HTTP(S) URL.
/// Returns [`AdpPublishError::InvalidTimedAccess`] when a timed policy has an empty interval.
pub fn build_adp_listing_event_builder<'a>(
    input: impl Into<AdpListingPublishInput<'a>>,
) -> Result<EventBuilder, AdpPublishError> {
    let input = input.into();
    let launch_tier_tags = input.launch_tier.map(build_launch_tier_tags).transpose()?;
    let input = input.listing;

    if input.price_sats > 0 && input.lud16.as_deref().unwrap_or_default().is_empty() {
        return Err(AdpPublishError::MissingLud16);
    }

    let fulfillment_fields = [
        ("server", !input.servers.is_empty()),
        (
            "file_hash",
            input
                .file_hash
                .as_deref()
                .is_some_and(|value| !value.is_empty()),
        ),
        (
            "version",
            input
                .version
                .as_deref()
                .is_some_and(|value| !value.is_empty()),
        ),
    ];
    let has_any_fulfillment = fulfillment_fields.iter().any(|(_, present)| *present)
        || !input.fulfillment_authorizations.is_empty();
    let missing_fulfillment = fulfillment_fields
        .iter()
        .filter_map(|(name, present)| (!present).then_some(*name))
        .collect::<Vec<_>>();
    if has_any_fulfillment && !missing_fulfillment.is_empty() {
        return Err(AdpPublishError::IncompleteFulfillmentTier {
            missing: missing_fulfillment.join(", "),
        });
    }

    for server in &input.servers {
        if !is_http_url(server) {
            return Err(AdpPublishError::MalformedServerUrl {
                url: server.clone(),
            });
        }
    }

    if matches!(
        input.acquisition,
        AcquisitionPolicy::TimedAccess { starts_at, ends_at } if starts_at >= ends_at
    ) {
        return Err(AdpPublishError::InvalidTimedAccess);
    }

    for image in &input.images {
        if !image.is_empty() && !is_http_url(image) {
            return Err(AdpPublishError::MalformedImageUrl { url: image.clone() });
        }
    }

    if launch_tier_tags.is_none() {
        let mut platforms = HashSet::new();
        for platform in &input.platforms {
            if !is_platform_tag(platform) || !platforms.insert(platform) {
                return Err(AdpPublishError::InvalidPlatform {
                    platform: platform.clone(),
                });
            }
        }
    }

    if input
        .file_hash
        .as_deref()
        .is_some_and(|hash| crate::distribution::Sha256Digest::parse(hash).is_err())
    {
        return Err(AdpPublishError::InvalidFileHash);
    }

    for (root_event_id, relay_hint) in &input.campaigns {
        if nostr::EventId::from_hex(root_event_id).is_err()
            || relay_hint
                .as_deref()
                .is_some_and(|relay| nostr::RelayUrl::parse(relay).is_err())
        {
            return Err(AdpPublishError::InvalidCampaignPointer);
        }
    }

    for authorization in &input.fulfillment_authorizations {
        if nostr::EventId::from_hex(&authorization.root_event_id).is_err()
            || nostr::PublicKey::from_hex(&authorization.fulfillment_pubkey).is_err()
            || authorization
                .relay_hint
                .as_deref()
                .is_some_and(|relay| nostr::RelayUrl::parse(relay).is_err())
        {
            return Err(AdpPublishError::InvalidFulfillmentAuthorization);
        }
    }

    if input
        .nip94_event_id
        .as_deref()
        .is_some_and(|event_id| nostr::EventId::from_hex(event_id).is_err())
    {
        return Err(AdpPublishError::InvalidNip94EventId);
    }

    let mut tags = vec![
        Tag::custom(TagKind::Custom("d".into()), [input.d_tag.clone()]),
        Tag::custom(TagKind::Custom("title".into()), [input.title.clone()]),
        Tag::custom(
            TagKind::Custom("price".into()),
            [input.price_sats.to_string(), "sat".to_string()],
        ),
        Tag::custom(TagKind::Custom("t".into()), ["game".to_string()]),
    ];

    let mut seen_fulfillment_authorizations = HashSet::new();
    for values in &input.preserved_tags {
        if launch_tier_tags.is_some() && values.first().is_some_and(|name| name == "launch_default")
        {
            continue;
        }
        if values
            .first()
            .is_some_and(|name| name == "fulfillment_authorization")
            && !seen_fulfillment_authorizations.insert(values.clone())
        {
            continue;
        }
        tags.push(Tag::parse(values.clone()).map_err(|_| AdpPublishError::MalformedPreservedTag)?);
    }

    tags.push(match &input.acquisition {
        AcquisitionPolicy::Gated => Tag::custom(TagKind::Custom("acquisition".into()), ["gated"]),
        AcquisitionPolicy::Public => Tag::custom(TagKind::Custom("acquisition".into()), ["public"]),
        AcquisitionPolicy::TimedAccess { starts_at, ends_at } => Tag::custom(
            TagKind::Custom("acquisition".into()),
            [
                "timed-access".to_string(),
                starts_at.to_string(),
                ends_at.to_string(),
            ],
        ),
    });

    for image in &input.images {
        if !image.is_empty() {
            tags.push(Tag::custom(
                TagKind::Custom("image".into()),
                [image.clone()],
            ));
        }
    }

    let mut topic_tags = HashSet::from(["game".to_string()]);
    for tag in &input.tags {
        let tag = tag.trim();
        if !tag.is_empty() && topic_tags.insert(tag.to_ascii_lowercase()) {
            tags.push(Tag::custom(TagKind::Custom("t".into()), [tag.to_string()]));
        }
    }

    if has_any_fulfillment {
        for server in &input.servers {
            tags.push(Tag::custom(
                TagKind::Custom("server".into()),
                [server.clone()],
            ));
        }
        tags.push(Tag::custom(
            TagKind::Custom("file_hash".into()),
            [input.file_hash.clone().expect("file_hash checked above")],
        ));
        tags.push(Tag::custom(
            TagKind::Custom("version".into()),
            [input.version.clone().expect("version checked above")],
        ));
        for authorization in &input.fulfillment_authorizations {
            let mut values = vec![
                authorization.root_event_id.clone(),
                authorization.fulfillment_pubkey.clone(),
            ];
            if let Some(relay_hint) = authorization.relay_hint.as_ref() {
                values.push(relay_hint.clone());
            }
            let mut raw_values = vec!["fulfillment_authorization".to_string()];
            raw_values.extend(values.clone());
            if seen_fulfillment_authorizations.insert(raw_values) {
                tags.push(Tag::custom(
                    TagKind::Custom("fulfillment_authorization".into()),
                    values,
                ));
            }
        }
    }

    if let Some(lud16) = &input.lud16 {
        if !lud16.is_empty() {
            tags.push(Tag::custom(
                TagKind::Custom("lud16".into()),
                [lud16.clone()],
            ));
        }
    }

    if let Some(launch_tier_tags) = launch_tier_tags {
        tags.extend(launch_tier_tags);
    } else {
        for platform in &input.platforms {
            tags.push(Tag::custom(
                TagKind::Custom("platform".into()),
                [platform.clone()],
            ));
        }
    }

    for (root_event_id, relay_hint) in &input.campaigns {
        if root_event_id.is_empty() {
            continue;
        }
        let mut values = vec![root_event_id.clone()];
        if let Some(relay_hint) = relay_hint.as_ref().filter(|value| !value.is_empty()) {
            values.push(relay_hint.clone());
        }
        tags.push(Tag::custom(TagKind::Custom("campaign".into()), values));
    }

    if let Some(nip94_event_id) = input
        .nip94_event_id
        .as_ref()
        .filter(|value| !value.is_empty())
    {
        tags.push(Tag::custom(
            TagKind::Custom("nip94".into()),
            [nip94_event_id.clone()],
        ));
    }

    Ok(EventBuilder::new(Kind::Custom(30402), input.description.clone()).tags(tags))
}

#[cfg(test)]
mod canonical_hex_tests {
    use nostr::Keys;

    use super::*;

    const LOWERCASE_SHA256: &str =
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const UPPERCASE_SHA256: &str =
        "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";

    #[test]
    fn listing_publish_rejects_uppercase_hash_and_emits_lowercase_equivalent() {
        let mut input = listing_input(UPPERCASE_SHA256);
        assert_eq!(
            build_adp_listing_event_builder(&input),
            Err(AdpPublishError::InvalidFileHash)
        );

        input.file_hash = Some(LOWERCASE_SHA256.to_owned());
        let event = build_adp_listing_event_builder(&input)
            .expect("lowercase digest is publishable")
            .sign_with_keys(&Keys::generate())
            .expect("listing signs");
        let emitted_hash = event
            .tags
            .iter()
            .map(nostr::Tag::as_slice)
            .find(|tag| tag.first().is_some_and(|name| name == "file_hash"))
            .and_then(|tag| tag.get(1));

        assert_eq!(emitted_hash.map(String::as_str), Some(LOWERCASE_SHA256));
    }

    fn listing_input(file_hash: &str) -> AdpListingInput {
        AdpListingInput {
            d_tag: "game".to_owned(),
            title: "Game".to_owned(),
            description: "Description".to_owned(),
            price_sats: 0,
            lud16: None,
            tags: Vec::new(),
            images: Vec::new(),
            servers: vec!["https://dist.example.com".to_owned()],
            file_hash: Some(file_hash.to_owned()),
            version: Some("1.0.0".to_owned()),
            fulfillment_authorizations: Vec::new(),
            acquisition: AcquisitionPolicy::Gated,
            platforms: Vec::new(),
            campaigns: Vec::new(),
            nip94_event_id: None,
            preserved_tags: Vec::new(),
        }
    }
}

#[cfg(test)]
mod launch_tier_tests {
    use nostr::Keys;

    use super::*;
    use crate::distribution::{resolve_install_plan, ProfileId, RelativePath, WorkingDir};

    const SHA256: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    #[test]
    fn signed_listing_round_trips_typed_launch_tier() {
        let profiles = three_profiles();
        let mut input = listing_input();
        input.platforms = vec!["linux-aarch64".to_owned()];
        input.preserved_tags = vec![
            vec!["launch_default".to_owned(), "configure".to_owned()],
            vec!["launch_default".to_owned(), "play".to_owned()],
        ];
        let launch_tier = AdpLaunchTierInput {
            package_format: PackageFormat::Zip,
            platforms: vec![PlatformTarget {
                os: "WINDOWS".to_owned(),
                arch: "X64".to_owned(),
            }],
            launch_profiles: profiles.clone(),
            publisher_defaults: vec![PublisherLaunchDefault {
                profile_id: ProfileId::parse("play").expect("valid default profile ID"),
                targets: vec![PlatformTarget {
                    os: "windows".to_owned(),
                    arch: "x86_64".to_owned(),
                }],
            }],
        };

        let event = build_adp_listing_event_builder(AdpListingPublishInput {
            listing: &input,
            launch_tier: Some(&launch_tier),
        })
        .expect("typed listing builds")
        .sign_with_keys(&Keys::generate())
        .expect("listing signs");
        assert_eq!(event.kind, Kind::Custom(30_402));
        assert!(event.verify().is_ok());

        let outcome = resolve_install_plan(
            &event,
            &PlatformTarget {
                os: "windows".to_owned(),
                arch: "x86_64".to_owned(),
            },
        )
        .expect("canonical host resolves the alias-authored target");
        assert_eq!(outcome.plan.artifact.package_format, PackageFormat::Zip);
        assert_eq!(outcome.plan.launch_profiles, profiles);
        assert_eq!(
            outcome.plan.publisher_default_profile_id.as_ref(),
            Some(&launch_tier.publisher_defaults[0].profile_id)
        );
        assert!(outcome.discarded_profiles.is_empty());

        let raw_tags = event
            .tags
            .iter()
            .map(nostr::Tag::as_slice)
            .collect::<Vec<_>>();
        let package_tags = named_tags(&raw_tags, "package");
        assert_eq!(package_tags.len(), 1);
        assert_eq!(package_tags[0], ["package", "zip"]);
        let platform_tags = named_tags(&raw_tags, "platform");
        assert_eq!(platform_tags.len(), 1);
        assert_eq!(platform_tags[0], ["platform", "windows-x86_64"]);

        let launch_tags = named_tags(&raw_tags, "launch");
        assert_eq!(launch_tags.len(), 3);
        for tag in launch_tags {
            assert_eq!(tag.len(), 7);
            let arguments = tag.get(4).expect("launch arguments are positional");
            assert!(serde_json::from_str::<Vec<String>>(arguments).is_ok());
        }
        let launch_default_tags = named_tags(&raw_tags, "launch_default");
        assert_eq!(launch_default_tags.len(), 1);
        assert_eq!(launch_default_tags[0], ["launch_default", "play"]);
    }

    #[test]
    fn unsupported_typed_platform_returns_exact_adapter_error() {
        let input = listing_input();
        let launch_tier = AdpLaunchTierInput {
            package_format: PackageFormat::Zip,
            platforms: vec![PlatformTarget {
                os: "haiku".to_owned(),
                arch: "X64".to_owned(),
            }],
            launch_profiles: three_profiles(),
            publisher_defaults: vec![PublisherLaunchDefault {
                profile_id: ProfileId::parse("play").expect("valid default profile ID"),
                targets: vec![PlatformTarget {
                    os: "linux".to_owned(),
                    arch: "x86_64".to_owned(),
                }],
            }],
        };

        assert_eq!(
            build_adp_listing_event_builder(AdpListingPublishInput {
                listing: &input,
                launch_tier: Some(&launch_tier),
            }),
            Err(AdpPublishError::UnsupportedPlatformOs {
                os: "haiku".to_owned(),
            })
        );
    }

    #[test]
    fn multi_target_builder_emits_explicit_non_overlapping_scopes() {
        let input = listing_input();
        let mut profiles = three_profiles();
        profiles[2].launch_type = LaunchType::Game;
        let launch_tier = AdpLaunchTierInput {
            package_format: PackageFormat::Zip,
            platforms: vec![
                PlatformTarget {
                    os: "linux".to_owned(),
                    arch: "x86_64".to_owned(),
                },
                PlatformTarget {
                    os: "linux".to_owned(),
                    arch: "aarch64".to_owned(),
                },
            ],
            launch_profiles: profiles,
            publisher_defaults: vec![
                PublisherLaunchDefault {
                    profile_id: ProfileId::parse("play").expect("valid profile ID"),
                    targets: vec![PlatformTarget {
                        os: "linux".to_owned(),
                        arch: "x86_64".to_owned(),
                    }],
                },
                PublisherLaunchDefault {
                    profile_id: ProfileId::parse("diagnostics").expect("valid profile ID"),
                    targets: vec![PlatformTarget {
                        os: "linux".to_owned(),
                        arch: "aarch64".to_owned(),
                    }],
                },
            ],
        };

        let unsigned = build_adp_listing_event_builder(AdpListingPublishInput {
            listing: &input,
            launch_tier: Some(&launch_tier),
        })
        .expect("multi-target listing builds")
        .build(Keys::generate().public_key());
        let defaults = unsigned
            .tags
            .iter()
            .map(Tag::as_slice)
            .filter(|tag| tag.first().is_some_and(|name| name == "launch_default"))
            .collect::<Vec<_>>();
        assert_eq!(defaults.len(), 2);
        assert_eq!(defaults[0], ["launch_default", "play", "linux-x86_64"]);
        assert_eq!(
            defaults[1],
            ["launch_default", "diagnostics", "linux-aarch64"]
        );
        assert_eq!(validate_adp_listing_unsigned_event(&unsigned), Ok(()));
    }

    #[test]
    fn download_only_input_without_launch_tier_still_builds() {
        let mut input = listing_input();
        input.servers.clear();
        input.file_hash = None;
        input.version = None;
        input.platforms = vec!["linux-x86_64".to_owned()];

        assert!(build_adp_listing_event_builder(&input).is_ok());
    }

    fn named_tags<'a>(tags: &'a [&'a [String]], name: &str) -> Vec<&'a [String]> {
        tags.iter()
            .copied()
            .filter(|tag| tag.first().is_some_and(|value| value == name))
            .collect()
    }

    fn three_profiles() -> Vec<LaunchProfile> {
        vec![
            LaunchProfile {
                id: ProfileId::parse("play").expect("valid profile ID"),
                display_name: "Play Game".to_owned(),
                executable: RelativePath::parse("bin/Game.exe").expect("valid executable"),
                working_dir: WorkingDir::parse("bin").expect("valid working directory"),
                arguments: vec!["--mode".to_owned(), "arcade mode".to_owned()],
                launch_type: LaunchType::Game,
            },
            LaunchProfile {
                id: ProfileId::parse("configure").expect("valid profile ID"),
                display_name: "Configure".to_owned(),
                executable: RelativePath::parse("Config.exe").expect("valid executable"),
                working_dir: WorkingDir::parse(".").expect("valid root working directory"),
                arguments: Vec::new(),
                launch_type: LaunchType::Config,
            },
            LaunchProfile {
                id: ProfileId::parse("diagnostics").expect("valid profile ID"),
                display_name: "Diagnostics Tool".to_owned(),
                executable: RelativePath::parse("tools/Diagnostics.exe").expect("valid executable"),
                working_dir: WorkingDir::parse("tools").expect("valid working directory"),
                arguments: vec!["--report=json".to_owned()],
                launch_type: LaunchType::Tool,
            },
        ]
    }

    fn listing_input() -> AdpListingInput {
        AdpListingInput {
            d_tag: "typed-launch-game".to_owned(),
            title: "Typed Launch Game".to_owned(),
            description: "A launch-tier round-trip fixture".to_owned(),
            price_sats: 0,
            lud16: None,
            tags: Vec::new(),
            images: Vec::new(),
            servers: vec!["https://dist.example.com".to_owned()],
            file_hash: Some(SHA256.to_owned()),
            version: Some("1.0.0".to_owned()),
            fulfillment_authorizations: Vec::new(),
            acquisition: AcquisitionPolicy::Gated,
            platforms: Vec::new(),
            campaigns: Vec::new(),
            nip94_event_id: None,
            preserved_tags: Vec::new(),
        }
    }
}

#[cfg(test)]
mod final_tag_validation_tests {
    use nostr::{EventBuilder, PublicKey, Tag};

    use super::*;
    use crate::distribution::{
        ArgumentError, DisplayNameError, PathError, ProfileId, ProfileIdError, ProfileRejection,
    };

    #[test]
    fn download_only_tags_pass_even_with_legacy_platform_metadata() {
        let event = unsigned_event(vec![
            vec!["d", "download-only"],
            vec!["platform", "win-x64"],
        ]);

        assert_eq!(validate_adp_listing_unsigned_event(&event), Ok(()));
    }

    #[test]
    fn complete_launch_tier_passes() {
        let event = unsigned_event(complete_tags());

        assert_eq!(validate_adp_listing_unsigned_event(&event), Ok(()));
    }

    #[test]
    fn package_only_reports_missing_game_before_missing_platform() {
        let event = unsigned_event(vec![vec!["package", "zip"]]);

        assert_eq!(
            validate_adp_listing_unsigned_event(&event),
            Err(AdpPublishError::MissingGameLaunchProfile)
        );
    }

    #[test]
    fn launch_only_reports_missing_package() {
        let event = unsigned_event(vec![valid_game_launch()]);

        assert_eq!(
            validate_adp_listing_unsigned_event(&event),
            Err(AdpPublishError::LaunchPackageCount { actual: 0 })
        );
    }

    #[test]
    fn valid_package_and_game_without_platform_reports_missing_platform() {
        let event = unsigned_event(vec![
            vec!["package", "zip"],
            valid_game_launch(),
            valid_launch_default(),
        ]);

        assert_eq!(
            validate_adp_listing_unsigned_event(&event),
            Err(AdpPublishError::MissingLaunchPlatform)
        );
    }

    #[test]
    fn multiple_packages_report_exact_count() {
        let event = unsigned_event(vec![
            vec!["package", "zip"],
            vec!["package", "tar.gz"],
            vec!["platform", "linux-x86_64"],
            valid_game_launch(),
        ]);

        assert_eq!(
            validate_adp_listing_unsigned_event(&event),
            Err(AdpPublishError::LaunchPackageCount { actual: 2 })
        );
    }

    #[test]
    fn malformed_and_unsupported_packages_are_distinct() {
        let malformed = unsigned_event(vec![
            vec!["package"],
            vec!["platform", "linux-x86_64"],
            valid_game_launch(),
        ]);
        assert_eq!(
            validate_adp_listing_unsigned_event(&malformed),
            Err(AdpPublishError::MalformedPackageTag)
        );

        let unsupported = unsigned_event(vec![
            vec!["package", "rar"],
            vec!["platform", "linux-x86_64"],
            valid_game_launch(),
        ]);
        assert_eq!(
            validate_adp_listing_unsigned_event(&unsupported),
            Err(AdpPublishError::UnsupportedPackageFormat {
                format: "rar".to_owned(),
            })
        );
    }

    #[test]
    fn malformed_launch_length_carries_raw_profile_id() {
        let event = unsigned_event(vec![
            vec!["package", "zip"],
            vec!["platform", "linux-x86_64"],
            vec!["launch", "play", "bin/game"],
        ]);

        assert_eq!(
            validate_adp_listing_unsigned_event(&event),
            Err(AdpPublishError::InvalidLaunchTagLength {
                profile_id: "play".to_owned(),
                actual: 3,
            })
        );
    }

    #[test]
    fn malformed_launch_fields_report_exact_structured_rejections() {
        let cases = [
            (
                1,
                "Bad",
                Err(AdpPublishError::MalformedLaunchProfile {
                    profile_id: "Bad".to_owned(),
                    rejection: ProfileRejection::ProfileId(ProfileIdError::InvalidGrammar),
                }),
            ),
            (
                2,
                "../game",
                Err(AdpPublishError::MalformedLaunchProfile {
                    profile_id: "play".to_owned(),
                    rejection: ProfileRejection::Executable(PathError::Rule2ParentSegment),
                }),
            ),
            (
                3,
                "work/./dir",
                Err(AdpPublishError::MalformedLaunchProfile {
                    profile_id: "play".to_owned(),
                    rejection: ProfileRejection::WorkingDir(PathError::Rule6EmptyOrDotSegment),
                }),
            ),
            (
                4,
                "{}",
                Err(AdpPublishError::MalformedLaunchProfile {
                    profile_id: "play".to_owned(),
                    rejection: ProfileRejection::Arguments(ArgumentError::InvalidJsonShape),
                }),
            ),
            (
                6,
                "",
                Err(AdpPublishError::MalformedLaunchProfile {
                    profile_id: "play".to_owned(),
                    rejection: ProfileRejection::DisplayName(DisplayNameError::InvalidLength),
                }),
            ),
        ];

        for (index, value, expected) in cases {
            let mut launch = valid_game_launch();
            launch[index] = value;
            let event = unsigned_event(vec![
                vec!["package", "zip"],
                vec!["platform", "linux-x86_64"],
                launch,
            ]);
            assert_eq!(validate_adp_listing_unsigned_event(&event), expected);
        }

        let mut launch = valid_game_launch();
        launch[5] = "Game";
        let event = unsigned_event(vec![
            vec!["package", "zip"],
            vec!["platform", "linux-x86_64"],
            launch,
        ]);
        assert_eq!(
            validate_adp_listing_unsigned_event(&event),
            Err(AdpPublishError::UnsupportedLaunchType {
                profile_id: "play".to_owned(),
                launch_type: "Game".to_owned(),
            })
        );
    }

    #[test]
    fn duplicate_valid_profile_id_precedes_other_launch_rejections() {
        let duplicate = unsigned_event(vec![
            vec!["package", "zip"],
            vec!["platform", "linux-x86_64"],
            valid_game_launch(),
            valid_game_launch(),
        ]);
        assert_eq!(
            validate_adp_listing_unsigned_event(&duplicate),
            Err(AdpPublishError::DuplicateLaunchProfileId {
                profile_id: crate::distribution::ProfileId::parse("play")
                    .expect("valid profile ID"),
            })
        );

        let malformed_duplicate = unsigned_event(vec![
            vec!["package", "zip"],
            vec!["platform", "linux-x86_64"],
            valid_game_launch(),
            vec!["launch", "play", "../bad"],
        ]);
        assert_eq!(
            validate_adp_listing_unsigned_event(&malformed_duplicate),
            Err(AdpPublishError::DuplicateLaunchProfileId {
                profile_id: crate::distribution::ProfileId::parse("play")
                    .expect("valid profile ID"),
            })
        );
    }

    #[test]
    fn config_and_tool_profiles_do_not_satisfy_game_requirement() {
        let event = unsigned_event(vec![
            vec!["package", "tar.zst"],
            vec!["platform", "macos-aarch64"],
            vec![
                "launch",
                "config",
                "config",
                ".",
                "[]",
                "config",
                "Configure",
            ],
            vec![
                "launch",
                "tools",
                "tools/run",
                "tools",
                "[]",
                "tool",
                "Tools",
            ],
        ]);

        assert_eq!(
            validate_adp_listing_unsigned_event(&event),
            Err(AdpPublishError::MissingGameLaunchProfile)
        );
    }

    #[test]
    fn aliases_and_unknown_platforms_return_distinct_errors() {
        let alias = unsigned_event(vec![
            vec!["package", "tar.gz"],
            vec!["platform", "win-x64"],
            valid_game_launch(),
            valid_launch_default(),
        ]);
        assert_eq!(
            validate_adp_listing_unsigned_event(&alias),
            Err(AdpPublishError::NonCanonicalLaunchPlatform {
                platform: "win-x64".to_owned(),
                canonical: "windows-x86_64".to_owned(),
            })
        );

        let unknown = unsigned_event(vec![
            vec!["package", "zip"],
            vec!["platform", "haiku-x86_64"],
            valid_game_launch(),
            valid_launch_default(),
        ]);
        assert_eq!(
            validate_adp_listing_unsigned_event(&unknown),
            Err(AdpPublishError::InvalidLaunchPlatform {
                platform: "haiku-x86_64".to_owned(),
            })
        );
    }

    #[test]
    fn publisher_default_is_strictly_validated() {
        let base = || {
            vec![
                vec!["package", "zip"],
                vec!["platform", "linux-x86_64"],
                valid_game_launch(),
                vec![
                    "launch",
                    "config",
                    "config",
                    ".",
                    "[]",
                    "config",
                    "Configure",
                ],
            ]
        };

        assert_eq!(
            validate_adp_listing_unsigned_event(&unsigned_event(base())),
            Err(AdpPublishError::MissingLaunchDefault)
        );

        let mut duplicate = base();
        duplicate.extend([valid_launch_default(), valid_launch_default()]);
        assert_eq!(
            validate_adp_listing_unsigned_event(&unsigned_event(duplicate)),
            Err(AdpPublishError::DuplicateLaunchDefaultProfile {
                profile_id: ProfileId::parse("play").expect("valid profile ID"),
            })
        );

        let cases = [
            (
                vec!["launch_default"],
                Err(AdpPublishError::MalformedLaunchDefault),
            ),
            (
                vec!["launch_default", "Bad"],
                Err(AdpPublishError::MalformedLaunchDefault),
            ),
            (
                vec!["launch_default", "play", "extra"],
                Err(AdpPublishError::InvalidLaunchDefaultTarget {
                    target: "extra".to_owned(),
                }),
            ),
            (
                vec!["launch_default", "missing"],
                Err(AdpPublishError::DanglingLaunchDefault {
                    profile_id: ProfileId::parse("missing").expect("valid profile ID"),
                }),
            ),
            (
                vec!["launch_default", "config"],
                Err(AdpPublishError::NonGameLaunchDefault {
                    profile_id: ProfileId::parse("config").expect("valid profile ID"),
                }),
            ),
        ];
        for (launch_default, expected) in cases {
            let mut tags = base();
            tags.push(launch_default);
            assert_eq!(
                validate_adp_listing_unsigned_event(&unsigned_event(tags)),
                expected
            );
        }
    }

    #[test]
    fn scoped_defaults_enforce_canonical_exact_coverage() {
        let base = || {
            vec![
                vec!["package", "zip"],
                vec!["platform", "linux-x86_64"],
                vec!["platform", "linux-aarch64"],
                valid_game_launch(),
                vec![
                    "launch",
                    "arm-play",
                    "bin/arm-game",
                    ".",
                    "[]",
                    "game",
                    "Play ARM",
                ],
            ]
        };

        let mut valid = base();
        valid.extend([
            vec!["launch_default", "play", "linux-x86_64"],
            vec!["launch_default", "arm-play", "linux-aarch64"],
        ]);
        assert_eq!(
            validate_adp_listing_unsigned_event(&unsigned_event(valid)),
            Ok(())
        );

        let mut duplicate_platform = base();
        duplicate_platform.push(vec!["platform", "linux-x86_64"]);
        duplicate_platform.extend([
            vec!["launch_default", "play", "linux-x86_64"],
            vec!["launch_default", "arm-play", "linux-aarch64"],
        ]);
        assert_eq!(
            validate_adp_listing_unsigned_event(&unsigned_event(duplicate_platform)),
            Err(AdpPublishError::InvalidLaunchPlatform {
                platform: "linux-x86_64".to_owned(),
            })
        );

        let cases = [
            (
                vec![vec!["launch_default", "play", "linux-x86_64"]],
                Err(AdpPublishError::UncoveredLaunchDefaultTarget {
                    target: PlatformTarget {
                        os: "linux".to_owned(),
                        arch: "aarch64".to_owned(),
                    },
                }),
            ),
            (
                vec![
                    vec!["launch_default", "play", "linux-x86_64"],
                    vec![
                        "launch_default",
                        "arm-play",
                        "linux-x86_64",
                        "linux-aarch64",
                    ],
                ],
                Err(AdpPublishError::OverlappingLaunchDefaultTarget {
                    target: PlatformTarget {
                        os: "linux".to_owned(),
                        arch: "x86_64".to_owned(),
                    },
                }),
            ),
            (
                vec![vec![
                    "launch_default",
                    "play",
                    "linux-amd64",
                    "linux-aarch64",
                ]],
                Err(AdpPublishError::NonCanonicalLaunchDefaultTarget {
                    target: "linux-amd64".to_owned(),
                    canonical: "linux-x86_64".to_owned(),
                }),
            ),
            (
                vec![vec!["launch_default", "play", "windows-x86_64"]],
                Err(AdpPublishError::UndeclaredLaunchDefaultTarget {
                    target: PlatformTarget {
                        os: "windows".to_owned(),
                        arch: "x86_64".to_owned(),
                    },
                }),
            ),
            (
                vec![vec!["launch_default", "play"]],
                Err(AdpPublishError::LegacyLaunchDefaultForMultiplePlatforms),
            ),
            (
                vec![
                    vec!["launch_default", "play"],
                    vec!["launch_default", "arm-play", "linux-aarch64"],
                ],
                Err(AdpPublishError::MixedLaunchDefaultScopes),
            ),
        ];
        for (defaults, expected) in cases {
            let mut tags = base();
            tags.extend(defaults);
            assert_eq!(
                validate_adp_listing_unsigned_event(&unsigned_event(tags)),
                expected
            );
        }
    }

    fn complete_tags() -> Vec<Vec<&'static str>> {
        vec![
            vec!["package", "tar.zst"],
            vec!["platform", "linux-x86_64"],
            valid_game_launch(),
            vec![
                "launch",
                "server",
                "server/run",
                ".",
                "[]",
                "server",
                "Server",
            ],
            valid_launch_default(),
        ]
    }

    fn valid_game_launch() -> Vec<&'static str> {
        vec![
            "launch",
            "play",
            "bin/game",
            "bin",
            "[\"--fullscreen\"]",
            "game",
            "Play",
        ]
    }

    fn valid_launch_default() -> Vec<&'static str> {
        vec!["launch_default", "play"]
    }

    fn unsigned_event(tags: Vec<Vec<&str>>) -> nostr::UnsignedEvent {
        let tags = tags
            .into_iter()
            .map(|values| {
                Tag::parse(values.into_iter().map(str::to_owned).collect::<Vec<_>>())
                    .expect("raw final tag fixture parses")
            })
            .collect::<Vec<_>>();
        let pubkey =
            PublicKey::from_hex("79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798")
                .expect("generator x-coordinate is a valid public key");

        EventBuilder::new(Kind::Custom(30_402), "")
            .tags(tags)
            .build(pubkey)
    }
}

#[cfg(any())]
mod tests {
    use super::*;
    use crate::authorization::{parse_authorization_event, AuthorizationTransition};
    use nostr::{Keys, Kind};

    const VALID_SHA256: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    fn tag_values(event: &nostr::Event, name: &str) -> Vec<Vec<String>> {
        event
            .tags
            .iter()
            .map(|tag| {
                tag.clone()
                    .to_vec()
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
            })
            .filter(|values| values.first().is_some_and(|kind| kind == name))
            .collect()
    }

    fn minimal_listing_input() -> AdpListingInput {
        AdpListingInput {
            d_tag: "game".into(),
            title: "Game".into(),
            description: "Description".into(),
            price_sats: 0,
            lud16: None,
            tags: Vec::new(),
            images: Vec::new(),
            servers: Vec::new(),
            file_hash: None,
            version: None,
            fulfillment_pubkey: None,
            fulfillment_valid_from: None,
            fulfillment_revoked_at: None,
            acquisition: AcquisitionPolicy::Gated,
            platforms: Vec::new(),
            campaigns: Vec::new(),
            nip94_event_id: None,
            preserved_tags: Vec::new(),
        }
    }

    #[test]
    fn rejects_malformed_phase7_metadata_before_signing() {
        let mut input = minimal_listing_input();
        input.images = vec!["file:///tmp/cover.png".into()];
        assert!(matches!(
            build_adp_listing_event_builder(&input),
            Err(AdpPublishError::MalformedImageUrl { .. })
        ));

        input.images.clear();
        input.platforms = vec!["linux-x86_64".into(), "linux-x86_64".into()];
        assert!(matches!(
            build_adp_listing_event_builder(&input),
            Err(AdpPublishError::InvalidPlatform { .. })
        ));

        input.platforms.clear();
        input.campaigns = vec![("not-an-event-id".into(), None)];
        assert_eq!(
            build_adp_listing_event_builder(&input),
            Err(AdpPublishError::InvalidCampaignPointer)
        );

        input.campaigns.clear();
        input.nip94_event_id = Some("not-an-event-id".into());
        assert_eq!(
            build_adp_listing_event_builder(&input),
            Err(AdpPublishError::InvalidNip94EventId)
        );

        input.nip94_event_id = None;
        input.servers = vec!["https://dist.example.com".into()];
        input.file_hash = Some("not-a-sha256".into());
        input.version = Some("1.0.0".into());
        input.fulfillment_pubkey = Some("fulfillment-key".into());
        assert_eq!(
            build_adp_listing_event_builder(&input),
            Err(AdpPublishError::InvalidFileHash)
        );
    }

    #[tokio::test]
    async fn builds_parseable_fulfillment_authorization_root() {
        let developer = Keys::generate();
        let fulfillment = Keys::generate();
        let authorization_id = "authorization-1";
        let coordinate = format!("30402:{}:game", developer.public_key().to_hex());
        let valid_from = 1_700_000_000;

        let event = build_fulfillment_authorization_event_builder(
            authorization_id,
            &coordinate,
            &fulfillment.public_key().to_hex(),
            valid_from,
        )
        .sign_with_keys(&developer)
        .expect("event should sign");

        assert_eq!(event.kind, Kind::Custom(FULFILLMENT_AUTHORIZATION_KIND));
        assert_eq!(event.tags.len(), 6);

        let parsed = parse_authorization_event(&event).expect("authorization should parse");
        let AuthorizationTransition::ActiveRoot(terms) = parsed.transition else {
            panic!("authorization should be an active root");
        };
        assert_eq!(terms.authorization_id, authorization_id);
        assert_eq!(terms.coordinate, coordinate);
        assert_eq!(terms.fulfillment_pubkey, fulfillment.public_key());
        assert_eq!(terms.valid_from, valid_from);
        assert_eq!(terms.valid_until, None);
    }

    #[tokio::test]
    async fn builds_kind_30402_adp_listing_with_required_tags() {
        let keys = Keys::generate();
        let input = AdpListingInput {
            d_tag: "my-game".to_string(),
            title: "My Game".to_string(),
            description: "Fun game".to_string(),
            price_sats: 2100,
            lud16: Some("studio@example.com".to_string()),
            tags: vec![
                "game".to_string(),
                "arcade".to_string(),
                "arcade".to_string(),
                "GAME".to_string(),
                "nostr".to_string(),
            ],
            images: vec!["https://cdn.example.com/cover.png".to_string()],
            servers: vec![
                "https://dist.example.com".to_string(),
                "https://mirror.example.com".to_string(),
            ],
            file_hash: Some(VALID_SHA256.to_string()),
            version: Some("1.0.0".to_string()),
            fulfillment_pubkey: Some("fulfillment-pubkey".to_string()),
            fulfillment_valid_from: Some(1_725_000_000),
            fulfillment_revoked_at: Some(1_725_000_999),
            acquisition: AcquisitionPolicy::Gated,
            platforms: vec!["linux-x86_64".to_string(), "windows-x86_64".to_string()],
            campaigns: vec![(
                "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string(),
                Some("wss://relay.example.com".to_string()),
            )],
            nip94_event_id: Some(
                "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".to_string(),
            ),
            preserved_tags: vec![vec!["summary".into(), "Short description".into()]],
        };

        let event = build_adp_listing_event_builder(&input)
            .expect("listing builder should be created")
            .sign_with_keys(&keys)
            .expect("event should sign");

        assert_eq!(event.kind, Kind::Custom(30402));
        assert_eq!(
            tag_values(&event, "acquisition")[0],
            vec!["acquisition", "gated"]
        );
        assert_eq!(tag_values(&event, "d")[0], vec!["d", "my-game"]);
        assert_eq!(tag_values(&event, "title")[0], vec!["title", "My Game"]);
        assert_eq!(
            tag_values(&event, "summary")[0],
            vec!["summary", "Short description"]
        );
        assert_eq!(
            tag_values(&event, "campaign")[0],
            vec![
                "campaign",
                "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "wss://relay.example.com"
            ]
        );
        assert_eq!(
            tag_values(&event, "nip94")[0],
            vec![
                "nip94",
                "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
            ]
        );
        assert_eq!(tag_values(&event, "price")[0], vec!["price", "2100", "sat"]);
        assert_eq!(
            tag_values(&event, "image"),
            vec![vec![
                "image".to_string(),
                "https://cdn.example.com/cover.png".to_string()
            ]]
        );
        assert_eq!(
            tag_values(&event, "t"),
            vec![
                vec!["t".to_string(), "game".to_string()],
                vec!["t".to_string(), "arcade".to_string()],
                vec!["t".to_string(), "nostr".to_string()],
            ]
        );
        assert_eq!(
            tag_values(&event, "server"),
            vec![
                vec!["server".to_string(), "https://dist.example.com".to_string()],
                vec![
                    "server".to_string(),
                    "https://mirror.example.com".to_string()
                ],
            ]
        );
        assert_eq!(
            tag_values(&event, "file_hash")[0],
            vec!["file_hash", VALID_SHA256]
        );
        assert_eq!(tag_values(&event, "version")[0], vec!["version", "1.0.0"]);
        assert_eq!(
            tag_values(&event, "fulfillment_pubkey")[0],
            vec![
                "fulfillment_pubkey",
                "fulfillment-pubkey",
                "1725000000",
                "1725000999"
            ]
        );
        assert_eq!(
            tag_values(&event, "lud16")[0],
            vec!["lud16", "studio@example.com"]
        );
        assert_eq!(tag_values(&event, "platform").len(), 2);
    }

    #[tokio::test]
    async fn emits_public_and_timed_acquisition_tags() {
        let keys = Keys::generate();
        let mut input = AdpListingInput {
            d_tag: "public-game".to_string(),
            title: "Public Game".to_string(),
            description: "Publicly accessible".to_string(),
            price_sats: 0,
            lud16: None,
            tags: vec![],
            images: vec![],
            servers: vec![],
            file_hash: None,
            version: None,
            fulfillment_pubkey: None,
            fulfillment_valid_from: None,
            fulfillment_revoked_at: None,
            acquisition: AcquisitionPolicy::Public,
            platforms: vec![],
            campaigns: Vec::new(),
            nip94_event_id: None,
            preserved_tags: Vec::new(),
        };

        let public_event = build_adp_listing_event_builder(&input)
            .expect("public listing should build")
            .sign_with_keys(&keys)
            .expect("public listing should sign");
        assert_eq!(
            tag_values(&public_event, "acquisition")[0],
            vec!["acquisition", "public"]
        );

        input.acquisition = AcquisitionPolicy::TimedAccess {
            starts_at: 100,
            ends_at: 200,
        };
        let timed_event = build_adp_listing_event_builder(&input)
            .expect("timed listing should build")
            .sign_with_keys(&keys)
            .expect("timed listing should sign");
        assert_eq!(
            tag_values(&timed_event, "acquisition")[0],
            vec!["acquisition", "timed-access", "100", "200"]
        );
    }

    #[test]
    fn rejects_invalid_timed_acquisition_window() {
        let input = AdpListingInput {
            d_tag: "invalid-timed-game".to_string(),
            title: "Invalid Timed Game".to_string(),
            description: "Invalid timed access".to_string(),
            price_sats: 0,
            lud16: None,
            tags: vec![],
            images: vec![],
            servers: vec![],
            file_hash: None,
            version: None,
            fulfillment_pubkey: None,
            fulfillment_valid_from: None,
            fulfillment_revoked_at: None,
            acquisition: AcquisitionPolicy::TimedAccess {
                starts_at: 200,
                ends_at: 100,
            },
            platforms: vec![],
            campaigns: Vec::new(),
            nip94_event_id: None,
            preserved_tags: Vec::new(),
        };

        assert_eq!(
            build_adp_listing_event_builder(&input).expect_err("invalid window should fail"),
            AdpPublishError::InvalidTimedAccess
        );
    }

    #[test]
    fn priced_listing_requires_lud16() {
        let input = AdpListingInput {
            d_tag: "my-game".to_string(),
            title: "My Game".to_string(),
            description: "Fun game".to_string(),
            price_sats: 1,
            lud16: None,
            tags: vec![],
            images: vec![],
            servers: vec![],
            file_hash: None,
            version: None,
            fulfillment_pubkey: None,
            fulfillment_valid_from: None,
            fulfillment_revoked_at: None,
            acquisition: AcquisitionPolicy::Gated,
            platforms: vec![],
            campaigns: Vec::new(),
            nip94_event_id: None,
            preserved_tags: Vec::new(),
        };

        let err = build_adp_listing_event_builder(&input)
            .expect_err("priced listing without lud16 should fail");

        assert!(matches!(err, AdpPublishError::MissingLud16));
    }

    #[tokio::test]
    async fn buy_only_paid_listing_with_lud16_emits_no_fulfillment_tags() {
        let keys = Keys::generate();
        let input = AdpListingInput {
            d_tag: "buy-only".to_string(),
            title: "Buy Only".to_string(),
            description: "Purchasable without automated install".to_string(),
            price_sats: 2100,
            lud16: Some("studio@example.com".to_string()),
            tags: vec!["puzzle".to_string()],
            images: vec![],
            servers: vec![],
            file_hash: None,
            version: None,
            fulfillment_pubkey: None,
            fulfillment_valid_from: None,
            fulfillment_revoked_at: None,
            acquisition: AcquisitionPolicy::Gated,
            platforms: vec!["linux-x86_64".to_string()],
            campaigns: Vec::new(),
            nip94_event_id: None,
            preserved_tags: Vec::new(),
        };

        let event = build_adp_listing_event_builder(&input)
            .expect("buy-only listing should be valid")
            .sign_with_keys(&keys)
            .expect("event should sign");

        assert_eq!(event.kind, Kind::Custom(30402));
        assert_eq!(tag_values(&event, "server"), Vec::<Vec<String>>::new());
        assert_eq!(tag_values(&event, "file_hash"), Vec::<Vec<String>>::new());
        assert_eq!(tag_values(&event, "version"), Vec::<Vec<String>>::new());
        assert_eq!(
            tag_values(&event, "fulfillment_pubkey"),
            Vec::<Vec<String>>::new()
        );
        assert_eq!(
            tag_values(&event, "lud16")[0],
            vec!["lud16", "studio@example.com"]
        );
    }

    #[test]
    fn partial_fulfillment_tier_reports_missing_fields() {
        let input = AdpListingInput {
            d_tag: "partial".to_string(),
            title: "Partial".to_string(),
            description: "Missing fulfillment fields".to_string(),
            price_sats: 0,
            lud16: None,
            tags: vec![],
            images: vec![],
            servers: vec!["https://dist.example.com".to_string()],
            file_hash: Some(VALID_SHA256.to_string()),
            version: None,
            fulfillment_pubkey: None,
            fulfillment_valid_from: None,
            fulfillment_revoked_at: None,
            acquisition: AcquisitionPolicy::Gated,
            platforms: vec![],
            campaigns: Vec::new(),
            nip94_event_id: None,
            preserved_tags: Vec::new(),
        };

        let err = build_adp_listing_event_builder(&input)
            .expect_err("partial fulfillment tier should fail");

        assert!(matches!(
            err,
            AdpPublishError::IncompleteFulfillmentTier { .. }
        ));
        assert!(err.to_string().contains("version"));
        assert!(err.to_string().contains("fulfillment_pubkey"));
        assert!(!err.to_string().contains("fulfillment_valid_from"));
    }

    #[test]
    fn malformed_server_url_is_rejected() {
        let input = AdpListingInput {
            d_tag: "bad-server".to_string(),
            title: "Bad Server".to_string(),
            description: "Invalid URL".to_string(),
            price_sats: 0,
            lud16: None,
            tags: vec![],
            images: vec![],
            servers: vec!["dist.example.com".to_string()],
            file_hash: Some(VALID_SHA256.to_string()),
            version: Some("1.0.0".to_string()),
            fulfillment_pubkey: Some("fulfillment-pubkey".to_string()),
            fulfillment_valid_from: Some(1_725_000_000),
            fulfillment_revoked_at: None,
            acquisition: AcquisitionPolicy::Gated,
            platforms: vec![],
            campaigns: Vec::new(),
            nip94_event_id: None,
            preserved_tags: Vec::new(),
        };

        let err =
            build_adp_listing_event_builder(&input).expect_err("malformed server URL should fail");

        assert!(matches!(err, AdpPublishError::MalformedServerUrl { .. }));
        assert!(err.to_string().contains("dist.example.com"));
    }

    #[tokio::test]
    async fn direct_signing_fulfillment_emits_developer_pubkey() {
        let keys = Keys::generate();
        let developer_pubkey = keys.public_key().to_hex();
        let input = AdpListingInput {
            d_tag: "direct".to_string(),
            title: "Direct".to_string(),
            description: "Direct fulfillment".to_string(),
            price_sats: 0,
            lud16: None,
            tags: vec![],
            images: vec![],
            servers: vec!["https://dist.example.com".to_string()],
            file_hash: Some(VALID_SHA256.to_string()),
            version: Some("1.0.0".to_string()),
            fulfillment_pubkey: Some(developer_pubkey.clone()),
            fulfillment_valid_from: Some(1_725_000_000),
            fulfillment_revoked_at: None,
            acquisition: AcquisitionPolicy::Gated,
            platforms: vec![],
            campaigns: Vec::new(),
            nip94_event_id: None,
            preserved_tags: Vec::new(),
        };

        let event = build_adp_listing_event_builder(&input)
            .expect("direct signing listing should be valid")
            .sign_with_keys(&keys)
            .expect("event should sign");

        assert_eq!(
            tag_values(&event, "fulfillment_pubkey"),
            vec![vec![
                "fulfillment_pubkey".to_string(),
                developer_pubkey,
                "1725000000".to_string(),
                String::new(),
            ]]
        );
    }

    #[tokio::test]
    async fn fulfillment_tag_preserves_empty_validity_and_revocation_positions() {
        let keys = Keys::generate();
        let input = AdpListingInput {
            d_tag: "legacy-delegated".to_string(),
            title: "Legacy Delegated".to_string(),
            description: "Existing fulfillment metadata without timestamps".to_string(),
            price_sats: 0,
            lud16: None,
            tags: vec![],
            images: vec![],
            servers: vec!["https://dist.example.com".to_string()],
            file_hash: Some(VALID_SHA256.to_string()),
            version: Some("1.0.0".to_string()),
            fulfillment_pubkey: Some("fulfillment-pubkey".to_string()),
            fulfillment_valid_from: None,
            fulfillment_revoked_at: None,
            acquisition: AcquisitionPolicy::Gated,
            platforms: vec![],
            campaigns: Vec::new(),
            nip94_event_id: None,
            preserved_tags: Vec::new(),
        };

        let event = build_adp_listing_event_builder(&input)
            .expect("legacy fulfillment metadata should remain publishable")
            .sign_with_keys(&keys)
            .expect("event should sign");

        assert_eq!(
            tag_values(&event, "fulfillment_pubkey"),
            vec![vec![
                "fulfillment_pubkey".to_string(),
                "fulfillment-pubkey".to_string(),
                String::new(),
                String::new(),
            ]]
        );
    }
}

#[cfg(test)]
mod authoritative_launch_decoder_tests {
    use nostr::{EventBuilder, Keys, Tag};

    use super::*;
    use crate::distribution::{PathError, ProfileId, ProfileRejection, RelativePath, WorkingDir};

    const SHA256: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    #[test]
    fn download_only_and_platform_only_events_are_exactly_absent() {
        let download_only = signed_event(vec![vec!["d", "download-only"]]);
        let platform_only = signed_event(vec![vec!["d", "legacy"], vec!["platform", "win-x64"]]);

        assert_eq!(
            decode_authoritative_launch_tier(&download_only),
            AuthoritativeLaunchTier::Absent
        );
        assert_eq!(
            decode_authoritative_launch_tier(&platform_only),
            AuthoritativeLaunchTier::Absent
        );
    }

    #[test]
    fn adapter_signed_three_profile_event_decodes_without_loss_or_defects() {
        let profiles = three_profiles();
        let input = listing_input();
        let launch_tier = AdpLaunchTierInput {
            package_format: PackageFormat::Zip,
            platforms: vec![PlatformTarget {
                os: "linux".to_owned(),
                arch: "x86_64".to_owned(),
            }],
            launch_profiles: profiles.clone(),
            publisher_defaults: vec![PublisherLaunchDefault {
                profile_id: ProfileId::parse("play").expect("valid default profile ID"),
                targets: vec![PlatformTarget {
                    os: "linux".to_owned(),
                    arch: "x86_64".to_owned(),
                }],
            }],
        };
        let event = build_adp_listing_event_builder(AdpListingPublishInput {
            listing: &input,
            launch_tier: Some(&launch_tier),
        })
        .expect("adapter event builds")
        .sign_with_keys(&Keys::generate())
        .expect("adapter event signs");

        let AuthoritativeLaunchTier::Present(decoded) = decode_authoritative_launch_tier(&event)
        else {
            panic!("package and launch tags claim the tier");
        };
        assert!(decoded.defects.is_empty());
        assert_eq!(
            decoded
                .publisher_defaults
                .first()
                .map(|mapping| mapping.profile_id.as_str()),
            Some("play")
        );
        let expected_raw = event
            .tags
            .iter()
            .filter(|tag| {
                tag.as_slice().first().is_some_and(|name| {
                    matches!(
                        name.as_str(),
                        "package" | "platform" | "launch" | "launch_default"
                    )
                })
            })
            .map(|tag| tag.as_slice().to_vec())
            .collect::<Vec<_>>();
        let actual_raw = decoded
            .tags
            .iter()
            .map(|tag| match tag {
                AuthoritativeLaunchTag::Package(tag) => tag.raw.clone(),
                AuthoritativeLaunchTag::Platform(tag) => tag.raw.clone(),
                AuthoritativeLaunchTag::Launch(tag) => tag.raw.clone(),
                AuthoritativeLaunchTag::LaunchDefault(tag) => tag.raw.clone(),
            })
            .collect::<Vec<_>>();
        assert_eq!(actual_raw, expected_raw);
        let parsed_profiles = decoded
            .tags
            .iter()
            .filter_map(|tag| match tag {
                AuthoritativeLaunchTag::Launch(tag) => Some(
                    tag.parsed
                        .profile
                        .as_ref()
                        .expect("adapter launch tag passes the shared parser"),
                ),
                AuthoritativeLaunchTag::LaunchDefault(_) => None,
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(parsed_profiles, profiles.iter().collect::<Vec<_>>());
    }

    #[test]
    fn hostile_event_retains_every_tag_and_reports_every_exact_defect() {
        let raw_tags = vec![
            vec!["d", "hostile"],
            vec!["package", "zip"],
            vec!["platform", "WIN-x64"],
            valid_game_launch(),
            vec!["package", "rar"],
            vec!["platform", "haiku-mips"],
            vec![
                "launch",
                "play",
                "../escape",
                ".",
                "[]",
                "game",
                "Broken duplicate",
            ],
            vec!["launch", "truncated"],
            vec![
                "launch",
                "future",
                "tools/run",
                ".",
                "[]",
                "future-kind",
                "Future tool",
            ],
        ];
        let expected_raw = raw_tags
            .iter()
            .enumerate()
            .filter(|(_, tag)| {
                matches!(
                    tag.first().copied(),
                    Some("package" | "platform" | "launch")
                )
            })
            .map(|(index, tag)| (index, tag.iter().map(|value| (*value).to_owned()).collect()))
            .collect::<Vec<(usize, Vec<String>)>>();
        let event = signed_event(raw_tags);

        let AuthoritativeLaunchTier::Present(decoded) = decode_authoritative_launch_tier(&event)
        else {
            panic!("hostile metadata still claims a present tier");
        };
        let actual_raw = decoded
            .tags
            .iter()
            .map(|tag| match tag {
                AuthoritativeLaunchTag::Package(tag) => (tag.event_tag_index, tag.raw.clone()),
                AuthoritativeLaunchTag::Platform(tag) => (tag.event_tag_index, tag.raw.clone()),
                AuthoritativeLaunchTag::Launch(tag) => (tag.event_tag_index, tag.raw.clone()),
                AuthoritativeLaunchTag::LaunchDefault(tag) => {
                    (tag.event_tag_index, tag.raw.clone())
                }
            })
            .collect::<Vec<_>>();
        assert_eq!(actual_raw, expected_raw);
        assert_eq!(
            decoded.defects,
            vec![
                AuthoritativeLaunchDefect::PackageCount {
                    actual: 2,
                    event_tag_indices: vec![1, 4],
                },
                AuthoritativeLaunchDefect::NonCanonicalPlatform {
                    event_tag_index: 2,
                    raw_platform: "WIN-x64".to_owned(),
                    canonical: "windows-x86_64".to_owned(),
                },
                AuthoritativeLaunchDefect::UnsupportedPackage {
                    event_tag_index: 4,
                    raw_format: "rar".to_owned(),
                },
                AuthoritativeLaunchDefect::InvalidPlatform {
                    event_tag_index: 5,
                    raw_platform: Some("haiku-mips".to_owned()),
                },
                AuthoritativeLaunchDefect::MalformedLaunch {
                    event_tag_index: 6,
                    rejection: ProfileRejection::Executable(PathError::Rule2ParentSegment),
                },
                AuthoritativeLaunchDefect::MalformedLaunch {
                    event_tag_index: 7,
                    rejection: ProfileRejection::MissingRequiredElement,
                },
                AuthoritativeLaunchDefect::UnknownLaunchType {
                    event_tag_index: 8,
                    raw_launch_type: "future-kind".to_owned(),
                },
                AuthoritativeLaunchDefect::DuplicateProfileId {
                    profile_id: ProfileId::parse("play").expect("valid duplicate ID"),
                    event_tag_indices: vec![3, 6],
                },
            ]
        );
        let unknown = decoded
            .tags
            .iter()
            .find_map(|tag| match tag {
                AuthoritativeLaunchTag::Launch(tag) if tag.event_tag_index == 8 => Some(tag),
                AuthoritativeLaunchTag::LaunchDefault(_) => None,
                _ => None,
            })
            .expect("unknown launch type tag is retained");
        assert_eq!(
            unknown
                .parsed
                .profile
                .as_ref()
                .expect("A2 maps unknown type without rejecting")
                .launch_type,
            LaunchType::Tool
        );
    }

    #[test]
    fn partial_package_only_and_launch_only_tiers_are_present_with_defects() {
        let package_only = signed_event(vec![vec!["package", "zip"]]);
        let launch_only = signed_event(vec![valid_game_launch()]);

        let AuthoritativeLaunchTier::Present(package) =
            decode_authoritative_launch_tier(&package_only)
        else {
            panic!("package claims a present tier");
        };
        assert_eq!(
            package.defects,
            vec![
                AuthoritativeLaunchDefect::MissingPlatform,
                AuthoritativeLaunchDefect::NoValidGameProfile,
            ]
        );
        let AuthoritativeLaunchTier::Present(launch) =
            decode_authoritative_launch_tier(&launch_only)
        else {
            panic!("launch claims a present tier");
        };
        assert_eq!(
            launch.defects,
            vec![
                AuthoritativeLaunchDefect::PackageCount {
                    actual: 0,
                    event_tag_indices: Vec::new(),
                },
                AuthoritativeLaunchDefect::MissingPlatform,
            ]
        );
    }

    #[test]
    fn hostile_publisher_defaults_are_retained_diagnosed_and_ignored() {
        let event = signed_event(vec![
            vec!["package", "zip"],
            vec!["platform", "linux-x86_64"],
            valid_game_launch(),
            vec![
                "launch",
                "configure",
                "tools/config",
                ".",
                "[]",
                "config",
                "Configure",
            ],
            vec!["launch_default", "Bad"],
            vec!["launch_default", "missing"],
            vec!["launch_default", "configure"],
            vec!["launch_default", "play"],
        ]);

        let AuthoritativeLaunchTier::Present(decoded) = decode_authoritative_launch_tier(&event)
        else {
            panic!("publisher defaults claim a present launch tier");
        };
        assert_eq!(decoded.publisher_defaults.len(), 1);
        assert_eq!(decoded.publisher_defaults[0].profile_id.as_str(), "play");
        assert_eq!(
            decoded
                .tags
                .iter()
                .filter(|tag| matches!(tag, AuthoritativeLaunchTag::LaunchDefault(_)))
                .count(),
            4
        );
        assert_eq!(
            decoded.defects,
            vec![
                AuthoritativeLaunchDefect::MalformedLaunchDefault {
                    event_tag_index: 4,
                    raw_profile_id: Some("Bad".to_owned()),
                },
                AuthoritativeLaunchDefect::DanglingLaunchDefault {
                    event_tag_index: 5,
                    profile_id: ProfileId::parse("missing").expect("valid profile ID"),
                },
                AuthoritativeLaunchDefect::NonGameLaunchDefault {
                    event_tag_index: 6,
                    profile_id: ProfileId::parse("configure").expect("valid profile ID"),
                },
                AuthoritativeLaunchDefect::OverlappingLaunchDefaultTarget {
                    target: PlatformTarget {
                        os: "linux".to_owned(),
                        arch: "x86_64".to_owned(),
                    },
                    event_tag_indices: vec![5, 6, 7],
                },
            ]
        );
    }

    #[test]
    fn scoped_defaults_retain_raw_targets_and_resolve_normalized_mappings() {
        let event = signed_event(vec![
            vec!["package", "zip"],
            vec!["platform", "linux-x86_64"],
            vec!["platform", "linux-aarch64"],
            valid_game_launch(),
            vec![
                "launch",
                "arm-play",
                "bin/arm-game",
                ".",
                "[]",
                "game",
                "Play ARM",
            ],
            vec!["launch_default", "play", "linux-amd64", "unknown-target"],
            vec!["launch_default", "arm-play", "linux-aarch64"],
        ]);

        let AuthoritativeLaunchTier::Present(decoded) = decode_authoritative_launch_tier(&event)
        else {
            panic!("scoped defaults claim a present launch tier");
        };
        assert_eq!(decoded.publisher_defaults.len(), 2);
        assert_eq!(decoded.publisher_defaults[0].profile_id.as_str(), "play");
        assert_eq!(
            decoded.publisher_defaults[0].targets,
            vec![PlatformTarget {
                os: "linux".to_owned(),
                arch: "x86_64".to_owned(),
            }]
        );
        let defaults = decoded
            .tags
            .iter()
            .filter_map(|tag| match tag {
                AuthoritativeLaunchTag::LaunchDefault(tag) => Some(tag),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(
            defaults[0].raw_targets,
            ["linux-amd64".to_owned(), "unknown-target".to_owned()]
        );
        assert_eq!(
            decoded.defects,
            vec![
                AuthoritativeLaunchDefect::NonCanonicalLaunchDefaultTarget {
                    event_tag_index: 5,
                    raw_target: "linux-amd64".to_owned(),
                    canonical: "linux-x86_64".to_owned(),
                },
                AuthoritativeLaunchDefect::InvalidLaunchDefaultTarget {
                    event_tag_index: 5,
                    raw_target: "unknown-target".to_owned(),
                },
            ]
        );

        let wire = AuthoritativeAdpEditMetadata::from(AuthoritativeLaunchTier::Present(decoded));
        let json = serde_json::to_value(&wire).expect("scoped metadata serializes");
        assert_eq!(
            json["metadata"]["tags"][5]["raw_targets"],
            serde_json::json!(["linux-amd64", "unknown-target"])
        );
        assert_eq!(
            serde_json::from_value::<AuthoritativeAdpEditMetadata>(json)
                .expect("scoped metadata round-trips"),
            wire
        );
    }

    fn signed_event(tags: Vec<Vec<&str>>) -> nostr::Event {
        let tags = tags
            .into_iter()
            .map(|raw| Tag::parse(raw).expect("test tag parses"))
            .collect::<Vec<_>>();
        EventBuilder::new(Kind::Custom(30_402), "")
            .tags(tags)
            .sign_with_keys(&Keys::generate())
            .expect("test event signs")
    }

    fn valid_game_launch() -> Vec<&'static str> {
        vec!["launch", "play", "bin/game", ".", "[]", "game", "Play"]
    }

    fn three_profiles() -> Vec<LaunchProfile> {
        vec![
            LaunchProfile {
                id: ProfileId::parse("play").expect("valid profile ID"),
                display_name: "Play".to_owned(),
                executable: RelativePath::parse("bin/game").expect("valid executable"),
                working_dir: WorkingDir::parse(".").expect("valid working directory"),
                arguments: vec!["--fullscreen".to_owned()],
                launch_type: LaunchType::Game,
            },
            LaunchProfile {
                id: ProfileId::parse("config").expect("valid profile ID"),
                display_name: "Configure".to_owned(),
                executable: RelativePath::parse("tools/config").expect("valid executable"),
                working_dir: WorkingDir::parse("tools").expect("valid working directory"),
                arguments: Vec::new(),
                launch_type: LaunchType::Config,
            },
            LaunchProfile {
                id: ProfileId::parse("diagnostics").expect("valid profile ID"),
                display_name: "Diagnostics".to_owned(),
                executable: RelativePath::parse("tools/diagnostics").expect("valid executable"),
                working_dir: WorkingDir::parse("tools").expect("valid working directory"),
                arguments: vec!["--json".to_owned()],
                launch_type: LaunchType::Tool,
            },
        ]
    }

    fn listing_input() -> AdpListingInput {
        AdpListingInput {
            d_tag: "decoded-tier".to_owned(),
            title: "Decoded Tier".to_owned(),
            description: "Decoder fixture".to_owned(),
            price_sats: 0,
            lud16: None,
            tags: Vec::new(),
            images: Vec::new(),
            servers: vec!["https://dist.example.com".to_owned()],
            file_hash: Some(SHA256.to_owned()),
            version: Some("1.0.0".to_owned()),
            fulfillment_authorizations: Vec::new(),
            acquisition: AcquisitionPolicy::Gated,
            platforms: Vec::new(),
            campaigns: Vec::new(),
            nip94_event_id: None,
            preserved_tags: Vec::new(),
        }
    }
}
