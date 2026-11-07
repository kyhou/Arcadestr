//! Reopen existing GUI fixture servers after restart/reboot. No quote/payment API.
#[allow(dead_code)]
#[path = "regtest_refunds.rs"]
mod refunds;
use anyhow::{ensure, Context, Result};
use nostr::Keys;
use refunds::common;
use serde_json::{json, Value};
use std::path::PathBuf;

async fn run() -> Result<()> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    ensure!(
        args.len() == 2 && args[0] == "--existing-fixtures",
        "existing fixture path required"
    );
    let root = PathBuf::from(&args[1]).canonicalize()?;
    let manifest: Value = serde_json::from_slice(&std::fs::read(root.join("gui-fixtures.json"))?)?;
    let (alice, bob) = common::accounts().await?;
    let operator = Keys::parse(&std::env::var("ADP_OPERATOR_NSEC")?)
        .map_err(|_| anyhow::anyhow!("original operator unavailable"))?;
    let urls = vec!["ws://127.0.0.1:10547".into(), "ws://127.0.0.1:10548".into()];
    let mut cases = Vec::new();
    for fixture in manifest["fixtures"]
        .as_array()
        .context("fixture list absent")?
    {
        let name = fixture["name"].as_str().context("fixture name absent")?;
        ensure!(
            matches!(name, "recipient-normal" | "recipient-interrupted"),
            "unexpected fixture name"
        );
        let directory = root.join(name);
        ensure!(
            directory.join("adp.db").is_file() && directory.join("client.db").is_file(),
            "original databases required"
        );
        let read = rusqlite::Connection::open_with_flags(
            directory.join("adp.db"),
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )?;
        let (clock, buyer): (i64, String) = read.query_row(
            "SELECT confirmation_expires_at+1,buyer_pubkey FROM purchase_quotes WHERE quote_id=?",
            [fixture["quote_id"].as_str().context("quote absent")?],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        ensure!(
            buyer == bob.public_key().to_hex() && clock > 0,
            "original buyer/clock mismatch"
        );
        drop(read);
        let case =
            refunds::restore_live(directory, &bob, &alice, &operator, &urls, clock as u64).await?;
        ensure!(
            Some(case.base.as_str()) == fixture["server_url"].as_str(),
            "original server binding changed"
        );
        cases.push(case);
    }
    common::write_private(
        &root.join("restored-servers.json"),
        &json!({"pid":std::process::id(),"servers":cases.len(),"payment_dispatches":0,"mode":"existing-only"}),
    )?;
    println!("Existing Gate 3 servers reopened; no invoices, quotes or payments created");
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
    if run().await.is_err() {
        eprintln!("Existing-only Gate 3 restoration blocked; original evidence retained");
        std::process::exit(1);
    }
}
