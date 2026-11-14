use leptos::prelude::*;

use crate::ui_v2::components::{EmptyState, FeedbackLayout, PageHeader};

fn community_unavailable_message() -> &'static str {
    "Community features are not available yet."
}

#[component]
pub fn SocialView() -> impl IntoView {
    view! {
        <section class="arc-community-page">
            <PageHeader title="Community".to_string() description="Community features are coming in a future update.".to_string() />
            <EmptyState
                title=community_unavailable_message()
                description="Posts, messages, and social activity are not available yet. You can view your profile and achievements from the account menu."
                icon="forum"
                layout=FeedbackLayout::Panel
            />
        </section>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn community_has_no_fabricated_feed_or_composer() {
        let production_source = include_str!("social.rs")
            .split("#[cfg(test)]")
            .next()
            .expect("production source should precede tests");

        for forbidden in [
            "<button",
            "<textarea",
            "Protocol Feed",
            "Broadcast Note",
            "v2-social-card",
        ] {
            assert!(!production_source.contains(forbidden));
        }
        assert_eq!(
            community_unavailable_message(),
            "Community features are not available yet."
        );
    }
}
