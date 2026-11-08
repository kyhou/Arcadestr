//! Pure publisher state and transitions for the typed ADP publishing flow.
//!
//! This module deliberately has no Leptos, rendering, transport, or clock
//! dependencies.  The UI owns scheduling and feeds correlated responses into
//! these transitions.

use arcadestr_core::adp_publish_contract::{
    AdpLaunchTierIntent, AdpPublishValidationViolation, ArgumentRejection,
    AuthoritativeAdpEditMetadata, AuthoritativeAdpEditMetadataPresent, AuthoritativeAdpEditTag,
    AuthoritativeProfileRejection, DisplayNameRejection, ProfileIdRejection,
    PublishAdpListingError, PublishAdpListingRequestExtension, PublisherLaunchDefault,
};
use arcadestr_core::archive_inspection_contract::{
    ArchiveAccountFailure, ArchiveCandidate, ArchiveFileSelection, ArchiveHandleFailure,
    ArchiveHazardDetail, ArchivePage, ArchiveRequestId, ArchiveSelectionError, ArchiveSelectionId,
    ArchiveSelectionSummary, ArchiveWarningDetail, BuildSelectionRef, SelectableArchiveFile,
    SelectableWorkingDirectory,
};
use arcadestr_core::distribution::{
    parse_platform_tag, LaunchProfile, LaunchType, PackageFormat, PlatformTarget, ProfileId,
    RelativePath, Sha256Digest, WorkingDir,
};

use crate::tauri_bridge::{PublishAdpListingRequest, PublishAdpListingResult};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ArchiveOperationIdentity {
    pub request_id: ArchiveRequestId,
    pub owner_npub: String,
    pub operation_generation: u64,
    pub selection_generation: u64,
    pub selection_id: Option<ArchiveSelectionId>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Transition<T> {
    Applied(T),
    Stale,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ArchiveSelectionState {
    pub owner_npub: String,
    pub selection: Option<ArchiveFileSelection>,
    pub summary: Option<ArchiveSelectionSummary>,
    pub selected_build: Option<BuildSelectionRef>,
    pub error: Option<ArchiveSelectionError>,
    pub queries: ArchiveQueryState,
    pub busy: bool,
    operation_generation: u64,
    selection_generation: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct ArchiveQueryState {
    pub files: Option<ArchivePage<SelectableArchiveFile>>,
    pub working_directories: Option<ArchivePage<SelectableWorkingDirectory>>,
    pub candidates: Option<ArchivePage<ArchiveCandidate>>,
    pub hazards: Option<ArchivePage<ArchiveHazardDetail>>,
    pub warnings: Option<ArchivePage<ArchiveWarningDetail>>,
    pub candidate_platform: Option<PlatformTarget>,
    pub loading: bool,
    pub error: Option<ArchiveSelectionError>,
    pub generation: u64,
}

impl ArchiveQueryState {
    fn invalidate(&mut self) {
        self.files = None;
        self.working_directories = None;
        self.candidates = None;
        self.hazards = None;
        self.warnings = None;
        self.candidate_platform = None;
        self.loading = false;
        self.error = None;
        self.generation = self.generation.wrapping_add(1);
    }

    fn invalidate_candidates(&mut self) {
        self.candidates = None;
        self.candidate_platform = None;
        self.loading = false;
        self.error = None;
        self.generation = self.generation.wrapping_add(1);
    }
}

impl ArchiveSelectionState {
    pub(crate) fn new(owner_npub: String) -> Self {
        Self {
            owner_npub,
            selection: None,
            summary: None,
            selected_build: None,
            error: None,
            queries: ArchiveQueryState::default(),
            busy: false,
            operation_generation: 0,
            selection_generation: 0,
        }
    }

    pub(crate) fn begin_picker(
        &mut self,
        request_id: ArchiveRequestId,
    ) -> ArchiveOperationIdentity {
        self.begin_operation(request_id, None)
    }

    pub(crate) fn begin_selection_operation(
        &mut self,
        request_id: ArchiveRequestId,
    ) -> Option<ArchiveOperationIdentity> {
        let selection_id = self.selection.as_ref()?.selection_id.clone();
        Some(self.begin_operation(request_id, Some(selection_id)))
    }

    fn begin_operation(
        &mut self,
        request_id: ArchiveRequestId,
        selection_id: Option<ArchiveSelectionId>,
    ) -> ArchiveOperationIdentity {
        self.operation_generation = self.operation_generation.wrapping_add(1);
        self.busy = true;
        self.error = None;
        ArchiveOperationIdentity {
            request_id,
            owner_npub: self.owner_npub.clone(),
            operation_generation: self.operation_generation,
            selection_generation: self.selection_generation,
            selection_id,
        }
    }

    fn accepts(&self, operation: &ArchiveOperationIdentity) -> bool {
        operation.owner_npub == self.owner_npub
            && operation.operation_generation == self.operation_generation
            && operation.selection_generation == self.selection_generation
            && operation.selection_id.as_ref().is_none_or(|operation_id| {
                self.selection
                    .as_ref()
                    .is_some_and(|selection| &selection.selection_id == operation_id)
            })
    }

    /// Applies picker completion. `Ok(None)` is cancellation. Both cancellation
    /// and picker failure leave the prior selection untouched. A successful
    /// replacement returns the old selection so its opaque handle can be
    /// discarded by the caller.
    pub(crate) fn apply_picker(
        &mut self,
        operation: &ArchiveOperationIdentity,
        response: Result<Option<ArchiveFileSelection>, ArchiveSelectionError>,
    ) -> Transition<Option<ArchiveFileSelection>> {
        if !self.accepts(operation) {
            return Transition::Stale;
        }
        self.busy = false;
        match response {
            Ok(None) => Transition::Applied(None),
            Err(error) => {
                self.apply_operation_error(error);
                Transition::Applied(None)
            }
            Ok(Some(selection)) => {
                let replacement = self.selection.replace(selection);
                self.summary = None;
                self.selected_build = None;
                self.error = None;
                self.queries.invalidate();
                self.selection_generation = self.selection_generation.wrapping_add(1);
                Transition::Applied(replacement)
            }
        }
    }

    pub(crate) fn apply_summary(
        &mut self,
        operation: &ArchiveOperationIdentity,
        response: Result<ArchiveSelectionSummary, ArchiveSelectionError>,
    ) -> Transition<()> {
        if !self.accepts(operation) {
            return Transition::Stale;
        }
        if response.as_ref().is_ok_and(|summary| {
            Some(&summary.selection.selection_id) != operation.selection_id.as_ref()
        }) {
            return Transition::Stale;
        }
        self.busy = false;
        match response {
            Ok(summary) => {
                self.summary = Some(summary);
                self.error = None;
            }
            Err(error) => self.apply_operation_error(error),
        }
        Transition::Applied(())
    }

    pub(crate) fn apply_hash(
        &mut self,
        operation: &ArchiveOperationIdentity,
        response: Result<BuildSelectionRef, ArchiveSelectionError>,
    ) -> Transition<()> {
        if !self.accepts(operation) {
            return Transition::Stale;
        }
        if response
            .as_ref()
            .is_ok_and(|build| Some(&build.selection_id) != operation.selection_id.as_ref())
        {
            return Transition::Stale;
        }
        self.busy = false;
        match response {
            Ok(build) => {
                self.selected_build = Some(build);
                self.error = None;
            }
            Err(error) => self.apply_operation_error(error),
        }
        Transition::Applied(())
    }

    pub(crate) fn invalidate_account(
        &mut self,
        next_owner_npub: String,
        failure: ArchiveAccountFailure,
    ) -> Option<ArchiveFileSelection> {
        self.owner_npub = next_owner_npub;
        self.busy = false;
        let selection = self.selection.take();
        self.summary = None;
        self.queries.invalidate();
        self.selected_build = None;
        self.error = Some(ArchiveSelectionError::Account { failure });
        self.operation_generation = self.operation_generation.wrapping_add(1);
        self.selection_generation = self.selection_generation.wrapping_add(1);
        selection
    }

    fn apply_operation_error(&mut self, error: ArchiveSelectionError) {
        if archive_error_is_terminal(&error) {
            self.selected_build = None;
            self.selection_generation = self.selection_generation.wrapping_add(1);
        }
        self.error = Some(error);
    }

    /// `expires_at_unix` is intentionally not consulted here. It is display
    /// data and a scheduling hint; only an authoritative backend response may
    /// change selection lifecycle state.
    pub(crate) fn build_ready(&self) -> bool {
        self.summary
            .as_ref()
            .filter(|summary| !summary.blocking_hazards)
            .zip(self.selected_build.as_ref())
            .is_some_and(|(summary, build)| {
                summary.selection.selection_id == build.selection_id
                    && self
                        .selection
                        .as_ref()
                        .is_some_and(|selection| selection.selection_id == build.selection_id)
            })
    }

    pub(crate) fn consume_success(&mut self) -> Option<ArchiveFileSelection> {
        let selection = self.selection.take();
        self.summary = None;
        self.queries.invalidate();
        self.selected_build = None;
        self.error = None;
        self.busy = false;
        self.operation_generation = self.operation_generation.wrapping_add(1);
        self.selection_generation = self.selection_generation.wrapping_add(1);
        selection
    }
}

pub(crate) fn archive_error_is_terminal(error: &ArchiveSelectionError) -> bool {
    matches!(
        error,
        ArchiveSelectionError::Account { .. }
            | ArchiveSelectionError::Handle {
                failure: ArchiveHandleFailure::Unknown | ArchiveHandleFailure::Discarded,
                ..
            }
            | ArchiveSelectionError::Expired { .. }
            | ArchiveSelectionError::Mutation { .. }
            | ArchiveSelectionError::DigestMismatch { .. }
    )
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub(crate) struct DraftRowId(u64);

impl DraftRowId {
    pub(crate) fn from_raw(value: u64) -> Self {
        Self(value)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) struct PackageFormatDraft {
    pub text: String,
}

impl PackageFormatDraft {
    fn parsed(&self) -> Option<PackageFormat> {
        match self.text.trim().to_ascii_lowercase().as_str() {
            "zip" => Some(PackageFormat::Zip),
            "tar.zst" | "tar.zstd" | "tar_zstd" => Some(PackageFormat::TarZstd),
            "tar.gz" | "tar_gz" => Some(PackageFormat::TarGz),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ArchiveBoundChoice<T> {
    pub selection_id: ArchiveSelectionId,
    pub value: T,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(bound = "")]
pub(crate) struct ArchivePathDraft<T> {
    pub text: String,
    #[serde(skip)]
    pub confirmed: Option<ArchiveBoundChoice<T>>,
    #[serde(skip)]
    pub authoritative_value: Option<T>,
    pub stale: bool,
}

impl<T: Clone> ArchivePathDraft<T> {
    pub(crate) fn confirm(
        &mut self,
        selection_id: ArchiveSelectionId,
        value: T,
        canonical_text: String,
    ) {
        self.text = canonical_text;
        self.confirmed = Some(ArchiveBoundChoice {
            selection_id,
            value,
        });
        self.authoritative_value = None;
        self.stale = false;
    }

    fn current_value(&self, selection_id: Option<&ArchiveSelectionId>) -> Option<T> {
        if self.stale {
            return None;
        }
        match selection_id {
            Some(selection_id) => self
                .confirmed
                .as_ref()
                .filter(|confirmed| &confirmed.selection_id == selection_id)
                .map(|confirmed| confirmed.value.clone()),
            None => self.authoritative_value.clone(),
        }
    }

    fn mark_stale(&mut self) {
        if self.confirmed.is_some() || self.authoritative_value.is_some() {
            self.stale = true;
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) struct LaunchProfileDraft {
    /// Stable UI identity: live edits to `profile_id` must never change which
    /// row is reordered or removed.
    pub row_id: DraftRowId,
    pub profile_id: String,
    pub display_name: String,
    pub executable: ArchivePathDraft<RelativePath>,
    pub working_dir: ArchivePathDraft<WorkingDir>,
    pub arguments: Vec<String>,
    pub launch_type: LaunchType,
}

impl LaunchProfileDraft {
    pub(crate) fn empty(row_id: DraftRowId) -> Self {
        Self {
            row_id,
            profile_id: String::new(),
            display_name: String::new(),
            executable: ArchivePathDraft {
                text: String::new(),
                confirmed: None,
                authoritative_value: None,
                stale: false,
            },
            working_dir: ArchivePathDraft {
                text: ".".to_string(),
                confirmed: None,
                authoritative_value: None,
                stale: false,
            },
            arguments: Vec::new(),
            launch_type: LaunchType::Game,
        }
    }

    fn to_profile(&self, selection_id: Option<&ArchiveSelectionId>) -> Option<LaunchProfile> {
        self.to_profile_result(selection_id).ok()
    }

    fn to_profile_result(
        &self,
        selection_id: Option<&ArchiveSelectionId>,
    ) -> Result<LaunchProfile, AdpPublishValidationViolation> {
        let id = ProfileId::parse(&self.profile_id).map_err(|_| {
            AdpPublishValidationViolation::MalformedLaunchProfile {
                profile_id: self.profile_id.clone(),
                rejection: AuthoritativeProfileRejection::ProfileId(
                    ProfileIdRejection::InvalidGrammar,
                ),
            }
        })?;
        arcadestr_core::distribution::validation::validate_display_name(&self.display_name)
            .map_err(
                |error| AdpPublishValidationViolation::InvalidLaunchDisplayName {
                    profile_id: self.profile_id.clone(),
                    reason: match error {
                        arcadestr_core::distribution::DisplayNameError::InvalidLength => {
                            DisplayNameRejection::InvalidLength
                        }
                        arcadestr_core::distribution::DisplayNameError::ControlCharacter => {
                            DisplayNameRejection::ControlCharacter
                        }
                    },
                },
            )?;
        arcadestr_core::distribution::validation::validate_arguments(&self.arguments).map_err(
            |error| AdpPublishValidationViolation::InvalidLaunchArguments {
                profile_id: self.profile_id.clone(),
                reason: match error {
                    arcadestr_core::distribution::ArgumentError::InvalidJsonShape => {
                        ArgumentRejection::InvalidJsonShape
                    }
                    arcadestr_core::distribution::ArgumentError::TooMany => {
                        ArgumentRejection::TooMany
                    }
                    arcadestr_core::distribution::ArgumentError::ElementTooLong => {
                        ArgumentRejection::ElementTooLong
                    }
                    arcadestr_core::distribution::ArgumentError::ContainsNul => {
                        ArgumentRejection::ContainsNul
                    }
                },
            },
        )?;
        Ok(LaunchProfile {
            id,
            display_name: self.display_name.clone(),
            executable: self.executable.current_value(selection_id).ok_or_else(|| {
                AdpPublishValidationViolation::MalformedLaunchProfile {
                    profile_id: self.profile_id.clone(),
                    rejection: AuthoritativeProfileRejection::MissingRequiredElement,
                }
            })?,
            working_dir: self
                .working_dir
                .current_value(selection_id)
                .ok_or_else(|| AdpPublishValidationViolation::MalformedLaunchProfile {
                    profile_id: self.profile_id.clone(),
                    rejection: AuthoritativeProfileRejection::MissingRequiredElement,
                })?,
            arguments: self.arguments.clone(),
            launch_type: self.launch_type.clone(),
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ProfileDraftError {
    InvalidId,
    DuplicateId,
    InvalidDisplayName,
    InvalidArguments,
    ExecutableNotConfirmed,
    WorkingDirectoryNotConfirmed,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ProfileValidation {
    pub row_id: DraftRowId,
    pub errors: Vec<ProfileDraftError>,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) struct LaunchTierDraft {
    pub package_format: PackageFormatDraft,
    pub platforms: Vec<String>,
    pub profiles: Vec<LaunchProfileDraft>,
    /// Canonical listing target to stable profile-row assignments. Profile IDs
    /// remain editable, so they cannot safely identify assignments in-form.
    pub publisher_default_assignments: Vec<PublisherDefaultAssignmentDraft>,
    /// Rows whose target controls are open. This permits an explicit multi-target
    /// choice without inventing an assignment before the publisher chooses.
    pub automatic_profile_rows: Vec<DraftRowId>,
    next_row_id: u64,
    candidate_generation: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) struct PublisherDefaultAssignmentDraft {
    pub target: String,
    pub row_id: DraftRowId,
}

impl Default for LaunchTierDraft {
    fn default() -> Self {
        Self {
            package_format: PackageFormatDraft {
                text: String::new(),
            },
            platforms: Vec::new(),
            profiles: Vec::new(),
            publisher_default_assignments: Vec::new(),
            automatic_profile_rows: Vec::new(),
            next_row_id: 1,
            candidate_generation: 0,
        }
    }
}

impl LaunchTierDraft {
    pub(crate) fn add_profile(&mut self) -> DraftRowId {
        let id = DraftRowId(self.next_row_id);
        self.next_row_id = self.next_row_id.wrapping_add(1);
        self.profiles.push(LaunchProfileDraft::empty(id));
        id
    }

    pub(crate) fn remove_profile(&mut self, row_id: DraftRowId) -> Option<LaunchProfileDraft> {
        let index = self.profiles.iter().position(|row| row.row_id == row_id)?;
        let removed = self.profiles.remove(index);
        // Do not silently move signed Play actions to another game.
        self.publisher_default_assignments
            .retain(|assignment| assignment.row_id != row_id);
        self.automatic_profile_rows
            .retain(|selected| *selected != row_id);
        Some(removed)
    }

    pub(crate) fn profile_has_publisher_defaults(&self, row_id: DraftRowId) -> bool {
        self.automatic_profile_rows.contains(&row_id)
    }

    /// Enables or disables a profile's automatic Play mapping. A single target
    /// is unambiguous and is assigned immediately; multi-target listings wait
    /// for an explicit target choice from the publisher.
    pub(crate) fn set_publisher_default_profile(
        &mut self,
        row_id: DraftRowId,
        enabled: bool,
    ) -> bool {
        let selectable = self
            .profiles
            .iter()
            .any(|profile| profile.row_id == row_id && profile.launch_type == LaunchType::Game);
        if !selectable {
            return false;
        }
        if !enabled {
            self.publisher_default_assignments
                .retain(|assignment| assignment.row_id != row_id);
            self.automatic_profile_rows
                .retain(|selected| *selected != row_id);
        } else if self.platforms.len() == 1 {
            if !self.automatic_profile_rows.contains(&row_id) {
                self.automatic_profile_rows.push(row_id);
            }
            let target = self.platforms[0].clone();
            return self.assign_publisher_default(row_id, &target, true);
        } else if !self.automatic_profile_rows.contains(&row_id) {
            self.automatic_profile_rows.push(row_id);
        }
        true
    }

    /// Assigns one canonical listing target. Occupied targets move atomically
    /// from their previous profile; no replacement is selected on removal.
    pub(crate) fn assign_publisher_default(
        &mut self,
        row_id: DraftRowId,
        target: &str,
        assigned: bool,
    ) -> bool {
        let selectable = self
            .profiles
            .iter()
            .any(|profile| profile.row_id == row_id && profile.launch_type == LaunchType::Game);
        if !selectable || !self.platforms.iter().any(|platform| platform == target) {
            return false;
        }
        let displaced_rows = self
            .publisher_default_assignments
            .iter()
            .filter(|assignment| assignment.target == target && assignment.row_id != row_id)
            .map(|assignment| assignment.row_id)
            .collect::<Vec<_>>();
        self.publisher_default_assignments.retain(|assignment| {
            if assigned {
                assignment.target != target
            } else {
                assignment.target != target || assignment.row_id != row_id
            }
        });
        if assigned {
            if !self.automatic_profile_rows.contains(&row_id) {
                self.automatic_profile_rows.push(row_id);
            }
            self.publisher_default_assignments
                .push(PublisherDefaultAssignmentDraft {
                    target: target.to_string(),
                    row_id,
                });
        } else if !self
            .publisher_default_assignments
            .iter()
            .any(|assignment| assignment.row_id == row_id)
        {
            self.automatic_profile_rows
                .retain(|selected| *selected != row_id);
        }
        for displaced in displaced_rows {
            if !self
                .publisher_default_assignments
                .iter()
                .any(|assignment| assignment.row_id == displaced)
            {
                self.automatic_profile_rows
                    .retain(|selected| *selected != displaced);
            }
        }
        true
    }

    pub(crate) fn set_profile_launch_type(
        &mut self,
        row_id: DraftRowId,
        launch_type: LaunchType,
    ) -> bool {
        let Some(profile) = self
            .profiles
            .iter_mut()
            .find(|profile| profile.row_id == row_id)
        else {
            return false;
        };
        profile.launch_type = launch_type;
        if profile.launch_type != LaunchType::Game {
            self.publisher_default_assignments
                .retain(|assignment| assignment.row_id != row_id);
            self.automatic_profile_rows
                .retain(|selected| *selected != row_id);
        }
        true
    }

    pub(crate) fn reorder_profile(&mut self, row_id: DraftRowId, target: usize) -> bool {
        let Some(source) = self.profiles.iter().position(|row| row.row_id == row_id) else {
            return false;
        };
        if target >= self.profiles.len() {
            return false;
        }
        let row = self.profiles.remove(source);
        self.profiles.insert(target, row);
        true
    }

    pub(crate) fn replace_platforms(&mut self, platforms: Vec<String>) {
        if self.platforms != platforms {
            self.platforms = platforms;
            self.publisher_default_assignments.retain(|assignment| {
                self.platforms
                    .iter()
                    .any(|platform| platform == &assignment.target)
            });
            self.automatic_profile_rows.retain(|row_id| {
                self.publisher_default_assignments
                    .iter()
                    .any(|assignment| assignment.row_id == *row_id)
            });
            self.candidate_generation = self.candidate_generation.wrapping_add(1);
        }
    }

    pub(crate) fn candidate_generation(&self) -> u64 {
        self.candidate_generation
    }

    pub(crate) fn mark_bindings_stale(&mut self) {
        for profile in &mut self.profiles {
            profile.executable.mark_stale();
            profile.working_dir.mark_stale();
        }
    }

    pub(crate) fn live_profile_validation(&self) -> Vec<ProfileValidation> {
        self.profiles
            .iter()
            .map(|profile| {
                let mut errors = Vec::new();
                let parsed_id = ProfileId::parse(&profile.profile_id);
                if parsed_id.is_err() {
                    errors.push(ProfileDraftError::InvalidId);
                } else if self
                    .profiles
                    .iter()
                    .filter(|other| other.profile_id == profile.profile_id)
                    .count()
                    > 1
                {
                    errors.push(ProfileDraftError::DuplicateId);
                }
                if arcadestr_core::distribution::validation::validate_display_name(
                    &profile.display_name,
                )
                .is_err()
                {
                    errors.push(ProfileDraftError::InvalidDisplayName);
                }
                if arcadestr_core::distribution::validation::validate_arguments(&profile.arguments)
                    .is_err()
                {
                    errors.push(ProfileDraftError::InvalidArguments);
                }
                if (profile.executable.confirmed.is_none()
                    && profile.executable.authoritative_value.is_none())
                    || profile.executable.stale
                {
                    errors.push(ProfileDraftError::ExecutableNotConfirmed);
                }
                if (profile.working_dir.confirmed.is_none()
                    && profile.working_dir.authoritative_value.is_none())
                    || profile.working_dir.stale
                {
                    errors.push(ProfileDraftError::WorkingDirectoryNotConfirmed);
                }
                ProfileValidation {
                    row_id: profile.row_id,
                    errors,
                }
            })
            .collect()
    }

    /// The sole wire conversion. It returns either a complete validated tier
    /// or `None`; an incomplete/partially valid tier has no representation.
    pub(crate) fn to_intent(
        &self,
        selection_id: Option<&ArchiveSelectionId>,
    ) -> Option<AdpLaunchTierIntent> {
        self.to_intent_result(selection_id).ok()
    }

    pub(crate) fn to_intent_result(
        &self,
        selection_id: Option<&ArchiveSelectionId>,
    ) -> Result<AdpLaunchTierIntent, AdpPublishValidationViolation> {
        let package_format = self.package_format.parsed().ok_or_else(|| {
            if self.package_format.text.trim().is_empty() {
                AdpPublishValidationViolation::LaunchPackageCount { actual: 0 }
            } else {
                AdpPublishValidationViolation::UnsupportedPackageFormat {
                    format: self.package_format.text.clone(),
                }
            }
        })?;
        let mut platforms = Vec::with_capacity(self.platforms.len());
        for raw in &self.platforms {
            let platform = parse_platform_tag(raw).ok_or_else(|| {
                AdpPublishValidationViolation::InvalidLaunchPlatform {
                    platform: raw.clone(),
                }
            })?;
            if platforms.contains(&platform) {
                return Err(AdpPublishValidationViolation::InvalidLaunchPlatform {
                    platform: raw.clone(),
                });
            }
            platforms.push(platform);
        }
        if platforms.is_empty() {
            return Err(AdpPublishValidationViolation::MissingLaunchPlatform);
        }

        let mut launch_profiles = Vec::with_capacity(self.profiles.len());
        for draft in &self.profiles {
            let profile = draft.to_profile_result(selection_id)?;
            if launch_profiles
                .iter()
                .any(|existing: &LaunchProfile| existing.id == profile.id)
            {
                return Err(AdpPublishValidationViolation::DuplicateLaunchProfileId {
                    profile_id: profile.id,
                });
            }
            launch_profiles.push(profile);
        }
        if !launch_profiles
            .iter()
            .any(|profile| profile.launch_type == LaunchType::Game)
        {
            return Err(AdpPublishValidationViolation::MissingGameLaunchProfile);
        }
        if self.publisher_default_assignments.is_empty() {
            return Err(AdpPublishValidationViolation::MissingLaunchDefault);
        }
        let mut assignments = Vec::<(usize, PlatformTarget)>::new();
        for assignment in &self.publisher_default_assignments {
            let Some(profile_index) = self
                .profiles
                .iter()
                .position(|profile| profile.row_id == assignment.row_id)
            else {
                return Err(AdpPublishValidationViolation::MissingLaunchDefault);
            };
            let profile = &launch_profiles[profile_index];
            if profile.launch_type != LaunchType::Game {
                return Err(AdpPublishValidationViolation::NonGameLaunchDefault {
                    profile_id: profile.id.clone(),
                });
            }
            let target = parse_platform_tag(&assignment.target).ok_or_else(|| {
                AdpPublishValidationViolation::InvalidLaunchDefaultTarget {
                    target: assignment.target.clone(),
                }
            })?;
            if !platforms.contains(&target) {
                return Err(
                    AdpPublishValidationViolation::UndeclaredLaunchDefaultTarget { target },
                );
            }
            if assignments.iter().any(|(_, existing)| existing == &target) {
                return Err(
                    AdpPublishValidationViolation::OverlappingLaunchDefaultTarget { target },
                );
            }
            assignments.push((profile_index, target));
        }
        for target in &platforms {
            if !assignments.iter().any(|(_, assigned)| assigned == target) {
                return Err(
                    AdpPublishValidationViolation::UncoveredLaunchDefaultTarget {
                        target: target.clone(),
                    },
                );
            }
        }
        let publisher_defaults = launch_profiles
            .iter()
            .enumerate()
            .filter_map(|(profile_index, profile)| {
                let targets = platforms
                    .iter()
                    .filter(|target| {
                        assignments.iter().any(|(assigned_index, assigned_target)| {
                            *assigned_index == profile_index && assigned_target == *target
                        })
                    })
                    .cloned()
                    .collect::<Vec<_>>();
                (!targets.is_empty()).then(|| PublisherLaunchDefault {
                    profile_id: profile.id.clone(),
                    targets,
                })
            })
            .collect();
        Ok(AdpLaunchTierIntent {
            package_format,
            platforms,
            launch_profiles,
            publisher_defaults,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) enum LaunchMode {
    DownloadOnly,
    Launchable(LaunchTierDraft),
}

impl LaunchMode {
    pub(crate) fn to_wire(
        &self,
        selection_id: Option<&ArchiveSelectionId>,
    ) -> Option<Option<AdpLaunchTierIntent>> {
        match self {
            Self::DownloadOnly => Some(None),
            Self::Launchable(draft) => draft.to_intent(selection_id).map(Some),
        }
    }

    pub(crate) fn to_wire_result(
        &self,
        selection_id: Option<&ArchiveSelectionId>,
    ) -> Result<Option<AdpLaunchTierIntent>, AdpPublishValidationViolation> {
        match self {
            Self::DownloadOnly => Ok(None),
            Self::Launchable(draft) => draft.to_intent_result(selection_id).map(Some),
        }
    }

    pub(crate) fn mark_bindings_stale(&mut self) {
        if let Self::Launchable(draft) = self {
            draft.mark_bindings_stale();
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum InvalidAuthority {
    MissingEventId,
    /// The original typed boundary failure is retained for diagnostics and
    /// retry policy rather than flattened into display text.
    Failure(PublishAdpListingError),
}

/// Seven required semantic authority outcomes plus a typed invalid-authority
/// state for signed-event failures outside the launch-tier table.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum AuthoritativeEditState {
    Loading,
    RelayError(PublishAdpListingError),
    NotFound(PublishAdpListingError),
    Absent,
    PresentValid(AuthoritativeAdpEditMetadataPresent),
    PresentDefective(AuthoritativeAdpEditMetadataPresent),
    Stale(PublishAdpListingError),
    InvalidAuthority(InvalidAuthority),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct AuthorityRequestIdentity {
    pub owner_npub: String,
    pub generation: u64,
    pub expected_event_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct AuthoritativeEditModel {
    pub owner_npub: String,
    pub state: AuthoritativeEditState,
    pub draft: LaunchMode,
    generation: u64,
    next_row_id: u64,
}

impl AuthoritativeEditModel {
    pub(crate) fn new(owner_npub: String, draft: LaunchMode) -> Self {
        Self {
            owner_npub,
            state: AuthoritativeEditState::Loading,
            draft,
            generation: 0,
            next_row_id: 1,
        }
    }

    pub(crate) fn begin_load(
        &mut self,
        expected_event_id: Option<String>,
    ) -> Option<AuthorityRequestIdentity> {
        let Some(expected_event_id) = expected_event_id.filter(|id| !id.is_empty()) else {
            self.state = AuthoritativeEditState::InvalidAuthority(InvalidAuthority::MissingEventId);
            return None;
        };
        self.generation = self.generation.wrapping_add(1);
        self.state = AuthoritativeEditState::Loading;
        Some(AuthorityRequestIdentity {
            owner_npub: self.owner_npub.clone(),
            generation: self.generation,
            expected_event_id,
        })
    }

    pub(crate) fn apply_response(
        &mut self,
        request: &AuthorityRequestIdentity,
        response: Result<AuthoritativeAdpEditMetadata, PublishAdpListingError>,
    ) -> Transition<()> {
        if request.owner_npub != self.owner_npub || request.generation != self.generation {
            return Transition::Stale;
        }

        match response {
            Ok(AuthoritativeAdpEditMetadata::Absent) => {
                self.state = AuthoritativeEditState::Absent;
                self.draft = LaunchMode::DownloadOnly;
            }
            Ok(AuthoritativeAdpEditMetadata::Present(metadata)) => {
                self.draft = LaunchMode::Launchable(project_authoritative_draft(
                    &metadata,
                    &mut self.next_row_id,
                ));
                self.state = if metadata.defects.is_empty() {
                    AuthoritativeEditState::PresentValid(metadata)
                } else {
                    AuthoritativeEditState::PresentDefective(metadata)
                };
            }
            Err(error) => self.state = authority_error_state(error),
        }
        Transition::Applied(())
    }
}

fn authority_error_state(error: PublishAdpListingError) -> AuthoritativeEditState {
    match error {
        error @ (PublishAdpListingError::AuthoritativeRelayRefetchFailure { .. }
        | PublishAdpListingError::Relay { .. }
        | PublishAdpListingError::Transport { .. }) => AuthoritativeEditState::RelayError(error),
        error @ PublishAdpListingError::AuthoritativeListingNotFound { .. } => {
            AuthoritativeEditState::NotFound(error)
        }
        error @ PublishAdpListingError::StaleExpectedEventId { .. } => {
            AuthoritativeEditState::Stale(error)
        }
        other => invalid_authority(InvalidAuthority::Failure(other)),
    }
}

fn invalid_authority(failure: InvalidAuthority) -> AuthoritativeEditState {
    AuthoritativeEditState::InvalidAuthority(failure)
}

fn project_authoritative_draft(
    metadata: &AuthoritativeAdpEditMetadataPresent,
    next_row_id: &mut u64,
) -> LaunchTierDraft {
    let package_tag = metadata
        .tags
        .iter()
        .filter_map(|tag| match tag {
            AuthoritativeAdpEditTag::Package {
                event_tag_index,
                raw,
                package_format,
            } => Some((*event_tag_index, raw, package_format)),
            _ => None,
        })
        .min_by_key(|(event_tag_index, _, _)| *event_tag_index);
    let package_format = package_tag
        .map(|(_, raw, parsed)| {
            parsed
                .as_ref()
                .map(package_format_text)
                .or_else(|| raw.get(1).cloned())
                .unwrap_or_default()
        })
        .unwrap_or_default();

    let mut platforms: Vec<_> = metadata
        .tags
        .iter()
        .filter_map(|tag| match tag {
            AuthoritativeAdpEditTag::Platform {
                event_tag_index,
                raw_platform,
                target,
                ..
            } => Some((
                *event_tag_index,
                target
                    .as_ref()
                    .map(platform_text)
                    .or_else(|| raw_platform.clone())
                    .unwrap_or_default(),
            )),
            _ => None,
        })
        .collect();
    platforms.sort_by_key(|(index, _)| *index);

    let mut profiles: Vec<_> = metadata
        .tags
        .iter()
        .filter_map(|tag| match tag {
            AuthoritativeAdpEditTag::Launch {
                event_tag_index,
                profile: Some(profile),
                ..
            } => Some((*event_tag_index, profile)),
            _ => None,
        })
        .collect();
    profiles.sort_by_key(|(index, _)| *index);
    let profiles: Vec<_> = profiles
        .into_iter()
        .map(|(_, profile)| {
            let row_id = DraftRowId(*next_row_id);
            *next_row_id = next_row_id.wrapping_add(1);
            LaunchProfileDraft {
                row_id,
                profile_id: profile.id.as_str().to_string(),
                display_name: profile.display_name.clone(),
                executable: ArchivePathDraft {
                    text: profile.executable.as_str().to_string(),
                    confirmed: None,
                    authoritative_value: Some(profile.executable.clone()),
                    stale: false,
                },
                working_dir: ArchivePathDraft {
                    text: profile.working_dir.as_str().to_string(),
                    confirmed: None,
                    authoritative_value: Some(profile.working_dir.clone()),
                    stale: false,
                },
                arguments: profile.arguments.clone(),
                launch_type: profile.launch_type.clone(),
            }
        })
        .collect();

    let has_launch_default_tag = metadata
        .tags
        .iter()
        .any(|tag| matches!(tag, AuthoritativeAdpEditTag::LaunchDefault { .. }));
    let overlapping_targets = metadata
        .defects
        .iter()
        .filter_map(|defect| match defect {
            arcadestr_core::adp_publish_contract::AuthoritativeAdpEditDefect::OverlappingLaunchDefaultTarget {
                target,
                ..
            } => Some(platform_text(target)),
            _ => None,
        })
        .collect::<Vec<_>>();
    let mut publisher_default_assignments = metadata
        .publisher_defaults
        .iter()
        .filter_map(|default| {
            profiles
                .iter()
                .find(|profile| profile.profile_id == default.profile_id.as_str())
                .map(|profile| (profile.row_id, &default.targets))
        })
        .flat_map(|(row_id, targets)| {
            let overlapping_targets = overlapping_targets.clone();
            targets.iter().filter_map(move |target| {
                let target = platform_text(target);
                (!overlapping_targets.contains(&target))
                    .then_some(PublisherDefaultAssignmentDraft { target, row_id })
            })
        })
        .collect::<Vec<_>>();
    if !has_launch_default_tag {
        if let Some(row_id) = profiles
            .iter()
            .find(|profile| profile.launch_type == LaunchType::Game)
            .map(|profile| profile.row_id)
        {
            publisher_default_assignments.extend(platforms.iter().filter_map(|(_, target)| {
                parse_platform_tag(target).map(|canonical| PublisherDefaultAssignmentDraft {
                    target: platform_text(&canonical),
                    row_id,
                })
            }));
        }
    }
    let automatic_profile_rows =
        publisher_default_assignments
            .iter()
            .fold(Vec::new(), |mut rows, assignment| {
                if !rows.contains(&assignment.row_id) {
                    rows.push(assignment.row_id);
                }
                rows
            });

    LaunchTierDraft {
        package_format: PackageFormatDraft {
            text: package_format,
        },
        platforms: platforms.into_iter().map(|(_, value)| value).collect(),
        profiles,
        publisher_default_assignments,
        automatic_profile_rows,
        next_row_id: *next_row_id,
        candidate_generation: 0,
    }
}

fn package_format_text(format: &PackageFormat) -> String {
    match format {
        PackageFormat::Zip => "zip",
        PackageFormat::TarZstd => "tar.zst",
        PackageFormat::TarGz => "tar.gz",
    }
    .to_string()
}

fn platform_text(platform: &PlatformTarget) -> String {
    format!("{}-{}", platform.os, platform.arch)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PreparedPublish {
    pub request: PublishAdpListingRequest,
    pub extension: PublishAdpListingRequestExtension,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct PublishAttempt {
    pub prepared: Option<PreparedPublish>,
    pub error: Option<PublishAdpListingError>,
    pub result: Option<PublishAdpListingResult>,
    pub partial_successful_servers: Vec<String>,
    pub in_flight: bool,
}

impl PublishAttempt {
    pub(crate) fn start(&mut self, prepared: PreparedPublish) {
        self.prepared = Some(prepared);
        self.error = None;
        self.result = None;
        self.partial_successful_servers.clear();
        self.in_flight = true;
    }

    pub(crate) fn succeed(&mut self, result: PublishAdpListingResult) {
        self.result = Some(result);
        self.error = None;
        self.partial_successful_servers.clear();
        self.in_flight = false;
    }

    pub(crate) fn fail(&mut self, error: PublishAdpListingError) {
        self.partial_successful_servers = partial_successful_servers(&error);
        self.error = Some(error);
        self.result = None;
        self.in_flight = false;
        // `prepared` intentionally survives so retry submits the exact draft,
        // selection, extension, and removal acknowledgement state.
    }
}

fn partial_successful_servers(error: &PublishAdpListingError) -> Vec<String> {
    match error {
        PublishAdpListingError::PartialPublication {
            successful_servers, ..
        } => successful_servers.clone(),
        _ => Vec::new(),
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct PublishIdentityOverlay {
    pub existing_event_id: Option<String>,
    pub d_tag: Option<String>,
    pub pending_d_tag: Option<String>,
}

/// Applies retry identity without reconstructing the prepared payload. In
/// particular, `selected_build` and `extension` are left byte-for-byte equal.
pub(crate) fn overlay_publish_identity(
    prepared: &PreparedPublish,
    overlay: PublishIdentityOverlay,
) -> PreparedPublish {
    let mut updated = prepared.clone();
    updated.request.existing_event_id = overlay.existing_event_id;
    updated.request.existing_d_tag = overlay.d_tag;
    updated.request.pending_d_tag = overlay.pending_d_tag;
    updated
}

pub(crate) fn partial_retry_payload(
    attempt: &PublishAttempt,
    current: PreparedPublish,
    overlay: PublishIdentityOverlay,
) -> PreparedPublish {
    attempt
        .prepared
        .as_ref()
        .filter(|_| {
            matches!(
                attempt.error,
                Some(PublishAdpListingError::PartialPublication { .. })
            )
        })
        .map(|previous| overlay_publish_identity(previous, overlay))
        .filter(|previous| previous == &current)
        .unwrap_or(current)
}

pub(crate) fn preserve_edited_platforms(
    mut projected: LaunchMode,
    current_platforms: Vec<String>,
    platform_user_revision: u64,
) -> LaunchMode {
    if platform_user_revision > 0 {
        if let LaunchMode::Launchable(tier) = &mut projected {
            tier.replace_platforms(current_platforms);
        }
    }
    projected
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ArtifactFlow {
    NoArtifact,
    SelectedArtifact,
    ExactExistingReuse,
    SelectedRequired,
}

pub(crate) fn classify_artifact_flow(
    artifact_required: bool,
    selected_build: Option<&BuildSelectionRef>,
    existing_digest: Option<&str>,
    desired_digest: Option<&str>,
) -> ArtifactFlow {
    if !artifact_required {
        ArtifactFlow::NoArtifact
    } else if selected_build.is_some() {
        ArtifactFlow::SelectedArtifact
    } else if existing_digest.is_some() && existing_digest == desired_digest {
        ArtifactFlow::ExactExistingReuse
    } else {
        ArtifactFlow::SelectedRequired
    }
}

pub(crate) fn prepare_publish(
    mut request: PublishAdpListingRequest,
    extension: PublishAdpListingRequestExtension,
    selected_build: Option<BuildSelectionRef>,
    artifact_required: bool,
    desired_digest: Option<&str>,
) -> Result<PreparedPublish, ArchiveSelectionError> {
    let flow = classify_artifact_flow(
        artifact_required,
        selected_build.as_ref(),
        request.existing_file_hash.as_deref(),
        desired_digest,
    );
    if flow == ArtifactFlow::SelectedRequired {
        return Err(ArchiveSelectionError::SelectedBuildRequired);
    }
    if flow == ArtifactFlow::NoArtifact && extension.launch_tier.is_some() {
        return Err(ArchiveSelectionError::SelectedBuildRequired);
    }
    request.file_path = None;
    request.selected_build = (flow == ArtifactFlow::SelectedArtifact)
        .then_some(selected_build)
        .flatten();
    if flow == ArtifactFlow::NoArtifact {
        request.existing_file_hash = None;
    }
    Ok(PreparedPublish { request, extension })
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ArchiveDirtyIdentity {
    pub selection_id: ArchiveSelectionId,
    pub sha256: Option<Sha256Digest>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct BoundPathSnapshot {
    pub text: String,
    pub confirmed_selection_id: Option<ArchiveSelectionId>,
    pub authoritative: bool,
    pub stale: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ProfileDirtySnapshot {
    pub row_id: DraftRowId,
    pub profile_id: String,
    pub display_name: String,
    pub executable: BoundPathSnapshot,
    pub working_dir: BoundPathSnapshot,
    pub arguments: Vec<String>,
    pub launch_type: LaunchType,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum LaunchDirtySnapshot {
    DownloadOnly,
    Launchable {
        package_format: String,
        platforms: Vec<String>,
        profiles: Vec<ProfileDirtySnapshot>,
        publisher_default_assignments: Vec<PublisherDefaultAssignmentDraft>,
        automatic_profile_rows: Vec<DraftRowId>,
    },
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct ProviderBuildFields {
    pub fulfillment_mode: String,
    pub operator_url: Option<String>,
    pub servers: Vec<String>,
    pub existing_file_hash: Option<String>,
    pub existing_fulfillment_pubkey: Option<String>,
    pub version: Option<String>,
    pub nip94_event_id: Option<String>,
    pub platforms: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct DefectReviewState {
    pub reviewed: bool,
    pub accepted_defect_indices: Vec<usize>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PublishDirtySnapshot {
    pub archive: Option<ArchiveDirtyIdentity>,
    pub launch: LaunchDirtySnapshot,
    pub provider_build: ProviderBuildFields,
    pub acknowledged_removed_profile_ids: Vec<ProfileId>,
    pub defect_review: DefectReviewState,
}

impl PublishDirtySnapshot {
    pub(crate) fn capture(
        archive: &ArchiveSelectionState,
        launch: &LaunchMode,
        provider_build: ProviderBuildFields,
        acknowledged_removed_profile_ids: Vec<ProfileId>,
        defect_review: DefectReviewState,
    ) -> Self {
        let archive = archive
            .selection
            .as_ref()
            .map(|selection| ArchiveDirtyIdentity {
                selection_id: selection.selection_id.clone(),
                sha256: archive
                    .selected_build
                    .as_ref()
                    .filter(|build| build.selection_id == selection.selection_id)
                    .map(|build| build.sha256.clone()),
            });
        let launch = match launch {
            LaunchMode::DownloadOnly => LaunchDirtySnapshot::DownloadOnly,
            LaunchMode::Launchable(tier) => LaunchDirtySnapshot::Launchable {
                package_format: tier.package_format.text.clone(),
                platforms: tier.platforms.clone(),
                publisher_default_assignments: tier.publisher_default_assignments.clone(),
                automatic_profile_rows: tier.automatic_profile_rows.clone(),
                profiles: tier
                    .profiles
                    .iter()
                    .map(|profile| ProfileDirtySnapshot {
                        row_id: profile.row_id,
                        profile_id: profile.profile_id.clone(),
                        display_name: profile.display_name.clone(),
                        executable: bound_path_snapshot(&profile.executable),
                        working_dir: bound_path_snapshot(&profile.working_dir),
                        arguments: profile.arguments.clone(),
                        launch_type: profile.launch_type.clone(),
                    })
                    .collect(),
            },
        };
        Self {
            archive,
            launch,
            provider_build,
            acknowledged_removed_profile_ids,
            defect_review,
        }
    }
}

fn bound_path_snapshot<T>(path: &ArchivePathDraft<T>) -> BoundPathSnapshot {
    BoundPathSnapshot {
        text: path.text.clone(),
        confirmed_selection_id: path
            .confirmed
            .as_ref()
            .map(|confirmed| confirmed.selection_id.clone()),
        authoritative: path.authoritative_value.is_some(),
        stale: path.stale,
    }
}

/// Coordinates archive invalidation with launch bindings while keeping both
/// pure submodels independently testable.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PublishState {
    pub archive: ArchiveSelectionState,
    pub launch: LaunchMode,
}

impl PublishState {
    pub(crate) fn apply_picker(
        &mut self,
        operation: &ArchiveOperationIdentity,
        response: Result<Option<ArchiveFileSelection>, ArchiveSelectionError>,
    ) -> Transition<Option<ArchiveFileSelection>> {
        let selected = response.as_ref().is_ok_and(Option::is_some);
        let transition = self.archive.apply_picker(operation, response);
        if selected && matches!(transition, Transition::Applied(_)) {
            self.launch.mark_bindings_stale();
        }
        transition
    }

    pub(crate) fn invalidate_account(
        &mut self,
        next_owner_npub: String,
        failure: ArchiveAccountFailure,
    ) -> Option<ArchiveFileSelection> {
        let selection = self.archive.invalidate_account(next_owner_npub, failure);
        self.launch.mark_bindings_stale();
        selection
    }

    pub(crate) fn archive_replaced(&mut self) {
        self.launch.mark_bindings_stale();
    }

    pub(crate) fn replace_launch_platforms(&mut self, platforms: Vec<String>) {
        if let LaunchMode::Launchable(tier) = &mut self.launch {
            let generation = tier.candidate_generation();
            tier.replace_platforms(platforms);
            if tier.candidate_generation() != generation {
                self.archive.queries.invalidate_candidates();
            }
        }
    }

    pub(crate) fn consume_selected_archive(&mut self) {
        self.archive.consume_success();
        self.launch.mark_bindings_stale();
    }

    pub(crate) fn apply_archive_error(&mut self, error: ArchiveSelectionError) {
        if archive_error_is_terminal(&error) {
            self.archive.apply_operation_error(error);
            self.launch.mark_bindings_stale();
        } else {
            self.archive.error = Some(error);
            self.archive.busy = false;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arcadestr_core::adp_publish_contract::AuthoritativeAdpEditDefect;
    use arcadestr_core::archive_inspection_contract::{
        ArchiveHazardCounts, ArchiveInspectionCounts, ArchiveWarningCounts, SafeArchiveFilename,
    };

    const SELECTION_A: &str = "2f9a1c34-5b6d-4e7f-8a9b-0c1d2e3f4a5b";
    const SELECTION_B: &str = "6d4b5a21-812e-49bc-a753-779d6f8cfed0";
    const REQUEST_A: &str = "9b8c7d61-2345-4abc-8def-1234567890ab";
    const REQUEST_B: &str = "7a6b5c41-1234-4def-9abc-0123456789ab";
    const DIGEST_A: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    fn selection_id(value: &str) -> ArchiveSelectionId {
        ArchiveSelectionId::parse(value).expect("valid selection fixture")
    }

    fn request_id(value: &str) -> ArchiveRequestId {
        ArchiveRequestId::parse(value).expect("valid request fixture")
    }

    fn digest() -> Sha256Digest {
        Sha256Digest::parse(DIGEST_A).expect("valid digest fixture")
    }

    fn selection(value: &str, expires_at_unix: u64) -> ArchiveFileSelection {
        ArchiveFileSelection {
            selection_id: selection_id(value),
            safe_filename: SafeArchiveFilename::parse("game.zip").expect("safe filename"),
            file_size: 42,
            expires_at_unix,
        }
    }

    fn build(value: &str) -> BuildSelectionRef {
        BuildSelectionRef {
            selection_id: selection_id(value),
            sha256: digest(),
        }
    }

    fn summary(value: &str) -> ArchiveSelectionSummary {
        ArchiveSelectionSummary {
            selection: selection(value, 1),
            detected_format: PackageFormat::Zip,
            extension_format: Some(PackageFormat::Zip),
            extension_mismatch_warning: None,
            counts: ArchiveInspectionCounts {
                logical_entries: 1,
                indexed_entries: 1,
                undecodable_entries: 0,
                files: 1,
                directory_entries: 0,
                symlinks: 0,
                hardlinks: 0,
                other_entry_kinds: 0,
                working_directories: 1,
                hazards: ArchiveHazardCounts {
                    path_rule: 0,
                    symlink: 0,
                    hardlink: 0,
                    collision: 0,
                    other: 0,
                    undecodable: 0,
                    type_conflict: 0,
                    zip_count_mismatch: 0,
                },
                warnings: ArchiveWarningCounts {
                    efs_divergence: 0,
                    extension_mismatch: 0,
                },
            },
            blocking_hazards: false,
        }
    }

    fn valid_tier(selection: &str) -> LaunchTierDraft {
        let mut tier = LaunchTierDraft::default();
        tier.package_format.text = "zip".to_string();
        tier.replace_platforms(vec!["linux-x86_64".to_string()]);
        let row_id = tier.add_profile();
        let profile = tier
            .profiles
            .iter_mut()
            .find(|profile| profile.row_id == row_id)
            .expect("new row exists");
        profile.profile_id = "game".to_string();
        profile.display_name = "Game".to_string();
        profile.executable.confirm(
            selection_id(selection),
            RelativePath::parse("bin/game").expect("valid path"),
            "bin/game".to_string(),
        );
        profile.working_dir.confirm(
            selection_id(selection),
            WorkingDir::parse("bin").expect("valid working directory"),
            "bin".to_string(),
        );
        profile.arguments = vec!["--fullscreen".to_string()];
        assert!(tier.set_publisher_default_profile(row_id, true));
        tier
    }

    fn request() -> PublishAdpListingRequest {
        PublishAdpListingRequest {
            expected_publisher_npub: "npub-owner".to_string(),
            existing_event_id: None,
            existing_d_tag: None,
            pending_d_tag: None,
            title: "Game".to_string(),
            description: "Description".to_string(),
            price_sats: 0,
            lud16: None,
            tags: Vec::new(),
            images: Vec::new(),
            fulfillment_mode: crate::tauri_bridge::FulfillmentMode::None,
            operator_url: None,
            servers: Vec::new(),
            accepted_terms: Vec::new(),
            file_path: Some("must-not-cross-boundary".to_string()),
            selected_build: None,
            existing_file_hash: None,
            existing_fulfillment_pubkey: None,
            version: None,
            acquisition: crate::models::AcquisitionPolicy::Gated,
            platforms: Vec::new(),
            campaigns: Vec::new(),
            nip94_event_id: None,
        }
    }

    #[test]
    fn picker_cancellation_and_failure_preserve_prior_selection() {
        let mut state = ArchiveSelectionState::new("owner".to_string());
        state.selection = Some(selection(SELECTION_A, 10));
        let operation = state.begin_picker(request_id(REQUEST_A));
        assert_eq!(
            state.apply_picker(&operation, Ok(None)),
            Transition::Applied(None)
        );
        assert_eq!(
            state
                .selection
                .as_ref()
                .map(|item| item.selection_id.as_str()),
            Some(SELECTION_A)
        );

        let operation = state.begin_picker(request_id(REQUEST_B));
        assert_eq!(
            state.apply_picker(&operation, Err(ArchiveSelectionError::PickerFailed)),
            Transition::Applied(None)
        );
        assert_eq!(
            state
                .selection
                .as_ref()
                .map(|item| item.selection_id.as_str()),
            Some(SELECTION_A)
        );
    }

    #[test]
    fn picker_replacement_returns_old_selection_and_stales_bindings() {
        let mut state = PublishState {
            archive: ArchiveSelectionState::new("owner".to_string()),
            launch: LaunchMode::Launchable(valid_tier(SELECTION_A)),
        };
        state.archive.selection = Some(selection(SELECTION_A, 10));
        let operation = state.archive.begin_picker(request_id(REQUEST_A));
        let replaced = state.apply_picker(&operation, Ok(Some(selection(SELECTION_B, 20))));
        assert!(matches!(replaced, Transition::Applied(Some(_))));
        let LaunchMode::Launchable(tier) = &state.launch else {
            panic!("launch tier retained");
        };
        assert!(tier.profiles[0].executable.stale);
        assert_eq!(tier.profiles[0].executable.text, "bin/game");
        assert_eq!(
            tier.profiles[0]
                .executable
                .confirmed
                .as_ref()
                .map(|binding| binding.selection_id.as_str()),
            Some(SELECTION_A)
        );
    }

    #[test]
    fn stale_archive_response_is_rejected_by_generation_and_owner() {
        let mut state = ArchiveSelectionState::new("owner".to_string());
        state.selection = Some(selection(SELECTION_A, 10));
        let old = state
            .begin_selection_operation(request_id(REQUEST_A))
            .expect("selection operation");
        let _new = state
            .begin_selection_operation(request_id(REQUEST_B))
            .expect("selection operation");
        assert_eq!(
            state.apply_summary(&old, Ok(summary(SELECTION_A))),
            Transition::Stale
        );
        assert!(state.summary.is_none());

        let mut wrong_owner = old;
        wrong_owner.operation_generation = state.operation_generation;
        wrong_owner.owner_npub = "other".to_string();
        assert_eq!(
            state.apply_summary(&wrong_owner, Ok(summary(SELECTION_A))),
            Transition::Stale
        );

        state.selected_build = Some(build(SELECTION_A));
        let old_hash = state
            .begin_selection_operation(request_id(REQUEST_A))
            .expect("hash operation");
        let _new_hash = state
            .begin_selection_operation(request_id(REQUEST_B))
            .expect("newer hash operation");
        let replacement_digest = Sha256Digest::parse(&"22".repeat(32)).expect("digest");
        assert_eq!(
            state.apply_hash(
                &old_hash,
                Ok(BuildSelectionRef {
                    selection_id: selection_id(SELECTION_A),
                    sha256: replacement_digest,
                }),
            ),
            Transition::Stale
        );
        assert_eq!(
            state
                .selected_build
                .as_ref()
                .map(|build| build.sha256.as_str()),
            Some(DIGEST_A)
        );
    }

    #[test]
    fn account_and_terminal_failures_invalidate_build_but_transport_retries() {
        let mut state = PublishState {
            archive: ArchiveSelectionState::new("owner".to_string()),
            launch: LaunchMode::Launchable(valid_tier(SELECTION_A)),
        };
        state.archive.selection = Some(selection(SELECTION_A, 10));
        state.archive.selected_build = Some(build(SELECTION_A));
        state.apply_archive_error(ArchiveSelectionError::Transport);
        assert!(state.archive.selected_build.is_some());
        assert!(state.archive.error.is_some());

        state.invalidate_account(
            "other-owner".to_string(),
            ArchiveAccountFailure::AccountMismatch,
        );
        assert!(state.archive.selected_build.is_none());
        assert!(state.archive.selection.is_none());
        let LaunchMode::Launchable(tier) = &state.launch else {
            panic!("launch tier retained");
        };
        assert!(tier.profiles[0].working_dir.stale);
    }

    #[test]
    fn platform_change_invalidates_only_candidate_generation() {
        let tier = valid_tier(SELECTION_A);
        let profile = tier.profiles[0].clone();
        let generation = tier.candidate_generation();
        let mut state = PublishState {
            archive: ArchiveSelectionState::new("owner".to_string()),
            launch: LaunchMode::Launchable(tier),
        };
        state.archive.selection = Some(selection(SELECTION_A, 10));
        state.archive.summary = Some(summary(SELECTION_A));
        state.archive.selected_build = Some(build(SELECTION_A));
        state.archive.queries.candidate_platform = parse_platform_tag("linux-x86_64");

        state.replace_launch_platforms(vec!["windows-x86_64".to_string()]);

        let LaunchMode::Launchable(tier) = state.launch else {
            panic!("launchable tier");
        };
        assert_ne!(tier.candidate_generation(), generation);
        assert_eq!(tier.profiles[0], profile);
        assert!(!tier.profiles[0].executable.stale);
        assert!(state.archive.queries.candidate_platform.is_none());
        assert!(state.archive.build_ready());
    }

    #[test]
    fn partial_launch_tier_can_never_cross_wire_boundary() {
        let selection = selection_id(SELECTION_A);
        let mut tier = valid_tier(SELECTION_A);
        assert!(tier.to_intent(Some(&selection)).is_some());
        tier.profiles[0].display_name.clear();
        assert!(tier.to_intent(Some(&selection)).is_none());
        assert!(matches!(
            tier.to_intent_result(Some(&selection)),
            Err(AdpPublishValidationViolation::InvalidLaunchDisplayName {
                reason: DisplayNameRejection::InvalidLength,
                ..
            })
        ));
        tier.profiles[0].display_name = "Play".to_string();
        tier.profiles[0].profile_id = "INVALID".to_string();
        assert!(matches!(
            tier.to_intent_result(Some(&selection)),
            Err(AdpPublishValidationViolation::MalformedLaunchProfile {
                rejection: AuthoritativeProfileRejection::ProfileId(
                    ProfileIdRejection::InvalidGrammar
                ),
                ..
            })
        ));
        assert_eq!(LaunchMode::Launchable(tier).to_wire(Some(&selection)), None);
        assert_eq!(LaunchMode::DownloadOnly.to_wire(None), Some(None));
    }

    #[test]
    fn profile_rows_add_remove_reorder_and_ignore_live_id_edits_for_identity() {
        let mut tier = LaunchTierDraft::default();
        tier.replace_platforms(vec!["linux-x86_64".to_string()]);
        let first = tier.add_profile();
        let second = tier.add_profile();
        assert!(tier.set_publisher_default_profile(first, true));
        tier.profiles[0].profile_id = "same".to_string();
        tier.profiles[1].profile_id = "same".to_string();
        assert!(tier
            .live_profile_validation()
            .iter()
            .all(|validation| { validation.errors.contains(&ProfileDraftError::DuplicateId) }));

        tier.profiles[0].profile_id = "renamed".to_string();
        assert_eq!(tier.profiles[0].row_id, first);
        assert!(tier.profile_has_publisher_defaults(first));
        assert!(tier.reorder_profile(second, 0));
        assert_eq!(tier.profiles[0].row_id, second);
        assert!(tier.profile_has_publisher_defaults(first));
        assert_eq!(
            tier.remove_profile(first).map(|row| row.row_id),
            Some(first)
        );
        assert!(tier.publisher_default_assignments.is_empty());
        assert_eq!(tier.profiles.len(), 1);
    }

    #[test]
    fn publisher_defaults_require_each_target_and_existing_game_rows() {
        let selection = selection_id(SELECTION_A);
        let mut tier = valid_tier(SELECTION_A);
        let default_row = tier.profiles[0].row_id;

        tier.publisher_default_assignments[0].row_id = DraftRowId(999);
        assert_eq!(
            tier.to_intent_result(Some(&selection)),
            Err(AdpPublishValidationViolation::MissingLaunchDefault)
        );

        tier.publisher_default_assignments[0].row_id = default_row;
        assert!(tier.set_profile_launch_type(default_row, LaunchType::Tool));
        assert!(tier.publisher_default_assignments.is_empty());
        assert_eq!(
            tier.to_intent_result(Some(&selection)),
            Err(AdpPublishValidationViolation::MissingGameLaunchProfile)
        );

        assert!(tier.set_profile_launch_type(default_row, LaunchType::Game));
        assert!(tier.set_publisher_default_profile(default_row, true));
        assert_eq!(
            tier.to_intent_result(Some(&selection))
                .expect("reselected game should publish")
                .publisher_defaults[0]
                .profile_id
                .as_str(),
            "game"
        );

        tier.profiles[0].launch_type = LaunchType::Tool;
        assert_eq!(
            tier.to_intent_result(Some(&selection)),
            Err(AdpPublishValidationViolation::MissingGameLaunchProfile)
        );
    }

    #[test]
    fn target_assignments_move_atomically_and_survive_rename_and_reorder() {
        let selection = selection_id(SELECTION_A);
        let mut tier = valid_tier(SELECTION_A);
        tier.replace_platforms(vec![
            "linux-x86_64".to_string(),
            "windows-x86_64".to_string(),
        ]);
        let first = tier.profiles[0].row_id;
        let template = tier.profiles[0].clone();
        let second = tier.add_profile();
        let second_profile = tier
            .profiles
            .iter_mut()
            .find(|profile| profile.row_id == second)
            .expect("second profile");
        *second_profile = LaunchProfileDraft {
            row_id: second,
            profile_id: "windows-game".to_string(),
            display_name: "Windows Game".to_string(),
            ..template
        };

        assert!(tier.set_publisher_default_profile(second, true));
        assert!(tier.assign_publisher_default(second, "windows-x86_64", true));
        assert!(tier.assign_publisher_default(second, "linux-x86_64", true));
        assert!(!tier.profile_has_publisher_defaults(first));
        assert!(tier.profile_has_publisher_defaults(second));
        assert!(tier.assign_publisher_default(first, "linux-x86_64", true));
        tier.profiles
            .iter_mut()
            .find(|profile| profile.row_id == first)
            .expect("first profile")
            .profile_id = "renamed-game".to_string();
        assert!(tier.reorder_profile(second, 0));

        let intent = tier
            .to_intent_result(Some(&selection))
            .expect("both targets are explicitly covered");
        assert_eq!(intent.publisher_defaults.len(), 2);
        assert_eq!(
            intent.publisher_defaults[0].profile_id.as_str(),
            "windows-game"
        );
        assert_eq!(
            intent.publisher_defaults[1].profile_id.as_str(),
            "renamed-game"
        );
    }

    #[test]
    fn removal_type_and_platform_changes_drop_assignments_without_replacement() {
        let mut tier = valid_tier(SELECTION_A);
        let first = tier.profiles[0].row_id;
        tier.replace_platforms(vec![
            "linux-x86_64".to_string(),
            "windows-x86_64".to_string(),
        ]);
        assert!(tier.assign_publisher_default(first, "windows-x86_64", true));
        tier.replace_platforms(vec!["windows-x86_64".to_string()]);
        assert_eq!(tier.publisher_default_assignments.len(), 1);
        assert_eq!(
            tier.publisher_default_assignments[0].target,
            "windows-x86_64"
        );

        assert!(tier.set_profile_launch_type(first, LaunchType::Tool));
        assert!(tier.publisher_default_assignments.is_empty());
        assert!(tier.automatic_profile_rows.is_empty());
        assert!(tier.set_profile_launch_type(first, LaunchType::Game));
        assert!(tier.publisher_default_assignments.is_empty());
        assert_eq!(
            tier.remove_profile(first).map(|profile| profile.row_id),
            Some(first)
        );
        assert!(tier.publisher_default_assignments.is_empty());
    }

    fn authoritative_profile() -> LaunchProfile {
        LaunchProfile {
            id: ProfileId::parse("game").expect("profile id"),
            display_name: "Game".to_string(),
            executable: RelativePath::parse("game.exe").expect("relative path"),
            working_dir: WorkingDir::parse(".").expect("working directory"),
            arguments: vec!["--safe".to_string()],
            launch_type: LaunchType::Game,
        }
    }

    fn metadata(defective: bool) -> AuthoritativeAdpEditMetadataPresent {
        AuthoritativeAdpEditMetadataPresent {
            tags: vec![
                AuthoritativeAdpEditTag::Launch {
                    event_tag_index: 8,
                    raw: vec!["launch".to_string(), "raw-evidence".to_string()],
                    raw_profile_id: Some("game".to_string()),
                    valid_profile_id: Some(ProfileId::parse("game").expect("profile id")),
                    profile: Some(authoritative_profile()),
                    rejection: None,
                },
                AuthoritativeAdpEditTag::Package {
                    event_tag_index: 2,
                    raw: vec!["package".to_string(), "zip".to_string()],
                    package_format: Some(PackageFormat::Zip),
                },
                AuthoritativeAdpEditTag::Platform {
                    event_tag_index: 4,
                    raw: vec!["platform".to_string(), "linux-x86_64".to_string()],
                    raw_platform: Some("linux-x86_64".to_string()),
                    target: parse_platform_tag("linux-x86_64"),
                },
                AuthoritativeAdpEditTag::LaunchDefault {
                    event_tag_index: 9,
                    raw: vec!["launch_default".to_string(), "game".to_string()],
                    raw_profile_id: Some("game".to_string()),
                    valid_profile_id: Some(ProfileId::parse("game").expect("profile id")),
                    raw_targets: Vec::new(),
                    targets: Vec::new(),
                    legacy: true,
                },
            ],
            defects: defective
                .then_some(AuthoritativeAdpEditDefect::MissingPlatform)
                .into_iter()
                .collect(),
            publisher_defaults: vec![PublisherLaunchDefault {
                profile_id: ProfileId::parse("game").expect("publisher default profile id"),
                targets: vec![parse_platform_tag("linux-x86_64").expect("target")],
            }],
        }
    }

    #[test]
    fn authority_has_seven_outcomes_and_preserves_raw_defects() {
        let mut model = AuthoritativeEditModel::new("owner".to_string(), LaunchMode::DownloadOnly);
        assert!(matches!(model.state, AuthoritativeEditState::Loading));

        let request = model
            .begin_load(Some("event".to_string()))
            .expect("request");
        model.apply_response(
            &request,
            Err(PublishAdpListingError::AuthoritativeRelayRefetchFailure {
                message: "offline".to_string(),
            }),
        );
        assert!(matches!(model.state, AuthoritativeEditState::RelayError(_)));

        let request = model
            .begin_load(Some("event".to_string()))
            .expect("request");
        model.apply_response(
            &request,
            Err(PublishAdpListingError::AuthoritativeListingNotFound {
                coordinate: "30402:owner:id".to_string(),
            }),
        );
        assert!(matches!(model.state, AuthoritativeEditState::NotFound(_)));

        let request = model
            .begin_load(Some("event".to_string()))
            .expect("request");
        model.apply_response(&request, Ok(AuthoritativeAdpEditMetadata::Absent));
        assert!(matches!(model.state, AuthoritativeEditState::Absent));

        let request = model
            .begin_load(Some("event".to_string()))
            .expect("request");
        model.apply_response(
            &request,
            Ok(AuthoritativeAdpEditMetadata::Present(metadata(false))),
        );
        assert!(matches!(
            model.state,
            AuthoritativeEditState::PresentValid(_)
        ));
        let LaunchMode::Launchable(authoritative_draft) = &model.draft else {
            panic!("authoritative launch draft");
        };
        assert!(authoritative_draft.to_intent(None).is_some());
        let merged =
            preserve_edited_platforms(model.draft.clone(), vec!["windows-x86_64".to_string()], 1);
        let LaunchMode::Launchable(merged) = merged else {
            panic!("merged authoritative launch draft");
        };
        assert_eq!(merged.platforms, vec!["windows-x86_64"]);
        let mut replaced = model.draft.clone();
        replaced.mark_bindings_stale();
        let LaunchMode::Launchable(replaced) = replaced else {
            panic!("authoritative launch draft");
        };
        assert!(replaced.to_intent(None).is_none());

        let request = model
            .begin_load(Some("event".to_string()))
            .expect("request");
        model.apply_response(
            &request,
            Ok(AuthoritativeAdpEditMetadata::Present(metadata(true))),
        );
        let AuthoritativeEditState::PresentDefective(raw) = &model.state else {
            panic!("defective state");
        };
        assert_eq!(raw.tags[0], metadata(true).tags[0]);
        assert_eq!(
            raw.defects,
            vec![AuthoritativeAdpEditDefect::MissingPlatform]
        );

        let request = model
            .begin_load(Some("event".to_string()))
            .expect("request");
        model.apply_response(
            &request,
            Err(PublishAdpListingError::StaleExpectedEventId {
                expected_event_id: "old".to_string(),
                actual_event_id: "new".to_string(),
            }),
        );
        assert!(matches!(model.state, AuthoritativeEditState::Stale(_)));

        assert!(model.begin_load(None).is_none());
        assert!(matches!(
            model.state,
            AuthoritativeEditState::InvalidAuthority(InvalidAuthority::MissingEventId)
        ));
    }

    #[test]
    fn authoritative_defaults_hydrate_by_profile_and_legacy_no_tag_falls_back() {
        let mut next_row_id = 1;
        let current = project_authoritative_draft(&metadata(false), &mut next_row_id);
        assert_eq!(
            current.publisher_default_assignments,
            vec![PublisherDefaultAssignmentDraft {
                target: "linux-x86_64".to_string(),
                row_id: current.profiles[0].row_id,
            }]
        );

        let mut legacy = metadata(false);
        legacy.publisher_defaults.clear();
        legacy
            .tags
            .retain(|tag| !matches!(tag, AuthoritativeAdpEditTag::LaunchDefault { .. }));
        let legacy = project_authoritative_draft(&legacy, &mut next_row_id);
        assert_eq!(
            legacy.publisher_default_assignments,
            vec![PublisherDefaultAssignmentDraft {
                target: "linux-x86_64".to_string(),
                row_id: legacy.profiles[0].row_id,
            }]
        );

        let mut malformed = metadata(false);
        malformed.publisher_defaults.clear();
        let malformed = project_authoritative_draft(&malformed, &mut next_row_id);
        assert!(malformed.publisher_default_assignments.is_empty());
    }

    #[test]
    fn scoped_authoritative_defaults_hydrate_by_target_and_overlap_is_not_chosen() {
        let mut scoped = metadata(false);
        scoped.tags.push(AuthoritativeAdpEditTag::Platform {
            event_tag_index: 5,
            raw: vec!["platform".to_string(), "windows-x86_64".to_string()],
            raw_platform: Some("windows-x86_64".to_string()),
            target: parse_platform_tag("windows-x86_64"),
        });
        let second = LaunchProfile {
            id: ProfileId::parse("windows-game").expect("profile id"),
            display_name: "Windows Game".to_string(),
            ..authoritative_profile()
        };
        scoped.tags.push(AuthoritativeAdpEditTag::Launch {
            event_tag_index: 10,
            raw: vec!["launch".to_string(), "windows-game".to_string()],
            raw_profile_id: Some("windows-game".to_string()),
            valid_profile_id: Some(second.id.clone()),
            profile: Some(second.clone()),
            rejection: None,
        });
        if let Some(AuthoritativeAdpEditTag::LaunchDefault {
            raw,
            raw_targets,
            targets,
            legacy,
            ..
        }) = scoped
            .tags
            .iter_mut()
            .find(|tag| matches!(tag, AuthoritativeAdpEditTag::LaunchDefault { .. }))
        {
            *raw = vec![
                "launch_default".to_string(),
                "game".to_string(),
                "linux-x86_64".to_string(),
            ];
            *raw_targets = vec!["linux-x86_64".to_string()];
            *targets = vec![parse_platform_tag("linux-x86_64").expect("target")];
            *legacy = false;
        }
        scoped.tags.push(AuthoritativeAdpEditTag::LaunchDefault {
            event_tag_index: 11,
            raw: vec![
                "launch_default".to_string(),
                "windows-game".to_string(),
                "windows-x86_64".to_string(),
            ],
            raw_profile_id: Some("windows-game".to_string()),
            valid_profile_id: Some(second.id.clone()),
            raw_targets: vec!["windows-x86_64".to_string()],
            targets: vec![parse_platform_tag("windows-x86_64").expect("target")],
            legacy: false,
        });
        scoped.publisher_defaults.push(PublisherLaunchDefault {
            profile_id: second.id,
            targets: vec![parse_platform_tag("windows-x86_64").expect("target")],
        });

        let mut next_row_id = 1;
        let projected = project_authoritative_draft(&scoped, &mut next_row_id);
        assert_eq!(projected.publisher_default_assignments.len(), 2);
        assert_eq!(
            projected
                .to_intent_result(None)
                .expect("scoped defaults cover both targets")
                .publisher_defaults
                .len(),
            2
        );

        scoped
            .defects
            .push(AuthoritativeAdpEditDefect::OverlappingLaunchDefaultTarget {
                target: parse_platform_tag("linux-x86_64").expect("target"),
                event_tag_indices: vec![9, 11],
            });
        let projected = project_authoritative_draft(&scoped, &mut next_row_id);
        assert!(!projected
            .publisher_default_assignments
            .iter()
            .any(|assignment| assignment.target == "linux-x86_64"));
    }

    #[test]
    fn failed_authority_response_does_not_overwrite_draft() {
        let original = LaunchMode::Launchable(valid_tier(SELECTION_A));
        let mut model = AuthoritativeEditModel::new("owner".to_string(), original.clone());
        let request = model
            .begin_load(Some("event".to_string()))
            .expect("request");
        model.apply_response(
            &request,
            Err(PublishAdpListingError::AuthoritativeSignatureInvalid {
                event_id: "bad".to_string(),
            }),
        );
        assert_eq!(model.draft, original);
    }

    #[test]
    fn retry_preserves_prepared_payload_including_partial_publication() {
        let prepared = prepare_publish(
            request(),
            PublishAdpListingRequestExtension {
                authorize_operators: false,
                server_roles: Vec::new(),
                commercial_intent: Default::default(),
                operation_id: None,
                launch_tier: valid_tier(SELECTION_A).to_intent(Some(&selection_id(SELECTION_A))),
                acknowledged_removed_profile_ids: vec![
                    ProfileId::parse("removed").expect("profile id")
                ],
            },
            Some(build(SELECTION_A)),
            true,
            None,
        )
        .expect("prepared");
        assert!(prepared.request.file_path.is_none());
        let mut attempt = PublishAttempt::default();
        attempt.start(prepared.clone());
        attempt.fail(PublishAdpListingError::PartialPublication {
            cause: Box::new(PublishAdpListingError::Upload {
                message: "second server failed".to_string(),
            }),
            successful_servers: vec!["https://one.example".to_string()],
        });
        assert_eq!(attempt.prepared, Some(prepared));
        assert_eq!(
            attempt.partial_successful_servers,
            vec!["https://one.example"]
        );
        let current = overlay_publish_identity(
            attempt.prepared.as_ref().expect("prepared attempt"),
            PublishIdentityOverlay {
                existing_event_id: Some("published-event".to_string()),
                d_tag: Some("published-id".to_string()),
                pending_d_tag: None,
            },
        );
        let retry = partial_retry_payload(
            &attempt,
            current.clone(),
            PublishIdentityOverlay {
                existing_event_id: Some("published-event".to_string()),
                d_tag: Some("published-id".to_string()),
                pending_d_tag: None,
            },
        );
        assert_eq!(retry, current);
        assert_eq!(
            retry
                .extension
                .launch_tier
                .as_ref()
                .and_then(|tier| tier.publisher_defaults.first())
                .map(|default| default.profile_id.as_str()),
            Some("game")
        );
        assert_eq!(
            retry.request.selected_build,
            attempt
                .prepared
                .as_ref()
                .and_then(|prepared| prepared.request.selected_build.clone())
        );
        assert_eq!(
            retry.extension,
            attempt
                .prepared
                .as_ref()
                .map(|prepared| prepared.extension.clone())
                .expect("prepared extension")
        );
    }

    #[test]
    fn durable_launch_drafts_preserve_text_but_never_restore_archive_capabilities() {
        let original = valid_tier(SELECTION_A);
        let encoded = serde_json::to_value(&original).expect("draft JSON");
        arcadestr_core::publisher_recovery::validate_durable_payload(&encoded)
            .expect("no native handles");
        let restored: LaunchTierDraft = serde_json::from_value(encoded).expect("restore draft");
        assert_eq!(restored.package_format, original.package_format);
        assert_eq!(
            restored.publisher_default_assignments,
            original.publisher_default_assignments
        );
        assert_eq!(
            restored.profiles[0].executable.text,
            original.profiles[0].executable.text
        );
        assert!(restored.profiles[0].executable.confirmed.is_none());
        assert!(restored.profiles[0]
            .executable
            .authoritative_value
            .is_none());
        assert!(restored.to_intent(None).is_none());
    }

    #[test]
    fn identity_overlay_changes_only_identity() {
        let prepared = prepare_publish(
            request(),
            PublishAdpListingRequestExtension::default(),
            Some(build(SELECTION_A)),
            true,
            None,
        )
        .expect("prepared");
        let updated = overlay_publish_identity(
            &prepared,
            PublishIdentityOverlay {
                existing_event_id: Some("event".to_string()),
                d_tag: Some("id".to_string()),
                pending_d_tag: Some("pending".to_string()),
            },
        );
        assert_eq!(
            updated.request.selected_build,
            prepared.request.selected_build
        );
        assert_eq!(updated.extension, prepared.extension);
        assert_eq!(updated.request.existing_event_id.as_deref(), Some("event"));
        assert_eq!(updated.request.existing_d_tag.as_deref(), Some("id"));
    }

    fn dirty_snapshot(
        archive: &ArchiveSelectionState,
        tier: &LaunchTierDraft,
        acknowledgements: Vec<ProfileId>,
    ) -> PublishDirtySnapshot {
        PublishDirtySnapshot::capture(
            archive,
            &LaunchMode::Launchable(tier.clone()),
            ProviderBuildFields::default(),
            acknowledgements,
            DefectReviewState::default(),
        )
    }

    #[test]
    fn dirty_snapshot_tracks_selection_order_arguments_and_acknowledgements() {
        let mut archive = ArchiveSelectionState::new("owner".to_string());
        archive.selection = Some(selection(SELECTION_A, 1));
        archive.selected_build = Some(build(SELECTION_A));
        let mut tier = valid_tier(SELECTION_A);
        let baseline = dirty_snapshot(&archive, &tier, Vec::new());

        archive.selected_build = Some(build(SELECTION_B));
        assert_ne!(baseline, dirty_snapshot(&archive, &tier, Vec::new()));
        archive.selected_build = Some(build(SELECTION_A));

        let second = tier.add_profile();
        assert_ne!(baseline, dirty_snapshot(&archive, &tier, Vec::new()));
        tier.remove_profile(second);
        tier.profiles[0].arguments.push("--new".to_string());
        assert_ne!(baseline, dirty_snapshot(&archive, &tier, Vec::new()));
        tier.profiles[0].arguments.pop();
        assert_ne!(
            baseline,
            dirty_snapshot(
                &archive,
                &tier,
                vec![ProfileId::parse("removed").expect("profile id")]
            )
        );
    }

    #[test]
    fn dirty_snapshot_tracks_profile_reordering() {
        let mut archive = ArchiveSelectionState::new("owner".to_string());
        archive.selection = Some(selection(SELECTION_A, 1));
        archive.selected_build = Some(build(SELECTION_A));
        let mut tier = valid_tier(SELECTION_A);
        let second = tier.add_profile();
        let row = tier
            .profiles
            .iter_mut()
            .find(|profile| profile.row_id == second)
            .expect("second row");
        row.profile_id = "config".to_string();
        row.display_name = "Configuration".to_string();
        let baseline = dirty_snapshot(&archive, &tier, Vec::new());
        assert!(tier.reorder_profile(second, 0));
        assert_ne!(baseline, dirty_snapshot(&archive, &tier, Vec::new()));
    }

    #[test]
    fn dirty_snapshot_tracks_publisher_default_selection() {
        let mut archive = ArchiveSelectionState::new("owner".to_string());
        archive.selection = Some(selection(SELECTION_A, 1));
        archive.selected_build = Some(build(SELECTION_A));
        let mut tier = valid_tier(SELECTION_A);
        let baseline = dirty_snapshot(&archive, &tier, Vec::new());
        tier.publisher_default_assignments.clear();

        assert_ne!(baseline, dirty_snapshot(&archive, &tier, Vec::new()));
    }

    #[test]
    fn selected_build_guard_distinguishes_no_artifact_reuse_and_required() {
        assert_eq!(
            classify_artifact_flow(false, None, None, None),
            ArtifactFlow::NoArtifact
        );
        assert_eq!(
            classify_artifact_flow(true, None, Some(DIGEST_A), Some(DIGEST_A)),
            ArtifactFlow::ExactExistingReuse
        );
        assert_eq!(
            classify_artifact_flow(true, None, Some(DIGEST_A), None),
            ArtifactFlow::SelectedRequired
        );
        assert!(prepare_publish(
            request(),
            PublishAdpListingRequestExtension::default(),
            None,
            true,
            None,
        )
        .is_err());

        let mut no_artifact = request();
        no_artifact.file_path = Some("/forged/archive.zip".to_string());
        no_artifact.existing_file_hash = Some(DIGEST_A.to_string());
        let prepared = prepare_publish(
            no_artifact,
            PublishAdpListingRequestExtension::default(),
            Some(build(SELECTION_A)),
            false,
            None,
        )
        .expect("metadata-only publication");
        assert_eq!(prepared.request.file_path, None);
        assert_eq!(prepared.request.selected_build, None);
        assert_eq!(prepared.request.existing_file_hash, None);

        let mut reused = request();
        reused.existing_file_hash = Some(DIGEST_A.to_string());
        let prepared = prepare_publish(
            reused,
            PublishAdpListingRequestExtension::default(),
            None,
            true,
            Some(DIGEST_A),
        )
        .expect("exact existing artifact reuse");
        assert_eq!(prepared.request.file_path, None);
        assert_eq!(prepared.request.selected_build, None);
        assert_eq!(
            prepared.request.existing_file_hash.as_deref(),
            Some(DIGEST_A)
        );

        let mut detached = request();
        detached.existing_file_hash = Some("22".repeat(32));
        assert!(prepare_publish(
            detached,
            PublishAdpListingRequestExtension::default(),
            None,
            true,
            Some(DIGEST_A),
        )
        .is_err());
    }

    #[test]
    fn dirty_snapshot_excludes_transient_archive_state() {
        let mut archive = ArchiveSelectionState::new("owner".to_string());
        archive.selection = Some(selection(SELECTION_A, 1));
        archive.selected_build = Some(build(SELECTION_A));
        let tier = valid_tier(SELECTION_A);
        let before = dirty_snapshot(&archive, &tier, Vec::new());
        archive.busy = true;
        archive.error = Some(ArchiveSelectionError::Transport);
        archive.operation_generation = 99;
        assert_eq!(before, dirty_snapshot(&archive, &tier, Vec::new()));
    }

    #[test]
    fn local_expiry_hint_never_drives_a_transition() {
        let mut archive = ArchiveSelectionState::new("owner".to_string());
        archive.selection = Some(selection(SELECTION_A, 0));
        archive.summary = Some(summary(SELECTION_A));
        archive.selected_build = Some(build(SELECTION_A));
        assert!(archive.build_ready());
        assert!(archive.error.is_none());
    }
}
