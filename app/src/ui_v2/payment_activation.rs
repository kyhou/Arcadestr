//! Payment activation as compiled into the desktop backend, which is the only
//! component that dispatches payments. The UI never infers activation from its
//! own (WASM) build; until the backend answers, payments are shown as locked.
use arcadestr_core::payment_boundary::{DispatchActivation, PaymentActivationStatus};
use leptos::prelude::*;

#[derive(Clone, Copy)]
pub struct PaymentActivation(pub RwSignal<Option<PaymentActivationStatus>>);

impl PaymentActivation {
    pub fn buyer_enabled(self) -> bool {
        self.0.with(|s| buyer_enabled(s.as_ref()))
    }
    pub fn buyer_label(self) -> &'static str {
        self.0
            .with(|s| activation_label(s.as_ref().map(|s| s.buyer_purchase)))
    }
    pub fn refund_label(self) -> &'static str {
        self.0
            .with(|s| activation_label(s.as_ref().map(|s| s.recipient_refund)))
    }
    pub fn reported_fees_allowed(self) -> bool {
        self.0
            .with(|s| s.as_ref().is_some_and(|s| s.reported_fees_allowed))
    }
    pub fn refund_enabled(self) -> bool {
        self.0.with(|s| refund_enabled(s.as_ref()))
    }
    pub fn refund_lock_reason(self) -> Option<String> {
        self.0.with(|s| refund_lock_reason(s.as_ref()))
    }
    pub fn test_build_label(self) -> Option<String> {
        self.0
            .with(|s| s.as_ref().and_then(|s| s.test_build_label.clone()))
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

/// Shared activation state; created and loaded once on first use.
pub fn use_payment_activation() -> PaymentActivation {
    if let Some(existing) = use_context::<PaymentActivation>() {
        return existing;
    }
    let state = PaymentActivation(RwSignal::new(None));
    provide_context(state);
    #[cfg(not(feature = "web"))]
    wasm_bindgen_futures::spawn_local(async move {
        if let Ok(status) = crate::tauri_invoke::invoke::<PaymentActivationStatus>(
            "get_payment_activation",
            serde_json::json!({}),
        )
        .await
        {
            state.0.set(Some(status));
        }
    });
    #[cfg(feature = "web")]
    state
        .0
        .set(Some(arcadestr_core::payment_boundary::activation_status()));
    state
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
}
