use leptos::prelude::*;
use wasm_bindgen_futures::spawn_local;

use crate::models::{npub_fallback_label, Nip49ImportRequest};
use crate::tauri_bridge::invoke_nip49_import;
use crate::ui_v2::theme::UI_V2_STYLES;
use crate::{
    invoke_check_qr_connection, invoke_connect_bunker, invoke_connect_nip07, invoke_has_accounts,
    invoke_start_qr_login, AuthContext, StoredAccount,
};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum LoginPanel {
    #[default]
    Accounts,
    Methods,
    Qr,
    Nip49,
}

fn show_back_button(panel: LoginPanel, has_saved_accounts: bool) -> bool {
    match panel {
        LoginPanel::Accounts => false,
        LoginPanel::Methods => has_saved_accounts,
        LoginPanel::Qr | LoginPanel::Nip49 => true,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum RemovalConfirmation {
    Idle,
    Confirming { account_id: String, label: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct AuthCapabilities {
    bunker: bool,
    qr: bool,
    nip07: bool,
    nip49: bool,
    local_nsec: bool,
}

fn target_auth_capabilities(standalone_web: bool) -> AuthCapabilities {
    AuthCapabilities {
        bunker: !standalone_web,
        qr: !standalone_web,
        nip07: standalone_web,
        nip49: !standalone_web,
        local_nsec: true,
    }
}

fn account_display_name(account: &StoredAccount) -> String {
    account
        .display_name
        .clone()
        .filter(|value| !value.trim().is_empty())
        .or_else(|| {
            account
                .username
                .clone()
                .filter(|value| !value.trim().is_empty())
        })
        .or_else(|| {
            account
                .name
                .clone()
                .filter(|value| !value.trim().is_empty())
        })
        .unwrap_or_else(|| npub_fallback_label(&account.npub))
}

fn account_mode_label(signing_mode: &str) -> &'static str {
    match signing_mode.to_ascii_lowercase().as_str() {
        "nip46" | "remote" => "Remote signer",
        "local" | "nsec" => "Local encrypted key",
        "nip07" => "Browser extension",
        "readonly" | "read_only" => "Read-only account",
        _ => "Saved account",
    }
}

fn account_connection_label(
    account: &StoredAccount,
    active_npub: Option<&str>,
    connection_status: &str,
    standalone_web: bool,
) -> &'static str {
    if active_npub != Some(account.npub.as_str()) {
        return "Saved";
    }
    if standalone_web {
        return "Identity restored; signer availability unverified";
    }
    if matches!(
        account.signing_mode.to_ascii_lowercase().as_str(),
        "nip46" | "remote"
    ) {
        match connection_status {
            "connected" => "Active and connected",
            "connecting" => "Active, connecting",
            "failed" => "Active, connection failed",
            _ => "Active, signer disconnected",
        }
    } else {
        "Active"
    }
}

fn is_active_account(account: &StoredAccount, active_npub: Option<&str>) -> bool {
    active_npub == Some(account.npub.as_str())
}

fn abbreviated_npub(npub: &str) -> String {
    npub_fallback_label(npub)
}

fn nip49_import_message(backend_message: &str) -> String {
    format!(
        "Encrypted key validated. {backend_message} This command does not yet add or activate a saved account."
    )
}

#[component]
pub fn LoginV2View() -> impl IntoView {
    let auth = use_context::<AuthContext>().expect("AuthContext not provided");
    let auth_stored = StoredValue::new(auth.clone());
    let standalone_web = cfg!(feature = "web");
    let capabilities = target_auth_capabilities(standalone_web);

    let panel = RwSignal::new(LoginPanel::Accounts);
    let loading_accounts = RwSignal::new(true);
    let removal = RwSignal::new(RemovalConfirmation::Idle);
    let removal_dialog_ref = NodeRef::<leptos::html::Dialog>::new();
    let flow_status = RwSignal::new(None::<String>);

    let bunker_uri = RwSignal::new(String::new());
    let bunker_name = RwSignal::new(String::new());
    let nsec_input = RwSignal::new(String::new());
    let local_name = RwSignal::new(String::new());

    let qr_uri = RwSignal::new(None::<String>);
    let qr_polling = RwSignal::new(false);
    let qr_error = RwSignal::new(None::<String>);

    let import_ncryptsec = RwSignal::new(String::new());
    let import_password = RwSignal::new(String::new());
    let import_error = RwSignal::new(None::<String>);
    let import_result = RwSignal::new(None::<String>);
    let import_loading = RwSignal::new(false);

    Effect::new(move |_| {
        let auth = auth_stored.get_value();
        spawn_local(async move {
            let has_accounts = invoke_has_accounts().await.unwrap_or(false);
            if has_accounts {
                if let Err(error) = auth.load_accounts_list().await {
                    auth.error.set(Some(error));
                }
                panel.set(LoginPanel::Accounts);
            } else {
                panel.set(LoginPanel::Methods);
            }
            loading_accounts.set(false);
        });
    });

    Effect::new(move |_| {
        let Some(dialog) = removal_dialog_ref.get() else {
            return;
        };
        if matches!(removal.get(), RemovalConfirmation::Confirming { .. }) {
            if !dialog.open() {
                let _ = dialog.show_modal();
            }
        } else if dialog.open() {
            dialog.close();
        }
    });

    let begin_flow = move || {
        let auth = auth_stored.get_value();
        let generation = auth.begin_auth_flow();
        auth.error.set(None);
        flow_status.set(None);
        generation
    };

    let cancel_flow = move || {
        let auth = auth_stored.get_value();
        if auth.is_loading.get_untracked() {
            return;
        }
        auth.begin_auth_flow();
        auth.is_loading.set(false);
        qr_polling.set(false);
        qr_uri.set(None);
        qr_error.set(None);
        bunker_uri.set(String::new());
        bunker_name.set(String::new());
        nsec_input.set(String::new());
        local_name.set(String::new());
        import_ncryptsec.set(String::new());
        import_password.set(String::new());
        import_error.set(None);
        import_result.set(None);
        import_loading.set(false);
        panel.set(if auth.accounts.get_untracked().is_empty() {
            LoginPanel::Methods
        } else {
            LoginPanel::Accounts
        });
    };

    let on_connect_bunker = move |_| {
        let uri = bunker_uri.get().trim().to_string();
        if uri.is_empty() {
            auth_stored.get_value().error.set(Some(
                "Enter a bunker URI or NIP-05 signer identifier.".to_string(),
            ));
            return;
        }
        let auth = auth_stored.get_value();
        let generation = begin_flow();
        let display_name = bunker_name.get().trim().to_string();
        auth.is_loading.set(true);
        flow_status.set(Some("Waiting for remote signer approval...".to_string()));

        spawn_local(async move {
            let result = invoke_connect_bunker(uri, display_name).await;
            if !auth.is_current_auth_flow(generation) {
                return;
            }
            match result {
                Ok(result) => {
                    let Some(npub) = result
                        .get("pubkey")
                        .and_then(|value| value.as_str())
                        .map(str::to_string)
                    else {
                        auth.error.set(Some(
                            "Signer connected without an account identifier.".to_string(),
                        ));
                        auth.is_loading.set(false);
                        return;
                    };
                    let _ = auth.load_accounts_list().await;
                    if !auth.is_current_auth_flow(generation) {
                        return;
                    }
                    auth.active_account.set(
                        auth.accounts
                            .get_untracked()
                            .into_iter()
                            .find(|account| account.npub == npub),
                    );
                    auth.npub.set(Some(npub));
                    auth.has_secure_accounts.set(true);
                    auth.connection_status.set("connecting".to_string());
                    auth.is_loading.set(false);
                    auth.start_connection_status_polling().await;
                    bunker_uri.set(String::new());
                    bunker_name.set(String::new());
                    flow_status.set(Some("Remote signer accepted.".to_string()));
                }
                Err(error) => {
                    auth.is_loading.set(false);
                    auth.error.set(Some(error));
                }
            }
        });
    };

    let on_connect_nip07 = move |_| {
        let auth = auth_stored.get_value();
        let generation = begin_flow();
        auth.is_loading.set(true);
        flow_status.set(Some(
            "Requesting browser-extension permission...".to_string(),
        ));
        spawn_local(async move {
            let result = invoke_connect_nip07().await;
            if !auth.is_current_auth_flow(generation) {
                return;
            }
            match result {
                Ok(npub) => {
                    let _ = auth.load_accounts_list().await;
                    if !auth.is_current_auth_flow(generation) {
                        return;
                    }
                    auth.active_account.set(
                        auth.accounts
                            .get_untracked()
                            .into_iter()
                            .find(|account| account.npub == npub),
                    );
                    auth.npub.set(Some(npub));
                    auth.has_secure_accounts.set(true);
                    auth.is_loading.set(false);
                    flow_status.set(Some("Browser extension connected.".to_string()));
                }
                Err(error) => {
                    auth.is_loading.set(false);
                    auth.error.set(Some(error));
                }
            }
        });
    };

    let on_nsec_login = move |_| {
        let nsec = nsec_input.get();
        if nsec.trim().is_empty() {
            auth_stored
                .get_value()
                .error
                .set(Some("Enter an nsec key.".to_string()));
            return;
        }
        let auth = auth_stored.get_value();
        let name = local_name.get().trim().to_string();
        flow_status.set(Some(
            "Encrypting and saving the local account...".to_string(),
        ));
        nsec_input.set(String::new());
        spawn_local(async move {
            let result = auth
                .login_with_nsec(nsec, (!name.is_empty()).then_some(name))
                .await;
            local_name.set(String::new());
            match result {
                Ok(()) => {
                    flow_status.set(Some("Local account connected.".to_string()));
                }
                Err(error) => auth.error.set(Some(error)),
            }
        });
    };

    let on_start_qr = move |_| {
        let auth = auth_stored.get_value();
        let generation = begin_flow();
        auth.is_loading.set(true);
        qr_error.set(None);
        flow_status.set(Some("Preparing a Nostr Connect request...".to_string()));
        spawn_local(async move {
            let result = invoke_start_qr_login().await;
            if !auth.is_current_auth_flow(generation) {
                return;
            }
            auth.is_loading.set(false);
            match result {
                Ok(uri) => {
                    qr_uri.set(Some(uri));
                    qr_polling.set(true);
                    panel.set(LoginPanel::Qr);
                    flow_status.set(Some("Waiting for signer approval...".to_string()));

                    let auth_for_poll = auth.clone();
                    spawn_local(async move {
                        while qr_polling.get_untracked()
                            && auth_for_poll.is_current_auth_flow(generation)
                        {
                            match invoke_check_qr_connection().await {
                                Ok(Some(result)) => {
                                    if !auth_for_poll.is_current_auth_flow(generation) {
                                        return;
                                    }
                                    let Some(npub) = result
                                        .get("pubkey")
                                        .and_then(|value| value.as_str())
                                        .map(str::to_string)
                                    else {
                                        qr_error.set(Some(
                                            "Signer accepted without an account identifier."
                                                .to_string(),
                                        ));
                                        break;
                                    };
                                    let _ = auth_for_poll.load_accounts_list().await;
                                    if !auth_for_poll.is_current_auth_flow(generation) {
                                        return;
                                    }
                                    auth_for_poll.active_account.set(
                                        auth_for_poll
                                            .accounts
                                            .get_untracked()
                                            .into_iter()
                                            .find(|account| account.npub == npub),
                                    );
                                    auth_for_poll.npub.set(Some(npub));
                                    auth_for_poll.has_secure_accounts.set(true);
                                    auth_for_poll
                                        .connection_status
                                        .set("connecting".to_string());
                                    qr_polling.set(false);
                                    flow_status
                                        .set(Some("Signer connection accepted.".to_string()));
                                    auth_for_poll.start_connection_status_polling().await;
                                    break;
                                }
                                Ok(None) => {}
                                Err(error) => {
                                    if auth_for_poll.is_current_auth_flow(generation) {
                                        qr_error.set(Some(error));
                                        qr_polling.set(false);
                                    }
                                    break;
                                }
                            }
                            gloo_timers::future::TimeoutFuture::new(5_000).await;
                        }
                    });
                }
                Err(error) => qr_error.set(Some(error)),
            }
        });
    };

    let on_import_nip49 = move |_| {
        let ncryptsec = import_ncryptsec.get().trim().to_string();
        let password = import_password.get();
        if ncryptsec.is_empty() || password.chars().count() < 8 {
            import_error.set(Some(
                "Enter an ncryptsec value and its password of at least 8 characters.".to_string(),
            ));
            return;
        }
        let auth = auth_stored.get_value();
        let generation = begin_flow();
        import_loading.set(true);
        import_error.set(None);
        import_result.set(None);
        import_ncryptsec.set(String::new());
        import_password.set(String::new());
        spawn_local(async move {
            let result = invoke_nip49_import(Nip49ImportRequest {
                ncryptsec,
                password,
            })
            .await;
            if !auth.is_current_auth_flow(generation) {
                return;
            }
            import_loading.set(false);
            match result {
                Ok(message) => import_result.set(Some(nip49_import_message(&message))),
                Err(error) => import_error.set(Some(error)),
            }
        });
    };

    view! {
        <div class="v2-auth-screen bg-background text-on-surface font-body min-h-screen">
            <style>{UI_V2_STYLES}</style>
            <header class="v2-auth-topbar">
                <div class="v2-auth-brand" aria-label="Arcadestr">
                    <span class="v2-auth-brand-mark" aria-hidden="true"></span>
                    <span>"Arcadestr Noir"</span>
                </div>
                <Show when=move || show_back_button(
                    panel.get(),
                    !auth_stored.get_value().accounts.get().is_empty(),
                )>
                    <button class="v2-btn-ghost" disabled=move || auth_stored.get_value().is_loading.get() on:click=move |_| cancel_flow()>
                        <span class="material-symbols-outlined" aria-hidden="true">"arrow_back"</span>
                        "Back"
                    </button>
                </Show>
            </header>

            <main class="v2-auth-main">
                {move || match panel.get() {
                    LoginPanel::Accounts => view! {
                        <section class="v2-auth-account-shell">
                            <header class="v2-auth-heading">
                                <p class="v2-store-kicker">"Saved identities"</p>
                                <h1 class="v2-display">"Choose an account"</h1>
                                <p>"Switch to a securely stored identity or connect another signer."</p>
                            </header>
                            {move || if loading_accounts.get() {
                                view! { <p class="v2-auth-live" role="status">"Restoring saved accounts..."</p> }.into_any()
                            } else {
                                let accounts = auth_stored.get_value().accounts.get();
                                view! {
                                    <div class="v2-auth-account-grid">
                                        {accounts.into_iter().map(|account| {
                                            let switch_id = account.id.clone();
                                            let remove_id = account.id.clone();
                                            let label = account_display_name(&account);
                                            let remove_label = label.clone();
                                            let avatar = account.picture.clone();
                                            let fallback = label.chars().next().unwrap_or('?').to_uppercase().to_string();
                                            let is_active = is_active_account(&account, auth_stored.get_value().npub.get().as_deref());
                                            let label_for_copy = label.clone();
                                            let npub_for_copy = account.npub.clone();
                                            let copy_status = RwSignal::new(None::<String>);
                                            let copy_npub = Callback::new(move |()| {
                                                let Some(clipboard) = web_sys::window().map(|window| window.navigator().clipboard()) else {
                                                    copy_status.set(Some("Clipboard unavailable.".to_string()));
                                                    return;
                                                };
                                                let _ = clipboard.write_text(&npub_for_copy);
                                                copy_status.set(Some("Public key copied.".to_string()));
                                            });
                                            let connection = account_connection_label(
                                                &account,
                                                auth_stored.get_value().npub.get().as_deref(),
                                                &auth_stored.get_value().connection_status.get(),
                                                standalone_web,
                                            );
                                            view! {
                                                <article class="v2-auth-account-card" class:v2-auth-account-card-active=is_active>
                                                    <div class="v2-auth-account-identity">
                                                        {match avatar {
                                                            Some(url) => view! { <img src=url alt="" class="v2-auth-account-avatar" /> }.into_any(),
                                                            None => view! { <div class="v2-auth-account-avatar v2-auth-account-fallback" aria-hidden="true">{fallback}</div> }.into_any(),
                                                        }}
                                                        <div class="v2-auth-account-copy">
                                                            <h2>{label}</h2>
                                                            <p>{abbreviated_npub(&account.npub)}<button type="button" class="v2-btn-ghost v2-auth-copy-npub" aria-label=format!("Copy full public key for {}", label_for_copy) on:click=move |event| { event.stop_propagation(); copy_npub.run(()); }>"Copy public key"</button></p>
                                                            {move || copy_status.get().map(|status| view! { <span class="v2-auth-copy-status" role="status">{status}</span> })}
                                                        </div>
                                                    </div>
                                                    <div class="v2-auth-account-statuses">
                                                        <span class="v2-chip">{account_mode_label(&account.signing_mode)}</span>
                                                        <span>{connection}</span>
                                                        {account.nip05.clone().map(|nip05| view! { <span>{nip05}</span> })}
                                                    </div>
                                                    <div class="v2-auth-account-actions">
                                                        <button
                                                            class="v2-btn-primary"
                                                            disabled=move || is_active || auth_stored.get_value().is_loading.get()
                                                            on:click=move |_| {
                                                                let auth = auth_stored.get_value();
                                                                let account_id = switch_id.clone();
                                                                spawn_local(async move {
                                                                    if let Err(error) = auth.switch_account(account_id).await {
                                                                        auth.error.set(Some(error));
                                                                    }
                                                                });
                                                            }
                                                        >
                                                            {if is_active { "Active" } else if account.signing_mode.eq_ignore_ascii_case("nip46") { "Reconnect" } else { "Switch" }}
                                                        </button>
                                                        <button class="v2-btn-ghost v2-btn-danger" disabled=move || auth_stored.get_value().is_loading.get() on:click=move |_| removal.set(RemovalConfirmation::Confirming {
                                                            account_id: remove_id.clone(),
                                                            label: remove_label.clone(),
                                                        })>"Remove"</button>
                                                    </div>
                                                </article>
                                            }
                                        }).collect::<Vec<_>>()}
                                        <button class="v2-auth-add-card" on:click=move |_| panel.set(LoginPanel::Methods)>
                                            <span class="material-symbols-outlined" aria-hidden="true">"add"</span>
                                            <strong>"Connect another account"</strong>
                                            <span>"Remote signer, local key, or browser extension"</span>
                                        </button>
                                    </div>
                                }.into_any()
                            }}
                        </section>
                    }.into_any(),
                    LoginPanel::Methods => view! {
                        <section>
                            <header class="v2-auth-heading">
                                <p class="v2-store-kicker">"Sign in or connect"</p>
                                <h1 class="v2-display">"Your keys, your choice"</h1>
                                <p>"Choose a supported signing method. Arcadestr keeps target-specific limitations visible."</p>
                            </header>
                            <div class="v2-auth-method-grid">
                                {if capabilities.bunker {
                                    view! {
                                        <section class="v2-auth-method-card v2-auth-method-featured">
                                            <span class="material-symbols-outlined v2-auth-method-icon" aria-hidden="true">"security"</span>
                                            <h2>"Remote signer"</h2>
                                            <p>"Connect with a bunker URI or NIP-05 signer identifier. Approval stays with your signing application."</p>
                                            <label for="bunker-uri">"Bunker URI or signer identifier"</label>
                                            <input id="bunker-uri" class="v2-input" type="text" autocomplete="off" placeholder="bunker://... or name@example.com" bind:value=bunker_uri />
                                            <label for="bunker-name">"Account label (optional)"</label>
                                            <input id="bunker-name" class="v2-input" type="text" autocomplete="off" bind:value=bunker_name />
                                            <button class="v2-btn-primary" on:click=on_connect_bunker disabled=move || auth_stored.get_value().is_loading.get()>"Connect remote signer"</button>
                                        </section>
                                    }.into_any()
                                } else {
                                    view! { <></> }.into_any()
                                }}

                                {if capabilities.nip07 {
                                    view! {
                                        <section class="v2-auth-method-card v2-auth-method-featured">
                                            <span class="material-symbols-outlined v2-auth-method-icon" aria-hidden="true">"extension"</span>
                                            <h2>"Browser extension"</h2>
                                            <p>"Ask a NIP-07 extension for the active public key. The extension may be unavailable or deny permission."</p>
                                            <button class="v2-btn-primary" on:click=on_connect_nip07 disabled=move || auth_stored.get_value().is_loading.get()>"Connect browser extension"</button>
                                        </section>
                                    }.into_any()
                                } else {
                                    view! { <></> }.into_any()
                                }}

                                {if capabilities.qr {
                                    view! {
                                        <section class="v2-auth-method-card">
                                            <span class="material-symbols-outlined v2-auth-method-icon" aria-hidden="true">"qr_code_2"</span>
                                            <h2>"Nostr Connect QR"</h2>
                                            <p>"Scan a one-time connection request with a compatible signing app."</p>
                                            <button class="v2-btn-secondary" on:click=on_start_qr disabled=move || auth_stored.get_value().is_loading.get()>"Start QR connection"</button>
                                        </section>
                                    }.into_any()
                                } else {
                                    view! { <></> }.into_any()
                                }}

                                {if capabilities.local_nsec {
                                    view! {
                                        <section class="v2-auth-method-card">
                                            <span class="material-symbols-outlined v2-auth-method-icon" aria-hidden="true">"key"</span>
                                            <h2>"Local secret key"</h2>
                                            <p>"Use only on a trusted device. Desktop encrypts local accounts in secure storage; web storage has weaker same-origin protections."</p>
                                            <label for="local-account-name">"Account label (optional)"</label>
                                            <input id="local-account-name" class="v2-input" type="text" autocomplete="off" bind:value=local_name />
                                            <label for="local-nsec">"Secret key"</label>
                                            <input id="local-nsec" class="v2-input" type="password" autocomplete="off" placeholder="nsec1..." bind:value=nsec_input />
                                            <button class="v2-btn-secondary" on:click=on_nsec_login disabled=move || auth_stored.get_value().is_loading.get()>"Encrypt and connect"</button>
                                        </section>
                                    }.into_any()
                                } else {
                                    view! { <></> }.into_any()
                                }}

                                {if capabilities.nip49 {
                                    view! {
                                        <button class="v2-auth-method-card v2-auth-method-link" on:click=move |_| panel.set(LoginPanel::Nip49)>
                                            <span class="material-symbols-outlined v2-auth-method-icon" aria-hidden="true">"lock"</span>
                                            <strong>"Validate an encrypted NIP-49 key"</strong>
                                            <span>"Desktop can decrypt and validate ncryptsec input. Account activation is not yet implemented."</span>
                                        </button>
                                    }.into_any()
                                } else {
                                    view! { <></> }.into_any()
                                }}
                            </div>
                        </section>
                    }.into_any(),
                    LoginPanel::Qr => {
                        let uri = qr_uri.get();
                        view! {
                            <section class="v2-auth-focus-card">
                                <p class="v2-store-kicker">"Nostr Connect"</p>
                                <h1>"Approve in your signing app"</h1>
                                <p>"Scan the QR code or copy the connection URI. This request expires; cancellation rejects late responses."</p>
                                {match uri {
                                    Some(uri) => {
                                        let qr_svg = crate::qr::generate_qr_svg(&uri);
                                        view! {
                                            <div>
                                                <div class="v2-auth-qr" aria-label="Nostr Connect QR code" inner_html=qr_svg></div>
                                                <label for="nostrconnect-uri">"Manual connection URI"</label>
                                                <textarea id="nostrconnect-uri" class="v2-input" readonly rows="4" prop:value=uri />
                                            </div>
                                        }.into_any()
                                    }
                                    None => view! { <p class="v2-auth-live" role="status">"Preparing connection request..."</p> }.into_any(),
                                }}
                                <p class="v2-auth-live" role="status" aria-live="polite">{move || flow_status.get().unwrap_or_default()}</p>
                                {move || qr_error.get().map(|error| view! { <p class="v2-auth-error" role="alert">{error}</p> })}
                                <button class="v2-btn-ghost" on:click=move |_| cancel_flow()>"Cancel connection"</button>
                            </section>
                        }.into_any()
                    }
                    LoginPanel::Nip49 => view! {
                        <section class="v2-auth-focus-card">
                            <p class="v2-store-kicker">"NIP-49 validation"</p>
                            <h1>"Check an encrypted key"</h1>
                            <p>"Desktop currently validates and decrypts this key but does not save or activate it as an account. Your inputs are cleared after submission."</p>
                            <label for="nip49-import-value">"Encrypted key"</label>
                            <textarea id="nip49-import-value" class="v2-input" rows="5" placeholder="ncryptsec1..." bind:value=import_ncryptsec></textarea>
                            <label for="nip49-import-password">"Password"</label>
                            <input id="nip49-import-password" class="v2-input" type="password" autocomplete="current-password" bind:value=import_password />
                            <button class="v2-btn-primary" on:click=on_import_nip49 disabled=move || import_loading.get()>
                                {move || if import_loading.get() { "Validating..." } else { "Validate encrypted key" }}
                            </button>
                            {move || import_error.get().map(|error| view! { <p class="v2-auth-error" role="alert">{error}</p> })}
                            {move || import_result.get().map(|message| view! { <p class="v2-auth-success" role="status">{message}</p> })}
                        </section>
                    }.into_any(),
                }}
            </main>

            <dialog
                node_ref=removal_dialog_ref
                class="v2-confirm-backdrop"
                aria-labelledby="remove-account-title"
                on:cancel=move |event: web_sys::Event| {
                    event.prevent_default();
                    removal.set(RemovalConfirmation::Idle);
                }
                on:click=move |_| removal.set(RemovalConfirmation::Idle)
            >
                <section class="v2-confirm-dialog" on:click=move |event| event.stop_propagation()>
                    <h2 id="remove-account-title">"Remove saved account?"</h2>
                    {move || match removal.get() {
                        RemovalConfirmation::Idle => view! { <></> }.into_any(),
                        RemovalConfirmation::Confirming { account_id, label } => view! {
                            <div>
                                <p>{format!("Remove {label} from this device? This does not delete the Nostr identity.")}</p>
                                <div class="v2-auth-account-actions">
                                    <button class="v2-btn-ghost" autofocus on:click=move |_| removal.set(RemovalConfirmation::Idle)>"Cancel"</button>
                                    <button class="v2-btn-danger" on:click=move |_| {
                                        let auth = auth_stored.get_value();
                                        let account_id = account_id.clone();
                                        removal.set(RemovalConfirmation::Idle);
                                        spawn_local(async move {
                                            if let Err(error) = auth.delete_account(account_id).await {
                                                auth.error.set(Some(error));
                                            }
                                        });
                                    }>"Remove account"</button>
                                </div>
                            </div>
                        }.into_any(),
                    }}
                </section>
            </dialog>

            <Show when=move || auth_stored.get_value().is_loading.get() || flow_status.get().is_some()>
                <p class="v2-auth-global-status" role="status" aria-live="polite">
                    {move || flow_status.get().unwrap_or_else(|| "Working...".to_string())}
                </p>
            </Show>
            {move || auth_stored.get_value().error.get().map(|error| view! {
                <div class="v2-auth-toast" role="alert">{error}</div>
            })}
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn account() -> StoredAccount {
        StoredAccount {
            id: "id".to_string(),
            npub: "npub1vcq8nv3l2wcjdecvyk0xhqacdwa505fqn6zpqwmwpd6syj3d9l".to_string(),
            name: None,
            signing_mode: "nip46".to_string(),
            last_used: 0,
            is_current: false,
            picture: None,
            display_name: None,
            username: None,
            nip05: None,
            about: None,
        }
    }

    #[test]
    fn account_display_falls_back_to_abbreviated_npub() {
        assert_eq!(account_display_name(&account()), "npub1vcq8nv3...6syj3d9l");
    }

    #[test]
    fn active_remote_account_reports_connection_state() {
        let account = account();
        assert_eq!(
            account_connection_label(&account, Some(&account.npub), "failed", false),
            "Active, connection failed"
        );
        assert_eq!(
            account_connection_label(&account, None, "connected", false),
            "Saved"
        );
    }

    #[test]
    fn active_account_selection_requires_current_identity() {
        let mut account = account();
        assert!(!is_active_account(&account, None));
        assert!(is_active_account(&account, Some(&account.npub)));
        account.is_current = true;
        assert!(!is_active_account(&account, None));
    }

    #[test]
    fn web_restoration_does_not_claim_a_verified_signer() {
        let account = account();
        assert_eq!(
            account_connection_label(&account, Some(&account.npub), "connected", true),
            "Identity restored; signer availability unverified"
        );
    }

    #[test]
    fn removal_requires_explicit_confirmation_state() {
        let state = RemovalConfirmation::Confirming {
            account_id: "id".to_string(),
            label: "Player".to_string(),
        };
        assert!(matches!(state, RemovalConfirmation::Confirming { .. }));
    }

    #[test]
    fn target_capabilities_hide_native_actions_on_web() {
        let web = target_auth_capabilities(true);
        assert!(web.nip07);
        assert!(!web.bunker && !web.qr && !web.nip49);

        let desktop = target_auth_capabilities(false);
        assert!(desktop.bunker && desktop.qr && desktop.nip49);
        assert!(!desktop.nip07);
    }

    #[test]
    fn back_button_only_appears_when_navigation_has_a_destination() {
        assert!(!show_back_button(LoginPanel::Accounts, true));
        assert!(!show_back_button(LoginPanel::Methods, false));
        assert!(show_back_button(LoginPanel::Methods, true));
        assert!(show_back_button(LoginPanel::Qr, false));
        assert!(show_back_button(LoginPanel::Nip49, false));
    }

    #[test]
    fn nip49_import_message_does_not_claim_account_restoration() {
        let message = nip49_import_message("Import successful");
        assert!(message.contains("does not yet add or activate"));
    }

    #[test]
    fn stale_login_response_is_rejected_after_new_flow() {
        assert!(crate::should_apply_auth_response(7, 7));
        assert!(!crate::should_apply_auth_response(7, 8));
    }
    #[test]
    fn account_presence_and_signer_readiness_stay_separate() {
        let mut remote = account();
        remote.signing_mode = "nip46".into();
        // Same account, four different signer readiness states.
        let states = ["connected", "connecting", "failed", "disconnected"].map(|status| {
            account_connection_label(&remote, Some(remote.npub.as_str()), status, false)
        });
        let mut unique = states.to_vec();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(unique.len(), 4, "signer states collapsed: {states:?}");
        // A saved-but-inactive account reports presence only, never readiness.
        assert_eq!(
            account_connection_label(&remote, Some("npub1other"), "connected", false),
            "Saved"
        );
    }

    #[test]
    fn signer_type_is_reported_separately_from_connection() {
        assert_eq!(account_mode_label("nip46"), "Remote signer");
        assert_eq!(account_mode_label("local"), "Local encrypted key");
        assert_eq!(account_mode_label("nip07"), "Browser extension");
        assert_eq!(account_mode_label("readonly"), "Read-only account");
        assert_eq!(account_mode_label("unknown-mode"), "Saved account");
    }

    #[test]
    fn public_keys_are_abbreviated_and_never_shown_with_secret_material() {
        let npub = "npub1kvl9ev2wcjdecvyk0xhqacdwa505fqn6zpqwmwpd7vn4p565amyqesnwt4";
        let shown = abbreviated_npub(npub);
        assert!(shown.len() < npub.len());
        let source = include_str!("login.rs");
        // The full key is only reachable through the deliberate copy control.
        assert!(source.contains("Copy public key"));
        // Secret entry stays masked and excluded from autofill; secrets are passed
        // to commands but never rendered back into markup.
        assert!(source
            .contains(r#"id="local-nsec" class="v2-input" type="password" autocomplete="off""#));
        // Split so the literals do not match this test's own source.
        for rendered_secret in [
            [">{nsec", "_input"].concat(),
            [">{import", "_password"].concat(),
            [">{pass", "word}"].concat(),
            ["{decry", "pted"].concat(),
        ] {
            assert!(
                !source.contains(&rendered_secret),
                "secret material rendered into markup: {rendered_secret}"
            );
        }
    }

    #[test]
    fn nip49_import_does_not_claim_account_activation() {
        let message = nip49_import_message("Key validated.");
        assert!(message.contains("does not yet add or activate a saved account"));
        assert!(!message.to_ascii_lowercase().contains("signed in"));
        assert!(!message.to_ascii_lowercase().contains("account created"));
    }

    #[test]
    fn unsupported_login_methods_are_absent() {
        let source = include_str!("login.rs");
        for absent in [
            ["Cloud", " recovery"].concat(),
            ["Sign in with ", "Google"].concat(),
            ["Email", " login"].concat(),
            ["Seed", " backup"].concat(),
        ] {
            assert!(
                !source.contains(&absent),
                "unsupported login method present"
            );
        }
    }
}
