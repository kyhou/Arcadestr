use leptos::prelude::*;
use std::collections::HashMap;
use wasm_bindgen_futures::spawn_local;

use crate::models::{npub_fallback_label, Nip49ExportResult, PlatformInfo};
use crate::tauri_bridge::{
    invoke_add_blossom_server, invoke_attempt_reconnect, invoke_get_blossom_server_settings,
    invoke_get_platform_info, invoke_probe_blossom_server_health, invoke_reconnect_relays,
    invoke_remove_blossom_server, invoke_set_preferred_blossom_server,
    invoke_update_blossom_server, AddBlossomServerRequest, BlossomServerHealthDto,
    BlossomServerOriginRequest, BlossomServerSettingsDto, ExpectedBlossomPublisherRequest,
    SetPreferredBlossomServerRequest, UpdateBlossomServerRequest,
};
use crate::ui_v2::components::blossom_media_upload::{publisher_hex, stable_error_message};
use crate::ui_v2::components::PageHeader;
use crate::{
    invoke_get_allow_insecure_public_ws, invoke_set_allow_insecure_public_ws, AuthContext,
    StoredAccount,
};

#[path = "../../components/nip49_modal.rs"]
mod nip49_modal;
use nip49_modal::Nip49Modal;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct SettingsAvailability {
    account: bool,
    security_and_keys: bool,
    native_network: bool,
    backup_controls: bool,
    appearance_controls: bool,
    diagnostics: bool,
}

fn settings_availability(standalone_web: bool) -> SettingsAvailability {
    SettingsAvailability {
        account: true,
        security_and_keys: !standalone_web,
        native_network: !standalone_web,
        backup_controls: false,
        appearance_controls: false,
        diagnostics: true,
    }
}

fn relay_state_label(count: usize) -> String {
    match count {
        0 => "No relays connected".to_string(),
        1 => "1 relay connected".to_string(),
        count => format!("{count} relays connected"),
    }
}

fn account_name(account: &StoredAccount) -> String {
    account
        .display_name
        .clone()
        .filter(|value| !value.trim().is_empty())
        .or_else(|| {
            account
                .name
                .clone()
                .filter(|value| !value.trim().is_empty())
        })
        .or_else(|| {
            account
                .username
                .clone()
                .filter(|value| !value.trim().is_empty())
        })
        .unwrap_or_else(|| npub_fallback_label(&account.npub))
}

fn signer_label(signing_mode: &str) -> &'static str {
    match signing_mode.to_ascii_lowercase().as_str() {
        "nip46" | "remote" => "Remote signer",
        "local" | "nsec" => "Local encrypted key",
        "nip07" => "Browser extension",
        "readonly" | "read_only" => "Read only",
        _ => "Saved account",
    }
}

fn supports_local_export(account: Option<&StoredAccount>, standalone_web: bool) -> bool {
    !standalone_web
        && account
            .map(|account| {
                matches!(
                    account.signing_mode.to_ascii_lowercase().as_str(),
                    "local" | "nsec"
                )
            })
            .unwrap_or(false)
}

fn diagnostics_summary(
    target: &str,
    platform: Option<&PlatformInfo>,
    relay_count: usize,
    connection_status: &str,
    signing_mode: Option<&str>,
) -> String {
    let platform = platform
        .map(|platform| platform.tag())
        .unwrap_or_else(|| "unavailable".to_string());
    format!(
        "Arcadestr {}\nTarget: {target}\nPlatform: {platform}\nRelays: {relay_count}\nSigner connection: {connection_status}\nSigner type: {}",
        env!("CARGO_PKG_VERSION"),
        signing_mode.unwrap_or("unknown")
    )
}

fn verified_blossom_settings(
    expected_publisher: &str,
    settings: BlossomServerSettingsDto,
) -> Result<BlossomServerSettingsDto, String> {
    if settings.publisher_pubkey == expected_publisher {
        Ok(settings)
    } else {
        Err("The Blossom settings belong to a different account.".into())
    }
}

fn blossom_settings_error_message(code: &str) -> &'static str {
    match code {
        "invalid_request" => "That server is already configured or the settings are invalid.",
        _ => stable_error_message(code),
    }
}

fn is_development_default_blossom_server(origin: &str) -> bool {
    cfg!(debug_assertions) && origin.trim_end_matches('/') == "http://localhost:9099"
}

fn blossom_health_presentation(status: &str) -> (&'static str, &'static str) {
    match status {
        "online" => ("Online", "v2-blossom-health-online"),
        "slow" => ("Slow", "v2-blossom-health-slow"),
        "offline" => ("Offline", "v2-blossom-health-offline"),
        _ => ("Unknown", ""),
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum SettingsRemoval {
    Idle,
    Confirming { account_id: String, label: String },
}

#[component]
pub fn SettingsView(
    connected_relays: Signal<Vec<String>>,
    allow_insecure_public_ws: RwSignal<bool>,
    settings_error: RwSignal<Option<String>>,
    on_sign_out: Callback<()>,
) -> impl IntoView {
    let auth = use_context::<AuthContext>().expect("AuthContext not provided");
    let auth_stored = StoredValue::new(auth.clone());
    let standalone_web = cfg!(feature = "web");
    let availability = settings_availability(standalone_web);

    let platform = RwSignal::new(None::<PlatformInfo>);
    let platform_error = RwSignal::new(None::<String>);
    let action_status = RwSignal::new(None::<String>);
    let action_error = RwSignal::new(None::<String>);
    let reconnecting = RwSignal::new(false);
    let relay_reconnecting = RwSignal::new(false);
    let insecure_setting_saving = RwSignal::new(false);
    let removal = RwSignal::new(SettingsRemoval::Idle);
    let removal_dialog_ref = NodeRef::<leptos::html::Dialog>::new();
    let show_export = RwSignal::new(false);
    let export_account = RwSignal::new(String::new());
    let export_status = RwSignal::new(None::<String>);
    let diagnostic_copy_status = RwSignal::new(None::<String>);
    let blossom_settings = RwSignal::new(None::<BlossomServerSettingsDto>);
    let blossom_origin = RwSignal::new(String::new());
    let blossom_label = RwSignal::new(String::new());
    let blossom_error = RwSignal::new(None::<String>);
    let blossom_status = RwSignal::new(None::<String>);
    let blossom_loading = RwSignal::new(false);
    let blossom_busy = RwSignal::new(false);
    let blossom_generation = RwSignal::new(0_u64);
    let blossom_health = RwSignal::new(HashMap::<String, BlossomServerHealthDto>::new());
    let blossom_health_loading = RwSignal::new(false);
    let blossom_health_error = RwSignal::new(None::<String>);
    let blossom_health_generation = RwSignal::new(0_u64);
    let blossom_health_refresh = RwSignal::new(0_u64);

    Effect::new(move |_| {
        let auth = auth_stored.get_value();
        spawn_local(async move {
            if let Err(error) = auth.load_accounts_list().await {
                action_error.set(Some(error));
            }
            if availability.diagnostics && !standalone_web {
                match invoke_get_platform_info().await {
                    Ok(info) => platform.set(Some(info)),
                    Err(error) => platform_error.set(Some(error)),
                }
            }
        });
    });

    Effect::new(move |_| {
        let account = auth.npub.get();
        blossom_generation.update(|value| *value = value.wrapping_add(1));
        let generation = blossom_generation.get_untracked();
        blossom_busy.set(false);
        blossom_settings.set(None);
        blossom_error.set(None);
        blossom_status.set(None);
        if !availability.native_network {
            return;
        }
        let Some(account) = account else {
            blossom_loading.set(false);
            return;
        };
        let Some(expected_publisher_hex) = publisher_hex(&account) else {
            blossom_error.set(Some("The active publisher key is invalid.".into()));
            blossom_loading.set(false);
            return;
        };
        blossom_loading.set(true);
        spawn_local(async move {
            let result = invoke_get_blossom_server_settings(ExpectedBlossomPublisherRequest {
                expected_publisher_hex: expected_publisher_hex.clone(),
            })
            .await;
            if blossom_generation.get_untracked() != generation
                || auth.npub.get_untracked().as_deref() != Some(account.as_str())
            {
                return;
            }
            match result {
                Ok(settings) => {
                    match verified_blossom_settings(&expected_publisher_hex, settings) {
                        Ok(settings) => blossom_settings.set(Some(settings)),
                        Err(error) => blossom_error.set(Some(error)),
                    }
                }
                Err(error) => {
                    blossom_error.set(Some(blossom_settings_error_message(&error.code).into()));
                }
            }
            blossom_loading.set(false);
        });
    });

    Effect::new(move |_| {
        let _refresh = blossom_health_refresh.get();
        let settings = blossom_settings.get();
        blossom_health_generation.update(|value| *value = value.wrapping_add(1));
        let health_generation = blossom_health_generation.get_untracked();
        blossom_health.set(HashMap::new());
        blossom_health_error.set(None);
        let Some(settings) = settings else {
            blossom_health_loading.set(false);
            return;
        };
        let Some(account) = auth.npub.get_untracked() else {
            blossom_health_loading.set(false);
            return;
        };
        let Some(expected_publisher_hex) = publisher_hex(&account) else {
            blossom_health_loading.set(false);
            return;
        };
        if expected_publisher_hex != settings.publisher_pubkey || settings.servers.is_empty() {
            blossom_health_loading.set(false);
            return;
        }
        let account_generation = blossom_generation.get_untracked();
        blossom_health_loading.set(true);
        spawn_local(async move {
            let result = invoke_probe_blossom_server_health(ExpectedBlossomPublisherRequest {
                expected_publisher_hex: expected_publisher_hex.clone(),
            })
            .await;
            if blossom_health_generation.get_untracked() != health_generation
                || blossom_generation.get_untracked() != account_generation
                || auth.npub.get_untracked().as_deref() != Some(account.as_str())
            {
                return;
            }
            match result {
                Ok(response) if response.publisher_pubkey == expected_publisher_hex => {
                    blossom_health.set(
                        response
                            .servers
                            .into_iter()
                            .map(|server| (server.origin.clone(), server))
                            .collect(),
                    );
                }
                Ok(_) => blossom_health_error.set(Some(
                    "The Blossom health response belongs to a different account.".into(),
                )),
                Err(error) => blossom_health_error
                    .set(Some(blossom_settings_error_message(&error.code).into())),
            }
            blossom_health_loading.set(false);
        });
    });

    Effect::new(move |_| {
        let Some(dialog) = removal_dialog_ref.get() else {
            return;
        };
        if matches!(removal.get(), SettingsRemoval::Confirming { .. }) {
            if !dialog.open() {
                let _ = dialog.show_modal();
            }
        } else if dialog.open() {
            dialog.close();
        }
    });

    Effect::new(move |_| {
        let current_npub = auth.npub.get().unwrap_or_default();
        if show_export.get() && current_npub != export_account.get() {
            show_export.set(false);
            export_status.set(Some(
                "Encrypted-key export was cancelled because the active account changed."
                    .to_string(),
            ));
        }
    });

    let active_account = Signal::derive(move || auth.active_account.get());
    let active_name = Signal::derive(move || {
        auth.profile
            .get()
            .map(|profile| profile.display())
            .or_else(|| auth.active_account.get().as_ref().map(account_name))
            .or_else(|| auth.npub.get().map(|npub| npub_fallback_label(&npub)))
            .unwrap_or_else(|| "No active account".to_string())
    });
    let active_avatar = Signal::derive(move || {
        auth.profile
            .get()
            .and_then(|profile| profile.picture)
            .or_else(|| {
                auth.active_account
                    .get()
                    .and_then(|account| account.picture)
            })
    });
    let active_fallback = Signal::derive(move || {
        active_name
            .get()
            .chars()
            .next()
            .unwrap_or('?')
            .to_uppercase()
            .to_string()
    });
    let diagnostic_text = Signal::derive(move || {
        diagnostics_summary(
            if standalone_web {
                "standalone web"
            } else {
                "desktop"
            },
            platform.get().as_ref(),
            connected_relays.get().len(),
            &auth.connection_status.get(),
            auth.active_account
                .get()
                .as_ref()
                .map(|account| account.signing_mode.as_str()),
        )
    });

    let on_reconnect_signer = move |_| {
        let auth = auth_stored.get_value();
        if auth.is_loading.get_untracked() {
            action_error.set(Some(
                "Another account operation is already in progress.".to_string(),
            ));
            return;
        }
        reconnecting.set(true);
        action_error.set(None);
        action_status.set(Some("Reconnecting remote signer...".to_string()));
        let expected_npub = auth.npub.get_untracked();
        let generation = auth.begin_auth_flow();
        auth.is_loading.set(true);
        spawn_local(async move {
            let result = invoke_attempt_reconnect().await;
            if !auth.is_current_auth_flow(generation) || auth.npub.get_untracked() != expected_npub
            {
                return;
            }
            reconnecting.set(false);
            auth.is_loading.set(false);
            match result {
                Ok(_) => {
                    auth.connection_status.set("connecting".to_string());
                    action_status.set(Some("Signer reconnected; checking status...".to_string()));
                    auth.start_connection_status_polling().await;
                }
                Err(error) => {
                    auth.connection_status.set("failed".to_string());
                    auth.connection_error.set(Some(error.clone()));
                    action_error.set(Some(error));
                    action_status.set(None);
                }
            }
        });
    };

    let on_reconnect_relays = move |_| {
        relay_reconnecting.set(true);
        settings_error.set(None);
        spawn_local(async move {
            match invoke_reconnect_relays().await {
                Ok(message) => action_status.set(Some(message)),
                Err(error) => settings_error.set(Some(error)),
            }
            relay_reconnecting.set(false);
        });
    };

    let on_add_blossom_server = move |_| {
        if blossom_loading.get_untracked() || blossom_busy.get_untracked() {
            return;
        }
        let origin = blossom_origin.get_untracked().trim().to_string();
        if origin.is_empty() {
            blossom_error.set(Some("Enter a Blossom server URL.".into()));
            return;
        }
        let label = blossom_label.get_untracked().trim().to_string();
        let Some(account) = auth.npub.get_untracked() else {
            blossom_error.set(Some("Sign in before changing Blossom settings.".into()));
            return;
        };
        let Some(expected_publisher_hex) = publisher_hex(&account) else {
            blossom_error.set(Some("The active publisher key is invalid.".into()));
            return;
        };
        let generation = blossom_generation.get_untracked();
        blossom_busy.set(true);
        blossom_error.set(None);
        blossom_status.set(None);
        spawn_local(async move {
            let result = invoke_add_blossom_server(AddBlossomServerRequest {
                expected_publisher_hex: expected_publisher_hex.clone(),
                origin,
                label: (!label.is_empty()).then_some(label),
            })
            .await;
            if blossom_generation.get_untracked() != generation
                || auth.npub.get_untracked().as_deref() != Some(account.as_str())
            {
                return;
            }
            match result {
                Ok(settings) => {
                    match verified_blossom_settings(&expected_publisher_hex, settings) {
                        Ok(settings) => {
                            blossom_settings.set(Some(settings));
                            blossom_origin.set(String::new());
                            blossom_label.set(String::new());
                            blossom_status.set(Some("Blossom server added.".into()));
                        }
                        Err(error) => blossom_error.set(Some(error)),
                    }
                }
                Err(error) => {
                    blossom_error.set(Some(blossom_settings_error_message(&error.code).into()))
                }
            }
            if blossom_generation.get_untracked() == generation {
                blossom_busy.set(false);
            }
        });
    };

    let on_toggle_blossom_server = Callback::new(
        move |(origin, label, enabled): (String, Option<String>, bool)| {
            if blossom_loading.get_untracked() || blossom_busy.get_untracked() {
                return;
            }
            let Some(account) = auth.npub.get_untracked() else {
                return;
            };
            let Some(expected_publisher_hex) = publisher_hex(&account) else {
                return;
            };
            let generation = blossom_generation.get_untracked();
            blossom_busy.set(true);
            blossom_error.set(None);
            blossom_status.set(None);
            spawn_local(async move {
                let result = invoke_update_blossom_server(UpdateBlossomServerRequest {
                    expected_publisher_hex: expected_publisher_hex.clone(),
                    origin,
                    label,
                    enabled,
                })
                .await;
                if blossom_generation.get_untracked() == generation
                    && auth.npub.get_untracked().as_deref() == Some(account.as_str())
                {
                    match result {
                        Ok(settings) => {
                            match verified_blossom_settings(&expected_publisher_hex, settings) {
                                Ok(settings) => blossom_settings.set(Some(settings)),
                                Err(error) => blossom_error.set(Some(error)),
                            }
                        }
                        Err(error) => blossom_error
                            .set(Some(blossom_settings_error_message(&error.code).into())),
                    }
                }
                if blossom_generation.get_untracked() == generation {
                    blossom_busy.set(false);
                }
            });
        },
    );

    let on_remove_blossom_server = Callback::new(move |origin: String| {
        if blossom_loading.get_untracked() || blossom_busy.get_untracked() {
            return;
        }
        let Some(account) = auth.npub.get_untracked() else {
            return;
        };
        let Some(expected_publisher_hex) = publisher_hex(&account) else {
            return;
        };
        let generation = blossom_generation.get_untracked();
        blossom_busy.set(true);
        blossom_error.set(None);
        blossom_status.set(None);
        spawn_local(async move {
            let result = invoke_remove_blossom_server(BlossomServerOriginRequest {
                expected_publisher_hex: expected_publisher_hex.clone(),
                origin,
            })
            .await;
            if blossom_generation.get_untracked() == generation
                && auth.npub.get_untracked().as_deref() == Some(account.as_str())
            {
                match result {
                    Ok(settings) => {
                        match verified_blossom_settings(&expected_publisher_hex, settings) {
                            Ok(settings) => {
                                blossom_settings.set(Some(settings));
                                blossom_status.set(Some("Blossom server removed.".into()));
                            }
                            Err(error) => blossom_error.set(Some(error)),
                        }
                    }
                    Err(error) => {
                        blossom_error.set(Some(blossom_settings_error_message(&error.code).into()))
                    }
                }
            }
            if blossom_generation.get_untracked() == generation {
                blossom_busy.set(false);
            }
        });
    });

    let on_prefer_blossom_server = Callback::new(move |origin: String| {
        if blossom_loading.get_untracked() || blossom_busy.get_untracked() {
            return;
        }
        let Some(account) = auth.npub.get_untracked() else {
            return;
        };
        let Some(expected_publisher_hex) = publisher_hex(&account) else {
            return;
        };
        let generation = blossom_generation.get_untracked();
        blossom_busy.set(true);
        blossom_error.set(None);
        blossom_status.set(None);
        spawn_local(async move {
            let result = invoke_set_preferred_blossom_server(SetPreferredBlossomServerRequest {
                expected_publisher_hex: expected_publisher_hex.clone(),
                origin: Some(origin),
            })
            .await;
            if blossom_generation.get_untracked() == generation
                && auth.npub.get_untracked().as_deref() == Some(account.as_str())
            {
                match result {
                    Ok(settings) => {
                        match verified_blossom_settings(&expected_publisher_hex, settings) {
                            Ok(settings) => {
                                blossom_settings.set(Some(settings));
                                blossom_status
                                    .set(Some("Preferred Blossom server updated.".into()));
                            }
                            Err(error) => blossom_error.set(Some(error)),
                        }
                    }
                    Err(error) => {
                        blossom_error.set(Some(blossom_settings_error_message(&error.code).into()))
                    }
                }
            }
            if blossom_generation.get_untracked() == generation {
                blossom_busy.set(false);
            }
        });
    });

    let on_export = Callback::new(move |result: Nip49ExportResult| {
        export_status.set(Some(if result.deferred {
            "Encrypted-key export was deferred by the signer state.".to_string()
        } else {
            "Encrypted-key export completed. Close the dialog to clear it from the screen."
                .to_string()
        }));
    });

    let on_copy_diagnostics = move |_| {
        let _summary = diagnostic_text.get_untracked();
        #[cfg(target_arch = "wasm32")]
        spawn_local(async move {
            let Some(window) = web_sys::window() else {
                diagnostic_copy_status.set(Some("Clipboard unavailable.".to_string()));
                return;
            };
            let promise = window.navigator().clipboard().write_text(&_summary);
            match wasm_bindgen_futures::JsFuture::from(promise).await {
                Ok(_) => diagnostic_copy_status.set(Some("Diagnostic summary copied.".to_string())),
                Err(_) => {
                    diagnostic_copy_status.set(Some("Clipboard permission denied.".to_string()))
                }
            }
        });
        #[cfg(not(target_arch = "wasm32"))]
        diagnostic_copy_status.set(Some(
            "Clipboard is available in the webview runtime.".to_string(),
        ));
    };

    view! {
        <section class="v2-settings-wrap">
            <PageHeader
                eyebrow="Account and client".to_string()
                title="Settings".to_string()
                description="Manage real signer, network, key-export, and application state. Unsupported controls are intentionally absent.".to_string()
            />

            <Show when=move || action_error.get().is_some()>
                <p class="v2-settings-alert v2-settings-alert-error" role="alert">{move || action_error.get().unwrap_or_default()}</p>
            </Show>
            <Show when=move || action_status.get().is_some()>
                <p class="v2-settings-alert" role="status">{move || action_status.get().unwrap_or_default()}</p>
            </Show>

            <div class="v2-settings-grid">
                <section class="v2-settings-card v2-settings-account-card">
                    <header class="v2-settings-card-header">
                        <span class="v2-settings-icon material-symbols-outlined" aria-hidden="true">"person"</span>
                        <div><p class="v2-store-kicker">"Identity"</p><h2>"Account and signer"</h2></div>
                    </header>
                    <div class="v2-settings-active-account">
                        {move || match active_avatar.get() {
                            Some(url) => view! { <img src=url alt="" class="v2-settings-avatar" /> }.into_any(),
                            None => view! { <div class="v2-settings-avatar v2-settings-avatar-fallback" aria-hidden="true">{move || active_fallback.get()}</div> }.into_any(),
                        }}
                        <div>
                            <h3>{move || active_name.get()}</h3>
                            <p>{move || auth.npub.get().map(|npub| npub_fallback_label(&npub)).unwrap_or_else(|| "Signed out".to_string())}</p>
                            <p>{move || {
                                let mode = active_account.get().map(|account| signer_label(&account.signing_mode)).unwrap_or("Unknown signer");
                                format!("{mode} · {}", auth.connection_status.get())
                            }}</p>
                        </div>
                    </div>
                    {move || auth.connection_error.get().map(|error| view! { <p class="v2-settings-alert v2-settings-alert-error">{error}</p> })}
                    <div class="v2-settings-actions">
                        <Show when=move || !standalone_web && active_account.get().map(|account| account.signing_mode.eq_ignore_ascii_case("nip46")).unwrap_or(false)>
                            <button class="v2-btn-secondary" on:click=on_reconnect_signer disabled=move || reconnecting.get() || auth_stored.get_value().is_loading.get()>
                                {move || if reconnecting.get() { "Reconnecting..." } else { "Reconnect signer" }}
                            </button>
                        </Show>
                        <button class="v2-btn-danger" disabled=move || auth_stored.get_value().is_loading.get() on:click=move |_| on_sign_out.run(())>"Sign out"</button>
                    </div>

                    <div class="v2-settings-account-list" aria-label="Saved accounts">
                        {move || auth.accounts.get().into_iter().map(|account| {
                            let account_id = account.id.clone();
                            let remove_id = account.id.clone();
                            let label = account_name(&account);
                            let remove_label = label.clone();
                            let is_active = auth.npub.get().as_deref() == Some(account.npub.as_str());
                            view! {
                                <article class="v2-settings-account-row">
                                    <div><strong>{label}</strong><span>{npub_fallback_label(&account.npub)}</span><span>{signer_label(&account.signing_mode)}</span></div>
                                    <div class="v2-settings-row-actions">
                                        <button class="v2-btn-ghost" disabled=move || is_active || auth_stored.get_value().is_loading.get() on:click=move |_| {
                                            let auth = auth_stored.get_value();
                                            let account_id = account_id.clone();
                                            spawn_local(async move {
                                                if let Err(error) = auth.switch_account(account_id).await {
                                                    auth.error.set(Some(error));
                                                }
                                            });
                                        }>{if is_active { "Active" } else { "Switch" }}</button>
                                        <button class="v2-btn-ghost v2-btn-danger" disabled=move || auth_stored.get_value().is_loading.get() on:click=move |_| removal.set(SettingsRemoval::Confirming { account_id: remove_id.clone(), label: remove_label.clone() })>"Remove"</button>
                                    </div>
                                </article>
                            }
                        }).collect::<Vec<_>>()}
                    </div>
                </section>

                <Show when=move || availability.security_and_keys>
                    <section class="v2-settings-card">
                        <header class="v2-settings-card-header">
                            <span class="v2-settings-icon material-symbols-outlined" aria-hidden="true">"key"</span>
                            <div><p class="v2-store-kicker">"Local protection"</p><h2>"Security and keys"</h2></div>
                        </header>
                        <p>"NIP-49 export encrypts a local secret key with a new password. Remote signers do not expose their keys to Arcadestr."</p>
                        {move || if supports_local_export(active_account.get().as_ref(), standalone_web) {
                            view! { <button class="v2-btn-primary" disabled=move || auth_stored.get_value().is_loading.get() on:click=move |_| {
                                export_account.set(auth.npub.get_untracked().unwrap_or_default());
                                show_export.set(true);
                            }>"Export encrypted NIP-49 key"</button> }.into_any()
                        } else {
                            view! { <p class="v2-settings-muted">"Encrypted-key export is unavailable for the active signer type."</p> }.into_any()
                        }}
                        {move || export_status.get().map(|status| view! { <p class="v2-settings-alert" role="status">{status}</p> })}
                    </section>
                </Show>

                <Show when=move || availability.native_network>
                    <section class="v2-settings-card">
                        <header class="v2-settings-card-header">
                            <span class="v2-settings-icon material-symbols-outlined" aria-hidden="true">"hub"</span>
                            <div><p class="v2-store-kicker">"Nostr network"</p><h2>"Network and relays"</h2></div>
                        </header>
                        <p>"Relays are network servers used to receive and publish Nostr events."</p>
                        <strong>{move || relay_state_label(connected_relays.get().len())}</strong>
                        {move || if connected_relays.get().is_empty() {
                            view! { <p class="v2-settings-muted">"The client currently has no confirmed relay connections."</p> }.into_any()
                        } else {
                            view! { <ul class="v2-settings-relay-list">{connected_relays.get().into_iter().map(|relay| view! { <li>{relay}</li> }).collect::<Vec<_>>()}</ul> }.into_any()
                        }}
                        <button class="v2-btn-secondary" on:click=on_reconnect_relays disabled=move || relay_reconnecting.get()>
                            {move || if relay_reconnecting.get() { "Reconnecting..." } else { "Reconnect relays" }}
                        </button>
                        <label class="v2-settings-toggle-row">
                            <span><strong>"Allow insecure public ws:// relays"</strong><small>"Keep off for safety. Local development relays remain allowed."</small></span>
                            <input type="checkbox" disabled=move || insecure_setting_saving.get() prop:checked=move || allow_insecure_public_ws.get() on:change=move |event| {
                                let next = event_target_checked(&event);
                                settings_error.set(None);
                                allow_insecure_public_ws.set(next);
                                insecure_setting_saving.set(true);
                                spawn_local(async move {
                                    if let Err(error) = invoke_set_allow_insecure_public_ws(next).await {
                                        settings_error.set(Some(error));
                                        if let Ok(current) = invoke_get_allow_insecure_public_ws().await {
                                            allow_insecure_public_ws.set(current);
                                        }
                                    }
                                    insecure_setting_saving.set(false);
                                });
                            } />
                        </label>
                        {move || settings_error.get().map(|error| view! { <p class="v2-settings-alert v2-settings-alert-error" role="alert">{error}</p> })}
                    </section>
                </Show>

                <Show when=move || availability.native_network>
                    <section class="v2-settings-card" aria-labelledby="settings-blossom-title">
                        <header class="v2-settings-card-header">
                            <span class="v2-settings-icon material-symbols-outlined" aria-hidden="true">"cloud_upload"</span>
                            <div><p class="v2-store-kicker">"Media hosting"</p><h2 id="settings-blossom-title">"Blossom servers"</h2></div>
                        </header>
                        <div class="flex flex-col items-start justify-between gap-3 sm:flex-row sm:items-center">
                            <p>"Blossom servers host game covers, screenshots, trailers, and Store Page media. Settings are scoped to the active account."</p>
                            <button type="button" class="v2-btn-ghost shrink-0" disabled=move || blossom_loading.get() || blossom_health_loading.get() || blossom_settings.get().is_none() on:click=move |_| blossom_health_refresh.update(|value| *value = value.wrapping_add(1))>{move || if blossom_health_loading.get() { "Checking…" } else { "Refresh status" }}</button>
                        </div>
                        <Show when=move || blossom_loading.get()>
                            <p class="v2-settings-muted" role="status">"Loading Blossom servers…"</p>
                        </Show>
                        {move || blossom_settings.get().map(|settings| {
                            let preferred = settings.preferred_server;
                            if settings.servers.is_empty() {
                                view! { <p class="v2-settings-muted">"No Blossom servers are configured."</p> }.into_any()
                            } else {
                                view! {
                                    <div class="v2-settings-account-list" aria-label="Configured Blossom servers">
                                        {settings.servers.into_iter().map(|server| {
                                            let origin_toggle = server.origin.clone();
                                            let origin_preferred = server.origin.clone();
                                            let origin_remove = server.origin.clone();
                                            let origin_health = server.origin.clone();
                                            let label_toggle = server.label.clone();
                                            let is_preferred = preferred.as_deref() == Some(server.origin.as_str());
                                            let is_development_default = is_development_default_blossom_server(&server.origin);
                                            let display = server.label.clone().unwrap_or_else(|| server.origin.clone());
                                            view! {
                                                <article class="v2-settings-account-row">
                                                    <div class="min-w-0"><strong>{display}</strong><span class="break-all font-mono">{server.origin}</span><span>{if is_preferred { "Preferred upload destination" } else if server.enabled { "Enabled" } else { "Disabled" }}</span></div>
                                                    <div class="v2-settings-row-actions">
                                                        {move || {
                                                            let health = blossom_health.get().get(&origin_health).cloned();
                                                            match health {
                                                                Some(health) => {
                                                                    let (label, color) = blossom_health_presentation(&health.status);
                                                                    let title = health.latency_ms.map(|latency| format!("{label} · {latency} ms")).unwrap_or_else(|| label.to_string());
                                                                    view! { <span class=format!("inline-flex items-center gap-1.5 text-xs font-bold {color}") title=title><span class="v2-blossom-health-dot h-2.5 w-2.5 rounded-full" aria-hidden="true"></span>{label}</span> }.into_any()
                                                                }
                                                                None if blossom_health_loading.get() => view! { <span class="inline-flex items-center gap-1.5 text-xs font-bold text-on-surface-variant" role="status"><span class="h-2.5 w-2.5 animate-pulse rounded-full bg-current" aria-hidden="true"></span>"Checking"</span> }.into_any(),
                                                                None => view! { <span class="inline-flex items-center gap-1.5 text-xs font-bold text-on-surface-variant"><span class="h-2.5 w-2.5 rounded-full bg-current" aria-hidden="true"></span>"Unknown"</span> }.into_any(),
                                                            }
                                                        }}
                                                        <label class="flex items-center gap-2 text-xs font-bold"><input type="checkbox" prop:checked=server.enabled disabled=move || blossom_busy.get() on:change=move |event| on_toggle_blossom_server.run((origin_toggle.clone(), label_toggle.clone(), event_target_checked(&event))) />"Enabled"</label>
                                                        <button type="button" class="v2-btn-ghost" disabled=move || is_preferred || !server.enabled || blossom_busy.get() on:click=move |_| on_prefer_blossom_server.run(origin_preferred.clone())>{if is_preferred { "Preferred" } else { "Make preferred" }}</button>
                                                        <button type="button" class="v2-btn-ghost v2-btn-danger" disabled=move || is_development_default || blossom_busy.get() on:click=move |_| on_remove_blossom_server.run(origin_remove.clone())>{if is_development_default { "Development default" } else { "Remove" }}</button>
                                                    </div>
                                                </article>
                                            }
                                        }).collect::<Vec<_>>()}
                                    </div>
                                }.into_any()
                            }
                        })}
                        <div class="grid gap-3 md:grid-cols-2">
                            <label class="text-sm font-bold">"Server URL"<input type="url" class="v2-input mt-2" placeholder="https://blossom.example" prop:value=move || blossom_origin.get() on:input=move |event| { blossom_origin.set(event_target_value(&event)); blossom_error.set(None); } disabled=move || blossom_loading.get() || blossom_busy.get() /></label>
                            <label class="text-sm font-bold">"Label (optional)"<input class="v2-input mt-2" placeholder="Primary media server" prop:value=move || blossom_label.get() on:input=move |event| blossom_label.set(event_target_value(&event)) disabled=move || blossom_loading.get() || blossom_busy.get() /></label>
                        </div>
                        <button type="button" class="v2-btn-primary" on:click=on_add_blossom_server disabled=move || blossom_loading.get() || blossom_busy.get() || blossom_origin.get().trim().is_empty()>{move || if blossom_busy.get() { "Saving…" } else { "Add server" }}</button>
                        {move || blossom_error.get().map(|error| view! { <p class="v2-settings-alert v2-settings-alert-error" role="alert">{error}</p> })}
                        {move || blossom_health_error.get().map(|error| view! { <p class="v2-settings-alert v2-settings-alert-error" role="alert">{format!("Server status check failed: {error}")}</p> })}
                        {move || blossom_status.get().map(|status| view! { <p class="v2-settings-alert" role="status">{status}</p> })}
                    </section>
                </Show>

                <section class="v2-settings-card">
                    <header class="v2-settings-card-header">
                        <span class="v2-settings-icon material-symbols-outlined" aria-hidden="true">"backup"</span>
                        <div><p class="v2-store-kicker">"Recovery"</p><h2>"Account backup"</h2></div>
                    </header>
                    <p class="v2-settings-muted">"Full account backup and restore are unavailable because this build has no registered create or restore commands. No nonfunctional controls are shown."</p>
                </section>

                <section class="v2-settings-card v2-settings-diagnostics">
                    <header class="v2-settings-card-header">
                        <span class="v2-settings-icon material-symbols-outlined" aria-hidden="true">"info"</span>
                        <div><p class="v2-store-kicker">"Application"</p><h2>"Information and diagnostics"</h2></div>
                    </header>
                    <div class="v2-settings-diagnostic-grid">
                        <div><span>"Version"</span><strong>{env!("CARGO_PKG_VERSION")}</strong></div>
                        <div><span>"Target"</span><strong>{if standalone_web { "Standalone web" } else { "Desktop" }}</strong></div>
                        <div><span>"Platform"</span><strong>{move || platform.get().map(|value| value.tag()).unwrap_or_else(|| "Unavailable".to_string())}</strong></div>
                        <div><span>"Network"</span><strong>{move || relay_state_label(connected_relays.get().len())}</strong></div>
                    </div>
                    {move || platform_error.get().map(|error| view! { <p class="v2-settings-muted">{format!("Platform detection unavailable: {error}")}</p> })}
                    <p class="v2-settings-muted">"For troubleshooting, launch Arcadestr from a terminal to capture application logs. The summary below contains no keys or authentication tokens."</p>
                    <pre class="v2-settings-diagnostic-summary">{move || diagnostic_text.get()}</pre>
                    <button class="v2-btn-secondary" on:click=on_copy_diagnostics>"Copy non-sensitive summary"</button>
                    {move || diagnostic_copy_status.get().map(|status| view! { <p class="v2-settings-alert" role="status">{status}</p> })}
                </section>
            </div>

            <dialog
                node_ref=removal_dialog_ref
                class="v2-confirm-backdrop"
                aria-labelledby="settings-remove-account-title"
                on:cancel=move |event: web_sys::Event| {
                    event.prevent_default();
                    removal.set(SettingsRemoval::Idle);
                }
                on:click=move |_| removal.set(SettingsRemoval::Idle)
            >
                <section class="v2-confirm-dialog" on:click=move |event| event.stop_propagation()>
                    <h2 id="settings-remove-account-title">"Remove saved account?"</h2>
                    {move || match removal.get() {
                        SettingsRemoval::Idle => view! { <></> }.into_any(),
                        SettingsRemoval::Confirming { account_id, label } => view! {
                            <div>
                                <p>{format!("Remove {label} from this device? This does not delete the Nostr identity.")}</p>
                                <div class="v2-settings-actions">
                                    <button class="v2-btn-ghost" autofocus on:click=move |_| removal.set(SettingsRemoval::Idle)>"Cancel"</button>
                                    <button class="v2-btn-danger" on:click=move |_| {
                                        let auth = auth_stored.get_value();
                                        let account_id = account_id.clone();
                                        removal.set(SettingsRemoval::Idle);
                                        spawn_local(async move {
                                            if let Err(error) = auth.delete_account(account_id).await {
                                                action_error.set(Some(error));
                                            }
                                        });
                                    }>"Remove account"</button>
                                </div>
                            </div>
                        }.into_any(),
                    }}
                </section>
            </dialog>

            <Nip49Modal
                show=show_export.into()
                npub=Signal::derive(move || export_account.get())
                on_export=on_export
                on_cancel=Callback::new(move |_| show_export.set(false))
            />
        </section>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn web_hides_native_security_and_network_actions() {
        let web = settings_availability(true);
        assert!(web.account && web.diagnostics);
        assert!(!web.security_and_keys && !web.native_network);
    }

    #[test]
    fn unsupported_settings_remain_absent() {
        let desktop = settings_availability(false);
        assert!(!desktop.backup_controls);
        assert!(!desktop.appearance_controls);
    }

    #[test]
    fn relay_presentation_distinguishes_disconnected_state() {
        assert_eq!(relay_state_label(0), "No relays connected");
        assert_eq!(relay_state_label(1), "1 relay connected");
        assert_eq!(relay_state_label(3), "3 relays connected");
    }

    #[test]
    fn only_local_desktop_accounts_can_export_keys() {
        let mut account = StoredAccount {
            id: "id".to_string(),
            npub: "npub1test".to_string(),
            name: None,
            signing_mode: "local".to_string(),
            last_used: 0,
            is_current: true,
            picture: None,
            display_name: None,
            username: None,
            nip05: None,
            about: None,
        };
        assert!(supports_local_export(Some(&account), false));
        assert!(!supports_local_export(Some(&account), true));
        account.signing_mode = "nip46".to_string();
        assert!(!supports_local_export(Some(&account), false));
    }

    #[test]
    fn diagnostics_summary_contains_no_account_identifier() {
        let summary = diagnostics_summary("desktop", None, 0, "disconnected", Some("local"));
        assert!(summary.contains("No") || !summary.contains("npub"));
        assert!(!summary.contains("nsec"));
    }

    #[test]
    fn blossom_settings_reject_cross_account_responses() {
        let settings = BlossomServerSettingsDto {
            publisher_pubkey: "publisher-a".into(),
            servers: vec![],
            preferred_server: None,
        };
        assert!(verified_blossom_settings("publisher-a", settings.clone()).is_ok());
        assert!(verified_blossom_settings("publisher-b", settings).is_err());
        assert!(blossom_settings_error_message("invalid_request").contains("already configured"));
        assert_eq!(
            is_development_default_blossom_server("http://localhost:9099/"),
            cfg!(debug_assertions)
        );
        assert_eq!(blossom_health_presentation("online").0, "Online");
        assert_eq!(blossom_health_presentation("slow").0, "Slow");
        assert_eq!(blossom_health_presentation("offline").0, "Offline");
    }

    #[test]
    fn source_omits_backup_and_appearance_controls() {
        let source = include_str!("settings.rs");
        assert!(!source.contains(concat!("Generate ", "Backup")));
        assert!(!source.contains(concat!("Restore ", "Accounts")));
        assert!(!source.contains(concat!("Reduce ", "motion")));
        assert!(!source.contains(concat!("Larger ", "text")));
    }
}
