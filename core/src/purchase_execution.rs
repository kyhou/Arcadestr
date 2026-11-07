//! Arcadestr-owned immutable approval and purchase-wide dispatch fencing.
//! No provider-specific database, permission API or custom NWC method is used.
pub use crate::purchase_contract::{InvoiceNetwork, PurchaseLimits};
use crate::purchase_contract::{
    LegProgress, PurchaseLegStatus, PurchasePlanReview, RoutingFeePolicy,
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
    #[error("Reported-fee accounting requires explicit acknowledgment that the wallet may exceed the desired maximum")]
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
}

pub struct PurchaseExecutor {
    pool: SqlitePool,
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
        guard: Option<&(dyn Fn() -> Result<(), String> + Send + Sync)>,
    ) -> Result<(), ExecutionError> {
        let (quote, event_id) = self.quote(signer, quote_id).await?;
        if event_id != expected_quote_event_id
            || quote.state != QuoteState::Active
            || quote.legs.is_empty()
            || limits.principal_msat != quote.gross_msat
            || limits.maximum_dispatches != quote.legs.len()
            || (!bind_original_disclosures && invoice_payees.len() != quote.legs.len())
            || limits
                .principal_msat
                .checked_add(limits.fee_reserve_msat)
                .is_none_or(|sum| sum > limits.total_msat)
            || [
                limits.principal_msat,
                limits.fee_reserve_msat,
                limits.total_msat,
            ]
            .iter()
            .any(|v| *v > i64::MAX as u64)
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
        };
        let snapshot = serde_json::to_string(&plan).map_err(|_| ExecutionError::Binding)?;
        if let Some(guard) = guard {
            guard().map_err(|_| ExecutionError::Binding)?;
        }
        let saved=sqlx::query("INSERT INTO purchase_execution_approvals(account_pubkey,quote_id,snapshot) VALUES(?,?,?) ON CONFLICT(account_pubkey,quote_id) DO UPDATE SET snapshot=purchase_execution_approvals.snapshot WHERE purchase_execution_approvals.snapshot=excluded.snapshot")
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
            halted,
            wallet_connected: wallet.is_some(),
            wallet_capabilities: None,
            legs: statuses,
            restriction: crate::purchase_contract::require_live_purchase_authorization()
                .err()
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

    /// Standard-provider-compatible accounting is deliberately opt-in. It does
    /// not establish a wallet-enforced fee ceiling or unlock the live release gate.
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
        if fee_policy == RoutingFeePolicy::ReportedFees && !accept_risk {
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
            Some(guard),
        )
        .await
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
            let fees = u64::try_from(totals.try_get::<i64, _>("fees")?)
                .map_err(|_| ExecutionError::Budget)?;
            let remaining = plan
                .limits
                .fee_reserve_msat
                .checked_sub(fees)
                .ok_or(ExecutionError::Budget)?;
            fee_bound = match plan.fee_policy {
                RoutingFeePolicy::ProviderCeiling => wallet
                    .maximum_fee_msat(&invoice.bolt11)
                    .ok_or(ExecutionError::FeeBoundUnavailable)?,
                RoutingFeePolicy::ReportedFees => wallet
                    .maximum_fee_msat(&invoice.bolt11)
                    .unwrap_or(remaining)
                    .min(remaining),
            };
            let principal = principal
                .checked_add(leg.amount_msat)
                .ok_or(ExecutionError::Budget)?;
            let fees = fees.checked_add(fee_bound).ok_or(ExecutionError::Budget)?;
            if n >= plan.limits.maximum_dispatches
                || principal > plan.limits.principal_msat
                || fees > plan.limits.fee_reserve_msat
                || principal
                    .checked_add(fees)
                    .is_none_or(|total| total > plan.limits.total_msat)
            {
                return Err(ExecutionError::Budget);
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
