//! Portable purchase IPC/presentation contract. No credentials or preimages.
use crate::adp_settlement::SignedQuote;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum InvoiceNetwork {
    Bitcoin,
    Testnet,
    Signet,
    Regtest,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RoutingFeePolicy {
    /// Strict: dispatch requires a wallet-enforced fee ceiling for the exact
    /// invoice, within the approved numerical fee reserve.
    #[default]
    ProviderCeiling,
    /// Legacy reserve accounting, retired for new approvals. Retained so that
    /// persisted approvals keep their original reserve and recovery semantics.
    /// The external wallet may exceed the reserve before reporting fees.
    ReportedFees,
    /// Standard-NWC warning-only policy. No fee estimate, reserve or maximum
    /// total exists: the wallet chooses routing fees, reported fees are
    /// recorded, and missing reports stay unknown. Requires the buyer's
    /// explicit acknowledgement of [`UNKNOWN_ROUTING_FEES_NOTICE`].
    UnknownFeesAcknowledged,
}

impl RoutingFeePolicy {
    /// Whether the approval carries a numerical fee reserve and total budget.
    pub const fn uses_fee_reserve(self) -> bool {
        !matches!(self, Self::UnknownFeesAcknowledged)
    }
}

/// Stable identifier of the routing-fee warning a buyer acknowledged. It is
/// stored in the immutable approval; changing the text requires a new version.
pub const UNKNOWN_ROUTING_FEES_NOTICE: &str = "arcadestr/unknown-lightning-routing-fees/v1";
pub const UNKNOWN_ROUTING_FEES_WARNING: &str = "Your Lightning wallet may charge additional network fees. Their amount is not known before payment, so your final cost may exceed the game price. Network fees may not be recoverable if a refund is necessary.";
pub const UNKNOWN_ROUTING_FEES_ACKNOWLEDGEMENT: &str =
    "I understand that additional Lightning fees may apply.";

/// Approved spending limits. `fee_reserve_msat` and `total_msat` exist only for
/// reserve-based policies; under `UnknownFeesAcknowledged` both are `None` and
/// no maximum wallet debit is implied. `Some(v)` serializes exactly as the
/// former plain integer, so persisted approval snapshots remain byte-stable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PurchaseLimits {
    pub principal_msat: u64,
    #[serde(default)]
    pub fee_reserve_msat: Option<u64>,
    #[serde(default)]
    pub total_msat: Option<u64>,
    pub maximum_dispatches: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WalletCapabilities {
    pub methods: Vec<String>,
    pub network: Option<String>,
}
impl WalletCapabilities {
    pub fn supports(&self, method: &str) -> bool {
        self.methods.iter().any(|v| v == method)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LegProgress {
    Pending,
    Uncertain,
    Paid,
    ProofAccepted,
    FailedNotSent,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PurchaseLegStatus {
    pub leg_id: String,
    pub progress: LegProgress,
    pub attempt_id: Option<String>,
    pub payment_hash: Option<String>,
    pub invoice_payee: Option<String>,
    pub fees_paid_msat: Option<u64>,
    pub can_reconcile: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PurchasePlanReview {
    pub quote: SignedQuote,
    pub quote_event_id: String,
    pub commitment: String,
    pub network: Option<InvoiceNetwork>,
    pub terms_verified: bool,
    pub approved: bool,
    pub approved_limits: Option<PurchaseLimits>,
    #[serde(default)]
    pub approved_fee_policy: Option<RoutingFeePolicy>,
    /// Routing-fee notice the buyer acknowledged in the immutable approval.
    #[serde(default)]
    pub approved_fee_notice: Option<String>,
    pub halted: bool,
    pub purchase_completed: bool,
    pub settlement_state: String,
    pub wallet_connected: bool,
    pub wallet_capabilities: Option<WalletCapabilities>,
    pub legs: Vec<PurchaseLegStatus>,
    pub restriction: String,
}

impl PurchasePlanReview {
    /// Presentation eligibility only: never replaces executor admission checks,
    /// provider fee assurance or separate authorization for live obligations.
    pub fn dispatch_ready(&self, now: u64) -> bool {
        self.approved
            && self.terms_verified
            && self.wallet_connected
            && !self.halted
            && !self.purchase_completed
            && self.settlement_state == "active"
            && now < self.quote.payment_expires_at
            && self
                .wallet_capabilities
                .as_ref()
                .is_none_or(|c| c.supports("pay_invoice"))
            && self.legs.len() == self.quote.legs.len()
            && self.legs.iter().any(|l| l.progress == LegProgress::Pending)
            && self.legs.iter().all(|l| {
                matches!(
                    l.progress,
                    LegProgress::Pending | LegProgress::ProofAccepted
                )
            })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PurchasePlanRequest {
    pub expected_account_pubkey: String,
    pub quote_id: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApprovePurchasePlanRequest {
    pub expected_account_pubkey: String,
    pub quote_id: String,
    pub quote_event_id: String,
    pub commitment: String,
    pub network: InvoiceNetwork,
    /// Only for reserve-based policies; must be absent for the warning-only policy.
    #[serde(default)]
    pub fee_reserve_msat: Option<u64>,
    #[serde(default)]
    pub total_msat: Option<u64>,
    pub confirm_complete_plan: bool,
    #[serde(default)]
    pub fee_policy: RoutingFeePolicy,
    /// Explicit buyer acknowledgement that routing fees are not wallet-capped.
    /// Never defaulted to true; required for every policy except the strict one.
    #[serde(default)]
    pub accept_unenforced_fee_risk: bool,
}

/// Buyer-purchase activation from the release payment boundary. Quote creation,
/// invoice disclosure and dispatch all require it. No environment flag,
/// historical budget file or UI checkbox overrides it.
pub fn require_live_purchase_authorization() -> Result<(), String> {
    crate::payment_boundary::require_release_activation(
        crate::payment_boundary::PaymentPurpose::BuyerPurchase,
    )
    .map_err(|refusal| refusal.to_string())
}

/// Recipient-refund activation, separate from buyer purchases.
pub fn require_live_refund_authorization() -> Result<(), String> {
    crate::payment_boundary::require_release_activation(
        crate::payment_boundary::PaymentPurpose::RecipientRefund,
    )
    .map_err(|refusal| refusal.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[cfg(not(any(
        feature = "regtest-buyer-payments",
        feature = "production-buyer-payments"
    )))]
    fn approval_does_not_authorize_live_execution() {
        assert!(require_live_purchase_authorization().is_err());
    }
    #[test]
    fn capability_support_is_exact_and_optional() {
        let c = WalletCapabilities {
            methods: vec!["pay_invoice".into()],
            network: None,
        };
        assert!(c.supports("pay_invoice"));
        assert!(!c.supports("multi_pay_invoice"));
        assert!(!c.supports("lookup_invoice"));
    }
}
