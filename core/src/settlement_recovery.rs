//! Read-only recovery primitives shared by automatic and manual desktop paths.
//! No invoice creation/replacement or payment dispatch exists in this module.
use crate::{
    adp_client::SettlementInvoiceResponse,
    durable_payments::{validate_payment_proof, DurableNwcPayments, PaymentWallet},
    nwc_client::PayInvoiceResult,
    settlement_repository::SettlementRepository,
    signers::NostrSigner,
};
use std::{
    collections::HashSet,
    sync::{Arc, Mutex},
};

#[derive(Default)]
struct Coordination {
    account: Option<String>,
    epoch: u64,
    running: HashSet<(String, String)>,
}
#[derive(Clone, Default)]
pub struct RecoveryCoordinator(Arc<Mutex<Coordination>>);
pub struct RecoveryLease {
    coordinator: RecoveryCoordinator,
    account: String,
    key: String,
    epoch: u64,
}
impl RecoveryCoordinator {
    /// Call when a session is replaced, including A -> B -> A transitions.
    pub fn activate(&self, account: Option<String>) {
        if let Ok(mut state) = self.0.lock() {
            state.account = account;
            state.epoch = state.epoch.wrapping_add(1);
        }
    }
    /// Observe the active account without invalidating an unchanged session.
    pub fn observe(&self, account: Option<String>) {
        if let Ok(mut state) = self.0.lock() {
            if state.account != account {
                state.account = account;
                state.epoch = state.epoch.wrapping_add(1);
            }
        }
    }
    pub fn claim(&self, account: &str, key: &str) -> Result<RecoveryLease, String> {
        let mut state = self
            .0
            .lock()
            .map_err(|_| "Recovery coordinator unavailable")?;
        if state.account.as_deref() != Some(account) {
            return Err("Account changed or signer locked; recovery paused".into());
        }
        if !state.running.insert((account.to_owned(), key.to_owned())) {
            return Err("Recovery already in progress; no payment dispatched".into());
        }
        Ok(RecoveryLease {
            coordinator: self.clone(),
            account: account.into(),
            key: key.into(),
            epoch: state.epoch,
        })
    }
}
impl RecoveryLease {
    pub fn ensure_current(&self) -> Result<(), String> {
        let state = self
            .coordinator
            .0
            .lock()
            .map_err(|_| "Recovery coordinator unavailable")?;
        if state.account.as_deref() == Some(&self.account) && state.epoch == self.epoch {
            Ok(())
        } else {
            Err("Account changed; original account recovery paused".into())
        }
    }
}
impl Drop for RecoveryLease {
    fn drop(&mut self) {
        if let Ok(mut state) = self.coordinator.0.lock() {
            state
                .running
                .remove(&(self.account.clone(), self.key.clone()));
        }
    }
}

/// Recover only the original, account-encrypted attempt. A missing wallet can
/// still submit an existing encrypted proof checkpoint; otherwise it blocks.
pub async fn existing_proof(
    repo: &SettlementRepository,
    payments: &DurableNwcPayments,
    signer: &dyn NostrSigner,
    wallet: Option<&dyn PaymentWallet>,
    quote_id: &str,
    attempt_id: &str,
) -> Result<(SettlementInvoiceResponse, String), String> {
    let account = signer
        .get_public_key()
        .await
        .map_err(|_| "Signer locked/unavailable")?;
    let stored = repo
        .invoice_attempt(&account.to_hex(), quote_id, attempt_id)
        .await
        .map_err(|_| "Invoice checkpoint unavailable")?
        .ok_or("No original invoice exists for this account/quote")?;
    let text =
        std::str::from_utf8(&stored.encrypted_attempt).map_err(|_| "Invalid invoice checkpoint")?;
    let plaintext = signer
        .nip44_decrypt(&account, text)
        .await
        .map_err(|_| "Invoice checkpoint decryption unavailable")?;
    let invoice: SettlementInvoiceResponse =
        serde_json::from_str(&plaintext).map_err(|_| "Invalid stored invoice")?;
    if invoice.quote_id != quote_id
        || invoice.attempt_id != attempt_id
        || invoice.payment_hash != stored.payment_hash
    {
        return Err("Stored invoice binding mismatch".into());
    }
    let parsed = invoice
        .bolt11
        .parse::<lightning_invoice::Bolt11Invoice>()
        .map_err(|_| "Invalid stored BOLT-11")?;
    if parsed.payment_hash().to_string() != stored.payment_hash
        || parsed.amount_milli_satoshis() != Some(invoice.amount_msat)
    {
        return Err("Original invoice amount/hash mismatch".into());
    }
    let proof = if let Some(encrypted) = stored.encrypted_proof {
        signer
            .nip44_decrypt(
                &account,
                std::str::from_utf8(&encrypted).map_err(|_| "Invalid proof checkpoint")?,
            )
            .await
            .map_err(|_| "Proof checkpoint locked")?
    } else {
        payments
            .reconcile(
                signer,
                wallet
                    .ok_or("Wallet unavailable; original intent retained, no payment attempted")?,
                &invoice.bolt11,
            )
            .await
            .map_err(|_| {
                "Payment outcome uncertain; original intent retained, no payment attempted"
            })?
            .preimage
    };
    validate_payment_proof(
        &invoice.bolt11,
        &PayInvoiceResult {
            preimage: proof.clone(),
            fees_paid_msat: None,
        },
    )
    .map_err(|_| "Original payment proof is invalid")?;
    let encrypted = signer
        .nip44_encrypt(&account, &proof)
        .await
        .map_err(|_| "Proof checkpoint encryption unavailable")?;
    if !repo
        .checkpoint_proof(
            &account.to_hex(),
            quote_id,
            attempt_id,
            encrypted.as_bytes(),
        )
        .await
        .map_err(|_| "Proof checkpoint unavailable")?
    {
        return Err("Original attempt disappeared".into());
    }
    Ok((invoice, proof))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn concurrent_recovery_and_account_roundtrip_are_fenced_without_async_mutex_guards() {
        let c = RecoveryCoordinator::default();
        c.activate(Some("alice".into()));
        let first = c.claim("alice", "quote").expect("claim");
        assert!(c.claim("alice", "quote").is_err());
        c.activate(Some("bob".into()));
        assert!(first.ensure_current().is_err());
        assert!(c.claim("alice", "other").is_err());
        c.activate(Some("alice".into()));
        assert!(first.ensure_current().is_err());
        drop(first);
        assert!(c
            .claim("alice", "quote")
            .expect("new session")
            .ensure_current()
            .is_ok());
        c.activate(None);
        assert!(c.claim("alice", "quote").is_err());
    }
}
