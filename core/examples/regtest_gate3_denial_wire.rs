//! Read-only audit of actual local-relay NIP-46 traffic. Never publishes or pays.
use anyhow::{ensure, Result};
use nostr::{Filter, Keys, Kind, PublicKey};
use serde_json::{json, Value};
use std::{collections::BTreeMap, path::PathBuf, time::Duration};

#[tokio::main]
async fn main() -> Result<()> {
    let root = PathBuf::from(std::env::var("ARCADESTR_REGTEST_DENIAL_ROOT")?);
    let secret = zeroize::Zeroizing::new(std::env::var("ARCADESTR_REGTEST_DENIAL_IDENTITY_KEY")?);
    let keys = Keys::parse(secret.as_str())
        .map_err(|_| anyhow::anyhow!("isolated identity unavailable"))?;
    let app = PublicKey::from_hex(&std::env::var("ARCADESTR_REGTEST_DENIAL_APP")?)?;
    let expected: BTreeMap<String, String> =
        serde_json::from_str(&std::env::var("ARCADESTR_REGTEST_DENIAL_REQUESTS")?)?;
    let client = nostr_sdk::Client::default();
    client.add_relay("ws://127.0.0.1:7778").await?;
    client.connect().await;
    let events = client
        .fetch_events(
            Filter::new()
                .kind(Kind::Custom(24133))
                .authors(vec![keys.public_key(), app])
                .limit(500),
            Duration::from_secs(5),
        )
        .await?;
    let mut requests = BTreeMap::new();
    let mut responses = BTreeMap::new();
    for event in events {
        event.verify()?;
        let Ok(plaintext) = nostr::nips::nip44::decrypt(keys.secret_key(), &app, &event.content)
        else {
            continue;
        };
        let Ok(message) = serde_json::from_str::<Value>(&plaintext) else {
            continue;
        };
        let Some(id) = message["id"].as_str() else {
            continue;
        };
        if !expected.contains_key(id) {
            continue;
        }
        if event.pubkey == app && message["method"].is_string() {
            requests.insert(
                id.to_owned(),
                json!({"event_id":event.id.to_hex(),"method":message["method"]}),
            );
        } else if event.pubkey == keys.public_key() {
            let denied = expected[id] == "reject";
            if denied {
                ensure!(
                    message["error"].as_str() == Some("denied: rejected"),
                    "actual refusal payload differs from expected explicit denial"
                );
                ensure!(
                    message["result"].as_str().is_none_or(str::is_empty),
                    "rejected response unexpectedly returned a result"
                );
            } else {
                ensure!(
                    message["error"].as_str().is_none_or(str::is_empty)
                        && !message["result"].is_null(),
                    "approved response was not successful"
                );
            }
            responses.insert(id.to_owned(), json!({"event_id":event.id.to_hex(),"error":if denied{Some("denied: rejected")}else{None},"result_present":!message["result"].is_null()}));
        }
    }
    ensure!(
        requests.len() == expected.len() && responses.len() == expected.len(),
        "incomplete actual request/response relay evidence"
    );
    let audit = json!({"only_relay":"ws://127.0.0.1:7778","signatures_verified":true,"nip44_transport_decrypted_locally":true,"private_keys_or_plaintext_results_written":false,"requests":requests,"responses":responses});
    let path = root.join("actual-nip46-wire-verification.json");
    use std::os::unix::fs::OpenOptionsExt;
    let file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)?;
    serde_json::to_writer_pretty(file, &audit)?;
    client.disconnect().await;
    println!("PASS signed local-relay NIP-46 requests and correlated explicit-denial/success responses; no plaintext results or credentials written");
    Ok(())
}
