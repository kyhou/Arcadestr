//! Offline launcher IPC. These types contain no acquisition or signer state.
use serde::{Deserialize, Serialize};

use crate::distribution::LaunchType;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LaunchOption {
    pub profile_id: String,
    pub display_name: String,
    pub launch_type: LaunchType,
    /// The install-time reason, not a fresh interpretation of listing metadata.
    pub unavailable_reason: Option<String>,
}

/// Authoritative backend view of launch choices and default resolution.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LaunchConfiguration {
    pub options: Vec<LaunchOption>,
    pub publisher_default_profile_id: Option<String>,
    pub saved_preference_profile_id: Option<String>,
    pub preference_updated_at: Option<i64>,
    pub publisher_default_status: ProfileReferenceStatus,
    pub saved_preference_status: ProfileReferenceStatus,
    pub effective_profile_id: Option<String>,
    pub resolution: LaunchResolutionStatus,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fallback_reasons: Vec<LaunchFallbackReason>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProfileReferenceStatus {
    NotConfigured,
    Available,
    Invalid,
    Missing,
    NonGame,
    Unavailable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", content = "source", rename_all = "snake_case")]
pub enum LaunchResolutionStatus {
    Selected(LaunchProfileSource),
    NoGameProfile,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LaunchProfileSource {
    SavedPreference,
    PublisherDefault,
    PublisherOrder,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LaunchFallbackReason {
    SavedPreferenceInvalid,
    SavedPreferenceMissing,
    SavedPreferenceNonGame,
    SavedPreferenceUnavailable,
    PublisherDefaultMissing,
    PublisherDefaultInvalid,
    PublisherDefaultNonGame,
    PublisherDefaultUnavailable,
}

impl LaunchOption {
    pub fn is_play_candidate(&self) -> bool {
        self.launch_type == LaunchType::Game && self.unavailable_reason.is_none()
    }
}

/// Session-only records. A PID is evidence of a start, not a durable process identity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LaunchSession {
    pub session_id: String,
    pub installation_id: String,
    pub profile_id: String,
    pub pid: u32,
    pub started_at: i64,
    pub ended_at: Option<i64>,
    pub outcome: Option<LaunchOutcome>,
}

impl LaunchSession {
    pub fn is_running(&self) -> bool {
        self.outcome.is_none()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LaunchOutcome {
    Exited {
        code: Option<i32>,
        signal: Option<i32>,
    },
    Detached,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[serde(tag = "code", content = "detail", rename_all = "snake_case")]
pub enum LaunchError {
    #[error("Installation not found")]
    NotFound,
    #[error("Installation is not ready to launch")]
    NotReady,
    #[error("Installation is for a different operating system or architecture")]
    UnsupportedPlatform,
    #[error("Installation directory is missing: {0}")]
    RootMissing(String),
    #[error("Installation root is not a directory: {0}")]
    RootNotDirectory(String),
    #[error("Executable is missing: {0}")]
    ExecutableMissing(String),
    #[error("Executable is not a regular file: {0}")]
    ExecutableNotFile(String),
    #[error("Working directory is missing: {0}")]
    WorkingDirectoryMissing(String),
    #[error("Working directory is not a directory: {0}")]
    WorkingDirectoryNotDirectory(String),
    #[error("Launch path resolves outside the installation root: {0}")]
    OutsideRoot(String),
    #[error("The configured launch target is not a supported executable for this platform: {0}")]
    UnsupportedExecutable(String),
    #[error("Executable requires privileges or is not executable: {0}")]
    ExecutablePermissions(String),
    #[error("This installation is already running in this client session")]
    AlreadyRunning,
    #[error("No available game launch profile is recorded")]
    NoGameProfile,
    #[error("Launch profile not found: {0}")]
    ProfileNotFound(String),
    #[error("Launch profile is not a game profile: {0}")]
    ProfileNotGame(String),
    #[error("Launch profile is unavailable: {0}")]
    ProfileUnavailable(String),
    #[error("Could not start executable: {0}")]
    Spawn(String),
    #[error("Launch operation failed: {0}")]
    Operation(String),
    #[error("The client is shutting down")]
    ShuttingDown,
}

impl From<String> for LaunchError {
    fn from(value: String) -> Self {
        Self::Operation(value)
    }
}
