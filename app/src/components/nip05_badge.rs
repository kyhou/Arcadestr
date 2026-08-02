use leptos::prelude::*;

use crate::models::Nip05Status;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Nip05PresentationState {
    Absent,
    Pending,
    Verified,
    Unverified,
    Invalid,
    Failed,
}

fn presentation_state(status: &Nip05Status) -> Nip05PresentationState {
    if status.identifier.trim().is_empty() {
        Nip05PresentationState::Absent
    } else if status.status.eq_ignore_ascii_case("verifying") {
        Nip05PresentationState::Pending
    } else if status.verified && status.status.eq_ignore_ascii_case("verified") {
        Nip05PresentationState::Verified
    } else if status.status.eq_ignore_ascii_case("failed") {
        Nip05PresentationState::Failed
    } else if status.status.eq_ignore_ascii_case("invalid")
        || status.normalized_identifier.trim().is_empty()
    {
        Nip05PresentationState::Invalid
    } else {
        Nip05PresentationState::Unverified
    }
}

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

    let current_state = move || presentation_state(&status.get());

    let badge_class = move || match current_state() {
        Nip05PresentationState::Verified => "nip05-badge nip05-badge-verified",
        Nip05PresentationState::Pending => "nip05-badge nip05-badge-verifying",
        Nip05PresentationState::Failed | Nip05PresentationState::Invalid => {
            "nip05-badge nip05-badge-failed"
        }
        Nip05PresentationState::Absent | Nip05PresentationState::Unverified => {
            "nip05-badge nip05-badge-unverified"
        }
    };

    let status_text = move || match current_state() {
        Nip05PresentationState::Absent => "No identifier",
        Nip05PresentationState::Pending => "Verification pending",
        Nip05PresentationState::Verified => "NIP-05 verified",
        Nip05PresentationState::Unverified => "Not verified",
        Nip05PresentationState::Invalid => "Invalid identifier",
        Nip05PresentationState::Failed => "Lookup failed",
    };

    let can_verify = move || {
        matches!(
            current_state(),
            Nip05PresentationState::Unverified | Nip05PresentationState::Failed
        ) && !identifier().is_empty()
    };
    let message_text = move || status.get().message;
    let has_status_message = move || !message_text().is_empty();

    view! {
        <div class="nip05-badge-container">
            <span class=badge_class role="status" aria-live="polite">
                {move || match current_state() {
                    Nip05PresentationState::Verified => "✓",
                    Nip05PresentationState::Failed | Nip05PresentationState::Invalid => "!",
                    Nip05PresentationState::Pending => "…",
                    Nip05PresentationState::Absent | Nip05PresentationState::Unverified => "?",
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

            {move || if can_verify() {
                view! {
                    <button
                        type="button"
                        class="v2-btn-secondary nip05-badge-verify-btn"
                        on:click={
                            let on_verify = on_verify.clone();
                            move |_| on_verify.run(identifier())
                        }
                    >
                        {move || if current_state() == Nip05PresentationState::Failed { "Retry verification" } else { "Verify NIP-05" }}
                    </button>
                }
                    .into_any()
            } else {
                view! { <></> }.into_any()
            }}
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn status(identifier: &str, normalized: &str, state: &str, verified: bool) -> Nip05Status {
        Nip05Status {
            identifier: identifier.into(),
            normalized_identifier: normalized.into(),
            local_part: String::new(),
            domain: String::new(),
            verified,
            status: state.into(),
            message: String::new(),
        }
    }

    #[test]
    fn all_nip05_states_remain_distinct() {
        assert_eq!(
            presentation_state(&status("", "", "unverified", false)),
            Nip05PresentationState::Absent
        );
        assert_eq!(
            presentation_state(&status("a@b.test", "a@b.test", "verifying", false)),
            Nip05PresentationState::Pending
        );
        assert_eq!(
            presentation_state(&status("a@b.test", "a@b.test", "verified", true)),
            Nip05PresentationState::Verified
        );
        assert_eq!(
            presentation_state(&status("a@b.test", "a@b.test", "verified", false)),
            Nip05PresentationState::Unverified
        );
        assert_eq!(
            presentation_state(&status("a@b.test", "a@b.test", "unverified", false)),
            Nip05PresentationState::Unverified
        );
        assert_eq!(
            presentation_state(&status("not-an-id", "", "unverified", false)),
            Nip05PresentationState::Invalid
        );
        assert_eq!(
            presentation_state(&status("a@b.test", "a@b.test", "failed", false)),
            Nip05PresentationState::Failed
        );
    }
}
