use leptos::prelude::*;

use crate::ui_v2::components::{EmptyState, FeedbackLayout, PageHeader};

fn mods_unavailable_message() -> &'static str {
    "Mods are not available yet."
}

/// Placeholder for the upcoming DEG Mods integration.
#[component]
pub fn ModsView() -> impl IntoView {
    view! {
        <section class="arc-mods-page">
            <PageHeader title="Mods".to_string() description="Mod browsing is coming in a future update.".to_string() />
            <EmptyState
                title=mods_unavailable_message()
                description="Browsing and installing mods through DEG Mods is planned for a future update. Mods for your games will appear here once the integration is available."
                icon="extension"
                layout=FeedbackLayout::Panel
            />
        </section>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mods_placeholder_has_no_fabricated_listings_or_controls() {
        let production_source = include_str!("mods.rs")
            .split("#[cfg(test)]")
            .next()
            .expect("production source should precede tests");

        for forbidden in ["<button", "<input", "<form"] {
            assert!(!production_source.contains(forbidden));
        }
        assert_eq!(mods_unavailable_message(), "Mods are not available yet.");
    }
}
