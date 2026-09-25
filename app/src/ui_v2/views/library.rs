use std::collections::{HashMap, HashSet};

use arcadestr_core::is_replaceable_event_newer;
use leptos::prelude::*;
use nostr::nips::nip19::FromBech32;
use wasm_bindgen_futures::spawn_local;

use crate::models::{
    npub_fallback_label, AcquisitionPolicy, GameListing, ListingSource, PlatformInfo,
};
use crate::tauri_bridge::{
    invoke_delete_retained_package, invoke_get_installed_games, invoke_get_library_games,
    invoke_get_listing_ownership, invoke_get_platform_info, invoke_get_uninstall_sizes,
    invoke_list_installation_lifecycle_issues, invoke_list_retained_packages,
    invoke_reinstall_retained_package, invoke_remove_failed_installation, invoke_resume_uninstall,
    invoke_retry_installation_recovery, invoke_uninstall_game, InstallationLifecycleIssue,
    InstallationRecoveryDiscriminator, InstallationState, InstalledGame, LibraryGame,
    RetainedPackage, UninstallSizes,
};
use crate::ui_v2::components::{
    artwork_state_from_url, ArtworkRole, Dialog, DialogCloseAction, DialogClosePolicy,
    DialogCloseRequest, DialogDismissal, DialogInitialFocus, DialogTone, DialogWidth, EmptyState,
    ErrorSeverity, ErrorState, FeedbackLayout, GameArtwork, LoadingState, PageHeader, PageTabItem,
    PageTabSemantics, PageTabs, StatusChip, StatusChipSize, StatusChipVariant,
};
use crate::ui_v2::views::marketplace_loader::use_marketplace_listings_with_limit;
use crate::ui_v2::views::valid_cover_url;
use crate::AuthContext;

const LIBRARY_MARKETPLACE_LIMIT: usize = 200;
const INSTALLATION_SCOPE_LABEL: &str = "Installed on this device; not account-scoped";

#[derive(Clone, Debug, PartialEq, Eq)]
enum LibraryDestructiveAction {
    Uninstall(InstalledGame),
    ResumeUninstall(InstallationLifecycleIssue),
    RemoveFailed(InstallationLifecycleIssue),
    DeleteRetained(RetainedPackage),
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum LibraryStorageCommand {
    Uninstall {
        installation_id: String,
        retain_package: bool,
    },
    RemoveFailed {
        installation_id: String,
    },
    ResumeUninstall {
        installation_id: String,
    },
    DeleteRetained {
        game_coordinate: String,
        expected_package_id: String,
    },
}

fn storage_command(
    action: &LibraryDestructiveAction,
    retain_package: bool,
) -> LibraryStorageCommand {
    match action {
        LibraryDestructiveAction::Uninstall(installed) => LibraryStorageCommand::Uninstall {
            installation_id: installed.installation_id.clone(),
            retain_package,
        },
        LibraryDestructiveAction::RemoveFailed(issue) => LibraryStorageCommand::RemoveFailed {
            installation_id: issue.installation_id.clone(),
        },
        LibraryDestructiveAction::ResumeUninstall(issue) => {
            LibraryStorageCommand::ResumeUninstall {
                installation_id: issue.installation_id.clone(),
            }
        }
        LibraryDestructiveAction::DeleteRetained(package) => {
            LibraryStorageCommand::DeleteRetained {
                game_coordinate: package.game_coordinate.clone(),
                expected_package_id: package.package_id.clone(),
            }
        }
    }
}

fn recovery_message(discriminator: Option<InstallationRecoveryDiscriminator>) -> &'static str {
    match discriminator {
        Some(InstallationRecoveryDiscriminator::FinalRootPresent) => {
            "Installation files were preserved, but may be incomplete and are not launchable. Cleanup deletes that tree; any retained installer package remains separate."
        }
        Some(InstallationRecoveryDiscriminator::FinalRootAbsent) | None => {
            "Installation was interrupted and no installed files remain. Cleanup removes only the failed installation record; any retained installer package remains separate."
        }
    }
}

fn lifecycle_issue_message(issue: &InstallationLifecycleIssue) -> &'static str {
    match issue.state {
        InstallationState::Installing => {
            "Installation is still marked busy. Retry status after the active operation has had time to finish."
        }
        InstallationState::Uninstalling => {
            "A previous uninstall was interrupted. Resume uninstall to remove the installation root while conservatively keeping any retained package."
        }
        InstallationState::Failed => recovery_message(issue.recovery_discriminator),
        InstallationState::Ready => "Installation is ready.",
    }
}

fn lifecycle_action_unavailable(
    registry_loading: bool,
    registry_failed: bool,
    lifecycle_loading: bool,
    lifecycle_failed: bool,
) -> bool {
    registry_loading || registry_failed || lifecycle_loading || lifecycle_failed
}

fn format_size(bytes: u64) -> String {
    const KIB: f64 = 1024.0;
    const MIB: f64 = KIB * 1024.0;
    const GIB: f64 = MIB * 1024.0;
    let bytes_f64 = bytes as f64;
    if bytes_f64 >= GIB {
        format!("{:.1} GiB", bytes_f64 / GIB)
    } else if bytes_f64 >= MIB {
        format!("{:.1} MiB", bytes_f64 / MIB)
    } else if bytes_f64 >= KIB {
        format!("{:.1} KiB", bytes_f64 / KIB)
    } else {
        format!("{bytes} B")
    }
}

fn default_retain_package() -> bool {
    true
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RetainedReinstallEligibility {
    Available,
    ReadyInstalled,
    Lifecycle(InstallationState),
}

fn retained_reinstall_eligibility(
    coordinate: &str,
    installed: &[InstalledGame],
    issues: &[InstallationLifecycleIssue],
) -> RetainedReinstallEligibility {
    if installed
        .iter()
        .any(|installation| installation.game_coordinate == coordinate)
    {
        RetainedReinstallEligibility::ReadyInstalled
    } else if let Some(issue) = issues
        .iter()
        .find(|issue| issue.game_coordinate == coordinate)
    {
        RetainedReinstallEligibility::Lifecycle(issue.state)
    } else {
        RetainedReinstallEligibility::Available
    }
}

fn retained_reinstall_unavailable_message(
    eligibility: RetainedReinstallEligibility,
) -> Option<&'static str> {
    match eligibility {
        RetainedReinstallEligibility::Available => None,
        RetainedReinstallEligibility::ReadyInstalled => Some("Already installed on this device."),
        RetainedReinstallEligibility::Lifecycle(InstallationState::Installing) => {
            Some("An installation is already in progress or awaiting status recovery.")
        }
        RetainedReinstallEligibility::Lifecycle(InstallationState::Uninstalling) => {
            Some("Resolve the interrupted uninstall before reinstalling.")
        }
        RetainedReinstallEligibility::Lifecycle(InstallationState::Failed) => {
            Some("Clean up the failed installation record before reinstalling.")
        }
        RetainedReinstallEligibility::Lifecycle(InstallationState::Ready) => {
            Some("Already installed on this device.")
        }
    }
}

fn listing_for_coordinate(listings: &[GameListing], coordinate: &str) -> Option<GameListing> {
    listings
        .iter()
        .filter(|listing| listing_coordinate(listing).as_deref() == Some(coordinate))
        .cloned()
        .reduce(|current, candidate| {
            if is_replaceable_event_newer(
                candidate.created_at,
                candidate.event_id.as_deref(),
                current.created_at,
                current.event_id.as_deref(),
            ) {
                candidate
            } else {
                current
            }
        })
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum LibraryFilter {
    #[default]
    All,
    Saved,
    Installed,
    NotInstalled,
    TimedAccess,
    NeedsAttention,
}

impl LibraryFilter {
    const fn id(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::Saved => "saved",
            Self::Installed => "installed",
            Self::NotInstalled => "not-installed",
            Self::TimedAccess => "timed-access",
            Self::NeedsAttention => "needs-attention",
        }
    }

    fn from_id(id: &str) -> Self {
        match id {
            "saved" => Self::Saved,
            "installed" => Self::Installed,
            "not-installed" => Self::NotInstalled,
            "timed-access" => Self::TimedAccess,
            "needs-attention" => Self::NeedsAttention,
            _ => Self::All,
        }
    }

    const fn empty_copy(self) -> (&'static str, &'static str) {
        match self {
            Self::All => (
                "Your library is empty",
                "Saved account games and installations on this device will appear here.",
            ),
            Self::Saved => (
                "No account-saved games",
                "Games added to the active account's library will appear here.",
            ),
            Self::Installed => (
                "No installed games",
                "Extracted installations recorded on this device will appear here.",
            ),
            Self::NotInstalled => (
                "No saved games waiting for download",
                "Every matched account-saved game is already installed on this device.",
            ),
            Self::TimedAccess => (
                "No timed-access games",
                "No saved or installed listing currently uses a timed-access policy.",
            ),
            Self::NeedsAttention => (
                "No library issues found",
                "No visible records have expired access, ownership lookup failures, malformed coordinates, or compatibility warnings.",
            ),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LibraryCompatibility {
    Compatible,
    Incompatible,
    Unknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LibraryAvailability {
    DesktopRegistry,
    DesktopOnly,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LibraryContentState {
    Loading,
    BlockingError,
    Empty,
    Ready,
}

fn library_content_state(
    visible_count: usize,
    has_saved_data: bool,
    has_installed_data: bool,
    waiting_for_saved: bool,
    waiting_for_installed: bool,
    saved_failed: bool,
    registry_failed: bool,
) -> LibraryContentState {
    if visible_count > 0 {
        LibraryContentState::Ready
    } else if waiting_for_saved || waiting_for_installed {
        LibraryContentState::Loading
    } else if !has_saved_data && !has_installed_data && (registry_failed || saved_failed) {
        LibraryContentState::BlockingError
    } else {
        LibraryContentState::Empty
    }
}

#[derive(Clone, Debug, PartialEq)]
struct LibraryEntry {
    installed: InstalledGame,
    listing: Option<GameListing>,
    malformed_coordinate: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct AccessPresentation {
    label: &'static str,
    durable_owned: bool,
}

fn ownership_for_active_account(
    ownership: &HashMap<(String, String), bool>,
    active_npub: Option<&str>,
    coordinate: &str,
) -> bool {
    active_npub
        .and_then(|npub| ownership.get(&(npub.to_string(), coordinate.to_string())))
        .copied()
        .unwrap_or(false)
}

fn target_library_availability(standalone_web: bool) -> LibraryAvailability {
    if standalone_web {
        LibraryAvailability::DesktopOnly
    } else {
        LibraryAvailability::DesktopRegistry
    }
}

fn installed_status(launchable: bool) -> (&'static str, StatusChipVariant) {
    if launchable {
        ("Installed", StatusChipVariant::Installed)
    } else {
        ("Installed, not launchable", StatusChipVariant::Warning)
    }
}

fn parse_game_coordinate(coordinate: &str) -> Option<(&str, &str)> {
    let mut parts = coordinate.splitn(3, ':');
    if parts.next()? != "30402" {
        return None;
    }
    let publisher_hex = parts.next()?;
    let listing_id = parts.next()?;
    if nostr::PublicKey::from_hex(publisher_hex).is_err() || listing_id.trim().is_empty() {
        return None;
    }
    Some((publisher_hex, listing_id))
}

fn listing_coordinate(listing: &GameListing) -> Option<String> {
    if listing.source != ListingSource::Nip99Listing {
        return None;
    }
    let publisher = nostr::PublicKey::from_bech32(&listing.publisher_npub).ok()?;
    (!listing.id.trim().is_empty()).then(|| format!("30402:{}:{}", publisher.to_hex(), listing.id))
}

fn reconcile_installed_entries(
    mut installed: Vec<InstalledGame>,
    listings: &[GameListing],
) -> Vec<LibraryEntry> {
    installed.sort_by(|left, right| {
        right
            .installed_at
            .cmp(&left.installed_at)
            .then_with(|| left.game_coordinate.cmp(&right.game_coordinate))
    });

    let mut listings_by_coordinate = HashMap::<String, &GameListing>::new();
    for listing in listings {
        let Some(coordinate) = listing_coordinate(listing) else {
            continue;
        };
        let replace = listings_by_coordinate
            .get(&coordinate)
            .map(|current| {
                is_replaceable_event_newer(
                    listing.created_at,
                    listing.event_id.as_deref(),
                    current.created_at,
                    current.event_id.as_deref(),
                )
            })
            .unwrap_or(true);
        if replace {
            listings_by_coordinate.insert(coordinate, listing);
        }
    }

    installed
        .into_iter()
        .map(|installed| {
            let malformed_coordinate = parse_game_coordinate(&installed.game_coordinate).is_none();
            let listing = listings_by_coordinate
                .get(&installed.game_coordinate)
                .map(|listing| (*listing).clone());
            LibraryEntry {
                installed,
                listing,
                malformed_coordinate,
            }
        })
        .collect()
}

fn access_presentation(
    listing: &GameListing,
    durable_owned_for_active_account: bool,
    now: u64,
) -> AccessPresentation {
    let label = match listing.acquisition {
        AcquisitionPolicy::Public => "Public access",
        AcquisitionPolicy::TimedAccess { starts_at, .. } if now < starts_at => {
            "Timed access has not started"
        }
        AcquisitionPolicy::TimedAccess { ends_at, .. } if now >= ends_at => {
            "Current timed access ended"
        }
        AcquisitionPolicy::TimedAccess { .. } => "Timed access currently available",
        AcquisitionPolicy::Gated if durable_owned_for_active_account => "Owned for active account",
        AcquisitionPolicy::Gated => "Ownership not confirmed for active account",
    };
    AccessPresentation {
        label,
        durable_owned: matches!(listing.acquisition, AcquisitionPolicy::Gated)
            && durable_owned_for_active_account,
    }
}

fn listing_compatibility(
    listing: Option<&GameListing>,
    platform: Option<&PlatformInfo>,
) -> LibraryCompatibility {
    let Some(listing) = listing else {
        return LibraryCompatibility::Unknown;
    };
    if listing.platforms.is_empty() {
        return LibraryCompatibility::Unknown;
    }
    match platform {
        Some(platform) => {
            let host = arcadestr_core::distribution::PlatformTarget {
                os: arcadestr_core::distribution::normalize_os(&platform.os)
                    .unwrap_or_else(|| platform.os.clone()),
                arch: arcadestr_core::distribution::normalize_arch(&platform.arch)
                    .unwrap_or_else(|| platform.arch.clone()),
            };
            if listing.platforms.iter().any(|value| {
                arcadestr_core::distribution::parse_platform_tag(value).as_ref() == Some(&host)
            }) {
                LibraryCompatibility::Compatible
            } else {
                LibraryCompatibility::Incompatible
            }
        }
        None => LibraryCompatibility::Unknown,
    }
}

fn normalize_search(value: &str) -> String {
    value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

fn fallback_title(entry: &LibraryEntry) -> String {
    entry
        .listing
        .as_ref()
        .map(|listing| listing.title.clone())
        .or_else(|| {
            parse_game_coordinate(&entry.installed.game_coordinate)
                .map(|(_, listing_id)| format!("Installed game: {listing_id}"))
        })
        .unwrap_or_else(|| "Installed game".to_string())
}

fn publisher_label(entry: &LibraryEntry) -> String {
    entry
        .listing
        .as_ref()
        .map(|listing| {
            listing
                .stall_name
                .clone()
                .filter(|value| !value.trim().is_empty())
                .unwrap_or_else(|| npub_fallback_label(&listing.publisher_npub))
        })
        .or_else(|| {
            parse_game_coordinate(&entry.installed.game_coordinate).map(|(publisher, _)| {
                format!(
                    "Publisher {}...{}",
                    &publisher[..8],
                    &publisher[publisher.len() - 8..]
                )
            })
        })
        .unwrap_or_else(|| "Publisher unavailable".to_string())
}

fn entry_matches_search(entry: &LibraryEntry, query: &str) -> bool {
    let query = normalize_search(query);
    if query.is_empty() {
        return true;
    }
    let mut fields = vec![
        fallback_title(entry),
        publisher_label(entry),
        entry.installed.game_coordinate.clone(),
        entry.installed.package_digest.clone(),
    ];
    if let Some(listing) = &entry.listing {
        fields.extend(listing.platforms.iter().cloned());
    }
    normalize_search(&fields.join(" ")).contains(&query)
}

fn installed_entry_matches_filter(
    entry: &LibraryEntry,
    filter: LibraryFilter,
    compatibility: LibraryCompatibility,
    ownership_failed: bool,
    now: u64,
) -> bool {
    match filter {
        LibraryFilter::All | LibraryFilter::Installed => true,
        LibraryFilter::Saved | LibraryFilter::NotInstalled => false,
        LibraryFilter::TimedAccess => entry
            .listing
            .as_ref()
            .is_some_and(|listing| matches!(listing.acquisition, AcquisitionPolicy::TimedAccess { .. })),
        LibraryFilter::NeedsAttention => {
            entry.malformed_coordinate
                || compatibility == LibraryCompatibility::Incompatible
                || ownership_failed
                || entry.listing.as_ref().is_some_and(|listing| {
                    matches!(listing.acquisition, AcquisitionPolicy::TimedAccess { ends_at, .. } if now >= ends_at)
                })
        }
    }
}

fn saved_entry_matches_search(
    saved: &LibraryGame,
    listing: Option<&GameListing>,
    query: &str,
) -> bool {
    let query = normalize_search(query);
    if query.is_empty() {
        return true;
    }
    let mut fields = vec![saved.game_coordinate.clone()];
    if let Some(listing) = listing {
        fields.push(listing.title.clone());
        fields.push(
            listing
                .stall_name
                .clone()
                .unwrap_or_else(|| npub_fallback_label(&listing.publisher_npub)),
        );
        fields.extend(listing.platforms.iter().cloned());
    }
    normalize_search(&fields.join(" ")).contains(&query)
}

fn saved_entry_matches_filter(
    saved: &LibraryGame,
    listing: Option<&GameListing>,
    installed_coordinates: &HashSet<String>,
    filter: LibraryFilter,
    compatibility: LibraryCompatibility,
    now: u64,
) -> bool {
    let installed = installed_coordinates.contains(&saved.game_coordinate);
    match filter {
        LibraryFilter::All => !installed,
        LibraryFilter::Saved => true,
        LibraryFilter::Installed => false,
        LibraryFilter::NotInstalled => !installed,
        LibraryFilter::TimedAccess => listing.is_some_and(|listing| {
            matches!(listing.acquisition, AcquisitionPolicy::TimedAccess { .. })
        }),
        LibraryFilter::NeedsAttention => {
            compatibility == LibraryCompatibility::Incompatible
                || listing.is_some_and(|listing| {
                    matches!(listing.acquisition, AcquisitionPolicy::TimedAccess { ends_at, .. } if now >= ends_at)
                })
        }
    }
}

fn installation_directory_label(path: &str) -> String {
    path.trim()
        .rsplit(['/', '\\'])
        .find(|name| !name.is_empty())
        .unwrap_or("Unavailable")
        .to_string()
}

fn access_chip(listing: &GameListing, now: u64) -> (&'static str, StatusChipVariant) {
    match listing.acquisition {
        AcquisitionPolicy::Public => ("Public", StatusChipVariant::Public),
        AcquisitionPolicy::Gated => ("Gated", StatusChipVariant::Gated),
        AcquisitionPolicy::TimedAccess { starts_at, .. } if now < starts_at => {
            ("Timed: upcoming", StatusChipVariant::Pending)
        }
        AcquisitionPolicy::TimedAccess { ends_at, .. } if now >= ends_at => {
            ("Timed: expired", StatusChipVariant::Expired)
        }
        AcquisitionPolicy::TimedAccess { .. } => ("Timed: active", StatusChipVariant::TimedAccess),
    }
}

fn compatibility_chip(compatibility: LibraryCompatibility) -> (&'static str, StatusChipVariant) {
    match compatibility {
        LibraryCompatibility::Compatible => ("Compatible", StatusChipVariant::Success),
        LibraryCompatibility::Incompatible => ("Unsupported", StatusChipVariant::Error),
        LibraryCompatibility::Unknown => ("Compatibility unknown", StatusChipVariant::Neutral),
    }
}

fn current_unix_secs() -> u64 {
    #[cfg(target_arch = "wasm32")]
    {
        (js_sys::Date::now() / 1000.0) as u64
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_secs())
            .unwrap_or_default()
    }
}

fn format_installed_at(timestamp: i64) -> String {
    if timestamp <= 0 {
        return "Installation time unavailable".to_string();
    }
    #[cfg(target_arch = "wasm32")]
    {
        let date = js_sys::Date::new(&((timestamp as f64) * 1000.0).into());
        return format!(
            "Installed {:02}/{:02}/{}",
            date.get_month() + 1,
            date.get_date(),
            date.get_full_year()
        );
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        format!("Installed at Unix timestamp {timestamp}")
    }
}

fn completes_retained_reinstall_refresh(
    reinstall_refresh_generation: Option<u64>,
    requested_generation: u64,
) -> bool {
    reinstall_refresh_generation == Some(requested_generation)
}

#[component]
pub fn LibraryView(on_open_listing: Callback<GameListing>) -> impl IntoView {
    let auth = use_context::<AuthContext>().expect("AuthContext not provided");
    let standalone_web = cfg!(feature = "web");
    let availability = target_library_availability(standalone_web);
    let installed_games = RwSignal::new(Vec::<InstalledGame>::new());
    let saved_games = RwSignal::new(Vec::<LibraryGame>::new());
    let saved_account = RwSignal::new(None::<String>);
    let saved_loading = RwSignal::new(false);
    let saved_error = RwSignal::new(None::<String>);
    let registry_loading = RwSignal::new(!standalone_web);
    let registry_error = RwSignal::new(None::<String>);
    let lifecycle_issues = RwSignal::new(Vec::<InstallationLifecycleIssue>::new());
    let lifecycle_loading = RwSignal::new(!standalone_web);
    let lifecycle_error = RwSignal::new(None::<String>);
    let recovery_completed = RwSignal::new(None::<u64>);
    let retained_packages = RwSignal::new(Vec::<RetainedPackage>::new());
    let retained_loading = RwSignal::new(!standalone_web);
    let retained_error = RwSignal::new(None::<String>);
    let retained_reinstalling = RwSignal::new(None::<String>);
    let retained_reinstall_refresh = RwSignal::new(None::<u64>);
    let retained_reinstall_error = RwSignal::new(None::<(String, String)>);
    let refresh_generation = RwSignal::new(0_u64);
    let platform = RwSignal::new(None::<PlatformInfo>);
    let platform_loading = RwSignal::new(!standalone_web);
    let platform_error = RwSignal::new(false);
    let search = RwSignal::new(String::new());
    let library_filter = RwSignal::new(LibraryFilter::All);
    let ownership = RwSignal::new(HashMap::<(String, String), bool>::new());
    let ownership_loading = RwSignal::new(HashSet::<String>::new());
    let ownership_errors = RwSignal::new(HashSet::<String>::new());
    let ownership_generation = RwSignal::new(0_u64);
    let library_now = RwSignal::new(current_unix_secs());
    let destructive_action = RwSignal::new(None::<LibraryDestructiveAction>);
    let destructive_action_generation = RwSignal::new(None::<u64>);
    let destructive_busy = RwSignal::new(false);
    let destructive_error = RwSignal::new(None::<String>);
    let destructive_failure = RwSignal::new(None::<String>);
    let uninstall_sizes = RwSignal::new(None::<UninstallSizes>);
    let uninstall_sizes_loading = RwSignal::new(false);
    let retain_package = RwSignal::new(default_retain_package());
    let action_message = RwSignal::new(None::<String>);
    let destructive_cancel_ref = NodeRef::<leptos::html::Button>::new();
    let marketplace = use_marketplace_listings_with_limit(LIBRARY_MARKETPLACE_LIMIT);

    Effect::new(move |_| {
        let current_generation = refresh_generation.get();
        if destructive_action_generation
            .get_untracked()
            .is_some_and(|generation| generation != current_generation)
        {
            destructive_action.set(None);
            destructive_action_generation.set(None);
            destructive_error.set(None);
            uninstall_sizes.set(None);
            uninstall_sizes_loading.set(false);
        }
    });

    let saved_auth = auth.clone();
    Effect::new(move |_| {
        let requested_generation = refresh_generation.get();
        let requested_account = saved_auth.npub.get();
        saved_error.set(None);
        let Some(requested_account) = requested_account else {
            saved_account.set(None);
            saved_games.set(Vec::new());
            saved_loading.set(false);
            return;
        };
        if saved_account.get_untracked().as_deref() != Some(requested_account.as_str()) {
            saved_games.set(Vec::new());
            saved_account.set(Some(requested_account.clone()));
        }
        if availability == LibraryAvailability::DesktopOnly {
            saved_loading.set(false);
            return;
        }

        saved_loading.set(true);
        let auth_for_response = saved_auth.clone();
        spawn_local(async move {
            let result = invoke_get_library_games().await;
            if refresh_generation.get_untracked() != requested_generation
                || auth_for_response.npub.get_untracked().as_deref()
                    != Some(requested_account.as_str())
            {
                return;
            }
            match result {
                Ok(games) => saved_games.set(games),
                Err(error) => saved_error.set(Some(error)),
            }
            saved_loading.set(false);
        });
    });

    #[cfg(target_arch = "wasm32")]
    {
        let clock = send_wrapper::SendWrapper::new(gloo_timers::callback::Interval::new(
            30_000,
            move || library_now.set(current_unix_secs()),
        ));
        on_cleanup(move || drop(clock));
    }

    Effect::new(move |_| {
        let requested_generation = refresh_generation.get();
        if availability == LibraryAvailability::DesktopOnly {
            registry_loading.set(false);
            return;
        }
        registry_loading.set(true);
        registry_error.set(None);
        recovery_completed.set(None);
        spawn_local(async move {
            // Recovery is intentionally lazy on Library access. Its command
            // result is authoritative; rows are fetched only after it returns.
            let recovery_result = invoke_retry_installation_recovery().await;
            if refresh_generation.get_untracked() != requested_generation {
                return;
            }
            if let Err(error) = recovery_result {
                registry_error.set(Some(format!("Installation recovery failed: {error}")));
                if completes_retained_reinstall_refresh(
                    retained_reinstall_refresh.get_untracked(),
                    requested_generation,
                ) {
                    retained_reinstalling.set(None);
                    retained_reinstall_refresh.set(None);
                }
                registry_loading.set(false);
                return;
            }
            recovery_completed.set(Some(requested_generation));
            let result = invoke_get_installed_games().await;
            if refresh_generation.get_untracked() != requested_generation {
                return;
            }
            let completes_reinstall = completes_retained_reinstall_refresh(
                retained_reinstall_refresh.get_untracked(),
                requested_generation,
            );
            match result {
                Ok(games) => installed_games.set(games),
                Err(error) => registry_error.set(Some(error)),
            }
            if completes_reinstall {
                retained_reinstalling.set(None);
                retained_reinstall_refresh.set(None);
            }
            registry_loading.set(false);
        });
    });

    Effect::new(move |_| {
        let requested_generation = refresh_generation.get();
        let recovery_generation = recovery_completed.get();
        if availability == LibraryAvailability::DesktopOnly
            || recovery_generation != Some(requested_generation)
        {
            lifecycle_loading.set(false);
            return;
        }
        lifecycle_loading.set(true);
        lifecycle_error.set(None);
        spawn_local(async move {
            let result = invoke_list_installation_lifecycle_issues().await;
            if refresh_generation.get_untracked() != requested_generation
                || recovery_completed.get_untracked() != Some(requested_generation)
            {
                return;
            }
            match result {
                Ok(issues) => lifecycle_issues.set(issues),
                Err(error) => lifecycle_error.set(Some(error)),
            }
            lifecycle_loading.set(false);
        });
    });

    Effect::new(move |_| {
        let requested_generation = refresh_generation.get();
        if availability == LibraryAvailability::DesktopOnly {
            retained_loading.set(false);
            return;
        }
        retained_loading.set(true);
        retained_error.set(None);
        spawn_local(async move {
            let result = invoke_list_retained_packages().await;
            if refresh_generation.get_untracked() != requested_generation {
                return;
            }
            match result {
                Ok(packages) => retained_packages.set(packages),
                Err(error) => retained_error.set(Some(error)),
            }
            retained_loading.set(false);
        });
    });

    Effect::new(move |_| {
        if availability == LibraryAvailability::DesktopOnly {
            return;
        }
        spawn_local(async move {
            match invoke_get_platform_info().await {
                Ok(value) => platform.set(Some(value)),
                Err(_) => platform_error.set(true),
            }
            platform_loading.set(false);
        });
    });

    let entries = Signal::derive(move || {
        reconcile_installed_entries(installed_games.get(), &marketplace.listings.get())
    });
    let saved_entries = Signal::derive(move || {
        saved_games
            .get()
            .into_iter()
            .map(|saved| {
                let listing = marketplace.listings.get().into_iter().find(|listing| {
                    listing_coordinate(listing).as_deref() == Some(saved.game_coordinate.as_str())
                });
                (saved, listing)
            })
            .collect::<Vec<_>>()
    });

    Effect::new(move |_| {
        let active_npub = auth.npub.get();
        let current_entries = entries.get();
        ownership_generation.update(|generation| *generation = generation.wrapping_add(1));
        let generation = ownership_generation.get_untracked();
        ownership.set(HashMap::new());
        ownership_errors.set(HashSet::new());

        let Some(buyer_npub) = active_npub else {
            ownership_loading.set(HashSet::new());
            return;
        };
        let mut candidates = current_entries
            .into_iter()
            .filter_map(|entry| {
                let listing = entry.listing?;
                Some((
                    entry.installed.game_coordinate,
                    listing.publisher_npub,
                    listing.id,
                ))
            })
            .collect::<Vec<_>>();
        let mut seen_coordinates = candidates
            .iter()
            .map(|(coordinate, _, _)| coordinate.clone())
            .collect::<HashSet<_>>();
        for (saved, listing) in saved_entries.get() {
            let Some(listing) = listing else {
                continue;
            };
            if seen_coordinates.insert(saved.game_coordinate.clone()) {
                candidates.push((saved.game_coordinate, listing.publisher_npub, listing.id));
            }
        }
        candidates.sort_by(|left, right| left.0.cmp(&right.0));
        ownership_loading.set(
            candidates
                .iter()
                .map(|(coordinate, _, _)| coordinate.clone())
                .collect(),
        );

        spawn_local(async move {
            for (coordinate, publisher_npub, listing_id) in candidates {
                let result =
                    invoke_get_listing_ownership(buyer_npub.clone(), publisher_npub, listing_id)
                        .await;
                if ownership_generation.get_untracked() != generation
                    || auth.npub.get_untracked().as_deref() != Some(buyer_npub.as_str())
                {
                    return;
                }
                ownership_loading.update(|loading| {
                    loading.remove(&coordinate);
                });
                match result {
                    Ok(is_owned) => {
                        ownership.update(|values| {
                            values.insert((buyer_npub.clone(), coordinate), is_owned);
                        });
                    }
                    Err(_) => {
                        ownership_errors.update(|errors| {
                            errors.insert(coordinate);
                        });
                    }
                }
            }
        });
    });
    let installed_coordinates = Signal::derive(move || {
        installed_games
            .get()
            .into_iter()
            .map(|game| game.game_coordinate)
            .collect::<HashSet<_>>()
    });
    let visible_installed_entries = Signal::derive(move || {
        entries
            .get()
            .into_iter()
            .filter(|entry| entry_matches_search(entry, &search.get()))
            .filter(|entry| {
                let coordinate = &entry.installed.game_coordinate;
                installed_entry_matches_filter(
                    entry,
                    library_filter.get(),
                    listing_compatibility(entry.listing.as_ref(), platform.get().as_ref()),
                    ownership_errors.get().contains(coordinate),
                    library_now.get(),
                )
            })
            .collect::<Vec<_>>()
    });
    let visible_saved_entries = Signal::derive(move || {
        let installed = installed_coordinates.get();
        saved_entries
            .get()
            .into_iter()
            .filter(|(saved, listing)| {
                saved_entry_matches_search(saved, listing.as_ref(), &search.get())
            })
            .filter(|(saved, listing)| {
                saved_entry_matches_filter(
                    saved,
                    listing.as_ref(),
                    &installed,
                    library_filter.get(),
                    listing_compatibility(listing.as_ref(), platform.get().as_ref()),
                    library_now.get(),
                )
            })
            .collect::<Vec<_>>()
    });
    let tabs = vec![
        PageTabItem::local("all", "All"),
        PageTabItem::local("saved", "Saved"),
        PageTabItem::local("installed", "Installed"),
        PageTabItem::local("not-installed", "Not installed"),
        PageTabItem::local("timed-access", "Timed access"),
        PageTabItem::local("needs-attention", "Needs attention"),
    ];
    let selected_filter = Signal::derive(move || library_filter.get().id().to_string());
    let select_filter = Callback::new(move |id: String| {
        library_filter.set(LibraryFilter::from_id(&id));
    });
    let retry_refresh = Callback::new(move |_| {
        if retained_reinstalling.get_untracked().is_some() {
            return;
        }
        refresh_generation.update(|generation| *generation = generation.wrapping_add(1));
    });
    let retry_refresh_error = Callback::new(move |_: leptos::ev::MouseEvent| {
        if retained_reinstalling.get_untracked().is_some() {
            return;
        }
        refresh_generation.update(|generation| *generation = generation.wrapping_add(1));
    });
    let open_uninstall = Callback::new(move |installed: InstalledGame| {
        if retained_reinstalling.get_untracked().is_some() {
            return;
        }
        retain_package.set(default_retain_package());
        uninstall_sizes.set(None);
        destructive_error.set(None);
        destructive_failure.set(None);
        uninstall_sizes_loading.set(true);
        destructive_action_generation.set(Some(refresh_generation.get_untracked()));
        destructive_action.set(Some(LibraryDestructiveAction::Uninstall(installed.clone())));
        spawn_local(async move {
            let result = invoke_get_uninstall_sizes(installed.installation_id.clone()).await;
            let still_current = destructive_action.get_untracked().is_some_and(|action| {
                matches!(action, LibraryDestructiveAction::Uninstall(current) if current.installation_id == installed.installation_id)
            });
            if !still_current {
                return;
            }
            match result {
                Ok(sizes) => uninstall_sizes.set(Some(sizes)),
                Err(error) => {
                    destructive_error.set(Some(format!("Could not load uninstall sizes: {error}")))
                }
            }
            uninstall_sizes_loading.set(false);
        });
    });
    let open_failed_cleanup = Callback::new(move |issue: InstallationLifecycleIssue| {
        if retained_reinstalling.get_untracked().is_some()
            || lifecycle_action_unavailable(
                registry_loading.get_untracked(),
                registry_error.get_untracked().is_some(),
                lifecycle_loading.get_untracked(),
                lifecycle_error.get_untracked().is_some(),
            )
        {
            return;
        }
        destructive_error.set(None);
        destructive_failure.set(None);
        destructive_action_generation.set(Some(refresh_generation.get_untracked()));
        destructive_action.set(Some(LibraryDestructiveAction::RemoveFailed(issue)));
    });
    let open_resume_uninstall = Callback::new(move |issue: InstallationLifecycleIssue| {
        if retained_reinstalling.get_untracked().is_some()
            || lifecycle_action_unavailable(
                registry_loading.get_untracked(),
                registry_error.get_untracked().is_some(),
                lifecycle_loading.get_untracked(),
                lifecycle_error.get_untracked().is_some(),
            )
        {
            return;
        }
        destructive_error.set(None);
        destructive_failure.set(None);
        destructive_action_generation.set(Some(refresh_generation.get_untracked()));
        destructive_action.set(Some(LibraryDestructiveAction::ResumeUninstall(issue)));
    });
    let open_retained_delete = Callback::new(move |package: RetainedPackage| {
        if retained_reinstalling.get_untracked().is_some() {
            return;
        }
        destructive_error.set(None);
        destructive_failure.set(None);
        destructive_action_generation.set(Some(refresh_generation.get_untracked()));
        destructive_action.set(Some(LibraryDestructiveAction::DeleteRetained(package)));
    });
    let reinstall_retained = Callback::new(
        move |(game_coordinate, package_id): (String, String)| {
            if retained_reinstalling.get_untracked().is_some()
                || destructive_busy.get_untracked()
                || destructive_action.get_untracked().is_some()
            {
                return;
            }
            retained_reinstall_error.set(None);
            retained_reinstalling.set(Some(game_coordinate.clone()));
            spawn_local(async move {
                let result =
                    invoke_reinstall_retained_package(game_coordinate.clone(), package_id).await;
                let succeeded = result.is_ok();
                match result {
                    Ok(()) => action_message.set(Some(
                        "Game reinstalled entirely from the retained package. No network or account authorization was used."
                            .to_string(),
                    )),
                    Err(error) => retained_reinstall_error.set(Some((game_coordinate, error))),
                }
                if succeeded {
                    let requested_generation = refresh_generation.get_untracked().wrapping_add(1);
                    retained_reinstall_refresh.set(Some(requested_generation));
                    refresh_generation.set(requested_generation);
                } else {
                    retained_reinstalling.set(None);
                    refresh_generation
                        .update(|generation| *generation = generation.wrapping_add(1));
                }
            });
        },
    );
    let marketplace_listings_for_cleanup = marketplace.listings;
    let confirm_destructive = {
        let on_open_listing = on_open_listing.clone();
        Callback::new(move |()| {
            if destructive_busy.get_untracked() || retained_reinstalling.get_untracked().is_some() {
                return;
            }
            let Some(action) = destructive_action.get_untracked() else {
                return;
            };
            if destructive_action_generation.get_untracked()
                != Some(refresh_generation.get_untracked())
            {
                destructive_action.set(None);
                destructive_action_generation.set(None);
                return;
            }
            let authoritative_state_unavailable = match &action {
                LibraryDestructiveAction::Uninstall(_) => {
                    registry_loading.get_untracked() || registry_error.get_untracked().is_some()
                }
                LibraryDestructiveAction::RemoveFailed(_)
                | LibraryDestructiveAction::ResumeUninstall(_) => lifecycle_action_unavailable(
                    registry_loading.get_untracked(),
                    registry_error.get_untracked().is_some(),
                    lifecycle_loading.get_untracked(),
                    lifecycle_error.get_untracked().is_some(),
                ),
                LibraryDestructiveAction::DeleteRetained(_) => {
                    retained_loading.get_untracked()
                        || retained_error.get_untracked().is_some()
                        || retained_reinstalling.get_untracked().is_some()
                }
            };
            if authoritative_state_unavailable {
                return;
            }
            if matches!(action, LibraryDestructiveAction::Uninstall(_))
                && (uninstall_sizes_loading.get_untracked()
                    || uninstall_sizes.get_untracked().is_none())
            {
                return;
            }
            destructive_busy.set(true);
            destructive_error.set(None);
            let retain = retain_package.get_untracked();
            let command = storage_command(&action, retain);
            let matching_listing = match &action {
                LibraryDestructiveAction::RemoveFailed(issue) => listing_for_coordinate(
                    &marketplace_listings_for_cleanup.get_untracked(),
                    &issue.game_coordinate,
                ),
                _ => None,
            };
            let on_open_listing = on_open_listing.clone();
            spawn_local(async move {
                let result = match command {
                    LibraryStorageCommand::Uninstall {
                        installation_id,
                        retain_package,
                    } => invoke_uninstall_game(installation_id, retain_package).await,
                    LibraryStorageCommand::RemoveFailed { installation_id } => {
                        invoke_remove_failed_installation(installation_id).await
                    }
                    LibraryStorageCommand::ResumeUninstall { installation_id } => {
                        invoke_resume_uninstall(installation_id).await
                    }
                    LibraryStorageCommand::DeleteRetained {
                        game_coordinate,
                        expected_package_id,
                    } => invoke_delete_retained_package(game_coordinate, expected_package_id).await,
                };
                match result {
                    Ok(()) => {
                        destructive_action.set(None);
                        destructive_action_generation.set(None);
                        uninstall_sizes.set(None);
                        refresh_generation
                            .update(|generation| *generation = generation.wrapping_add(1));
                        match action {
                            LibraryDestructiveAction::Uninstall(_) => action_message.set(Some(
                                if retain {
                                    "Game uninstalled. The retained package remains available for an offline reinstall; account ownership was not changed."
                                } else {
                                    "Game and retained package deleted from this device; account ownership was not changed."
                                }
                                .to_string(),
                            )),
                            LibraryDestructiveAction::DeleteRetained(_) => action_message.set(Some(
                                "Retained package deleted. Installation and ownership records were not changed."
                                    .to_string(),
                            )),
                            LibraryDestructiveAction::ResumeUninstall(_) => action_message.set(Some(
                                "Interrupted uninstall completed. The installation root was removed and the retained package was conservatively kept; account ownership was not changed."
                                    .to_string(),
                            )),
                            LibraryDestructiveAction::RemoveFailed(_) => {
                                if let Some(listing) = matching_listing {
                                    on_open_listing.run(listing);
                                } else {
                                    action_message.set(Some(
                                        "Failed installation cleaned up. Store metadata is unavailable in the current discovery window, so Library was refreshed; the retained package was not deleted."
                                            .to_string(),
                                    ));
                                }
                            }
                        }
                    }
                    Err(error) => {
                        destructive_action.set(None);
                        destructive_action_generation.set(None);
                        destructive_error.set(None);
                        uninstall_sizes.set(None);
                        destructive_failure.set(Some(format!(
                            "{error} Library is refreshing authoritative device state before another storage action can be confirmed."
                        )));
                        registry_loading.set(true);
                        lifecycle_loading.set(true);
                        retained_loading.set(true);
                        refresh_generation
                            .update(|generation| *generation = generation.wrapping_add(1));
                    }
                }
                destructive_busy.set(false);
            });
        })
    };

    view! {
        <section class="arc-library-page">
            <PageHeader
                title="Library".to_string()
                action=view! {
                    <span class="arc-library-summary">
                        {move || format!("{} saved · {} installed", saved_games.get().len(), installed_games.get().len())}
                    </span>
                }.into_any()
            />

            {if availability == LibraryAvailability::DesktopOnly {
                view! {
                    <EmptyState
                        title="Desktop Library unavailable on web"
                        description="Installed-game records and native filesystem information are available only in the Arcadestr desktop application."
                        icon="desktop_windows"
                        layout=FeedbackLayout::Panel
                    />
                }.into_any()
            } else {
                view! {
                    <PageTabs
                        items=tabs
                        selected=selected_filter
                        on_select=select_filter
                        aria_label="Filter library".to_string()
                        semantics=PageTabSemantics::Filter
                    />

                    <section class="arc-library-toolbar" aria-label="Library controls">
                        <label class="arc-library-search" for="library-search">
                            <span class="material-symbols-outlined" aria-hidden="true">"search"</span>
                            <span class="sr-only">"Search library"</span>
                            <input
                                id="library-search"
                                type="search"
                                placeholder="Search title, publisher, version, platform, or coordinate"
                                bind:value=search
                            />
                        </label>
                        <button class="v2-btn-secondary" on:click=move |_| retry_refresh.run(()) disabled=move || registry_loading.get() || saved_loading.get() || retained_reinstalling.get().is_some()>
                            {move || if registry_loading.get() || saved_loading.get() { "Refreshing..." } else { "Refresh" }}
                        </button>
                    </section>

                    <p class="arc-library-result-count" role="status" aria-live="polite">
                        {move || format!("{} library record(s) shown", visible_saved_entries.get().len() + visible_installed_entries.get().len())}
                    </p>

                    <Show when=move || auth.npub.get().is_none()>
                        <div class="arc-library-notice">
                            <span class="material-symbols-outlined" aria-hidden="true">"person_off"</span>
                            <div><strong>"Account library unavailable while signed out"</strong><p>"Device installation records remain visible. Sign in to load account-saved games and ownership."</p></div>
                        </div>
                    </Show>
                    <Show when=move || marketplace.error.get().is_some() && (!saved_games.get().is_empty() || !installed_games.get().is_empty())>
                        <div class="arc-library-notice">
                            <span class="material-symbols-outlined" aria-hidden="true">"cloud_off"</span>
                            <div><strong>"Store metadata is partial"</strong><p>"Local and account records remain visible, but some artwork, titles, access policy, and compatibility details may be unavailable."</p><p>{move || marketplace.error.get().unwrap_or_default()}</p></div>
                        </div>
                    </Show>
                    <Show when=move || registry_error.get().is_some() && !installed_games.get().is_empty()>
                        <div class="arc-library-notice">
                            <span class="material-symbols-outlined" aria-hidden="true">"database"</span>
                            <div><strong>"Local registry refresh failed"</strong><p>"Previously loaded device records remain visible below."</p></div>
                            <button class="v2-btn-secondary" on:click=move |_| retry_refresh.run(()) disabled=move || registry_loading.get()>"Retry"</button>
                        </div>
                    </Show>
                    <Show when=move || saved_error.get().is_some() && (!saved_games.get().is_empty() || !installed_games.get().is_empty())>
                        <div class="arc-library-notice">
                            <span class="material-symbols-outlined" aria-hidden="true">"sync_problem"</span>
                            <div><strong>"Account library refresh failed"</strong><p>"Previously loaded account records remain visible below."</p></div>
                            <button class="v2-btn-secondary" on:click=move |_| retry_refresh.run(()) disabled=move || saved_loading.get()>"Retry"</button>
                        </div>
                    </Show>
                    <Show when=move || platform_loading.get() || platform_error.get()>
                        <div class="arc-library-notice" role="status">
                            <span class="material-symbols-outlined" aria-hidden="true">"devices"</span>
                            <div><strong>{move || if platform_loading.get() { "Checking device compatibility" } else { "Device compatibility unavailable" }}</strong><p>"Unknown compatibility does not imply that an artifact is supported."</p></div>
                        </div>
                    </Show>
                    <Show when=move || action_message.get().is_some()>
                        <div class="arc-library-notice" role="status">
                            <span class="material-symbols-outlined" aria-hidden="true">"info"</span>
                            <div><strong>"Device storage updated"</strong><p>{move || action_message.get().unwrap_or_default()}</p></div>
                            <button class="v2-btn-secondary" on:click=move |_| action_message.set(None)>"Dismiss message"</button>
                        </div>
                    </Show>
                    <Show when=move || destructive_failure.get().is_some()>
                        <div class="arc-library-notice" role="alert">
                            <span class="material-symbols-outlined" aria-hidden="true">"warning"</span>
                            <div><strong>"Device storage action failed"</strong><p>{move || destructive_failure.get().unwrap_or_default()}</p></div>
                            <button class="v2-btn-secondary" on:click=move |_| destructive_failure.set(None)>"Dismiss error"</button>
                        </div>
                    </Show>
                    <Show when=move || lifecycle_error.get().is_some()>
                        <div class="arc-library-notice" role="status">
                            <span class="material-symbols-outlined" aria-hidden="true">"warning"</span>
                            <div><strong>"Installation recovery records unavailable"</strong><p>"Ready installations and retained packages remain independently available."</p></div>
                            <button class="v2-btn-secondary" on:click=move |_| retry_refresh.run(()) disabled=move || lifecycle_loading.get()>"Retry recovery records"</button>
                        </div>
                    </Show>
                    <Show when=move || retained_error.get().is_some()>
                        <div class="arc-library-notice" role="status">
                            <span class="material-symbols-outlined" aria-hidden="true">"inventory_2"</span>
                            <div><strong>"Retained packages unavailable"</strong><p>"Installation rows remain available. No package was automatically deleted."</p></div>
                            <button class="v2-btn-secondary" on:click=move |_| retry_refresh.run(()) disabled=move || retained_loading.get()>"Retry retained packages"</button>
                        </div>
                    </Show>

                    {move || {
                        let no_saved_data = saved_games.get().is_empty();
                        let no_installed_data = installed_games.get().is_empty();
                        let visible_count = visible_saved_entries.get().len() + visible_installed_entries.get().len();
                        let content_state = library_content_state(
                            visible_count,
                            !no_saved_data,
                            !no_installed_data,
                            auth.npub.get().is_some() && saved_loading.get() && no_saved_data,
                            registry_loading.get() && no_installed_data,
                            saved_error.get().is_some(),
                            registry_error.get().is_some(),
                        );
                        if content_state == LibraryContentState::Loading {
                            view! {
                                <LoadingState
                                    title="Loading Library"
                                    description="Reading device installation records and account-saved games without hiding partial results.".to_string()
                                    layout=FeedbackLayout::Panel
                                />
                            }.into_any()
                        } else if content_state == LibraryContentState::BlockingError {
                            let account_failed = saved_error.get().is_some();
                            let local_failed = registry_error.get().is_some();
                            let (title, message) = match (account_failed, local_failed) {
                                (true, true) => ("Library records unavailable", "Neither account-saved games nor the local installation registry could be loaded."),
                                (true, false) => ("Account library unavailable", "Account-saved games could not be loaded and no local installation records are available."),
                                (false, true) => ("Installation registry unavailable", "The local installation registry could not be read and no account-saved games are available."),
                                (false, false) => ("Library records unavailable", "No Library records are currently available."),
                            };
                            view! {
                                <ErrorState
                                    title=title
                                    message=message
                                    on_retry=retry_refresh_error
                                    retry_busy=Signal::derive(move || registry_loading.get())
                                    severity=ErrorSeverity::Recoverable
                                />
                            }.into_any()
                        } else {
                            let saved_rows = visible_saved_entries.get().into_iter().map(|(saved, listing)| {
                                let fallback = parse_game_coordinate(&saved.game_coordinate)
                                    .map(|(_, listing_id)| format!("Saved game: {listing_id}"))
                                    .unwrap_or_else(|| "Saved game".to_string());
                                let title = listing.as_ref().map(|item| item.title.clone()).unwrap_or(fallback);
                                let publisher = listing.as_ref().map(|item| item.stall_name.clone().filter(|value| !value.trim().is_empty()).unwrap_or_else(|| npub_fallback_label(&item.publisher_npub))).unwrap_or_else(|| "Publisher unavailable".to_string());
                                let artwork = artwork_state_from_url(listing.as_ref().and_then(|item| valid_cover_url(&item.images)));
                                let compatibility = listing_compatibility(listing.as_ref(), platform.get().as_ref());
                                let compatibility_state = compatibility_chip(compatibility);
                                let detail_listing = listing.clone();
                                let saved_is_installed = installed_coordinates.get().contains(&saved.game_coordinate);
                                let active_npub = auth.npub.get();
                                let durable_owned = ownership_for_active_account(&ownership.get(), active_npub.as_deref(), &saved.game_coordinate);
                                let checking_ownership = ownership_loading.get().contains(&saved.game_coordinate);
                                let ownership_failed = ownership_errors.get().contains(&saved.game_coordinate);
                                view! {
                                    <article class="arc-library-row">
                                        <div class="arc-library-art"><GameArtwork title=title.clone() state=artwork role=ArtworkRole::Thumbnail /></div>
                                        <div class="arc-library-row-copy"><h2>{title}</h2><p>{publisher}</p><small>{if saved_is_installed { "Saved to the active account · Installed on this device" } else { "Saved to the active account · Not installed on this device" }}</small></div>
                                        <div class="arc-library-row-states" aria-label="Game states">
                                            {listing.as_ref().map(|listing| { let (label, variant) = access_chip(listing, library_now.get()); view! { <StatusChip label=label variant=variant icon=None size=StatusChipSize::Compact /> } })}
                                            {if durable_owned {
                                                view! { <StatusChip label="Owned" variant=StatusChipVariant::Owned icon=Some("verified_user") size=StatusChipSize::Compact /> }.into_any()
                                            } else if checking_ownership {
                                                view! { <StatusChip label="Checking ownership" variant=StatusChipVariant::Pending icon=None size=StatusChipSize::Compact /> }.into_any()
                                            } else if ownership_failed {
                                                view! { <StatusChip label="Ownership unknown" variant=StatusChipVariant::Warning icon=None size=StatusChipSize::Compact /> }.into_any()
                                            } else { view! { <></> }.into_any() }}
                                            <StatusChip label="Saved" variant=StatusChipVariant::Neutral icon=Some("bookmark") size=StatusChipSize::Compact />
                                            {saved_is_installed.then(|| view! { <StatusChip label="Installed" variant=StatusChipVariant::Installed icon=Some("download_done") size=StatusChipSize::Compact /> })}
                                            <StatusChip label=compatibility_state.0 variant=compatibility_state.1 icon=None size=StatusChipSize::Compact />
                                        </div>
                                        <div class="arc-library-row-action">
                                            {detail_listing.map(|listing| { let selected = listing.clone(); view! { <button class="v2-btn-primary" on:click=move |_| on_open_listing.run(selected.clone())>"View details"</button> } })}
                                        </div>
                                    </article>
                                }
                            }).collect::<Vec<_>>();

                            let installed_rows = visible_installed_entries.get().into_iter().map(|entry| {
                                let listing = entry.listing.clone();
                                let title = fallback_title(&entry);
                                let publisher = publisher_label(&entry);
                                let artwork = artwork_state_from_url(entry.listing.as_ref().and_then(|listing| valid_cover_url(&listing.images)));
                                let coordinate = entry.installed.game_coordinate.clone();
                                let active_npub = auth.npub.get();
                                let durable_owned = ownership_for_active_account(&ownership.get(), active_npub.as_deref(), &coordinate);
                                let checking_ownership = ownership_loading.get().contains(&coordinate);
                                let ownership_failed = ownership_errors.get().contains(&coordinate);
                                let compatibility = listing_compatibility(entry.listing.as_ref(), platform.get().as_ref());
                                let compatibility_state = compatibility_chip(compatibility);
                                let installed_meta = format_installed_at(entry.installed.installed_at);
                                let installation_directory = installation_directory_label(&entry.installed.installation_root);
                                let stored_coordinate = if coordinate.trim().is_empty() { "Malformed or unavailable".to_string() } else { coordinate.clone() };
                                let installed_for_uninstall = entry.installed.clone();
                                let launchable = entry.installed.launchable;
                                let (installed_label, installed_variant) = installed_status(launchable);
                                let installed_icon = if launchable { "download_done" } else { "warning" };
                                view! {
                                    <article class="arc-library-row arc-library-row-installed">
                                        <div class="arc-library-art"><GameArtwork title=title.clone() state=artwork role=ArtworkRole::Thumbnail /></div>
                                        <div class="arc-library-row-copy">
                                            <h2>{title}</h2><p>{publisher}</p><small>{installed_meta}</small>
                                            {if entry.listing.is_none() { view! { <span class="arc-library-row-warning">"Current store metadata was not loaded in the discovery window."</span> }.into_any() } else { view! { <></> }.into_any() }}
                                            {if entry.malformed_coordinate { view! { <span class="arc-library-row-warning">"Stored listing coordinate is malformed."</span> }.into_any() } else { view! { <></> }.into_any() }}
                                            <details class="arc-library-technical"><summary>"Local record details"</summary><dl><div><dt>"Installation directory"</dt><dd>{installation_directory}</dd></div><div><dt>"Listing coordinate"</dt><dd>{stored_coordinate}</dd></div></dl></details>
                                        </div>
                                        <div class="arc-library-row-states" aria-label="Game states">
                                            {entry.listing.as_ref().map(|listing| { let (label, variant) = access_chip(listing, library_now.get()); view! { <StatusChip label=label variant=variant icon=None size=StatusChipSize::Compact /> } })}
                                            {if durable_owned {
                                                view! { <StatusChip label="Owned" variant=StatusChipVariant::Owned icon=Some("verified_user") size=StatusChipSize::Compact /> }.into_any()
                                            } else if checking_ownership {
                                                view! { <StatusChip label="Checking ownership" variant=StatusChipVariant::Pending icon=None size=StatusChipSize::Compact /> }.into_any()
                                            } else if ownership_failed {
                                                view! { <StatusChip label="Ownership unknown" variant=StatusChipVariant::Warning icon=None size=StatusChipSize::Compact /> }.into_any()
                                            } else { view! { <></> }.into_any() }}
                                            {if launchable {
                                                view! { <></> }.into_any()
                                            } else {
                                                view! { <span class="arc-library-row-warning" role="status">"No recorded launch profile resolves to this installation. The verified files remain installed and can be uninstalled or reinstalled."</span> }.into_any()
                                            }}
                                            <StatusChip label=installed_label variant=installed_variant icon=Some(installed_icon) size=StatusChipSize::Compact />
                                            <StatusChip label=compatibility_state.0 variant=compatibility_state.1 icon=None size=StatusChipSize::Compact />
                                        </div>
                                        <div class="arc-library-row-action">
                                            <crate::ui_v2::components::InstallationLauncher installation_id=entry.installed.installation_id.clone() />
                                            {match listing { Some(listing) => { let selected = listing.clone(); view! { <button class="v2-btn-primary" on:click=move |_| on_open_listing.run(selected.clone())>"View details"</button> }.into_any() }, None => view! { <span class="arc-library-action-unavailable">"Details unavailable"</span> }.into_any() }}
                                            <button class="v2-btn-danger" disabled=move || registry_loading.get() || registry_error.get().is_some() || destructive_busy.get() || retained_reinstalling.get().is_some() on:click=move |_| open_uninstall.run(installed_for_uninstall.clone())>"Uninstall game"</button>
                                        </div>
                                    </article>
                                }
                            }).collect::<Vec<_>>();

                            if saved_rows.is_empty() && installed_rows.is_empty() {
                                let (title, description) = library_filter.get().empty_copy();
                                let has_filter = library_filter.get() != LibraryFilter::All || !search.get().trim().is_empty();
                                if has_filter {
                                    view! {
                                        <EmptyState
                                            title=title
                                            description=description
                                            icon="library_books"
                                            primary_action=view! { <button class="v2-btn-secondary" on:click=move |_| { library_filter.set(LibraryFilter::All); search.set(String::new()); }>"Clear filters"</button> }.into_any()
                                            layout=FeedbackLayout::Panel
                                        />
                                    }.into_any()
                                } else {
                                    view! {
                                        <EmptyState title=title description=description icon="library_books" layout=FeedbackLayout::Panel />
                                    }.into_any()
                                }
                            } else {
                                view! {
                                    <div class="arc-library-sections">
                                        {(!saved_rows.is_empty()).then(|| view! { <section><p class="v2-store-kicker">"Account saved"</p><div class="arc-library-list">{saved_rows}</div></section> })}
                                        {(!installed_rows.is_empty()).then(|| view! { <section><p class="v2-store-kicker">"Installed on this device"</p><div class="arc-library-list">{installed_rows}</div><p class="arc-library-device-note">{format!("{INSTALLATION_SCOPE_LABEL}. Installation does not prove account ownership.")}</p></section> })}
                                    </div>
                                }.into_any()
                            }
                        }
                    }}

                    <Show when=move || lifecycle_loading.get() && lifecycle_issues.get().is_empty()>
                        <p class="arc-library-result-count" role="status">"Checking installation recovery records…"</p>
                    </Show>
                    <Show when=move || !lifecycle_issues.get().is_empty()>
                        <section class="arc-library-storage-section" aria-labelledby="library-recovery-title">
                            <div>
                                <p class="v2-store-kicker">"Needs recovery"</p>
                                <h2 id="library-recovery-title" class="sr-only">"Installation lifecycle issues"</h2>
                            </div>
                            <div class="arc-library-list">
                                {move || lifecycle_issues.get().into_iter().map(|issue| {
                                    let listing = listing_for_coordinate(&marketplace.listings.get(), &issue.game_coordinate);
                                    let title = listing.as_ref().map(|listing| listing.title.clone()).unwrap_or_else(|| parse_game_coordinate(&issue.game_coordinate).map(|(_, id)| format!("Installation: {id}")).unwrap_or_else(|| "Installation record".to_string()));
                                    let state_label = match issue.state {
                                        InstallationState::Installing => "Installation busy",
                                        InstallationState::Uninstalling => "Uninstall busy",
                                        InstallationState::Failed => "Installation failed",
                                        InstallationState::Ready => "Ready",
                                    };
                                    let message = lifecycle_issue_message(&issue);
                                    let issue_for_cleanup = issue.clone();
                                    let issue_for_resume = issue.clone();
                                    view! {
                                        <article class="arc-library-row arc-library-storage-row">
                                            <div class="arc-library-storage-icon"><span class="material-symbols-outlined" aria-hidden="true">"build_circle"</span></div>
                                            <div class="arc-library-row-copy">
                                                <h2>{title}</h2>
                                                <p>{state_label}</p>
                                                <small>{message}</small>
                                                <details class="arc-library-technical"><summary>"Recovery record details"</summary><dl><div><dt>"Listing coordinate"</dt><dd>{issue.game_coordinate.clone()}</dd></div><div><dt>"Installation ID"</dt><dd>{issue.installation_id.clone()}</dd></div></dl></details>
                                            </div>
                                            <div class="arc-library-row-states" aria-label="Recovery state">
                                                <StatusChip label=state_label variant=if issue.state == InstallationState::Failed { StatusChipVariant::Error } else { StatusChipVariant::Pending } icon=None size=StatusChipSize::Compact />
                                            </div>
                                            <div class="arc-library-row-action">
                                                {match issue.state {
                                                    InstallationState::Failed => view! { <button class="v2-btn-danger" disabled=move || lifecycle_action_unavailable(registry_loading.get(), registry_error.get().is_some(), lifecycle_loading.get(), lifecycle_error.get().is_some()) || destructive_busy.get() || retained_reinstalling.get().is_some() on:click=move |_| open_failed_cleanup.run(issue_for_cleanup.clone())>{if issue.recovery_discriminator == Some(InstallationRecoveryDiscriminator::FinalRootPresent) { "Delete failed installation files" } else { "Remove failed installation record" }}</button> }.into_any(),
                                                    InstallationState::Uninstalling => view! { <button class="v2-btn-danger" disabled=move || lifecycle_action_unavailable(registry_loading.get(), registry_error.get().is_some(), lifecycle_loading.get(), lifecycle_error.get().is_some()) || destructive_busy.get() || retained_reinstalling.get().is_some() on:click=move |_| open_resume_uninstall.run(issue_for_resume.clone())>"Resume uninstall"</button> }.into_any(),
                                                    InstallationState::Installing => view! { <button class="v2-btn-secondary" on:click=move |_| retry_refresh.run(()) disabled=move || registry_loading.get() || lifecycle_loading.get()>"Retry installation status"</button> }.into_any(),
                                                    InstallationState::Ready => view! { <></> }.into_any(),
                                                }}
                                            </div>
                                        </article>
                                    }
                                }).collect_view()}
                            </div>
                        </section>
                    </Show>

                    <section class="arc-library-storage-section" aria-labelledby="retained-packages-title">
                        <div>
                            <p class="v2-store-kicker">"Retained packages"</p>
                            <h2 id="retained-packages-title" class="sr-only">"Retained installer packages"</h2>
                            <p class="arc-library-device-note">"Packages are device-local copies for offline reinstall. Keeping or deleting one does not grant, remove, or prove account ownership."</p>
                        </div>
                        <Show when=move || retained_loading.get() && retained_packages.get().is_empty()>
                            <p class="arc-library-result-count" role="status">"Loading retained packages…"</p>
                        </Show>
                        <Show when=move || !retained_loading.get() && retained_error.get().is_none() && retained_packages.get().is_empty()>
                            <p class="arc-library-result-count">"No retained packages on this device."</p>
                        </Show>
                        <div class="arc-library-list">
                            {move || retained_packages.get().into_iter().map(|package| {
                                let package_for_delete = package.clone();
                                let reinstall_coordinate = package.game_coordinate.clone();
                                let reinstall_package_id = package.package_id.clone();
                                let eligibility = retained_reinstall_eligibility(
                                    &package.game_coordinate,
                                    &installed_games.get(),
                                    &lifecycle_issues.get(),
                                );
                                let reinstall_busy_coordinate = retained_reinstalling.get();
                                let reinstall_busy = reinstall_busy_coordinate.as_deref() == Some(package.game_coordinate.as_str());
                                let another_reinstall_busy = reinstall_busy_coordinate.is_some() && !reinstall_busy;
                                let device_state_loading = registry_loading.get() || lifecycle_loading.get();
                                let device_state_failed = registry_error.get().is_some() || lifecycle_error.get().is_some();
                                let reinstall_error = retained_reinstall_error.get().and_then(|(coordinate, error)| (coordinate == package.game_coordinate).then_some(error));
                                let title = listing_for_coordinate(&marketplace.listings.get(), &package.game_coordinate).map(|listing| listing.title).unwrap_or_else(|| parse_game_coordinate(&package.game_coordinate).map(|(_, id)| format!("Retained package: {id}")).unwrap_or_else(|| "Retained package".to_string()));
                                view! {
                                    <article class="arc-library-row arc-library-storage-row">
                                        <div class="arc-library-storage-icon"><span class="material-symbols-outlined" aria-hidden="true">"inventory_2"</span></div>
                                        <div class="arc-library-row-copy"><h2>{title}</h2><p>{format_size(package.size_bytes)}</p><small>{package.game_coordinate.clone()}</small>{reinstall_error.map(|error| view! { <span class="arc-library-row-warning" role="alert">{format!("Local reinstall failed: {error}")}</span> })}</div>
                                        <div class="arc-library-row-states" aria-label="Package state"><StatusChip label="Offline reinstall package" variant=StatusChipVariant::Neutral icon=None size=StatusChipSize::Compact /></div>
                                        <div class="arc-library-row-action">
                                            {if device_state_loading {
                                                view! { <span class="arc-library-action-unavailable">"Checking device installation state…"</span> }.into_any()
                                            } else if device_state_failed {
                                                view! { <span class="arc-library-action-unavailable">"Device installation state is unavailable; retry Library refresh before reinstalling."</span> }.into_any()
                                            } else if eligibility == RetainedReinstallEligibility::Available {
                                                 view! { <button class="v2-btn-secondary" disabled=another_reinstall_busy || reinstall_busy || destructive_busy.get() || destructive_action.get().is_some() on:click=move |_| reinstall_retained.run((reinstall_coordinate.clone(), reinstall_package_id.clone()))>{if reinstall_busy { "Reinstalling from package…" } else { "Reinstall from package" }}</button> }.into_any()
                                            } else {
                                                view! { <span class="arc-library-action-unavailable">{retained_reinstall_unavailable_message(eligibility).unwrap_or_default()}</span> }.into_any()
                                            }}
                                            <button class="v2-btn-danger" disabled=move || reinstall_busy || retained_reinstalling.get().is_some() || retained_loading.get() || retained_error.get().is_some() || destructive_busy.get() on:click=move |_| open_retained_delete.run(package_for_delete.clone())>"Delete retained package"</button>
                                        </div>
                                    </article>
                                }
                            }).collect_view()}
                        </div>
                    </section>
                }.into_any()
            }}

            <Dialog
                id="library-destructive-dialog"
                open=Signal::derive(move || destructive_action.get().is_some())
                title=Signal::derive(move || match destructive_action.get() {
                    Some(LibraryDestructiveAction::Uninstall(_)) => "Uninstall game".to_string(),
                    Some(LibraryDestructiveAction::ResumeUninstall(_)) => "Resume uninstall".to_string(),
                    Some(LibraryDestructiveAction::RemoveFailed(issue)) if issue.recovery_discriminator == Some(InstallationRecoveryDiscriminator::FinalRootPresent) => "Delete failed installation files".to_string(),
                    Some(LibraryDestructiveAction::RemoveFailed(_)) => "Remove failed installation record".to_string(),
                    Some(LibraryDestructiveAction::DeleteRetained(_)) => "Delete retained package".to_string(),
                    None => String::new(),
                })
                kicker="Device storage"
                width=DialogWidth::Compact
                tone=DialogTone::Destructive
                policy=DialogClosePolicy::BlockedWhileBusy
                dismissal=DialogDismissal::busy_aware()
                busy=Signal::derive(move || destructive_busy.get())
                initial_focus=DialogInitialFocus::Button(destructive_cancel_ref)
                close_label="Close device storage confirmation"
                on_close=UnsyncCallback::new(move |request: DialogCloseRequest| {
                    if request.action == DialogCloseAction::Dismiss {
                        destructive_action.set(None);
                        destructive_action_generation.set(None);
                        destructive_error.set(None);
                        uninstall_sizes.set(None);
                    }
                })
                actions=move || view! {
                    <button node_ref=destructive_cancel_ref type="button" class="v2-btn-secondary" disabled=move || destructive_busy.get() on:click=move |_| {
                        destructive_action.set(None);
                        destructive_action_generation.set(None);
                        destructive_error.set(None);
                        uninstall_sizes.set(None);
                    }>"Cancel"</button>
                    <button type="button" class="v2-btn-danger" disabled=move || destructive_busy.get() || match destructive_action.get() {
                        Some(LibraryDestructiveAction::Uninstall(_)) => registry_loading.get() || registry_error.get().is_some() || uninstall_sizes_loading.get() || uninstall_sizes.get().is_none(),
                        Some(LibraryDestructiveAction::RemoveFailed(_)) | Some(LibraryDestructiveAction::ResumeUninstall(_)) => lifecycle_action_unavailable(registry_loading.get(), registry_error.get().is_some(), lifecycle_loading.get(), lifecycle_error.get().is_some()),
                        Some(LibraryDestructiveAction::DeleteRetained(_)) => retained_loading.get() || retained_error.get().is_some() || retained_reinstalling.get().is_some(),
                        None => true,
                    } on:click=move |_| confirm_destructive.run(())>
                        {move || if destructive_busy.get() { "Working…" } else { match destructive_action.get() {
                            Some(LibraryDestructiveAction::Uninstall(_)) => "Uninstall game",
                            Some(LibraryDestructiveAction::ResumeUninstall(_)) => "Resume uninstall",
                            Some(LibraryDestructiveAction::RemoveFailed(issue)) if issue.recovery_discriminator == Some(InstallationRecoveryDiscriminator::FinalRootPresent) => "Delete files and record",
                            Some(LibraryDestructiveAction::RemoveFailed(_)) => "Remove failed record",
                            Some(LibraryDestructiveAction::DeleteRetained(_)) => "Delete retained package",
                            None => "Confirm",
                        } }}
                    </button>
                }
            >
                {move || destructive_action.get().map(|action| match action {
                    LibraryDestructiveAction::Uninstall(_) => view! {
                        <p>"Uninstalling removes the playable installation from this device. It does not change account ownership."</p>
                        {if uninstall_sizes_loading.get() {
                            view! { <p class="arc-dialog-status" role="status">"Calculating device storage…"</p> }.into_any()
                        } else if let Some(sizes) = uninstall_sizes.get() {
                            view! { <dl class="arc-library-size-summary"><div><dt>"Installed game"</dt><dd>{format_size(sizes.installation_size_bytes)}</dd></div><div><dt>"Retained package"</dt><dd>{sizes.retained_package_size_bytes.map(format_size).unwrap_or_else(|| "None".to_string())}</dd></div></dl> }.into_any()
                        } else {
                            view! { <p class="arc-dialog-alert" role="alert">"Sizes must load before uninstall can continue."</p> }.into_any()
                        }}
                        <label class="arc-library-retention-choice"><input type="checkbox" checked=move || retain_package.get() on:change=move |event| retain_package.set(event_target_checked(&event)) /><span><strong>"Keep retained package for offline reinstall"</strong><small>"Recommended. Reinstall uses the previously validated device-local snapshot without checking current account or relay access; the package does not prove ownership."</small></span></label>
                    }.into_any(),
                    LibraryDestructiveAction::ResumeUninstall(_) => view! {
                        <p>"Resume the interrupted uninstall and permanently remove the installation root from this device?"</p>
                        <p class="arc-dialog-alert">"Removing the installation root is destructive and cannot be undone."</p>
                        <p class="arc-dialog-note">"The previous package-discard choice was not persisted. Resumed uninstall conservatively keeps the retained package for offline reinstall. Account ownership is not changed."</p>
                    }.into_any(),
                    LibraryDestructiveAction::RemoveFailed(issue) => view! {
                        <p>{recovery_message(issue.recovery_discriminator)}</p>
                        <p class="arc-dialog-note">"Cleanup never deletes a retained installer package and never changes account ownership. If store metadata is available, Arcadestr opens the matching game detail afterward so the buyer can retry."</p>
                    }.into_any(),
                    LibraryDestructiveAction::DeleteRetained(package) => view! {
                        <p>{format!("Delete the {} retained package from this device?", format_size(package.size_bytes))}</p>
                        <p class="arc-dialog-note">"This deletes only the selected offline-reinstall package. Any installation record and account ownership remain unchanged."</p>
                    }.into_any(),
                })}
                <Show when=move || destructive_error.get().is_some()><p class="arc-dialog-alert" role="alert">{move || destructive_error.get().unwrap_or_default()}</p></Show>
            </Dialog>
        </section>
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::nips::nip19::ToBech32;

    fn listing(id: &str) -> GameListing {
        let publisher_npub = nostr::Keys::generate()
            .public_key()
            .to_bech32()
            .expect("generated public key should encode");
        serde_json::from_value(serde_json::json!({
            "id": id,
            "source": "nip99_listing",
            "title": "Neon Game",
            "description": "Description",
            "publisher_npub": publisher_npub,
            "created_at": 100
        }))
        .expect("listing fixture should deserialize")
    }

    fn installed(coordinate: String, installed_at: i64) -> InstalledGame {
        InstalledGame {
            installation_id: "installation-1".to_string(),
            game_coordinate: coordinate,
            installation_root: "/games/0123456789abcdef0123456789abcdef".to_string(),
            package_digest: "ab".repeat(32),
            package_format: "zip".to_string(),
            state: InstallationState::Ready,
            recovery_discriminator: None,
            retained_package: true,
            launchable: true,
            installed_at,
        }
    }

    #[test]
    fn installed_record_matches_listing_only_by_stable_coordinate() {
        let listing = listing("game-a");
        let coordinate = listing_coordinate(&listing).expect("valid coordinate");
        let entries = reconcile_installed_entries(vec![installed(coordinate, 1)], &[listing]);
        assert!(entries[0].listing.is_some());
    }

    #[test]
    fn ready_installation_without_an_available_profile_is_not_launchable() {
        assert_eq!(
            installed_status(false),
            ("Installed, not launchable", StatusChipVariant::Warning)
        );
        assert_eq!(
            installed_status(true),
            ("Installed", StatusChipVariant::Installed)
        );
    }

    #[test]
    fn only_authoritative_refresh_completes_retained_reinstall() {
        assert!(!completes_retained_reinstall_refresh(Some(4), 3));
        assert!(completes_retained_reinstall_refresh(Some(4), 4));
        assert!(!completes_retained_reinstall_refresh(None, 4));
    }

    #[test]
    fn similar_title_never_causes_unsafe_merge() {
        let installed_listing = listing("installed-id");
        let coordinate = listing_coordinate(&installed_listing).expect("valid coordinate");
        let same_title_different_id = listing("different-id");
        let entries =
            reconcile_installed_entries(vec![installed(coordinate, 1)], &[same_title_different_id]);
        assert!(entries[0].listing.is_none());
    }

    #[test]
    fn non_nip99_listing_never_matches_installed_coordinate() {
        let mut value = listing("game-a");
        let coordinate = listing_coordinate(&value).expect("valid coordinate");
        value.source = ListingSource::Nip15Product;
        let entries = reconcile_installed_entries(vec![installed(coordinate, 1)], &[value]);
        assert!(entries[0].listing.is_none());
    }

    #[test]
    fn equal_timestamp_replacements_use_lower_event_id_in_both_orders() {
        let higher = {
            let mut value = listing("game-a");
            value.event_id = Some("bbb".into());
            value.title = "Higher event id".into();
            value
        };
        let lower = {
            let mut value = higher.clone();
            value.event_id = Some("aaa".into());
            value.title = "Lower event id".into();
            value
        };
        let coordinate = listing_coordinate(&higher).expect("valid coordinate");
        for candidates in [
            vec![higher.clone(), lower.clone()],
            vec![lower.clone(), higher.clone()],
        ] {
            let entries =
                reconcile_installed_entries(vec![installed(coordinate.clone(), 1)], &candidates);
            assert_eq!(
                entries[0]
                    .listing
                    .as_ref()
                    .map(|value| value.title.as_str()),
                Some("Lower event id")
            );
        }
    }

    #[test]
    fn malformed_publisher_key_is_rejected() {
        assert!(parse_game_coordinate(&format!("30402:{}:game", "z".repeat(64))).is_none());
    }

    #[test]
    fn missing_metadata_uses_coordinate_fallback_and_preserves_entry() {
        let listing = listing("missing-game");
        let coordinate = listing_coordinate(&listing).expect("valid coordinate");
        let entries = reconcile_installed_entries(vec![installed(coordinate, 1)], &[]);
        assert_eq!(entries.len(), 1);
        assert_eq!(fallback_title(&entries[0]), "Installed game: missing-game");
    }

    #[test]
    fn malformed_installed_metadata_degrades_without_being_hidden() {
        let mut game = installed("not-a-coordinate".to_string(), -1);
        game.installation_root.clear();
        let entries = reconcile_installed_entries(vec![game], &[]);
        assert_eq!(entries.len(), 1);
        assert!(entries[0].malformed_coordinate);
        assert_eq!(fallback_title(&entries[0]), "Installed game");
    }

    #[test]
    fn installation_does_not_imply_durable_ownership() {
        let value = listing("game");
        let presentation = access_presentation(&value, false, 100);
        assert!(!presentation.durable_owned);
        assert_eq!(
            presentation.label,
            "Ownership not confirmed for active account"
        );
    }

    #[test]
    fn ownership_from_another_account_is_never_presented() {
        let coordinate = "30402:publisher:game";
        let ownership = HashMap::from([(("npub-a".into(), coordinate.into()), true)]);
        assert!(ownership_for_active_account(
            &ownership,
            Some("npub-a"),
            coordinate
        ));
        assert!(!ownership_for_active_account(
            &ownership,
            Some("npub-b"),
            coordinate
        ));
        assert!(!ownership_for_active_account(&ownership, None, coordinate));
    }

    #[test]
    fn authoritative_paid_or_entitlement_ownership_is_presented_as_owned() {
        let mut paid = listing("paid");
        paid.price_sats = 2_100;
        paid.is_owned = true;
        assert!(access_presentation(&paid, true, 100).durable_owned);

        let mut entitlement = listing("entitlement");
        entitlement.is_owned = true;
        assert!(access_presentation(&entitlement, true, 100).durable_owned);
    }

    #[test]
    fn public_and_timed_access_are_never_labeled_owned() {
        let mut public = listing("public");
        public.acquisition = AcquisitionPolicy::Public;
        let public_access = access_presentation(&public, true, 100);
        assert!(!public_access.durable_owned);
        assert_eq!(public_access.label, "Public access");

        let mut timed = listing("timed");
        timed.acquisition = AcquisitionPolicy::TimedAccess {
            starts_at: 50,
            ends_at: 150,
        };
        assert!(!access_presentation(&timed, true, 100).durable_owned);
        assert_eq!(
            access_presentation(&timed, false, 150).label,
            "Current timed access ended"
        );
    }

    #[test]
    fn expired_timed_access_does_not_remove_installed_entry() {
        let mut timed = listing("timed");
        timed.acquisition = AcquisitionPolicy::TimedAccess {
            starts_at: 1,
            ends_at: 2,
        };
        let coordinate = listing_coordinate(&timed).expect("valid coordinate");
        let entries = reconcile_installed_entries(vec![installed(coordinate, 10)], &[timed]);
        assert_eq!(entries.len(), 1);
        assert_eq!(
            access_presentation(entries[0].listing.as_ref().expect("listing"), false, 3).label,
            "Current timed access ended"
        );
    }

    #[test]
    fn installed_entries_have_stable_newest_first_order() {
        let first = installed(
            "30402:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa:a".to_string(),
            10,
        );
        let second = installed(
            "30402:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb:b".to_string(),
            20,
        );
        let third = installed(
            "30402:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc:c".to_string(),
            20,
        );
        let entries = reconcile_installed_entries(vec![first, third, second], &[]);
        assert!(entries[0].installed.game_coordinate.ends_with(":b"));
        assert!(entries[1].installed.game_coordinate.ends_with(":c"));
        assert!(entries[2].installed.game_coordinate.ends_with(":a"));
    }

    #[test]
    fn search_is_case_insensitive_and_whitespace_normalized() {
        let listing = listing("search-game");
        let coordinate = listing_coordinate(&listing).expect("valid coordinate");
        let entries = reconcile_installed_entries(vec![installed(coordinate, 1)], &[listing]);
        assert!(entry_matches_search(&entries[0], "  NEON   game "));
        assert!(entry_matches_search(&entries[0], &"ab".repeat(32)));
        assert!(!entry_matches_search(&entries[0], "different"));
        assert!(!entry_matches_search(&entries[0], "/games/artifact.bin"));
        assert_eq!(
            installation_directory_label("/private/home/player/game"),
            "game"
        );
        assert_eq!(
            installation_directory_label(r"C:\Users\player\Arcadestr\0123456789abcdef"),
            "0123456789abcdef"
        );
    }

    #[test]
    fn library_filters_map_only_to_real_saved_and_installed_state() {
        let listing = listing("filter-game");
        let coordinate = listing_coordinate(&listing).expect("valid coordinate");
        let saved = LibraryGame {
            game_coordinate: coordinate.clone(),
            added_at: 10,
        };
        let installed_coordinates = HashSet::from([coordinate.clone()]);

        assert!(saved_entry_matches_filter(
            &saved,
            Some(&listing),
            &installed_coordinates,
            LibraryFilter::Saved,
            LibraryCompatibility::Compatible,
            100,
        ));
        assert!(!saved_entry_matches_filter(
            &saved,
            Some(&listing),
            &installed_coordinates,
            LibraryFilter::NotInstalled,
            LibraryCompatibility::Compatible,
            100,
        ));

        let entries = reconcile_installed_entries(
            vec![installed(coordinate, 20)],
            std::slice::from_ref(&listing),
        );
        assert!(installed_entry_matches_filter(
            &entries[0],
            LibraryFilter::Installed,
            LibraryCompatibility::Compatible,
            false,
            100,
        ));
        assert!(!installed_entry_matches_filter(
            &entries[0],
            LibraryFilter::Saved,
            LibraryCompatibility::Compatible,
            false,
            100,
        ));
    }

    #[test]
    fn loading_is_never_presented_as_an_empty_library() {
        assert_eq!(
            library_content_state(0, false, false, true, true, false, false),
            LibraryContentState::Loading
        );
        assert_eq!(
            library_content_state(0, false, false, false, false, false, false),
            LibraryContentState::Empty
        );
        assert_eq!(
            library_content_state(1, false, true, true, false, false, false),
            LibraryContentState::Ready
        );
    }

    #[test]
    fn registry_failure_is_blocking_only_without_preserved_records() {
        assert_eq!(
            library_content_state(0, false, false, false, false, false, true),
            LibraryContentState::BlockingError
        );
        assert_eq!(
            library_content_state(1, false, true, false, false, false, true),
            LibraryContentState::Ready
        );
    }

    #[test]
    fn timed_and_attention_filters_preserve_access_boundaries() {
        let mut timed = listing("timed-filter");
        timed.acquisition = AcquisitionPolicy::TimedAccess {
            starts_at: 10,
            ends_at: 20,
        };
        let coordinate = listing_coordinate(&timed).expect("valid coordinate");
        let entries = reconcile_installed_entries(vec![installed(coordinate, 1)], &[timed]);

        assert!(installed_entry_matches_filter(
            &entries[0],
            LibraryFilter::TimedAccess,
            LibraryCompatibility::Compatible,
            false,
            15,
        ));
        assert!(installed_entry_matches_filter(
            &entries[0],
            LibraryFilter::NeedsAttention,
            LibraryCompatibility::Compatible,
            false,
            20,
        ));
        assert_eq!(
            access_chip(entries[0].listing.as_ref().expect("listing"), 20),
            ("Timed: expired", StatusChipVariant::Expired)
        );
    }

    #[test]
    fn compatibility_mapping_does_not_infer_from_partial_labels() {
        let mut value = listing("platform-game");
        assert_eq!(
            listing_compatibility(
                Some(&value),
                Some(&PlatformInfo {
                    os: "linux".into(),
                    arch: "x86_64".into(),
                })
            ),
            LibraryCompatibility::Unknown
        );
        value.platforms = vec!["linux-x86_64".to_string()];
        assert_eq!(
            listing_compatibility(Some(&value), None),
            LibraryCompatibility::Unknown
        );
        assert_eq!(
            listing_compatibility(
                Some(&value),
                Some(&PlatformInfo {
                    os: "linux".into(),
                    arch: "x86_64".into(),
                })
            ),
            LibraryCompatibility::Compatible
        );
        assert_eq!(
            listing_compatibility(
                Some(&value),
                Some(&PlatformInfo {
                    os: "linux".into(),
                    arch: "aarch64".into(),
                })
            ),
            LibraryCompatibility::Incompatible
        );
        value.platforms = vec!["WIN64-X64".to_string()];
        assert_eq!(
            listing_compatibility(
                Some(&value),
                Some(&PlatformInfo {
                    os: "windows".into(),
                    arch: "amd64".into(),
                })
            ),
            LibraryCompatibility::Compatible
        );
    }

    #[test]
    fn relay_failure_or_account_change_never_erases_unscoped_installations() {
        let listing = listing("local");
        let coordinate = listing_coordinate(&listing).expect("valid coordinate");
        let local = installed(coordinate, 1);
        assert_eq!(
            reconcile_installed_entries(vec![local.clone()], &[]).len(),
            1
        );

        let mut owned_for_another_state = listing;
        owned_for_another_state.is_owned = true;
        assert_eq!(
            reconcile_installed_entries(vec![local], &[owned_for_another_state]).len(),
            1
        );
        assert_eq!(
            INSTALLATION_SCOPE_LABEL,
            "Installed on this device; not account-scoped"
        );
    }

    #[test]
    fn standalone_web_reports_native_registry_unavailable() {
        assert_eq!(
            target_library_availability(true),
            LibraryAvailability::DesktopOnly
        );
        assert_eq!(
            target_library_availability(false),
            LibraryAvailability::DesktopRegistry
        );
    }

    #[test]
    fn unsupported_lifecycle_actions_are_omitted() {
        let source = include_str!("library.rs");
        for action in [
            concat!("Launch ", "Game"),
            concat!("Update ", "Game"),
            concat!("Verify ", "Files"),
            concat!("Open ", "Folder"),
        ] {
            assert!(
                !source.contains(action),
                "unsupported action present: {action}"
            );
        }
    }

    #[test]
    fn recovery_messages_are_stable_and_do_not_infer_ownership() {
        assert_eq!(
            recovery_message(Some(
                InstallationRecoveryDiscriminator::FinalRootAbsent
            )),
            "Installation was interrupted and no installed files remain. Cleanup removes only the failed installation record; any retained installer package remains separate."
        );
        let present = recovery_message(Some(InstallationRecoveryDiscriminator::FinalRootPresent));
        assert!(present.contains("files were preserved"));
        assert!(present.contains("may be incomplete"));
        assert!(present.contains("not launchable"));
        assert!(!present.to_lowercase().contains("owned"));
    }

    #[test]
    fn uninstall_retention_defaults_to_keep_and_formats_both_sizes() {
        assert!(default_retain_package());
        assert_eq!(format_size(1024), "1.0 KiB");
        let sizes = UninstallSizes {
            installation_size_bytes: 2 * 1024 * 1024,
            retained_package_size_bytes: Some(3 * 1024 * 1024),
        };
        assert_eq!(format_size(sizes.installation_size_bytes), "2.0 MiB");
        assert_eq!(
            sizes
                .retained_package_size_bytes
                .map(format_size)
                .as_deref(),
            Some("3.0 MiB")
        );
    }

    #[test]
    fn failed_cleanup_and_individual_package_delete_are_distinct_actions() {
        let issue = InstallationLifecycleIssue {
            installation_id: "failed-1".into(),
            game_coordinate: "30402:publisher:game".into(),
            installation_root: "/managed/failed-1".into(),
            state: InstallationState::Failed,
            recovery_discriminator: Some(InstallationRecoveryDiscriminator::FinalRootPresent),
            updated_at: 1,
        };
        let package = RetainedPackage {
            game_coordinate: issue.game_coordinate.clone(),
            package_id: "package-1".into(),
            package_path: "/retained/package-1".into(),
            listing_event_id: Some("event-1".into()),
            size_bytes: 42,
        };
        let uninstalling = InstallationLifecycleIssue {
            installation_id: "uninstalling-1".into(),
            game_coordinate: issue.game_coordinate.clone(),
            installation_root: "/managed/uninstalling-1".into(),
            state: InstallationState::Uninstalling,
            recovery_discriminator: None,
            updated_at: 2,
        };
        assert_eq!(
            storage_command(&LibraryDestructiveAction::RemoveFailed(issue), true),
            LibraryStorageCommand::RemoveFailed {
                installation_id: "failed-1".into()
            }
        );
        assert_eq!(
            storage_command(&LibraryDestructiveAction::DeleteRetained(package), true),
            LibraryStorageCommand::DeleteRetained {
                game_coordinate: "30402:publisher:game".into(),
                expected_package_id: "package-1".into(),
            }
        );
        assert_eq!(
            storage_command(
                &LibraryDestructiveAction::ResumeUninstall(uninstalling.clone()),
                false,
            ),
            LibraryStorageCommand::ResumeUninstall {
                installation_id: "uninstalling-1".into()
            }
        );
        assert!(lifecycle_issue_message(&uninstalling).contains("retained package"));
        assert!(lifecycle_issue_message(&uninstalling).contains("Resume uninstall"));
    }

    #[test]
    fn installing_rows_remain_status_only_while_uninstalling_requires_resolution() {
        let installing = InstallationLifecycleIssue {
            installation_id: "installing-1".into(),
            game_coordinate: "30402:publisher:game".into(),
            installation_root: "/managed/installing-1".into(),
            state: InstallationState::Installing,
            recovery_discriminator: None,
            updated_at: 1,
        };
        assert!(lifecycle_issue_message(&installing).contains("Retry status"));
        assert!(!lifecycle_issue_message(&installing).contains("remove the installation root"));
    }

    #[test]
    fn lifecycle_actions_require_registry_recovery_and_lifecycle_state() {
        assert!(!lifecycle_action_unavailable(false, false, false, false));
        assert!(lifecycle_action_unavailable(true, false, false, false));
        assert!(lifecycle_action_unavailable(false, true, false, false));
        assert!(lifecycle_action_unavailable(false, false, true, false));
        assert!(lifecycle_action_unavailable(false, false, false, true));
    }

    #[test]
    fn retained_reinstall_eligibility_depends_only_on_device_installation_state() {
        let coordinate = "30402:publisher:offline-game";
        assert_eq!(
            retained_reinstall_eligibility(coordinate, &[], &[]),
            RetainedReinstallEligibility::Available
        );

        let ready = installed(coordinate.to_string(), 1);
        assert_eq!(
            retained_reinstall_eligibility(coordinate, &[ready], &[]),
            RetainedReinstallEligibility::ReadyInstalled
        );

        for state in [
            InstallationState::Installing,
            InstallationState::Failed,
            InstallationState::Uninstalling,
        ] {
            let issue = InstallationLifecycleIssue {
                installation_id: format!("{state:?}"),
                game_coordinate: coordinate.into(),
                installation_root: "/managed/device-state".into(),
                state,
                recovery_discriminator: None,
                updated_at: 1,
            };
            assert_eq!(
                retained_reinstall_eligibility(coordinate, &[], &[issue]),
                RetainedReinstallEligibility::Lifecycle(state)
            );
        }

        // The eligibility API deliberately accepts no listing, authentication,
        // or ownership input: a retained package is a fully local reinstall.
        assert_eq!(
            retained_reinstall_unavailable_message(RetainedReinstallEligibility::Available),
            None
        );
    }
}
