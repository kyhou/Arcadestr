//! Read-only local relay diagnosis; never saves parameters, results or credentials.
use anyhow::Result;
use nostr::{Filter, Keys, Kind, PublicKey, Timestamp};
use serde_json::{json, Value};
use std::{path::PathBuf, time::Duration};

#[tokio::main]
async fn main() -> Result<()> {
    let root = PathBuf::from(std::env::var("ARCADESTR_REGTEST_DENIAL_ROOT")?);
    let secret = zeroize::Zeroizing::new(std::env::var("ARCADESTR_REGTEST_DENIAL_IDENTITY_KEY")?);
    let keys = Keys::parse(secret.as_str())?;
    let app = PublicKey::from_hex(&std::env::var("ARCADESTR_REGTEST_DENIAL_APP")?)?;
    let since: u64 = std::env::var("ARCADESTR_REGTEST_SESSION_SINCE")?.parse()?;
    let client = nostr_sdk::Client::default();
    client.add_relay("ws://127.0.0.1:7778").await?;
    client.connect().await;
    let events = client
        .fetch_events(
            Filter::new()
                .kind(Kind::Custom(24133))
                .authors(vec![keys.public_key(), app])
                .since(Timestamp::from(since))
                .limit(100),
            Duration::from_secs(5),
        )
        .await?;
    let mut summary = vec![];
    for event in events {
        event.verify()?;
        let Ok(plaintext) = nostr::nips::nip44::decrypt(keys.secret_key(), &app, &event.content)
        else {
            continue;
        };
        let Ok(message) = serde_json::from_str::<Value>(&plaintext) else {
            continue;
        };
        let error = message["error"].as_str().map(|e| match e {
            "denied: no matching pairing in progress" => e,
            "denied: rejected" => e,
            _ => "other remote error (details redacted)",
        });
        summary.push(
            json!({"id":message["id"],"method":message["method"],"error":error,
            "request":event.pubkey==app,"event_id":event.id.to_hex()}),
        );
    }
    let path = root.join("phase1-restored-wire-baseline.json");
    use std::os::unix::fs::OpenOptionsExt;
    let file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)?;
    serde_json::to_writer_pretty(file, &summary)?;
    client.disconnect().await;
    println!("Retained verified local-relay request/error metadata only; no credentials or plaintext results");
    Ok(())
}
