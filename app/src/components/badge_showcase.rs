//! Profile badge showcase for NIP-58 achievements.

use crate::models::{EarnedBadgeSummary, ProfileBadgeEntry};
use crate::tauri_bridge::{
    fetch_earned_badges, fetch_profile_badges, get_cached_earned_badges, get_cached_profile_badges,
};
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
    Ready(Vec<EarnedBadgeSummary>),
    WebUnavailable,
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
        if requested_profile_identifier.is_empty() {
            state.set(BadgeShowcaseState::Empty);
            return;
        }

        let generation = request_generation.get_untracked().saturating_add(1);
        request_generation.set(generation);

        state.set(BadgeShowcaseState::Loading);
        spawn_local(async move {
            let mut cached_rendered = false;
            let cached_profile_result: Result<Vec<ProfileBadgeEntry>, String> =
                get_cached_profile_badges(requested_profile_identifier.clone()).await;

            match cached_profile_result {
                Ok(profile_entries) if !profile_entries.is_empty() => {
                    let cached_count = profile_entries.len();
                    if generation == request_generation.get_untracked() {
                        state.set(BadgeShowcaseState::Ready(profile_entries_to_summaries(
                            profile_entries,
                        )));
                        cached_rendered = true;
                        info!(
                            count = cached_count,
                            "rendered cached profile badge showcase before relay refresh"
                        );
                    }
                }
                Ok(_) | Err(_) => {
                    match get_cached_earned_badges(requested_profile_identifier.clone()).await {
                        Ok(earned) => {
                            let capped = cap_fallback_badges(earned);
                            if !capped.is_empty()
                                && generation == request_generation.get_untracked()
                            {
                                let cached_count = capped.len();
                                state.set(BadgeShowcaseState::Ready(capped));
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
                    if generation == request_generation.get_untracked() {
                        state.set(BadgeShowcaseState::Ready(profile_entries_to_summaries(
                            profile_entries,
                        )));
                    }
                }
                Ok(_) | Err(_) => match fetch_earned_badges(requested_profile_identifier).await {
                    Ok(earned) => {
                        info!(
                            count = earned.len(),
                            "relay refreshed earned badge showcase fallback"
                        );
                        let capped = cap_fallback_badges(earned);
                        if generation == request_generation.get_untracked() {
                            if capped.is_empty() {
                                state.set(BadgeShowcaseState::Empty);
                            } else {
                                state.set(BadgeShowcaseState::Ready(capped));
                            }
                        }
                    }
                    Err(error) => {
                        if generation == request_generation.get_untracked() {
                            if !matches!(state.get_untracked(), BadgeShowcaseState::Ready(_)) {
                                warn!("failed to refresh badge showcase: {}", error);
                                state.set(BadgeShowcaseState::Error);
                            }
                        }
                    }
                },
            }
        });
    });

    view! {
        <section class="v2-panel v2-badge-showcase">
            <div class="v2-badge-showcase-header">
                <span class="material-symbols-outlined" aria-hidden="true">"workspace_premium"</span>
                <div><p class="v2-store-kicker">"NIP-58"</p><h3>"Achievements"</h3></div>
            </div>
            {move || match state.get() {
                BadgeShowcaseState::Loading => {
                    view! { <p class="v2-badge-showcase-state" role="status">"Loading verified badges..."</p> }.into_any()
                }
                BadgeShowcaseState::Empty => {
                    view! { <p class="v2-badge-showcase-state">"No verified badges are available for this profile."</p> }.into_any()
                }
                BadgeShowcaseState::Error => {
                    view! { <p class="v2-badge-showcase-state v2-badge-showcase-error" role="alert">"Could not load this badge showcase. Please try again later."</p> }.into_any()
                }
                BadgeShowcaseState::WebUnavailable => view! {
                    <p class="v2-badge-showcase-state">
                        "Badge relay display is not available on the web target. No badge data is shown."
                    </p>
                }
                .into_any(),
                BadgeShowcaseState::Ready(badges) => view! {
                    <div class="v2-badge-showcase-row">
                        {badges.into_iter().map(render_badge_chip).collect::<Vec<_>>()}
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

fn render_badge_chip(badge: EarnedBadgeSummary) -> impl IntoView {
    let image = badge
        .definition
        .thumb_url
        .clone()
        .or_else(|| badge.definition.image_url.clone());
    let name = badge
        .definition
        .name
        .clone()
        .unwrap_or_else(|| badge.definition.badge_id.clone());

    view! {
        <article class="v2-badge-chip">
            <div class="v2-badge-chip-art">
                {match image {
                    Some(src) => view! { <img src=src alt=name.clone() /> }.into_any(),
                    None => view! { <span class="material-symbols-outlined" aria-hidden="true">"military_tech"</span> }.into_any(),
                }}
            </div>
            <div>
                <strong>{name}</strong>
                <span>{format!("Issued by {}", short_pubkey(&badge.definition.issuer_pubkey))}</span>
            </div>
        </article>
    }
}

fn short_pubkey(pubkey: &str) -> String {
    if pubkey.len() <= 12 {
        pubkey.to_string()
    } else {
        format!("{}…{}", &pubkey[..6], &pubkey[pubkey.len() - 6..])
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
}
