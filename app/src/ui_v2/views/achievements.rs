//! Achievements view for displaying verified NIP-58 earned badges.

use crate::models::EarnedBadgeSummary;
use crate::tauri_bridge::{fetch_earned_badges, get_cached_earned_badges};
use crate::AuthContext;
use gloo_timers::future::TimeoutFuture;
use leptos::prelude::*;
use tracing::{info, warn};
use wasm_bindgen_futures::spawn_local;

#[derive(Debug, Clone)]
enum AchievementsState {
    Loading,
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
    fn relay_refresh_yields_only_after_cached_render() {
        assert!(should_yield_before_relay_refresh(true));
        assert!(!should_yield_before_relay_refresh(false));
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

fn should_yield_before_relay_refresh(cached_rendered: bool) -> bool {
    cached_rendered
}

fn short_pubkey(pubkey: &str) -> String {
    if pubkey.len() <= 12 {
        pubkey.to_string()
    } else {
        format!("{}…{}", &pubkey[..6], &pubkey[pubkey.len() - 6..])
    }
}

fn badge_display_name(badge: &EarnedBadgeSummary) -> String {
    badge
        .definition
        .name
        .clone()
        .unwrap_or_else(|| badge.definition.badge_id.clone())
}

fn badge_image(badge: &EarnedBadgeSummary) -> Option<String> {
    badge
        .definition
        .thumb_url
        .clone()
        .or_else(|| badge.definition.image_url.clone())
}

#[component]
pub fn AchievementsView() -> impl IntoView {
    let auth = use_context::<AuthContext>().expect("AuthContext not provided");
    let state = RwSignal::new(AchievementsState::Loading);
    let request_generation = RwSignal::new(0_u64);
    let profile_pubkey = Signal::derive(move || auth.npub.get().unwrap_or_default());

    #[cfg(feature = "web")]
    {
        state.set(AchievementsState::WebUnavailable);
    }

    #[cfg(not(feature = "web"))]
    Effect::new(move |_| {
        let generation = next_generation(request_generation.get_untracked());
        request_generation.set(generation);

        let requested_profile_pubkey = profile_pubkey.get();
        if requested_profile_pubkey.is_empty() {
            state.set(AchievementsState::Empty);
            return;
        }

        state.set(AchievementsState::Loading);

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
                            state.set(AchievementsState::Empty);
                        } else {
                            state.set(AchievementsState::Ready(badges));
                        }
                    }
                }
                Err(error) => {
                    warn!("failed to fetch achievements: {}", error);
                    if should_apply_generation(generation, request_generation.get_untracked()) {
                        if !matches!(state.get_untracked(), AchievementsState::Ready(_)) {
                            state.set(AchievementsState::Error);
                        }
                    }
                }
            }
        });
    });

    view! {
        <section class="v2-panel p-6 space-y-4">
                <div>
                    <h2 class="font-headline text-3xl font-bold tracking-tight">"Achievements"</h2>
                    <p class="text-on-surface-variant mt-2">
                        "Verified badges earned by this profile."
                    </p>
                </div>

                {move || match state.get() {
                    AchievementsState::Loading => {
                        view! { <p>"Loading achievements..."</p> }.into_any()
                    }
                    AchievementsState::Empty => {
                        view! { <p>"No verified badges yet."</p> }.into_any()
                    }
                    AchievementsState::Error => {
                        view! { <p>{achievements_error_message()}</p> }.into_any()
                    }
                    #[cfg(feature = "web")]
                    AchievementsState::WebUnavailable => view! {
                        <p class="badge-showcase-unavailable">
                            "Badge relay display is not yet available on the web target. Badges will appear here once web relay support is added."
                        </p>
                    }
                    .into_any(),
                    AchievementsState::Ready(badges) => view! {
                        <div class="grid grid-cols-1 md:grid-cols-2 xl:grid-cols-3 gap-4">
                            {badges
                                .into_iter()
                                .map(|badge| {
                                    let image = badge_image(&badge);
                                    let name = badge_display_name(&badge);
                                    let description = badge.definition.description.clone();
                                    let issuer_pubkey = short_pubkey(&badge.definition.issuer_pubkey);
                                    let award_date = format_award_date(badge.award.created_at);

                                    view! {
                                        <article class="bg-surface-container-high rounded-xl p-4 border border-outline-variant/10 space-y-3">
                                            {image
                                                .map(|src| {
                                                    view! {
                                                        <img src=src alt=name.clone() class="w-16 h-16 rounded-lg object-cover" />
                                                    }
                                                })}
                                            <div class="space-y-1">
                                                <h3 class="text-lg font-semibold">{name}</h3>
                                                <p class="text-sm text-on-surface-variant">
                                                    {description.unwrap_or_else(|| "No description provided.".to_string())}
                                                </p>
                                            </div>
                                            <div class="text-xs text-on-surface-variant space-y-1">
                                                <p>{format!("Issuer: {issuer_pubkey}")}</p>
                                                <p>{format!("Awarded: {award_date}")}</p>
                                            </div>
                                        </article>
                                    }
                                })
                                .collect::<Vec<_>>()}
                        </div>
                    }
                    .into_any(),
                }}
        </section>
    }
}
