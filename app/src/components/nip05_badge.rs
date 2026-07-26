use leptos::prelude::*;

use crate::models::Nip05Status;

/// Renders NIP-05 verification state for the current account context.
#[component]
pub fn Nip05Badge(status: Signal<Nip05Status>, on_verify: Callback<String>) -> impl IntoView {
    let identifier = move || {
        let current = status.get();
        if current.normalized_identifier.is_empty() {
            current.identifier
        } else {
            current.normalized_identifier
        }
    };

    let is_verifying = move || status.get().status.eq_ignore_ascii_case("verifying");
    let is_verified = move || {
        let current = status.get();
        current.verified || current.status.eq_ignore_ascii_case("verified")
    };
    let is_failed = move || status.get().status.eq_ignore_ascii_case("failed");
    let is_unverified = move || !is_verified() && !is_verifying() && !is_failed();

    let badge_class = move || {
        if is_verified() {
            "nip05-badge nip05-badge-verified"
        } else if is_verifying() {
            "nip05-badge nip05-badge-verifying"
        } else if is_failed() {
            "nip05-badge nip05-badge-failed"
        } else {
            "nip05-badge nip05-badge-unverified"
        }
    };

    let status_text = move || {
        if is_verified() {
            "Verified"
        } else if is_verifying() {
            "Verifying..."
        } else if is_failed() {
            "Verification failed"
        } else {
            "Not verified"
        }
    };

    let can_verify = move || !is_verifying() && !identifier().is_empty();
    let message_text = move || status.get().message;
    let has_status_message = move || !message_text().is_empty();

    view! {
        <div class="nip05-badge-container">
            <span class=badge_class>
                {move || if is_verified() {
                    "✓"
                } else if is_failed() {
                    "!"
                } else if is_verifying() {
                    "…"
                } else {
                    "?"
                }}
                " "
                {move || if identifier().is_empty() {
                    "No NIP-05 set".to_string()
                } else {
                    identifier()
                }}
                " — "
                {status_text}
            </span>

            {move || if has_status_message() {
                    view! { <p class="nip05-badge-message">{message_text()}</p> }.into_any()
                } else {
                    view! { <></> }.into_any()
                }}

            {move || if is_unverified() || is_failed() {
                view! {
                    <button
                        type="button"
                        class="v2-btn-secondary nip05-badge-verify-btn"
                        disabled=move || !can_verify()
                        on:click={
                            let on_verify = on_verify.clone();
                            move |_| on_verify.run(identifier())
                        }
                    >
                        "Verify"
                    </button>
                }
                    .into_any()
            } else {
                view! { <></> }.into_any()
            }}
        </div>
    }
}
