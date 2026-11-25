//! NIP-46 GUI fixture server only. Never connects/re-pairs a remote signer or pays.
#[allow(dead_code)]
#[path = "regtest_refunds.rs"]
mod refunds;
use anyhow::{ensure, Context, Result};
use nostr::{Event, EventId, Filter, Keys};
use refunds::common;
use serde_json::json;
use std::{path::PathBuf, time::Duration};

async fn run() -> Result<()> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    ensure!(
        args.len() == 2 && args[0] == "--new-server",
        "explicit new isolated fixture path required"
    );
    let root = PathBuf::from(&args[1]);
    ensure!(root.is_dir(), "private evidence directory required");
    let _once = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(root.join("server-setup-started"))?;
    let (alice, bob) = common::accounts().await?;
    let operator = Keys::parse(&std::env::var("ADP_OPERATOR_NSEC")?)
        .map_err(|_| anyhow::anyhow!("existing operator unavailable"))?;
    let client = nostr_sdk::Client::new(operator.clone());
    for url in [
        "ws://127.0.0.1:7777",
        "ws://127.0.0.1:7778",
        "ws://127.0.0.1:10547",
        "ws://127.0.0.1:10548",
    ] {
        client.add_relay(url).await?;
    }
    client.connect().await;
    let id = EventId::from_hex(&std::env::var("ADP_TERMS_EVENT_ID")?)?;
    let terms = client
        .fetch_events(Filter::new().id(id), Duration::from_secs(10))
        .await?
        .into_iter()
        .find(|e| e.id == id)
        .context("original immutable terms unavailable")?;
    let case =
        refunds::setup_remote_buyer_gui(root.join("fixture"), &bob, &alice, &operator, terms)
            .await?;
    // Publish only the new, uniquely coordinated listing and authorization.
    // Never replace the operator's already-published commercial terms.
    let authorization: Event =
        serde_json::from_slice(&std::fs::read(case.root.join("authorization.json"))?)?;
    client.send_event(&authorization).await?;
    client.send_event(&case.listing).await?;
    let listing_id = case
        .listing
        .tags
        .iter()
        .find_map(|t| {
            let v = t.as_slice();
            (v.first().map(String::as_str) == Some("d"))
                .then(|| v.get(1).cloned())
                .flatten()
        })
        .context("listing d tag absent")?;
    use nostr::nips::nip19::ToBech32;
    common::write_private(
        &root.join("fixture-ready.json"),
        &json!({"server_url":case.base,"publisher_npub":alice.public_key().to_bech32()?,"listing_id":listing_id,"listing_event_id":case.listing.id.to_hex(),"coordinate":format!("30402:{}:{}",alice.public_key(),listing_id),"expected_gross_msat":200000,"expected_leg_msat":100000,"expected_recipient_nodes":["alice","carol"],"payer_node":"bob","payment_dispatches_from_server":0,"initial_signing_authorization":"existing nak bunker authorized-client list; automatic approval","manual_per_request_approval":false}),
    )?;
    println!("READY existing-authorization NIP-46 fixture; no payment or remote-signer connection created by server");
    tokio::signal::ctrl_c().await?;
    if let Some(stop) = case.stop {
        let _ = stop.send(());
    }
    let _ = case.task.await;
    client.disconnect().await;
    Ok(())
}

#[tokio::main]
async fn main() {
    if let Err(error) = run().await {
        eprintln!("NIP-46 fixture server blocked: {error:#}; no payment dispatched by this server");
        std::process::exit(1);
    }
}
