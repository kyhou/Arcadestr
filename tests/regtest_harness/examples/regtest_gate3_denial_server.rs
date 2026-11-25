//! Isolated non-paying quote fixture for real GUI NIP-46 refusal checks.
#[allow(dead_code)]
#[path = "regtest_refunds.rs"]
mod refunds;
use anyhow::{ensure, Result};
use nostr::{nips::nip19::ToBech32, Keys};
use serde_json::json;
use std::path::PathBuf;

#[tokio::main]
async fn main() -> Result<()> {
    let root = PathBuf::from(std::env::var("ARCADESTR_REGTEST_DENIAL_ROOT")?);
    ensure!(root.is_dir(), "existing private test directory required");
    let _once = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(root.join("nonpaying-server-started"))?;
    let publisher = Keys::generate();
    let operator = Keys::generate();
    let unused_buyer = Keys::generate();
    std::fs::create_dir_all(root.join("fixture"))?;
    std::fs::write(
        root.join("fixture/gui-pause-invoice-preparation"),
        b"No Lightning invoice preparation in denial tests",
    )?;
    refunds::common::write_private(
        &root.join("fixture-keys.json"),
        &json!({
            "publisher":publisher.secret_key().to_secret_hex(),
            "operator":operator.secret_key().to_secret_hex(),
            "unused_fixture_owner":unused_buyer.secret_key().to_secret_hex()
        }),
    )?;
    use arcadestr_core::adp_settlement::{
        build_terms_event_builder, CommercialTerms, FeePolicy, TermsPayload,
    };
    let terms = build_terms_event_builder(&TermsPayload::Offer {
        terms: CommercialTerms {
            version: 1,
            operator_pubkey: operator.public_key().to_hex(),
            payment_destination: "carol@127.0.0.1:18080".into(),
            fee: FeePolicy::Fixed {
                amount_msat: 100000,
            },
            limitations: vec!["non-paying GUI refusal fixture".into()],
        },
    })?
    .sign_with_keys(&operator)?;
    let case = refunds::setup_remote_buyer_gui(
        root.join("fixture"),
        &unused_buyer,
        &publisher,
        &operator,
        terms.clone(),
    )
    .await?;
    let client = nostr_sdk::Client::new(operator);
    client.add_relay("ws://127.0.0.1:7778").await?;
    client.connect().await;
    let authorization: nostr::Event =
        serde_json::from_slice(&std::fs::read(case.root.join("authorization.json"))?)?;
    for event in [&terms, &authorization, &case.listing] {
        client.send_event(event).await?;
    }
    refunds::common::write_private(
        &root.join("nonpaying-fixture.json"),
        &json!({
            "server_url":case.base,"publisher_npub":publisher.public_key().to_bech32()?,
            "listing_event":case.listing,"lightning_payments":0,"only_relay":"ws://127.0.0.1:7778"
        }),
    )?;
    println!("READY isolated non-paying GUI quote fixture; only local strfry 7778");
    tokio::signal::ctrl_c().await?;
    if let Some(stop) = case.stop {
        let _ = stop.send(());
    }
    let _ = case.task.await;
    client.disconnect().await;
    Ok(())
}
