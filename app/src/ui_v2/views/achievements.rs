//! Achievements view for displaying verified NIP-58 earned badges.

use crate::models::EarnedBadgeSummary;
use crate::tauri_bridge::{fetch_earned_badges, get_cached_earned_badges};
use crate::ui_v2::components::{artwork_state_from_url, ArtworkRole, GameArtwork};
use crate::AuthContext;
use gloo_timers::future::TimeoutFuture;
use leptos::prelude::*;
use tracing::{info, warn};
use wasm_bindgen_futures::spawn_local;

#[derive(Debug, Clone)]
enum AchievementsState {
    Loading,
    SignedOut,
    Empty,
    Error,
    Ready(Vec<EarnedBadgeSummary>),
    #[cfg(feature = "web")]
    WebUnavailable,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_award_date_returns_unix_prefix() {
        assert_eq!(format_award_date(1_714_633_600), "1714633600");
    }

    #[test]
    fn stale_generation_is_rejected() {
        assert!(!should_apply_generation(2, 3));
        assert!(should_apply_generation(3, 3));
    }

    #[test]
    fn achievements_error_message_is_sanitized() {
        assert_eq!(
            achievements_error_message(),
            "Could not load achievements. Please try again."
        );
    }

    #[test]
    fn achievement_state_messages_are_truthful() {
        assert_eq!(achievements_empty_message(), "No verified badges yet.");
        assert!(!achievements_error_message().contains("relay"));
    }

    #[test]
    fn cached_achievements_remain_visible_on_refresh_error() {
        assert!(should_preserve_ready_state(&AchievementsState::Ready(
            Vec::new()
        )));
        assert!(!should_preserve_ready_state(&AchievementsState::Loading));
    }

    #[test]
    fn badge_fallback_and_visibility_are_truthful() {
        let mut badge = sample_badge();
        badge.definition.name = Some("  ".to_string());
        badge.visible_on_profile = false;

        assert_eq!(badge_display_name(&badge), "first-clear");
        assert_eq!(profile_visibility_label(&badge), "Not selected for profile");
    }

    #[test]
    fn badge_media_rejects_non_public_schemes() {
        assert_eq!(
            valid_badge_image_url("https://cdn.example.org/badge.png".into()),
            Some("https://cdn.example.org/badge.png".into())
        );
        assert_eq!(valid_badge_image_url("javascript:alert(1)".into()), None);
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
    fn relay_refresh_yields_only_after_cached_render() {
        assert!(should_yield_before_relay_refresh(true));
        assert!(!should_yield_before_relay_refresh(false));
    }

    fn sample_badge() -> EarnedBadgeSummary {
        use crate::models::{BadgeAward, BadgeDefinition};

        EarnedBadgeSummary {
            definition: BadgeDefinition {
                coordinate: "30009:issuer:first-clear".to_string(),
                issuer_pubkey: "issuer".to_string(),
                badge_id: "first-clear".to_string(),
                name: None,
                description: None,
                image_url: None,
                image_dimensions: None,
                thumb_url: None,
                thumb_dimensions: None,
                relay_url: None,
                event_id: "definition".to_string(),
                created_at: 1,
            },
            award: BadgeAward {
                event_id: "award".to_string(),
                issuer_pubkey: "issuer".to_string(),
                recipient_pubkey: "recipient".to_string(),
                badge_coordinate: "30009:issuer:first-clear".to_string(),
                relay_url: None,
                created_at: 2,
            },
            visible_on_profile: true,
        }
    }
}

fn format_award_date(created_at: u64) -> String {
    created_at.to_string()
}

fn next_generation(generation: u64) -> u64 {
    generation.saturating_add(1)
}

fn should_apply_generation(request_generation: u64, current_generation: u64) -> bool {
    request_generation == current_generation
}

fn achievements_error_message() -> &'static str {
    "Could not load achievements. Please try again."
}

fn achievements_empty_message() -> &'static str {
    "No verified badges yet."
}

fn should_preserve_ready_state(state: &AchievementsState) -> bool {
    matches!(state, AchievementsState::Ready(_))
}

fn should_yield_before_relay_refresh(cached_rendered: bool) -> bool {
    cached_rendered
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

fn badge_display_name(badge: &EarnedBadgeSummary) -> String {
    badge
        .definition
        .name
        .clone()
        .filter(|name| !name.trim().is_empty())
        .unwrap_or_else(|| badge.definition.badge_id.clone())
}

fn profile_visibility_label(badge: &EarnedBadgeSummary) -> &'static str {
    if badge.visible_on_profile {
        "Visible on profile"
    } else {
        "Not selected for profile"
    }
}

fn badge_image(badge: &EarnedBadgeSummary) -> Option<String> {
    badge
        .definition
        .thumb_url
        .clone()
        .or_else(|| badge.definition.image_url.clone())
        .and_then(valid_badge_image_url)
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

#[component]
pub fn AchievementsView() -> impl IntoView {
    let auth = use_context::<AuthContext>().expect("AuthContext not provided");
    let state = RwSignal::new(AchievementsState::Loading);
    let request_generation = RwSignal::new(0_u64);
    let refresh_generation = RwSignal::new(0_u64);
    let refresh_warning = RwSignal::new(false);
    let state_profile = RwSignal::new(String::new());
    let profile_auth = auth.clone();
    let profile_pubkey = Signal::derive(move || profile_auth.npub.get().unwrap_or_default());
    let loading_auth = auth.clone();
    let auth_loading = Signal::derive(move || loading_auth.is_loading.get());
    let auth_error = Signal::derive(move || auth.error.get().is_some());

    #[cfg(feature = "web")]
    {
        state.set(AchievementsState::WebUnavailable);
    }

    #[cfg(not(feature = "web"))]
    Effect::new(move |_| {
        let _ = refresh_generation.get();
        let generation = next_generation(request_generation.get_untracked());
        request_generation.set(generation);

        if auth_loading.get() {
            state.set(AchievementsState::Loading);
            return;
        }

        let requested_profile_pubkey = profile_pubkey.get();
        if requested_profile_pubkey.is_empty() {
            state.set(if auth_error.get() {
                AchievementsState::Error
            } else {
                AchievementsState::SignedOut
            });
            state_profile.set(String::new());
            refresh_warning.set(false);
            return;
        }

        let account_changed = state_profile.get_untracked() != requested_profile_pubkey;
        state_profile.set(requested_profile_pubkey.clone());
        if account_changed || !should_preserve_ready_state(&state.get_untracked()) {
            state.set(AchievementsState::Loading);
        }
        refresh_warning.set(false);

        spawn_local(async move {
            let mut cached_rendered = false;
            match get_cached_earned_badges(requested_profile_pubkey.clone()).await {
                Ok(cached) if !cached.is_empty() => {
                    let cached_count = cached.len();
                    if should_apply_generation(generation, request_generation.get_untracked()) {
                        state.set(AchievementsState::Ready(cached));
                        cached_rendered = true;
                        info!(
                            count = cached_count,
                            "rendered cached earned achievements before relay refresh"
                        );
                    }
                }
                Ok(_) => info!("no cached earned achievements found before relay refresh"),
                Err(error) => warn!("failed to load cached achievements: {}", error),
            }

            if should_yield_before_relay_refresh(cached_rendered) {
                TimeoutFuture::new(0).await;
            }

            match fetch_earned_badges(requested_profile_pubkey).await {
                Ok(badges) => {
                    info!(count = badges.len(), "relay refreshed earned achievements");
                    if should_apply_generation(generation, request_generation.get_untracked()) {
                        if badges.is_empty() {
                            if should_preserve_ready_state(&state.get_untracked()) {
                                refresh_warning.set(true);
                            } else {
                                state.set(AchievementsState::Empty);
                            }
                        } else {
                            state.set(AchievementsState::Ready(badges));
                            refresh_warning.set(false);
                        }
                    }
                }
                Err(error) => {
                    warn!("failed to fetch achievements: {}", error);
                    if should_apply_generation(generation, request_generation.get_untracked()) {
                        if !should_preserve_ready_state(&state.get_untracked()) {
                            state.set(AchievementsState::Error);
                        } else {
                            refresh_warning.set(true);
                        }
                    }
                }
            }
        });
    });

    view! {
        <section class="v2-achievements">
            <header class="v2-achievements-hero v2-panel-glass">
                <div class="v2-achievements-hero-mark" aria-hidden="true">
                    <span class="material-symbols-outlined">"military_tech"</span>
                </div>
                <div>
                    <p class="v2-store-kicker">"NIP-58 proof collection"</p>
                    <h1 class="v2-display">"Achievements"</h1>
                    <p>"Badges awarded to this profile and verified from signed Nostr events."</p>
                </div>
            </header>

            {move || match state.get() {
                AchievementsState::Loading => achievement_state_view(
                    "progress_activity",
                    "Loading achievements",
                    "Checking the local cache before refreshing from relays.",
                    false,
                ),
                AchievementsState::SignedOut => achievement_state_view(
                    "person_off",
                    "Sign in to view earned badges",
                    "NIP-58 awards are resolved for the active public key.",
                    false,
                ),
                AchievementsState::Empty => achievement_state_view(
                    "workspace_premium",
                    achievements_empty_message(),
                    "NIP-58 awards for this profile will appear here when they are found.",
                    false,
                ),
                AchievementsState::Error => view! {
                    <section class="v2-achievement-state v2-achievement-state-error v2-panel" role="alert">
                        <span class="material-symbols-outlined" aria-hidden="true">"cloud_off"</span>
                        <div><h2>"Achievements unavailable"</h2><p>{achievements_error_message()}</p></div>
                        <button class="v2-btn-secondary" on:click=move |_| refresh_generation.update(|value| *value = value.wrapping_add(1))>"Retry"</button>
                    </section>
                }.into_any(),
                #[cfg(feature = "web")]
                AchievementsState::WebUnavailable => achievement_state_view(
                    "desktop_windows",
                    "Desktop relay support required",
                    "Badge relay display is not available on the web target. No achievement data is shown.",
                    false,
                ),
                AchievementsState::Ready(badges) => view! {
                    <div class="v2-achievement-results">
                    <Show when=move || refresh_warning.get()>
                        <div class="v2-achievement-partial"><div role="status"><strong>"Cached badges remain visible"</strong><span>"The relay refresh did not return a usable replacement set."</span></div><button class="v2-btn-secondary" on:click=move |_| refresh_generation.update(|value| *value = value.wrapping_add(1))>"Retry"</button></div>
                    </Show>
                    <div class="v2-achievement-grid">
                        {badges
                            .into_iter()
                            .map(|badge| {
                                let image = badge_image(&badge);
                                let name = badge_display_name(&badge);
                                let description = badge.definition.description.clone();
                                let issuer_pubkey = short_pubkey(&badge.definition.issuer_pubkey);
                                let award_date = format_award_date(badge.award.created_at);
                                let visibility = profile_visibility_label(&badge);

                                view! {
                                    <article class="v2-achievement-card">
                                        <div class="v2-achievement-art"><GameArtwork title=name.clone() state=artwork_state_from_url(image) role=ArtworkRole::Thumbnail /></div>
                                        <div class="v2-achievement-copy">
                                            <p class="v2-store-kicker">"Verified award"</p>
                                            <h2>{name}</h2>
                                            <p>{description.unwrap_or_else(|| "No description provided.".to_string())}</p>
                                        </div>
                                        <dl class="v2-achievement-meta">
                                            <div><dt>"Issuer"</dt><dd>{issuer_pubkey}</dd></div>
                                            <div><dt>"Nostr timestamp"</dt><dd>{award_date}</dd></div>
                                            <div><dt>"Profile"</dt><dd>{visibility}</dd></div>
                                        </dl>
                                    </article>
                                }
                            })
                            .collect::<Vec<_>>()}
                    </div>
                    </div>
                }
                .into_any(),
            }}
        </section>
    }
}

fn achievement_state_view(
    icon: &'static str,
    title: &'static str,
    message: &'static str,
    is_error: bool,
) -> AnyView {
    view! {
        <section
            class="v2-achievement-state v2-panel"
            class:v2-achievement-state-error=is_error
            role=if is_error { "alert" } else { "status" }
        >
            <span class="material-symbols-outlined" aria-hidden="true">{icon}</span>
            <div><h2>{title}</h2><p>{message}</p></div>
        </section>
    }
    .into_any()
}
