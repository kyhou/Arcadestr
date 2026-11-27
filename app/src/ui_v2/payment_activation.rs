//! Payment activation as compiled into the desktop backend, which is the only
//! component that dispatches payments. The UI never infers activation from its
//! own (WASM) build; until the backend answers, payments are shown as locked.
use arcadestr_core::payment_boundary::{DispatchActivation, PaymentActivationStatus};
use leptos::prelude::*;

#[derive(Clone, Copy, Default)]
pub struct PaymentActivation(Option<RwSignal<Option<PaymentActivationStatus>>>);

impl PaymentActivation {
    fn with_status<T>(self, f: impl FnOnce(Option<&PaymentActivationStatus>) -> T) -> T {
        let status = self.0.and_then(|state| state.try_read());
        f(status.as_deref().and_then(Option::as_ref))
    }

    pub fn buyer_enabled(self) -> bool {
        self.with_status(buyer_enabled)
    }
    pub fn buyer_label(self) -> &'static str {
        self.with_status(|s| activation_label(s.map(|s| s.buyer_purchase)))
    }
    pub fn refund_label(self) -> &'static str {
        self.with_status(|s| activation_label(s.map(|s| s.recipient_refund)))
    }
    pub fn reported_fees_allowed(self) -> bool {
        self.with_status(|s| s.is_some_and(|s| s.reported_fees_allowed))
    }
    pub fn refund_enabled(self) -> bool {
        self.with_status(refund_enabled)
    }
    pub fn refund_lock_reason(self) -> Option<String> {
        self.with_status(refund_lock_reason)
    }
    pub fn test_build_label(self) -> Option<String> {
        self.with_status(|s| s.and_then(|s| s.test_build_label.clone()))
    }
}

pub fn buyer_enabled(status: Option<&PaymentActivationStatus>) -> bool {
    status.is_some_and(|s| s.buyer_purchase != DispatchActivation::Disabled)
}
fn activation_label(status: Option<DispatchActivation>) -> &'static str {
    match status {
        Some(DispatchActivation::Enabled) => "BITCOIN MAINNET ENABLED",
        Some(DispatchActivation::RegtestOnly) => "REGTEST ONLY — TEST BUILD",
        Some(DispatchActivation::Disabled) => "DISABLED IN THIS BUILD",
        None => "BUILD POLICY UNAVAILABLE — PAYMENT DISABLED",
    }
}
pub fn refund_enabled(status: Option<&PaymentActivationStatus>) -> bool {
    status.is_some_and(|s| s.recipient_refund != DispatchActivation::Disabled)
}
pub fn refund_lock_reason(status: Option<&PaymentActivationStatus>) -> Option<String> {
    match status {
        None => Some("Checking whether refund payments are enabled in this build…".into()),
        Some(s) => s.refund_lock_reason.clone(),
    }
}

/// Install at a stable owner above the route renderer. A route owner's context
/// can survive a rerender even after its arena signals have been disposed.
pub(crate) fn provide_payment_activation() {
    let state = RwSignal::new(None);
    provide_context(PaymentActivation(Some(state)));
    #[cfg(all(target_arch = "wasm32", not(feature = "web")))]
    wasm_bindgen_futures::spawn_local(async move {
        if let Ok(status) = crate::tauri_invoke::invoke::<PaymentActivationStatus>(
            "get_payment_activation",
            serde_json::json!({}),
        )
        .await
        {
            // The shell may have unmounted while the IPC request was pending.
            let _ = state.try_set(Some(status));
        }
    });
    #[cfg(feature = "web")]
    state.set(Some(arcadestr_core::payment_boundary::activation_status()));
}

pub fn use_payment_activation() -> PaymentActivation {
    use_context().unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn status(buyer: DispatchActivation, refund: DispatchActivation) -> PaymentActivationStatus {
        PaymentActivationStatus {
            buyer_purchase: buyer,
            recipient_refund: refund,
            test_build_label: None,
            buyer_lock_reason: None,
            refund_lock_reason: (refund == DispatchActivation::Disabled).then(|| "locked".into()),
            reported_fees_allowed: false,
        }
    }

    #[test]
    fn unknown_activation_is_locked_and_purposes_are_independent() {
        assert!(!buyer_enabled(None) && !refund_enabled(None));
        assert!(refund_lock_reason(None).is_some());
        let buyer_only = status(
            DispatchActivation::RegtestOnly,
            DispatchActivation::Disabled,
        );
        assert!(buyer_enabled(Some(&buyer_only)));
        assert!(!refund_enabled(Some(&buyer_only)));
        assert_eq!(
            refund_lock_reason(Some(&buyer_only)).as_deref(),
            Some("locked")
        );
        assert_eq!(
            activation_label(Some(DispatchActivation::Enabled)),
            "BITCOIN MAINNET ENABLED"
        );
        let none = status(DispatchActivation::Disabled, DispatchActivation::Disabled);
        assert!(!buyer_enabled(Some(&none)));
    }

    #[test]
    fn activation_survives_route_owner_cleanup_and_fails_closed_after_shell_cleanup() {
        let shell = Owner::new();
        shell.with(provide_payment_activation);
        let route = shell.child();

        let first = route.with(use_payment_activation);
        let signal = first.0.expect("route inherits shell activation");
        let mut active = status(
            DispatchActivation::RegtestOnly,
            DispatchActivation::Disabled,
        );
        active.test_build_label = Some("Test build".into());
        signal.set(Some(active));
        assert!(first.buyer_enabled());

        let reopened = route.with_cleanup(use_payment_activation);
        assert_eq!(reopened.test_build_label().as_deref(), Some("Test build"));
        assert!(reopened.buyer_enabled());
        assert!(!reopened.refund_enabled());

        shell.cleanup();
        assert!(!reopened.buyer_enabled());
        assert!(reopened.test_build_label().is_none());
    }
}
