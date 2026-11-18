//! Release payment-execution boundary.
//!
//! Every irreversible Lightning payment passes three independent layers:
//! 1. Activation: a compiled release decision for each payment purpose. It is
//!    deliberately not an environment variable, settings file or UI toggle.
//! 2. Purpose-specific authorization (buyer purchase or recipient refund),
//!    evaluated here from verified inputs immediately before dispatch.
//! 3. The durable executors (`purchase_execution`, `durable_payments`), which
//!    bind the approval snapshot, admit each leg once and never retry an
//!    uncertain payment.
//!
//! This module only decides; it never contacts a wallet, relay or server.
use crate::purchase_contract::{InvoiceNetwork, RoutingFeePolicy, WalletCapabilities};
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PaymentPurpose {
    BuyerPurchase,
    RecipientRefund,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DispatchActivation {
    Disabled,
    /// Debug test builds only, opted in per purpose at compile time. Every
    /// payment must be on Bitcoin regtest and within the hard test ceilings.
    RegtestOnly,
    /// Explicit production release activation, Bitcoin mainnet only.
    Enabled,
}

#[cfg(all(
    debug_assertions,
    any(
        feature = "production-buyer-payments",
        feature = "production-refund-payments"
    )
))]
compile_error!("production payment activation requires a non-debug build");

#[cfg(all(
    any(
        feature = "production-buyer-payments",
        feature = "production-refund-payments"
    ),
    any(
        feature = "regtest-buyer-payments",
        feature = "regtest-refund-payments"
    )
))]
compile_error!("production and regtest payment activation cannot be combined");

// Test activation can never ship in an optimized/release build.
#[cfg(all(
    not(debug_assertions),
    any(
        feature = "regtest-buyer-payments",
        feature = "regtest-refund-payments"
    )
))]
compile_error!("regtest payment test activation is only allowed in debug builds");

/// Compiled activation per purpose. Default builds are Disabled. A debug build
/// compiled with `regtest-buyer-payments` / `regtest-refund-payments` gets
/// independent RegtestOnly activation for that purpose; nothing else (env vars,
/// files, UI) can change it. Production activation is a reviewed release
/// build feature, selected separately for each purpose.
pub const fn release_activation(purpose: PaymentPurpose) -> DispatchActivation {
    let production = match purpose {
        PaymentPurpose::BuyerPurchase => cfg!(feature = "production-buyer-payments"),
        PaymentPurpose::RecipientRefund => cfg!(feature = "production-refund-payments"),
    };
    let regtest = match purpose {
        PaymentPurpose::BuyerPurchase => cfg!(feature = "regtest-buyer-payments"),
        PaymentPurpose::RecipientRefund => cfg!(feature = "regtest-refund-payments"),
    };
    if production {
        DispatchActivation::Enabled
    } else if regtest && cfg!(debug_assertions) {
        DispatchActivation::RegtestOnly
    } else {
        DispatchActivation::Disabled
    }
}

/// Hard ceilings of RegtestOnly activation, enforced in code (not documented
/// limits): one purchase total (principal + fee reserve), the cumulative total
/// admitted for one account in this database, and one refund payment.
pub const REGTEST_MAX_PURCHASE_TOTAL_MSAT: u64 = 10_000_000;
pub const REGTEST_MAX_ACCOUNT_TOTAL_MSAT: u64 = 50_000_000;
pub const REGTEST_MAX_REFUND_MSAT: u64 = 10_000_000;

/// Shown on every payment surface while a test activation is compiled in.
pub const REGTEST_TEST_BUILD_LABEL: &str =
    "REGTEST PAYMENT TEST BUILD - regtest only, not for release";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PaymentActivationStatus {
    pub buyer_purchase: DispatchActivation,
    pub recipient_refund: DispatchActivation,
    pub test_build_label: Option<String>,
    pub buyer_lock_reason: Option<String>,
    pub refund_lock_reason: Option<String>,
    #[serde(default)]
    pub reported_fees_allowed: bool,
}

pub fn activation_status() -> PaymentActivationStatus {
    let buyer = release_activation(PaymentPurpose::BuyerPurchase);
    let refund = release_activation(PaymentPurpose::RecipientRefund);
    let regtest = [buyer, refund].contains(&DispatchActivation::RegtestOnly);
    PaymentActivationStatus {
        buyer_purchase: buyer,
        recipient_refund: refund,
        test_build_label: regtest.then(|| REGTEST_TEST_BUILD_LABEL.to_owned()),
        buyer_lock_reason: require_activation(buyer, PaymentPurpose::BuyerPurchase)
            .err()
            .map(|e| e.to_string()),
        refund_lock_reason: require_activation(refund, PaymentPurpose::RecipientRefund)
            .err()
            .map(|e| e.to_string()),
        reported_fees_allowed: reported_fees_allowed(buyer),
    }
}

/// The compiled activation selects exactly one permissible payment network.
pub fn require_network(
    activation: DispatchActivation,
    network: InvoiceNetwork,
) -> Result<(), BoundaryRefusal> {
    match activation {
        DispatchActivation::Enabled if network == InvoiceNetwork::Bitcoin => Ok(()),
        DispatchActivation::Enabled => Err(BoundaryRefusal::MainnetOnly),
        DispatchActivation::RegtestOnly if network == InvoiceNetwork::Regtest => Ok(()),
        DispatchActivation::RegtestOnly => Err(BoundaryRefusal::TestNetworkOnly),
        DispatchActivation::Disabled => Err(BoundaryRefusal::Locked(
            "Payment dispatch is disabled in this build".into(),
        )),
    }
}

pub const fn reported_fees_allowed(activation: DispatchActivation) -> bool {
    match activation {
        DispatchActivation::Enabled => cfg!(feature = "production-reported-fees"),
        DispatchActivation::RegtestOnly => true,
        DispatchActivation::Disabled => false,
    }
}

pub fn require_fee_policy(
    activation: DispatchActivation,
    policy: RoutingFeePolicy,
    acknowledged: bool,
) -> Result<(), BoundaryRefusal> {
    if policy == RoutingFeePolicy::ReportedFees {
        if !reported_fees_allowed(activation) {
            return Err(BoundaryRefusal::ReportedFeesNotEnabled);
        }
        if !acknowledged {
            return Err(BoundaryRefusal::FeeRiskNotAcknowledged);
        }
    }
    Ok(())
}

/// RegtestOnly ceilings; other activations rely on the approval limits alone.
pub fn require_test_ceilings(
    activation: DispatchActivation,
    approved_total_msat: u64,
    account_after_msat: u64,
) -> Result<(), BoundaryRefusal> {
    if activation == DispatchActivation::RegtestOnly
        && (approved_total_msat > REGTEST_MAX_PURCHASE_TOTAL_MSAT
            || account_after_msat > REGTEST_MAX_ACCOUNT_TOTAL_MSAT)
    {
        return Err(BoundaryRefusal::TestCeilingExceeded);
    }
    Ok(())
}

/// NIP-47 methods required before any dispatch: `pay_invoice` to pay, and
/// `lookup_invoice` so an uncertain outcome can be reconciled without paying again.
pub const REQUIRED_WALLET_METHODS: [&str; 2] = ["pay_invoice", "lookup_invoice"];

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum BoundaryRefusal {
    #[error("{0}")]
    Locked(String),
    #[error("The active account changed; no payment was sent")]
    AccountChanged,
    #[error("The connected wallet differs from the approved wallet; review and approve again")]
    WalletChanged,
    #[error("The wallet does not offer required methods ({0}); no payment was sent")]
    WalletUnverified(String),
    #[error("The wallet reports a different Lightning network than the invoice")]
    WalletNetworkMismatch,
    #[error("The wallet did not report a Lightning network; no payment was sent")]
    WalletNetworkMissing,
    #[error("This production build pays only on Bitcoin mainnet; no payment was sent")]
    MainnetOnly,
    #[error("Reported-fee accounting is not enabled in this build")]
    ReportedFeesNotEnabled,
    #[error("This wallet cannot enforce a fee ceiling for this payment")]
    FeeCeilingUnavailable,
    #[error("Explicit approval of the exact purchase plan is required")]
    ApprovalRequired,
    #[error("This purchase is halted; review existing payments before continuing")]
    ApprovalHalted,
    #[error("The wallet cannot enforce a fee ceiling; acknowledge that risk explicitly")]
    FeeRiskNotAcknowledged,
    #[error("The listing could not be confirmed as the approved revision: {0}")]
    ListingNotCurrent(String),
    #[error("Explicit confirmation is required before dispatching a refund payment")]
    ConfirmationRequired,
    #[error("The active identity is not the obligated refund recipient")]
    RefundRecipientMismatch,
    #[error("This refund obligation is not payable in its current state ({0})")]
    RefundNotOwed(String),
    #[error("The refund invoice does not match the owed principal")]
    RefundAmountMismatch,
    #[error("A previous refund payment outcome is unresolved; reconcile it, never repay")]
    RefundOutcomeUncertain,
    #[error("This test build pays only on Bitcoin regtest; no payment was sent")]
    TestNetworkOnly,
    #[error("This test build's spending ceiling would be exceeded; no payment was sent")]
    TestCeilingExceeded,
}

pub fn require_activation(
    activation: DispatchActivation,
    purpose: PaymentPurpose,
) -> Result<(), BoundaryRefusal> {
    match activation {
        DispatchActivation::Enabled | DispatchActivation::RegtestOnly => Ok(()),
        DispatchActivation::Disabled => Err(BoundaryRefusal::Locked(match purpose {
            PaymentPurpose::BuyerPurchase => "Buyer payment dispatch is disabled in this build. Recovery remains available.".into(),
            PaymentPurpose::RecipientRefund => "Recipient refund dispatch is disabled in this build. Refund review and recovery remain available.".into(),
        })),
    }
}

/// The release activation for `purpose`.
pub fn require_release_activation(purpose: PaymentPurpose) -> Result<(), BoundaryRefusal> {
    require_activation(release_activation(purpose), purpose)
}

/// A wallet whose standard `get_info` response has been checked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedWallet {
    pub identity: String,
    network: InvoiceNetwork,
}

fn network_name(network: InvoiceNetwork) -> &'static [&'static str] {
    match network {
        InvoiceNetwork::Bitcoin => &["mainnet", "bitcoin"],
        InvoiceNetwork::Testnet => &["testnet", "testnet3", "testnet4"],
        InvoiceNetwork::Signet => &["signet"],
        InvoiceNetwork::Regtest => &["regtest"],
    }
}

/// `get_info` is the only standard capability source. An omitted or unknown
/// network cannot authorize either an approval or a payment.
pub fn verify_wallet(
    identity: &str,
    capabilities: WalletCapabilities,
    network: InvoiceNetwork,
) -> Result<VerifiedWallet, BoundaryRefusal> {
    if identity.is_empty() {
        return Err(BoundaryRefusal::WalletUnverified("wallet identity".into()));
    }
    let missing = REQUIRED_WALLET_METHODS
        .iter()
        .filter(|method| !capabilities.supports(method))
        .copied()
        .collect::<Vec<_>>();
    if !missing.is_empty() {
        return Err(BoundaryRefusal::WalletUnverified(missing.join(", ")));
    }
    let reported = capabilities
        .network
        .as_deref()
        .ok_or(BoundaryRefusal::WalletNetworkMissing)?
        .trim()
        .to_ascii_lowercase();
    if !network_name(network).contains(&reported.as_str()) {
        return Err(BoundaryRefusal::WalletNetworkMismatch);
    }
    Ok(VerifiedWallet {
        identity: identity.to_owned(),
        network,
    })
}

/// Outcome of the checkout listing-freshness policy, as seen by the boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ListingCheck {
    /// The approved revision is the newest signed revision under the policy.
    Current { listing_event_id: String },
    /// Anything else; the reason is shown and dispatch stops.
    NotCurrent(String),
}

pub struct BuyerDispatchCheck<'a> {
    pub initiating_account: &'a str,
    pub current_account: Option<&'a str>,
    pub approved_wallet_identity: Option<&'a str>,
    pub approval_halted: bool,
    pub wallet: &'a VerifiedWallet,
    pub fee_policy: RoutingFeePolicy,
    pub fee_risk_acknowledged: bool,
    pub wallet_fee_ceiling_available: bool,
    pub pinned_listing_event_id: &'a str,
    pub listing: &'a ListingCheck,
    pub invoice_network: InvoiceNetwork,
    /// Approved total for this purchase (principal + fee reserve).
    pub approved_total_msat: u64,
    /// Principal + fee bounds already admitted for this account, all purchases.
    pub account_admitted_msat: u64,
    /// Principal + fee bound of the leg about to be admitted.
    pub next_leg_msat: u64,
}

/// Buyer purchase authorization, evaluated before each leg's dispatch.
pub fn authorize_buyer_dispatch(
    activation: DispatchActivation,
    check: &BuyerDispatchCheck<'_>,
) -> Result<(), BoundaryRefusal> {
    require_activation(activation, PaymentPurpose::BuyerPurchase)?;
    require_network(activation, check.invoice_network)?;
    if check.wallet.network != check.invoice_network {
        return Err(BoundaryRefusal::WalletNetworkMismatch);
    }
    require_test_ceilings(
        activation,
        check.approved_total_msat,
        check
            .account_admitted_msat
            .saturating_add(check.next_leg_msat),
    )?;
    if check.current_account != Some(check.initiating_account) {
        return Err(BoundaryRefusal::AccountChanged);
    }
    let Some(approved_wallet) = check.approved_wallet_identity else {
        return Err(BoundaryRefusal::ApprovalRequired);
    };
    if approved_wallet != check.wallet.identity {
        return Err(BoundaryRefusal::WalletChanged);
    }
    if check.approval_halted {
        return Err(BoundaryRefusal::ApprovalHalted);
    }
    require_fee_policy(activation, check.fee_policy, check.fee_risk_acknowledged)?;
    if check.fee_policy == RoutingFeePolicy::ProviderCeiling && !check.wallet_fee_ceiling_available
    {
        return Err(BoundaryRefusal::FeeCeilingUnavailable);
    }
    match check.listing {
        ListingCheck::Current { listing_event_id }
            if listing_event_id == check.pinned_listing_event_id =>
        {
            Ok(())
        }
        ListingCheck::Current { .. } => Err(BoundaryRefusal::ListingNotCurrent(
            "a different listing revision is current".into(),
        )),
        ListingCheck::NotCurrent(reason) => Err(BoundaryRefusal::ListingNotCurrent(reason.clone())),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefundIntent {
    None,
    /// A dispatch was journaled but its outcome is not known.
    Uncertain,
}

pub struct RefundDispatchCheck<'a> {
    pub initiating_account: &'a str,
    pub current_account: Option<&'a str>,
    pub explicit_confirmation: bool,
    /// Recipient named by the processor's authenticated obligation view.
    pub obligation_recipient: &'a str,
    pub obligation_state: &'a str,
    pub obligation_principal_msat: u64,
    pub invoice_amount_msat: u64,
    pub prior_intent: RefundIntent,
    pub wallet: &'a VerifiedWallet,
    pub invoice_network: InvoiceNetwork,
}

/// Obligation states in which a recipient may still pay. Preparing the original
/// refund invoice moves `refund_owed`/`overdue` to `initiated` on the processor.
pub const PAYABLE_REFUND_STATES: [&str; 3] = ["refund_owed", "initiated", "overdue"];

/// Recipient refund authorization. Independent of buyer purchase approval.
pub fn authorize_refund_dispatch(
    activation: DispatchActivation,
    check: &RefundDispatchCheck<'_>,
) -> Result<(), BoundaryRefusal> {
    require_activation(activation, PaymentPurpose::RecipientRefund)?;
    require_network(activation, check.invoice_network)?;
    if check.wallet.network != check.invoice_network {
        return Err(BoundaryRefusal::WalletNetworkMismatch);
    }
    if activation == DispatchActivation::RegtestOnly
        && check.invoice_amount_msat > REGTEST_MAX_REFUND_MSAT
    {
        return Err(BoundaryRefusal::TestCeilingExceeded);
    }
    if !check.explicit_confirmation {
        return Err(BoundaryRefusal::ConfirmationRequired);
    }
    if check.current_account != Some(check.initiating_account) {
        return Err(BoundaryRefusal::AccountChanged);
    }
    if check.obligation_recipient != check.initiating_account {
        return Err(BoundaryRefusal::RefundRecipientMismatch);
    }
    if !PAYABLE_REFUND_STATES.contains(&check.obligation_state) {
        return Err(BoundaryRefusal::RefundNotOwed(
            check.obligation_state.into(),
        ));
    }
    if check.invoice_amount_msat == 0
        || check.invoice_amount_msat != check.obligation_principal_msat
    {
        return Err(BoundaryRefusal::RefundAmountMismatch);
    }
    if check.prior_intent == RefundIntent::Uncertain {
        return Err(BoundaryRefusal::RefundOutcomeUncertain);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn caps(methods: &[&str], network: Option<&str>) -> WalletCapabilities {
        WalletCapabilities {
            methods: methods.iter().map(|m| m.to_string()).collect(),
            network: network.map(str::to_owned),
        }
    }
    fn wallet() -> VerifiedWallet {
        verify_wallet(
            "wallet:client",
            caps(
                &["get_info", "pay_invoice", "lookup_invoice"],
                Some("regtest"),
            ),
            InvoiceNetwork::Regtest,
        )
        .expect("verified")
    }

    #[cfg(not(any(
        feature = "regtest-buyer-payments",
        feature = "regtest-refund-payments",
        feature = "production-buyer-payments",
        feature = "production-refund-payments"
    )))]
    #[test]
    fn default_builds_are_disabled_with_no_test_label() {
        let status = activation_status();
        assert_eq!(status.buyer_purchase, DispatchActivation::Disabled);
        assert_eq!(status.recipient_refund, DispatchActivation::Disabled);
        assert!(status.test_build_label.is_none());
        assert!(status.buyer_lock_reason.is_some() && status.refund_lock_reason.is_some());
    }

    #[cfg(all(
        feature = "regtest-buyer-payments",
        not(feature = "regtest-refund-payments")
    ))]
    #[test]
    fn buyer_test_activation_is_independent_and_labelled() {
        let status = activation_status();
        assert_eq!(status.buyer_purchase, DispatchActivation::RegtestOnly);
        assert_eq!(status.recipient_refund, DispatchActivation::Disabled);
        assert_eq!(
            status.test_build_label.as_deref(),
            Some(REGTEST_TEST_BUILD_LABEL)
        );
        assert!(status.buyer_lock_reason.is_none() && status.refund_lock_reason.is_some());
    }

    #[test]
    fn regtest_only_activation_never_pays_other_networks_or_beyond_ceilings() {
        let wallet = wallet();
        let current = ListingCheck::Current {
            listing_event_id: "listing-1".into(),
        };
        let a = DispatchActivation::RegtestOnly;
        assert_eq!(
            authorize_buyer_dispatch(a, &buyer(&wallet, &current)),
            Ok(())
        );
        for network in [
            InvoiceNetwork::Bitcoin,
            InvoiceNetwork::Testnet,
            InvoiceNetwork::Signet,
        ] {
            assert_eq!(
                require_network(a, network),
                Err(BoundaryRefusal::TestNetworkOnly)
            );
            let check = BuyerDispatchCheck {
                invoice_network: network,
                ..buyer(&wallet, &current)
            };
            assert_eq!(
                authorize_buyer_dispatch(a, &check),
                Err(BoundaryRefusal::TestNetworkOnly)
            );
            let refund = RefundDispatchCheck {
                invoice_network: network,
                ..refund(&wallet)
            };
            assert_eq!(
                authorize_refund_dispatch(a, &refund),
                Err(BoundaryRefusal::TestNetworkOnly)
            );
        }
        for check in [
            BuyerDispatchCheck {
                approved_total_msat: REGTEST_MAX_PURCHASE_TOTAL_MSAT + 1,
                ..buyer(&wallet, &current)
            },
            BuyerDispatchCheck {
                account_admitted_msat: REGTEST_MAX_ACCOUNT_TOTAL_MSAT - 999_999,
                ..buyer(&wallet, &current)
            },
        ] {
            assert_eq!(
                authorize_buyer_dispatch(a, &check),
                Err(BoundaryRefusal::TestCeilingExceeded)
            );
        }
        let at_limit = BuyerDispatchCheck {
            account_admitted_msat: REGTEST_MAX_ACCOUNT_TOTAL_MSAT - 1_000_000,
            ..buyer(&wallet, &current)
        };
        assert_eq!(authorize_buyer_dispatch(a, &at_limit), Ok(()));
        let big = RefundDispatchCheck {
            invoice_amount_msat: REGTEST_MAX_REFUND_MSAT + 1,
            obligation_principal_msat: REGTEST_MAX_REFUND_MSAT + 1,
            ..refund(&wallet)
        };
        assert_eq!(
            authorize_refund_dispatch(a, &big),
            Err(BoundaryRefusal::TestCeilingExceeded)
        );
        // Every other binding still applies under test activation.
        let other = BuyerDispatchCheck {
            current_account: Some("other"),
            ..buyer(&wallet, &current)
        };
        assert_eq!(
            authorize_buyer_dispatch(a, &other),
            Err(BoundaryRefusal::AccountChanged)
        );
        let stale = ListingCheck::NotCurrent("superseded".into());
        assert!(matches!(
            authorize_buyer_dispatch(a, &buyer(&wallet, &stale)),
            Err(BoundaryRefusal::ListingNotCurrent(_))
        ));
        assert_eq!(
            authorize_refund_dispatch(
                a,
                &RefundDispatchCheck {
                    explicit_confirmation: false,
                    ..refund(&wallet)
                }
            ),
            Err(BoundaryRefusal::ConfirmationRequired)
        );
        // Disabled stays disabled regardless of network.
        assert!(matches!(
            require_network(DispatchActivation::Disabled, InvoiceNetwork::Regtest),
            Err(BoundaryRefusal::Locked(_))
        ));
    }

    #[cfg(not(any(
        feature = "regtest-buyer-payments",
        feature = "regtest-refund-payments",
        feature = "production-buyer-payments",
        feature = "production-refund-payments"
    )))]
    #[test]
    fn release_activation_is_disabled_for_every_purpose() {
        for purpose in [
            PaymentPurpose::BuyerPurchase,
            PaymentPurpose::RecipientRefund,
        ] {
            assert_eq!(release_activation(purpose), DispatchActivation::Disabled);
            assert!(matches!(
                require_release_activation(purpose),
                Err(BoundaryRefusal::Locked(_))
            ));
        }
    }

    #[test]
    fn wallet_verification_requires_pay_and_lookup_and_matching_network() {
        assert!(matches!(
            verify_wallet("w", caps(&["pay_invoice"], None), InvoiceNetwork::Regtest),
            Err(BoundaryRefusal::WalletUnverified(m)) if m == "lookup_invoice"
        ));
        assert!(matches!(
            verify_wallet(
                "",
                caps(&REQUIRED_WALLET_METHODS, None),
                InvoiceNetwork::Regtest
            ),
            Err(BoundaryRefusal::WalletUnverified(_))
        ));
        assert_eq!(
            verify_wallet(
                "w",
                caps(&REQUIRED_WALLET_METHODS, Some("mainnet")),
                InvoiceNetwork::Regtest
            ),
            Err(BoundaryRefusal::WalletNetworkMismatch)
        );
        assert_eq!(
            verify_wallet(
                "w",
                caps(&REQUIRED_WALLET_METHODS, None),
                InvoiceNetwork::Regtest
            ),
            Err(BoundaryRefusal::WalletNetworkMissing)
        );
        assert!(verify_wallet(
            "w",
            caps(&REQUIRED_WALLET_METHODS, Some("Regtest")),
            InvoiceNetwork::Regtest
        )
        .is_ok());
    }

    #[test]
    fn compiled_policy_matches_independent_features() {
        let status = activation_status();
        assert_eq!(
            status.buyer_purchase == DispatchActivation::Enabled,
            cfg!(feature = "production-buyer-payments")
        );
        assert_eq!(
            status.recipient_refund == DispatchActivation::Enabled,
            cfg!(feature = "production-refund-payments")
        );
        assert_eq!(
            status.reported_fees_allowed,
            cfg!(any(
                feature = "regtest-buyer-payments",
                feature = "production-reported-fees"
            ))
        );
    }

    #[test]
    fn mainnet_and_fee_policy_fail_closed() {
        let activation = DispatchActivation::Enabled;
        for network in [
            InvoiceNetwork::Regtest,
            InvoiceNetwork::Testnet,
            InvoiceNetwork::Signet,
        ] {
            assert_eq!(
                require_network(activation, network),
                Err(BoundaryRefusal::MainnetOnly)
            );
            assert_eq!(
                verify_wallet(
                    "w",
                    caps(&REQUIRED_WALLET_METHODS, Some(network_name(network)[0])),
                    InvoiceNetwork::Bitcoin
                ),
                Err(BoundaryRefusal::WalletNetworkMismatch)
            );
        }
        assert_eq!(require_network(activation, InvoiceNetwork::Bitcoin), Ok(()));
        assert_eq!(
            verify_wallet(
                "w",
                caps(&REQUIRED_WALLET_METHODS, Some("unknown")),
                InvoiceNetwork::Bitcoin
            ),
            Err(BoundaryRefusal::WalletNetworkMismatch)
        );
        assert!(verify_wallet(
            "w",
            caps(&REQUIRED_WALLET_METHODS, Some("mainnet")),
            InvoiceNetwork::Bitcoin
        )
        .is_ok());
        assert_eq!(
            require_fee_policy(activation, RoutingFeePolicy::ReportedFees, false),
            Err(if cfg!(feature = "production-reported-fees") {
                BoundaryRefusal::FeeRiskNotAcknowledged
            } else {
                BoundaryRefusal::ReportedFeesNotEnabled
            })
        );
        assert_eq!(
            require_fee_policy(activation, RoutingFeePolicy::ReportedFees, true).is_ok(),
            cfg!(feature = "production-reported-fees")
        );
    }

    fn buyer<'a>(wallet: &'a VerifiedWallet, listing: &'a ListingCheck) -> BuyerDispatchCheck<'a> {
        BuyerDispatchCheck {
            initiating_account: "buyer",
            current_account: Some("buyer"),
            approved_wallet_identity: Some("wallet:client"),
            approval_halted: false,
            wallet,
            fee_policy: RoutingFeePolicy::ReportedFees,
            fee_risk_acknowledged: true,
            wallet_fee_ceiling_available: false,
            pinned_listing_event_id: "listing-1",
            listing,
            invoice_network: InvoiceNetwork::Regtest,
            approved_total_msat: 2_000_000,
            account_admitted_msat: 0,
            next_leg_msat: 1_000_000,
        }
    }

    #[test]
    fn buyer_dispatch_requires_every_binding() {
        let wallet = wallet();
        let current = ListingCheck::Current {
            listing_event_id: "listing-1".into(),
        };
        let ok = buyer(&wallet, &current);
        assert_eq!(
            authorize_buyer_dispatch(DispatchActivation::RegtestOnly, &ok),
            Ok(())
        );
        assert!(matches!(
            authorize_buyer_dispatch(DispatchActivation::Disabled, &ok),
            Err(BoundaryRefusal::Locked(_))
        ));
        let cases: Vec<(BuyerDispatchCheck, BoundaryRefusal)> = vec![
            (
                BuyerDispatchCheck {
                    current_account: Some("other"),
                    ..buyer(&wallet, &current)
                },
                BoundaryRefusal::AccountChanged,
            ),
            (
                BuyerDispatchCheck {
                    current_account: None,
                    ..buyer(&wallet, &current)
                },
                BoundaryRefusal::AccountChanged,
            ),
            (
                BuyerDispatchCheck {
                    approved_wallet_identity: None,
                    ..buyer(&wallet, &current)
                },
                BoundaryRefusal::ApprovalRequired,
            ),
            (
                BuyerDispatchCheck {
                    approved_wallet_identity: Some("other"),
                    ..buyer(&wallet, &current)
                },
                BoundaryRefusal::WalletChanged,
            ),
            (
                BuyerDispatchCheck {
                    approval_halted: true,
                    ..buyer(&wallet, &current)
                },
                BoundaryRefusal::ApprovalHalted,
            ),
            (
                BuyerDispatchCheck {
                    fee_risk_acknowledged: false,
                    ..buyer(&wallet, &current)
                },
                BoundaryRefusal::FeeRiskNotAcknowledged,
            ),
            (
                BuyerDispatchCheck {
                    fee_policy: RoutingFeePolicy::ProviderCeiling,
                    ..buyer(&wallet, &current)
                },
                BoundaryRefusal::FeeCeilingUnavailable,
            ),
        ];
        for (check, refusal) in cases {
            assert_eq!(
                authorize_buyer_dispatch(DispatchActivation::RegtestOnly, &check),
                Err(refusal)
            );
        }
        let newer = ListingCheck::Current {
            listing_event_id: "listing-2".into(),
        };
        assert!(matches!(
            authorize_buyer_dispatch(DispatchActivation::RegtestOnly, &buyer(&wallet, &newer)),
            Err(BoundaryRefusal::ListingNotCurrent(_))
        ));
        let insufficient = ListingCheck::NotCurrent("insufficient relay evidence".into());
        assert_eq!(
            authorize_buyer_dispatch(
                DispatchActivation::RegtestOnly,
                &buyer(&wallet, &insufficient)
            ),
            Err(BoundaryRefusal::ListingNotCurrent(
                "insufficient relay evidence".into()
            ))
        );
        let provider = BuyerDispatchCheck {
            fee_policy: RoutingFeePolicy::ProviderCeiling,
            wallet_fee_ceiling_available: true,
            fee_risk_acknowledged: false,
            ..buyer(&wallet, &current)
        };
        assert_eq!(
            authorize_buyer_dispatch(DispatchActivation::RegtestOnly, &provider),
            Ok(())
        );
    }

    fn refund(wallet: &VerifiedWallet) -> RefundDispatchCheck<'_> {
        RefundDispatchCheck {
            initiating_account: "recipient",
            current_account: Some("recipient"),
            explicit_confirmation: true,
            obligation_recipient: "recipient",
            obligation_state: "refund_owed",
            obligation_principal_msat: 10_000,
            invoice_amount_msat: 10_000,
            prior_intent: RefundIntent::None,
            wallet,
            invoice_network: InvoiceNetwork::Regtest,
        }
    }

    #[test]
    fn refund_dispatch_has_its_own_rules() {
        let wallet = wallet();
        assert_eq!(
            authorize_refund_dispatch(DispatchActivation::RegtestOnly, &refund(&wallet)),
            Ok(())
        );
        assert!(matches!(
            authorize_refund_dispatch(DispatchActivation::Disabled, &refund(&wallet)),
            Err(BoundaryRefusal::Locked(m)) if m.contains("Refund")
        ));
        for payable in PAYABLE_REFUND_STATES {
            let check = RefundDispatchCheck {
                obligation_state: payable,
                ..refund(&wallet)
            };
            assert_eq!(
                authorize_refund_dispatch(DispatchActivation::RegtestOnly, &check),
                Ok(())
            );
        }
        let cases: Vec<(RefundDispatchCheck, BoundaryRefusal)> = vec![
            (
                RefundDispatchCheck {
                    explicit_confirmation: false,
                    ..refund(&wallet)
                },
                BoundaryRefusal::ConfirmationRequired,
            ),
            (
                RefundDispatchCheck {
                    current_account: Some("other"),
                    ..refund(&wallet)
                },
                BoundaryRefusal::AccountChanged,
            ),
            (
                RefundDispatchCheck {
                    obligation_recipient: "developer",
                    ..refund(&wallet)
                },
                BoundaryRefusal::RefundRecipientMismatch,
            ),
            (
                RefundDispatchCheck {
                    obligation_state: "verified",
                    ..refund(&wallet)
                },
                BoundaryRefusal::RefundNotOwed("verified".into()),
            ),
            (
                RefundDispatchCheck {
                    obligation_state: "disputed",
                    ..refund(&wallet)
                },
                BoundaryRefusal::RefundNotOwed("disputed".into()),
            ),
            (
                RefundDispatchCheck {
                    invoice_amount_msat: 10_001,
                    ..refund(&wallet)
                },
                BoundaryRefusal::RefundAmountMismatch,
            ),
            (
                RefundDispatchCheck {
                    invoice_amount_msat: 0,
                    obligation_principal_msat: 0,
                    ..refund(&wallet)
                },
                BoundaryRefusal::RefundAmountMismatch,
            ),
            (
                RefundDispatchCheck {
                    prior_intent: RefundIntent::Uncertain,
                    ..refund(&wallet)
                },
                BoundaryRefusal::RefundOutcomeUncertain,
            ),
        ];
        for (check, refusal) in cases {
            assert_eq!(
                authorize_refund_dispatch(DispatchActivation::RegtestOnly, &check),
                Err(refusal)
            );
        }
    }
}
/// Guard for the separately built Gate 8 partial-settlement desktop only.
/// The per-run pin is `quote UUID:payment hash` and is never set by IPC.
#[cfg(feature = "native")]
pub fn require_carol_only_dispatch(
    pin: Option<&str>,
    account: &str,
    quote: &str,
    leg: &str,
    invoice: &lightning_invoice::Bolt11Invoice,
) -> Result<(), String> {
    const ALICE: &str = "f3d701f3dd838212966180be3467b9a32076d4cf23bfe13a920ba54cb3352899";
    const CAROL_LEG: &str =
        "operator:f44b43a39d63bd8b04c4fcdddcc58abb92215348c74c2e62858a8f87b93d5927";
    const CAROL_NODE: &str = "031b3f9d917efe1b88977534cce4f1e4f05036632a5ada028299ea93214fcb019c";
    check_carol_only_dispatch(
        pin,
        account,
        quote,
        leg,
        invoice,
        (ALICE, CAROL_LEG, CAROL_NODE),
    )
}

#[cfg(feature = "native")]
fn check_carol_only_dispatch(
    pin: Option<&str>,
    account: &str,
    quote: &str,
    leg: &str,
    invoice: &lightning_invoice::Bolt11Invoice,
    (buyer, recipient, node): (&str, &str, &str),
) -> Result<(), String> {
    let bound = pin.and_then(|v| v.split_once(':')).is_some_and(|(q, h)| {
        uuid::Uuid::parse_str(q).is_ok() && q == quote && h == invoice.payment_hash().to_string()
    });
    if bound
        && account == buyer
        && leg == recipient
        && invoice.currency() == lightning_invoice::Currency::Regtest
        && invoice.amount_milli_satoshis() == Some(100_000)
        && invoice.recover_payee_pub_key().to_string() == node
    {
        Ok(())
    } else {
        Err("Carol-only regtest build: payment does not match the single pinned invoice".into())
    }
}

#[cfg(all(test, feature = "native"))]
#[test]
fn carol_only_dispatch_pin_accepts_one_exact_fixture_invoice() {
    use bitcoin::{
        hashes::{sha256, Hash},
        secp256k1::{Secp256k1, SecretKey},
    };
    use lightning_invoice::{Currency, InvoiceBuilder};
    use lightning_types::payment::PaymentSecret;
    let make = |network, sats, hash, key| {
        InvoiceBuilder::new(network)
            .description("isolated partial settlement".into())
            .payment_hash(sha256::Hash::hash(&[hash; 32]))
            .payment_secret(PaymentSecret([8; 32]))
            .amount_milli_satoshis(sats)
            .duration_since_epoch(std::time::Duration::from_secs(1000))
            .min_final_cltv_expiry_delta(18)
            .build_signed(|msg| {
                Secp256k1::new()
                    .sign_ecdsa_recoverable(msg, &SecretKey::from_slice(&[key; 32]).unwrap())
            })
            .unwrap()
    };
    let carol = make(Currency::Regtest, 100_000, 1, 1);
    let node = carol.recover_payee_pub_key().to_string();
    let quote = "8fc365c4-2e97-430a-9b7a-cacdba6fdc78";
    let pin = format!("{quote}:{}", carol.payment_hash());
    let check = |p: Option<&str>,
                 buyer: &str,
                 q: &str,
                 leg: &str,
                 invoice: &lightning_invoice::Bolt11Invoice| {
        check_carol_only_dispatch(
            p,
            buyer,
            q,
            leg,
            invoice,
            ("alice", "operator:carol", &node),
        )
    };
    assert!(check(Some(&pin), "alice", quote, "operator:carol", &carol).is_ok());
    assert!(check(None, "alice", quote, "operator:carol", &carol).is_err());
    assert!(check(Some("malformed"), "alice", quote, "operator:carol", &carol).is_err());
    assert!(check(Some(&pin), "alice", "other-quote", "operator:carol", &carol).is_err());
    assert!(check(Some(&pin), "bob", quote, "operator:carol", &carol).is_err());
    for leg in ["operator:dave", "developer"] {
        assert!(check(Some(&pin), "alice", quote, leg, &carol).is_err());
    }
    for other in [
        make(Currency::Regtest, 100_000, 2, 1),
        make(Currency::Regtest, 150_000, 1, 1),
        make(Currency::Regtest, 100_000, 1, 2),
        make(Currency::Bitcoin, 100_000, 1, 1),
    ] {
        assert!(check(Some(&pin), "alice", quote, "operator:carol", &other).is_err());
    }
}
