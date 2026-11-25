//! One isolated live-relay purchase fixture; this server never dispatches payments.
#[allow(dead_code)]
#[path = "regtest_refunds.rs"]
mod refunds;
use anyhow::{ensure, Result};
use nostr::{nips::nip19::ToBech32, Keys};
use serde_json::json;
use std::path::PathBuf;

async fn publish_profile(root: &std::path::Path, publisher: &Keys) -> Result<()> {
    ensure!(
        !root.join("publisher-profile.json").exists(),
        "profile publication already staged; retain evidence"
    );
    let event = nostr::EventBuilder::new(
        nostr::Kind::Metadata,
        json!({"name":"Gate3 isolated regtest publisher","lud16":"bob@127.0.0.1:18080"})
            .to_string(),
    )
    .sign_with_keys(publisher)?;
    refunds::common::write_private(&root.join("publisher-profile.json"), &event)?;
    let client = nostr_sdk::Client::new(publisher.clone());
    client.add_relay("ws://127.0.0.1:7778").await?;
    client.connect().await;
    client.send_event(&event).await?;
    client.disconnect().await;
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    let root = PathBuf::from(std::env::var("ARCADESTR_REGTEST_PURCHASE_ROOT")?);
    ensure!(root.is_dir(), "existing private fixture root required");
    if std::env::args().nth(1).as_deref() == Some("--restore-existing") {
        let keys: serde_json::Value = serde_json::from_slice(&std::fs::read(
            root.join("fixture-identities-private.json"),
        )?)?;
        let load = |name: &str| -> Result<Keys> {
            Ok(Keys::parse(keys[name].as_str().ok_or_else(|| {
                anyhow::anyhow!("existing fixture identity absent")
            })?)?)
        };
        let publisher = load("publisher")?;
        let operator = load("operator")?;
        let owner = load("fixture_owner")?;
        let case = refunds::restore_live(
            root.join("server"),
            &owner,
            &publisher,
            &operator,
            &["ws://127.0.0.1:7778".into()],
            nostr::Timestamp::now().as_secs(),
        )
        .await?;
        let manifest: serde_json::Value =
            serde_json::from_slice(&std::fs::read(root.join("purchase-fixture-ready.json"))?)?;
        ensure!(
            manifest["listing_event"]["id"].as_str() == Some(case.listing.id.to_hex().as_str())
                && manifest["server_url"].as_str() == Some(case.base.as_str()),
            "existing fixture binding changed"
        );
        println!("READY existing-only purchase fixture restored with production wall clock; original listing/invoices retained; no payment");
        tokio::signal::ctrl_c().await?;
        if let Some(stop) = case.stop {
            let _ = stop.send(());
        }
        let _ = case.task.await;
        return Ok(());
    }
    if std::env::args().nth(1).as_deref() == Some("--publish-existing-fixture-profile") {
        let keys: serde_json::Value = serde_json::from_slice(&std::fs::read(
            root.join("fixture-identities-private.json"),
        )?)?;
        let publisher = Keys::parse(
            keys["publisher"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("fixture publisher absent"))?,
        )?;
        let manifest: serde_json::Value =
            serde_json::from_slice(&std::fs::read(root.join("purchase-fixture-ready.json"))?)?;
        ensure!(
            manifest["listing_event"]["pubkey"].as_str()
                == Some(publisher.public_key().to_hex().as_str()),
            "existing fixture publisher mismatch"
        );
        publish_profile(&root, &publisher).await?;
        println!("Published existing isolated fixture's missing seller metadata; no new listing, invoice, quote or payment");
        return Ok(());
    }
    let _once = std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(root.join("fixture-started"))?;
    let publisher = Keys::generate();
    let operator = Keys::generate();
    let owner = Keys::generate();
    refunds::common::write_private(
        &root.join("fixture-identities-private.json"),
        &json!({
        "publisher":publisher.secret_key().to_secret_hex(),"operator":operator.secret_key().to_secret_hex(),
        "fixture_owner":owner.secret_key().to_secret_hex()}),
    )?;
    publish_profile(&root, &publisher).await?;
    let case =
        refunds::setup_nip46_nwc_purchase(root.join("server"), &owner, &publisher, &operator)
            .await?;
    refunds::common::write_private(
        &root.join("purchase-fixture-ready.json"),
        &json!({
        "server_url":case.base,"publisher_npub":publisher.public_key().to_bech32()?,"listing_event":case.listing,
        "gross_msat":200000,"leg_msat":100000,"recipients":["bob","carol"],"payer":"alice",
        "only_relay":"ws://127.0.0.1:7778","server_payment_dispatches":0,"phase":"pre-payment setup"}),
    )?;
    println!("READY new live-relay NIP-46/NWC purchase fixture; no payments dispatched by server");
    tokio::signal::ctrl_c().await?;
    if let Some(stop) = case.stop {
        let _ = stop.send(());
    }
    let _ = case.task.await;
    Ok(())
}
