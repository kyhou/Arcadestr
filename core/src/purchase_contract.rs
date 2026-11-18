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
    #[default]
    ProviderCeiling,
    /// Application accounting only. The external wallet may exceed the desired
    /// maximum before reporting fees. Never eligible for a hard-ceiling claim.
    ReportedFees,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PurchaseLimits {
    pub principal_msat: u64,
    pub fee_reserve_msat: u64,
    pub total_msat: u64,
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
    pub fee_reserve_msat: u64,
    pub total_msat: u64,
    pub confirm_complete_plan: bool,
    #[serde(default)]
    pub fee_policy: RoutingFeePolicy,
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
