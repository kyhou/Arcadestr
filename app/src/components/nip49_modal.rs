use leptos::ev::MouseEvent;
use leptos::prelude::*;
use wasm_bindgen_futures::spawn_local;

use crate::models::Nip49ExportResult;
use crate::tauri_bridge::invoke_nip49_export;
use crate::ui_v2::components::{
    Dialog, DialogCloseAction, DialogCloseButtonPolicy, DialogClosePolicy, DialogCloseRequest,
    DialogDismissal, DialogInitialFocus, DialogSourcePolicy, DialogWidth,
};

/// The declared dismissal contract for the encrypted-key export dialog.
///
/// The encryption call has no cancellation path, so dismissal is refused
/// outright while it runs rather than silently orphaning the operation.
fn export_close_contract() -> (DialogClosePolicy, DialogDismissal) {
    (
        DialogClosePolicy::BlockedWhileBusy,
        DialogDismissal {
            escape: DialogSourcePolicy::Allowed,
            backdrop: DialogSourcePolicy::Allowed,
            close_button: DialogCloseButtonPolicy::DisabledWhileBusy,
        },
    )
}

const MIN_EXPORT_PASSWORD_CHARS: usize = 8;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum SecretVisibility {
    #[default]
    Hidden,
    Revealed,
}

fn validate_export_password(password: &str, confirmation: &str) -> Result<(), &'static str> {
    if password.chars().count() < MIN_EXPORT_PASSWORD_CHARS {
        return Err("Use at least 8 characters for the export password.");
    }
    if password != confirmation {
        return Err("Passwords do not match.");
    }
    Ok(())
}

fn export_result_label(result: &Nip49ExportResult) -> &'static str {
    if result.deferred {
        "Export deferred"
    } else {
        "Encrypted key ready"
    }
}

fn cleared_secret_state() -> (String, String, SecretVisibility) {
    (String::new(), String::new(), SecretVisibility::Hidden)
}

/// Modal UI for an explicit, local NIP-49 encrypted-key export.
#[component]
pub fn Nip49Modal(
    show: Signal<bool>,
    npub: Signal<String>,
    on_export: Callback<Nip49ExportResult>,
    on_cancel: Callback<()>,
) -> impl IntoView {
    let password = RwSignal::new(String::new());
    let confirm_password = RwSignal::new(String::new());
    let validation_error = RwSignal::new(None::<String>);
    let export_result = RwSignal::new(None::<Nip49ExportResult>);
    let visibility = RwSignal::new(SecretVisibility::Hidden);
    let copy_confirmation = RwSignal::new(false);
    let copy_status = RwSignal::new(None::<String>);
    let exporting = RwSignal::new(false);
    let request_generation = RwSignal::new(0_u64);
    let password_ref = NodeRef::<leptos::html::Input>::new();

    let reset_form = move || {
        let (cleared_password, cleared_confirmation, cleared_visibility) = cleared_secret_state();
        request_generation.update(|generation| *generation = generation.wrapping_add(1));
        password.set(cleared_password);
        confirm_password.set(cleared_confirmation);
        validation_error.set(None);
        export_result.set(None);
        visibility.set(cleared_visibility);
        copy_confirmation.set(false);
        copy_status.set(None);
        exporting.set(false);
    };

    // The dialog primitive owns open/close; this preserves the existing
    // secret-clearing behaviour on the open -> closed transition only.
    let was_open = StoredValue::new(false);
    Effect::new(move |_| {
        let is_visible = show.get();
        if was_open.get_value() && !is_visible {
            reset_form();
        }
        was_open.set_value(is_visible);
    });

    let handle_cancel = {
        let on_cancel = on_cancel.clone();
        move || {
            if exporting.get_untracked() {
                return;
            }
            reset_form();
            on_cancel.run(());
        }
    };

    let on_export_click = {
        let on_export = on_export.clone();
        Callback::new(move |_: MouseEvent| {
            let password_value = password.get();
            let confirmation_value = confirm_password.get();
            if let Err(message) = validate_export_password(&password_value, &confirmation_value) {
                validation_error.set(Some(message.to_string()));
                return;
            }

            request_generation.update(|generation| *generation = generation.wrapping_add(1));
            let generation = request_generation.get_untracked();
            exporting.set(true);
            validation_error.set(None);
            export_result.set(None);
            visibility.set(SecretVisibility::Hidden);
            copy_confirmation.set(false);
            copy_status.set(None);
            password.set(String::new());
            confirm_password.set(String::new());

            let npub = npub.get_untracked();
            let on_export = on_export.clone();
            spawn_local(async move {
                let result = invoke_nip49_export(npub, password_value).await;
                if request_generation.get_untracked() != generation {
                    return;
                }
                exporting.set(false);
                match result {
                    Ok(result) => {
                        export_result.set(Some(result.clone()));
                        on_export.run(result);
                    }
                    Err(error) => validation_error.set(Some(error)),
                }
            });
        })
    };

    let on_confirm_copy = Callback::new(move |_: MouseEvent| {
        let Some(_result) = export_result.get_untracked() else {
            return;
        };
        copy_confirmation.set(false);

        #[cfg(target_arch = "wasm32")]
        spawn_local(async move {
            let Some(window) = web_sys::window() else {
                copy_status.set(Some(
                    "Clipboard unavailable in this environment.".to_string(),
                ));
                return;
            };
            let promise = window
                .navigator()
                .clipboard()
                .write_text(&_result.ncryptsec);
            match wasm_bindgen_futures::JsFuture::from(promise).await {
                Ok(_) => copy_status.set(Some("Encrypted key copied.".to_string())),
                Err(_) => copy_status.set(Some(
                    "Clipboard permission was denied. Select the revealed value manually."
                        .to_string(),
                )),
            }
        });

        #[cfg(not(target_arch = "wasm32"))]
        copy_status.set(Some(
            "Clipboard copy is available in the webview runtime.".to_string(),
        ));
    });
    let export_click_stored = StoredValue::new(on_export_click);
    let confirm_copy_stored = StoredValue::new(on_confirm_copy);

    view! {
        <Dialog
            id="nip49-export"
            open=Signal::derive(move || show.get())
            title="Export encrypted key"
            kicker="Security and keys"
            description="This creates an encrypted ncryptsec value for local-key accounts. Remote signers keep keys outside Arcadestr and cannot be exported here."
            width=DialogWidth::Standard
            // The encryption call cannot be cancelled, so dismissal is refused
            // outright while it runs rather than silently orphaning it.
            policy=export_close_contract().0
            dismissal=export_close_contract().1
            busy=Signal::derive(move || exporting.get())
            initial_focus=DialogInitialFocus::Input(password_ref)
            close_label="Cancel encrypted-key export"
            close_blocked_hint="Encrypting the key. This dialog will remain open until the operation finishes."
            on_close={
                let handle_cancel = handle_cancel.clone();
                UnsyncCallback::new(move |request: DialogCloseRequest| {
                    // Unchanged: cancelling clears the password inputs and any
                    // revealed secret before the parent hides the dialog.
                    if request.action == DialogCloseAction::Dismiss {
                        handle_cancel();
                    }
                })
            }
            actions={
                let handle_cancel = handle_cancel.clone();
                move || view! {
                    <button
                        type="button"
                        class="nip49-modal-cancel"
                        disabled=move || exporting.get()
                        on:click={
                            let handle_cancel = handle_cancel.clone();
                            move |_| handle_cancel()
                        }
                    >"Close"</button>
                    <Show when=move || export_result.get().is_none()>
                        <button
                            type="button"
                            class="nip49-modal-export"
                            on:click=move |event| export_click_stored.get_value().run(event)
                            disabled=move || exporting.get()
                        >
                            {move || if exporting.get() { "Encrypting..." } else { "Create encrypted export" }}
                        </button>
                    </Show>
                }
            }
        >
            <p class="nip49-modal-npub">{move || format!("Account: {}", npub.get())}</p>

            <Show when=move || export_result.get().is_none()>
                <div class="nip49-modal-field">
                    <label for="nip49-password">"New export password"</label>
                    <input
                        node_ref=password_ref
                        id="nip49-password"
                        type="password"
                        bind:value=password
                        autocomplete="new-password"
                        placeholder="At least 8 characters"
                    />
                </div>
                <div class="nip49-modal-field">
                    <label for="nip49-password-confirm">"Confirm export password"</label>
                    <input
                        id="nip49-password-confirm"
                        type="password"
                        bind:value=confirm_password
                        autocomplete="new-password"
                        placeholder="Repeat password"
                    />
                </div>
            </Show>

            <Show when=move || validation_error.get().is_some()>
                <p class="arc-dialog-alert nip49-modal-error" role="alert">
                    {move || validation_error.get().unwrap_or_default()}
                </p>
            </Show>

            <Show when=move || export_result.get().is_some()>
                {move || export_result.get().map(|result| {
                    let has_secret = !result.ncryptsec.trim().is_empty();
                    view! {
                        <div class="nip49-modal-result">
                            <strong>{export_result_label(&result)}</strong>
                            <p class="nip49-modal-result-message">{result.message}</p>
                            {if has_secret {
                                view! {
                                    <div>
                                        {move || if visibility.get() == SecretVisibility::Hidden {
                                            view! {
                                                <button type="button" class="nip49-modal-export" on:click=move |_| visibility.set(SecretVisibility::Revealed)>
                                                    "Reveal sensitive encrypted key"
                                                </button>
                                            }.into_any()
                                        } else {
                                            view! {
                                                <div>
                                                    <label for="nip49-export-value">"Sensitive encrypted key"</label>
                                                    <textarea
                                                        id="nip49-export-value"
                                                        readonly
                                                        class="nip49-modal-result-text"
                                                        prop:value=result.ncryptsec.clone()
                                                        rows="4"
                                                    />
                                                    {move || if copy_confirmation.get() {
                                                        view! {
                                                            <div class="nip49-modal-copy-confirm">
                                                                <p>"Copy this sensitive value to the system clipboard?"</p>
                                                                <button type="button" class="nip49-modal-copy" on:click=move |event| confirm_copy_stored.get_value().run(event)>"Confirm copy"</button>
                                                                <button type="button" class="nip49-modal-cancel" on:click=move |_| copy_confirmation.set(false)>"Cancel"</button>
                                                            </div>
                                                        }.into_any()
                                                    } else {
                                                        view! {
                                                            <button type="button" class="nip49-modal-copy" on:click=move |_| copy_confirmation.set(true)>
                                                                "Copy encrypted key"
                                                            </button>
                                                        }.into_any()
                                                    }}
                                                </div>
                                            }.into_any()
                                        }}
                                    </div>
                                }.into_any()
                            } else {
                                view! { <p class="v2-social-meta">"No secret value was returned for this account."</p> }.into_any()
                            }}
                        </div>
                    }
                })}
            </Show>

            <Show when=move || copy_status.get().is_some()>
                <p class="arc-dialog-status nip49-modal-copy-status" role="status">{move || copy_status.get().unwrap_or_default()}</p>
            </Show>
        </Dialog>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn export_password_requires_length_and_match() {
        assert!(validate_export_password("short", "short").is_err());
        assert!(validate_export_password("long enough", "different").is_err());
        assert!(validate_export_password("long enough", "long enough").is_ok());
    }

    #[test]
    fn deferred_export_is_presented_truthfully() {
        let result = Nip49ExportResult {
            npub: "npub1test".to_string(),
            ncryptsec: String::new(),
            deferred: true,
            message: "Deferred".to_string(),
        };

        assert_eq!(export_result_label(&result), "Export deferred");
    }

    #[test]
    fn secret_visibility_defaults_to_hidden() {
        assert_eq!(SecretVisibility::default(), SecretVisibility::Hidden);
    }

    #[test]
    fn a_busy_export_cannot_be_dismissed_by_any_channel() {
        use crate::ui_v2::components::{resolve_close, DialogCloseAction, DialogCloseSource};

        let (policy, dismissal) = export_close_contract();
        for source in [
            DialogCloseSource::Escape,
            DialogCloseSource::Backdrop,
            DialogCloseSource::CloseButton,
            DialogCloseSource::Cancel,
        ] {
            assert_eq!(
                resolve_close(policy, dismissal, true, source),
                DialogCloseAction::Ignore,
                "{source:?} must not dismiss a running export"
            );
        }
    }

    #[test]
    fn an_idle_export_dialog_dismisses_normally() {
        use crate::ui_v2::components::{resolve_close, DialogCloseAction, DialogCloseSource};

        let (policy, dismissal) = export_close_contract();
        assert_eq!(
            resolve_close(policy, dismissal, false, DialogCloseSource::Escape),
            DialogCloseAction::Dismiss
        );
    }

    #[test]
    fn the_export_dialog_never_claims_an_unsupported_cancellation() {
        use crate::ui_v2::components::{resolve_close, DialogCloseAction, DialogCloseSource};

        let (policy, dismissal) = export_close_contract();
        // No confirmation prompt is offered, because there is nothing to
        // cancel: the backend export cannot be interrupted.
        assert_ne!(
            resolve_close(policy, dismissal, true, DialogCloseSource::Escape),
            DialogCloseAction::RequestConfirmation
        );
        let source = include_str!("nip49_modal.rs")
            .split("#[cfg(test)]")
            .next()
            .expect("production source precedes the test module");
        assert!(!source.contains(concat!("invoke_", "cancel")));
    }

    #[test]
    fn cancellation_cleanup_clears_passwords_and_secret_visibility() {
        let (password, confirmation, visibility) = cleared_secret_state();
        assert!(password.is_empty() && confirmation.is_empty());
        assert_eq!(visibility, SecretVisibility::Hidden);
    }
}
