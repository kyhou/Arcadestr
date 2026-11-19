//! Arcadestr-owned immutable approval and purchase-wide dispatch fencing.
//! No provider-specific database, permission API or custom NWC method is used.
pub use crate::purchase_contract::{InvoiceNetwork, PurchaseLimits};
use crate::purchase_contract::{
    LegProgress, PurchaseLegStatus, PurchasePlanReview, RoutingFeePolicy,
    UNKNOWN_ROUTING_FEES_NOTICE,
};
use crate::{
    adp_client::SettlementInvoiceResponse,
    adp_settlement::{verify_quote_event, verify_quote_listing_terms, QuoteState, SignedQuote},
    durable_payments::{DurableNwcPayments, DurablePaymentError, PaymentWallet},
    nwc_client::PayInvoiceResult,
    settlement_repository::{SettlementRepository, SettlementRepositoryError},
    signers::NostrSigner,
};
use lightning_invoice::{Bolt11Invoice, Currency};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::{Row, SqlitePool};
use std::{collections::BTreeMap, str::FromStr};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ExecutionError {
    #[error("Explicit immutable purchase approval is required")]
    ApprovalRequired,
    #[error("Purchase, account, recipient or original invoice binding changed")]
    Binding,
    #[error("Purchase limit exceeded or purchase halted; review existing obligations")]
    Budget,
    #[error("Reconcile the original uncertain leg before any further dispatch")]
    Uncertain,
    #[error("Existing wallet capabilities do not establish the approved fee ceiling")]
    FeeBoundUnavailable,
    #[error(
        "Explicit acknowledgement that additional Lightning routing fees may apply is required"
    )]
    FeeRiskAcknowledgementRequired,
    #[error("Purchase storage unavailable")]
    Storage(#[from] sqlx::Error),
    #[error("Settlement storage unavailable")]
    Repository(#[from] SettlementRepositoryError),
    #[error(transparent)]
    Payment(#[from] DurablePaymentError),
}

impl InvoiceNetwork {
    fn currency(self) -> Currency {
        match self {
            Self::Bitcoin => Currency::Bitcoin,
            Self::Testnet => Currency::BitcoinTestnet,
            Self::Signet => Currency::Signet,
            Self::Regtest => Currency::Regtest,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct ApprovedPlan {
    quote: SignedQuote,
    quote_event_id: String,
    wallet_identity: String,
    limits: PurchaseLimits,
    network: InvoiceNetwork,
    // Lightning invoice signing keys are not Nostr recipient keys. They are
    // separate, explicitly reviewed plan inputs; never inferred from a name.
    invoice_payees: BTreeMap<String, String>,
    /// The buyer approves logical recipients/destinations, with invoice keys
    /// pinned on their original ADP disclosure under the current protocol.
    #[serde(default)]
    bind_original_disclosures: bool,
    #[serde(default)]
    fee_policy: RoutingFeePolicy,
    /// Routing-fee notice the buyer explicitly acknowledged. Absent from older
    /// snapshots (and omitted when absent) so they stay byte-identical.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    fee_risk_notice: Option<String>,
}

pub struct PurchaseExecutor {
    pool: SqlitePool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SupersedeOutcome {
    /// Nothing was dispatched; the local quote can never be paid.
    Superseded,
    /// At least one leg was admitted; further dispatch is halted for recovery.
    HaltedAfterDispatch,
    AlreadySettled,
}
impl PurchaseExecutor {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    async fn quote(
        &self,
        signer: &dyn NostrSigner,
        id: &str,
    ) -> Result<(SignedQuote, String), ExecutionError> {
        let buyer = signer
            .get_public_key()
            .await
            .map_err(|_| ExecutionError::Binding)?;
        let stored = SettlementRepository::new(self.pool.clone())
            .load(&buyer.to_hex(), id)
            .await?
            .ok_or(ExecutionError::Binding)?;
        let event: nostr::Event =
            serde_json::from_str(&stored.quote_event_json).map_err(|_| ExecutionError::Binding)?;
        let text = signer
            .nip44_decrypt(&event.pubkey, &event.content)
            .await
            .map_err(|_| ExecutionError::Binding)?;
        let quote = verify_quote_event(&event, &text).map_err(|_| ExecutionError::Binding)?;
        if quote.buyer_pubkey != buyer.to_hex()
            || quote.quote_id != id
            || quote.listing_event_id != stored.listing_event_id
        {
            return Err(ExecutionError::Binding);
        }
        Ok((quote, event.id.to_hex()))
    }

    /// Explicit buyer-approval boundary, not called by quote creation or payment.
    /// A UI must present these exact inputs before invoking this operation.
    pub async fn approve(
        &self,
        signer: &dyn NostrSigner,
        wallet: &dyn PaymentWallet,
        quote_id: &str,
        expected_quote_event_id: &str,
        limits: PurchaseLimits,
        network: InvoiceNetwork,
        invoice_payees: BTreeMap<String, String>,
    ) -> Result<(), ExecutionError> {
        self.approve_inner(
            signer,
            wallet,
            quote_id,
            expected_quote_event_id,
            limits,
            network,
            invoice_payees,
            false,
            RoutingFeePolicy::ProviderCeiling,
            None,
            None,
        )
        .await
    }

    async fn approve_inner(
        &self,
        signer: &dyn NostrSigner,
        wallet: &dyn PaymentWallet,
        quote_id: &str,
        expected_quote_event_id: &str,
        limits: PurchaseLimits,
        network: InvoiceNetwork,
        invoice_payees: BTreeMap<String, String>,
        bind_original_disclosures: bool,
        fee_policy: RoutingFeePolicy,
        fee_risk_notice: Option<&str>,
        guard: Option<&(dyn Fn() -> Result<(), String> + Send + Sync)>,
    ) -> Result<(), ExecutionError> {
        if fee_policy == RoutingFeePolicy::UnknownFeesAcknowledged
            && fee_risk_notice != Some(UNKNOWN_ROUTING_FEES_NOTICE)
        {
            return Err(ExecutionError::FeeRiskAcknowledgementRequired);
        }
        let (quote, event_id) = self.quote(signer, quote_id).await?;
        // Reserve policies need an exact, consistent numerical budget. The
        // warning-only policy must not carry an invented reserve or maximum.
        let fee_limits_valid = match (
            fee_policy.uses_fee_reserve(),
            limits.fee_reserve_msat,
            limits.total_msat,
        ) {
            (true, Some(reserve), Some(total)) => {
                limits
                    .principal_msat
                    .checked_add(reserve)
                    .is_some_and(|sum| sum <= total)
                    && reserve <= i64::MAX as u64
                    && total <= i64::MAX as u64
            }
            (false, None, None) => true,
            _ => false,
        };
        if event_id != expected_quote_event_id
            || quote.state != QuoteState::Active
            || quote.legs.is_empty()
            || limits.principal_msat != quote.gross_msat
            || limits.maximum_dispatches != quote.legs.len()
            || (!bind_original_disclosures && invoice_payees.len() != quote.legs.len())
            || !fee_limits_valid
            || limits.principal_msat > i64::MAX as u64
            || invoice_payees.iter().any(|(id, key)| {
                !quote.legs.iter().any(|leg| &leg.leg_id == id)
                    || nostr::secp256k1::PublicKey::from_str(key).is_err()
            })
        {
            return Err(ExecutionError::Budget);
        }
        let identity = wallet.identity().map_err(|_| ExecutionError::Binding)?;
        if identity.is_empty() {
            return Err(ExecutionError::Binding);
        }
        let plan = ApprovedPlan {
            quote,
            quote_event_id: event_id,
            wallet_identity: identity,
            limits,
            network,
            invoice_payees,
            bind_original_disclosures,
            fee_policy,
            // Legacy policies never stored a notice; keep their snapshots unchanged.
            fee_risk_notice: (fee_policy == RoutingFeePolicy::UnknownFeesAcknowledged)
                .then(|| UNKNOWN_ROUTING_FEES_NOTICE.to_owned()),
        };
        let snapshot = serde_json::to_string(&plan).map_err(|_| ExecutionError::Binding)?;
        if let Some(guard) = guard {
            guard().map_err(|_| ExecutionError::Binding)?;
        }
        // Proofs outside verified app accounting block (re)approval. Under the
        // warning-only policy a verified leg with unknown fees is accounted for.
        let saved=sqlx::query("INSERT INTO purchase_execution_approvals(account_pubkey,quote_id,snapshot) SELECT ?1,?2,?3 WHERE NOT EXISTS (SELECT 1 FROM settlement_invoice_attempts a WHERE a.account_pubkey=?1 AND a.quote_id=?2 AND (a.encrypted_proof IS NOT NULL OR a.state='settled') AND NOT EXISTS (SELECT 1 FROM purchase_execution_admissions p WHERE p.account_pubkey=?1 AND p.quote_id=?2 AND p.attempt_id=a.attempt_id AND p.state='verified' AND (p.fees_paid_msat IS NOT NULL OR json_extract(?3,'$.fee_policy')='unknown_fees_acknowledged'))) ON CONFLICT(account_pubkey,quote_id) DO UPDATE SET snapshot=purchase_execution_approvals.snapshot WHERE purchase_execution_approvals.snapshot=excluded.snapshot")
            .bind(&plan.quote.buyer_pubkey).bind(quote_id).bind(snapshot).execute(&self.pool).await?.rows_affected();
        if saved != 1 {
            return Err(ExecutionError::Binding);
        }
        Ok(())
    }

    /// Persist signed source evidence only after independently reconstructing
    /// the split. Replays are identical; changing source roots is never an upsert.
    pub async fn verify_sources(
        &self,
        signer: &dyn NostrSigner,
        quote_id: &str,
        listing: &nostr::Event,
        terms: &[nostr::Event],
    ) -> Result<(), ExecutionError> {
        let (quote, _) = self.quote(signer, quote_id).await?;
        verify_quote_listing_terms(&quote, listing, terms).map_err(|_| ExecutionError::Binding)?;
        let listing = serde_json::to_string(listing).map_err(|_| ExecutionError::Binding)?;
        let terms = serde_json::to_string(terms).map_err(|_| ExecutionError::Binding)?;
        let changed=sqlx::query("INSERT INTO purchase_quote_verification(account_pubkey,quote_id,listing_event_json,terms_events_json) VALUES(?,?,?,?) ON CONFLICT(account_pubkey,quote_id) DO UPDATE SET listing_event_json=purchase_quote_verification.listing_event_json WHERE purchase_quote_verification.listing_event_json=excluded.listing_event_json AND purchase_quote_verification.terms_events_json=excluded.terms_events_json")
            .bind(&quote.buyer_pubkey).bind(quote_id).bind(listing).bind(terms).execute(&self.pool).await?.rows_affected();
        if changed != 1 {
            return Err(ExecutionError::Binding);
        }
        Ok(())
    }

    async fn sources_verified(&self, quote: &SignedQuote) -> Result<bool, ExecutionError> {
        let evidence:Option<(String,String)>=sqlx::query_as("SELECT listing_event_json,terms_events_json FROM purchase_quote_verification WHERE account_pubkey=? AND quote_id=?")
            .bind(&quote.buyer_pubkey).bind(&quote.quote_id).fetch_optional(&self.pool).await?;
        let Some((listing, terms)) = evidence else {
            return Ok(false);
        };
        let listing: nostr::Event =
            serde_json::from_str(&listing).map_err(|_| ExecutionError::Binding)?;
        let terms: Vec<nostr::Event> =
            serde_json::from_str(&terms).map_err(|_| ExecutionError::Binding)?;
        verify_quote_listing_terms(quote, &listing, &terms).map_err(|_| ExecutionError::Binding)?;
        Ok(true)
    }

    /// Completion is backed by the receipt validation/persistence path, not by
    /// the count of paid legs or an arbitrary server status string.
    pub async fn validated_completion(
        &self,
        account: &str,
        quote: &SignedQuote,
        event_id: &str,
        receipt_json: Option<&str>,
    ) -> Result<bool, ExecutionError> {
        let Some(text) = receipt_json else {
            return Ok(false);
        };
        let Ok(event) = serde_json::from_str::<nostr::Event>(text) else {
            return Ok(false);
        };
        let Ok(parsed) = crate::purchases::parse_receipt_event(&event) else {
            return Ok(false);
        };
        let Some(aggregate) = parsed.aggregate else {
            return Ok(false);
        };
        if parsed.buyer_pubkey.to_hex() != account
            || parsed.coordinate != quote.listing_coordinate
            || !matches!(parsed.status.as_str(), "paid" | "fulfilled")
            || aggregate.quote_event_id.to_hex() != event_id
            || aggregate.listing_event_id.to_hex() != quote.listing_event_id
            || aggregate.processor_pubkey.to_hex() != quote.processor_pubkey
            || aggregate.gross_msat != quote.gross_msat
            || aggregate.legs.len() != quote.legs.len()
        {
            return Ok(false);
        }
        for (proof, leg) in aggregate.legs.iter().zip(&quote.legs) {
            if proof.leg_id != leg.leg_id
                || proof.role != leg.role
                || proof.recipient_pubkey != leg.recipient_pubkey
                || proof.amount_msat != leg.amount_msat
                || proof.accepted_terms_id != leg.accepted_terms_id
            {
                return Ok(false);
            }
        }
        let persisted:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM purchases WHERE event_id=? AND buyer_pubkey=? AND listing_coordinate=? AND status IN ('paid','fulfilled'))")
            .bind(event.id.to_hex()).bind(account).bind(&quote.listing_coordinate).fetch_one(&self.pool).await?;
        Ok(persisted)
    }

    /// Complete immutable review, including unpaid/uncertain legs. No request
    /// is made to the wallet; the desktop may attach optional get_info metadata.
    pub async fn review(
        &self,
        signer: &dyn NostrSigner,
        wallet: Option<&dyn PaymentWallet>,
        quote_id: &str,
    ) -> Result<PurchasePlanReview, ExecutionError> {
        let (quote, event_id) = self.quote(signer, quote_id).await?;
        let account = &quote.buyer_pubkey;
        let buyer = signer
            .get_public_key()
            .await
            .map_err(|_| ExecutionError::Binding)?;
        let identity = wallet
            .map(|w| w.identity())
            .transpose()
            .map_err(|_| ExecutionError::Binding)?;
        let stored = SettlementRepository::new(self.pool.clone())
            .load(account, quote_id)
            .await?
            .ok_or(ExecutionError::Binding)?;
        let approval:Option<(String,i64)>=sqlx::query_as("SELECT snapshot,halted FROM purchase_execution_approvals WHERE account_pubkey=? AND quote_id=?")
            .bind(account).bind(quote_id).fetch_optional(&self.pool).await?;
        let mut approved = false;
        let mut halted = false;
        let mut approved_limits = None;
        let mut approved_fee_policy = None;
        let mut approved_fee_notice = None;
        let mut network = None;
        if let Some((text, flag)) = approval {
            let plan: ApprovedPlan =
                serde_json::from_str(&text).map_err(|_| ExecutionError::Binding)?;
            if plan.quote != quote || plan.quote_event_id != event_id {
                return Err(ExecutionError::Binding);
            }
            approved = identity.as_ref() == Some(&plan.wallet_identity);
            halted = flag != 0;
            approved_limits = Some(plan.limits);
            approved_fee_policy = Some(plan.fee_policy);
            approved_fee_notice = plan.fee_risk_notice;
            network = Some(plan.network);
        }
        let mut statuses = Vec::new();
        for leg in &quote.legs {
            let attempt:Option<(String,Vec<u8>,String,String,bool,bool)>=sqlx::query_as("SELECT a.attempt_id,a.encrypted_attempt,a.payment_hash,a.state,a.encrypted_proof IS NOT NULL,EXISTS(SELECT 1 FROM nwc_payment_journal p WHERE p.account_pubkey=a.account_pubkey AND p.payment_hash=a.payment_hash) FROM settlement_invoice_attempts a WHERE a.account_pubkey=? AND a.quote_id=? AND a.leg_id=? ORDER BY rowid LIMIT 1")
                .bind(account).bind(quote_id).bind(&leg.leg_id).fetch_optional(&self.pool).await?;
            let admission:Option<(String,Option<i64>)>=sqlx::query_as("SELECT state,fees_paid_msat FROM purchase_execution_admissions WHERE account_pubkey=? AND quote_id=? AND leg_id=?")
                .bind(account).bind(quote_id).bind(&leg.leg_id).fetch_optional(&self.pool).await?;
            let mut status = PurchaseLegStatus {
                leg_id: leg.leg_id.clone(),
                progress: if stored.state == "failed" {
                    LegProgress::FailedNotSent
                } else {
                    LegProgress::Pending
                },
                attempt_id: None,
                payment_hash: None,
                invoice_payee: None,
                fees_paid_msat: None,
                can_reconcile: admission.is_some(),
            };
            if let Some((id, encrypted, hash, state, has_proof, has_intent)) = attempt {
                let text = signer
                    .nip44_decrypt(
                        &buyer,
                        std::str::from_utf8(&encrypted).map_err(|_| ExecutionError::Binding)?,
                    )
                    .await
                    .map_err(|_| ExecutionError::Binding)?;
                let invoice: SettlementInvoiceResponse =
                    serde_json::from_str(&text).map_err(|_| ExecutionError::Binding)?;
                let parsed = Bolt11Invoice::from_str(&invoice.bolt11)
                    .map_err(|_| ExecutionError::Binding)?;
                if invoice.quote_id != quote_id
                    || invoice.leg_id != leg.leg_id
                    || invoice.attempt_id != id
                    || invoice.payment_hash != hash
                    || parsed.payment_hash().to_string() != hash
                    || invoice.amount_msat != leg.amount_msat
                    || parsed.amount_milli_satoshis() != Some(leg.amount_msat)
                {
                    return Err(ExecutionError::Binding);
                }
                let current = match parsed.currency() {
                    Currency::Bitcoin => InvoiceNetwork::Bitcoin,
                    Currency::BitcoinTestnet => InvoiceNetwork::Testnet,
                    Currency::Signet => InvoiceNetwork::Signet,
                    Currency::Regtest => InvoiceNetwork::Regtest,
                    _ => return Err(ExecutionError::Binding),
                };
                if network.is_some_and(|n| n != current) {
                    return Err(ExecutionError::Binding);
                }
                network = Some(current);
                status.attempt_id = Some(id);
                status.payment_hash = Some(hash);
                status.invoice_payee = Some(parsed.recover_payee_pub_key().to_string());
                status.can_reconcile |= has_proof || has_intent;
                if state == "settled" {
                    status.progress = LegProgress::ProofAccepted
                } else if has_intent || has_proof {
                    status.progress = LegProgress::Uncertain
                }
            }
            if let Some((state, fees)) = admission {
                status.fees_paid_msat = fees
                    .map(u64::try_from)
                    .transpose()
                    .map_err(|_| ExecutionError::Binding)?;
                if status.progress != LegProgress::ProofAccepted {
                    status.progress = if state == "verified" {
                        LegProgress::Paid
                    } else {
                        LegProgress::Uncertain
                    }
                }
            }
            statuses.push(status);
        }
        let commitment = hex::encode(Sha256::digest(
            serde_json::to_vec(&(&quote, &event_id, &identity))
                .map_err(|_| ExecutionError::Binding)?,
        ));
        let completed = stored.state == "settled"
            && self
                .validated_completion(
                    account,
                    &quote,
                    &event_id,
                    stored.receipt_event_json.as_deref(),
                )
                .await?;
        Ok(PurchasePlanReview {
            terms_verified: self.sources_verified(&quote).await?,
            purchase_completed: completed,
            settlement_state: stored.state,
            quote,
            quote_event_id: event_id,
            commitment,
            network,
            approved,
            approved_limits,
            approved_fee_policy,
            approved_fee_notice,
            halted,
            wallet_connected: wallet.is_some(),
            wallet_capabilities: None,
            legs: statuses,
            // Lock reason, or the visible test-build label under test activation.
            restriction: crate::purchase_contract::require_live_purchase_authorization()
                .err()
                .or_else(|| crate::payment_boundary::activation_status().test_build_label)
                .unwrap_or_default(),
        })
    }

    /// Buyer explicitly approves the verified plan; invoice keys/hashes are bound
    /// on original ADP disclosure, not invented or requested from a custom wallet.
    pub async fn approve_verified(
        &self,
        signer: &dyn NostrSigner,
        wallet: &dyn PaymentWallet,
        quote_id: &str,
        event_id: &str,
        commitment: &str,
        limits: PurchaseLimits,
        network: InvoiceNetwork,
    ) -> Result<(), ExecutionError> {
        self.approve_verified_guarded(
            signer,
            wallet,
            quote_id,
            event_id,
            commitment,
            limits,
            network,
            &|| Ok(()),
        )
        .await
    }
    pub async fn approve_verified_guarded(
        &self,
        signer: &dyn NostrSigner,
        wallet: &dyn PaymentWallet,
        quote_id: &str,
        event_id: &str,
        commitment: &str,
        limits: PurchaseLimits,
        network: InvoiceNetwork,
        guard: &(dyn Fn() -> Result<(), String> + Send + Sync),
    ) -> Result<(), ExecutionError> {
        self.approve_verified_with_fee_policy(
            signer,
            wallet,
            quote_id,
            event_id,
            commitment,
            limits,
            network,
            RoutingFeePolicy::ProviderCeiling,
            false,
            guard,
        )
        .await
    }

    /// Any policy other than the strict ceiling is deliberately opt-in and
    /// requires the buyer's explicit acknowledgement (`accept_risk`). It does
    /// not establish a wallet-enforced fee ceiling or unlock the live release
    /// gate. The warning-only policy records the acknowledged notice in the
    /// immutable approval and must not be given a fee reserve or total.
    #[allow(clippy::too_many_arguments)]
    pub async fn approve_verified_with_fee_policy(
        &self,
        signer: &dyn NostrSigner,
        wallet: &dyn PaymentWallet,
        quote_id: &str,
        event_id: &str,
        commitment: &str,
        limits: PurchaseLimits,
        network: InvoiceNetwork,
        fee_policy: RoutingFeePolicy,
        accept_risk: bool,
        guard: &(dyn Fn() -> Result<(), String> + Send + Sync),
    ) -> Result<(), ExecutionError> {
        if fee_policy != RoutingFeePolicy::ProviderCeiling && !accept_risk {
            return Err(ExecutionError::FeeRiskAcknowledgementRequired);
        }
        let review = self.review(signer, Some(wallet), quote_id).await?;
        if !review.terms_verified
            || review.commitment != commitment
            || review.quote_event_id != event_id
            || review.network.is_some_and(|n| n != network)
            || review.purchase_completed
            || review.settlement_state != "active"
            || nostr::Timestamp::now().as_secs() >= review.quote.payment_expires_at
        {
            return Err(ExecutionError::Binding);
        }
        let payees = review
            .legs
            .iter()
            .filter_map(|leg| {
                leg.invoice_payee
                    .clone()
                    .map(|key| (leg.leg_id.clone(), key))
            })
            .collect();
        self.approve_inner(
            signer,
            wallet,
            quote_id,
            event_id,
            limits,
            network,
            payees,
            true,
            fee_policy,
            accept_risk.then_some(UNKNOWN_ROUTING_FEES_NOTICE),
            Some(guard),
        )
        .await
    }

    /// A listing change invalidated the approved quote. If nothing was ever
    /// admitted for dispatch (no admission, journaled intent or proof), the local
    /// quote becomes `superseded` and its approval is halted: no payment can
    /// follow, and the processor's unpaid quote simply expires. If any leg was
    /// admitted, only further dispatch is halted and the purchase continues
    /// through partial-settlement recovery. Never pays, deletes or retries.
    pub async fn supersede_unpaid(
        &self,
        account: &str,
        quote_id: &str,
    ) -> Result<SupersedeOutcome, ExecutionError> {
        let mut tx = self.pool.begin().await?;
        // Write first: serialize with dispatch admission for this quote.
        let state: Option<String> = sqlx::query_scalar(
            "UPDATE settlement_quotes SET updated_at=updated_at WHERE account_pubkey=? AND quote_id=? RETURNING state",
        )
        .bind(account)
        .bind(quote_id)
        .fetch_optional(&mut *tx)
        .await?;
        let Some(state) = state else {
            return Err(ExecutionError::Binding);
        };
        match state.as_str() {
            "superseded" => return Ok(SupersedeOutcome::Superseded),
            "settled" => return Ok(SupersedeOutcome::AlreadySettled),
            _ => {}
        }
        let dispatched:i64=sqlx::query_scalar("SELECT (SELECT COUNT(*) FROM purchase_execution_admissions WHERE account_pubkey=?1 AND quote_id=?2)+(SELECT COUNT(*) FROM settlement_invoice_attempts a WHERE a.account_pubkey=?1 AND a.quote_id=?2 AND (a.encrypted_proof IS NOT NULL OR a.state='settled' OR EXISTS(SELECT 1 FROM nwc_payment_journal p WHERE p.account_pubkey=a.account_pubkey AND p.payment_hash=a.payment_hash)))")
            .bind(account).bind(quote_id).fetch_one(&mut *tx).await?;
        sqlx::query("UPDATE purchase_execution_approvals SET halted=1 WHERE account_pubkey=? AND quote_id=?")
            .bind(account).bind(quote_id).execute(&mut *tx).await?;
        let outcome = if dispatched == 0 && state == "active" {
            sqlx::query("UPDATE settlement_quotes SET state='superseded',updated_at=unixepoch() WHERE account_pubkey=? AND quote_id=? AND state='active'")
                .bind(account).bind(quote_id).execute(&mut *tx).await?;
            SupersedeOutcome::Superseded
        } else {
            SupersedeOutcome::HaltedAfterDispatch
        };
        tx.commit().await?;
        Ok(outcome)
    }

    pub async fn pay_leg(
        &self,
        signer: &dyn NostrSigner,
        wallet: &dyn PaymentWallet,
        quote_id: &str,
        attempt_id: &str,
    ) -> Result<PayInvoiceResult, ExecutionError> {
        self.pay_leg_guarded(signer, wallet, quote_id, attempt_id, &|| Ok(()))
            .await
    }

    pub async fn pay_leg_guarded(
        &self,
        signer: &dyn NostrSigner,
        wallet: &dyn PaymentWallet,
        quote_id: &str,
        attempt_id: &str,
        guard: &(dyn Fn() -> Result<(), String> + Send + Sync),
    ) -> Result<PayInvoiceResult, ExecutionError> {
        self.execute(signer, wallet, quote_id, attempt_id, true, guard)
            .await
    }

    /// Read-only: never creates an admission/intent or exercises payment authority.
    pub async fn reconcile_leg(
        &self,
        signer: &dyn NostrSigner,
        wallet: &dyn PaymentWallet,
        quote_id: &str,
        attempt_id: &str,
    ) -> Result<PayInvoiceResult, ExecutionError> {
        self.execute(signer, wallet, quote_id, attempt_id, false, &|| Ok(()))
            .await
    }

    async fn execute(
        &self,
        signer: &dyn NostrSigner,
        wallet: &dyn PaymentWallet,
        quote_id: &str,
        attempt_id: &str,
        allow_dispatch: bool,
        guard: &(dyn Fn() -> Result<(), String> + Send + Sync),
    ) -> Result<PayInvoiceResult, ExecutionError> {
        let (quote, event_id) = self.quote(signer, quote_id).await?;
        let sources_verified = self.sources_verified(&quote).await?;
        let account = &quote.buyer_pubkey;
        let buyer = signer
            .get_public_key()
            .await
            .map_err(|_| ExecutionError::Binding)?;
        let repo = SettlementRepository::new(self.pool.clone());
        let stored = repo
            .invoice_attempt(account, quote_id, attempt_id)
            .await?
            .ok_or(ExecutionError::Binding)?;
        let text = signer
            .nip44_decrypt(
                &buyer,
                std::str::from_utf8(&stored.encrypted_attempt)
                    .map_err(|_| ExecutionError::Binding)?,
            )
            .await
            .map_err(|_| ExecutionError::Binding)?;
        let invoice: SettlementInvoiceResponse =
            serde_json::from_str(&text).map_err(|_| ExecutionError::Binding)?;
        crate::adp_settlement::verify_original_invoice(&quote, &invoice)
            .map_err(|_| ExecutionError::Binding)?;
        let parsed =
            Bolt11Invoice::from_str(&invoice.bolt11).map_err(|_| ExecutionError::Binding)?;
        let commitment = hex::encode(Sha256::digest(invoice.bolt11.as_bytes()));
        let leg = quote
            .legs
            .iter()
            .find(|leg| leg.leg_id == invoice.leg_id)
            .ok_or(ExecutionError::Binding)?;
        if invoice.quote_id != quote_id
            || invoice.attempt_id != attempt_id
            || invoice.payment_hash != stored.payment_hash
            || invoice.payment_hash != parsed.payment_hash().to_string()
            || invoice.amount_msat != leg.amount_msat
            || parsed.amount_milli_satoshis() != Some(leg.amount_msat)
        {
            return Err(ExecutionError::Binding);
        }
        let first:String=sqlx::query_scalar("SELECT attempt_id FROM settlement_invoice_attempts WHERE account_pubkey=? AND quote_id=? AND leg_id=? ORDER BY rowid LIMIT 1")
            .bind(account).bind(quote_id).bind(&leg.leg_id).fetch_one(&self.pool).await?;
        if first != attempt_id {
            return Err(ExecutionError::Binding);
        }
        let identity = wallet.identity().map_err(|_| ExecutionError::Binding)?;
        let mut tx = self.pool.begin().await?;
        // First statement is a write: serialize purchase admissions across pools
        // and processes without holding an application mutex over wallet awaits.
        let locked=sqlx::query("UPDATE purchase_execution_approvals SET snapshot=snapshot WHERE account_pubkey=? AND quote_id=?")
            .bind(account).bind(quote_id).execute(&mut *tx).await?.rows_affected();
        if locked != 1 {
            return Err(ExecutionError::ApprovalRequired);
        }
        let row=sqlx::query("SELECT snapshot,halted FROM purchase_execution_approvals WHERE account_pubkey=? AND quote_id=?")
            .bind(account).bind(quote_id).fetch_one(&mut *tx).await?;
        let plan: ApprovedPlan = serde_json::from_str(&row.try_get::<String, _>("snapshot")?)
            .map_err(|_| ExecutionError::Binding)?;
        if plan.quote != quote
            || plan.quote_event_id != event_id
            || plan.wallet_identity != identity
            || (plan.bind_original_disclosures && !sources_verified)
            || parsed.currency() != plan.network.currency()
            || match plan.invoice_payees.get(&leg.leg_id) {
                Some(expected) => expected != &parsed.recover_payee_pub_key().to_string(),
                None => !plan.bind_original_disclosures,
            }
        {
            return Err(ExecutionError::Binding);
        }
        let unknown_fees = plan.fee_policy == RoutingFeePolicy::UnknownFeesAcknowledged;
        let existing=sqlx::query("SELECT attempt_id,payment_hash,invoice_commitment,fee_bound_msat FROM purchase_execution_admissions WHERE account_pubkey=? AND quote_id=? AND leg_id=?")
            .bind(account).bind(quote_id).bind(&leg.leg_id).fetch_optional(&mut *tx).await?;
        let dispatch = existing.is_none();
        let fee_bound;
        if let Some(existing) = existing {
            if existing.try_get::<String, _>("attempt_id")? != attempt_id
                || existing.try_get::<String, _>("payment_hash")? != invoice.payment_hash
                || existing.try_get::<String, _>("invoice_commitment")? != commitment
            {
                return Err(ExecutionError::Binding);
            }
            fee_bound = u64::try_from(existing.try_get::<i64, _>("fee_bound_msat")?)
                .map_err(|_| ExecutionError::Binding)?;
        } else {
            if !allow_dispatch {
                return Err(ExecutionError::Uncertain);
            }
            let now = nostr::Timestamp::now().as_secs();
            if row.try_get::<i64, _>("halted")? != 0
                || now >= quote.payment_expires_at
                || parsed
                    .duration_since_epoch()
                    .as_secs()
                    .checked_add(parsed.expiry_time().as_secs())
                    .is_none_or(|end| end <= now)
                || now >= invoice.expires_at
            {
                return Err(ExecutionError::Budget);
            }
            let unresolved:i64=sqlx::query_scalar("SELECT COUNT(*) FROM purchase_execution_admissions WHERE account_pubkey=? AND quote_id=? AND state!='verified'")
                .bind(account).bind(quote_id).fetch_one(&mut *tx).await?;
            let old_intents:i64=sqlx::query_scalar("SELECT COUNT(*) FROM settlement_invoice_attempts a WHERE a.account_pubkey=? AND a.quote_id=? AND a.state!='settled' AND (a.encrypted_proof IS NOT NULL OR EXISTS(SELECT 1 FROM nwc_payment_journal p WHERE p.account_pubkey=a.account_pubkey AND p.payment_hash=a.payment_hash))")
                .bind(account).bind(quote_id).fetch_one(&mut *tx).await?;
            if unresolved != 0 || old_intents != 0 {
                return Err(ExecutionError::Uncertain);
            }
            let state: String = sqlx::query_scalar(
                "SELECT state FROM settlement_quotes WHERE account_pubkey=? AND quote_id=?",
            )
            .bind(account)
            .bind(quote_id)
            .fetch_one(&mut *tx)
            .await?;
            if state != "active" {
                return Err(ExecutionError::Budget);
            }
            let totals=sqlx::query("SELECT COUNT(*) AS n,COALESCE(SUM(amount_msat),0) AS principal,COALESCE(SUM(CASE WHEN state='verified' THEN COALESCE(fees_paid_msat,fee_bound_msat) ELSE fee_bound_msat END),0) AS fees FROM purchase_execution_admissions WHERE account_pubkey=? AND quote_id=?")
                .bind(account).bind(quote_id).fetch_one(&mut *tx).await?;
            let n = usize::try_from(totals.try_get::<i64, _>("n")?)
                .map_err(|_| ExecutionError::Budget)?;
            let principal = u64::try_from(totals.try_get::<i64, _>("principal")?)
                .map_err(|_| ExecutionError::Budget)?;
            let principal = principal
                .checked_add(leg.amount_msat)
                .ok_or(ExecutionError::Budget)?;
            if n >= plan.limits.maximum_dispatches || principal > plan.limits.principal_msat {
                return Err(ExecutionError::Budget);
            }
            if unknown_fees {
                // Warning-only: no reserve, no hidden cap. The wallet chooses the
                // routing fee; recorded `fee_bound_msat` is 0 and is not a bound.
                if plan.fee_risk_notice.as_deref() != Some(UNKNOWN_ROUTING_FEES_NOTICE) {
                    return Err(ExecutionError::FeeRiskAcknowledgementRequired);
                }
                fee_bound = 0;
            } else {
                let fees = u64::try_from(totals.try_get::<i64, _>("fees")?)
                    .map_err(|_| ExecutionError::Budget)?;
                let reserve = plan.limits.fee_reserve_msat.ok_or(ExecutionError::Budget)?;
                let total = plan.limits.total_msat.ok_or(ExecutionError::Budget)?;
                let remaining = reserve.checked_sub(fees).ok_or(ExecutionError::Budget)?;
                fee_bound = match plan.fee_policy {
                    RoutingFeePolicy::ReportedFees => wallet
                        .maximum_fee_msat(&invoice.bolt11)
                        .unwrap_or(remaining)
                        .min(remaining),
                    RoutingFeePolicy::ProviderCeiling
                    | RoutingFeePolicy::UnknownFeesAcknowledged => wallet
                        .maximum_fee_msat(&invoice.bolt11)
                        .ok_or(ExecutionError::FeeBoundUnavailable)?,
                };
                let fees = fees.checked_add(fee_bound).ok_or(ExecutionError::Budget)?;
                if fees > reserve || principal.checked_add(fees).is_none_or(|sum| sum > total) {
                    return Err(ExecutionError::Budget);
                }
            }
            guard().map_err(|_| ExecutionError::Binding)?;
            sqlx::query("INSERT INTO purchase_execution_admissions(account_pubkey,quote_id,leg_id,attempt_id,payment_hash,invoice_commitment,amount_msat,fee_bound_msat,state) VALUES(?,?,?,?,?,?,?,?,'uncertain')")
                .bind(account).bind(quote_id).bind(&leg.leg_id).bind(attempt_id).bind(&invoice.payment_hash).bind(&commitment).bind(leg.amount_msat as i64).bind(fee_bound as i64).execute(&mut *tx).await?;
        }
        tx.commit().await?;
        let journal = DurableNwcPayments::new(self.pool.clone());
        let mut result = if dispatch {
            journal
                .pay_or_resume_guarded(signer, wallet, &invoice.bolt11, guard)
                .await?
        } else {
            journal.reconcile(signer, wallet, &invoice.bolt11).await?
        };
        if unknown_fees {
            // The journal has validated the preimage: the payment outcome is
            // known. A missing fee report is unknown fee data, never zero and
            // never a reason to treat the leg as uncertain or to pay again.
            if result.fees_paid_msat.is_none() {
                // Best effort, read-only: only adds a report the wallet exposes.
                if let Ok(Some(found)) = wallet
                    .lookup(&invoice.payment_hash, invoice.amount_msat)
                    .await
                {
                    if crate::durable_payments::validate_payment_proof(&invoice.bolt11, &found)
                        .is_ok()
                    {
                        result.fees_paid_msat = found.fees_paid_msat;
                    }
                }
            }
            // An unrepresentable report is malformed: keep the fee unknown.
            let reported = result
                .fees_paid_msat
                .and_then(|fee| i64::try_from(fee).ok());
            result.fees_paid_msat = reported.and_then(|fee| u64::try_from(fee).ok());
            guard().map_err(|_| ExecutionError::Binding)?;
            // A known fee is never replaced by a later missing report.
            sqlx::query("UPDATE purchase_execution_admissions SET state='verified',fees_paid_msat=COALESCE(fees_paid_msat,?) WHERE account_pubkey=? AND quote_id=? AND leg_id=?")
                .bind(reported).bind(account).bind(quote_id).bind(&leg.leg_id).execute(&self.pool).await?;
            return Ok(result);
        }
        if result.fees_paid_msat.is_none() {
            result = wallet
                .lookup(&invoice.payment_hash, invoice.amount_msat)
                .await
                .map_err(|_| ExecutionError::Uncertain)?
                .ok_or(ExecutionError::Uncertain)?;
            crate::durable_payments::validate_payment_proof(&invoice.bolt11, &result)?;
        }
        let fee = result.fees_paid_msat.ok_or(ExecutionError::Uncertain)?;
        if fee > fee_bound {
            let mut tx = self.pool.begin().await?;
            sqlx::query("UPDATE purchase_execution_approvals SET halted=1 WHERE account_pubkey=? AND quote_id=?").bind(account).bind(quote_id).execute(&mut *tx).await?;
            // A valid preimage is still paid evidence when a provider exceeds
            // the fee reservation. Halt spending, retain the true fee, and let
            // read-only recovery submit the original proof rather than repay.
            if let Ok(actual) = i64::try_from(fee) {
                sqlx::query("UPDATE purchase_execution_admissions SET state='verified',fees_paid_msat=? WHERE account_pubkey=? AND quote_id=? AND leg_id=?")
                    .bind(actual).bind(account).bind(quote_id).bind(&leg.leg_id).execute(&mut *tx).await?;
            }
            tx.commit().await?;
            return if !allow_dispatch && fee <= i64::MAX as u64 {
                Ok(result)
            } else {
                Err(ExecutionError::Budget)
            };
        }
        guard().map_err(|_| ExecutionError::Binding)?;
        sqlx::query("UPDATE purchase_execution_admissions SET state='verified',fees_paid_msat=? WHERE account_pubkey=? AND quote_id=? AND leg_id=?")
            .bind(fee as i64).bind(account).bind(quote_id).bind(&leg.leg_id).execute(&self.pool).await?;
        Ok(result)
    }
}
