//! Gate 1: real local relays and Lightning; only ADP deadline time is injected.
#[allow(dead_code)]
#[path = "regtest_refunds.rs"]
mod refunds;
use adp_server::relay::{Relay, RelayClient, RelayLookup};
use anyhow::{anyhow, ensure, Context, Result};
use arcadestr_core::{
    adp_client::{CreateSettlementQuoteRequest, SettlementInvoiceResponse, SettlementProofRequest},
    adp_settlement::{verify_quote_event, QuoteState, RefundDestination, SignedQuote},
    durable_payments::{DurableNwcPayments, PaymentWallet},
    nwc_client::{load_nwc_connection, NwcClient, NwcClientError, PayInvoiceResult},
    signers::NostrSigner,
    storage::Database,
};
use async_trait::async_trait;
use futures_util::{SinkExt, StreamExt};
use nostr::{Event, EventBuilder, EventId, Keys, Kind, Timestamp};
use refunds::{common, Case};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    path::{Path, PathBuf},
    sync::{atomic::Ordering, Arc},
    time::{Duration, Instant},
};
use tokio_tungstenite::{connect_async, tungstenite::Message};

const LEG: u64 = 100_000;
const PORTS: [u16; 4] = [7777, 7778, 10547, 10548];
fn urls() -> Vec<String> {
    PORTS
        .iter()
        .map(|p| format!("ws://127.0.0.1:{p}"))
        .collect()
}
fn at(case: &Case, time: u64) {
    case.clock.store(time, Ordering::SeqCst);
}
fn mark(root: &Path, key: &str, value: Value) -> Result<()> {
    let path = root.join("progress.json");
    let mut report: Value = if path.exists() {
        serde_json::from_slice(&std::fs::read(&path)?)?
    } else {
        json!({})
    };
    report[key] = value;
    common::write_private(&path, &report)
}

// Real per-relay protocol requests with explicit matching EOSE, not a timeout
// interpreted as absence. Raw messages are never printed or written to public logs.
async fn fetch(port: u16, id: EventId) -> Result<Option<Event>> {
    let (mut ws, _) = connect_async(format!("ws://127.0.0.1:{port}")).await?;
    let subscription = format!("gate-{}", uuid::Uuid::new_v4());
    ws.send(Message::Text(
        json!(["REQ",subscription,{"ids":[id.to_hex()]}])
            .to_string()
            .into(),
    ))
    .await?;
    let result = tokio::time::timeout(Duration::from_secs(6), async {
        let mut found = None;
        while let Some(message) = ws.next().await {
            if let Message::Text(text) = message? {
                let value: Value = serde_json::from_str(&text)?;
                if value[0] == "EVENT" && value[1] == subscription {
                    let event: Event = serde_json::from_value(value[2].clone())?;
                    event.verify()?;
                    ensure!(event.id == id, "relay returned wrong event");
                    found = Some(event);
                }
                if value[0] == "EOSE" && value[1] == subscription {
                    return Ok::<_, anyhow::Error>(found);
                }
            }
        }
        Err(anyhow!("relay disconnected before EOSE"))
    })
    .await
    .context("relay EOSE deadline; absence not established")??;
    ws.send(Message::Text(
        json!(["CLOSE", subscription]).to_string().into(),
    ))
    .await?;
    ws.close(None).await?;
    Ok(result)
}
async fn publish(port: u16, event: &Event) -> Result<()> {
    event.verify()?;
    let (mut ws, _) = connect_async(format!("ws://127.0.0.1:{port}")).await?;
    ws.send(Message::Text(json!(["EVENT", event]).to_string().into()))
        .await?;
    tokio::time::timeout(Duration::from_secs(6), async {
        while let Some(message) = ws.next().await {
            if let Message::Text(text) = message? {
                let value: Value = serde_json::from_str(&text)?;
                if value[0] == "OK" && value[1] == event.id.to_hex() {
                    ensure!(
                        value[2] == true,
                        "relay rejected signed event (response suppressed)"
                    );
                    return Ok::<_, anyhow::Error>(());
                }
            }
        }
        Err(anyhow!("relay disconnected before acknowledgement"))
    })
    .await
    .context("relay acknowledgement timed out")??;
    ws.close(None).await?;
    Ok(())
}
async fn all_have(event: &Event) -> Result<()> {
    for port in PORTS {
        ensure!(
            fetch(port, event.id).await?.as_ref() == Some(event),
            "event not retrievable byte-identically on relay {port}"
        );
    }
    Ok(())
}
async fn quote(case: &Case, destination: Option<String>) -> Result<(String, SignedQuote)> {
    let buyer = common::signer(&case.buyer)?;
    let terms: Event = serde_json::from_slice(&std::fs::read(case.root.join("terms.json"))?)?;
    let id = format!("gate1-{}", uuid::Uuid::new_v4());
    let response = case
        .client
        .create_settlement_quote(
            &buyer,
            CreateSettlementQuoteRequest {
                purchase_id: id.clone(),
                listing_event: case.listing.clone(),
                terms_events: vec![terms],
                refund_destination: RefundDestination {
                    endpoint: destination.unwrap_or_else(|| "alice@127.0.0.1:18080".into()),
                    min_sendable_msat: 1000,
                    max_sendable_msat: 10_000_000,
                },
                zap_requests: Default::default(),
            },
        )
        .await
        .map_err(common::safe_adp)?;
    let parsed = verify_quote_event(
        &response.quote,
        &buyer
            .nip44_decrypt(&response.quote.pubkey, &response.quote.content)
            .await?,
    )?;
    ensure!(
        parsed.gross_msat == 2 * LEG && parsed.legs.len() == 2,
        "unexpected test amount"
    );
    common::write_private(&case.root.join("quote.json"), &response.quote)?;
    mark(&case.root, "quote_id", json!(id))?;
    Ok((id, parsed))
}
async fn no_ownership(case: &Case, id: &str) -> Result<()> {
    let status = case
        .client
        .settlement_status(&common::signer(&case.buyer)?, id)
        .await
        .map_err(common::safe_adp)?;
    ensure!(
        status.state == QuoteState::Failed && status.receipt_event_id.is_none(),
        "failed quote gained receipt"
    );
    for table in ["purchases", "download_tokens"] {
        let count: i64 = sqlx::query_scalar(&format!("SELECT COUNT(*) FROM {table}"))
            .fetch_one(&case.store.pool())
            .await?;
        ensure!(count == 0, "failed quote gained ownership/token");
    }
    Ok(())
}
async fn shutdown(mut case: Case) -> Result<()> {
    if let Some(stop) = case.stop.take() {
        let _ = stop.send(());
    }
    case.task.await?;
    Ok(())
}
async fn fallback(case: &Case, id: &str, operator: &Keys) -> Result<(Event, String)> {
    let buyer = common::signer(&case.buyer)?;
    let response = case
        .client
        .prepare_recovery(&buyer, id)
        .await
        .map_err(common::safe_adp)?;
    ensure!(
        response.claimant == "buyer" && response.event.is_none(),
        "processor claimed after grace"
    );
    let plaintext = response.plaintext.context("fallback evidence absent")?;
    let encrypted = buyer
        .nip44_encrypt(&operator.public_key(), &plaintext)
        .await?;
    let event = EventBuilder::new(Kind::Custom(1034), encrypted)
        .custom_created_at(Timestamp::from_secs(case.clock.load(Ordering::SeqCst)))
        .sign_with_keys(&case.buyer)?;
    Ok((event, plaintext))
}
async fn unpaid(root: PathBuf, buyer: &Keys, bob: &Keys, operator: &Keys) -> Result<Value> {
    // Failure and fallback timestamps land in the past, within normal relay policy.
    let issued = Timestamp::now().as_secs() - 7200;
    let case = refunds::setup_live(
        root,
        "relay-deadlines",
        buyer,
        bob,
        operator,
        &urls(),
        issued,
    )
    .await?;
    all_have(&case.listing).await?;
    let (id, q) = quote(&case, None).await?;
    ensure!(
        q.payment_expires_at == issued + 1800 && q.confirmation_expires_at == issued + 86400,
        "noncanonical deadlines"
    );
    let signer = common::signer(buyer)?;
    at(&case, q.payment_expires_at - 1);
    let first = case
        .client
        .settlement_next_invoice(&signer, &id)
        .await
        .map_err(common::safe_adp)?;
    ensure!(
        case.client
            .settlement_status(&signer, &id)
            .await
            .map_err(common::safe_adp)?
            .state
            == QuoteState::Active,
        "failed early"
    );
    at(&case, q.payment_expires_at);
    ensure!(
        case.client
            .settlement_next_invoice(&signer, &id)
            .await
            .is_err(),
        "invoice disclosed at deadline"
    );
    let status = case
        .client
        .settlement_status(&signer, &id)
        .await
        .map_err(common::safe_adp)?;
    ensure!(
        status.failure_at == Some(q.payment_expires_at) && status.settled_legs == 0,
        "disclosure timeout drift"
    );
    let initial = case
        .client
        .prepare_recovery(&signer, &id)
        .await
        .map_err(common::safe_adp)?;
    let processor = initial.event.context("missing processor claim")?;
    all_have(&processor).await?;
    let plaintext = signer
        .nip44_decrypt(&processor.pubkey, &processor.content)
        .await?;
    arcadestr_core::adp_settlement::verify_recovery_event(&processor, &plaintext)?;
    at(&case, q.payment_expires_at + 3600);
    let repeated = case
        .client
        .prepare_recovery(&signer, &id)
        .await
        .map_err(common::safe_adp)?;
    ensure!(
        repeated
            .event
            .as_ref()
            .is_some_and(|e| e.id == processor.id),
        "processor boundary/replay changed identity"
    );
    let mut early = EventBuilder::new(
        Kind::Custom(1034),
        signer
            .nip44_encrypt(&operator.public_key(), &plaintext)
            .await?,
    )
    .sign_with_keys(buyer)?;
    ensure!(
        case.client
            .submit_recovery(&signer, &id, early.clone(), plaintext.clone())
            .await
            .is_err(),
        "buyer bypassed grace"
    );
    at(&case, q.payment_expires_at + 3601);
    let (event, evidence) = fallback(&case, &id, operator).await?;
    // Known absence requires actual EOSE from >=2 local relays in production code.
    let relay = RelayClient::connect_with_timeout(
        operator.clone(),
        &urls(),
        case.base.clone(),
        Duration::from_secs(5),
    )
    .await?;
    ensure!(
        matches!(relay.lookup_event(event.id).await,RelayLookup::ConfirmedAbsent{successful_relays} if successful_relays>=2),
        "missing event not confirmed absent"
    );
    let start = Instant::now();
    publish(7777, &event).await?;
    ensure!(
        fetch(7777, event.id).await?.is_some()
            && fetch(7778, event.id).await?.is_none()
            && fetch(10547, event.id).await?.is_none(),
        "unexpected relay propagation"
    );
    ensure!(
        matches!(relay.lookup_event(event.id).await,RelayLookup::Found(e) if e.id==event.id),
        "cross-relay union lost event"
    );
    tokio::time::sleep(Duration::from_millis(250)).await;
    publish(10547, &event).await?;
    ensure!(
        fetch(10547, event.id).await?.as_ref() == Some(&event)
            && fetch(7778, event.id).await?.is_none(),
        "delayed NAK visibility mismatch"
    );
    tokio::time::sleep(Duration::from_millis(250)).await;
    publish(7778, &event).await?;
    publish(10548, &event).await?;
    all_have(&event).await?;
    let propagation_ms = start.elapsed().as_millis();
    let confirmed = case
        .client
        .submit_recovery(&signer, &id, event.clone(), evidence.clone())
        .await
        .map_err(common::safe_adp)?;
    ensure!(
        confirmed["event"]["id"] == event.id.to_hex(),
        "fallback identity changed"
    );
    for _ in 0..3 {
        let response = case
            .client
            .submit_recovery(&signer, &id, event.clone(), evidence.clone())
            .await
            .map_err(common::safe_adp)?;
        ensure!(
            response["event"]["id"] == event.id.to_hex(),
            "duplicate fallback changed identity"
        );
    }
    // Signed malformed/conflicting data may live on a Nostr relay, but must not
    // be admitted as authoritative settlement evidence by the authenticated route.
    let malformed = EventBuilder::new(
        Kind::Custom(1034),
        signer
            .nip44_encrypt(&operator.public_key(), "not JSON")
            .await?,
    )
    .sign_with_keys(buyer)?;
    publish(10547, &malformed).await?;
    ensure!(
        case.client
            .submit_recovery(&signer, &id, malformed, "not JSON".into())
            .await
            .is_err(),
        "malformed evidence admitted"
    );
    let mut conflicting: Value = serde_json::from_str(&evidence)?;
    conflicting["failure_at"] = json!(q.payment_expires_at + 1);
    let conflict_plaintext = conflicting.to_string();
    early = EventBuilder::new(
        Kind::Custom(1034),
        signer
            .nip44_encrypt(&operator.public_key(), &conflict_plaintext)
            .await?,
    )
    .sign_with_keys(buyer)?;
    publish(7777, &early).await?;
    ensure!(
        case.client
            .submit_recovery(&signer, &id, early, conflict_plaintext)
            .await
            .is_err(),
        "conflicting evidence admitted"
    );
    let mismatched =
        EventBuilder::new(Kind::Custom(1034), "not encrypted evidence").sign_with_keys(buyer)?;
    ensure!(
        case.client
            .submit_recovery(&signer, &id, mismatched, evidence.clone())
            .await
            .is_err(),
        "signed ciphertext/plaintext mismatch admitted"
    );
    let mut bad_signature = serde_json::to_value(&event)?;
    bad_signature["sig"] = json!("00".repeat(64));
    let bad: Event = serde_json::from_value(bad_signature)?;
    ensure!(
        case.client
            .submit_recovery(&signer, &id, bad, evidence.clone())
            .await
            .is_err(),
        "invalid signature admitted"
    );
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM recovery_claims WHERE quote_id=?")
        .bind(&id)
        .fetch_one(&case.store.pool())
        .await?;
    ensure!(
        count == 2,
        "duplicates or conflicts persisted as new claims"
    );
    ensure!(
        common::signer(operator)?
            .nip44_decrypt(&event.pubkey, &event.content)
            .await?
            == evidence,
        "fallback ciphertext does not bind evidence"
    );
    no_ownership(&case, &id).await?;
    let result = json!({"status":"pass","quote_id":id,"processor_event":processor.id.to_hex(),"buyer_event":event.id.to_hex(),
        "relays":PORTS,"controlled_forwarding_delay_ms":500,"observed_propagation_sequence_ms":propagation_ms,
        "deadline_time":"injected; no 30-minute/hour wall-clock wait","disclosed_invoice_hash":first.payment_hash,
        "publication_retrieval":true,"missing_delayed_split_relays":true,"duplicate_conflicting_malformed":true,"claims":count});
    common::write_private(&case.root.join("result.json"), &result)?;
    shutdown(case).await?;
    Ok(result)
}

#[derive(Serialize, Deserialize)]
struct Job {
    base: String,
    quote_id: String,
    leg_id: String,
    attempt_id: String,
    bolt11: String,
    db: PathBuf,
    audit: PathBuf,
    result: PathBuf,
}
struct AmbiguousWallet {
    audit: PathBuf,
    mode: String,
}
impl AmbiguousWallet {
    fn count(&self, key: &str) -> Result<(), NwcClientError> {
        let mut value: Value = if self.audit.exists() {
            serde_json::from_slice(
                &std::fs::read(&self.audit).map_err(|_| NwcClientError::InvalidLookup)?,
            )
            .map_err(|_| NwcClientError::InvalidLookup)?
        } else {
            json!({"pay_calls":0,"lookup_calls":0,"response_observed":false})
        };
        value[key] = json!(value[key].as_u64().unwrap_or(0) + 1);
        common::write_private(&self.audit, &value).map_err(|_| NwcClientError::InvalidLookup)
    }
}
#[async_trait]
impl PaymentWallet for AmbiguousWallet {
    fn identity(&self) -> Result<String, NwcClientError> {
        let nodes = common::check_network().map_err(|_| NwcClientError::InvalidLookup)?;
        Ok(format!("polar-lnd-regtest:{}", nodes["bob"]))
    }
    async fn pay(&self, invoice: &str) -> Result<PayInvoiceResult, NwcClientError> {
        self.count("pay_calls")?;
        if self.mode != "dispatch-crash" {
            return Err(NwcClientError::InvalidLookup);
        }
        let nodes = common::check_network().map_err(|_| NwcClientError::InvalidLookup)?;
        common::validate_invoice(invoice, &nodes["alice"], LEG)
            .map_err(|_| NwcClientError::InvalidLookup)?;
        let log = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(self.audit.with_extension("wallet-private.log"))
            .map_err(|_| NwcClientError::InvalidLookup)?;
        let mut command = std::process::Command::new("docker");
        command
            .args([
                "exec",
                "polar-n1-bob",
                "lncli",
                "--lnddir=/home/lnd/.lnd",
                "--network=regtest",
                "payinvoice",
                "--force",
                "--json",
                "--fee_limit=10",
                "--timeout=300s",
            ])
            .arg(format!("--pay_req={invoice}"))
            .stdin(std::process::Stdio::null())
            .stderr(std::process::Stdio::from(
                log.try_clone().map_err(|_| NwcClientError::InvalidLookup)?,
            ))
            .stdout(std::process::Stdio::from(log));
        let child = command.spawn().map_err(|_| NwcClientError::WalletTimeout)?;
        common::write_private(
            &self.audit.with_extension("dispatch.json"),
            &json!({"wallet_process_id":child.id(),"result_received":false}),
        )
        .map_err(|_| NwcClientError::InvalidLookup)?;
        // Do not wait, parse stdout, or receive a wallet result in this process.
        std::process::exit(87);
    }
    async fn lookup(
        &self,
        hash: &str,
        amount: u64,
    ) -> Result<Option<PayInvoiceResult>, NwcClientError> {
        self.count("lookup_calls")?;
        let response = common::lnd("bob", "listpayments", &["--include_incomplete"])
            .map_err(|_| NwcClientError::InvalidLookup)?;
        // Every mode makes a real read-only lookup. Fault modes suppress its
        // reply at the adapter boundary; they do not claim LND returned NOT_FOUND.
        match self.mode.as_str() {
            "not-found" => return Ok(None),
            "timeout" => return Err(NwcClientError::WalletTimeout),
            "unsupported" => {
                return Err(NwcClientError::Relay(
                    "injected unsupported lookup reply".into(),
                ))
            }
            _ => {}
        }
        let record = response["payments"]
            .as_array()
            .ok_or(NwcClientError::InvalidLookup)?
            .iter()
            .find(|p| p["payment_hash"].as_str() == Some(hash));
        match record {
            Some(p)
                if p["status"] == "SUCCEEDED"
                    && common::num(&p["value_msat"]).ok() == Some(amount) =>
            {
                Ok(Some(PayInvoiceResult {
                    preimage: p["payment_preimage"]
                        .as_str()
                        .ok_or(NwcClientError::InvalidLookup)?
                        .into(),
                    fees_paid_msat: Some(
                        common::num(&p["fee_msat"]).map_err(|_| NwcClientError::InvalidLookup)?,
                    ),
                }))
            }
            _ => Ok(None),
        }
    }
}
async fn child(path: &Path, mode: &str) -> Result<()> {
    let job: Job = serde_json::from_slice(&std::fs::read(path)?)?;
    ensure!(
        job.base.starts_with("http://127.0.0.1:"),
        "nonlocal child server"
    );
    let (_, bob) = common::accounts().await?;
    let signer = common::signer(&bob)?;
    let db = Database::new(&job.db).await?;
    let payments = DurableNwcPayments::new(db.pool().clone());
    let wallet = AmbiguousWallet {
        audit: job.audit.clone(),
        mode: mode.into(),
    };
    if mode == "dispatch-crash" {
        let _ = payments
            .pay_or_resume(&signer, &wallet, &job.bolt11)
            .await?;
        return Err(anyhow!("child did not crash"));
    }
    if ["not-found", "timeout", "unsupported", "pending"].contains(&mode) {
        // Intentionally use dispatch-capable API against existing intent: it
        // must only LOOK UP. A regression would be visible in the pay audit.
        ensure!(
            matches!(
                payments.pay_or_resume(&signer, &wallet, &job.bolt11).await,
                Err(arcadestr_core::durable_payments::DurablePaymentError::Unresolved)
            ),
            "unresolved lookup did not block"
        );
        common::write_private(&job.result, &json!({"state":"blocked","mode":mode}))?;
        return Ok(());
    }
    let paid = payments.reconcile(&signer, &wallet, &job.bolt11).await?;
    let http: Arc<dyn arcadestr_core::http_client::HttpClient> = Arc::new(
        arcadestr_core::http_client::ReqwestHttpClient::new(Duration::from_secs(30))?,
    );
    let client = arcadestr_core::adp_client::AdpClient::new(job.base, http);
    let response = client
        .confirm_refund(
            &signer,
            &job.quote_id,
            &job.leg_id,
            &job.attempt_id,
            &paid.preimage,
        )
        .await
        .map_err(common::safe_adp)?;
    ensure!(
        response["state"] == "verified",
        "refund confirmation failed"
    );
    common::write_private(&job.result, &json!({"state":"verified"}))?;
    Ok(())
}
async fn run_child(path: &Path, mode: &str) -> Result<()> {
    let response = tokio::process::Command::new(std::env::current_exe()?)
        .arg("child")
        .arg(path)
        .arg(mode)
        .output()
        .await?;
    ensure!(
        if mode == "dispatch-crash" {
            response.status.code() == Some(87)
        } else {
            response.status.success()
        },
        "refund child {mode} failed; private output suppressed"
    );
    Ok(())
}
async fn wait_invoice(hash: &str, wanted: &str) -> Result<()> {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        if common::lnd("alice", "lookupinvoice", &[hash])?["state"] == wanted {
            return Ok(());
        }
        ensure!(
            Instant::now() < deadline,
            "Alice refund did not reach {wanted}"
        );
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}
async fn pending_intent(case: &Case, hash: &str) -> Result<()> {
    let count:i64=sqlx::query_scalar("SELECT COUNT(*) FROM nwc_payment_journal WHERE account_pubkey=? AND payment_hash=? AND encrypted_payment IS NULL")
        .bind(case.bob.public_key().to_hex()).bind(hash).fetch_one(case.db.pool()).await?;
    ensure!(
        count == 1,
        "original unresolved refund intent lost/incorrectly checkpointed"
    );
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM refund_attempts WHERE payment_hash=? AND verified_at IS NULL",
    )
    .bind(hash)
    .fetch_one(&case.store.pool())
    .await?;
    ensure!(count == 1, "unresolved refund marked verified");
    Ok(())
}

struct Hold {
    endpoint: String,
    preimage: String,
    hash: String,
    task: tokio::task::JoinHandle<()>,
}
async fn hold_fixture(root: &Path, original_endpoint: Option<String>) -> Result<Hold> {
    let existing: Option<Value> = if original_endpoint.is_some() {
        Some(serde_json::from_slice(&std::fs::read(
            root.join("hold-secret.json"),
        )?)?)
    } else {
        None
    };
    let preimage = match &existing {
        Some(value) => value["preimage"]
            .as_str()
            .context("private hold secret missing")?
            .to_owned(),
        None => Keys::generate().secret_key().to_secret_hex(),
    };
    let hash = hex::encode(Sha256::digest(hex::decode(&preimage)?));
    if existing.is_none() {
        common::write_private(
            &root.join("hold-secret.json"),
            &json!({"preimage":preimage,"hash":hash}),
        )?;
    }
    let address = match original_endpoint {
        Some(endpoint) => endpoint
            .strip_prefix("http://")
            .and_then(|s| s.strip_suffix("/lnurlp/alice"))
            .context("nonlocal original hold endpoint")?
            .to_owned(),
        None => "127.0.0.1:0".into(),
    };
    ensure!(address.starts_with("127.0.0.1:"), "nonlocal hold fixture");
    let listener = tokio::net::TcpListener::bind(address).await?;
    let endpoint = format!("http://{}/lnurlp/alice", listener.local_addr()?);
    let metadata = "[[\"text/plain\",\"Gate 1 Alice hold refund\"]]";
    let invoice = if existing.is_some() {
        common::lnd("alice", "lookupinvoice", &[&hash])?
    } else {
        common::lnd(
            "alice",
            "addholdinvoice",
            &[
                &hash,
                "--amt_msat=100000",
                "--expiry=600",
                &format!(
                    "--description_hash={}",
                    hex::encode(Sha256::digest(metadata.as_bytes()))
                ),
            ],
        )?
    };
    let bolt11 = invoice["payment_request"]
        .as_str()
        .context("hold invoice missing")?
        .to_owned();
    let callback = format!("{endpoint}/callback");
    let nostr_pubkey = std::env::var("POLAR_ALICE_NOSTR_PUBKEY")?;
    let app=axum::Router::new().route("/lnurlp/alice",axum::routing::get(move||async move{axum::Json(json!({"tag":"payRequest","callback":callback,"minSendable":1000,"maxSendable":10_000_000,"metadata":metadata,"nostrPubkey":nostr_pubkey,"allowsNostr":false}))}))
        .route("/lnurlp/alice/callback",axum::routing::get(move|axum::extract::Query(params):axum::extract::Query<std::collections::HashMap<String,String>>|async move{
            if params.get("amount").map(String::as_str)!=Some("100000"){return axum::Json(json!({"status":"ERROR","reason":"wrong amount"}));}
            axum::Json(json!({"pr":bolt11,"routes":[]}))
        }));
    let task = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    Ok(Hold {
        endpoint,
        preimage,
        hash,
        task,
    })
}
async fn buyer_wallet(buyer: &Keys) -> Result<NwcClient> {
    let connection = load_nwc_connection(&buyer.public_key())
        .map_err(|_| anyhow!("existing buyer NWC missing"))?;
    let hub = rusqlite::Connection::open_with_flags(
        common::env_path("ARCADESTR_REGTEST_HUB_DB")?,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )?;
    let backend: String = hub.query_row(
        "SELECT value FROM user_configs WHERE key='LNBackendType' AND encrypted=0",
        [],
        |r| r.get(0),
    )?;
    let address: String = hub.query_row(
        "SELECT value FROM user_configs WHERE key='LNDAddress' AND encrypted=0",
        [],
        |r| r.get(0),
    )?;
    let pubkey:String=hub.query_row("SELECT wallet_pubkey FROM apps WHERE name IN ('Arcadestr Regtest Buyer','Alice Arcadestr Regtest Buyer')",[],|r|r.get(0))?;
    ensure!(
        backend == "LND"
            && address == "127.0.0.1:10001"
            && connection.wallet_pubkey_hex() == pubkey
            && !connection.relay_urls().is_empty()
            && connection
                .relay_urls()
                .iter()
                .all(|u| matches!(u.as_str(), "ws://127.0.0.1:7778" | "ws://127.0.0.1:7778/")),
        "existing buyer NWC binding changed"
    );
    let wallet = NwcClient::new(connection).with_timeout(Duration::from_secs(30));
    wallet
        .verify()
        .await
        .map_err(|_| anyhow!("existing buyer NWC unavailable"))?;
    Ok(wallet)
}
async fn paid_case(
    root: PathBuf,
    name: &str,
    buyer: &Keys,
    bob: &Keys,
    operator: &Keys,
    hold: bool,
) -> Result<Value> {
    std::fs::create_dir_all(&root)?;
    let before = common::balances()?;
    let nodes = common::check_network()?;
    // Leave 750 seconds of real invoice validity; claims at 24h are still within
    // normal strfry future tolerance. Do NOT alter any relay admission policy.
    let issued = Timestamp::now().as_secs() - 86400 + 750;
    let case = refunds::setup_live(root, name, buyer, bob, operator, &urls(), issued).await?;
    let fixture = if hold {
        Some(hold_fixture(&case.root, None).await?)
    } else {
        None
    };
    let (id, q) = quote(&case, fixture.as_ref().map(|h| h.endpoint.clone())).await?;
    let signer = common::signer(buyer)?;
    let wallet = buyer_wallet(buyer).await?;
    let payments = DurableNwcPayments::new(case.db.pool().clone());
    let first = case
        .client
        .settlement_next_invoice(&signer, &id)
        .await
        .map_err(common::safe_adp)?;
    common::validate_invoice(&first.bolt11, &nodes["bob"], LEG)?;
    ensure!(
        common::check_network()? == nodes,
        "Lightning identities changed"
    );
    mark(
        &case.root,
        "purchase_dispatch",
        json!({"hash":first.payment_hash,"calls":1}),
    )?;
    let paid = payments
        .pay_or_resume(&signer, &wallet, &first.bolt11)
        .await?;
    let failure;
    if !hold {
        // A real Bob payment whose proof reaches ADP after deterministic failure.
        failure = q.payment_expires_at;
        at(&case, failure);
        let status = case
            .client
            .settlement_status(&signer, &id)
            .await
            .map_err(common::safe_adp)?;
        ensure!(
            status.state == QuoteState::Failed && status.settled_legs == 0,
            "missing-proof deadline mismatch"
        );
        let obligation: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM refund_obligations WHERE quote_id=?")
                .bind(&id)
                .fetch_one(&case.store.pool())
                .await?;
        ensure!(
            obligation == 0,
            "unproven principal created refund prematurely"
        );
        let initial = case
            .client
            .prepare_recovery(&signer, &id)
            .await
            .map_err(common::safe_adp)?
            .event
            .context("initial claim missing")?;
        all_have(&initial).await?;
        at(&case, failure + 1);
        let response = case
            .client
            .submit_settlement_proof(
                &signer,
                &id,
                SettlementProofRequest {
                    attempt_id: first.attempt_id.clone(),
                    preimage: paid.preimage.clone(),
                },
            )
            .await
            .map_err(common::safe_adp)?;
        ensure!(
            response.state == "failed"
                && response.receipt.is_none()
                && response.download_token.is_none(),
            "late proof resurrected failed quote"
        );
        let updated = case
            .client
            .prepare_recovery(&signer, &id)
            .await
            .map_err(common::safe_adp)?;
        let later = updated.event.context("late proof claim missing")?;
        ensure!(
            later.id != initial.id && updated.evidence.paid_leg_ids == vec![first.leg_id.clone()],
            "late proof evidence not versioned"
        );
        all_have(&later).await?;
        all_have(&initial).await?;
        let repeated = case
            .client
            .prepare_recovery(&signer, &id)
            .await
            .map_err(common::safe_adp)?;
        ensure!(
            repeated.event.as_ref().is_some_and(|e| e.id == later.id),
            "late evidence retry not canonical"
        );
        mark(
            &case.root,
            "late_proof",
            json!({"initial_event":initial.id.to_hex(),"updated_event":later.id.to_hex(),"status":"pass"}),
        )?;
    } else {
        case.client
            .submit_settlement_proof(
                &signer,
                &id,
                SettlementProofRequest {
                    attempt_id: first.attempt_id.clone(),
                    preimage: paid.preimage.clone(),
                },
            )
            .await
            .map_err(common::safe_adp)?;
        let second = case
            .client
            .settlement_next_invoice(&signer, &id)
            .await
            .map_err(common::safe_adp)?;
        common::validate_invoice(&second.bolt11, &nodes["carol"], LEG)?;
        // Carol's disclosed invoice remains unpaid; no attempted second transfer.
        failure = q.confirmation_expires_at;
        at(&case, failure - 1);
        ensure!(
            case.client
                .settlement_status(&signer, &id)
                .await
                .map_err(common::safe_adp)?
                .state
                == QuoteState::Active,
            "confirmation failed early"
        );
        at(&case, failure);
        ensure!(
            case.client
                .settlement_status(&signer, &id)
                .await
                .map_err(common::safe_adp)?
                .failure_at
                == Some(failure),
            "confirmation deadline drift"
        );
        let event = case
            .client
            .prepare_recovery(&signer, &id)
            .await
            .map_err(common::safe_adp)?
            .event
            .context("confirmation claim missing")?;
        all_have(&event).await?;
        common::lnd("carol", "cancelinvoice", &[&second.payment_hash])?;
    }
    let obligation: (String, i64) =
        sqlx::query_as("SELECT state,due_at FROM refund_obligations WHERE quote_id=?")
            .bind(&id)
            .fetch_one(&case.store.pool())
            .await?;
    ensure!(
        obligation == ("refund_owed".into(), (failure + 604800) as i64),
        "late proof changed original refund deadline"
    );
    at(&case, failure + 604800);
    case.client
        .settlement_status(&signer, &id)
        .await
        .map_err(common::safe_adp)?;
    let state: String = sqlx::query_scalar("SELECT state FROM refund_obligations WHERE quote_id=?")
        .bind(&id)
        .fetch_one(&case.store.pool())
        .await?;
    ensure!(
        state == "refund_owed",
        "refund became overdue before due boundary"
    );
    at(&case, failure + 604801);
    case.client
        .settlement_status(&signer, &id)
        .await
        .map_err(common::safe_adp)?;
    let state: String = sqlx::query_scalar("SELECT state FROM refund_obligations WHERE quote_id=?")
        .bind(&id)
        .fetch_one(&case.store.pool())
        .await?;
    ensure!(state == "overdue", "seven-day overdue transition missing");
    mark(
        &case.root,
        "deadline_checks",
        json!({"confirmation_or_disclosure":true,"due_boundary":true,"overdue":true}),
    )?;
    finish_refund(case, id, first, failure, before, fixture).await
}

async fn finish_refund(
    case: Case,
    id: String,
    first: SettlementInvoiceResponse,
    failure: u64,
    before: std::collections::BTreeMap<String, u64>,
    fixture: Option<Hold>,
) -> Result<Value> {
    let hold = fixture.is_some();
    let nodes = common::check_network()?;
    let bob_keys = case.bob.clone();
    let bob = &bob_keys;
    let bob_signer = common::signer(bob)?;
    let invoice = case
        .client
        .prepare_refund(&bob_signer, &id, &first.leg_id)
        .await
        .map_err(common::safe_adp)?;
    let refund_hash = invoice
        .bolt11
        .parse::<lightning_invoice::Bolt11Invoice>()
        .map_err(|_| anyhow!("invalid refund invoice"))?
        .payment_hash()
        .to_string();
    common::validate_invoice(&invoice.bolt11, &nodes["alice"], LEG)?;
    if let Some(h) = &fixture {
        ensure!(refund_hash == h.hash, "hold invoice was replaced");
    }
    let job = Job {
        base: case.base.clone(),
        quote_id: id.clone(),
        leg_id: first.leg_id.clone(),
        attempt_id: invoice.attempt_id.clone(),
        bolt11: invoice.bolt11.clone(),
        db: case.root.join("client.db"),
        audit: case.root.join("refund-audit.json"),
        result: case.root.join("child-result.json"),
    };
    let path = case.root.join("job.json");
    common::write_private(&path, &job)?;
    run_child(&path, "dispatch-crash").await?;
    wait_invoice(&refund_hash, if hold { "ACCEPTED" } else { "SETTLED" }).await?;
    pending_intent(&case, &refund_hash).await?;
    let modes: Vec<&str> = if hold {
        vec!["pending", "pending"]
    } else {
        vec![
            "not-found",
            "timeout",
            "unsupported",
            "not-found",
            "timeout",
            "unsupported",
        ]
    };
    for mode in modes {
        run_child(&path, mode).await?;
        pending_intent(&case, &refund_hash).await?;
        let again = case
            .client
            .prepare_refund(&bob_signer, &id, &first.leg_id)
            .await
            .map_err(common::safe_adp)?;
        ensure!(
            again.attempt_id == invoice.attempt_id && again.bolt11 == invoice.bolt11,
            "ambiguous outcome yielded replacement invoice"
        );
    }
    if let Some(h) = &fixture {
        let outgoing = common::lnd("bob", "listpayments", &["--include_incomplete"])?;
        ensure!(
            outgoing["payments"]
                .as_array()
                .context("payments missing")?
                .iter()
                .any(|p| p["payment_hash"].as_str() == Some(&refund_hash)
                    && p["status"] == "IN_FLIGHT"),
            "unresolved refund not actually in flight"
        );
        mark(
            &case.root,
            "real_pending_htlc",
            json!({"hash":refund_hash,"invoice_state":"ACCEPTED","outgoing_state":"IN_FLIGHT","intent_preserved":true}),
        )?;
        common::lnd("alice", "settleinvoice", &[&h.preimage])?;
        wait_invoice(&refund_hash, "SETTLED").await?;
    }
    // Fresh process, real wallet lookup; another process retries from cached proof.
    run_child(&path, "settled").await?;
    run_child(&path, "settled").await?;
    let audit: Value = serde_json::from_slice(&std::fs::read(&job.audit)?)?;
    ensure!(
        audit["pay_calls"] == 1 && audit["lookup_calls"] == if hold { 3 } else { 7 },
        "refund dispatch/lookup count mismatch"
    );
    let persisted:i64=sqlx::query_scalar("SELECT COUNT(*) FROM nwc_payment_journal WHERE account_pubkey=? AND payment_hash=? AND encrypted_payment IS NOT NULL")
        .bind(bob.public_key().to_hex()).bind(&refund_hash).fetch_one(case.db.pool()).await?;
    let verified:i64=sqlx::query_scalar("SELECT COUNT(*) FROM refund_attempts WHERE payment_hash=? AND verified_at IS NOT NULL AND proof_encrypted IS NOT NULL")
        .bind(&refund_hash).fetch_one(&case.store.pool()).await?;
    ensure!(
        persisted == 1 && verified == 1,
        "refund proof persistence mismatch"
    );
    no_ownership(&case, &id).await?;
    let after = common::balances()?;
    ensure!(before == after, "balances not restored by refund");
    let outgoing = common::lnd("bob", "listpayments", &["--include_incomplete"])?;
    let matches: Vec<_> = outgoing["payments"]
        .as_array()
        .context("payments missing")?
        .iter()
        .filter(|p| p["payment_hash"].as_str() == Some(&refund_hash))
        .collect();
    ensure!(
        matches.len() == 1
            && matches[0]["status"] == "SUCCEEDED"
            && common::num(&matches[0]["value_msat"])? == LEG,
        "outgoing refund mismatch"
    );
    let result = json!({"status":"pass","case":if hold{"confirmation-deadline-real-inflight"}else{"late-proof-lost-settled-reply"},"quote_id":id,
        "failure_at":failure,"refund_due_at":failure+604800,"refund_hash":refund_hash,"refund_state":"verified","refund_audit":audit,
        "interruption":"child exits 87 immediately after spawning real wallet dispatch; no response read",
        "lookup_faults":if hold{"real IN_FLIGHT returns unresolved"}else{"real lookup response masked as NOT_FOUND/timeout/unsupported; each blocks"},
        "purchase_dispatches":1,"refund_dispatches":1,"balances_before_msat":before,"balances_after_msat":after,"ownership":false,
        "deadline_time":"test-injected; real invoices/HTLC settlement use wall time"});
    common::write_private(&case.root.join("result.json"), &result)?;
    if let Some(h) = fixture {
        h.task.abort();
    }
    shutdown(case).await?;
    Ok(result)
}
async fn resume_pending(root: &Path, buyer: &Keys, bob: &Keys, operator: &Keys) -> Result<Value> {
    let directory = root.join("pending-refund");
    ensure!(
        !directory.join("job.json").exists(),
        "refund dispatch job already exists; inspect/reconcile it, never start a new dispatch"
    );
    let progress: Value = serde_json::from_slice(&std::fs::read(directory.join("progress.json"))?)?;
    let id = progress["quote_id"]
        .as_str()
        .context("pending quote absent")?
        .to_owned();
    let store = adp_server::storage::Storage::connect(
        directory.join("adp.db").to_str().context("DB path")?,
    )
    .await?;
    let status = store
        .settlement_status(&id)
        .await?
        .context("pending quote missing")?;
    let failure = status.failure_at.context("pending quote not failed")?;
    let attempt = store
        .settlement_attempts(&id)
        .await?
        .into_iter()
        .find(|a| a.settled_at.is_some())
        .context("original Bob proof missing")?;
    let context = store
        .refund_context(&id, &attempt.leg_id)
        .await?
        .context("original obligation missing")?;
    ensure!(
        context.state == "overdue"
            && store
                .refund_attempt_for_leg(&id, &attempt.leg_id)
                .await?
                .is_none(),
        "refund already attempted; do not replace it"
    );
    let cipher =
        adp_server::crypto::MasterCipher::new(&hex::decode(std::env::var("ADP_MASTER_KEY")?)?)?;
    let destination: RefundDestination =
        serde_json::from_slice(&cipher.decrypt(&serde_json::from_slice::<
            adp_server::crypto::EncryptedSecret,
        >(&context.encrypted_destination)?)?)?;
    let bolt11 = String::from_utf8(cipher.decrypt(&serde_json::from_slice::<
        adp_server::crypto::EncryptedSecret,
    >(&attempt.encrypted_bolt11)?)?)?;
    let first = SettlementInvoiceResponse {
        quote_id: id.clone(),
        attempt_id: attempt.attempt_id,
        leg_id: attempt.leg_id,
        amount_msat: attempt.amount_msat,
        payment_hash: attempt.payment_hash,
        bolt11,
        expires_at: attempt.expires_at,
    };
    let prior: Value = serde_json::from_slice(&std::fs::read(root.join("progress.json"))?)?;
    let before: std::collections::BTreeMap<String, u64> =
        serde_json::from_value(prior["late_proof"]["balances_after_msat"].clone())?;
    let current = common::balances()?;
    for node in ["alice", "bob", "carol", "dave"] {
        let delta = if node == "alice" {
            -100000
        } else if node == "bob" {
            100000
        } else {
            0
        };
        ensure!(
            current[node] as i64 - before[node] as i64 == delta,
            "pending purchase balances differ from prior completed snapshot"
        );
    }
    let case =
        refunds::restore_live(directory, buyer, bob, operator, &urls(), failure + 604801).await?;
    let wallet = buyer_wallet(buyer).await?;
    DurableNwcPayments::new(case.db.pool().clone())
        .reconcile(&common::signer(buyer)?, &wallet, &first.bolt11)
        .await?;
    let fixture = hold_fixture(&case.root, Some(destination.endpoint)).await?;
    mark(
        &case.root,
        "continued_same_order",
        json!({"new_purchase_dispatches":0,"reservation_retained":true,"balance_reference":"preceding completed late-proof case snapshot; current principal delta checked"}),
    )?;
    finish_refund(case, id, first, failure, before, Some(fixture)).await
}

/// Read-only revalidation: signed quote/authorization chains, append-only claim
/// IDs on every live relay, and exact persisted/refunded Lightning hashes.
async fn verify_existing(root: &Path) -> Result<Value> {
    let report: Value = serde_json::from_slice(&std::fs::read(root.join("report.json"))?)?;
    ensure!(
        report["status"] == "pass",
        "cannot verify an incomplete Gate 1 run"
    );
    let (buyer, bob) = common::accounts().await?;
    let operator = Keys::parse(&std::env::var("ADP_OPERATOR_NSEC")?)?;
    let buyer_signer = common::signer(&buyer)?;
    let operator_signer = common::signer(&operator)?;
    let relay = RelayClient::connect_with_timeout(
        operator.clone(),
        &urls(),
        "http://127.0.0.1:1".into(),
        Duration::from_secs(5),
    )
    .await?;
    let mut total_claims = 0;
    for name in ["live-relays", "late-proof", "pending-refund"] {
        let directory = root.join(name);
        let quote_event: Event =
            serde_json::from_slice(&std::fs::read(directory.join("quote.json"))?)?;
        let q = verify_quote_event(
            &quote_event,
            &buyer_signer
                .nip44_decrypt(&operator.public_key(), &quote_event.content)
                .await?,
        )?;
        let (listing, claims, failure, refunds) = {
            let db = rusqlite::Connection::open_with_flags(
                directory.join("adp.db"),
                rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
            )?;
            let listing: String = db.query_row(
                "SELECT listing_event_json FROM purchase_quotes WHERE quote_id=?",
                [&q.quote_id],
                |r| r.get(0),
            )?;
            let failure: u64 = db.query_row(
                "SELECT failure_at FROM purchase_quotes WHERE quote_id=?",
                [&q.quote_id],
                |r| r.get(0),
            )?;
            let failed:i64=db.query_row("SELECT COUNT(*) FROM purchase_quotes WHERE quote_id=? AND state='failed' AND receipt_event_id IS NULL",[&q.quote_id],|r|r.get(0))?;
            ensure!(failed == 1, "historical quote resurrected");
            let incorrect_obligations:i64=db.query_row("SELECT COUNT(*) FROM refund_obligations WHERE quote_id=? AND (state!='verified' OR due_at!=?)",rusqlite::params![&q.quote_id,(failure+604800) as i64],|r|r.get(0))?;
            ensure!(incorrect_obligations == 0, "refund state/due time drifted");
            let mut statement =
                db.prepare("SELECT event_json FROM recovery_claims WHERE quote_id=?")?;
            let claims = statement
                .query_map([&q.quote_id], |r| r.get::<_, String>(0))?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            let mut statement=db.prepare("SELECT payment_hash FROM refund_attempts WHERE quote_id=? AND verified_at IS NOT NULL AND proof_encrypted IS NOT NULL")?;
            let refunds = statement
                .query_map([&q.quote_id], |r| r.get::<_, String>(0))?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            for table in ["purchases", "download_tokens"] {
                ensure!(
                    db.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r
                        .get::<_, i64>(0))?
                        == 0,
                    "failed order gained ownership"
                );
            }
            (
                serde_json::from_str::<Event>(&listing)?,
                claims,
                failure,
                refunds,
            )
        };
        listing.verify()?;
        ensure!(
            listing.id.to_hex() == q.listing_event_id && listing.pubkey == bob.public_key(),
            "listing/quote signature chain mismatch"
        );
        let root_id = listing
            .tags
            .iter()
            .find_map(|t| {
                let v = t.as_slice();
                if v.first().map(String::as_str) == Some("fulfillment_authorization") {
                    v.get(1).cloned()
                } else {
                    None
                }
            })
            .context("authorization root absent")?;
        let events = match relay
            .lookup_authorizations(bob.public_key(), &q.listing_coordinate)
            .await
        {
            RelayLookup::Found(events) => events,
            _ => {
                return Err(anyhow!(
                    "authorization chain not available from live relays"
                ))
            }
        };
        let resolved = arcadestr_core::authorization::resolve_authorization_at(
            EventId::from_hex(&root_id)?,
            &events,
            q.issued_at,
        )?;
        ensure!(
            resolved.developer_pubkey == bob.public_key()
                && resolved.terms.operator_pubkey == operator.public_key()
                && resolved.authorizes(
                    &resolved.terms.fulfillment_pubkey,
                    &q.listing_coordinate,
                    arcadestr_core::authorization::CAPABILITY_ISSUE_RECEIPT,
                    q.issued_at
                ),
            "authorization chain does not authorize quote"
        );
        for encoded in claims {
            let event: Event = serde_json::from_str(&encoded)?;
            let plaintext = if event.pubkey == operator.public_key() {
                buyer_signer
                    .nip44_decrypt(&event.pubkey, &event.content)
                    .await?
            } else {
                operator_signer
                    .nip44_decrypt(&event.pubkey, &event.content)
                    .await?
            };
            let evidence =
                arcadestr_core::adp_settlement::verify_recovery_event(&event, &plaintext)?;
            ensure!(
                evidence.quote_id == q.quote_id
                    && evidence.quote_event_id == quote_event.id.to_hex()
                    && evidence.failure_at == failure
                    && evidence.buyer_pubkey == buyer.public_key().to_hex()
                    && evidence.processor_pubkey == operator.public_key().to_hex(),
                "claim/quote evidence chain mismatch"
            );
            let commitment = hex::encode(Sha256::digest(serde_json::to_vec(&(
                evidence.version,
                &evidence.quote_id,
                &evidence.quote_event_id,
                evidence.failure_at,
                &evidence.paid_leg_ids,
            ))?));
            ensure!(
                commitment == evidence.evidence_commitment,
                "recovery commitment mismatch"
            );
            all_have(&event).await?;
            total_claims += 1;
        }
        let outgoing = common::lnd("bob", "listpayments", &["--include_incomplete"])?;
        for hash in refunds {
            let records: Vec<_> = outgoing["payments"]
                .as_array()
                .context("outgoing missing")?
                .iter()
                .filter(|p| p["payment_hash"].as_str() == Some(&hash))
                .collect();
            ensure!(
                records.len() == 1
                    && records[0]["status"] == "SUCCEEDED"
                    && common::num(&records[0]["value_msat"])? == LEG,
                "refund paid more than once/wrong amount"
            );
            let invoice = common::lnd("alice", "lookupinvoice", &[&hash])?;
            ensure!(invoice["state"] == "SETTLED", "refund not settled");
            let checkpoint: Vec<u8> = {
                let db = rusqlite::Connection::open_with_flags(
                    directory.join("client.db"),
                    rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
                )?;
                db.query_row("SELECT encrypted_payment FROM nwc_payment_journal WHERE account_pubkey=? AND payment_hash=?",[bob.public_key().to_hex(),hash],|r|r.get(0))?
            };
            let paid: PayInvoiceResult = serde_json::from_str(
                &common::signer(&bob)?
                    .nip44_decrypt(&bob.public_key(), &String::from_utf8(checkpoint)?)
                    .await?,
            )?;
            arcadestr_core::durable_payments::validate_payment_proof(
                invoice["payment_request"]
                    .as_str()
                    .context("invoice absent")?,
                &paid,
            )?;
        }
    }
    let audit = json!({"status":"pass","signed_quote_listing_authorization_and_claim_chains":true,"live_relay_claim_copies":total_claims*4,"new_payment_dispatches":0,"additional_wallet_adapter_lookups":0});
    common::write_private(&root.join("verification.json"), &audit)?;
    Ok(audit)
}

async fn run() -> Result<()> {
    common::check_network()?;
    let budget: u64 = std::env::var("ARCADESTR_REGTEST_PAYMENT_BUDGET_MSAT")?.parse()?;
    ensure!(
        (990_000..=50_000_000).contains(&budget),
        "invalid shared Gate 1 budget"
    );
    let root = common::env_path("ARCADESTR_REGTEST_GATE1_DIR")?;
    std::fs::create_dir_all(&root)?;
    let (buyer, bob) = common::accounts().await?;
    let operator = Keys::parse(&std::env::var("ADP_OPERATOR_NSEC")?)
        .map_err(|_| anyhow!("invalid operator signer"))?;
    ensure!(
        operator.public_key().to_hex() == std::env::var("POLAR_CAROL_NOSTR_PUBKEY")?,
        "operator identity changed"
    );
    let live = unpaid(root.join("live-relays"), &buyer, &bob, &operator).await?;
    mark(&root, "live_relays", live.clone())?;
    let late = paid_case(
        root.join("late-proof"),
        "late-proof",
        &buyer,
        &bob,
        &operator,
        false,
    )
    .await?;
    mark(&root, "late_proof", late.clone())?;
    let pending = paid_case(
        root.join("pending-refund"),
        "pending-refund",
        &buyer,
        &bob,
        &operator,
        true,
    )
    .await?;
    let report = json!({"gate":"settlement-verification-1","status":"pass","live_relays":live,"paid_cases":[late,pending],
        "deadline_time":"simulated via test-owned injection; not elapsed wall time","actual_purchase_dispatches":2,"actual_refund_dispatches":2,
        "actual_refund_wallet_lookups":10,"reserved_msat":990000,"controlled_relay_adapters":false});
    common::write_private(&root.join("report.json"), &report)?;
    println!("{}", report);
    Ok(())
}
#[tokio::main]
async fn main() {
    let args: Vec<_> = std::env::args().collect();
    let result = match args.get(1).map(String::as_str) {
        Some("run") => run().await,
        Some("verify")=>async{let value=verify_existing(&common::env_path("ARCADESTR_REGTEST_GATE1_DIR")?).await?;println!("{}",value);Ok(())}.await,
        Some("continue") => async {
            let root=common::env_path("ARCADESTR_REGTEST_GATE1_DIR")?;
            let (buyer,bob)=common::accounts().await?;
            let operator=Keys::parse(&std::env::var("ADP_OPERATOR_NSEC")?)?;
            let progress:Value=serde_json::from_slice(&std::fs::read(root.join("progress.json"))?)?;
            ensure!(progress["live_relays"]["status"]=="pass" && progress["late_proof"]["status"]=="pass","cannot skip incomplete earlier stages");
            let pending=resume_pending(&root,&buyer,&bob,&operator).await?;
            let report=json!({"gate":"settlement-verification-1","status":"pass","live_relays":progress["live_relays"],"paid_cases":[progress["late_proof"],pending],
                "deadline_time":"simulated via test-owned injection; not elapsed wall time","actual_purchase_dispatches":2,"actual_refund_dispatches":2,"actual_refund_wallet_lookups":10,"reserved_msat":990000,"controlled_relay_adapters":false,"continued_original_reservation":true});
            common::write_private(&root.join("report.json"),&report)?;println!("{}",report);Ok(())
        }.await,
        Some("child") if args.len() == 4 => child(Path::new(&args[2]), &args[3]).await,
        _ => Err(anyhow!("use scripts/regtest test --gate1")),
    };
    if let Err(error) = result {
        eprintln!("Gate 1 failed: {error:#}");
        std::process::exit(1);
    }
}
