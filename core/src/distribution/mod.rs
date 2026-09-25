// Protocol-neutral distribution domain types and boundary validation.

pub mod domain;
pub mod errors;
pub mod listing;
pub mod platform;
pub mod validation;

pub use domain::{
    ArtifactSource, DiscardedProfile, GameCoordinate, LaunchProfile, LaunchType, PackageFormat,
    PlatformEligibility, PlatformTarget, ProfileId, RelativePath, ResolveOutcome, ResolvedArtifact,
    ResolvedInstallPlan, Sha256Digest, WorkingDir,
};
pub use errors::{
    ArgumentError, DigestError, DisplayNameError, NoLaunchTierReason, PathError, ProfileIdError,
    ProfileRejection, ResolveError,
};
pub use listing::{
    parse_launch_profile, parse_launch_tag, parse_launch_type, resolve_install_plan,
    ParsedLaunchTag,
};
pub use platform::{host_platform, normalize_arch, normalize_os, parse_platform_tag};
