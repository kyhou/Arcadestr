use leptos::prelude::*;

use crate::models::EarnedBadgeSummary;

#[component]
pub fn BadgeEarnedModal(
    show: Signal<bool>,
    badge: Signal<Option<EarnedBadgeSummary>>,
    on_close: Callback<()>,
) -> impl IntoView {
    let close = move |_| on_close.run(());

    view! {
        <Show when=move || show.get()>
            <div class="badge-earned-modal-backdrop" on:click=close>
                <div class="badge-earned-modal-panel" on:click=|ev| ev.stop_propagation()>
                    <button
                        class="badge-earned-modal-close"
                        aria-label="Close badge earned modal"
                        on:click=close
                    >
                        "×"
                    </button>

                    {move || badge.get().map(render_badge_content)}
                </div>
            </div>
        </Show>
    }
}

fn render_badge_content(badge: EarnedBadgeSummary) -> impl IntoView {
    let image = preferred_badge_image(&badge);
    let name = badge_display_name(&badge);
    let description = badge
        .definition
        .description
        .clone()
        .unwrap_or_else(|| "No description provided.".to_string());

    view! {
        <article class="badge-earned-modal-content">
            <h3>"Achievement unlocked!"</h3>
            {image.map(|src| view! { <img src=src alt=name.clone() class="badge-earned-modal-image" /> })}
            <h4>{name}</h4>
            <p>{description}</p>
            <p>
                <strong>"Issuer: "</strong>
                <span>{badge.definition.issuer_pubkey.clone()}</span>
            </p>
        </article>
    }
}

fn preferred_badge_image(_badge: &EarnedBadgeSummary) -> Option<String> {
    _badge
        .definition
        .thumb_url
        .clone()
        .or_else(|| _badge.definition.image_url.clone())
}

fn badge_display_name(_badge: &EarnedBadgeSummary) -> String {
    _badge
        .definition
        .name
        .clone()
        .filter(|name| !name.trim().is_empty())
        .unwrap_or_else(|| _badge.definition.badge_id.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{BadgeAward, BadgeDefinition};

    #[test]
    fn preferred_badge_image_uses_thumb_before_image() {
        let badge = sample_badge(Some("thumb"), Some("image"), Some("name"));

        let image = preferred_badge_image(&badge);

        assert_eq!(image.as_deref(), Some("thumb"));
    }

    #[test]
    fn badge_display_name_falls_back_to_badge_id() {
        let badge = sample_badge(None, None, None);

        let name = badge_display_name(&badge);

        assert_eq!(name, "first-clear");
    }

    fn sample_badge(
        thumb_url: Option<&str>,
        image_url: Option<&str>,
        name: Option<&str>,
    ) -> EarnedBadgeSummary {
        EarnedBadgeSummary {
            definition: BadgeDefinition {
                coordinate: "30009:issuer:first-clear".to_string(),
                issuer_pubkey: "issuer_pubkey".to_string(),
                badge_id: "first-clear".to_string(),
                name: name.map(ToString::to_string),
                description: Some("Desc".to_string()),
                image_url: image_url.map(ToString::to_string),
                image_dimensions: None,
                thumb_url: thumb_url.map(ToString::to_string),
                thumb_dimensions: None,
                relay_url: None,
                event_id: "def_event".to_string(),
                created_at: 1,
            },
            award: BadgeAward {
                event_id: "award_event".to_string(),
                issuer_pubkey: "issuer_pubkey".to_string(),
                recipient_pubkey: "recipient".to_string(),
                badge_coordinate: "30009:issuer:first-clear".to_string(),
                relay_url: None,
                created_at: 2,
            },
            visible_on_profile: true,
        }
    }
}
