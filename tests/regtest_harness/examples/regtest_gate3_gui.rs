//! Explicit real-principal fixture for recipient GUI acceptance.
//! Existing Bob pays Alice once per isolated order; only the GUI may refund.
//! Event lookup and deadline advancement use the established controlled adapter.
#[allow(dead_code)]
#[path = "regtest_refunds.rs"]
mod refunds;
use anyhow::{ensure, Context, Result};
use arcadestr_core::{
    adp_client::{CreateSettlementQuoteRequest, SettlementProofRequest},
    adp_settlement::{verify_quote_event, QuoteState, RefundDestination},
    signers::NostrSigner,
};
use nostr::{Event, Keys};
use refunds::common;
use serde_json::json;
use std::sync::atomic::Ordering;

async fn run() -> Result<()> {
    ensure!(
        std::env::var("ARCADESTR_REGTEST_GATE3_PAYMENTS").as_deref() == Ok("1"),
        "explicit Gate 3 payment opt-in required"
    );
    let root = common::env_path("ARCADESTR_REGTEST_GATE3_DIR")?;
    ensure!(
        root.join("payment-authorization.json").is_file(),
        "private budget reservation required"
    );
    // A crash never permits this fixture to redispatch a principal on rerun.
    let _once = std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(root.join("principal-fixture-started"))?;
    let nodes = common::check_network()?;
    let before = common::balances()?;
    let (alice, bob) = common::accounts().await?;
    let operator = Keys::parse(&std::env::var("ADP_OPERATOR_NSEC")?)
        .map_err(|_| anyhow::anyhow!("existing operator unavailable"))?;
    let buyer = common::signer(&bob)?;
    let recipient = common::signer(&alice)?;
    let mut cases = Vec::new();
    let mut reports = Vec::new();
    for name in ["recipient-normal", "recipient-interrupted"] {
        let case =
            refunds::setup_recipient_gui(root.join(name), name, &bob, &alice, &operator).await?;
        if name == "recipient-interrupted" {
            std::fs::write(
                case.root.join("gui-pause-refund-confirmation"),
                b"controlled confirmation outage; no payment endpoint is intercepted",
            )?;
        }
        let terms: Event = serde_json::from_slice(&std::fs::read(case.root.join("terms.json"))?)?;
        let id = format!("regtest-gate3-{}", uuid::Uuid::new_v4());
        let response = case
            .client
            .create_settlement_quote(
                &buyer,
                CreateSettlementQuoteRequest {
                    purchase_id: id.clone(),
                    listing_event: case.listing.clone(),
                    terms_events: vec![terms],
                    refund_destination: RefundDestination {
                        endpoint: "bob@127.0.0.1:18080".into(),
                        min_sendable_msat: 1000,
                        max_sendable_msat: 10000000,
                    },
                    zap_requests: Default::default(),
                },
            )
            .await
            .map_err(common::safe_adp)?;
        let quote = verify_quote_event(
            &response.quote,
            &buyer
                .nip44_decrypt(&response.quote.pubkey, &response.quote.content)
                .await?,
        )?;
        ensure!(
            quote.gross_msat == 200000
                && quote.legs.len() == 2
                && quote.legs[0].recipient_pubkey == alice.public_key().to_hex(),
            "recipient/amount binding differs"
        );
        let invoice = case
            .client
            .settlement_next_invoice(&buyer, &id)
            .await
            .map_err(common::safe_adp)?;
        common::validate_invoice(&invoice.bolt11, &nodes["alice"], 100000)?;
        ensure!(
            common::check_network()? == nodes,
            "regtest identities changed"
        );
        common::write_private(
            &case.root.join("principal-intent.json"),
            &json!({"invoice":invoice,"dispatch_admitted":1,"payer":"bob"}),
        )?;
        let paid = common::lnd(
            "bob",
            "payinvoice",
            &[
                "--force",
                "--json",
                "--fee_limit=10",
                "--timeout=30s",
                &format!("--pay_req={}", invoice.bolt11),
            ],
        )?;
        common::write_private(&case.root.join("principal-payment.json"), &paid)?;
        ensure!(
            paid["status"] == "SUCCEEDED",
            "principal outcome unresolved; original intent retained, do not rerun"
        );
        let preimage = paid["payment_preimage"]
            .as_str()
            .context("principal proof unavailable")?;
        let partial = case
            .client
            .submit_settlement_proof(
                &buyer,
                &id,
                SettlementProofRequest {
                    attempt_id: invoice.attempt_id.clone(),
                    preimage: preimage.into(),
                },
            )
            .await
            .map_err(common::safe_adp)?;
        ensure!(
            partial.state == "active"
                && partial.receipt.is_none()
                && partial.download_token.is_none(),
            "partial order issued ownership"
        );
        case.clock
            .store(quote.confirmation_expires_at + 1, Ordering::SeqCst);
        let failed = case
            .client
            .settlement_status(&buyer, &id)
            .await
            .map_err(common::safe_adp)?;
        ensure!(
            failed.state == QuoteState::Failed
                && case
                    .store
                    .settlement_quote(&id)
                    .await?
                    .context("stored failed quote")?
                    .receipt
                    .is_none(),
            "isolated order did not fail without ownership"
        );
        let obligations = case
            .client
            .refund_obligations(&recipient, None)
            .await
            .map_err(common::safe_adp)?;
        ensure!(
            obligations.obligations.len() == 1
                && obligations.obligations[0].principal_msat == 100000,
            "original principal obligation missing"
        );
        common::write_private(&case.root.join("original-quote.json"), &response.quote)?;
        reports.push(json!({"name":name,"server_url":case.base,"quote_id":id,"leg_id":invoice.leg_id,"principal_msat":100000,"recipient":alice.public_key().to_hex(),"buyer":bob.public_key().to_hex(),"principal_dispatches":1,"refund_dispatches":0,"event_adapter":"controlled","deadline":"injected"}));
        cases.push(case);
    }
    common::write_private(
        &root.join("gui-fixtures.json"),
        &json!({"fixtures":reports,"balances_before":before,"balances_after_principals":common::balances()?}),
    )?;
    println!("READY two isolated GUI refund obligations; Bob -> Alice 100 sats each; no refunds dispatched");
    tokio::signal::ctrl_c().await?;
    for mut case in cases {
        if let Some(stop) = case.stop.take() {
            let _ = stop.send(());
        }
        let _ = case.task.await;
    }
    Ok(())
}

#[tokio::main]
async fn main() {
    if let Err(error) = run().await {
        eprintln!("Gate 3 fixture blocked: {error}");
        std::process::exit(1);
    }
}
