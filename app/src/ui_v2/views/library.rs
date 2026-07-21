use std::collections::{HashMap, HashSet};

use arcadestr_core::is_replaceable_event_newer;
use leptos::prelude::*;
use nostr::nips::nip19::FromBech32;
use wasm_bindgen_futures::spawn_local;

use crate::models::{
    npub_fallback_label, AcquisitionPolicy, GameListing, ListingSource, PlatformInfo,
};
use crate::tauri_bridge::{
    invoke_get_installed_games, invoke_get_listing_ownership, invoke_get_platform_info,
    InstalledGame,
};
use crate::ui_v2::components::PageHeader;
use crate::ui_v2::views::marketplace_loader::use_marketplace_listings_with_limit;
use crate::ui_v2::views::{use_fallback_cover, valid_cover_url, FALLBACK_COVER};
use crate::AuthContext;

const LIBRARY_MARKETPLACE_LIMIT: usize = 200;
const INSTALLATION_SCOPE_LABEL: &str = "Installed on this device; not account-scoped";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum MetadataFilter {
    #[default]
    All,
    Available,
    Missing,
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
        entry.installed.file_path.clone(),
        entry.installed.version.clone().unwrap_or_default(),
    ];
    if let Some(listing) = &entry.listing {
        fields.extend(listing.platforms.iter().cloned());
    }
    normalize_search(&fields.join(" ")).contains(&query)
}

fn entry_matches_metadata_filter(entry: &LibraryEntry, filter: MetadataFilter) -> bool {
    match filter {
        MetadataFilter::All => true,
        MetadataFilter::Available => entry.listing.is_some(),
        MetadataFilter::Missing => entry.listing.is_none(),
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
    let registry_loading = RwSignal::new(!standalone_web);
    let registry_error = RwSignal::new(None::<String>);
    let refresh_generation = RwSignal::new(0_u64);
    let platform = RwSignal::new(None::<PlatformInfo>);
    let search = RwSignal::new(String::new());
    let metadata_filter = RwSignal::new(MetadataFilter::All);
    let ownership = RwSignal::new(HashMap::<(String, String), bool>::new());
    let ownership_loading = RwSignal::new(HashSet::<String>::new());
    let ownership_errors = RwSignal::new(HashSet::<String>::new());
    let ownership_generation = RwSignal::new(0_u64);
    let library_now = RwSignal::new(current_unix_secs());
    let marketplace = use_marketplace_listings_with_limit(LIBRARY_MARKETPLACE_LIMIT);

    #[cfg(target_arch = "wasm32")]
    {
        let clock = send_wrapper::SendWrapper::new(gloo_timers::callback::Interval::new(
            30_000,
            move || library_now.set(current_unix_secs()),
        ));
        on_cleanup(move || drop(clock));
    }

    Effect::new(move |_| {
        let _ = refresh_generation.get();
        if availability == LibraryAvailability::DesktopOnly {
            registry_loading.set(false);
            return;
        }
        registry_loading.set(true);
        registry_error.set(None);
        spawn_local(async move {
            match invoke_get_installed_games().await {
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
            if let Ok(value) = invoke_get_platform_info().await {
                platform.set(Some(value));
            }
        });
    });

    let entries = Signal::derive(move || {
        reconcile_installed_entries(installed_games.get(), &marketplace.listings.get())
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
        let candidates = current_entries
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
    let visible_entries = Signal::derive(move || {
        entries
            .get()
            .into_iter()
            .filter(|entry| entry_matches_search(entry, &search.get()))
            .filter(|entry| entry_matches_metadata_filter(entry, metadata_filter.get()))
            .collect::<Vec<_>>()
    });

    view! {
        <section class="v2-library-wrap">
            <PageHeader
                eyebrow="Your collection".to_string()
                title="Installed Library".to_string()
                description="Downloaded game artifacts recorded on this device. Installations are device-wide and do not prove ownership for the active account.".to_string()
                action=view! { <span class="v2-library-collection-label">"Installed registry"</span> }.into_any()
            />

            {if availability == LibraryAvailability::DesktopOnly {
                view! {
                    <section class="v2-library-state-card v2-panel-glass">
                        <span class="material-symbols-outlined" aria-hidden="true">"desktop_windows"</span>
                        <h2>"Desktop Library unavailable on web"</h2>
                        <p>"Installed-game records and native filesystem information are available only in the Arcadestr desktop application."</p>
                    </section>
                }.into_any()
            } else {
                view! {
                    <div class="v2-library-layout">
                        <main class="v2-library-main">
                            <section class="v2-library-controls v2-panel">
                                <div class="v2-library-search-field">
                                    <label for="library-search">"Search installed games"</label>
                                    <div>
                                        <span class="material-symbols-outlined" aria-hidden="true">"search"</span>
                                        <input
                                            id="library-search"
                                            class="v2-input"
                                            type="search"
                                            placeholder="Title, publisher, version, platform, or coordinate"
                                            bind:value=search
                                        />
                                    </div>
                                </div>
                                <fieldset class="v2-library-filter-group">
                                    <legend>"Store metadata"</legend>
                                    <button class="v2-tab" class:active=move || metadata_filter.get() == MetadataFilter::All aria-pressed=move || metadata_filter.get() == MetadataFilter::All on:click=move |_| metadata_filter.set(MetadataFilter::All)>"All"</button>
                                    <button class="v2-tab" class:active=move || metadata_filter.get() == MetadataFilter::Available aria-pressed=move || metadata_filter.get() == MetadataFilter::Available on:click=move |_| metadata_filter.set(MetadataFilter::Available)>"Details available"</button>
                                    <button class="v2-tab" class:active=move || metadata_filter.get() == MetadataFilter::Missing aria-pressed=move || metadata_filter.get() == MetadataFilter::Missing on:click=move |_| metadata_filter.set(MetadataFilter::Missing)>"Details unavailable"</button>
                                </fieldset>
                                <button class="v2-btn-secondary" on:click=move |_| refresh_generation.update(|generation| *generation = generation.wrapping_add(1)) disabled=move || registry_loading.get()>
                                    {move || if registry_loading.get() { "Refreshing..." } else { "Refresh local registry" }}
                                </button>
                            </section>

                            <p class="v2-library-result-count" role="status" aria-live="polite">
                                {move || format!("{} of {} installed entries shown", visible_entries.get().len(), entries.get().len())}
                            </p>

                            <Show when=move || marketplace.error.get().is_some() && !installed_games.get().is_empty()>
                                <div class="v2-library-notice">
                                    <strong>"Current store details could not be refreshed."</strong>
                                    <span>"Local installed entries remain available below. Some covers, titles, ownership, and compatibility information may be missing."</span>
                                </div>
                            </Show>
                            <Show when=move || registry_error.get().is_some() && !installed_games.get().is_empty()>
                                <div class="v2-library-notice">
                                    <strong>"The local registry refresh failed."</strong>
                                    <span>"Previously loaded installed records remain visible. Retry when the desktop service is available."</span>
                                </div>
                            </Show>
                            <Show when=move || marketplace.loading.get() && !installed_games.get().is_empty()>
                                <div class="v2-library-notice" role="status">
                                    <strong>"Loading current store details..."</strong>
                                    <span>"Installed records are already available; covers and listing metadata will appear when matched."</span>
                                </div>
                            </Show>

                            {move || if registry_loading.get() {
                                view! {
                                    <section class="v2-library-state-card v2-panel-glass" role="status">
                                        <h2>"Loading installed games"</h2>
                                        <p>"Reading the local installed-game registry."</p>
                                    </section>
                                }.into_any()
                            } else if registry_error.get().is_some() && installed_games.get().is_empty() {
                                let error = registry_error.get().unwrap_or_default();
                                view! {
                                    <section class="v2-library-state-card v2-panel-glass" role="alert">
                                        <h2>"Installed registry unavailable"</h2>
                                        <p>{error}</p>
                                        <button class="v2-btn-secondary" on:click=move |_| refresh_generation.update(|generation| *generation = generation.wrapping_add(1))>"Retry"</button>
                                    </section>
                                }.into_any()
                            } else if installed_games.get().is_empty() {
                                view! {
                                    <section class="v2-library-state-card v2-panel-glass">
                                        <span class="material-symbols-outlined" aria-hidden="true">"download_done"</span>
                                        <h2>"No installed games"</h2>
                                        <p>"Games installed through the desktop acquisition flow will appear here."</p>
                                    </section>
                                }.into_any()
                            } else if visible_entries.get().is_empty() {
                                view! {
                                    <section class="v2-library-state-card v2-panel-glass">
                                        <h2>"No installed entries match"</h2>
                                        <p>"Change the search text or metadata filter to show other local records."</p>
                                    </section>
                                }.into_any()
                            } else {
                                let cards = visible_entries.get().into_iter().map(|entry| {
                                    let listing = entry.listing.clone();
                                    let title = fallback_title(&entry);
                                    let publisher = publisher_label(&entry);
                                    let cover = entry.listing.as_ref().and_then(|listing| valid_cover_url(&listing.images)).unwrap_or_else(|| FALLBACK_COVER.to_string());
                                    let version = sanitize_optional_value(entry.installed.version.as_deref());
                                    let coordinate = entry.installed.game_coordinate.clone();
                                    let active_npub = auth.npub.get();
                                    let durable_owned = ownership_for_active_account(
                                        &ownership.get(),
                                        active_npub.as_deref(),
                                        &coordinate,
                                    );
                                    let access = entry.listing.as_ref().map(|listing| access_presentation(listing, durable_owned, library_now.get()));
                                    let checking_ownership = ownership_loading.get().contains(&coordinate);
                                    let ownership_failed = ownership_errors.get().contains(&coordinate);
                                    let compatibility = listing_compatibility(entry.listing.as_ref(), platform.get().as_ref());
                                    view! {
                                        <article class="v2-library-card">
                                            <div class="v2-library-card-media">
                                                <img src=cover alt=format!("{title} cover") loading="lazy" on:error=use_fallback_cover />
                                                <div class="v2-library-card-badges">
                                                    <span>"Installed"</span>
                                                    {access.map(|access| view! { <span class:v2-library-owned=access.durable_owned>{access.label}</span> })}
                                                    {if checking_ownership {
                                                        view! { <span>"Checking ownership"</span> }.into_any()
                                                    } else if ownership_failed {
                                                        view! { <span>"Ownership check unavailable"</span> }.into_any()
                                                    } else {
                                                        view! { <></> }.into_any()
                                                    }}
                                                    {match compatibility {
                                                        LibraryCompatibility::Compatible => view! { <span>"Compatible"</span> }.into_any(),
                                                        LibraryCompatibility::Incompatible => view! { <span class="v2-library-incompatible">"Not compatible with this device"</span> }.into_any(),
                                                        LibraryCompatibility::Unknown => view! { <></> }.into_any(),
                                                    }}
                                                </div>
                                            </div>
                                            <div class="v2-library-card-body">
                                                <div>
                                                    <p class="v2-store-kicker">{publisher}</p>
                                                    <h2>{title}</h2>
                                                    <p class="v2-library-scope">{INSTALLATION_SCOPE_LABEL}</p>
                                                </div>
                                                {if entry.listing.is_none() && marketplace.loading.get() {
                                                    view! { <p class="v2-library-metadata-missing">"Looking for the matching signed listing..."</p> }.into_any()
                                                } else if entry.listing.is_none() {
                                                    view! { <p class="v2-library-metadata-missing">"Current store details are unavailable. The local installed record is preserved."</p> }.into_any()
                                                } else {
                                                    view! { <></> }.into_any()
                                                }}
                                                {if entry.malformed_coordinate {
                                                    view! { <p class="v2-library-metadata-missing">"The stored listing coordinate is malformed; metadata matching is disabled for this entry."</p> }.into_any()
                                                } else {
                                                    view! { <></> }.into_any()
                                                }}
                                                <dl class="v2-library-card-details">
                                                    {version.map(|version| view! { <div><dt>"Installed version"</dt><dd>{version}</dd></div> })}
                                                    <div><dt>"Installed"</dt><dd>{format_installed_at(entry.installed.installed_at)}</dd></div>
                                                    <div><dt>"Artifact location"</dt><dd>{if entry.installed.file_path.trim().is_empty() { "Unavailable".to_string() } else { entry.installed.file_path.clone() }}</dd></div>
                                                    <div><dt>"Listing coordinate"</dt><dd>{if entry.installed.game_coordinate.trim().is_empty() { "Malformed or unavailable".to_string() } else { entry.installed.game_coordinate.clone() }}</dd></div>
                                                </dl>
                                                {match listing {
                                                    Some(listing) => {
                                                        let selected = listing.clone();
                                                        view! { <button class="v2-btn-primary" on:click=move |_| on_open_listing.run(selected.clone())>"View store details"</button> }.into_any()
                                                    }
                                                    None => view! { <p class="v2-library-no-action">"A detail action will be available when the matching signed listing is loaded."</p> }.into_any(),
                                                }}
                                            </div>
                                        </article>
                                    }
                                }).collect::<Vec<_>>();
                                view! { <section class="v2-library-card-grid" aria-label="Installed games">{cards}</section> }.into_any()
                            }}
                        </main>

                        <aside class="v2-library-summary">
                            <section class="v2-library-summary-card v2-panel">
                                <p class="v2-store-kicker">"Local inventory"</p>
                                <h2>"Library summary"</h2>
                                <dl>
                                    <div><dt>"Installed records"</dt><dd>{move || entries.get().len()}</dd></div>
                                    <div><dt>"Store details found"</dt><dd>{move || entries.get().iter().filter(|entry| entry.listing.is_some()).count()}</dd></div>
                                    <div><dt>"Store details missing"</dt><dd>{move || entries.get().iter().filter(|entry| entry.listing.is_none()).count()}</dd></div>
                                    <div><dt>"Incompatible listings"</dt><dd>{move || entries.get().iter().filter(|entry| listing_compatibility(entry.listing.as_ref(), platform.get().as_ref()) == LibraryCompatibility::Incompatible).count()}</dd></div>
                                </dl>
                            </section>
                            <section class="v2-library-summary-card v2-panel">
                                <p class="v2-store-kicker">"What this means"</p>
                                <h2>"Device registry"</h2>
                                <p>"These records are shared by accounts using this Arcadestr installation. An installed artifact does not by itself prove a purchase or permanent entitlement for the active account."</p>
                            </section>
                        </aside>
                    </div>
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
