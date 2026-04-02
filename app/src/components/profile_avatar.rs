//! Profile avatar component - displays profile picture or fallback

use crate::store::profiles::use_profile;
use leptos::prelude::*;

#[component]
pub fn ProfileAvatar(npub: String, #[prop(default = "32px")] size: &'static str) -> impl IntoView {
    let profile = use_profile(npub.clone());

    view! {
        <div
            class="profile-avatar-container"
            style:width=size
            style:height=size
            style:border-radius="50%"
            style:overflow="hidden"
            style:flex-shrink="0"
        >
            {move || {
                match profile.get() {
                    Some(p) => {
                        if let Some(pic) = p.picture {
                            // Show profile picture
                            view! {
                                <img
                                    src=pic
                                    class="profile-avatar-img"
                                    alt="Profile"
                                    style:width="100%"
                                    style:height="100%"
                                    style:object-fit="cover"
                                />
                            }.into_any()
                        } else {
                            // Fallback: first letter of display name
                            let letter = p.display()
                                .chars()
                                .next()
                                .map(|c| c.to_uppercase().to_string())
                                .unwrap_or_else(|| "?".to_string());

                            view! {
                                <div
                                    class="profile-avatar-fallback"
                                    style:width="100%"
                                    style:height="100%"
                                    style:background="#555"
                                    style:display="flex"
                                    style:align-items="center"
                                    style:justify-content="center"
                                    style:font-size="0.6em"
                                    style:color="white"
                                    style:font-weight="bold"
                                >
                                    {letter}
                                </div>
                            }.into_any()
                        }
                    }
                    None => {
                        // Loading state - placeholder
                        view! {
                            <div
                                class="profile-avatar-placeholder"
                                style:width="100%"
                                style:height="100%"
                                style:background="#333"
                                style:animation="pulse 1.5s infinite"
                            />
                        }.into_any()
                    }
                }
            }}
        </div>
    }
}
