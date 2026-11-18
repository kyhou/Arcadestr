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
            expected_attempt_id: None,
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
    pub payment_state: String,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RefundPaymentRequest {
    pub server_url: String,
    pub quote_id: String,
    pub leg_id: String,
    #[serde(default)]
    pub confirm_payment: bool,
    pub expected_attempt_id: Option<String>,
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
mod payment_gate_tests {
    /// Body of `pub async fn <name>` up to the next command, from production source.
    fn command_body<'a>(source: &'a str, name: &str) -> &'a str {
        let start = source
            .find(&format!("pub async fn {name}("))
            .unwrap_or_else(|| panic!("{name} exists"));
        let rest = &source[start..];
        let end = rest[1..]
            .find("#[tauri::command]")
            .map_or(rest.len(), |i| i + 1);
        &rest[..end]
    }

    const GATE: &str = "require_live_purchase_authorization()?";

    #[test]
    fn every_live_dispatch_entry_point_is_behind_the_live_execution_gate() {
        let adp = include_str!("adp_commands.rs");
        for command in [
            "pay_purchase_leg",
            "start_split_purchase",
            "next_split_invoice",
        ] {
            let body = command_body(adp, command);
            assert!(body.contains(GATE), "{command} must check the live gate");
        }
        let refunds = include_str!("settlement_ui.rs");
        let pay = command_body(refunds, "pay_split_refund");
        let gate = pay
            .find("require_live_refund_authorization()?")
            .expect("refund dispatch is gated by the separate refund activation");
        let dispatch = pay
            .find("refund_operation(request, state, true")
            .expect("dispatch call");
        assert!(gate < dispatch, "gate precedes refund dispatch");
        // The full purpose-specific boundary also runs right before each send.
        let operation = &refunds[refunds
            .find("async fn refund_operation(")
            .expect("operation")..];
        let boundary = operation
            .find("refund_dispatch_boundary(")
            .expect("refund boundary");
        let send = operation.find(".pay_or_resume_guarded(").expect("send");
        assert!(boundary < send, "refund boundary precedes the send");
        let pay_nwc = command_body(adp, "pay_purchase_leg");
        let boundary = pay_nwc
            .find("buyer_dispatch_boundary(")
            .expect("buyer boundary");
        let send = pay_nwc.find(".pay_leg_guarded(").expect("buyer send");
        assert!(boundary < send, "buyer boundary precedes the send");
    }

    #[test]
    fn read_only_refund_reconciliation_never_dispatches() {
        let refunds = include_str!("settlement_ui.rs");
        let resume = command_body(refunds, "resume_split_refund");
        assert!(resume.contains("refund_operation(request, state, false"));
        assert!(!resume.contains("refund_operation(request, state, true"));
    }

    #[test]
    fn managed_ipc_rejects_raw_inputs_and_cannot_export_invoices() {
        use crate::adp_commands::{
            PayPurchaseLegRequest, SplitInvoiceResponse, SplitRefundInvoiceResponse,
        };
        use serde_json::{json, to_value};

        let mut request = json!({"expected_account_pubkey": "account", "quote_id": "quote", "attempt_id": "attempt"});
        assert!(serde_json::from_value::<PayPurchaseLegRequest>(request.clone()).is_ok());
        request["bolt11"] = json!("externally obtained invoice");
        assert!(serde_json::from_value::<PayPurchaseLegRequest>(request).is_err());
        assert!(serde_json::from_value::<PayPurchaseLegRequest>(json!({
            "expected_account_pubkey": "account", "bolt11": "legacy invoice"
        }))
        .is_err());

        let buyer = to_value(SplitInvoiceResponse {
            quote_id: "quote".into(),
            attempt_id: "attempt".into(),
            leg_id: "leg".into(),
            amount_msat: 1,
            expires_at: 2,
        })
        .unwrap();
        assert_eq!(
            buyer,
            json!({"quote_id":"quote","attempt_id":"attempt","leg_id":"leg","amount_msat":1,"expires_at":2})
        );
        let refund = to_value(SplitRefundInvoiceResponse {
            attempt_id: "attempt".into(),
            leg_id: "leg".into(),
            amount_msat: 1,
            expires_at: 2,
        })
        .unwrap();
        assert_eq!(
            refund,
            json!({"attempt_id":"attempt","leg_id":"leg","amount_msat":1,"expires_at":2})
        );
        let history = to_value(super::RefundHistoryItem {
            quote_id: "quote".into(),
            leg_id: "leg".into(),
            recipient_pubkey: "recipient".into(),
            principal_msat: 1,
            due_at: 2,
            state: "initiated".into(),
            attempt_id: Some("attempt".into()),
            payment_state: "awaiting_payment".into(),
        })
        .unwrap();
        assert_eq!(history.as_object().unwrap().len(), 8);
        assert!(!history.to_string().contains("invoice"));

        let handlers = include_str!("main.rs");
        assert!(!handlers.contains("adp_commands::request_lnurl_invoice"));
        assert!(!handlers.contains("adp_commands::pay_nwc_invoice"));
        for proof in [
            "confirm_purchase",
            "submit_split_payment",
            "confirm_split_refund",
        ] {
            let body = command_body(include_str!("adp_commands.rs"), proof);
            assert!(!body.contains(".pay_invoice("));
            assert!(!body.contains(".pay_leg_guarded("));
        }
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
        let stored=sqlx::query("SELECT attempt_id,encrypted_proof,EXISTS(SELECT 1 FROM nwc_payment_journal p WHERE p.account_pubkey=r.account_pubkey AND p.payment_hash=r.payment_hash) AS has_intent FROM client_refund_attempts r WHERE account_pubkey=? AND processor_url=? AND quote_id=? AND leg_id=?")
            .bind(account.to_hex()).bind(&request.server_url).bind(&row.quote_id).bind(&row.leg_id).fetch_optional(state.database.pool()).await.map_err(|_|"Local refund history unavailable")?;
        let (attempt, payment_state) = if let Some(stored) = stored {
            let attempt_id: String = stored
                .try_get("attempt_id")
                .map_err(|_| "Invalid refund checkpoint")?;
            let proof: Option<Vec<u8>> = stored
                .try_get("encrypted_proof")
                .map_err(|_| "Invalid refund checkpoint")?;
            let intent: bool = stored
                .try_get("has_intent")
                .map_err(|_| "Invalid refund checkpoint")?;
            (
                Some(attempt_id),
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
    // Recipient refunds are a legitimate obligation, but dispatching one is a
    // live payment under the separate refund activation of the release payment
    // boundary. Read-only reconciliation (`resume_split_refund`) and processor
    // confirmation remain available.
    arcadestr_core::purchase_contract::require_live_refund_authorization()?;
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

/// Recipient refund rules of the release payment boundary, evaluated
/// immediately before a refund is journaled and sent.
async fn refund_dispatch_boundary(
    state: &State<'_, AppState>,
    wallet: &NwcClient,
    account: nostr::PublicKey,
    request: &RefundPaymentRequest,
    obligation: &arcadestr_core::adp_client::RefundObligationView,
    invoice_amount_msat: u64,
    invoice: &lightning_invoice::Bolt11Invoice,
) -> Result<(), String> {
    use arcadestr_core::payment_boundary::{
        authorize_refund_dispatch, release_activation, require_network, verify_wallet,
        PaymentPurpose, RefundDispatchCheck, RefundIntent,
    };
    use arcadestr_core::purchase_contract::InvoiceNetwork;
    let network = match invoice.currency() {
        lightning_invoice::Currency::Bitcoin => InvoiceNetwork::Bitcoin,
        lightning_invoice::Currency::BitcoinTestnet => InvoiceNetwork::Testnet,
        lightning_invoice::Currency::Signet => InvoiceNetwork::Signet,
        lightning_invoice::Currency::Regtest => InvoiceNetwork::Regtest,
        _ => return Err("Unsupported refund invoice network".into()),
    };
    let activation = release_activation(PaymentPurpose::RecipientRefund);
    require_network(activation, network).map_err(|refusal| refusal.to_string())?;
    let verified = verify_wallet(
        &wallet
            .payment_identity()
            .map_err(|_| "Refund wallet identity unavailable; no payment was sent")?,
        wallet
            .capabilities()
            .await
            .map_err(|_| "Refund wallet did not answer get_info; no payment was sent")?,
        network,
    )
    .map_err(|refusal| refusal.to_string())?;
    let unresolved: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM nwc_payment_journal WHERE account_pubkey=? AND payment_hash=? AND encrypted_payment IS NULL)")
        .bind(account.to_hex())
        .bind(invoice.payment_hash().to_string())
        .fetch_one(state.database.pool())
        .await
        .map_err(|_| "Refund payment journal unavailable")?;
    let current = state.auth.lock().await.public_key().map(|key| key.to_hex());
    let initiating = account.to_hex();
    authorize_refund_dispatch(
        activation,
        &RefundDispatchCheck {
            initiating_account: &initiating,
            current_account: current.as_deref(),
            explicit_confirmation: request.confirm_payment,
            obligation_recipient: &obligation.recipient_pubkey,
            obligation_state: &obligation.state,
            obligation_principal_msat: obligation.principal_msat,
            invoice_amount_msat,
            prior_intent: if unresolved {
                RefundIntent::Uncertain
            } else {
                RefundIntent::None
            },
            wallet: &verified,
            invoice_network: network,
        },
    )
    .map_err(|refusal| refusal.to_string())
}

async fn refund_operation(
    request: RefundPaymentRequest,
    state: State<'_, AppState>,
    dispatch: bool,
    expected: Option<nostr::PublicKey>,
) -> Result<serde_json::Value, String> {
    let (auth, wallet_fence) = {
        let auth = state.auth.lock().await;
        let fence = if dispatch {
            let account = auth.public_key().ok_or("Recipient signer locked")?.to_hex();
            state.settlement_recovery.observe(Some(account.clone()));
            Some(state.settlement_recovery.wallet_dispatch_fence(&account)?)
        } else {
            None
        };
        (auth.clone(), fence)
    };
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
    if dispatch && request.expected_attempt_id.as_deref() != Some(invoice.attempt_id.as_str()) {
        return Err(
            "Refund confirmation does not match the original attempt; refresh and review it".into(),
        );
    }
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
        let mut wallet = NwcClient::new(connection);
        if let Some(fence) = &wallet_fence {
            fence.ensure_current()?;
            wallet = wallet.with_dispatch_fence(fence.clone());
        }
        let payments = DurableNwcPayments::new(state.database.pool().clone());
        if dispatch {
            refund_dispatch_boundary(
                &state,
                &wallet,
                account,
                &request,
                &obligation,
                invoice.amount_msat,
                &parsed,
            )
            .await?;
            lease.ensure_current()?;
            if let Some(fence) = &wallet_fence {
                fence.ensure_current()?;
            }
        }
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
    Ok(serde_json::json!({"state": "verified"}))
}
