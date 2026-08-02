//! Profile badge showcase for NIP-58 achievements.

use crate::models::{EarnedBadgeSummary, ProfileBadgeEntry};
use crate::tauri_bridge::{
    fetch_earned_badges, fetch_profile_badges, get_cached_earned_badges, get_cached_profile_badges,
};
use crate::ui_v2::components::{artwork_state_from_url, ArtworkRole, GameArtwork};
use gloo_timers::future::TimeoutFuture;
use leptos::prelude::*;
use tracing::{info, warn};
use wasm_bindgen_futures::spawn_local;

const SHOWCASE_FALLBACK_LIMIT: usize = 8;

#[derive(Debug, Clone)]
enum BadgeShowcaseState {
    Loading,
    Empty,
    Error,
    Ready {
        badges: Vec<EarnedBadgeSummary>,
        source: BadgeShowcaseSource,
        refresh_warning: Option<BadgeShowcaseWarning>,
    },
    WebUnavailable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BadgeShowcaseSource {
    ProfileSelection,
    EarnedFallback,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BadgeShowcaseWarning {
    SelectionRefresh,
    EarnedFallbackRefresh,
}

#[component]
pub fn BadgeShowcase(profile_identifier: Signal<String>) -> impl IntoView {
    let state = RwSignal::new(BadgeShowcaseState::Loading);
    let request_generation = RwSignal::new(0_u64);

    #[cfg(feature = "web")]
    {
        state.set(BadgeShowcaseState::WebUnavailable);
    }

    #[cfg(not(feature = "web"))]
    Effect::new(move |_| {
        let requested_profile_identifier = profile_identifier.get();
        let generation = request_generation.get_untracked().saturating_add(1);
        request_generation.set(generation);
        if requested_profile_identifier.is_empty() {
            state.set(BadgeShowcaseState::Empty);
            return;
        }

        state.set(BadgeShowcaseState::Loading);
        spawn_local(async move {
            let mut cached_rendered = false;
            let cached_profile_result: Result<Vec<ProfileBadgeEntry>, String> =
                get_cached_profile_badges(requested_profile_identifier.clone()).await;

            match cached_profile_result {
                Ok(profile_entries) if !profile_entries.is_empty() => {
                    let cached_count = profile_entries.len();
                    if should_apply_generation(generation, request_generation.get_untracked()) {
                        state.set(BadgeShowcaseState::Ready {
                            badges: profile_entries_to_summaries(profile_entries),
                            source: BadgeShowcaseSource::ProfileSelection,
                            refresh_warning: None,
                        });
                        cached_rendered = true;
                        info!(
                            count = cached_count,
                            "rendered cached profile badge showcase before relay refresh"
                        );
                    }
                }
                Ok(_) => {
                    match get_cached_earned_badges(requested_profile_identifier.clone()).await {
                        Ok(earned) => {
                            let capped = cap_fallback_badges(earned);
                            if !capped.is_empty()
                                && should_apply_generation(
                                    generation,
                                    request_generation.get_untracked(),
                                )
                            {
                                let cached_count = capped.len();
                                state.set(BadgeShowcaseState::Ready {
                                    badges: capped,
                                    source: BadgeShowcaseSource::EarnedFallback,
                                    refresh_warning: None,
                                });
                                cached_rendered = true;
                                info!(
                                    count = cached_count,
                                    "rendered cached earned badge showcase before relay refresh"
                                );
                            }
                        }
                        Err(error) => warn!("failed to load cached badge showcase: {}", error),
                    }
                }
                Err(error) => warn!("failed to load cached profile badge selection: {}", error),
            }

            if should_yield_before_relay_refresh(cached_rendered) {
                TimeoutFuture::new(0).await;
            }

            let profile_result: Result<Vec<ProfileBadgeEntry>, String> =
                fetch_profile_badges(requested_profile_identifier.clone()).await;

            match profile_result {
                Ok(profile_entries) if !profile_entries.is_empty() => {
                    info!(
                        count = profile_entries.len(),
                        "relay refreshed profile badge showcase"
                    );
                    if should_apply_generation(generation, request_generation.get_untracked()) {
                        state.set(BadgeShowcaseState::Ready {
                            badges: profile_entries_to_summaries(profile_entries),
                            source: BadgeShowcaseSource::ProfileSelection,
                            refresh_warning: None,
                        });
                    }
                }
                Ok(_) => match fetch_earned_badges(requested_profile_identifier).await {
                    Ok(earned) => {
                        info!(
                            count = earned.len(),
                            "relay refreshed earned badge showcase fallback"
                        );
                        let capped = cap_fallback_badges(earned);
                        if should_apply_generation(generation, request_generation.get_untracked()) {
                            if capped.is_empty() {
                                state.set(BadgeShowcaseState::Empty);
                            } else {
                                state.set(BadgeShowcaseState::Ready {
                                    badges: capped,
                                    source: BadgeShowcaseSource::EarnedFallback,
                                    refresh_warning: None,
                                });
                            }
                        }
                    }
                    Err(error) => {
                        if should_apply_generation(generation, request_generation.get_untracked()) {
                            warn!("failed to refresh badge showcase: {}", error);
                            match state.get_untracked() {
                                BadgeShowcaseState::Ready { badges, source, .. } => {
                                    state.set(BadgeShowcaseState::Ready {
                                        badges,
                                        source,
                                        refresh_warning: Some(
                                            BadgeShowcaseWarning::EarnedFallbackRefresh,
                                        ),
                                    });
                                }
                                _ => state.set(BadgeShowcaseState::Error),
                            }
                        }
                    }
                },
                Err(error) => {
                    warn!("failed to refresh profile badge selection: {}", error);
                    if should_apply_generation(generation, request_generation.get_untracked()) {
                        match state.get_untracked() {
                            BadgeShowcaseState::Ready { badges, source, .. } => {
                                state.set(BadgeShowcaseState::Ready {
                                    badges,
                                    source,
                                    refresh_warning: Some(BadgeShowcaseWarning::SelectionRefresh),
                                });
                            }
                            _ => state.set(BadgeShowcaseState::Error),
                        }
                    }
                }
            }
        });
    });

    view! {
        <section class="v2-panel v2-badge-showcase">
            <div class="v2-badge-showcase-header">
                <span class="material-symbols-outlined" aria-hidden="true">"workspace_premium"</span>
                <div><p class="v2-store-kicker">"NIP-58"</p><h2>"Achievements"</h2></div>
            </div>
            {move || match state.get() {
                BadgeShowcaseState::Loading => {
                    view! { <p class="v2-badge-showcase-state" role="status">"Loading verified badges..."</p> }.into_any()
                }
                BadgeShowcaseState::Empty => {
                    view! { <p class="v2-badge-showcase-state">{badge_showcase_empty_message()}</p> }.into_any()
                }
                BadgeShowcaseState::Error => {
                    view! { <p class="v2-badge-showcase-state v2-badge-showcase-error" role="alert">{badge_showcase_error_message()}</p> }.into_any()
                }
                BadgeShowcaseState::WebUnavailable => view! {
                    <p class="v2-badge-showcase-state">
                        "Badge relay display is not available on the web target. No badge data is shown."
                    </p>
                }
                .into_any(),
                BadgeShowcaseState::Ready { badges, source, refresh_warning } => view! {
                    <div>
                        <p class="v2-badge-showcase-source">{showcase_source_message(source)}</p>
                        {refresh_warning.map(|warning| view! { <p class="v2-badge-showcase-warning" role="status">{showcase_warning_message(warning)}</p> })}
                        <div class="v2-badge-showcase-row">
                            {badges.into_iter().map(render_badge_chip).collect::<Vec<_>>()}
                        </div>
                    </div>
                }
                .into_any(),
            }}
        </section>
    }
}

fn profile_entries_to_summaries(mut entries: Vec<ProfileBadgeEntry>) -> Vec<EarnedBadgeSummary> {
    entries.sort_by_key(|entry| entry.display_order);

    entries
        .into_iter()
        .map(|entry| EarnedBadgeSummary {
            definition: entry.definition,
            award: entry.award,
            visible_on_profile: entry.visible,
        })
        .collect()
}

fn cap_fallback_badges(mut earned: Vec<EarnedBadgeSummary>) -> Vec<EarnedBadgeSummary> {
    earned.truncate(SHOWCASE_FALLBACK_LIMIT);
    earned
}

fn should_yield_before_relay_refresh(cached_rendered: bool) -> bool {
    cached_rendered
}

fn badge_showcase_empty_message() -> &'static str {
    "No showcased or earned badges were resolved for this profile."
}

fn badge_showcase_error_message() -> &'static str {
    "Badge showcase data is unavailable. Previously verified badges are not being inferred."
}

fn showcase_source_message(source: BadgeShowcaseSource) -> &'static str {
    match source {
        BadgeShowcaseSource::ProfileSelection => {
            "Selected by this profile in its current NIP-58 showcase."
        }
        BadgeShowcaseSource::EarnedFallback => {
            "Verified earned badges shown as a fallback; these are not selected for the profile showcase."
        }
    }
}

fn showcase_warning_message(warning: BadgeShowcaseWarning) -> &'static str {
    match warning {
        BadgeShowcaseWarning::SelectionRefresh => {
            "Previously resolved badges remain visible because showcase selection could not be refreshed."
        }
        BadgeShowcaseWarning::EarnedFallbackRefresh => {
            "Previously resolved badges remain visible because earned badge fallback data could not be refreshed."
        }
    }
}

fn should_apply_generation(request_generation: u64, current_generation: u64) -> bool {
    request_generation == current_generation
}

fn profile_visibility_label(visible_on_profile: bool) -> &'static str {
    if visible_on_profile {
        "Visible on profile"
    } else {
        "Not selected for profile"
    }
}

fn render_badge_chip(badge: EarnedBadgeSummary) -> impl IntoView {
    let image = badge
        .definition
        .thumb_url
        .clone()
        .or_else(|| badge.definition.image_url.clone())
        .and_then(valid_badge_image_url);
    let name = badge
        .definition
        .name
        .clone()
        .filter(|name| !name.trim().is_empty())
        .unwrap_or_else(|| badge.definition.badge_id.clone());
    let visibility = profile_visibility_label(badge.visible_on_profile);

    view! {
        <article class="v2-badge-chip">
            <div class="v2-badge-chip-art">
                <GameArtwork title=name.clone() state=artwork_state_from_url(image) role=ArtworkRole::Thumbnail />
            </div>
            <div>
                <strong>{name}</strong>
                <span>{format!("Issued by {}", short_pubkey(&badge.definition.issuer_pubkey))}</span>
                <span class="v2-badge-chip-visibility">{visibility}</span>
            </div>
        </article>
    }
}

fn valid_badge_image_url(value: String) -> Option<String> {
    let parsed = url::Url::parse(value.trim()).ok()?;
    if parsed.scheme() != "https" || !is_public_url_host(&parsed) {
        return None;
    }
    Some(value.trim().to_string())
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

fn short_pubkey(pubkey: &str) -> String {
    let chars = pubkey.chars().collect::<Vec<_>>();
    if chars.len() <= 12 {
        pubkey.to_string()
    } else {
        format!(
            "{}…{}",
            chars[..6].iter().collect::<String>(),
            chars[chars.len() - 6..].iter().collect::<String>()
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{BadgeAward, BadgeDefinition};

    #[test]
    fn fallback_caps_badges_to_eight() {
        let earned = (0..10)
            .map(|i| EarnedBadgeSummary {
                definition: BadgeDefinition {
                    coordinate: format!("30009:issuer:badge-{i}"),
                    issuer_pubkey: "issuer_pubkey".to_string(),
                    badge_id: format!("badge-{i}"),
                    name: None,
                    description: None,
                    image_url: None,
                    image_dimensions: None,
                    thumb_url: None,
                    thumb_dimensions: None,
                    relay_url: None,
                    event_id: format!("def-{i}"),
                    created_at: i,
                },
                award: BadgeAward {
                    event_id: format!("award-{i}"),
                    issuer_pubkey: "issuer_pubkey".to_string(),
                    recipient_pubkey: "recipient_pubkey".to_string(),
                    badge_coordinate: format!("30009:issuer:badge-{i}"),
                    relay_url: None,
                    created_at: i,
                },
                visible_on_profile: false,
            })
            .collect::<Vec<_>>();

        let capped = cap_fallback_badges(earned);

        assert_eq!(capped.len(), SHOWCASE_FALLBACK_LIMIT);
        assert_eq!(capped[0].definition.badge_id, "badge-0");
        assert_eq!(capped[7].definition.badge_id, "badge-7");
    }

    #[test]
    fn relay_refresh_yields_only_after_cached_showcase_render() {
        assert!(should_yield_before_relay_refresh(true));
        assert!(!should_yield_before_relay_refresh(false));
    }

    #[test]
    fn stale_showcase_generation_is_rejected() {
        assert!(!should_apply_generation(4, 5));
        assert!(should_apply_generation(5, 5));
    }

    #[test]
    fn fallback_visibility_does_not_claim_profile_selection() {
        assert_eq!(profile_visibility_label(false), "Not selected for profile");
        assert_eq!(profile_visibility_label(true), "Visible on profile");
    }

    #[test]
    fn showcase_empty_and_error_messages_are_safe() {
        assert_eq!(
            badge_showcase_empty_message(),
            "No showcased or earned badges were resolved for this profile."
        );
        assert!(!badge_showcase_error_message().contains("relay"));
    }

    #[test]
    fn selected_and_fallback_badges_are_never_conflated() {
        assert!(
            showcase_source_message(BadgeShowcaseSource::ProfileSelection)
                .contains("Selected by this profile")
        );
        assert!(
            showcase_source_message(BadgeShowcaseSource::EarnedFallback).contains("not selected")
        );
    }

    #[test]
    fn badge_media_accepts_only_public_http_schemes() {
        assert_eq!(
            valid_badge_image_url("https://cdn.example.org/badge.png".into()),
            Some("https://cdn.example.org/badge.png".into())
        );
        assert_eq!(valid_badge_image_url("javascript:alert(1)".into()), None);
        assert_eq!(
            valid_badge_image_url("http://cdn.example.org/badge.png".into()),
            None
        );
        assert_eq!(
            valid_badge_image_url("https://127.0.0.1/badge.png".into()),
            None
        );
        assert_eq!(
            valid_badge_image_url("https://localhost/badge.png".into()),
            None
        );
        assert_eq!(
            valid_badge_image_url("https://[fe80::1]/badge.png".into()),
            None
        );
        assert_eq!(
            valid_badge_image_url("file:///tmp/private.png".into()),
            None
        );
    }

    #[test]
    fn pubkey_abbreviation_handles_non_ascii_input() {
        assert_eq!(short_pubkey("abcdeféghijklmnop"), "abcdef…klmnop");
    }

    #[test]
    fn refresh_warning_identifies_the_failed_evidence_source() {
        assert!(
            showcase_warning_message(BadgeShowcaseWarning::SelectionRefresh).contains("selection")
        );
        assert!(
            showcase_warning_message(BadgeShowcaseWarning::EarnedFallbackRefresh)
                .contains("fallback")
        );
    }
}
