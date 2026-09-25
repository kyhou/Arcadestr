// Structured distribution validation and resolution failures.

use thiserror::Error;

use super::domain::PlatformTarget;

#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum PathError {
    #[error("path is absolute or begins with a root separator")]
    Rule1Absolute,
    #[error("path contains a parent-directory segment")]
    Rule2ParentSegment,
    #[error("path contains a backslash")]
    Rule3Backslash,
    #[error("path contains a colon or device prefix")]
    Rule4ColonOrPrefix,
    #[error("path contains a NUL or C0 control character")]
    Rule5ControlCharacter,
    #[error("path contains an empty or current-directory segment")]
    Rule6EmptyOrDotSegment,
    #[error("path segment ends in a space or period")]
    Rule7TrailingSpaceOrPeriod,
    #[error("path segment uses a reserved device name")]
    Rule8ReservedDeviceName,
    #[error("path or path segment exceeds its length limit")]
    Rule9LengthExceeded,
}

#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum ProfileIdError {
    #[error("profile ID does not match [a-z0-9][a-z0-9_-]{{0,63}}")]
    InvalidGrammar,
}

#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum ArgumentError {
    #[error("arguments must be a JSON array of strings")]
    InvalidJsonShape,
    #[error("arguments exceed the 32-element limit")]
    TooMany,
    #[error("an argument exceeds the 1024-character limit")]
    ElementTooLong,
    #[error("an argument contains NUL")]
    ContainsNul,
}

#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum DisplayNameError {
    #[error("display name must contain between 1 and 64 characters")]
    InvalidLength,
    #[error("display name contains a control character")]
    ControlCharacter,
}

#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum DigestError {
    #[error("digest must contain exactly 64 lowercase hexadecimal characters")]
    InvalidSha256,
}

#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum ProfileRejection {
    #[error("launch tag is missing a required positional element")]
    MissingRequiredElement,
    #[error("invalid profile ID: {0}")]
    ProfileId(ProfileIdError),
    #[error("invalid executable path: {0}")]
    Executable(PathError),
    #[error("invalid working directory: {0}")]
    WorkingDir(PathError),
    #[error("invalid arguments: {0}")]
    Arguments(ArgumentError),
    #[error("invalid display name: {0}")]
    DisplayName(DisplayNameError),
}

#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum NoLaunchTierReason {
    #[error("file_hash tag is missing")]
    MissingFileHash,
    #[error("file_hash tag is invalid")]
    InvalidFileHash,
    #[error("listing is download-only")]
    DownloadOnly,
    #[error("package tag is missing")]
    MissingPackage,
    #[error("package claims a launch tier but no launch profiles are present")]
    MissingLaunchProfiles,
    #[error("more than one package tag is present")]
    MultiplePackages,
    #[error("no platform tag is present")]
    MissingPlatform,
    #[error("duplicate syntactically valid profile ID: {profile_id}")]
    DuplicateProfileId { profile_id: String },
    #[error("no valid game launch profile remains")]
    NoGameProfile,
}

#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum ResolveError {
    #[error("listing has no launch tier: {reason}")]
    NoLaunchTier { reason: NoLaunchTierReason },
    #[error("unsupported package format: {declared}")]
    UnsupportedPackageFormat { declared: String },
    #[error("declared platforms are unsupported on this host")]
    UnsupportedPlatform {
        declared: Vec<String>,
        host: PlatformTarget,
    },
    #[error("listing has no artifact source")]
    NoSource,
    #[error("listing coordinate is malformed")]
    MalformedCoordinate,
}
