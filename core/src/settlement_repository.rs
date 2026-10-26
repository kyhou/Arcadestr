//! Account-scoped persistence for resumable split purchases.
use sqlx::{Row, SqlitePool};
use thiserror::Error;

use crate::adp_settlement::{PaymentLeg, QuoteState, SignedQuote};

#[derive(Debug, Error)]
pub enum SettlementRepositoryError {
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),
    #[error("numeric value is outside SQLite range")]
    NumericRange,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredSettlement {
    pub account_pubkey: String,
    pub quote_id: String,
    pub processor_url: String,
    pub quote_event_json: String,
    pub listing_event_id: String,
    pub state: String,
    pub failure_at: Option<u64>,
    pub receipt_event_json: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredRecoveryObligation {
    pub quote_id: String,
    pub leg_id: String,
    pub state: String,
    pub principal_msat: u64,
    pub due_at: u64,
}

#[derive(Clone)]
pub struct SettlementRepository {
    pool: SqlitePool,
}

impl SettlementRepository {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn save_quote(
        &self,
        account_pubkey: &str,
        processor_url: &str,
        quote_event_json: &str,
        quote: &SignedQuote,
        updated_at: u64,
    ) -> Result<(), SettlementRepositoryError> {
        let mut tx = self.pool.begin().await?;
        sqlx::query("INSERT INTO settlement_quotes (account_pubkey, quote_id, processor_url, quote_event_json, listing_event_id, state, failure_at, updated_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?) ON CONFLICT(account_pubkey, quote_id) DO UPDATE SET quote_event_json=excluded.quote_event_json, state=excluded.state, failure_at=excluded.failure_at, updated_at=excluded.updated_at")
            .bind(account_pubkey).bind(&quote.quote_id).bind(processor_url).bind(quote_event_json)
            .bind(&quote.listing_event_id).bind(state_text(quote.state)).bind(to_i64_opt(quote.failure_at)?).bind(to_i64(updated_at)?)
            .execute(&mut *tx).await?;
        for (order, leg) in quote.legs.iter().enumerate() {
            save_leg(&mut tx, account_pubkey, &quote.quote_id, order, leg).await?;
        }
        tx.commit().await?;
        Ok(())
    }

    pub async fn load(
        &self,
        account_pubkey: &str,
        quote_id: &str,
    ) -> Result<Option<StoredSettlement>, SettlementRepositoryError> {
        let row = sqlx::query("SELECT processor_url, quote_event_json, listing_event_id, state, failure_at, receipt_event_json FROM settlement_quotes WHERE account_pubkey=? AND quote_id=?")
            .bind(account_pubkey).bind(quote_id).fetch_optional(&self.pool).await?;
        row.map(|row| {
            Ok(StoredSettlement {
                account_pubkey: account_pubkey.to_string(),
                quote_id: quote_id.to_string(),
                processor_url: row.try_get("processor_url")?,
                quote_event_json: row.try_get("quote_event_json")?,
                listing_event_id: row.try_get("listing_event_id")?,
                state: row.try_get("state")?,
                failure_at: row
                    .try_get::<Option<i64>, _>("failure_at")?
                    .map(|v| u64::try_from(v).map_err(|_| SettlementRepositoryError::NumericRange))
                    .transpose()?,
                receipt_event_json: row.try_get("receipt_event_json")?,
            })
        })
        .transpose()
    }

    pub async fn record_invoice_attempt(
        &self,
        account_pubkey: &str,
        quote_id: &str,
        leg_id: &str,
        attempt_id: &str,
        encrypted_attempt: &[u8],
        payment_hash: &str,
        state: &str,
        updated_at: u64,
    ) -> Result<(), SettlementRepositoryError> {
        sqlx::query("INSERT INTO settlement_invoice_attempts (account_pubkey, quote_id, leg_id, attempt_id, encrypted_attempt, payment_hash, state, updated_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?) ON CONFLICT(account_pubkey, attempt_id) DO UPDATE SET encrypted_attempt=excluded.encrypted_attempt, state=excluded.state, updated_at=excluded.updated_at")
            .bind(account_pubkey).bind(quote_id).bind(leg_id).bind(attempt_id).bind(encrypted_attempt).bind(payment_hash).bind(state).bind(to_i64(updated_at)?)
            .execute(&self.pool).await?;
        Ok(())
    }

    pub async fn mark_receipt(
        &self,
        account_pubkey: &str,
        quote_id: &str,
        receipt_event_json: &str,
        updated_at: u64,
    ) -> Result<bool, SettlementRepositoryError> {
        let result = sqlx::query("UPDATE settlement_quotes SET state='settled', receipt_event_json=?, updated_at=? WHERE account_pubkey=? AND quote_id=? AND state='active'")
            .bind(receipt_event_json).bind(to_i64(updated_at)?).bind(account_pubkey).bind(quote_id).execute(&self.pool).await?;
        Ok(result.rows_affected() == 1)
    }

    pub async fn update_status(
        &self,
        account_pubkey: &str,
        quote_id: &str,
        state: QuoteState,
        failure_at: Option<u64>,
        updated_at: u64,
    ) -> Result<bool, SettlementRepositoryError> {
        let result = sqlx::query("UPDATE settlement_quotes SET state=?, failure_at=?, updated_at=? WHERE account_pubkey=? AND quote_id=?")
            .bind(state_text(state)).bind(to_i64_opt(failure_at)?).bind(to_i64(updated_at)?)
            .bind(account_pubkey).bind(quote_id).execute(&self.pool).await?;
        Ok(result.rows_affected() == 1)
    }

    pub async fn update_invoice_state(
        &self,
        account_pubkey: &str,
        attempt_id: &str,
        state: &str,
        updated_at: u64,
    ) -> Result<bool, SettlementRepositoryError> {
        let result = sqlx::query("UPDATE settlement_invoice_attempts SET state=?, updated_at=? WHERE account_pubkey=? AND attempt_id=?")
            .bind(state).bind(to_i64(updated_at)?).bind(account_pubkey).bind(attempt_id)
            .execute(&self.pool).await?;
        Ok(result.rows_affected() == 1)
    }

    pub async fn list_for_account(
        &self,
        account_pubkey: &str,
    ) -> Result<Vec<StoredSettlement>, SettlementRepositoryError> {
        let rows = sqlx::query("SELECT quote_id, processor_url, quote_event_json, listing_event_id, state, failure_at, receipt_event_json FROM settlement_quotes WHERE account_pubkey=? ORDER BY updated_at DESC")
            .bind(account_pubkey).fetch_all(&self.pool).await?;
        rows.into_iter()
            .map(|row| {
                Ok(StoredSettlement {
                    account_pubkey: account_pubkey.to_string(),
                    quote_id: row.try_get("quote_id")?,
                    processor_url: row.try_get("processor_url")?,
                    quote_event_json: row.try_get("quote_event_json")?,
                    listing_event_id: row.try_get("listing_event_id")?,
                    state: row.try_get("state")?,
                    failure_at: row
                        .try_get::<Option<i64>, _>("failure_at")?
                        .map(|value| {
                            u64::try_from(value)
                                .map_err(|_| SettlementRepositoryError::NumericRange)
                        })
                        .transpose()?,
                    receipt_event_json: row.try_get("receipt_event_json")?,
                })
            })
            .collect()
    }

    pub async fn list_recovery_for_quote(
        &self,
        account_pubkey: &str,
        quote_id: &str,
    ) -> Result<Vec<StoredRecoveryObligation>, SettlementRepositoryError> {
        let rows = sqlx::query(
            "SELECT quote_id, leg_id, state, principal_msat, due_at \
             FROM settlement_recovery WHERE account_pubkey=? AND quote_id=? \
             ORDER BY due_at, leg_id",
        )
        .bind(account_pubkey)
        .bind(quote_id)
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(|row| {
                Ok(StoredRecoveryObligation {
                    quote_id: row.try_get("quote_id")?,
                    leg_id: row.try_get("leg_id")?,
                    state: row.try_get("state")?,
                    principal_msat: u64::try_from(row.try_get::<i64, _>("principal_msat")?)
                        .map_err(|_| SettlementRepositoryError::NumericRange)?,
                    due_at: u64::try_from(row.try_get::<i64, _>("due_at")?)
                        .map_err(|_| SettlementRepositoryError::NumericRange)?,
                })
            })
            .collect()
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn save_recovery_obligation(
        &self,
        account_pubkey: &str,
        quote_id: &str,
        leg_id: &str,
        principal_msat: u64,
        due_at: u64,
        evidence_event_json: Option<&str>,
        state: &str,
    ) -> Result<(), SettlementRepositoryError> {
        sqlx::query("INSERT INTO settlement_recovery (account_pubkey, quote_id, leg_id, state, principal_msat, due_at, evidence_event_json) VALUES (?, ?, ?, ?, ?, ?, ?) ON CONFLICT(account_pubkey, quote_id, leg_id) DO UPDATE SET state=excluded.state, evidence_event_json=COALESCE(excluded.evidence_event_json, settlement_recovery.evidence_event_json)")
            .bind(account_pubkey).bind(quote_id).bind(leg_id).bind(state)
            .bind(to_i64(principal_msat)?).bind(to_i64(due_at)?).bind(evidence_event_json)
            .execute(&self.pool).await?;
        Ok(())
    }

    pub async fn update_recovery_state(
        &self,
        account_pubkey: &str,
        quote_id: &str,
        leg_id: &str,
        state: &str,
    ) -> Result<bool, SettlementRepositoryError> {
        let result = sqlx::query("UPDATE settlement_recovery SET state=? WHERE account_pubkey=? AND quote_id=? AND leg_id=?")
            .bind(state).bind(account_pubkey).bind(quote_id).bind(leg_id)
            .execute(&self.pool).await?;
        Ok(result.rows_affected() == 1)
    }
}

/// Side effects are permitted only while the initiating account remains active.
pub fn account_can_continue(initiating_account: &str, active_account: Option<&str>) -> bool {
    active_account.is_some_and(|active| active == initiating_account)
}

async fn save_leg(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    account: &str,
    quote_id: &str,
    order: usize,
    leg: &PaymentLeg,
) -> Result<(), SettlementRepositoryError> {
    sqlx::query("INSERT INTO settlement_legs (account_pubkey, quote_id, leg_id, leg_order, recipient_pubkey, amount_msat, state) VALUES (?, ?, ?, ?, ?, ?, 'pending') ON CONFLICT(account_pubkey, quote_id, leg_id) DO UPDATE SET leg_order=excluded.leg_order, recipient_pubkey=excluded.recipient_pubkey, amount_msat=excluded.amount_msat")
        .bind(account).bind(quote_id).bind(&leg.leg_id).bind(i64::try_from(order).map_err(|_| SettlementRepositoryError::NumericRange)?).bind(&leg.recipient_pubkey).bind(to_i64(leg.amount_msat)?).execute(&mut **tx).await?;
    Ok(())
}

fn state_text(state: QuoteState) -> &'static str {
    match state {
        QuoteState::Draft => "draft",
        QuoteState::Preflighting => "preflighting",
        QuoteState::Active => "active",
        QuoteState::Failed => "failed",
        QuoteState::Settled => "settled",
    }
}
fn to_i64(value: u64) -> Result<i64, SettlementRepositoryError> {
    i64::try_from(value).map_err(|_| SettlementRepositoryError::NumericRange)
}
fn to_i64_opt(value: Option<u64>) -> Result<Option<i64>, SettlementRepositoryError> {
    value.map(to_i64).transpose()
}
