//! Durable at-most-one-dispatch NWC payments. Unknown outcomes are reconciled,
//! never retried as a fresh payment, including after invoice expiry.
use async_trait::async_trait;
use lightning_invoice::Bolt11Invoice;
use sha2::{Digest, Sha256};
use sqlx::{Row, SqlitePool};
use std::{
    str::FromStr,
    time::{SystemTime, UNIX_EPOCH},
};
use thiserror::Error;

use crate::{
    nwc_client::{NwcClient, NwcClientError, PayInvoiceResult},
    signers::NostrSigner,
};

#[async_trait]
pub trait PaymentWallet: Send + Sync {
    fn identity(&self) -> Result<String, NwcClientError>;
    /// A guaranteed existing-provider fee ceiling, never an application estimate.
    /// Standard NWC pay_invoice alone supplies no such guarantee. Unknown means
    /// capped purchase execution must stop; adapters must not invent capabilities.
    fn maximum_fee_msat(&self, _invoice: &str) -> Option<u64> {
        None
    }
    async fn pay(&self, invoice: &str) -> Result<PayInvoiceResult, NwcClientError>;
    async fn lookup(
        &self,
        payment_hash: &str,
        amount_msat: u64,
    ) -> Result<Option<PayInvoiceResult>, NwcClientError>;
}

#[async_trait]
impl PaymentWallet for NwcClient {
    fn identity(&self) -> Result<String, NwcClientError> {
        self.payment_identity()
    }
    async fn pay(&self, invoice: &str) -> Result<PayInvoiceResult, NwcClientError> {
        self.pay_invoice(invoice).await
    }
    async fn lookup(
        &self,
        hash: &str,
        amount: u64,
    ) -> Result<Option<PayInvoiceResult>, NwcClientError> {
        self.lookup_payment(hash, amount).await
    }
}

#[derive(Debug, Error)]
pub enum DurablePaymentError {
    #[error("invalid or expired invoice for a new payment")]
    InvalidInvoice,
    #[error("payment is bound to a different account, wallet or invoice")]
    Binding,
    #[error(
        "payment outcome unresolved; reconcile the existing wallet, never pay again automatically"
    )]
    Unresolved,
    #[error("no durable payment intent exists; reconciliation cannot initiate payment")]
    NoIntent,
    #[error("wallet returned an invalid payment proof")]
    InvalidProof,
    #[error("payment credential encryption/decryption unavailable")]
    Crypto,
    #[error("durable payment storage unavailable")]
    Storage(#[from] sqlx::Error),
}

pub struct DurableNwcPayments {
    pool: SqlitePool,
}

impl DurableNwcPayments {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// The only call allowed to dispatch, and only after winning a durable INSERT.
    pub async fn pay_or_resume(
        &self,
        signer: &dyn NostrSigner,
        wallet: &dyn PaymentWallet,
        invoice: &str,
    ) -> Result<PayInvoiceResult, DurablePaymentError> {
        self.execute(signer, wallet, invoice, true, None).await
    }

    /// Fence a session immediately before intent creation and external dispatch.
    pub async fn pay_or_resume_guarded(
        &self,
        signer: &dyn NostrSigner,
        wallet: &dyn PaymentWallet,
        invoice: &str,
        guard: &(dyn Fn() -> Result<(), String> + Send + Sync),
    ) -> Result<PayInvoiceResult, DurablePaymentError> {
        self.execute(signer, wallet, invoice, true, Some(guard))
            .await
    }

    /// Read-only wallet reconciliation: this path cannot create an intent or pay.
    pub async fn reconcile(
        &self,
        signer: &dyn NostrSigner,
        wallet: &dyn PaymentWallet,
        invoice: &str,
    ) -> Result<PayInvoiceResult, DurablePaymentError> {
        self.execute(signer, wallet, invoice, false, None).await
    }

    async fn execute(
        &self,
        signer: &dyn NostrSigner,
        wallet: &dyn PaymentWallet,
        invoice: &str,
        allow_dispatch: bool,
        guard: Option<&(dyn Fn() -> Result<(), String> + Send + Sync)>,
    ) -> Result<PayInvoiceResult, DurablePaymentError> {
        let parsed =
            Bolt11Invoice::from_str(invoice).map_err(|_| DurablePaymentError::InvalidInvoice)?;
        let amount = parsed
            .amount_milli_satoshis()
            .filter(|a| *a > 0 && *a <= i64::MAX as u64)
            .ok_or(DurablePaymentError::InvalidInvoice)?;
        let hash = parsed.payment_hash().to_string();
        let commitment = hex::encode(Sha256::digest(invoice.as_bytes()));
        let account = signer
            .get_public_key()
            .await
            .map_err(|_| DurablePaymentError::Crypto)?;
        let identity = wallet
            .identity()
            .map_err(|_| DurablePaymentError::Binding)?;
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| DurablePaymentError::InvalidInvoice)?
            .as_secs();
        let mut row = sqlx::query("SELECT wallet_identity,invoice_commitment,amount_msat,encrypted_payment FROM nwc_payment_journal WHERE account_pubkey=? AND payment_hash=?")
            .bind(account.to_hex()).bind(&hash).fetch_optional(&self.pool).await?;
        let mut dispatch = false;
        if row.is_none() {
            if !allow_dispatch {
                return Err(DurablePaymentError::NoIntent);
            }
            if parsed
                .duration_since_epoch()
                .as_secs()
                .checked_add(parsed.expiry_time().as_secs())
                .is_none_or(|end| end <= now)
            {
                return Err(DurablePaymentError::InvalidInvoice);
            }
            let encrypted = signer
                .nip44_encrypt(&account, invoice)
                .await
                .map_err(|_| DurablePaymentError::Crypto)?;
            if let Some(guard) = guard {
                guard().map_err(|_| DurablePaymentError::Binding)?;
            }
            dispatch = sqlx::query("INSERT INTO nwc_payment_journal (account_pubkey,payment_hash,wallet_identity,invoice_commitment,amount_msat,encrypted_invoice,created_at,updated_at,dispatch_count,lookup_count) VALUES (?,?,?,?,?,?,?,?,0,0) ON CONFLICT(account_pubkey,payment_hash) DO NOTHING")
                .bind(account.to_hex()).bind(&hash).bind(&identity).bind(&commitment).bind(amount as i64)
                .bind(encrypted.as_bytes()).bind(now as i64).bind(now as i64).execute(&self.pool).await?.rows_affected() == 1;
            row = sqlx::query("SELECT wallet_identity,invoice_commitment,amount_msat,encrypted_payment FROM nwc_payment_journal WHERE account_pubkey=? AND payment_hash=?")
                .bind(account.to_hex()).bind(&hash).fetch_optional(&self.pool).await?;
        }
        let row = row.ok_or(DurablePaymentError::Binding)?;
        if row.try_get::<String, _>("wallet_identity")? != identity
            || row.try_get::<String, _>("invoice_commitment")? != commitment
            || row.try_get::<i64, _>("amount_msat")? != amount as i64
        {
            return Err(DurablePaymentError::Binding);
        }
        if let Some(encrypted) = row.try_get::<Option<Vec<u8>>, _>("encrypted_payment")? {
            let text = std::str::from_utf8(&encrypted).map_err(|_| DurablePaymentError::Crypto)?;
            let plaintext = signer
                .nip44_decrypt(&account, text)
                .await
                .map_err(|_| DurablePaymentError::Crypto)?;
            let result =
                serde_json::from_str(&plaintext).map_err(|_| DurablePaymentError::Crypto)?;
            validate_payment_proof(invoice, &result)?;
            return Ok(result);
        }
        let result = if dispatch {
            let counted=sqlx::query("UPDATE nwc_payment_journal SET dispatch_count=1 WHERE account_pubkey=? AND payment_hash=? AND dispatch_count=0")
                .bind(account.to_hex()).bind(&hash).execute(&self.pool).await?.rows_affected();
            if counted != 1 {
                return Err(DurablePaymentError::Binding);
            }
            if let Some(guard) = guard {
                guard().map_err(|_| DurablePaymentError::Binding)?;
            }
            wallet
                .pay(invoice)
                .await
                .map_err(|_| DurablePaymentError::Unresolved)?
        } else {
            sqlx::query("UPDATE nwc_payment_journal SET lookup_count=COALESCE(lookup_count,0)+1 WHERE account_pubkey=? AND payment_hash=?")
                .bind(account.to_hex()).bind(&hash).execute(&self.pool).await?;
            wallet
                .lookup(&hash, amount)
                .await
                .map_err(|_| DurablePaymentError::Unresolved)?
                .ok_or(DurablePaymentError::Unresolved)?
        };
        validate_payment_proof(invoice, &result)?;
        let plaintext = serde_json::to_string(&result).map_err(|_| DurablePaymentError::Crypto)?;
        let encrypted = signer
            .nip44_encrypt(&account, &plaintext)
            .await
            .map_err(|_| DurablePaymentError::Crypto)?;
        let saved = sqlx::query("UPDATE nwc_payment_journal SET encrypted_payment=COALESCE(encrypted_payment,?),updated_at=? WHERE account_pubkey=? AND payment_hash=? AND wallet_identity=? AND invoice_commitment=?")
            .bind(encrypted.as_bytes()).bind(now as i64).bind(account.to_hex()).bind(&hash).bind(identity).bind(commitment)
            .execute(&self.pool).await?.rows_affected();
        if saved != 1 {
            return Err(DurablePaymentError::Binding);
        }
        Ok(result)
    }
}

/// The same proof check applies to wallet replies, reconciled results and manual submissions.
pub fn validate_payment_proof(
    invoice: &str,
    result: &PayInvoiceResult,
) -> Result<(), DurablePaymentError> {
    let parsed =
        Bolt11Invoice::from_str(invoice).map_err(|_| DurablePaymentError::InvalidInvoice)?;
    let bytes = hex::decode(&result.preimage).map_err(|_| DurablePaymentError::InvalidProof)?;
    if bytes.len() != 32 || hex::encode(Sha256::digest(bytes)) != parsed.payment_hash().to_string()
    {
        return Err(DurablePaymentError::InvalidProof);
    }
    Ok(())
}
