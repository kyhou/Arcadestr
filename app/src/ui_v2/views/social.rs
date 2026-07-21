use leptos::prelude::*;

fn community_unavailable_message() -> &'static str {
    "Community features are not available yet."
}

#[component]
pub fn SocialView() -> impl IntoView {
    view! {
        <section class="v2-community">
            <header class="v2-community-hero v2-panel-glass">
                <p class="v2-store-kicker">"Nostr community"</p>
                <h1 class="v2-display">"Community"</h1>
                <p>"A future home for signed player notes, creator updates, and social discovery."</p>
            </header>

            <section class="v2-community-unavailable v2-panel" role="status" aria-labelledby="community-unavailable-title">
                <div class="v2-community-unavailable-mark" aria-hidden="true">
                    <span class="material-symbols-outlined">"forum"</span>
                </div>
                <div>
                    <p class="v2-store-kicker">"Feature unavailable"</p>
                    <h2 id="community-unavailable-title">{community_unavailable_message()}</h2>
                    <p>
                        "This client does not currently fetch or publish community notes. No feed, trends, recommendations, or zap activity is shown until those protocol flows are implemented."
                    </p>
                </div>
            </section>
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
