//! Feature-scoped A3.5 launch-tier editor.

use gloo_timers::future::TimeoutFuture;
use leptos::prelude::*;
use wasm_bindgen_futures::spawn_local;

use arcadestr_core::adp_publish_contract::{
    AdpLaunchTierIntent, AuthoritativeAdpEditDefect, AuthoritativeProfileRejection,
};
use arcadestr_core::archive_inspection_contract::{
    ArchiveCandidate, ArchiveCandidateConfidence, ArchiveCandidateReason, ArchiveEntryKind,
    ArchiveHazardCategory, ArchiveHazardDetail, ArchivePage, ArchivePageRequest,
    ArchivePathRuleCode, ArchiveSearch, ArchiveSelectionId, ArchiveWarningCategory,
    ArchiveWarningDetail, QueryArchiveCandidatesRequest, QueryArchiveFilesRequest,
    QueryArchiveHazardsRequest, QueryArchiveWarningsRequest, QueryArchiveWorkingDirectoriesRequest,
    SelectableArchiveFile, SelectableWorkingDirectory,
};
use arcadestr_core::distribution::{
    parse_platform_tag, LaunchType, NoLaunchTierReason, PackageFormat, WorkingDir,
};

use crate::publish_state::{
    ArchiveSelectionState, AuthoritativeEditModel, AuthoritativeEditState, DefectReviewState,
    DraftRowId, LaunchMode, ProfileDraftError, PublishState,
};
use crate::tauri_bridge::{
    invoke_query_archive_candidates, invoke_query_archive_files, invoke_query_archive_hazards,
    invoke_query_archive_warnings, invoke_query_archive_working_directories,
};

use super::publish::{archive_error_message, archive_request_id};

const PAGE_SIZE: u16 = 100;
const DETAIL_PAGE_SIZE: u16 = 200;
const ARGUMENT_MAX: usize = 32;
const LARGE_BROWSE_TOTAL: u64 = 1_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PickerKind {
    Executable,
    WorkingDirectory,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct OpenPicker {
    row_id: DraftRowId,
    kind: PickerKind,
    browse_all: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct HazardPage {
    label: &'static str,
    page: ArchivePage<ArchiveHazardDetail>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct WarningPage {
    label: &'static str,
    page: ArchivePage<ArchiveWarningDetail>,
}

fn package_format_label(format: &PackageFormat) -> &'static str {
    match format {
        PackageFormat::Zip => "ZIP",
        PackageFormat::TarZstd => "tar + Zstandard",
        PackageFormat::TarGz => "tar + gzip",
    }
}

fn candidate_confidence(value: ArchiveCandidateConfidence) -> &'static str {
    match value {
        ArchiveCandidateConfidence::High => "High confidence",
        ArchiveCandidateConfidence::Medium => "Medium confidence",
        ArchiveCandidateConfidence::Low => "Low confidence",
    }
}

fn candidate_reason(value: ArchiveCandidateReason) -> &'static str {
    match value {
        ArchiveCandidateReason::GodotPackageBesideBinary => "Godot package is beside this binary",
        ArchiveCandidateReason::UnityDataDirectoryBesideBinary => {
            "Unity data directory is beside this binary"
        }
        ArchiveCandidateReason::UnrealWin64Layout => "Matches an Unreal Win64 build layout",
        ArchiveCandidateReason::WindowsExecutableExtension { archive_root: true } => {
            "Windows executable at archive root"
        }
        ArchiveCandidateReason::WindowsExecutableExtension {
            archive_root: false,
        } => "Windows .exe file",
        ArchiveCandidateReason::UnixExecutableMode { archive_root: true } => {
            "Executable Unix file at archive root"
        }
        ArchiveCandidateReason::UnixExecutableMode {
            archive_root: false,
        } => "Archive marks this Unix file executable",
    }
}

fn launch_type_value(value: &LaunchType) -> &'static str {
    match value {
        LaunchType::Game => "game",
        LaunchType::Config => "config",
        LaunchType::Server => "server",
        LaunchType::Editor => "editor",
        LaunchType::Tool => "tool",
    }
}

fn parse_launch_type(value: &str) -> LaunchType {
    match value {
        "config" => LaunchType::Config,
        "server" => LaunchType::Server,
        "editor" => LaunchType::Editor,
        "tool" => LaunchType::Tool,
        _ => LaunchType::Game,
    }
}

fn source_property(kind: PickerKind) -> &'static str {
    match kind {
        PickerKind::Executable => "validated regular files from the inspected archive index",
        PickerKind::WorkingDirectory => {
            "validated directories from the inspected archive index, including root (.)"
        }
    }
}

fn format_indices(indices: &[usize]) -> String {
    match indices {
        [] => "listing-level (no individual tag)".to_string(),
        [index] => format!("tag index {index}"),
        _ => format!(
            "tag indices {}",
            indices
                .iter()
                .map(usize::to_string)
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

fn defect_copy(defect: &AuthoritativeAdpEditDefect) -> (String, &'static str) {
    match defect {
        AuthoritativeAdpEditDefect::PackageCount {
            actual,
            event_tag_indices,
        } => (
            format!(
                "Package count is {actual}; exactly one is required ({})",
                format_indices(event_tag_indices)
            ),
            "publish-launch-package",
        ),
        AuthoritativeAdpEditDefect::MalformedPackage { event_tag_index } => (
            format!(
                "Malformed package at {}",
                format_indices(&[*event_tag_index])
            ),
            "publish-launch-package",
        ),
        AuthoritativeAdpEditDefect::UnsupportedPackage {
            event_tag_index,
            raw_format,
        } => (
            format!(
                "Unsupported package {raw_format:?} at {}",
                format_indices(&[*event_tag_index])
            ),
            "publish-launch-package",
        ),
        AuthoritativeAdpEditDefect::MissingPlatform => (
            "No platform tag; listing-level defect".to_string(),
            "publish-launch-platforms",
        ),
        AuthoritativeAdpEditDefect::MalformedPlatform { event_tag_index } => (
            format!(
                "Malformed platform at {}",
                format_indices(&[*event_tag_index])
            ),
            "publish-launch-platforms",
        ),
        AuthoritativeAdpEditDefect::InvalidPlatform {
            event_tag_index,
            raw_platform,
        } => (
            format!(
                "Invalid platform {:?} at {}",
                raw_platform,
                format_indices(&[*event_tag_index])
            ),
            "publish-launch-platforms",
        ),
        AuthoritativeAdpEditDefect::NonCanonicalPlatform {
            event_tag_index,
            raw_platform,
            canonical,
        } => (
            format!(
                "Noncanonical platform {raw_platform:?}; use {canonical:?} ({})",
                format_indices(&[*event_tag_index])
            ),
            "publish-launch-platforms",
        ),
        AuthoritativeAdpEditDefect::MalformedLaunch {
            event_tag_index,
            rejection,
        } => (
            format!(
                "Malformed launch profile ({}) at {}",
                profile_rejection_copy(rejection),
                format_indices(&[*event_tag_index])
            ),
            "publish-launch-profiles",
        ),
        AuthoritativeAdpEditDefect::DuplicateProfileId {
            profile_id,
            event_tag_indices,
        } => (
            format!(
                "Duplicate profile ID {:?} at {}",
                profile_id.as_str(),
                format_indices(event_tag_indices)
            ),
            "publish-launch-profiles",
        ),
        AuthoritativeAdpEditDefect::UnknownLaunchType {
            event_tag_index,
            raw_launch_type,
        } => (
            format!(
                "Unknown launch type {raw_launch_type:?} at {}; it was interpreted as tool",
                format_indices(&[*event_tag_index])
            ),
            "publish-launch-profiles",
        ),
        AuthoritativeAdpEditDefect::NoValidGameProfile => (
            "No valid game profile; listing-level defect".to_string(),
            "publish-launch-profiles",
        ),
        AuthoritativeAdpEditDefect::MalformedLaunchDefault {
            event_tag_index,
            raw_profile_id,
        } => (
            format!(
                "Malformed Automatic Play profile reference {:?} at {}",
                raw_profile_id,
                format_indices(&[*event_tag_index])
            ),
            "publish-launch-profiles",
        ),
        AuthoritativeAdpEditDefect::DuplicateLaunchDefaultProfile {
            profile_id,
            event_tag_indices,
        } => (
            format!(
                "Automatic Play profile {:?} is declared more than once at {}",
                profile_id.as_str(),
                format_indices(event_tag_indices)
            ),
            "publish-launch-profiles",
        ),
        AuthoritativeAdpEditDefect::InvalidLaunchDefaultTarget {
            event_tag_index,
            raw_target,
        } => (
            format!(
                "Invalid Automatic Play target {raw_target:?} at {}",
                format_indices(&[*event_tag_index])
            ),
            "publish-launch-profiles",
        ),
        AuthoritativeAdpEditDefect::NonCanonicalLaunchDefaultTarget {
            event_tag_index,
            raw_target,
            canonical,
        } => (
            format!(
                "Noncanonical Automatic Play target {raw_target:?}; use {canonical:?} ({})",
                format_indices(&[*event_tag_index])
            ),
            "publish-launch-profiles",
        ),
        AuthoritativeAdpEditDefect::UndeclaredLaunchDefaultTarget {
            event_tag_index,
            target,
        } => (
            format!(
                "Automatic Play target {}-{} is not declared by the listing ({})",
                target.os,
                target.arch,
                format_indices(&[*event_tag_index])
            ),
            "publish-launch-profiles",
        ),
        AuthoritativeAdpEditDefect::OverlappingLaunchDefaultTarget {
            target,
            event_tag_indices,
        } => (
            format!(
                "Automatic Play target {}-{} is claimed by multiple profiles at {}",
                target.os,
                target.arch,
                format_indices(event_tag_indices)
            ),
            "publish-launch-profiles",
        ),
        AuthoritativeAdpEditDefect::UncoveredLaunchDefaultTarget { target } => (
            format!(
                "Automatic Play target {}-{} has no profile; listing-level defect",
                target.os, target.arch
            ),
            "publish-launch-profiles",
        ),
        AuthoritativeAdpEditDefect::MixedLaunchDefaultScopes { event_tag_indices } => (
            format!(
                "Legacy global and target-scoped Automatic Play claims are mixed at {}",
                format_indices(event_tag_indices)
            ),
            "publish-launch-profiles",
        ),
        AuthoritativeAdpEditDefect::DanglingLaunchDefault {
            event_tag_index,
            profile_id,
        } => (
            format!(
                "Automatic Play profile {:?} does not exist ({})",
                profile_id.as_str(),
                format_indices(&[*event_tag_index])
            ),
            "publish-launch-profiles",
        ),
        AuthoritativeAdpEditDefect::NonGameLaunchDefault {
            event_tag_index,
            profile_id,
        } => (
            format!(
                "Automatic Play profile {:?} is not a game ({})",
                profile_id.as_str(),
                format_indices(&[*event_tag_index])
            ),
            "publish-launch-profiles",
        ),
    }
}

fn profile_rejection_copy(rejection: &AuthoritativeProfileRejection) -> String {
    match rejection {
        AuthoritativeProfileRejection::MissingRequiredElement => "required element missing".into(),
        AuthoritativeProfileRejection::ProfileId(reason) => format!("profile ID: {reason:?}"),
        AuthoritativeProfileRejection::Executable(reason) => format!("executable: {reason:?}"),
        AuthoritativeProfileRejection::WorkingDir(reason) => format!("working dir: {reason:?}"),
        AuthoritativeProfileRejection::Arguments(reason) => format!("arguments: {reason:?}"),
        AuthoritativeProfileRejection::DisplayName(reason) => format!("display name: {reason:?}"),
    }
}

fn profile_error_copy(error: ProfileDraftError) -> &'static str {
    match error {
        ProfileDraftError::InvalidId => "ID must match [a-z0-9][a-z0-9_-]{0,63}.",
        ProfileDraftError::DuplicateId => "Profile ID must be unique.",
        ProfileDraftError::InvalidDisplayName => "Display name must be 1–64 safe characters.",
        ProfileDraftError::InvalidArguments => {
            "Arguments allow at most 32 rows, 1024 characters each, and no NUL."
        }
        ProfileDraftError::ExecutableNotConfirmed => {
            "Select this executable from the current archive index."
        }
        ProfileDraftError::WorkingDirectoryNotConfirmed => {
            "Select this working directory from the current archive index."
        }
    }
}

fn hazard_copy(detail: &ArchiveHazardDetail) -> String {
    match detail {
        ArchiveHazardDetail::PathRule { display_path, code } => {
            format!("{display_path}: {}", path_rule_copy(*code))
        }
        ArchiveHazardDetail::Symlink { display_path } => {
            format!("{display_path}: symbolic links are forbidden")
        }
        ArchiveHazardDetail::Hardlink { display_path } => {
            format!("{display_path}: hard links are forbidden")
        }
        ArchiveHazardDetail::Collision { display_paths } => format!(
            "Names collide case-insensitively: {}",
            display_paths.join(", ")
        ),
        ArchiveHazardDetail::Other { display_path } => {
            format!("{display_path}: entry is not a regular file or directory")
        }
        ArchiveHazardDetail::Undecodable { lossy, byte_length } => format!(
            "Undecodable entry {lossy:?} ({byte_length} filename bytes); rename it in the build"
        ),
        ArchiveHazardDetail::TypeConflict {
            display_path,
            container_kind,
        } => format!(
            "{display_path}: conflicting archive entry type {}",
            entry_kind_copy(*container_kind)
        ),
        ArchiveHazardDetail::ZipCountMismatch { declared, parsed } => {
            format!("ZIP entry count mismatch: header declares {declared}, parser found {parsed}")
        }
    }
}

fn path_rule_copy(code: ArchivePathRuleCode) -> &'static str {
    match code {
        ArchivePathRuleCode::Rule1Absolute => "rule 1: absolute path",
        ArchivePathRuleCode::Rule2ParentSegment => "rule 2: parent (..) segment",
        ArchivePathRuleCode::Rule3Backslash => "rule 3: backslash",
        ArchivePathRuleCode::Rule4ColonOrPrefix => "rule 4: colon, drive, or device prefix",
        ArchivePathRuleCode::Rule5ControlCharacter => "rule 5: control character",
        ArchivePathRuleCode::Rule6EmptyOrDotSegment => "rule 6: empty or dot segment",
        ArchivePathRuleCode::Rule7TrailingSpaceOrPeriod => "rule 7: trailing space or period",
        ArchivePathRuleCode::Rule8ReservedDeviceName => "rule 8: reserved device name",
        ArchivePathRuleCode::Rule9LengthExceeded => "rule 9: path or segment is too long",
    }
}

fn entry_kind_copy(kind: ArchiveEntryKind) -> &'static str {
    match kind {
        ArchiveEntryKind::File => "file",
        ArchiveEntryKind::Directory => "directory",
        ArchiveEntryKind::Symlink => "symlink",
        ArchiveEntryKind::Hardlink => "hardlink",
        ArchiveEntryKind::Other => "other",
    }
}

fn warning_copy(detail: &ArchiveWarningDetail) -> String {
    match detail {
        ArchiveWarningDetail::EfsDivergence(detail) => format!(
            "Filename encoding differs: UTF-8 {:?}, CP437 {:?} ({} bytes). Re-export with one unambiguous name.",
            detail.utf8, detail.cp437, detail.byte_length
        ),
        ArchiveWarningDetail::ExtensionMismatch(detail) => format!(
            "Filename extension suggests {}, but inspection detected {}. The detected format is authoritative.",
            detail
                .extension_format
                .as_ref()
                .map(package_format_label)
                .unwrap_or("an unknown format"),
            package_format_label(&detail.detected_format)
        ),
    }
}

fn hazard_categories(state: &PublishState) -> Vec<(ArchiveHazardCategory, &'static str, u64)> {
    let Some(summary) = state.archive.summary.as_ref() else {
        return Vec::new();
    };
    let counts = &summary.counts.hazards;
    [
        (
            ArchiveHazardCategory::PathRule,
            "Path-rule violations",
            counts.path_rule,
        ),
        (
            ArchiveHazardCategory::Symlink,
            "Symbolic links",
            counts.symlink,
        ),
        (
            ArchiveHazardCategory::Hardlink,
            "Hard links",
            counts.hardlink,
        ),
        (
            ArchiveHazardCategory::Collision,
            "Colliding names",
            counts.collision,
        ),
        (
            ArchiveHazardCategory::Other,
            "Unsupported entry types",
            counts.other,
        ),
        (
            ArchiveHazardCategory::Undecodable,
            "Undecodable names",
            counts.undecodable,
        ),
        (
            ArchiveHazardCategory::TypeConflict,
            "Type conflicts",
            counts.type_conflict,
        ),
        (
            ArchiveHazardCategory::ZipCountMismatch,
            "ZIP count mismatch",
            counts.zip_count_mismatch,
        ),
    ]
    .into_iter()
    .filter(|(_, _, count)| *count > 0)
    .collect()
}

fn warning_categories(state: &PublishState) -> Vec<(ArchiveWarningCategory, &'static str, u64)> {
    let Some(summary) = state.archive.summary.as_ref() else {
        return Vec::new();
    };
    let counts = &summary.counts.warnings;
    [
        (
            ArchiveWarningCategory::EfsDivergence,
            "Filename encoding divergence",
            counts.efs_divergence,
        ),
        (
            ArchiveWarningCategory::ExtensionMismatch,
            "Extension mismatch",
            counts.extension_mismatch,
        ),
    ]
    .into_iter()
    .filter(|(_, _, count)| *count > 0)
    .collect()
}

fn selected_platforms(state: &PublishState, platforms_input: &str) -> Vec<String> {
    match &state.launch {
        LaunchMode::Launchable(tier) => tier.platforms.clone(),
        LaunchMode::DownloadOnly => platforms_input
            .split(',')
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .collect(),
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum ReviewArtifact {
    Selected { filename: String, digest: String },
    Existing { digest: String },
    None,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct LaunchReviewProjection {
    launch_tier: Option<AdpLaunchTierIntent>,
    artifact: ReviewArtifact,
    warnings: Vec<String>,
}

fn validated_review_projection(
    launch: &LaunchMode,
    archive: &ArchiveSelectionState,
    artifact_enabled: bool,
    existing_artifact_digest: Option<String>,
    warnings: Vec<String>,
) -> Result<LaunchReviewProjection, String> {
    let selection_id = archive
        .selected_build
        .as_ref()
        .map(|build| &build.selection_id);
    let launch_tier = launch.to_wire_result(selection_id).map_err(|error| {
        format!("Launch review is blocked by invalid launch metadata: {error:?}")
    })?;
    let artifact = artifact_enabled
        .then(|| {
            archive
                .selection
                .as_ref()
                .zip(archive.selected_build.as_ref())
                .filter(|(selection, build)| selection.selection_id == build.selection_id)
                .map(|(selection, build)| ReviewArtifact::Selected {
                    filename: selection.safe_filename.as_str().to_string(),
                    digest: build.sha256.as_str().to_string(),
                })
                .or_else(|| {
                    existing_artifact_digest.map(|digest| ReviewArtifact::Existing { digest })
                })
        })
        .flatten()
        .unwrap_or(ReviewArtifact::None);
    Ok(LaunchReviewProjection {
        launch_tier,
        artifact,
        warnings,
    })
}

#[component]
pub(super) fn PublishLaunchEditor(
    state: RwSignal<PublishState>,
    authoritative: RwSignal<Option<AuthoritativeEditModel>>,
    defect_review: RwSignal<DefectReviewState>,
    platforms_input: RwSignal<String>,
    distribution_revision: RwSignal<u64>,
    platform_revision: RwSignal<u64>,
    surfaced_warnings: RwSignal<Vec<String>>,
    disabled: Signal<bool>,
) -> impl IntoView {
    let os_choice = RwSignal::new("windows".to_string());
    let arch_choice = RwSignal::new("x86_64".to_string());
    let picker = RwSignal::new(None::<OpenPicker>);
    let picker_search = RwSignal::new(String::new());
    let candidate_target = RwSignal::new(String::new());
    let picker_offset = RwSignal::new(0_u64);
    let picker_generation = RwSignal::new(0_u64);
    let picker_selection_id = RwSignal::new(None::<ArchiveSelectionId>);
    let picker_loading = RwSignal::new(false);
    let picker_error = RwSignal::new(None::<String>);
    let file_page = RwSignal::new(None::<ArchivePage<SelectableArchiveFile>>);
    let directory_page = RwSignal::new(None::<ArchivePage<SelectableWorkingDirectory>>);
    let candidate_page = RwSignal::new(None::<ArchivePage<ArchiveCandidate>>);
    let hazard_pages = RwSignal::new(Vec::<HazardPage>::new());
    let warning_pages = RwSignal::new(Vec::<WarningPage>::new());
    let hazard_loading = RwSignal::new(false);
    let hazard_error = RwSignal::new(None::<String>);
    let hazard_generation = RwSignal::new(0_u64);
    let archive_selection_id = Memo::new(move |_| {
        state.with(|state| {
            state
                .archive
                .summary
                .as_ref()
                .map(|summary| summary.selection.selection_id.clone())
        })
    });

    let close_picker = Callback::new(move |()| {
        picker_generation.update(|generation| *generation = generation.wrapping_add(1));
        picker_selection_id.set(None);
        picker_loading.set(false);
        file_page.set(None);
        directory_page.set(None);
        candidate_page.set(None);
        picker.set(None);
    });

    Effect::new(move |_| {
        let current = archive_selection_id.get();
        if picker_selection_id
            .get_untracked()
            .is_some_and(|selection_id| Some(selection_id) != current)
        {
            close_picker.run(());
        }
    });

    Effect::new(move |_| {
        let selected = archive_selection_id.get();
        hazard_generation.update(|generation| *generation = generation.wrapping_add(1));
        let generation = hazard_generation.get_untracked();
        hazard_pages.set(Vec::new());
        warning_pages.set(Vec::new());
        surfaced_warnings.set(Vec::new());
        hazard_error.set(None);
        hazard_loading.set(false);
        let Some(selection_id) = selected else {
            return;
        };
        let snapshot = state.get_untracked();
        let owner = snapshot.archive.owner_npub.clone();
        let hazards = hazard_categories(&snapshot);
        let warnings = warning_categories(&snapshot);
        if hazards.is_empty() && warnings.is_empty() {
            return;
        }
        hazard_loading.set(true);
        spawn_local(async move {
            let page = match ArchivePageRequest::new(0, DETAIL_PAGE_SIZE) {
                Ok(page) => page,
                Err(error) => {
                    if hazard_generation.get_untracked() == generation {
                        hazard_error.set(Some(error.to_string()));
                        hazard_loading.set(false);
                    }
                    return;
                }
            };
            for (category, label, _) in hazards {
                let request_id = match archive_request_id() {
                    Ok(value) => value,
                    Err(error) => {
                        if hazard_generation.get_untracked() == generation {
                            hazard_error.set(Some(archive_error_message(&error)));
                        }
                        continue;
                    }
                };
                match invoke_query_archive_hazards(QueryArchiveHazardsRequest {
                    request_id,
                    selection_id: selection_id.clone(),
                    expected_publisher_npub: owner.clone(),
                    category,
                    page,
                })
                .await
                {
                    Ok(response)
                        if state
                            .get_untracked()
                            .archive
                            .selection
                            .as_ref()
                            .is_some_and(|selection| selection.selection_id == selection_id) =>
                    {
                        hazard_pages.update(|pages| {
                            pages.push(HazardPage {
                                label,
                                page: response.page,
                            })
                        });
                    }
                    Ok(_) => return,
                    Err(error) if hazard_generation.get_untracked() == generation => {
                        hazard_error.set(Some(archive_error_message(&error)))
                    }
                    Err(_) => return,
                }
            }
            for (category, label, _) in warnings {
                let request_id = match archive_request_id() {
                    Ok(value) => value,
                    Err(error) => {
                        if hazard_generation.get_untracked() == generation {
                            hazard_error.set(Some(archive_error_message(&error)));
                        }
                        continue;
                    }
                };
                match invoke_query_archive_warnings(QueryArchiveWarningsRequest {
                    request_id,
                    selection_id: selection_id.clone(),
                    expected_publisher_npub: owner.clone(),
                    category,
                    page,
                })
                .await
                {
                    Ok(response)
                        if state
                            .get_untracked()
                            .archive
                            .selection
                            .as_ref()
                            .is_some_and(|selection| selection.selection_id == selection_id) =>
                    {
                        let surfaced = response
                            .page
                            .items
                            .iter()
                            .map(warning_copy)
                            .collect::<Vec<_>>();
                        warning_pages.update(|pages| {
                            pages.push(WarningPage {
                                label,
                                page: response.page,
                            })
                        });
                        surfaced_warnings.update(|warnings| warnings.extend(surfaced));
                    }
                    Ok(_) => return,
                    Err(error) if hazard_generation.get_untracked() == generation => {
                        hazard_error.set(Some(archive_error_message(&error)))
                    }
                    Err(_) => return,
                }
            }
            if hazard_generation.get_untracked() == generation {
                hazard_loading.set(false);
            }
        });
    });

    Effect::new(move |_| {
        let open = picker.get();
        let search_text = picker_search.get();
        let candidate_target_value = candidate_target.get();
        let offset = picker_offset.get();
        let Some(open) = open else {
            return;
        };
        picker_generation.update(|generation| *generation = generation.wrapping_add(1));
        let generation = picker_generation.get_untracked();
        picker_loading.set(true);
        picker_error.set(None);
        file_page.set(None);
        directory_page.set(None);
        candidate_page.set(None);
        spawn_local(async move {
            TimeoutFuture::new(250).await;
            if picker_generation.get_untracked() != generation {
                return;
            }
            let search = match ArchiveSearch::parse(&search_text) {
                Ok(search) => search,
                Err(error) => {
                    picker_error.set(Some(error.to_string()));
                    picker_loading.set(false);
                    return;
                }
            };
            let page = match ArchivePageRequest::new(offset, PAGE_SIZE) {
                Ok(page) => page,
                Err(error) => {
                    picker_error.set(Some(error.to_string()));
                    picker_loading.set(false);
                    return;
                }
            };
            let snapshot = state.get_untracked();
            let Some(ref selection) = snapshot.archive.selection else {
                picker_error.set(Some("Select and inspect an archive first.".into()));
                picker_loading.set(false);
                return;
            };
            let queried_selection_id = selection.selection_id.clone();
            let owner = snapshot.archive.owner_npub.clone();
            let result = match open.kind {
                PickerKind::WorkingDirectory => {
                    let request_id = match archive_request_id() {
                        Ok(value) => value,
                        Err(error) => {
                            picker_error.set(Some(archive_error_message(&error)));
                            picker_loading.set(false);
                            return;
                        }
                    };
                    invoke_query_archive_working_directories(
                        QueryArchiveWorkingDirectoriesRequest {
                            request_id,
                            selection_id: selection.selection_id.clone(),
                            expected_publisher_npub: owner,
                            search,
                            page,
                        },
                    )
                    .await
                    .map(|response| {
                        if picker_generation.get_untracked() == generation
                            && picker_selection_id.get_untracked().as_ref()
                                == Some(&queried_selection_id)
                            && state
                                .get_untracked()
                                .archive
                                .selection
                                .as_ref()
                                .is_some_and(|selection| {
                                    selection.selection_id == queried_selection_id
                                })
                        {
                            directory_page.set(Some(response.page));
                        }
                    })
                }
                PickerKind::Executable if open.browse_all => {
                    let request_id = match archive_request_id() {
                        Ok(value) => value,
                        Err(error) => {
                            picker_error.set(Some(archive_error_message(&error)));
                            picker_loading.set(false);
                            return;
                        }
                    };
                    invoke_query_archive_files(QueryArchiveFilesRequest {
                        request_id,
                        selection_id: selection.selection_id.clone(),
                        expected_publisher_npub: owner,
                        search,
                        page,
                    })
                    .await
                    .map(|response| {
                        if picker_generation.get_untracked() == generation
                            && picker_selection_id.get_untracked().as_ref()
                                == Some(&queried_selection_id)
                            && state
                                .get_untracked()
                                .archive
                                .selection
                                .as_ref()
                                .is_some_and(|selection| {
                                    selection.selection_id == queried_selection_id
                                })
                        {
                            file_page.set(Some(response.page));
                        }
                    })
                }
                PickerKind::Executable => {
                    let target = snapshot
                        .launch_platforms()
                        .into_iter()
                        .find(|target| target == &candidate_target_value)
                        .or_else(|| snapshot.launch_platforms().first().cloned());
                    let Some(platform) = target.and_then(|raw| parse_platform_tag(&raw)) else {
                        picker_error.set(Some(
                            "Declare a canonical target before requesting executable candidates."
                                .into(),
                        ));
                        picker_loading.set(false);
                        return;
                    };
                    let request_id = match archive_request_id() {
                        Ok(value) => value,
                        Err(error) => {
                            picker_error.set(Some(archive_error_message(&error)));
                            picker_loading.set(false);
                            return;
                        }
                    };
                    invoke_query_archive_candidates(QueryArchiveCandidatesRequest {
                        request_id,
                        selection_id: selection.selection_id.clone(),
                        expected_publisher_npub: owner,
                        search,
                        platform,
                        page,
                    })
                    .await
                    .map(|response| {
                        if picker_generation.get_untracked() == generation
                            && picker_selection_id.get_untracked().as_ref()
                                == Some(&queried_selection_id)
                            && state
                                .get_untracked()
                                .archive
                                .selection
                                .as_ref()
                                .is_some_and(|selection| {
                                    selection.selection_id == queried_selection_id
                                })
                        {
                            candidate_page.set(Some(response.page));
                        }
                    })
                }
            };
            if picker_generation.get_untracked() == generation {
                if let Err(error) = result {
                    picker_error.set(Some(archive_error_message(&error)));
                }
                picker_loading.set(false);
            }
        });
    });

    let choose_file = Callback::new(move |item: SelectableArchiveFile| {
        let Some(open) = picker.get_untracked() else {
            return;
        };
        let Some(selection_id) = picker_selection_id.get_untracked() else {
            return;
        };
        if state
            .get_untracked()
            .archive
            .selection
            .as_ref()
            .is_none_or(|selection| selection.selection_id != selection_id)
        {
            close_picker.run(());
            return;
        }
        state.update(|state| {
            if let LaunchMode::Launchable(tier) = &mut state.launch {
                if let Some(profile) = tier
                    .profiles
                    .iter_mut()
                    .find(|profile| profile.row_id == open.row_id)
                {
                    profile.executable.confirm(
                        selection_id,
                        item.path.clone(),
                        item.path.as_str().to_string(),
                    );
                }
            }
        });
        distribution_revision.update(|revision| *revision = revision.wrapping_add(1));
        close_picker.run(());
    });

    let choose_candidate = Callback::new(move |item: ArchiveCandidate| {
        choose_file.run(SelectableArchiveFile {
            path: item.path,
            declared_size: 0,
        });
    });

    let choose_directory = Callback::new(move |item: SelectableWorkingDirectory| {
        let Some(open) = picker.get_untracked() else {
            return;
        };
        let Some(selection_id) = picker_selection_id.get_untracked() else {
            return;
        };
        if state
            .get_untracked()
            .archive
            .selection
            .as_ref()
            .is_none_or(|selection| selection.selection_id != selection_id)
        {
            close_picker.run(());
            return;
        }
        state.update(|state| {
            if let LaunchMode::Launchable(tier) = &mut state.launch {
                if let Some(profile) = tier
                    .profiles
                    .iter_mut()
                    .find(|profile| profile.row_id == open.row_id)
                {
                    profile.working_dir.confirm(
                        selection_id,
                        item.directory.clone(),
                        item.directory.as_str().to_string(),
                    );
                }
            }
        });
        distribution_revision.update(|revision| *revision = revision.wrapping_add(1));
        close_picker.run(());
    });

    let open_picker = Callback::new(move |(row_id, kind): (DraftRowId, PickerKind)| {
        let Some(selection_id) = archive_selection_id.get_untracked() else {
            return;
        };
        picker_selection_id.set(Some(selection_id));
        picker_search.set(String::new());
        let snapshot = state.get_untracked();
        let assigned_target = match &snapshot.launch {
            LaunchMode::Launchable(tier) => tier
                .publisher_default_assignments
                .iter()
                .find(|assignment| assignment.row_id == row_id)
                .map(|assignment| assignment.target.clone()),
            LaunchMode::DownloadOnly => None,
        };
        candidate_target.set(
            assigned_target
                .or_else(|| snapshot.launch_platforms().first().cloned())
                .unwrap_or_default(),
        );
        picker_offset.set(0);
        file_page.set(None);
        directory_page.set(None);
        candidate_page.set(None);
        picker.set(Some(OpenPicker {
            row_id,
            kind,
            browse_all: false,
        }));
    });

    view! {
        <section class="v2-create-launch" aria-labelledby="publish-launch-title">
            <div class="v2-create-row-between">
                <div>
                    <h3 id="publish-launch-title" class="v2-create-subsection-title">"Launch behavior"</h3>
                    <p class="v2-create-help">"Choose one complete publication tier. Partial launch metadata is never published."</p>
                </div>
                <div class="v2-create-launch-mode" role="group" aria-label="Launch behavior">
                    <button type="button" class="v2-btn-secondary" aria-pressed=move || matches!(state.get().launch, LaunchMode::DownloadOnly) disabled=move || disabled.get() || authoritative.get().is_some_and(|model| matches!(model.state, AuthoritativeEditState::Loading)) on:click=move |_| {
                        state.update(|state| state.launch = LaunchMode::DownloadOnly);
                        distribution_revision.update(|revision| *revision = revision.wrapping_add(1));
                        platform_revision.update(|revision| *revision = revision.wrapping_add(1));
                    }>"Download-only"</button>
                    <button type="button" class="v2-btn-secondary" aria-pressed=move || matches!(state.get().launch, LaunchMode::Launchable(_)) disabled=move || disabled.get() || authoritative.get().is_some_and(|model| matches!(model.state, AuthoritativeEditState::Loading)) on:click=move |_| {
                        let platforms = selected_platforms(&state.get_untracked(), &platforms_input.get_untracked());
                        state.update(|state| {
                            if matches!(&state.launch, LaunchMode::DownloadOnly) {
                                let mut tier = crate::publish_state::LaunchTierDraft::default();
                                if let Some(summary) = state.archive.summary.as_ref() {
                                    tier.package_format.text = match &summary.detected_format {
                                        PackageFormat::Zip => "zip",
                                        PackageFormat::TarZstd => "tar.zst",
                                        PackageFormat::TarGz => "tar.gz",
                                    }.to_string();
                                }
                                tier.replace_platforms(platforms);
                                state.launch = LaunchMode::Launchable(tier);
                            }
                        });
                        distribution_revision.update(|revision| *revision = revision.wrapping_add(1));
                    }>"Launchable"</button>
                </div>
            </div>

            {move || authoritative.get().map(|model| view! {
                <AuthoritativeLaunchStatus model=model defect_review=defect_review disabled=disabled />
            })}

            <div id="publish-launch-package" class="v2-create-subpanel">
                <span class="v2-create-field-label">"Detected package format"</span>
                <p class="v2-create-mono-value">{move || state.get().archive.summary.map(|summary| package_format_label(&summary.detected_format).to_string()).or_else(|| {
                    match state.get().launch {
                        LaunchMode::Launchable(tier) if !tier.package_format.text.is_empty() => Some(format!("{} (authoritative listing)", tier.package_format.text)),
                        _ => None,
                    }
                }).unwrap_or_else(|| "Select and inspect an archive".to_string())}</p>
                <p class="v2-create-help">"Archive contents determine replacement formats. Download-only omits the launch package tag; existing authoritative metadata remains visible until replacement."</p>
            </div>

            <ArchiveSafety state=state hazard_pages=hazard_pages warning_pages=warning_pages loading=hazard_loading error=hazard_error />

            <PlatformEditor state=state platforms_input=platforms_input distribution_revision=distribution_revision platform_revision=platform_revision os_choice=os_choice arch_choice=arch_choice disabled=disabled />

            <Show when=move || matches!(state.get().launch, LaunchMode::DownloadOnly)>
                <p class="v2-create-inline-note">"Download-only: buyers can acquire and verify the archive, but no Play action is offered."</p>
            </Show>

            <Show when=move || matches!(state.get().launch, LaunchMode::Launchable(_))>
                <div class="v2-create-fields">
                    <div id="publish-launch-profiles" class="v2-create-fields">
                        <div class="v2-create-row-between">
                            <div><h4 class="v2-create-subsection-title">"Launch profiles"</h4><p class="v2-create-help">"At least one canonical game profile is required."</p></div>
                            <button type="button" class="v2-btn-secondary" disabled=move || disabled.get() on:click=move |_| {
                                state.update(|state| if let LaunchMode::Launchable(tier) = &mut state.launch { tier.add_profile(); });
                                distribution_revision.update(|revision| *revision = revision.wrapping_add(1));
                            }>"Add profile"</button>
                        </div>
                        <Show when=move || matches!(state.get().launch, LaunchMode::Launchable(tier) if !tier.profiles.is_empty() && tier.platforms.iter().any(|target| !tier.publisher_default_assignments.iter().any(|assignment| &assignment.target == target)))>
                            <p class="v2-create-inline-error" role="alert">"Assign every canonical target to an Automatic Play Game profile before publishing."</p>
                        </Show>
                        {move || {
                            let (profiles, validations) = match state.get().launch {
                                LaunchMode::Launchable(tier) => {
                                    let validations = tier.live_profile_validation();
                                    (tier.profiles, validations)
                                },
                                LaunchMode::DownloadOnly => (Vec::new(), Vec::new()),
                            };
                            if profiles.is_empty() {
                                view! { <p class="v2-create-empty">"No profiles yet. Add a Game profile, then assign its Automatic Play targets."</p> }.into_any()
                            } else {
                                let count = profiles.len();
                                profiles.into_iter().enumerate().map(|(index, profile)| {
                                        let row_id = profile.row_id;
                                        let is_game = profile.launch_type == LaunchType::Game;
                                        let errors = validations.iter().find(|entry| entry.row_id == row_id).map(|entry| entry.errors.clone()).unwrap_or_default();
                                    view! {
                                        <article class="v2-create-profile-card">
                                            <div class="v2-create-row-between">
                                                <h5 class="v2-create-field-label">{format!("Profile {}", index + 1)}</h5>
                                                <div class="v2-create-inline-row">
                                                    <button type="button" class="v2-create-media-link" disabled=move || disabled.get() || index == 0 on:click=move |_| { state.update(|state| if let LaunchMode::Launchable(tier) = &mut state.launch { tier.reorder_profile(row_id, index.saturating_sub(1)); }); }>"Move up"</button>
                                                    <button type="button" class="v2-create-media-link" disabled=move || disabled.get() || (index + 1 >= count) on:click=move |_| { state.update(|state| if let LaunchMode::Launchable(tier) = &mut state.launch { tier.reorder_profile(row_id, index + 1); }); }>"Move down"</button>
                                                    <button type="button" class="v2-create-media-link v2-create-media-remove" disabled=move || disabled.get() on:click=move |_| { state.update(|state| if let LaunchMode::Launchable(tier) = &mut state.launch { tier.remove_profile(row_id); }); }>"Remove"</button>
                                                </div>
                                            </div>
                                            <div class="v2-create-field-pair">
                                                <label><span class="v2-create-field-label">"Profile ID"</span><input class="v2-input" prop:value=profile.profile_id on:input:target=move |event| state.update(|state| if let LaunchMode::Launchable(tier) = &mut state.launch { if let Some(profile) = tier.profiles.iter_mut().find(|profile| profile.row_id == row_id) { profile.profile_id = event.target().value(); } }) disabled=move || disabled.get() /></label>
                                                <label><span class="v2-create-field-label">"Display name"</span><input class="v2-input" prop:value=profile.display_name on:input:target=move |event| state.update(|state| if let LaunchMode::Launchable(tier) = &mut state.launch { if let Some(profile) = tier.profiles.iter_mut().find(|profile| profile.row_id == row_id) { profile.display_name = event.target().value(); } }) disabled=move || disabled.get() /></label>
                                            </div>
                                            <label><span class="v2-create-field-label">"Type"</span><select class="v2-input" prop:value=launch_type_value(&profile.launch_type) on:change:target=move |event| state.update(|state| if let LaunchMode::Launchable(tier) = &mut state.launch { tier.set_profile_launch_type(row_id, parse_launch_type(&event.target().value())); }) disabled=move || disabled.get()><option value="game">"game"</option><option value="config">"config"</option><option value="server">"server"</option><option value="editor">"editor"</option><option value="tool">"tool"</option></select></label>
                                            {is_game.then(|| view! {
                                                <div class="v2-create-fields">
                                                    <label class="v2-create-checkbox-row">
                                                        <input
                                                            type="checkbox"
                                                            prop:checked=move || match state.get().launch {
                                                                LaunchMode::Launchable(tier) => tier.profile_has_publisher_defaults(row_id),
                                                                LaunchMode::DownloadOnly => false,
                                                            }
                                                            on:change:target=move |event| state.update(|state| if let LaunchMode::Launchable(tier) = &mut state.launch { tier.set_publisher_default_profile(row_id, event.target().checked()); })
                                                            disabled=move || disabled.get()
                                                        />
                                                        "Automatic Play profile"
                                                    </label>
                                                    <Show when=move || matches!(state.get().launch, LaunchMode::Launchable(tier) if tier.profile_has_publisher_defaults(row_id))>
                                                        <fieldset class="v2-create-fields">
                                                            <legend class="v2-create-help">"Targets using this profile for Automatic Play"</legend>
                                                            {move || match state.get().launch {
                                                                LaunchMode::Launchable(tier) => tier.platforms.into_iter().map(|target| {
                                                                    let checked_target = target.clone();
                                                                    let changed_target = target.clone();
                                                                    view! {
                                                                        <label class="v2-create-checkbox-row">
                                                                            <input
                                                                                type="checkbox"
                                                                                prop:checked=move || matches!(state.get().launch, LaunchMode::Launchable(tier) if tier.publisher_default_assignments.iter().any(|assignment| assignment.row_id == row_id && assignment.target == checked_target))
                                                                                on:change:target=move |event| state.update(|state| if let LaunchMode::Launchable(tier) = &mut state.launch { tier.assign_publisher_default(row_id, &changed_target, event.target().checked()); })
                                                                                disabled=move || disabled.get()
                                                                            />
                                                                            {target}
                                                                        </label>
                                                                    }
                                                                }).collect_view(),
                                                                LaunchMode::DownloadOnly => Vec::new().into_iter().collect_view(),
                                                            }}
                                                        </fieldset>
                                                    </Show>
                                                </div>
                                            })}
                                            <div class="v2-create-field-pair">
                                                <div><span class="v2-create-field-label">"Executable"</span><p class="v2-create-mono-value">{if profile.executable.text.is_empty() { "Not selected".to_string() } else { profile.executable.text.clone() }}</p>{profile.executable.stale.then(|| view! { <p class="v2-create-inline-error">"This value belongs to the previous archive and requires reconfirmation. Reselect it against the new index."</p> })}<button type="button" class="v2-btn-secondary" disabled=move || disabled.get() || state.get().archive.summary.is_none() on:click=move |_| open_picker.run((row_id, PickerKind::Executable))>"Choose from archive"</button></div>
                                                <div><span class="v2-create-field-label">"Working directory"</span><p class="v2-create-mono-value">{profile.working_dir.text.clone()}</p>{profile.working_dir.stale.then(|| view! { <p class="v2-create-inline-error">"This value belongs to the previous archive and requires reconfirmation. Reselect it against the new index."</p> })}<button type="button" class="v2-btn-secondary" disabled=move || disabled.get() || state.get().archive.summary.is_none() on:click=move |_| open_picker.run((row_id, PickerKind::WorkingDirectory))>"Choose from archive"</button></div>
                                            </div>
                                            <div><div class="v2-create-row-between"><span class="v2-create-field-label">"Arguments"</span><span class="v2-create-help">{format!("{} / {ARGUMENT_MAX} separate arguments", profile.arguments.len())}</span></div>{profile.arguments.into_iter().enumerate().map(|(argument_index, argument)| view! { <div class="v2-create-inline-row"><input class="v2-input" aria-label=format!("Argument {}", argument_index + 1) prop:value=argument on:input:target=move |event| state.update(|state| if let LaunchMode::Launchable(tier) = &mut state.launch { if let Some(profile) = tier.profiles.iter_mut().find(|profile| profile.row_id == row_id) { if let Some(argument) = profile.arguments.get_mut(argument_index) { *argument = event.target().value(); } } }) disabled=move || disabled.get() /><button type="button" class="v2-create-media-link v2-create-media-remove" on:click=move |_| state.update(|state| if let LaunchMode::Launchable(tier) = &mut state.launch { if let Some(profile) = tier.profiles.iter_mut().find(|profile| profile.row_id == row_id) { if argument_index < profile.arguments.len() { profile.arguments.remove(argument_index); } } }) disabled=move || disabled.get()>"Remove"</button></div> }).collect_view()}<button type="button" class="v2-btn-secondary" disabled=move || disabled.get() || (state.get().launch_profile_argument_count(row_id) >= ARGUMENT_MAX) on:click=move |_| state.update(|state| if let LaunchMode::Launchable(tier) = &mut state.launch { if let Some(profile) = tier.profiles.iter_mut().find(|profile| profile.row_id == row_id) { if profile.arguments.len() < ARGUMENT_MAX { profile.arguments.push(String::new()); } } })>"Add argument"</button><p class="v2-create-help">"Each row is one literal argument. No command string, JSON, shell parsing, expansion, or substitution."</p></div>
                                            {(!errors.is_empty()).then(|| view! { <ul class="v2-create-profile-errors">{errors.into_iter().map(|error| view! { <li>{profile_error_copy(error)}</li> }).collect_view()}</ul> })}
                                        </article>
                                    }
                                }).collect_view().into_any()
                            }
                        }}
                    </div>
                </div>
            </Show>

            <Show when=move || picker.get().is_some()>
                <PathPickerShell picker=picker search=picker_search candidate_target=candidate_target offset=picker_offset loading=picker_loading error=picker_error file_page=file_page directory_page=directory_page candidate_page=candidate_page choose_file=choose_file choose_directory=choose_directory choose_candidate=choose_candidate close=close_picker state=state />
            </Show>
        </section>
    }
}

#[component]
pub(super) fn PublishLaunchReview(
    state: RwSignal<PublishState>,
    artifact_enabled: Signal<bool>,
    existing_artifact_digest: Signal<Option<String>>,
    surfaced_warnings: RwSignal<Vec<String>>,
) -> impl IntoView {
    view! {
        <section class="v2-create-subpanel" aria-labelledby="publish-launch-review-title">
            <h3 id="publish-launch-review-title" class="v2-create-subsection-title">"Exact signed launch review"</h3>
            {move || {
                let snapshot = state.get();
                match validated_review_projection(
                    &snapshot.launch,
                    &snapshot.archive,
                    artifact_enabled.get(),
                    existing_artifact_digest.get(),
                    surfaced_warnings.get(),
                ) {
                Err(error) => view! { <p class="v2-create-alert v2-create-alert-error" role="alert">{error}</p> }.into_any(),
                Ok(review) => {
                    let artifact = match review.artifact {
                        ReviewArtifact::Selected { filename, digest } => format!("Selected archive: {filename}; full SHA-256: {digest}"),
                        ReviewArtifact::Existing { digest } => format!("Existing authoritative artifact reused; full SHA-256: {digest}"),
                        ReviewArtifact::None => "No distribution artifact".to_string(),
                    };
                    match review.launch_tier {
                        None => view! {
                            <div class="v2-create-fields">
                                <p><strong>"Download-only."</strong> " Buyers can acquire the artifact, but no Play action is signed."</p>
                                <p class="v2-create-mono-value">{artifact}</p>
                                <ReviewWarnings warnings=review.warnings />
                            </div>
                        }.into_any(),
                        Some(tier) => {
                            let publisher_defaults = tier.publisher_defaults;
                            let package = match tier.package_format {
                                PackageFormat::Zip => "zip",
                                PackageFormat::TarZstd => "tar.zst",
                                PackageFormat::TarGz => "tar.gz",
                            };
                            let targets = tier
                                .platforms
                                .iter()
                                .map(|target| format!("{}-{}", target.os, target.arch))
                                .collect::<Vec<_>>()
                                .join(", ");
                            let profiles = tier.launch_profiles;
                            view! {
                                <div class="v2-create-fields">
                                    <p><strong>"Launchable."</strong> " The following canonical values are the validated values that will be signed."</p>
                                    <dl class="v2-create-review-facts">
                                        <div><dt>"Package"</dt><dd>{package}</dd></div>
                                        <div><dt>"Targets"</dt><dd>{targets}</dd></div>
                                        <div><dt>"Artifact"</dt><dd>{artifact}</dd></div>
                                        <div><dt>"Automatic Play mapping"</dt><dd>
                                            <ul>{publisher_defaults.into_iter().flat_map(|default| {
                                                let profile_id = default.profile_id.as_str().to_string();
                                                default.targets.into_iter().map(move |target| {
                                                    let profile_id = profile_id.clone();
                                                    view! { <li>{format!("{}-{} → {}", target.os, target.arch, profile_id)}</li> }
                                                })
                                            }).collect_view()}</ul>
                                        </dd></div>
                                    </dl>
                                    <ol>
                                        {profiles.into_iter().map(|profile| {
                                            let profile_id = profile.id.as_str().to_string();
                                            let display_name = profile.display_name;
                                            let launch_type = launch_type_value(&profile.launch_type);
                                            let executable = profile.executable.as_str().to_string();
                                            let working_dir = profile.working_dir.as_str().to_string();
                                            let arguments = profile.arguments;
                                            view! {
                                                <li class="v2-create-profile-card">
                                                    <dl class="v2-create-review-facts">
                                                        <div><dt>"Profile ID"</dt><dd>{profile_id}</dd></div>
                                                        <div><dt>"Display name"</dt><dd>{display_name}</dd></div>
                                                        <div><dt>"Type"</dt><dd>{launch_type}</dd></div>
                                                        <div><dt>"Executable"</dt><dd>{executable}</dd></div>
                                                        <div><dt>"Working directory"</dt><dd>{working_dir}</dd></div>
                                                    </dl>
                                                    <p class="v2-create-field-label">"Arguments, in order"</p>
                                                    {if arguments.is_empty() {
                                                        view! { <p class="v2-create-help">"No arguments"</p> }.into_any()
                                                    } else {
                                                        view! { <ol>{arguments.into_iter().map(|argument| view! { <li class="v2-create-mono-value">{argument}</li> }).collect_view()}</ol> }.into_any()
                                                    }}
                                                </li>
                                            }
                                        }).collect_view()}
                                    </ol>
                                    <ReviewWarnings warnings=review.warnings />
                                </div>
                            }.into_any()
                        }
                    }
                }
            }}}
        </section>
    }
}

#[component]
fn ReviewWarnings(warnings: Vec<String>) -> impl IntoView {
    (!warnings.is_empty()).then(|| view! {
        <div class="v2-create-warning-group">
            <strong>"Archive warnings surfaced during inspection"</strong>
            <ul>{warnings.into_iter().map(|warning| view! { <li>{warning}</li> }).collect_view()}</ul>
        </div>
    })
}

trait LaunchStateUi {
    fn launch_platforms(&self) -> Vec<String>;
    fn launch_profile_argument_count(&self, row_id: DraftRowId) -> usize;
}

impl LaunchStateUi for PublishState {
    fn launch_platforms(&self) -> Vec<String> {
        match &self.launch {
            LaunchMode::Launchable(tier) => tier.platforms.clone(),
            LaunchMode::DownloadOnly => Vec::new(),
        }
    }

    fn launch_profile_argument_count(&self, row_id: DraftRowId) -> usize {
        match &self.launch {
            LaunchMode::Launchable(tier) => tier
                .profiles
                .iter()
                .find(|profile| profile.row_id == row_id)
                .map(|profile| profile.arguments.len())
                .unwrap_or_default(),
            LaunchMode::DownloadOnly => 0,
        }
    }
}

#[component]
fn PlatformEditor(
    state: RwSignal<PublishState>,
    platforms_input: RwSignal<String>,
    distribution_revision: RwSignal<u64>,
    platform_revision: RwSignal<u64>,
    os_choice: RwSignal<String>,
    arch_choice: RwSignal<String>,
    disabled: Signal<bool>,
) -> impl IntoView {
    view! {
        <div id="publish-launch-platforms" class="v2-create-subpanel">
            <span class="v2-create-field-label">"Canonical targets"</span>
            <div class="v2-create-platform-controls">
                <label><span class="v2-create-help">"Operating system"</span><select class="v2-input" prop:value=move || os_choice.get() on:change:target=move |event| os_choice.set(event.target().value()) disabled=move || disabled.get()><option value="windows">"windows"</option><option value="linux">"linux"</option><option value="macos">"macos"</option></select></label>
                <label><span class="v2-create-help">"Architecture"</span><select class="v2-input" prop:value=move || arch_choice.get() on:change:target=move |event| arch_choice.set(event.target().value()) disabled=move || disabled.get()><option value="x86_64">"x86_64"</option><option value="aarch64">"aarch64"</option></select></label>
                <button type="button" class="v2-btn-secondary" disabled=move || disabled.get() on:click=move |_| {
                    let target = format!("{}-{}", os_choice.get_untracked(), arch_choice.get_untracked());
                    let mut values = selected_platforms(&state.get_untracked(), &platforms_input.get_untracked());
                    if !values.contains(&target) {
                        values.push(target);
                    }
                    state.update(|state| if let LaunchMode::Launchable(tier) = &mut state.launch { tier.replace_platforms(values.clone()); });
                    platforms_input.set(values.join(", "));
                    distribution_revision.update(|revision| *revision = revision.wrapping_add(1));
                    platform_revision.update(|revision| *revision = revision.wrapping_add(1));
                }>"Add target"</button>
            </div>
            <div class="v2-create-chip-row">{move || selected_platforms(&state.get(), &platforms_input.get()).into_iter().map(|target| { let removed = target.clone(); view! { <span class="v2-create-chip">{target}<button type="button" class="v2-create-chip-remove" aria-label=format!("Remove {removed}") disabled=move || disabled.get() on:click=move |_| { let values = selected_platforms(&state.get_untracked(), &platforms_input.get_untracked()).into_iter().filter(|value| value != &removed).collect::<Vec<_>>(); state.update(|state| if let LaunchMode::Launchable(tier) = &mut state.launch { tier.replace_platforms(values.clone()); }); platforms_input.set(values.join(", ")); distribution_revision.update(|revision| *revision = revision.wrapping_add(1)); platform_revision.update(|revision| *revision = revision.wrapping_add(1)); }>"Remove"</button></span> } }).collect_view()}</div>
            <p class="v2-create-help">"Launchable listings require at least one target. Platform tags are optional for download-only listings. Candidate ranking uses the selected target in the executable picker."</p>
        </div>
    }
}

fn download_only_capability_copy(reason: &NoLaunchTierReason) -> Option<&'static str> {
    match reason {
        NoLaunchTierReason::DownloadOnly => Some(
            "Downloadable, not launchable. This listing does not claim the optional launch tier.",
        ),
        _ => None,
    }
}

#[component]
fn AuthoritativeLaunchStatus(
    model: AuthoritativeEditModel,
    defect_review: RwSignal<DefectReviewState>,
    disabled: Signal<bool>,
) -> impl IntoView {
    match model.state {
        AuthoritativeEditState::Loading => view! { <div class="v2-create-alert">"Loading authoritative signed launch metadata…"</div> }.into_any(),
        AuthoritativeEditState::Absent => {
            let copy = download_only_capability_copy(&NoLaunchTierReason::DownloadOnly)
                .expect("download-only has capability copy");
            view! { <div class="v2-create-inline-note">{copy}</div> }.into_any()
        },
        AuthoritativeEditState::PresentValid(_) => view! { <div class="v2-create-inline-note">"Valid profiles were preloaded from the authoritative signed listing."</div> }.into_any(),
        AuthoritativeEditState::PresentDefective(metadata) => {
            let defect_count = metadata.defects.len();
            let defect_views = metadata
                .defects
                .iter()
                .map(|defect| {
                    let (copy, target) = defect_copy(defect);
                    view! { <li><a href=format!("#{target}")>{copy}</a></li> }
                })
                .collect_view();
            view! {
                <section class="v2-create-defects" aria-labelledby="publish-launch-defects-title">
                    <h4 id="publish-launch-defects-title" class="v2-create-field-label">"Authoritative listing defects — repair before publishing"</h4>
                    <p class="v2-create-help">"Malformed signed tags are shown as evidence and are not silently repaired or preloaded."</p>
                    <ul>{defect_views}</ul>
                    <label class="v2-create-checkbox-row"><input type="checkbox" checked=move || defect_review.get().reviewed disabled=move || disabled.get() on:change:target=move |event| defect_review.update(|review| { review.reviewed = event.target().checked(); review.accepted_defect_indices = if review.reviewed { (0..defect_count).collect() } else { Vec::new() }; }) />"I reviewed every authoritative defect and will replace the malformed launch metadata with the editor values below."</label>
                </section>
            }.into_any()
        },
        AuthoritativeEditState::RelayError(error) | AuthoritativeEditState::NotFound(error) => view! { <div class="v2-create-alert v2-create-alert-error" role="alert">{format!("Authoritative launch metadata is unavailable: {error}")}</div> }.into_any(),
        AuthoritativeEditState::Stale(error) => view! {
            <div class="v2-create-alert v2-create-alert-error" role="alert">
                <strong>"A concurrent listing revision was detected. Publication is blocked."</strong>
                <p>{format!("Your local work is preserved and was not overwritten ({error}). Leave this workflow, return to Published games, and reopen the latest revision before publishing.")}</p>
            </div>
        }.into_any(),
        AuthoritativeEditState::InvalidAuthority(error) => view! { <div class="v2-create-alert v2-create-alert-error" role="alert">{format!("Authoritative launch metadata is invalid: {error:?}")}</div> }.into_any(),
    }
}

#[component]
fn ArchiveSafety(
    state: RwSignal<PublishState>,
    hazard_pages: RwSignal<Vec<HazardPage>>,
    warning_pages: RwSignal<Vec<WarningPage>>,
    loading: RwSignal<bool>,
    error: RwSignal<Option<String>>,
) -> impl IntoView {
    view! {
        <Show when=move || state.get().archive.summary.is_some()>
            <section class="v2-create-archive-safety" aria-labelledby="publish-archive-safety-title">
                <h4 id="publish-archive-safety-title" class="v2-create-subsection-title">"Archive safety inspection"</h4>
                {move || state.get().archive.summary.map(|summary| if summary.blocking_hazards { view! { <p class="v2-create-alert v2-create-alert-error" role="alert">"This archive has blocking hazards. They cannot be overridden; the backend blocks publication. Fix and re-export the archive."</p> }.into_any() } else { view! { <p class="v2-create-inline-note">"No blocking archive hazards were detected."</p> }.into_any() })}
                <Show when=move || loading.get()><p class="v2-create-help">"Loading every nonzero hazard and warning category…"</p></Show>
                {move || error.get().map(|error| view! { <p class="v2-create-inline-error" role="alert">{error}</p> })}
                {move || hazard_pages.get().into_iter().map(|group| view! { <div class="v2-create-hazard-group"><strong>{format!("{} — showing {} of {}", group.label, group.page.items.len(), group.page.total)}</strong><ul>{group.page.items.iter().map(|detail| view! { <li>{hazard_copy(detail)}</li> }).collect_view()}</ul>{group.page.next_offset.is_some().then(|| view! { <p class="v2-create-help">{format!("Details are capped at {DETAIL_PAGE_SIZE} for this category. Fix the named entries, then reselect the archive to inspect again.")}</p> })}</div> }).collect_view()}
                {move || warning_pages.get().into_iter().map(|group| view! { <div class="v2-create-warning-group"><strong>{format!("{} — showing {} of {}", group.label, group.page.items.len(), group.page.total)}</strong><ul>{group.page.items.iter().map(|detail| view! { <li>{warning_copy(detail)}</li> }).collect_view()}</ul></div> }).collect_view()}
            </section>
        </Show>
    }
}

#[component]
fn PathPickerShell(
    picker: RwSignal<Option<OpenPicker>>,
    search: RwSignal<String>,
    candidate_target: RwSignal<String>,
    offset: RwSignal<u64>,
    loading: RwSignal<bool>,
    error: RwSignal<Option<String>>,
    file_page: RwSignal<Option<ArchivePage<SelectableArchiveFile>>>,
    directory_page: RwSignal<Option<ArchivePage<SelectableWorkingDirectory>>>,
    candidate_page: RwSignal<Option<ArchivePage<ArchiveCandidate>>>,
    choose_file: Callback<SelectableArchiveFile>,
    choose_directory: Callback<SelectableWorkingDirectory>,
    choose_candidate: Callback<ArchiveCandidate>,
    close: Callback<()>,
    state: RwSignal<PublishState>,
) -> impl IntoView {
    let kind = move || picker.get().map(|picker| picker.kind);
    let page_total = move || match kind() {
        Some(PickerKind::WorkingDirectory) => directory_page.get().map(|page| page.total),
        Some(PickerKind::Executable) if picker.get().is_some_and(|picker| !picker.browse_all) => {
            candidate_page.get().map(|page| page.total)
        }
        Some(PickerKind::Executable) => file_page.get().map(|page| page.total),
        None => None,
    };
    let next_offset = move || match kind() {
        Some(PickerKind::WorkingDirectory) => {
            directory_page.get().and_then(|page| page.next_offset)
        }
        Some(PickerKind::Executable) if picker.get().is_some_and(|picker| !picker.browse_all) => {
            candidate_page.get().and_then(|page| page.next_offset)
        }
        Some(PickerKind::Executable) => file_page.get().and_then(|page| page.next_offset),
        None => None,
    };
    view! {
        <section class="v2-create-picker" aria-labelledby="publish-path-picker-title">
            <div class="v2-create-row-between"><div><h4 id="publish-path-picker-title" class="v2-create-subsection-title">{move || match kind() { Some(PickerKind::Executable) => "Choose executable", Some(PickerKind::WorkingDirectory) => "Choose working directory", None => "Archive path picker" }}</h4><p class="v2-create-help">{move || kind().map(source_property).unwrap_or_default()}</p></div><button type="button" class="v2-create-media-link" on:click=move |_| close.run(())>"Close"</button></div>
            <label><span class="v2-create-field-label">"Search complete archive paths"</span><input class="v2-input" type="search" prop:value=move || search.get() on:input:target=move |event| { search.set(event.target().value()); offset.set(0); } placeholder="Type part of a path" /></label>
            <Show when=move || matches!(kind(), Some(PickerKind::Executable)) && picker.get().is_some_and(|picker| !picker.browse_all)>
                <label><span class="v2-create-field-label">"Candidate target"</span><select class="v2-input" on:change:target=move |event| { candidate_target.set(event.target().value()); offset.set(0); }>{move || state.get().launch_platforms().into_iter().map(|target| { let selected = target.clone(); let label = target.clone(); view! { <option value=target selected=move || candidate_target.get() == selected>{label}</option> } }).collect_view()}</select></label>
            </Show>
            <Show when=move || matches!(kind(), Some(PickerKind::Executable)) && picker.get().is_some_and(|picker| !picker.browse_all)><div class="v2-create-row-between"><p class="v2-create-help">"Ranked candidates are suggestions only; none is preselected."</p><button type="button" class="v2-create-media-link" on:click=move |_| { picker.update(|open| if let Some(open) = open { open.browse_all = true; }); offset.set(0); }>"Browse all files"</button></div></Show>
            <Show when=move || matches!(kind(), Some(PickerKind::WorkingDirectory))><button type="button" class="v2-create-picker-option v2-create-picker-root" on:click=move |_| if let Ok(directory) = WorkingDir::parse(".") { choose_directory.run(SelectableWorkingDirectory { directory }); }><span class="v2-create-mono-inline">"."</span><span>"Installation root (pinned)"</span></button></Show>
            {move || error.get().map(|error| view! { <p class="v2-create-inline-error" role="alert">{error}</p> })}
            <p class="v2-create-help" role="status" aria-live="polite" aria-atomic="true">
                {move || if loading.get() {
                    "Searching archive index…".to_string()
                } else {
                    page_total().map(|total| if total == 0 {
                        "No archive picker results.".to_string()
                    } else {
                        format!("{total} archive picker result(s) available.")
                    }).unwrap_or_default()
                }}
            </p>
            <div class="v2-create-picker-results">
                {move || candidate_page.get().filter(|_| matches!(kind(), Some(PickerKind::Executable)) && picker.get().is_some_and(|picker| !picker.browse_all)).map(|page| page.items.into_iter().map(|candidate| { let chosen = candidate.clone(); view! { <button type="button" class="v2-create-picker-option" on:click=move |_| choose_candidate.run(chosen.clone())><span class="v2-create-mono-inline">{candidate.path.as_str().to_string()}</span><span>{format!("{} · {}", candidate_confidence(candidate.confidence), candidate_reason(candidate.reason))}</span></button> } }).collect_view())}
                {move || file_page.get().filter(|_| matches!(kind(), Some(PickerKind::Executable)) && picker.get().is_some_and(|picker| picker.browse_all)).map(|page| page.items.into_iter().map(|file| { let chosen = file.clone(); view! { <button type="button" class="v2-create-picker-option" on:click=move |_| choose_file.run(chosen.clone())><span class="v2-create-mono-inline">{file.path.as_str().to_string()}</span><span>{format!("{} bytes", file.declared_size)}</span></button> } }).collect_view())}
                {move || directory_page.get().filter(|_| matches!(kind(), Some(PickerKind::WorkingDirectory))).map(|page| page.items.into_iter().filter(|directory| !directory.directory.is_root()).map(|directory| { let chosen = directory.clone(); view! { <button type="button" class="v2-create-picker-option" on:click=move |_| choose_directory.run(chosen.clone())><span class="v2-create-mono-inline">{directory.directory.as_str().to_string()}</span></button> } }).collect_view())}
            </div>
            <Show when=move || !loading.get() && page_total() == Some(0)><p class="v2-create-empty">{move || if matches!(kind(), Some(PickerKind::Executable)) && picker.get().is_some_and(|picker| !picker.browse_all) { "No suggested executables match. This is valid; change the search or browse all files." } else if search.get().is_empty() { "No matching entries. Change the search or inspect another archive." } else { "No archive paths match this search." }}</p></Show>
            <Show when=move || search.get().is_empty() && page_total().is_some_and(|total| total >= LARGE_BROWSE_TOTAL)><p class="v2-create-help">{move || format!("This index has {} entries; browsing is impractical. Search by path instead.", page_total().unwrap_or_default())}</p></Show>
            <div class="v2-create-row-between"><button type="button" class="v2-btn-secondary" disabled=move || offset.get() == 0 || loading.get() on:click=move |_| offset.update(|value| *value = value.saturating_sub(PAGE_SIZE as u64))>"Previous"</button><span class="v2-create-help">{move || page_total().map(|total| format!("Showing from {} of {total}", offset.get())).unwrap_or_default()}</span><button type="button" class="v2-btn-secondary" disabled=move || next_offset().is_none() || loading.get() on:click=move |_| if let Some(next) = next_offset() { offset.set(next); }>"Next"</button></div>
            <p class="v2-create-help">{move || state.get().archive.summary.map(|summary| format!("Inspected archive: {}", summary.selection.safe_filename.as_str())).unwrap_or_default()}</p>
        </section>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_control_choices_are_exact() {
        assert_eq!(
            ["windows", "linux", "macos"].join(","),
            "windows,linux,macos"
        );
        assert_eq!(["x86_64", "aarch64"].join(","), "x86_64,aarch64");
        assert!(parse_platform_tag("windows-x86_64").is_some());
    }

    #[test]
    fn intentional_download_only_is_capability_copy_not_defect_copy() {
        let copy = download_only_capability_copy(&NoLaunchTierReason::DownloadOnly)
            .expect("intentional download-only copy");

        assert_eq!(
            copy,
            "Downloadable, not launchable. This listing does not claim the optional launch tier."
        );
        for defect_word in ["error", "defect", "broken", "missing"] {
            assert!(!copy.to_lowercase().contains(defect_word));
        }
        assert_eq!(
            download_only_capability_copy(&NoLaunchTierReason::MissingLaunchProfiles),
            None
        );
        assert_eq!(
            download_only_capability_copy(&NoLaunchTierReason::MissingPlatform),
            None
        );
        assert_eq!(
            download_only_capability_copy(&NoLaunchTierReason::NoGameProfile),
            None
        );
    }

    #[test]
    fn source_copy_never_describes_text_entry() {
        assert!(source_property(PickerKind::Executable).contains("archive index"));
        assert!(source_property(PickerKind::WorkingDirectory).contains("root (.)"));
    }

    #[test]
    fn original_indices_are_preserved_in_copy() {
        assert_eq!(format_indices(&[2, 7]), "tag indices 2, 7");
        assert_eq!(format_indices(&[]), "listing-level (no individual tag)");
    }

    #[test]
    fn review_projection_uses_validated_canonical_launch_values() {
        let mut tier = crate::publish_state::LaunchTierDraft::default();
        tier.package_format.text = "ZIP".into();
        tier.replace_platforms(vec!["windows-x86_64".into()]);
        let row = tier.add_profile();
        let profile = tier
            .profiles
            .iter_mut()
            .find(|profile| profile.row_id == row)
            .expect("added profile must exist");
        profile.profile_id = "default".into();
        profile.display_name = "Play".into();
        profile.executable.authoritative_value = Some(
            arcadestr_core::distribution::RelativePath::parse("bin/Game.exe")
                .expect("test executable must be valid"),
        );
        profile.working_dir.authoritative_value =
            Some(WorkingDir::parse("bin").expect("test working directory must be valid"));
        profile.arguments = vec!["--renderer".into(), "vulkan".into()];
        assert!(tier.set_publisher_default_profile(row, true));

        let projection = validated_review_projection(
            &LaunchMode::Launchable(tier),
            &ArchiveSelectionState::new("npub-test".into()),
            true,
            Some("a".repeat(64)),
            vec!["Filename extension mismatch".into()],
        )
        .expect("complete launch tier must project");
        let intent = projection.launch_tier.expect("launchable tier expected");
        assert_eq!(intent.package_format, PackageFormat::Zip);
        assert_eq!(intent.platforms.len(), 1);
        assert_eq!(intent.launch_profiles[0].id.as_str(), "default");
        assert_eq!(intent.publisher_defaults[0].profile_id.as_str(), "default");
        assert_eq!(intent.publisher_defaults[0].targets, intent.platforms);
        assert_eq!(
            intent.launch_profiles[0].executable.as_str(),
            "bin/Game.exe"
        );
        assert_eq!(
            intent.launch_profiles[0].arguments,
            vec!["--renderer".to_string(), "vulkan".to_string()]
        );
        assert_eq!(
            projection.warnings,
            vec!["Filename extension mismatch".to_string()]
        );
        assert!(matches!(
            projection.artifact,
            ReviewArtifact::Existing { .. }
        ));
    }

    #[test]
    fn launch_default_defects_use_existing_review_targets() {
        let dangling = AuthoritativeAdpEditDefect::DanglingLaunchDefault {
            event_tag_index: 7,
            profile_id: arcadestr_core::distribution::ProfileId::parse("missing")
                .expect("profile id"),
        };
        let malformed = AuthoritativeAdpEditDefect::MalformedLaunchDefault {
            event_tag_index: 8,
            raw_profile_id: Some("Bad".to_string()),
        };

        let (dangling_copy, dangling_target) = defect_copy(&dangling);
        let (malformed_copy, malformed_target) = defect_copy(&malformed);
        assert!(dangling_copy.contains("Automatic Play profile"));
        assert!(malformed_copy.contains("Automatic Play profile"));
        assert_eq!(dangling_target, "publish-launch-profiles");
        assert_eq!(malformed_target, "publish-launch-profiles");
    }
}
