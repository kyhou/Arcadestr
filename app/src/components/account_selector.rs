use crate::store::profiles::use_profile;
use crate::{AuthContext, StoredAccount};
use leptos::prelude::*;

/// Account selection/switching UI
/// Primary view for login - shows list of stored accounts
#[component]
pub fn AccountSelector(
    auth: AuthContext,
    #[prop(into)] on_switch: Callback<String>,
    #[prop(into)] on_delete: Callback<String>,
    #[prop(into)] on_add_account: Callback<()>,
) -> impl IntoView {
    let accounts = move || auth.accounts.get();

    view! {
        <div class="account-selector">
            // Removed duplicate header - parent component already has "Welcome Back"

            <div class="accounts-list">
                <For
                    each=accounts
                    key=|account| account.id.clone()
                    children=move |account: StoredAccount| {
                        // Clone values for use in closures
                        let account_id = account.id.clone();
                        let account_name = account.name.clone();
                        let account_npub = account.npub.clone();
                        let account_mode = account.signing_mode.clone();
                        let is_current = account.is_current;

                        // Profile data from StoredAccount (immediately available)
                        let stored_picture = account.picture.clone();
                        let stored_display_name = account.display_name.clone();
                        let stored_username = account.username.clone();

                        // Create reactive signal for profile - this will update when store changes
                        let profile_signal = use_profile(account_npub.clone());

                        view! {
                            <div
                                class={format!("account-card {}", if is_current { "active" } else { "" })}
                                on:click={
                                    let account_id = account_id.clone();
                                    move |_| {
                                        // Only switch if not currently active
                                        if !is_current {
                                            on_switch.run(account_id.clone());
                                        }
                                    }
                                }
                            >
                                <div class="account-avatar">
                                    {{
                                        let account_name_for_avatar = account_name.clone();
                                        let stored_pic = stored_picture.clone();
                                        let stored_disp_name = stored_display_name.clone();

                                        move || {
                                            // First try to get from reactive profile store (may have fresher data)
                                            match profile_signal.get() {
                                                Some(profile) => {
                                                    if let Some(picture) = profile.picture {
                                                        view! {
                                                            <img src={picture} class="account-avatar-img" alt="avatar" />
                                                        }.into_any()
                                                    } else {
                                                        let letter = profile.display().chars().next().unwrap_or('?');
                                                        view! {
                                                            <div class="avatar-placeholder">{letter}</div>
                                                        }.into_any()
                                                    }
                                                }
                                                None => {
                                                    // No profile in store, use stored data from StoredAccount
                                                    if let Some(picture) = stored_pic.clone() {
                                                        view! {
                                                            <img src={picture} class="account-avatar-img" alt="avatar" />
                                                        }.into_any()
                                                    } else {
                                                        // Fallback to first letter of display name or account name
                                                        let name_for_letter = stored_disp_name.clone()
                                                            .or_else(|| account_name_for_avatar.clone())
                                                            .unwrap_or_else(|| "?".to_string());
                                                        let letter = name_for_letter.chars().next().unwrap_or('?');
                                                        view! {
                                                            <div class="avatar-placeholder">{letter}</div>
                                                        }.into_any()
                                                    }
                                                }
                                            }
                                        }
                                    }}
                                </div>

                                <div class="account-info">
                                    <span class="account-name">
                                        {{
                                            let account_name_for_display = account_name.clone();
                                            let stored_disp_name = stored_display_name.clone();
                                            let stored_user = stored_username.clone();

                                            move || {
                                                // First try reactive profile store
                                                match profile_signal.get() {
                                                    Some(profile) => profile.display(),
                                                    None => {
                                                        // Use stored data: display_name > username > account name > npub
                                                        stored_disp_name.clone()
                                                            .or_else(|| stored_user.clone())
                                                            .or_else(|| account_name_for_display.clone())
                                                            .unwrap_or_else(|| "Unnamed Account".to_string())
                                                    }
                                                }
                                            }
                                        }}
                                    </span>
                                    <span class="account-npub">
                                        {format!("{}...{}", &account_npub[..8], &account_npub[account_npub.len()-8..])}
                                    </span>
                                    <span class="account-mode">
                                        {account_mode.clone()}
                                    </span>
                                </div>

                                <div class="account-actions">
                                    <Show
                                        when=move || is_current
                                        fallback={
                                            let account_id = account_id.clone();
                                            move || view! {
                                                <button
                                                    class="switch-btn"
                                                    on:click={
                                                        let account_id = account_id.clone();
                                                        move |e| {
                                                            e.stop_propagation(); // Prevent card click
                                                            on_switch.run(account_id.clone());
                                                        }
                                                    }
                                                >
                                                    "Connect"
                                                </button>
                                            }
                                        }
                                    >
                                        <span class="current-badge">"Current"</span>
                                    </Show>

                                    <button
                                        class="delete-btn"
                                        on:click={
                                            let account_id = account_id.clone();
                                            move |e| {
                                                e.stop_propagation(); // Prevent card click
                                                on_delete.run(account_id.clone());
                                            }
                                        }
                                        title="Delete account"
                                    >
                                        "×"
                                    </button>
                                </div>
                            </div>
                        }
                    }
                />
            </div>

            <div class="account-selector-footer">
                <button class="add-account-btn" on:click=move |_| on_add_account.run(())>
                    "+ Add New Account"
                </button>
            </div>
        </div>
    }
}
