use leptos::prelude::*;
use wasm_bindgen_futures::spawn_local;

use crate::campaign_management::current_user_listings;
use crate::components::BadgeShowcase;
use crate::models::{
    npub_fallback_label, AcquisitionPolicy, GameListing, Nip05Status, UserProfile,
};
use crate::tauri_bridge::invoke_verify_nip05;
use crate::ui_v2::components::{
    artwork_state_from_url, ArtworkRole, EmptyState, FeedbackLayout, GameArtwork, LoadingState,
    StatusChip, StatusChipSize, StatusChipVariant,
};
use crate::ui_v2::views::marketplace_loader::use_marketplace_listings_with_limit;
use crate::ui_v2::views::valid_cover_url;
use crate::AuthContext;

#[path = "../../components/nip05_badge.rs"]
mod nip05_badge;
use nip05_badge::Nip05Badge;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ProfileMetadataState {
    SignedOut,
    Loading,
    Missing,
    Error,
    Ready,
    ReadyWithError,
}

fn profile_has_visible_metadata(profile: &UserProfile) -> bool {
    [
        profile.display_name.as_deref(),
        profile.name.as_deref(),
        profile.picture.as_deref(),
        profile.about.as_deref(),
        profile.website.as_deref(),
        profile.nip05.as_deref(),
        profile.lud16.as_deref(),
    ]
    .into_iter()
    .flatten()
    .any(|value| !value.trim().is_empty())
}

fn profile_for_account(profile: Option<UserProfile>, npub: Option<&str>) -> Option<UserProfile> {
    profile.filter(|profile| Some(profile.npub.as_str()) == npub)
}

fn profile_metadata_state(
    npub: Option<&str>,
    profile: Option<&UserProfile>,
    loading: bool,
    has_error: bool,
) -> ProfileMetadataState {
    if loading {
        ProfileMetadataState::Loading
    } else if npub.is_none() {
        if has_error {
            ProfileMetadataState::Error
        } else {
            ProfileMetadataState::SignedOut
        }
    } else if profile.is_some_and(profile_has_visible_metadata) {
        if has_error {
            ProfileMetadataState::ReadyWithError
        } else {
            ProfileMetadataState::Ready
        }
    } else if has_error {
        ProfileMetadataState::Error
    } else {
        ProfileMetadataState::Missing
    }
}

fn profile_display_name(profile: Option<&UserProfile>, npub: Option<&str>) -> String {
    profile
        .map(UserProfile::display)
        .filter(|value| !value.trim().is_empty())
        .or_else(|| npub.map(npub_fallback_label))
        .unwrap_or_else(|| "Profile unavailable".to_string())
}

/// Whether the heading is showing the public key itself rather than a name.
///
/// With no usable display name, `profile_display_name` falls back to the
/// abbreviated key — so rendering the standalone key line underneath printed
/// the same identifier twice, in two different abbreviations.
/// Checks the name fields directly rather than `UserProfile::display()`:
/// `display()` already falls back to the abbreviated key itself, so it is
/// never empty and cannot distinguish "has a name" from "showing the key".
fn heading_is_public_key(profile: Option<&UserProfile>) -> bool {
    profile
        .and_then(|profile| {
            profile
                .display_name
                .clone()
                .or_else(|| profile.name.clone())
        })
        .filter(|value| !value.trim().is_empty())
        .is_none()
}

/// Abbreviate a public key for display.
///
/// Delegates to the shared label so an identifier reads identically wherever it
/// appears. This used to truncate to the same 12/8 split but join with `…`
/// while `npub_fallback_label` joined with `...`, so the profile heading and
/// the key line disagreed character-for-character on the same key.
fn abbreviate_public_key(value: &str) -> String {
    npub_fallback_label(value)
}

fn profile_access_chip(listing: &GameListing) -> (&'static str, StatusChipVariant) {
    match listing.acquisition {
        AcquisitionPolicy::Public => ("Public", StatusChipVariant::Public),
        AcquisitionPolicy::TimedAccess { .. } => ("Timed access", StatusChipVariant::TimedAccess),
        AcquisitionPolicy::Gated if listing.has_declared_price() => {
            ("Paid", StatusChipVariant::Active)
        }
        AcquisitionPolicy::Gated => ("Gated", StatusChipVariant::Gated),
    }
}

fn reviews_unavailable_message() -> &'static str {
    "Verified reviews are not available because Arcadestr has no authoritative review query or purchase-linked review model."
}

fn normalize_nip05_identifier(identifier: &str) -> Option<(String, String, String)> {
    let trimmed = identifier.trim();
    let (local_part, domain) = trimmed.split_once('@')?;
    let normalized_local = local_part.trim().to_lowercase();
    let normalized_domain = domain.trim().to_lowercase();
    if normalized_local.is_empty() || normalized_domain.is_empty() {
        return None;
    }
    let normalized_identifier = format!("{normalized_local}@{normalized_domain}");
    Some((normalized_identifier, normalized_local, normalized_domain))
}

fn default_nip05_status(identifier: Option<String>) -> Nip05Status {
    let raw_identifier = identifier.unwrap_or_default();
    if let Some((normalized_identifier, local_part, domain)) =
        normalize_nip05_identifier(&raw_identifier)
    {
        Nip05Status {
            identifier: raw_identifier,
            normalized_identifier,
            local_part,
            domain,
            verified: false,
            status: "unverified".to_string(),
            message: "Identifier has not been verified for this account.".to_string(),
        }
    } else {
        Nip05Status {
            identifier: raw_identifier,
            normalized_identifier: String::new(),
            local_part: String::new(),
            domain: String::new(),
            verified: false,
            status: "unverified".to_string(),
            message: "No valid NIP-05 identifier is available.".to_string(),
        }
    }
}

fn nip05_status_from_profile(profile: Option<&UserProfile>) -> Nip05Status {
    default_nip05_status(profile.and_then(|profile| profile.nip05.clone()))
}

fn nip05_status_identifier(status: &Nip05Status) -> String {
    if status.normalized_identifier.is_empty() {
        status.identifier.clone()
    } else {
        status.normalized_identifier.clone()
    }
}

fn should_auto_verify_nip05(
    status: &Nip05Status,
    expected_npub: &str,
    last_attempt_key: Option<&str>,
) -> bool {
    let identifier = nip05_status_identifier(status);
    if identifier.is_empty()
        || status.normalized_identifier.trim().is_empty()
        || expected_npub.is_empty()
    {
        return false;
    }
    if status.verified
        || status.status.eq_ignore_ascii_case("verified")
        || status.status.eq_ignore_ascii_case("verifying")
        || status.status.eq_ignore_ascii_case("failed")
    {
        return false;
    }
    let attempt_key = format!("{expected_npub}|{identifier}");
    last_attempt_key != Some(attempt_key.as_str())
}

fn should_apply_nip05_response(
    requested_npub: &str,
    requested_identifier: &str,
    current_npub: &str,
    current_identifier: &str,
) -> bool {
    !requested_npub.is_empty()
        && requested_npub == current_npub
        && requested_identifier == current_identifier
}

fn checked_nip05_response(status: Nip05Status, requested_identifier: &str) -> Option<Nip05Status> {
    let (expected, _, _) = normalize_nip05_identifier(requested_identifier)?;
    let (returned, _, _) = normalize_nip05_identifier(&nip05_status_identifier(&status))?;
    let consistent_verification = status.verified == status.status.eq_ignore_ascii_case("verified");
    (expected == returned && consistent_verification).then_some(status)
}

fn valid_public_url(value: Option<String>) -> Option<String> {
    value.and_then(|value| {
        let parsed = url::Url::parse(value.trim()).ok()?;
        if parsed.scheme() != "https" || !is_public_url_host(&parsed) {
            return None;
        }
        Some(value.trim().to_string())
    })
}

fn is_public_url_host(url: &url::Url) -> bool {
    let Some(host) = url.host() else {
        return false;
    };
    match host {
        url::Host::Domain(domain) => !domain.eq_ignore_ascii_case("localhost"),
        url::Host::Ipv4(address) => {
            let octets = address.octets();
            !(address.is_private()
                || address.is_loopback()
                || address.is_link_local()
                || address.is_unspecified()
                || address.is_multicast()
                || address.is_broadcast()
                || octets[0] == 0
                || (octets[0] == 100 && (64..=127).contains(&octets[1])))
        }
        url::Host::Ipv6(address) => {
            !(address.is_loopback()
                || address.is_unspecified()
                || address.is_unique_local()
                || address.is_unicast_link_local()
                || address.is_multicast())
        }
    }
}

#[component]
pub fn ProfileV2View(
    on_open_publish: Callback<()>,
    on_open_listing: Callback<GameListing>,
) -> impl IntoView {
    let auth = use_context::<AuthContext>().expect("AuthContext not provided");
    let verification_available = !cfg!(feature = "web");
    let marketplace_state = use_marketplace_listings_with_limit(50);
    let initial_npub = auth.npub.get_untracked();
    let initial_profile =
        profile_for_account(auth.profile.get_untracked(), initial_npub.as_deref());
    let nip05_status = RwSignal::new(nip05_status_from_profile(initial_profile.as_ref()));
    let last_auto_nip05_attempt = RwSignal::new(None::<String>);
    let avatar_failed = RwSignal::new(false);

    let listings = marketplace_state.listings;
    let listings_auth = auth.clone();
    let my_listings = Signal::derive(move || {
        let Some(npub) = listings_auth.npub.get() else {
            return Vec::new();
        };
        current_user_listings(listings.get(), &npub)
    });
    let current_npub_auth = auth.clone();
    let current_npub = Signal::derive(move || current_npub_auth.npub.get().unwrap_or_default());
    let profile_auth = auth.clone();
    let profile = Signal::derive(move || {
        let npub = profile_auth.npub.get();
        profile_for_account(profile_auth.profile.get(), npub.as_deref())
    });
    let display_name = Signal::derive(move || {
        profile_display_name(profile.get().as_ref(), Some(current_npub.get().as_str()))
    });
    let metadata_auth = auth.clone();
    let metadata_state = Signal::derive(move || {
        profile_metadata_state(
            metadata_auth.npub.get().as_deref(),
            profile.get().as_ref(),
            metadata_auth.is_loading.get(),
            metadata_auth.error.get().is_some(),
        )
    });

    let auth_for_auto_verify = auth.clone();
    Effect::new(move |_| {
        nip05_status.set(nip05_status_from_profile(profile.get().as_ref()));
        last_auto_nip05_attempt.set(None);
        avatar_failed.set(false);
    });

    Effect::new(move |_| {
        if !verification_available {
            return;
        }
        let status = nip05_status.get();
        let requested_npub = current_npub.get();
        let last_attempt = last_auto_nip05_attempt.get();
        if !should_auto_verify_nip05(&status, &requested_npub, last_attempt.as_deref()) {
            return;
        }
        let identifier = nip05_status_identifier(&status);
        last_auto_nip05_attempt.set(Some(format!("{requested_npub}|{identifier}")));
        nip05_status.set(Nip05Status {
            status: "verifying".to_string(),
            message: "Checking identifier...".to_string(),
            ..status
        });
        let auth_for_response = auth_for_auto_verify.clone();
        spawn_local(async move {
            let result = invoke_verify_nip05(identifier.clone(), requested_npub.clone()).await;
            if !should_apply_nip05_response(
                &requested_npub,
                &identifier,
                &auth_for_response.npub.get_untracked().unwrap_or_default(),
                &nip05_status_identifier(&nip05_status.get_untracked()),
            ) {
                return;
            }
            match result {
                Ok(status) => match checked_nip05_response(status, &identifier) {
                    Some(status) => nip05_status.set(status),
                    None => nip05_status.set(Nip05Status {
                        status: "failed".to_string(),
                        message: "The identity service returned an inconsistent response."
                            .to_string(),
                        ..default_nip05_status(Some(identifier))
                    }),
                },
                Err(_) => nip05_status.set(Nip05Status {
                    status: "failed".to_string(),
                    message: "NIP-05 lookup failed. Retry when the identity service is available."
                        .to_string(),
                    ..default_nip05_status(Some(identifier))
                }),
            }
        });
    });

    let on_verify_nip05 = Callback::new(move |identifier: String| {
        let requested_npub = current_npub.get_untracked();
        let mut verifying = nip05_status.get_untracked();
        verifying.status = "verifying".to_string();
        verifying.message = "Checking identifier...".to_string();
        nip05_status.set(verifying);
        let auth_for_response = auth.clone();
        spawn_local(async move {
            let result = invoke_verify_nip05(identifier.clone(), requested_npub.clone()).await;
            if !should_apply_nip05_response(
                &requested_npub,
                &identifier,
                &auth_for_response.npub.get_untracked().unwrap_or_default(),
                &nip05_status_identifier(&nip05_status.get_untracked()),
            ) {
                return;
            }
            match result {
                Ok(status) => match checked_nip05_response(status, &identifier) {
                    Some(status) => nip05_status.set(status),
                    None => nip05_status.set(Nip05Status {
                        status: "failed".to_string(),
                        message: "The identity service returned an inconsistent response."
                            .to_string(),
                        ..default_nip05_status(Some(identifier))
                    }),
                },
                Err(_) => nip05_status.set(Nip05Status {
                    status: "failed".to_string(),
                    message: "NIP-05 lookup failed. Retry when the identity service is available."
                        .to_string(),
                    ..default_nip05_status(Some(identifier))
                }),
            }
        });
    });

    view! {
        <section class="arc-profile-page">
            <Show
                when=move || !current_npub.get().is_empty()
                fallback=move || match metadata_state.get() {
                    ProfileMetadataState::Loading => view! {
                        <div><h1 class="arc-profile-page-title">"Profile"</h1><LoadingState title="Loading active account" description="Profile identity remains hidden until the active public key is available.".to_string() layout=FeedbackLayout::Panel /></div>
                    }.into_any(),
                    ProfileMetadataState::Error | ProfileMetadataState::ReadyWithError => view! {
                        <div><h1 class="arc-profile-page-title">"Profile"</h1><div class="arc-profile-notice arc-profile-notice-error" role="alert"><strong>"Active account unavailable"</strong><span>"Profile identity cannot be shown because the active public key could not be resolved."</span></div></div>
                    }.into_any(),
                    ProfileMetadataState::SignedOut | ProfileMetadataState::Missing | ProfileMetadataState::Ready => view! {
                        <div><h1 class="arc-profile-page-title">"Profile"</h1><EmptyState title="Sign in to view your profile" description="Public metadata, authored listings, NIP-05 identity, and earned badges are loaded for the active account." icon="person_off" layout=FeedbackLayout::Panel /></div>
                    }.into_any(),
                }
            >
                <header class="arc-profile-header">
                    <div class="arc-profile-avatar">
                        {move || {
                            let picture = profile.get().and_then(|profile| valid_public_url(profile.picture));
                            if let Some(url) = picture.filter(|_| !avatar_failed.get()) {
                                view! { <img src=url alt=format!("Profile image for {}", display_name.get()) on:error=move |_| avatar_failed.set(true) /> }.into_any()
                            } else {
                                view! { <div class="arc-profile-avatar-fallback" role="img" aria-label=format!("Initial avatar for {}", display_name.get())>{display_name.get().chars().next().unwrap_or('?').to_uppercase().to_string()}</div> }.into_any()
                            }
                        }}
                    </div>
                    <div class="arc-profile-identity">
                        <div class="arc-profile-title-row">
                            <div>
                                <p class="v2-store-kicker">"Public profile"</p>
                                <h1>{move || display_name.get()}</h1>
                            </div>
                            <div class="arc-profile-actions"><button class="v2-btn-secondary" on:click=move |_| on_open_publish.run(())>"Open publishing"</button></div>
                        </div>
                        <div class="arc-profile-identity-chips">
                            <StatusChip label="Current account" variant=StatusChipVariant::Active icon=Some("person") size=StatusChipSize::Compact />
                            {move || (!my_listings.get().is_empty()).then(|| view! { <StatusChip label=format!("Publisher of {} loaded game(s)", my_listings.get().len()) variant=StatusChipVariant::Published icon=Some("sports_esports") size=StatusChipSize::Compact /> })}
                        </div>
                        {move || profile.get().and_then(|profile| profile.name).filter(|name| !name.trim().is_empty()).map(|name| view! { <p class="arc-profile-username">{name}</p> })}
                        {move || (!heading_is_public_key(profile.get().as_ref())).then(|| view! {
                            <p class="arc-profile-key">{abbreviate_public_key(&current_npub.get())}</p>
                        })}
                        {if verification_available {
                            view! { <Nip05Badge status=nip05_status.into() on_verify=on_verify_nip05 /> }.into_any()
                        } else {
                            view! { <p class="arc-profile-muted">{move || { let identifier = nip05_status_identifier(&nip05_status.get()); if identifier.is_empty() { "No NIP-05 identifier is available.".to_string() } else { format!("{identifier} · Verification unavailable on standalone web") } }}</p> }.into_any()
                        }}
                    </div>
                </header>

                {move || match metadata_state.get() {
                    ProfileMetadataState::Loading => view! { <LoadingState title="Loading profile metadata" description="Checking cached metadata while the active account finishes loading.".to_string() layout=FeedbackLayout::Compact /> }.into_any(),
                    ProfileMetadataState::Error => view! { <div class="arc-profile-notice arc-profile-notice-error" role="alert"><strong>"Profile metadata unavailable"</strong><span>"The current account remains visible, but relay metadata could not be resolved."</span></div> }.into_any(),
                    ProfileMetadataState::Missing => view! { <div class="arc-profile-notice" role="status"><strong>"No public profile metadata"</strong><span>"This account has no usable name, biography, website, or profile image in the current metadata result."</span></div> }.into_any(),
                    ProfileMetadataState::ReadyWithError => view! { <div class="arc-profile-notice" role="status"><strong>"Profile refresh incomplete"</strong><span>"Previously loaded metadata remains visible because the latest refresh failed."</span></div> }.into_any(),
                    ProfileMetadataState::Ready | ProfileMetadataState::SignedOut => view! { <></> }.into_any(),
                }}

                <div class="arc-profile-layout">
                    <main class="arc-profile-main">
                        <section class="arc-profile-about" aria-labelledby="profile-about-title">
                            <p class="v2-store-kicker">"About"</p><h2 id="profile-about-title" class="sr-only">"About this profile"</h2>
                            {move || profile.get().and_then(|profile| profile.about).filter(|about| !about.trim().is_empty()).map(|about| view! { <p>{about}</p> }.into_any()).unwrap_or_else(|| view! { <p class="arc-profile-muted">"No public biography is available."</p> }.into_any())}
                            <dl class="arc-profile-metadata">
                                {move || profile.get().and_then(|profile| valid_public_url(profile.website)).map(|website| { let href = website.clone(); view! { <div><dt>"Website"</dt><dd><a href=href target="_blank" rel="noopener noreferrer">{website}</a></dd></div> } })}
                                {move || profile.get().and_then(|profile| profile.lud16).filter(|value| !value.trim().is_empty()).map(|value| view! { <div><dt>"Lightning address"</dt><dd>{value}</dd></div> })}
                            </dl>
                            <details class="arc-profile-key-details"><summary>"Public identity details"</summary><p>{move || current_npub.get()}</p></details>
                            <p class="arc-profile-readonly">"Read-only signed metadata. Profile editing is not implemented in this client."</p>
                        </section>

                        <section class="arc-profile-section" aria-labelledby="profile-games-title">
                            <div class="arc-profile-section-header"><div><p class="v2-store-kicker">"Publisher catalog"</p><h2 id="profile-games-title">"Published games"</h2></div></div>
                            {move || if marketplace_state.loading.get() && my_listings.get().is_empty() {
                                view! { <LoadingState title="Loading published listings" description="The profile remains available while authored listing events load.".to_string() layout=FeedbackLayout::Compact /> }.into_any()
                            } else if marketplace_state.error.get().is_some() && my_listings.get().is_empty() {
                                view! { <div class="arc-profile-notice arc-profile-notice-error" role="alert"><strong>"Published listings unavailable"</strong><span>"No authored listings are available from the current marketplace result."</span></div> }.into_any()
                            } else if my_listings.get().is_empty() {
                                view! { <EmptyState title="No authored listings in the loaded catalog" description="The latest marketplace window contains no current NIP-99 listings authored by this account." icon="sports_esports" layout=FeedbackLayout::Compact /> }.into_any()
                            } else {
                                let refresh_failed = marketplace_state.error.get().is_some();
                                view! {
                                    <div>
                                        {refresh_failed.then(|| if marketplace_state.using_cached_data.get() {
                                            view! { <div class="arc-profile-notice" role="status"><strong>"Cached publisher catalog"</strong><span>"Cached authored listings remain visible after a relay refresh failure."</span></div> }
                                        } else {
                                            view! { <div class="arc-profile-notice" role="status"><strong>"Partial publisher catalog"</strong><span>"Authored listings received before the marketplace refresh failed remain visible."</span></div> }
                                        })}
                                        <div class="arc-profile-listings-grid">
                                            {my_listings.get().into_iter().map(|listing| {
                                                let selected = listing.clone();
                                                let title = listing.title.clone();
                                                let artwork = artwork_state_from_url(valid_cover_url(&listing.images));
                                                let (access_label, access_variant) = profile_access_chip(&listing);
                                                view! {
                                                    <button class="arc-profile-listing-card" on:click=move |_| on_open_listing.run(selected.clone())>
                                                        <div class="arc-profile-listing-art"><GameArtwork title=title.clone() state=artwork role=ArtworkRole::Card /></div>
                                                        <span class="arc-profile-listing-copy"><strong>{title}</strong><small>{format!("{} {}", listing.price, listing.currency)}</small><StatusChip label=access_label variant=access_variant icon=None size=StatusChipSize::Compact /></span>
                                                    </button>
                                                }
                                            }).collect::<Vec<_>>()}
                                        </div>
                                    </div>
                                }.into_any()
                            }}
                        </section>

                        <section class="arc-profile-section arc-profile-unavailable" aria-labelledby="profile-reviews-title">
                            <p class="v2-store-kicker">"Reviews"</p><h2 id="profile-reviews-title">"Verified buyer reviews unavailable"</h2><p>{reviews_unavailable_message()}</p>
                        </section>
                    </main>
                    <aside class="arc-profile-side"><BadgeShowcase profile_identifier=current_npub /></aside>
                </div>
            </Show>
        </section>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn profile(verified: bool) -> UserProfile {
        UserProfile {
            npub: "npub1test".to_string(),
            nip05: Some("Player@Example.com".to_string()),
            nip05_verified: verified,
            ..UserProfile::default()
        }
    }

    #[test]
    fn verified_and_unverified_nip05_are_distinct() {
        let mut verified = nip05_status_from_profile(Some(&profile(true)));
        verified.verified = true;
        verified.status = "verified".to_string();
        let unverified = nip05_status_from_profile(Some(&profile(false)));
        assert!(verified.verified);
        assert!(!unverified.verified);
        assert_eq!(unverified.status, "unverified");
    }

    #[test]
    fn stale_nip05_response_is_rejected() {
        assert!(should_apply_nip05_response(
            "npub1a",
            "a@example.com",
            "npub1a",
            "a@example.com"
        ));
        assert!(!should_apply_nip05_response(
            "npub1a",
            "old@example.com",
            "npub1a",
            "new@example.com"
        ));
    }

    #[test]
    fn nip05_response_must_match_identifier_and_verification_flag() {
        let valid = Nip05Status {
            verified: true,
            status: "verified".into(),
            ..default_nip05_status(Some("player@example.com".into()))
        };
        assert!(checked_nip05_response(valid.clone(), "player@example.com").is_some());
        assert!(checked_nip05_response(valid.clone(), "other@example.com").is_none());

        let mut contradictory = valid;
        contradictory.verified = false;
        assert!(checked_nip05_response(contradictory, "player@example.com").is_none());
    }

    #[test]
    fn auto_verify_skips_completed_or_duplicate_attempts() {
        let mut status = nip05_status_from_profile(Some(&profile(false)));
        assert!(should_auto_verify_nip05(&status, "npub1test", None));
        assert!(!should_auto_verify_nip05(
            &status,
            "npub1test",
            Some("npub1test|player@example.com")
        ));
        status.verified = true;
        status.status = "verified".to_string();
        assert!(!should_auto_verify_nip05(&status, "npub1test", None));

        let invalid = default_nip05_status(Some("not-an-identifier".into()));
        assert!(!should_auto_verify_nip05(&invalid, "npub1test", None));
    }

    #[test]
    fn an_account_without_a_display_name_shows_its_key_once() {
        // The heading falls back to the abbreviated key when there is no
        // display name, so the standalone key line has to stand down or the
        // same identifier renders twice.
        assert!(heading_is_public_key(None));

        let npub = "npub1kvl9ev2qqqqqqqqqqqqqqqqqqqqqqqqqqqqyqesnwt4";
        assert_eq!(
            profile_display_name(None, Some(npub)),
            abbreviate_public_key(npub),
            "heading and key line must agree character-for-character"
        );

        // A profile record that exists but carries no name still headings on
        // the key. UserProfile::display() falls back to the key internally, so
        // testing through it would wrongly report a name here.
        let nameless = UserProfile {
            npub: npub.into(),
            ..UserProfile::default()
        };
        assert!(heading_is_public_key(Some(&nameless)));
        assert_eq!(nameless.display(), abbreviate_public_key(npub));

        let named = UserProfile {
            npub: npub.into(),
            display_name: Some("Astra".into()),
            ..UserProfile::default()
        };
        assert!(!heading_is_public_key(Some(&named)));

        let name_only = UserProfile {
            npub: npub.into(),
            name: Some("astra".into()),
            ..UserProfile::default()
        };
        assert!(!heading_is_public_key(Some(&name_only)));
    }

    #[test]
    fn identifiers_abbreviate_identically_everywhere() {
        // Two helpers previously split at the same 12/8 boundary but joined
        // with different ellipsis characters, so one key rendered two ways.
        for value in [
            "npub1kvl9ev2qqqqqqqqqqqqqqqqqqqqqqqqqqqqyqesnwt4",
            "npub1short",
            "",
        ] {
            assert_eq!(abbreviate_public_key(value), npub_fallback_label(value));
        }
    }

    #[test]
    fn unsupported_profile_editing_is_omitted() {
        let source = include_str!("profile.rs");
        assert!(!source.contains(concat!("Edit ", "Profile")));
        assert!(!source.contains(concat!("Save ", "Profile")));
        assert!(!source.contains(concat!("Follow ", "profile")));
        assert!(!source.contains(concat!("Message ", "profile")));
    }

    #[test]
    fn profile_state_keeps_signed_out_loading_missing_and_error_distinct() {
        assert_eq!(
            profile_metadata_state(None, None, false, false),
            ProfileMetadataState::SignedOut
        );
        assert_eq!(
            profile_metadata_state(Some("npub"), None, true, false),
            ProfileMetadataState::Loading
        );
        assert_eq!(
            profile_metadata_state(Some("npub"), None, false, true),
            ProfileMetadataState::Error
        );
        assert_eq!(
            profile_metadata_state(Some("npub"), None, false, false),
            ProfileMetadataState::Missing
        );
        let mut loaded = UserProfile::default();
        loaded.name = Some("Player".into());
        assert_eq!(
            profile_metadata_state(Some("npub"), Some(&loaded), false, false),
            ProfileMetadataState::Ready
        );
        assert_eq!(
            profile_metadata_state(Some("npub"), Some(&loaded), false, true),
            ProfileMetadataState::ReadyWithError
        );

        let about_only = UserProfile {
            about: Some("Biography".into()),
            ..UserProfile::default()
        };
        assert_eq!(
            profile_metadata_state(Some("npub"), Some(&about_only), false, false),
            ProfileMetadataState::Ready
        );
    }

    #[test]
    fn profile_metadata_must_match_the_active_account() {
        assert!(profile_for_account(Some(profile(false)), Some("npub1test")).is_some());
        assert!(profile_for_account(Some(profile(false)), Some("npub1other")).is_none());
    }

    #[test]
    fn profile_media_and_links_require_public_https_urls() {
        assert_eq!(
            valid_public_url(Some("https://cdn.example.org/avatar.png".into())),
            Some("https://cdn.example.org/avatar.png".into())
        );
        assert_eq!(valid_public_url(Some("http://example.org".into())), None);
        assert_eq!(
            valid_public_url(Some("https://127.0.0.1/image".into())),
            None
        );
        assert_eq!(
            valid_public_url(Some("https://localhost/image".into())),
            None
        );
        assert_eq!(
            valid_public_url(Some("https://[fe80::1]/image".into())),
            None
        );
    }

    #[test]
    fn display_name_and_public_key_fallbacks_are_safe() {
        assert_eq!(
            profile_display_name(None, Some("npub1abcdefghijklmnop")),
            npub_fallback_label("npub1abcdefghijklmnop")
        );
        assert_eq!(abbreviate_public_key("short-key"), "short-key");
        // The separator is the shared label's `...`, not the `…` this helper
        // used to emit on its own; one key must not render two ways.
        assert_eq!(
            abbreviate_public_key("npub1abcdefghijklmnopqrstuvwxyz0123456789"),
            "npub1abcdefg...23456789"
        );
    }

    #[test]
    fn publisher_status_requires_current_nip99_author_evidence() {
        let mut authored = serde_json::from_value::<GameListing>(serde_json::json!({
            "id": "game",
            "source": "nip99_listing",
            "title": "Game",
            "description": "Description",
            "publisher_npub": "npub1author",
            "created_at": 1
        }))
        .expect("listing");
        let mut legacy = authored.clone();
        legacy.id = "legacy".into();
        legacy.source = crate::models::ListingSource::Legacy;
        let mut other = authored.clone();
        other.id = "other".into();
        other.publisher_npub = "npub1other".into();

        let mut result =
            current_user_listings(vec![authored.clone(), legacy, other], "npub1author");
        assert_eq!(result.len(), 1);
        assert_eq!(result.remove(0).id, authored.id);
    }

    #[test]
    fn reviews_are_unavailable_not_successfully_empty() {
        assert!(reviews_unavailable_message().contains("no authoritative review query"));
        assert!(!reviews_unavailable_message().contains("No reviews yet"));
    }
}
