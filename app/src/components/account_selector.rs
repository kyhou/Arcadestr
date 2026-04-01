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
                                    <div class="avatar-placeholder">
                                        {
                                            let account_name = account_name.clone();
                                            move || {
                                                account_name.as_deref().unwrap_or("?").chars().next().unwrap_or('?')
                                            }
                                        }
                                    </div>
                                </div>

                                <div class="account-info">
                                    <span class="account-name">
                                        {
                                            let account_name = account_name.clone();
                                            move || {
                                                account_name.as_deref().unwrap_or("Unnamed Account").to_string()
                                            }
                                        }
                                    </span>
                                    <span class="account-npub">
                                        {
                                            let account_npub = account_npub.clone();
                                            move || {
                                                format!("{}...{}", &account_npub[..8], &account_npub[account_npub.len()-8..])
                                            }
                                        }
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
