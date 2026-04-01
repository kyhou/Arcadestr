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
                        // Store values that need to be accessed in closures
                        let account_id = StoredValue::new(account.id);
                        let account_name = StoredValue::new(account.name);
                        let account_npub = StoredValue::new(account.npub);
                        let account_mode = StoredValue::new(account.signing_mode);
                        let is_current = StoredValue::new(account.is_current);

                        view! {
                            <div
                                class={format!("account-card {}", if is_current.get_value() { "active" } else { "" })}
                                on:click=move |_| {
                                    // Only switch if not currently active
                                    if !is_current.get_value() {
                                        let id = account_id.get_value();
                                        on_switch.run(id);
                                    }
                                }
                            >
                                <div class="account-avatar">
                                    <div class="avatar-placeholder">
                                        {move || {
                                            let name = account_name.get_value();
                                            name.as_deref().unwrap_or("?").chars().next().unwrap_or('?')
                                        }}
                                    </div>
                                </div>

                                <div class="account-info">
                                    <span class="account-name">
                                        {move || {
                                            let name = account_name.get_value();
                                            name.as_deref().unwrap_or("Unnamed Account").to_string()
                                        }}
                                    </span>
                                    <span class="account-npub">
                                        {move || {
                                            let npub = account_npub.get_value();
                                            format!("{}...{}", &npub[..8], &npub[npub.len()-8..])
                                        }}
                                    </span>
                                    <span class="account-mode">
                                        {move || account_mode.get_value()}
                                    </span>
                                </div>

                                <div class="account-actions">
                                    <Show
                                        when=move || is_current.get_value()
                                        fallback=move || view! {
                                            <button
                                                class="switch-btn"
                                                on:click=move |e| {
                                                    e.stop_propagation(); // Prevent card click
                                                    let id = account_id.get_value();
                                                    on_switch.run(id);
                                                }
                                            >
                                                "Connect"
                                            </button>
                                        }
                                    >
                                        <span class="current-badge">"Current"</span>
                                    </Show>

                                    <button
                                        class="delete-btn"
                                        on:click=move |e| {
                                            e.stop_propagation(); // Prevent card click
                                            let id = account_id.get_value();
                                            on_delete.run(id);
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
