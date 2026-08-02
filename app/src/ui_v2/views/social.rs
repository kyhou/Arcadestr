use leptos::prelude::*;

use crate::ui_v2::components::{EmptyState, FeedbackLayout, PageHeader};

fn community_unavailable_message() -> &'static str {
    "Community features are not available yet."
}

#[component]
pub fn SocialView() -> impl IntoView {
    view! {
        <section class="arc-community-page">
            <PageHeader title="Community".to_string() description="Public identity and publisher information come from signed Nostr data. Social activity is not implemented.".to_string() />
            <EmptyState
                title=community_unavailable_message()
                description="Arcadestr does not currently query or publish a feed, follows, reactions, messages, player presence, trends, recommendations, or zap activity. Your current-account public profile and verified NIP-58 badges remain available from their existing destinations."
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
