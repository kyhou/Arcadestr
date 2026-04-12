use leptos::ev::MouseEvent;
use leptos::prelude::*;

use crate::models::Nip49ExportResult;

/// Modal UI for local NIP-49 export interaction.
#[component]
pub fn Nip49Modal(
    show: Signal<bool>,
    npub: String,
    on_export: Callback<Nip49ExportResult>,
    on_cancel: Callback<()>,
) -> impl IntoView {
    let password = RwSignal::new(String::new());
    let confirm_password = RwSignal::new(String::new());
    let validation_error = RwSignal::new(None::<String>);
    let export_result = RwSignal::new(None::<Nip49ExportResult>);
    let copy_status = RwSignal::new(None::<String>);

    let can_export = Signal::derive(move || {
        let password_value = password.get();
        let confirm_value = confirm_password.get();
        !password_value.is_empty() && password_value == confirm_value
    });

    let reset_form = {
        let password = password;
        let confirm_password = confirm_password;
        let validation_error = validation_error;
        let copy_status = copy_status;
        let export_result = export_result;
        move || {
            password.set(String::new());
            confirm_password.set(String::new());
            validation_error.set(None);
            copy_status.set(None);
            export_result.set(None);
        }
    };

    Effect::new(move |_| {
        if !show.get() {
            reset_form();
        }
    });

    let handle_cancel = {
        let reset_form = reset_form.clone();
        let on_cancel = on_cancel.clone();
        move || {
            reset_form();
            on_cancel.run(());
        }
    };

    let on_backdrop_click = {
        let handle_cancel = handle_cancel.clone();
        move |_| handle_cancel()
    };

    let on_panel_click = move |event: MouseEvent| {
        event.stop_propagation();
    };

    let on_keydown = {
        let handle_cancel = handle_cancel.clone();
        move |event: leptos::ev::KeyboardEvent| {
            if event.key() == "Escape" && show.get() {
                handle_cancel();
            }
        }
    };

    let on_close_button_click = {
        let handle_cancel = handle_cancel.clone();
        move |_| handle_cancel()
    };

    let on_cancel_button_click = {
        let handle_cancel = handle_cancel.clone();
        move |_| handle_cancel()
    };

    let copy_result = move |_| {
        if let Some(_result) = export_result.get() {
            #[cfg(target_arch = "wasm32")]
            {
                let result = _result;
                if let Some(window) = web_sys::window() {
                    let _ = window.navigator().clipboard().write_text(&result.ncryptsec);
                    copy_status.set(Some("Copied to clipboard.".to_string()));
                } else {
                    copy_status.set(Some(
                        "Clipboard unavailable in this environment.".to_string(),
                    ));
                }
            }

            #[cfg(not(target_arch = "wasm32"))]
            {
                let _ = &_result;
                copy_status.set(Some(
                    "Clipboard copy is available in wasm/webview runtime.".to_string(),
                ));
            }
        }
    };

    view! {
        <Show when=move || show.get()>
            <div class="nip49-modal-backdrop" tabindex="0" on:click=on_backdrop_click on:keydown=on_keydown>
                <div class="nip49-modal-panel" on:click=on_panel_click>
                    <header class="nip49-modal-header">
                        <h3>"Export NIP-49 (ncryptsec)"</h3>
                        <button class="nip49-modal-close" aria-label="Close modal" on:click=on_close_button_click>
                            "×"
                        </button>
                    </header>

                    <p class="nip49-modal-warning">
                        "Keep your export password safe. Losing it can make the backup unrecoverable."
                    </p>

                    <div class="nip49-modal-field">
                        <label for="nip49-password">"Password"</label>
                        <input
                            id="nip49-password"
                            type="password"
                            bind:value=password
                            autocomplete="new-password"
                            placeholder="Create export password"
                        />
                    </div>

                    <div class="nip49-modal-field">
                        <label for="nip49-password-confirm">"Confirm password"</label>
                        <input
                            id="nip49-password-confirm"
                            type="password"
                            bind:value=confirm_password
                            autocomplete="new-password"
                            placeholder="Repeat password"
                        />
                    </div>

                    <p class="nip49-modal-npub">{format!("Account: {}", npub)}</p>

                    <Show when=move || validation_error.get().is_some()>
                        <p class="nip49-modal-error">{move || validation_error.get().unwrap_or_default()}</p>
                    </Show>

                    <div class="nip49-modal-actions">
                        <button class="nip49-modal-cancel" on:click=on_cancel_button_click>
                            "Cancel"
                        </button>
                        <button
                            class="nip49-modal-export"
                            on:click={
                                let npub = npub.clone();
                                let on_export = on_export.clone();
                                move |_| {
                                    let password_value = password.get();
                                    let confirm_value = confirm_password.get();

                                    if password_value.is_empty() {
                                        validation_error.set(Some("Password is required.".to_string()));
                                        return;
                                    }

                                    if password_value != confirm_value {
                                        validation_error.set(Some("Passwords do not match.".to_string()));
                                        return;
                                    }

                                    validation_error.set(None);

                                    let result = Nip49ExportResult {
                                        npub: npub.clone(),
                                        ncryptsec: format!("ncryptsec-export-pending-{}", npub),
                                        deferred: true,
                                        message:
                                            "Export prepared in UI only. Desktop command wiring generates the real ncryptsec."
                                                .to_string(),
                                    };

                                    export_result.set(Some(result.clone()));
                                    on_export.run(result);
                                }
                            }
                            disabled=move || !can_export.get()
                        >
                            "Export"
                        </button>
                    </div>

                    <Show when=move || export_result.get().is_some()>
                        <div class="nip49-modal-result">
                            <p class="nip49-modal-result-message">
                                {move || {
                                    export_result
                                        .get()
                                        .map(|result| result.message)
                                        .unwrap_or_default()
                                }}
                            </p>
                            <textarea
                                readonly
                                class="nip49-modal-result-text"
                                prop:value=move || {
                                    export_result
                                        .get()
                                        .map(|result| result.ncryptsec)
                                        .unwrap_or_default()
                                }
                                rows="3"
                            />
                            <button class="nip49-modal-copy" on:click=copy_result>
                                "Copy to clipboard"
                            </button>
                            <Show when=move || copy_status.get().is_some()>
                                <p class="nip49-modal-copy-status">{move || copy_status.get().unwrap_or_default()}</p>
                            </Show>
                        </div>
                    </Show>
                </div>
            </div>
        </Show>
    }
}
