use leptos::prelude::*;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ArtworkState {
    Loading,
    Missing,
    Failed,
    Available(String),
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ArtworkRole {
    #[default]
    Card,
    Hero,
    Thumbnail,
}

impl ArtworkRole {
    const fn class(self) -> &'static str {
        match self {
            Self::Card => "arc-artwork-card",
            Self::Hero => "arc-artwork-hero",
            Self::Thumbnail => "arc-artwork-thumbnail",
        }
    }
}

pub fn artwork_state_from_url(url: Option<String>) -> ArtworkState {
    match url.filter(|url| !url.trim().is_empty()) {
        Some(url) => ArtworkState::Available(url),
        None => ArtworkState::Missing,
    }
}

fn fallback_label(state: &ArtworkState) -> Option<&'static str> {
    match state {
        ArtworkState::Loading => Some("ARTWORK LOADING"),
        ArtworkState::Missing => Some("ARTWORK PENDING"),
        ArtworkState::Failed => Some("ARTWORK UNAVAILABLE"),
        ArtworkState::Available(_) => None,
    }
}

/// Missing hero art names who supplies it; every other case keeps the shared
/// role-agnostic label.
fn fallback_label_for(state: &ArtworkState, role: ArtworkRole) -> Option<&'static str> {
    match (state, role) {
        (ArtworkState::Missing, ArtworkRole::Hero) => {
            Some("KEY ART \u{2014} SUPPLIED BY PUBLISHER")
        }
        _ => fallback_label(state),
    }
}

#[component]
pub fn GameArtwork(
    #[prop(into)] title: String,
    state: ArtworkState,
    #[prop(optional)] role: ArtworkRole,
) -> impl IntoView {
    let artwork_state = RwSignal::new(state);
    let class = format!("arc-game-artwork {}", role.class());

    view! {
        <div class=class>
            {move || match artwork_state.get() {
                ArtworkState::Available(url) => view! {
                    <img
                        src=url
                        alt={format!("Artwork for {title}")}
                        loading=if role == ArtworkRole::Hero { "eager" } else { "lazy" }
                        decoding="async"
                        on:error=move |_| artwork_state.set(ArtworkState::Failed)
                    />
                }
                .into_any(),
                state => {
                    let label = fallback_label_for(&state, role).unwrap_or("ARTWORK PENDING");
                    view! {
                        <div
                            class="arc-artwork-fallback"
                            class:arc-artwork-loading=state == ArtworkState::Loading
                            role=(state == ArtworkState::Loading).then_some("status")
                            aria-label={format!("{label} for {title}")}
                        >
                            <span>{label}</span>
                        </div>
                    }
                    .into_any()
                }
            }}
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_and_available_artwork_are_selected_deterministically() {
        assert_eq!(artwork_state_from_url(None), ArtworkState::Missing);
        assert_eq!(
            artwork_state_from_url(Some("  ".to_string())),
            ArtworkState::Missing
        );
        assert_eq!(
            artwork_state_from_url(Some("https://cdn.example/game.webp".to_string())),
            ArtworkState::Available("https://cdn.example/game.webp".to_string())
        );
    }

    #[test]
    fn fallback_states_remain_visually_distinct() {
        assert_eq!(
            fallback_label(&ArtworkState::Loading),
            Some("ARTWORK LOADING")
        );
        assert_eq!(
            fallback_label(&ArtworkState::Missing),
            Some("ARTWORK PENDING")
        );
        assert_eq!(
            fallback_label(&ArtworkState::Failed),
            Some("ARTWORK UNAVAILABLE")
        );
        assert_eq!(fallback_label(&ArtworkState::Available("x".into())), None);
    }

    #[test]
    fn missing_hero_art_names_the_publisher_supplied_key_art() {
        assert_eq!(
            fallback_label_for(&ArtworkState::Missing, ArtworkRole::Hero),
            Some("KEY ART \u{2014} SUPPLIED BY PUBLISHER")
        );
        assert_eq!(
            fallback_label_for(&ArtworkState::Missing, ArtworkRole::Card),
            Some("ARTWORK PENDING")
        );
        assert_eq!(
            fallback_label_for(&ArtworkState::Failed, ArtworkRole::Hero),
            Some("ARTWORK UNAVAILABLE")
        );
    }
}
