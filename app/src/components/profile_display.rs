//! Profile display name component - shows display_name or name or npub fallback

use crate::components::ProfileAvatar;
use crate::store::profiles::use_profile;
use leptos::prelude::*;

#[component]
pub fn ProfileDisplayName(
    npub: String,
    #[prop(optional)] truncate_npub: Option<usize>,
    #[prop(default = false)] show_verified: bool,
) -> impl IntoView {
    let profile = use_profile(npub.clone());

    view! {
        <span class="profile-display-name">
            {move || {
                match profile.get() {
                    Some(p) => {
                        // Show display name or name
                        let display = p.display();

                        view! {
                            <span class="profile-name">
                                {display}
                                {if show_verified && p.nip05_verified {
                                    Some(view! {
                                        <span class="verified-badge" title="NIP-05 verified">" ✓"</span>
                                    }.into_any())
                                } else {
                                    None
                                }}
                            </span>
                        }.into_any()
                    }
                    None => {
                        // Show truncated npub as fallback
                        let display = if let Some(len) = truncate_npub {
                            if npub.len() > len {
                                format!("{}...", &npub[..len])
                            } else {
                                npub.clone()
                            }
                        } else {
                            npub.clone()
                        };
                        view! {
                            <span class="npub-fallback" style:font-family="monospace" style:color="#666">
                                {display}
                            </span>
                        }.into_any()
                    }
                }
            }}
        </span>
    }
}

/// Profile row component - combines avatar + display name
#[component]
pub fn ProfileRow(
    npub: String,
    #[prop(default = "32px")] avatar_size: &'static str,
    #[prop(optional)] truncate_npub: Option<usize>,
) -> impl IntoView {
    view! {
        <div
            class="profile-row"
            style:display="flex"
            style:align-items="center"
            style:gap="8px"
        >
            <ProfileAvatar npub={npub.clone()} size={avatar_size} />
            {match truncate_npub {
                Some(len) => view! {
                    <ProfileDisplayName npub={npub} truncate_npub={len} show_verified=false />
                }.into_any(),
                None => view! {
                    <ProfileDisplayName npub={npub} show_verified=false />
                }.into_any(),
            }}
        </div>
    }
}
