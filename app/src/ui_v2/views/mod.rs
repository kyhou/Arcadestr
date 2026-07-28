//! UI v2 view modules.

pub mod achievements;
pub mod browse_games;
pub mod game_detail;
pub mod library;
pub mod login;
pub mod marketplace_loader;
pub mod profile;
pub mod publish;
pub mod purchases;
pub mod settings;
pub mod social;
pub mod store_front;
pub mod store_page_publish;

pub use achievements::AchievementsView;
pub use browse_games::{BrowseGamesView, BrowseRequest};
pub use game_detail::GameDetailView;
pub use library::LibraryView;
pub use login::LoginV2View;
pub use profile::ProfileV2View;
pub use publish::{PublishV2View, PublishViewState};
pub use purchases::PurchasesView;
pub use settings::SettingsView;
pub use social::SocialView;
pub use store_front::StoreFrontView;
pub use store_page_publish::StorePageEditorView;

const FALLBACK_COVER: &str = "data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 16 10'%3E%3Crect width='16' height='10' fill='%2325232a'/%3E%3Cpath d='M6 4h4v2H6z' fill='%23a9a2b3'/%3E%3C/svg%3E";

pub(crate) fn valid_cover_url(images: &[String]) -> Option<String> {
    images.iter().find_map(|candidate| {
        let trimmed = candidate.trim();
        let parsed = url::Url::parse(trimmed).ok()?;
        if !matches!(parsed.scheme(), "http" | "https") {
            return None;
        }
        let host = parsed.host_str()?;
        if host == "example"
            || host.ends_with(".example")
            || host == "example.com"
            || host.ends_with(".example.com")
        {
            return None;
        }
        Some(trimmed.to_string())
    })
}

pub(crate) fn use_fallback_cover(event: web_sys::ErrorEvent) {
    use leptos::prelude::event_target;

    let image = event_target::<web_sys::HtmlElement>(&event);
    let _ = image.set_attribute("src", FALLBACK_COVER);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cover_url_skips_invalid_and_placeholder_values() {
        let images = vec![
            String::new(),
            "https://example.com/cover.png".into(),
            "https://cdn.example.com/cover.png".into(),
            "https://cdn.arcadestr.com/cover.png".into(),
        ];

        assert_eq!(
            valid_cover_url(&images).as_deref(),
            Some("https://cdn.arcadestr.com/cover.png")
        );
    }

    #[test]
    fn cover_url_rejects_non_http_schemes() {
        assert_eq!(valid_cover_url(&["file:///tmp/cover.png".into()]), None);
    }
}
