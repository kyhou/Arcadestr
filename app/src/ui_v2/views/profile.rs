use leptos::prelude::*;
use wasm_bindgen_futures::spawn_local;

use crate::components::BadgeShowcase;
use crate::models::{npub_fallback_label, GameListing, Nip05Status, UserProfile};
use crate::tauri_bridge::invoke_verify_nip05;
use crate::ui_v2::views::marketplace_loader::use_marketplace_listings_with_limit;
use crate::ui_v2::views::{use_fallback_cover, valid_cover_url, FALLBACK_COVER};
use crate::AuthContext;

#[path = "../../components/nip05_badge.rs"]
mod nip05_badge;
use nip05_badge::Nip05Badge;

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
    if identifier.is_empty() || expected_npub.is_empty() {
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

fn valid_public_url(value: Option<String>) -> Option<String> {
    value.and_then(|value| {
        let parsed = url::Url::parse(value.trim()).ok()?;
        matches!(parsed.scheme(), "http" | "https").then(|| value.trim().to_string())
    })
}

#[component]
pub fn ProfileV2View(
    on_open_publish: Callback<()>,
    on_open_listing: Callback<GameListing>,
) -> impl IntoView {
    let auth = use_context::<AuthContext>().expect("AuthContext not provided");
    let verification_available = !cfg!(feature = "web");
    let marketplace_state = use_marketplace_listings_with_limit(50);
    let nip05_status = RwSignal::new(nip05_status_from_profile(
        auth.profile.get_untracked().as_ref(),
    ));
    let last_auto_nip05_attempt = RwSignal::new(None::<String>);

    let listings = marketplace_state.listings;
    let listings_auth = auth.clone();
    let my_listings = Signal::derive(move || {
        let Some(npub) = listings_auth.npub.get() else {
            return Vec::new();
        };
        listings
            .get()
            .into_iter()
            .filter(|listing| listing.publisher_npub == npub)
            .collect::<Vec<_>>()
    });
    let current_npub = Signal::derive(move || auth.npub.get().unwrap_or_default());
    let display_name = Signal::derive(move || {
        auth.profile
            .get()
            .map(|profile| profile.display())
            .or_else(|| auth.npub.get().map(|npub| npub_fallback_label(&npub)))
            .unwrap_or_else(|| "Unknown account".to_string())
    });
    let profile = Signal::derive(move || auth.profile.get());

    let auth_for_auto_verify = auth.clone();
    Effect::new(move |_| {
        nip05_status.set(nip05_status_from_profile(profile.get().as_ref()));
        last_auto_nip05_attempt.set(None);
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
                Ok(status) => nip05_status.set(status),
                Err(error) => nip05_status.set(Nip05Status {
                    status: "failed".to_string(),
                    message: format!("Verification failed: {error}"),
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
                Ok(status) => nip05_status.set(status),
                Err(error) => nip05_status.set(Nip05Status {
                    status: "failed".to_string(),
                    message: format!("Verification failed: {error}"),
                    ..default_nip05_status(Some(identifier))
                }),
            }
        });
    });

    view! {
        <section class="v2-profile-wrap">
            <header class="v2-profile-hero v2-panel-glass">
                <div class="v2-profile-avatar-wrap">
                    {move || {
                        let value = profile.get();
                        let picture = value.as_ref().and_then(|profile| profile.picture.clone());
                        match picture {
                            Some(url) => view! { <img class="v2-profile-avatar" src=url alt="" on:error=use_fallback_cover /> }.into_any(),
                            None => view! {
                                <div class="v2-profile-avatar v2-profile-avatar-fallback" aria-hidden="true">
                                    {move || display_name.get().chars().next().unwrap_or('?').to_uppercase().to_string()}
                                </div>
                            }.into_any(),
                        }
                    }}
                </div>
                <div class="v2-profile-identity">
                    <p class="v2-store-kicker">"Public profile"</p>
                    <h1 class="v2-display">{move || display_name.get()}</h1>
                    {move || profile.get().and_then(|profile| profile.name).filter(|name| !name.trim().is_empty()).map(|name| view! { <p class="v2-profile-username">{name}</p> })}
                    <p class="v2-profile-npub">{move || current_npub.get()}</p>
                    {if verification_available {
                        view! { <Nip05Badge status=nip05_status.into() on_verify=on_verify_nip05 /> }.into_any()
                    } else {
                        view! {
                            <p class="v2-profile-muted">
                                {move || {
                                    let identifier = nip05_status_identifier(&nip05_status.get());
                                    if identifier.is_empty() {
                                        "No NIP-05 identifier is available.".to_string()
                                    } else {
                                        format!("{identifier} · Verification unavailable on standalone web")
                                    }
                                }}
                            </p>
                        }.into_any()
                    }}
                </div>
            </header>

            <div class="v2-profile-layout">
                <main class="v2-profile-main">
                    <section class="v2-profile-card v2-panel">
                        <h2>"About"</h2>
                        {move || profile.get().and_then(|profile| profile.about).filter(|about| !about.trim().is_empty()).map(|about| view! { <p class="v2-profile-about">{about}</p> }).unwrap_or_else(|| view! { <p class="v2-profile-muted">{"No public biography is available.".to_string()}</p> })}
                        <dl class="v2-profile-metadata">
                            {move || profile.get().and_then(|profile| valid_public_url(profile.website)).map(|website| {
                                let href = website.clone();
                                view! { <div><dt>"Website"</dt><dd><a href=href target="_blank" rel="noopener noreferrer">{website}</a></dd></div> }
                            })}
                            {move || profile.get().and_then(|profile| profile.lud16).filter(|value| !value.trim().is_empty()).map(|value| view! { <div><dt>"Lightning address"</dt><dd>{value}</dd></div> })}
                            {move || profile.get().and_then(|profile| profile.nip05).filter(|value| !value.trim().is_empty()).map(|value| view! {
                                <div><dt>"NIP-05 identifier"</dt><dd>{value}<span>{move || if nip05_status.get().verified { "Verified" } else { "Not verified" }}</span></dd></div>
                            })}
                        </dl>
                        <p class="v2-profile-readonly-note">"Profile editing is not available in this client. This page reflects signed public metadata fetched from relays."</p>
                    </section>

                    <section class="v2-profile-card v2-panel">
                        <div class="v2-profile-section-header">
                            <div><p class="v2-store-kicker">"Publisher catalog"</p><h2>"Published games"</h2></div>
                            <button class="v2-btn-secondary" on:click=move |_| on_open_publish.run(())>"Open publishing"</button>
                        </div>
                        {move || if marketplace_state.loading.get() && my_listings.get().is_empty() {
                            view! { <p class="v2-profile-muted" role="status">"Loading published listings..."</p> }.into_any()
                        } else if marketplace_state.error.get().is_some() && my_listings.get().is_empty() {
                            let error = marketplace_state.error.get().unwrap_or_default();
                            view! { <p class="v2-settings-alert v2-settings-alert-error">{error}</p> }.into_any()
                        } else if my_listings.get().is_empty() {
                            view! { <p class="v2-profile-muted">"No published listings were found for this account."</p> }.into_any()
                        } else {
                            let refresh_error = marketplace_state.error.get();
                            view! {
                                <div>
                                    {refresh_error.map(|_| view! { <p class="v2-settings-alert" role="status">"Relay refresh failed; cached published games remain available."</p> })}
                                    <div class="v2-profile-listings-grid">
                                        {my_listings.get().into_iter().map(|listing| {
                                            let selected = listing.clone();
                                            let image = valid_cover_url(&listing.images).unwrap_or_else(|| FALLBACK_COVER.to_string());
                                            view! {
                                                <button class="v2-profile-listing-card" on:click=move |_| on_open_listing.run(selected.clone())>
                                                    <img src=image alt="" on:error=use_fallback_cover />
                                                    <span><strong>{listing.title}</strong><small>{format!("{} {}", listing.price, listing.currency)}</small></span>
                                                </button>
                                            }
                                        }).collect::<Vec<_>>()}
                                    </div>
                                </div>
                            }.into_any()
                        }}
                    </section>
                </main>

                <aside class="v2-profile-badges">
                    <BadgeShowcase profile_identifier=current_npub />
                </aside>
            </div>
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
    }

    #[test]
    fn unsupported_profile_editing_is_omitted() {
        let source = include_str!("profile.rs");
        assert!(!source.contains(concat!("Edit ", "Profile")));
        assert!(!source.contains(concat!("Save ", "Profile")));
    }
}
