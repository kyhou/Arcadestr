use std::collections::{HashMap, HashSet};

use arcadestr_core::is_replaceable_event_newer;
use leptos::prelude::*;
use nostr::nips::nip19::FromBech32;
use wasm_bindgen_futures::spawn_local;

use crate::models::{
    npub_fallback_label, AcquisitionPolicy, GameListing, ListingSource, PlatformInfo,
};
use crate::tauri_bridge::{
    invoke_get_installed_games, invoke_get_library_games, invoke_get_listing_ownership,
    invoke_get_platform_info, InstalledGame, LibraryGame,
};
use crate::ui_v2::components::{
    artwork_state_from_url, ArtworkRole, EmptyState, ErrorSeverity, ErrorState, FeedbackLayout,
    GameArtwork, LoadingState, PageHeader, PageTabItem, PageTabSemantics, PageTabs, StatusChip,
    StatusChipSize, StatusChipVariant,
};
use crate::ui_v2::views::marketplace_loader::use_marketplace_listings_with_limit;
use crate::ui_v2::views::valid_cover_url;
use crate::AuthContext;

const LIBRARY_MARKETPLACE_LIMIT: usize = 200;
const INSTALLATION_SCOPE_LABEL: &str = "Installed on this device; not account-scoped";

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
                "Saved account games and artifacts registered on this device will appear here.",
            ),
            Self::Saved => (
                "No account-saved games",
                "Games added to the active account's library will appear here.",
            ),
            Self::Installed => (
                "No installed artifacts",
                "Verified artifacts registered on this device will appear here.",
            ),
            Self::NotInstalled => (
                "No saved games waiting for download",
                "Every matched account-saved game is already registered on this device.",
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
        return LibraryCompatibility::Compatible;
    }
    match platform {
        Some(platform)
            if listing
                .platforms
                .iter()
                .any(|value| value == &platform.tag()) =>
        {
            LibraryCompatibility::Compatible
        }
        Some(_) => LibraryCompatibility::Incompatible,
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
        entry.installed.version.clone().unwrap_or_default(),
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

fn artifact_file_label(path: &str) -> String {
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

fn sanitize_optional_value(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
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
    let marketplace = use_marketplace_listings_with_limit(LIBRARY_MARKETPLACE_LIMIT);

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
        spawn_local(async move {
            let result = invoke_get_installed_games().await;
            if refresh_generation.get_untracked() != requested_generation {
                return;
            }
            match result {
                Ok(games) => installed_games.set(games),
                Err(error) => registry_error.set(Some(error)),
            }
            registry_loading.set(false);
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
        refresh_generation.update(|generation| *generation = generation.wrapping_add(1));
    });
    let retry_refresh_error = Callback::new(move |_: leptos::ev::MouseEvent| {
        refresh_generation.update(|generation| *generation = generation.wrapping_add(1));
    });

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
                        <button class="v2-btn-secondary" on:click=move |_| retry_refresh.run(()) disabled=move || registry_loading.get() || saved_loading.get()>
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
                            <div><strong>"Store metadata is partial"</strong><p>"Local and account records remain visible, but some artwork, titles, access policy, and compatibility details may be unavailable."</p></div>
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
                                let version = sanitize_optional_value(entry.installed.version.as_deref());
                                let coordinate = entry.installed.game_coordinate.clone();
                                let active_npub = auth.npub.get();
                                let durable_owned = ownership_for_active_account(&ownership.get(), active_npub.as_deref(), &coordinate);
                                let checking_ownership = ownership_loading.get().contains(&coordinate);
                                let ownership_failed = ownership_errors.get().contains(&coordinate);
                                let compatibility = listing_compatibility(entry.listing.as_ref(), platform.get().as_ref());
                                let compatibility_state = compatibility_chip(compatibility);
                                let installed_meta = [version.clone().map(|value| format!("Version {value}")), Some(format_installed_at(entry.installed.installed_at))].into_iter().flatten().collect::<Vec<_>>().join(" · ");
                                let artifact_file = artifact_file_label(&entry.installed.file_path);
                                let stored_coordinate = if coordinate.trim().is_empty() { "Malformed or unavailable".to_string() } else { coordinate.clone() };
                                view! {
                                    <article class="arc-library-row arc-library-row-installed">
                                        <div class="arc-library-art"><GameArtwork title=title.clone() state=artwork role=ArtworkRole::Thumbnail /></div>
                                        <div class="arc-library-row-copy">
                                            <h2>{title}</h2><p>{publisher}</p><small>{installed_meta}</small>
                                            {if entry.listing.is_none() { view! { <span class="arc-library-row-warning">"Current store metadata was not loaded in the discovery window."</span> }.into_any() } else { view! { <></> }.into_any() }}
                                            {if entry.malformed_coordinate { view! { <span class="arc-library-row-warning">"Stored listing coordinate is malformed."</span> }.into_any() } else { view! { <></> }.into_any() }}
                                            <details class="arc-library-technical"><summary>"Local record details"</summary><dl><div><dt>"Artifact file"</dt><dd>{artifact_file}</dd></div><div><dt>"Listing coordinate"</dt><dd>{stored_coordinate}</dd></div></dl></details>
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
                                            <StatusChip label="Installed" variant=StatusChipVariant::Installed icon=Some("download_done") size=StatusChipSize::Compact />
                                            <StatusChip label=compatibility_state.0 variant=compatibility_state.1 icon=None size=StatusChipSize::Compact />
                                        </div>
                                        <div class="arc-library-row-action">
                                            {match listing { Some(listing) => { let selected = listing.clone(); view! { <button class="v2-btn-primary" on:click=move |_| on_open_listing.run(selected.clone())>"View details"</button> }.into_any() }, None => view! { <span class="arc-library-action-unavailable">"Details unavailable"</span> }.into_any() }}
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
                }.into_any()
            }}
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
            game_coordinate: coordinate,
            file_path: "/games/artifact.bin".to_string(),
            file_hash: "ab".repeat(32),
            version: Some("1.0.0".to_string()),
            server_url: "https://distribution.arcadestr.test".to_string(),
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
        game.file_path.clear();
        game.version = Some("   ".to_string());
        let entries = reconcile_installed_entries(vec![game], &[]);
        assert_eq!(entries.len(), 1);
        assert!(entries[0].malformed_coordinate);
        assert_eq!(fallback_title(&entries[0]), "Installed game");
        assert_eq!(
            sanitize_optional_value(entries[0].installed.version.as_deref()),
            None
        );
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
        assert!(entry_matches_search(&entries[0], "1.0.0"));
        assert!(!entry_matches_search(&entries[0], "different"));
        assert!(!entry_matches_search(&entries[0], "/games/artifact.bin"));
        assert_eq!(
            artifact_file_label("/private/home/player/game.bin"),
            "game.bin"
        );
        assert_eq!(
            artifact_file_label(r"C:\Users\player\Arcadestr\game.exe"),
            "game.exe"
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
            concat!("Uninstall ", "Game"),
            concat!("Open ", "Folder"),
        ] {
            assert!(
                !source.contains(action),
                "unsupported action present: {action}"
            );
        }
    }
}
