use leptos::prelude::*;
#[cfg(target_arch = "wasm32")]
use wasm_bindgen::closure::Closure;
#[cfg(target_arch = "wasm32")]
use wasm_bindgen::JsCast;

use crate::models::EarnedBadgeSummary;

#[component]
pub fn BadgeEarnedModal(
    badge: Signal<Option<EarnedBadgeSummary>>,
    on_close: Callback<()>,
) -> impl IntoView {
    let show = Signal::derive(move || badge.get().is_some());

    #[cfg(target_arch = "wasm32")]
    let last_focused_element = RwSignal::new(None::<web_sys::Element>);

    let close = {
        let on_close = on_close.clone();
        move |_| on_close.run(())
    };

    let on_keydown = {
        let on_close = on_close.clone();
        move |event: leptos::ev::KeyboardEvent| {
            if should_close_on_key(&event.key()) && show.get() {
                on_close.run(());
            }
        }
    };

    Effect::new(move |_| {
        #[cfg(target_arch = "wasm32")]
        {
            if !show.get() {
                return;
            }

            if let Some(window) = web_sys::window() {
                if let Some(document) = window.document() {
                    let on_close = on_close.clone();
                    let callback = Closure::wrap(Box::new(move |event: web_sys::KeyboardEvent| {
                        if should_close_on_key(&event.key()) {
                            on_close.run(());
                        }
                    }) as Box<dyn FnMut(_)>);

                    let _ = document.add_event_listener_with_callback(
                        "keydown",
                        callback.as_ref().unchecked_ref(),
                    );

                    on_cleanup(move || {
                        if let Some(window) = web_sys::window() {
                            if let Some(document) = window.document() {
                                let _ = document.remove_event_listener_with_callback(
                                    "keydown",
                                    callback.as_ref().unchecked_ref(),
                                );
                            }
                        }
                    });
                }
            }
        }
    });

    Effect::new(move |_| {
        #[cfg(target_arch = "wasm32")]
        {
            if show.get() {
                if let Some(window) = web_sys::window() {
                    if let Some(document) = window.document() {
                        last_focused_element.set(document.active_element());

                        if let Some(close_button) = document
                            .get_element_by_id("badge-earned-modal-close-button")
                            .and_then(|el| el.dyn_into::<web_sys::HtmlElement>().ok())
                        {
                            let _ = close_button.focus();
                        }
                    }
                }
            } else if let Some(previous) = last_focused_element.get_untracked() {
                if let Ok(previous_focusable) = previous.dyn_into::<web_sys::HtmlElement>() {
                    let _ = previous_focusable.focus();
                }
            }
        }
    });

    view! {
        <Show when=move || show.get()>
            <div
                class=badge_modal_class_name("backdrop")
                tabindex="0"
                role="dialog"
                aria-modal="true"
                aria-labelledby=badge_modal_title_id()
                on:click=close
                on:keydown=on_keydown
            >
                <div class=badge_modal_class_name("panel") on:click=|ev| ev.stop_propagation()>
                    <button
                        id="badge-earned-modal-close-button"
                        class=badge_modal_class_name("close")
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
        <article class=badge_modal_class_name("content")>
            <h3 id=badge_modal_title_id()>"Achievement unlocked!"</h3>
            {image.map(|src| view! { <img src=src alt=name.clone() class=badge_modal_class_name("image") /> })}
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

fn badge_modal_title_id() -> &'static str {
    "badge-earned-modal-title"
}

fn badge_modal_class_name(suffix: &str) -> String {
    format!("badge-earned-modal-{suffix}")
}

fn should_close_on_key(_key: &str) -> bool {
    _key == "Escape"
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

    #[test]
    fn badge_modal_uses_neutral_semantic_identifiers() {
        assert_eq!(badge_modal_title_id(), "badge-earned-modal-title");
        assert_eq!(
            badge_modal_class_name("backdrop"),
            "badge-earned-modal-backdrop"
        );
        assert!(!badge_modal_class_name("panel").contains("nip49"));
    }

    #[test]
    fn should_close_on_key_only_for_escape() {
        assert!(should_close_on_key("Escape"));
        assert!(!should_close_on_key("Enter"));
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
