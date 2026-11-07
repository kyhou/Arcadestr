//! Account-scoped desktop recovery. Startup/manual reconciliation never pays.
use crate::{
    adp_commands::{ResumeSplitPaymentRequest, SplitRefundRequest},
    AppState,
};
use arcadestr_core::{
    adp_client::{AdpClient, RefundInvoiceResponse},
    durable_payments::DurableNwcPayments,
    nwc_client::{load_nwc_connection, NwcClient},
    settlement_recovery::RecoveryLease,
    settlement_repository::SettlementRepository,
    signers::NostrSigner,
};
use serde::{Deserialize, Serialize};
use sqlx::Row;
use tauri::{AppHandle, Emitter, Manager, State};

fn now() -> Result<i64, String> {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| "Invalid system time".to_owned())
        .and_then(|d| i64::try_from(d.as_secs()).map_err(|_| "Invalid system time".into()))
}
pub(crate) async fn claim(
    state: &AppState,
    account: nostr::PublicKey,
    key: &str,
) -> Result<RecoveryLease, String> {
    let auth = state.auth.lock().await;
    if auth.public_key() != Some(account) || auth.signer().is_none() {
        return Err("Account changed or signer unavailable".into());
    }
    state.settlement_recovery.observe(Some(account.to_hex()));
    state.settlement_recovery.claim(&account.to_hex(), key)
}
async fn progress(
    state: &AppState,
    account: &str,
    server: &str,
    quote: &str,
    attempt: &str,
    status: &str,
    message: &str,
) -> Result<(), String> {
    sqlx::query("INSERT INTO client_settlement_progress (account_pubkey,processor_url,quote_id,attempt_id,state,message,updated_at) VALUES (?,?,?,?,?,?,?) ON CONFLICT(account_pubkey,processor_url,quote_id,attempt_id) DO UPDATE SET state=excluded.state,message=excluded.message,updated_at=excluded.updated_at")
        .bind(account).bind(server).bind(quote).bind(attempt).bind(status).bind(message).bind(now()?).execute(state.database.pool()).await.map_err(|_|"Recovery progress could not be persisted")?;
    Ok(())
}

#[derive(Clone, Serialize, Deserialize, Default)]
pub struct RecoveryReport {
    pub recovered: u64,
    pub blocked: u64,
    pub skipped: u64,
    pub message: String,
}

#[tauri::command]
pub async fn reconcile_pending_settlements(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<RecoveryReport, String> {
    let auth = { state.auth.lock().await.clone() };
    let account = auth
        .public_key()
        .ok_or("Connect/unlock an account to reconcile existing payments")?;
    let scan = claim(&state, account, "account-recovery-scan").await?;
    let repository = SettlementRepository::new(state.database.pool().clone());
    let attempts = repository
        .recovery_attempts(&account.to_hex())
        .await
        .map_err(|_| "Stored recovery queue unavailable")?;
    let mut report = RecoveryReport::default();
    for attempt in attempts {
        scan.ensure_current()?;
        progress(
            &state,
            &account.to_hex(),
            &attempt.processor_url,
            &attempt.quote_id,
            &attempt.attempt_id,
            "recovering",
            "Checking the original payment; no payment will be sent",
        )
        .await?;
        let result = crate::adp_commands::resume_split_payment_for_account(
            ResumeSplitPaymentRequest {
                server_url: attempt.processor_url.clone(),
                quote_id: attempt.quote_id.clone(),
                attempt_id: attempt.attempt_id.clone(),
            },
            app.state(),
            Some(account),
        )
        .await;
        scan.ensure_current()?;
        match result {
            Ok(response) => {
                report.recovered += 1;
                progress(
                    &state,
                    &account.to_hex(),
                    &attempt.processor_url,
                    &attempt.quote_id,
                    &attempt.attempt_id,
                    &response.state,
                    "Original proof/receipt recovered without payment dispatch",
                )
                .await?;
            }
            Err(_) => {
                report.blocked += 1;
                progress(&state,&account.to_hex(),&attempt.processor_url,&attempt.quote_id,&attempt.attempt_id,"uncertain","Recovery paused: wallet, signer, relay or payment outcome unavailable. Original intent retained; no payment sent").await?;
            }
        }
    }
    let rows=sqlx::query("SELECT processor_url,quote_id,leg_id FROM client_refund_attempts r WHERE account_pubkey=? AND state!='verified' AND (encrypted_proof IS NOT NULL OR EXISTS(SELECT 1 FROM nwc_payment_journal p WHERE p.account_pubkey=r.account_pubkey AND p.payment_hash=r.payment_hash))")
        .bind(account.to_hex()).fetch_all(state.database.pool()).await.map_err(|_|"Refund recovery queue unavailable")?;
    for row in rows {
        scan.ensure_current()?;
        let request = RefundPaymentRequest {
            server_url: row
                .try_get("processor_url")
                .map_err(|_| "Invalid refund record")?,
            quote_id: row
                .try_get("quote_id")
                .map_err(|_| "Invalid refund record")?,
            leg_id: row.try_get("leg_id").map_err(|_| "Invalid refund record")?,
            confirm_payment: false,
        };
        match refund_operation(request, app.state(), false, Some(account)).await {
            Ok(_) => report.recovered += 1,
            Err(_) => report.blocked += 1,
        };
    }
    report.message = if report.blocked > 0 {
        "Some original payments remain unresolved. No payment was dispatched."
    } else {
        "Recovery check complete. No payment was dispatched."
    }
    .into();
    let _=app.emit("settlement_recovery_updated",serde_json::json!({"account":account.to_hex(),"recovered":report.recovered,"blocked":report.blocked}));
    Ok(report)
}

pub fn spawn_startup_recovery(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        // Restore the already-selected account once, never create/select a new one.
        if app.state::<AppState>().auth.lock().await.signer().is_none() {
            let _ =
                crate::nip46_commands::load_active_account(app.state(), app.state(), app.state())
                    .await;
        }
        loop {
            let state = app.state::<AppState>();
            let active = {
                let auth = state.auth.lock().await;
                auth.public_key().filter(|_| auth.signer().is_some())
            };
            state
                .settlement_recovery
                .observe(active.map(|p| p.to_hex()));
            if active.is_some() {
                let _ = reconcile_pending_settlements(app.clone(), app.state()).await;
            }
            tokio::time::sleep(std::time::Duration::from_secs(30)).await;
        }
    });
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RefundHistoryRequest {
    pub server_url: String,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct RefundHistoryItem {
    pub quote_id: String,
    pub leg_id: String,
    pub recipient_pubkey: String,
    pub principal_msat: u64,
    pub due_at: u64,
    pub state: String,
    pub attempt_id: Option<String>,
    pub invoice: Option<String>,
    pub payment_state: String,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct RefundPaymentRequest {
    pub server_url: String,
    pub quote_id: String,
    pub leg_id: String,
    #[serde(default)]
    pub confirm_payment: bool,
}

fn obligations_read_error(error: arcadestr_core::adp_client::AdpClientError) -> &'static str {
    use arcadestr_core::{
        adp_client::AdpClientError, nip98_client::Nip98ClientError, signers::SignerError,
    };
    match error {
        AdpClientError::Auth(Nip98ClientError::Signer(SignerError::AuthorizationDenied)) =>
            "Remote signer denied authorization. Grant permission in the signer and retry; no payment was sent.",
        AdpClientError::Auth(Nip98ClientError::Signer(SignerError::ConnectionTimeout)) =>
            "Remote signer timed out. Check its connection and retry; no payment was sent.",
        _ => "Authenticated recipient obligations unavailable",
    }
}

#[cfg(test)]
mod denial_tests {
    #[test]
    fn authenticated_read_preserves_refusal_without_leaking_backend_details() {
        use arcadestr_core::{
            adp_client::AdpClientError, nip98_client::Nip98ClientError, signers::SignerError,
        };
        for (response, expected) in [
            ("denied: rejected", "denied authorization"),
            ("timeout", "timed out"),
        ] {
            let signer = SignerError::from(nostr::signer::SignerError::from(response));
            let error = AdpClientError::Auth(Nip98ClientError::Signer(signer));
            let message = super::obligations_read_error(error);
            assert!(message.contains(expected));
            assert!(message.contains("no payment was sent"));
        }
        let error = AdpClientError::Auth(Nip98ClientError::Signer(SignerError::SigningFailed(
            "untrusted signer error containing private payload".into(),
        )));
        assert_eq!(
            super::obligations_read_error(error),
            "Authenticated recipient obligations unavailable"
        );
    }
}

#[tauri::command]
pub async fn recipient_refund_history(
    request: RefundHistoryRequest,
    state: State<'_, AppState>,
) -> Result<Vec<RefundHistoryItem>, String> {
    let auth = { state.auth.lock().await.clone() };
    let signer = auth
        .signer()
        .ok_or("Connect/unlock the obligated recipient account")?;
    let account = signer
        .get_public_key()
        .await
        .map_err(|_| "Signer unavailable")?;
    let client = AdpClient::new(request.server_url.clone(), state.http_client.clone());
    let response = client
        .refund_obligations(signer, None)
        .await
        .map_err(obligations_read_error)?;
    let mut history = vec![];
    for row in response.obligations {
        if row.recipient_pubkey != account.to_hex() {
            return Err("Processor returned an obligation for another recipient".into());
        }
        let stored=sqlx::query("SELECT attempt_id,encrypted_invoice,encrypted_proof,EXISTS(SELECT 1 FROM nwc_payment_journal p WHERE p.account_pubkey=r.account_pubkey AND p.payment_hash=r.payment_hash) AS has_intent FROM client_refund_attempts r WHERE account_pubkey=? AND processor_url=? AND quote_id=? AND leg_id=?")
            .bind(account.to_hex()).bind(&request.server_url).bind(&row.quote_id).bind(&row.leg_id).fetch_optional(state.database.pool()).await.map_err(|_|"Local refund history unavailable")?;
        let (attempt, invoice, payment_state) = if let Some(stored) = stored {
            let ciphertext: Vec<u8> = stored
                .try_get("encrypted_invoice")
                .map_err(|_| "Invalid invoice checkpoint")?;
            let plaintext = signer
                .nip44_decrypt(
                    &account,
                    std::str::from_utf8(&ciphertext).map_err(|_| "Invalid invoice checkpoint")?,
                )
                .await
                .map_err(|_| "Refund invoice locked")?;
            let original: RefundInvoiceResponse =
                serde_json::from_str(&plaintext).map_err(|_| "Invalid stored refund")?;
            let proof: Option<Vec<u8>> = stored
                .try_get("encrypted_proof")
                .map_err(|_| "Invalid refund checkpoint")?;
            let intent: bool = stored
                .try_get("has_intent")
                .map_err(|_| "Invalid refund checkpoint")?;
            (
                Some(original.attempt_id),
                Some(original.bolt11),
                if row.state == "verified" {
                    "verified"
                } else if proof.is_some() {
                    "proof_checkpointed"
                } else if intent {
                    "uncertain"
                } else {
                    "awaiting_payment"
                }
                .to_owned(),
            )
        } else {
            (
                None,
                None,
                if row.state == "verified" {
                    "verified"
                } else {
                    "not_prepared"
                }
                .into(),
            )
        };
        history.push(RefundHistoryItem {
            quote_id: row.quote_id,
            leg_id: row.leg_id,
            recipient_pubkey: row.recipient_pubkey,
            principal_msat: row.principal_msat,
            due_at: row.due_at,
            state: row.state,
            attempt_id: attempt,
            invoice,
            payment_state,
        });
    }
    if state.auth.lock().await.public_key() != Some(account) {
        return Err("Account changed".into());
    }
    Ok(history)
}

pub(crate) async fn store_prepared(
    state: &AppState,
    signer: &dyn NostrSigner,
    account: nostr::PublicKey,
    request: &SplitRefundRequest,
    invoice: &RefundInvoiceResponse,
) -> Result<(), String> {
    let client = AdpClient::new(request.server_url.clone(), state.http_client.clone());
    let obligation = client
        .refund_obligations(signer, None)
        .await
        .map_err(|_| "Recipient obligation unavailable")?
        .obligations
        .into_iter()
        .find(|r| {
            r.quote_id == request.quote_id
                && r.leg_id == request.leg_id
                && r.recipient_pubkey == account.to_hex()
        })
        .ok_or("This account is not the obligated recipient")?;
    if obligation.principal_msat != invoice.amount_msat
        || invoice.leg_id != request.leg_id
        || obligation.state == "verified"
    {
        return Err("Refund invoice differs from original principal obligation".into());
    }
    let parsed = invoice
        .bolt11
        .parse::<lightning_invoice::Bolt11Invoice>()
        .map_err(|_| "Invalid invoice")?;
    let encrypted = signer
        .nip44_encrypt(
            &account,
            &serde_json::to_string(invoice).map_err(|_| "Invoice serialization failed")?,
        )
        .await
        .map_err(|_| "Refund invoice encryption unavailable")?;
    let result=sqlx::query("INSERT INTO client_refund_attempts (account_pubkey,processor_url,quote_id,leg_id,attempt_id,payment_hash,amount_msat,due_at,encrypted_invoice,state,updated_at) VALUES (?,?,?,?,?,?,?,?,?,'prepared',?) ON CONFLICT(account_pubkey,processor_url,quote_id,leg_id) DO UPDATE SET updated_at=excluded.updated_at WHERE client_refund_attempts.attempt_id=excluded.attempt_id AND client_refund_attempts.payment_hash=excluded.payment_hash AND client_refund_attempts.amount_msat=excluded.amount_msat")
        .bind(account.to_hex()).bind(&request.server_url).bind(&request.quote_id).bind(&request.leg_id).bind(&invoice.attempt_id).bind(parsed.payment_hash().to_string()).bind(i64::try_from(invoice.amount_msat).map_err(|_|"Invalid amount")?).bind(i64::try_from(obligation.due_at).map_err(|_|"Invalid deadline")?).bind(encrypted.as_bytes()).bind(now()?).execute(state.database.pool()).await.map_err(|_|"Original refund invoice could not be persisted")?;
    if result.rows_affected() != 1 {
        return Err("Refund attempt changed. Original invoice retained; do not pay again".into());
    }
    Ok(())
}

#[tauri::command]
pub async fn pay_split_refund(
    request: RefundPaymentRequest,
    state: State<'_, AppState>,
) -> Result<serde_json::Value, String> {
    if !request.confirm_payment {
        return Err("Explicit confirmation is required before dispatching a refund payment".into());
    }
    refund_operation(request, state, true, None).await
}
#[tauri::command]
pub async fn resume_split_refund(
    request: RefundPaymentRequest,
    state: State<'_, AppState>,
) -> Result<serde_json::Value, String> {
    refund_operation(request, state, false, None).await
}

async fn refund_operation(
    request: RefundPaymentRequest,
    state: State<'_, AppState>,
    dispatch: bool,
    expected: Option<nostr::PublicKey>,
) -> Result<serde_json::Value, String> {
    let auth = { state.auth.lock().await.clone() };
    let signer = auth.signer().ok_or("Recipient signer locked")?;
    let account = signer
        .get_public_key()
        .await
        .map_err(|_| "Signer unavailable")?;
    if expected.is_some_and(|p| p != account) {
        return Err("Account changed".into());
    }
    let lease = claim(
        &state,
        account,
        &format!(
            "refund:{}:{}:{}",
            request.server_url, request.quote_id, request.leg_id
        ),
    )
    .await?;
    let row=sqlx::query("SELECT attempt_id,payment_hash,amount_msat,encrypted_invoice,encrypted_proof,state FROM client_refund_attempts WHERE account_pubkey=? AND processor_url=? AND quote_id=? AND leg_id=?")
        .bind(account.to_hex()).bind(&request.server_url).bind(&request.quote_id).bind(&request.leg_id).fetch_optional(state.database.pool()).await.map_err(|_|"Refund storage unavailable")?.ok_or("Prepare the original refund invoice with the obligated account first")?;
    let local_state: String = row.try_get("state").map_err(|_| "Invalid refund record")?;
    if local_state == "verified" {
        return Ok(serde_json::json!({"state":"verified"}));
    }
    let encrypted: Vec<u8> = row
        .try_get("encrypted_invoice")
        .map_err(|_| "Invalid refund invoice")?;
    let plaintext = signer
        .nip44_decrypt(
            &account,
            std::str::from_utf8(&encrypted).map_err(|_| "Invalid refund invoice")?,
        )
        .await
        .map_err(|_| "Refund invoice locked")?;
    let invoice: RefundInvoiceResponse =
        serde_json::from_str(&plaintext).map_err(|_| "Refund invoice invalid")?;
    let hash: String = row
        .try_get("payment_hash")
        .map_err(|_| "Invalid refund record")?;
    let amount: i64 = row
        .try_get("amount_msat")
        .map_err(|_| "Invalid refund record")?;
    let parsed = invoice
        .bolt11
        .parse::<lightning_invoice::Bolt11Invoice>()
        .map_err(|_| "Invalid BOLT-11")?;
    if parsed.payment_hash().to_string() != hash
        || parsed.amount_milli_satoshis() != u64::try_from(amount).ok()
        || invoice.leg_id != request.leg_id
        || invoice.attempt_id
            != row
                .try_get::<String, _>("attempt_id")
                .map_err(|_| "Invalid attempt")?
    {
        return Err("Original refund binding mismatch".into());
    }
    let client = AdpClient::new(request.server_url.clone(), state.http_client.clone());
    let obligation = client
        .refund_obligations(signer, None)
        .await
        .map_err(|_| "Recipient authentication/status unavailable")?
        .obligations
        .into_iter()
        .find(|r| {
            r.quote_id == request.quote_id
                && r.leg_id == request.leg_id
                && r.recipient_pubkey == account.to_hex()
        })
        .ok_or("Active identity is not the obligated recipient")?;
    if obligation.principal_msat != invoice.amount_msat {
        return Err("Original refund principal changed".into());
    }
    lease.ensure_current()?;
    // An authenticated processor acknowledgement dominates stale local UI.
    // Never pay an obligation that was already fulfilled from another session.
    if obligation.state == "verified" {
        sqlx::query("UPDATE client_refund_attempts SET state='verified',updated_at=? WHERE account_pubkey=? AND processor_url=? AND quote_id=? AND leg_id=?")
            .bind(now()?).bind(account.to_hex()).bind(&request.server_url).bind(&request.quote_id).bind(&request.leg_id)
            .execute(state.database.pool()).await.map_err(|_|"Refund state persistence unavailable")?;
        return Ok(serde_json::json!({"state":"verified"}));
    }
    let checkpoint: Option<Vec<u8>> = row
        .try_get("encrypted_proof")
        .map_err(|_| "Invalid proof checkpoint")?;
    let proof = if let Some(encrypted) = checkpoint {
        signer
            .nip44_decrypt(
                &account,
                std::str::from_utf8(&encrypted).map_err(|_| "Invalid proof checkpoint")?,
            )
            .await
            .map_err(|_| "Refund proof locked")?
    } else {
        let connection=load_nwc_connection(&account).map_err(|_|"Obligated recipient wallet unavailable. No payment was sent; reconnect that account's original wallet")?;
        let wallet = NwcClient::new(connection);
        let payments = DurableNwcPayments::new(state.database.pool().clone());
        let result = if dispatch {
            payments
                .pay_or_resume_guarded(signer, &wallet, &invoice.bolt11, &|| lease.ensure_current())
                .await
        } else {
            payments.reconcile(signer, &wallet, &invoice.bolt11).await
        };
        match result {
            Ok(result) => result.preimage,
            Err(_) => {
                sqlx::query("UPDATE client_refund_attempts SET state='uncertain',updated_at=? WHERE account_pubkey=? AND processor_url=? AND quote_id=? AND leg_id=?").bind(now()?).bind(account.to_hex()).bind(&request.server_url).bind(&request.quote_id).bind(&request.leg_id).execute(state.database.pool()).await.map_err(|_|"Refund progress unavailable")?;
                return Err("Refund outcome unresolved. Original intent retained; use read-only recovery, never repay".into());
            }
        }
    };
    arcadestr_core::durable_payments::validate_payment_proof(
        &invoice.bolt11,
        &arcadestr_core::nwc_client::PayInvoiceResult {
            preimage: proof.clone(),
            fees_paid_msat: None,
        },
    )
    .map_err(|_| "Invalid original refund proof")?;
    let encrypted = signer
        .nip44_encrypt(&account, &proof)
        .await
        .map_err(|_| "Refund proof encryption unavailable")?;
    sqlx::query("UPDATE client_refund_attempts SET encrypted_proof=COALESCE(encrypted_proof,?),state='confirming',updated_at=? WHERE account_pubkey=? AND processor_url=? AND quote_id=? AND leg_id=?")
        .bind(encrypted.as_bytes()).bind(now()?).bind(account.to_hex()).bind(&request.server_url).bind(&request.quote_id).bind(&request.leg_id).execute(state.database.pool()).await.map_err(|_|"Refund proof persistence unavailable")?;
    lease.ensure_current()?;
    let response = client
        .confirm_refund(
            signer,
            &request.quote_id,
            &request.leg_id,
            &invoice.attempt_id,
            &proof,
        )
        .await
        .map_err(|_| {
            "Original refund proof saved; confirmation unavailable. Recover without paying again"
        })?;
    lease.ensure_current()?;
    if response["state"] != "verified" {
        return Err("Processor has not verified the refund".into());
    }
    sqlx::query("UPDATE client_refund_attempts SET state='verified',updated_at=? WHERE account_pubkey=? AND processor_url=? AND quote_id=? AND leg_id=?")
        .bind(now()?).bind(account.to_hex()).bind(&request.server_url).bind(&request.quote_id).bind(&request.leg_id).execute(state.database.pool()).await.map_err(|_|"Refund state persistence unavailable")?;
    Ok(response)
}
