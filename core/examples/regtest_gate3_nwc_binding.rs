//! Read-only dedicated NWC binding/network audit using the private desktop keyring.
use anyhow::{ensure, Result};
use arcadestr_core::nwc_client::{
    get_nwc_connection, load_nwc_connection, NwcClient, NwcConnection,
};
use nostr::{Filter, Keys, Kind, PublicKey};
use serde_json::{json, Value};
use std::{collections::BTreeSet, path::PathBuf, time::Duration};

#[tokio::main]
async fn main() -> Result<()> {
    let root = PathBuf::from(std::env::var("ARCADESTR_REGTEST_INTEGRATION_ROOT")?);
    let buyer = PublicKey::from_hex(&std::env::var("ARCADESTR_REGTEST_BUYER_PUBKEY")?)?;
    let raw = zeroize::Zeroizing::new(std::fs::read_to_string(root.join("nwc-setup.env"))?);
    let uri = raw
        .trim()
        .strip_prefix("NWC_URI=")
        .ok_or_else(|| anyhow::anyhow!("protected NWC capture absent"))?;
    let stored = load_nwc_connection(&buyer)?;
    ensure!(
        stored == NwcConnection::parse(uri)?,
        "stored buyer credential differs from dedicated application"
    );
    for other in [
        "b33e5cb14ec49b9c309679ae0ee1aeed1f44827a1040edb82df32750d354eec8",
        "f3d701f3dd838212966180be3467b9a32076d4cf23bfe13a920ba54cb3352899",
    ] {
        ensure!(
            get_nwc_connection(&PublicKey::from_hex(other)?)?.is_none(),
            "private buyer credential leaked to another account"
        );
    }
    let nwc = NwcClient::new(stored.clone()).with_timeout(Duration::from_secs(10));
    let identity = nwc.payment_identity()?;
    nwc.verify().await?;
    let url = url::Url::parse(uri)?;
    let secret = zeroize::Zeroizing::new(
        url.query_pairs()
            .find(|(k, _)| k == "secret")
            .ok_or_else(|| anyhow::anyhow!("secret absent"))?
            .1
            .into_owned(),
    );
    let app = Keys::parse(secret.as_str())?;
    let wallet = PublicKey::from_hex(stored.wallet_pubkey_hex())?;
    let client = nostr_sdk::Client::default();
    for relay in stored.relay_urls() {
        client.add_relay(relay).await?;
    }
    client.connect().await;
    let events = client
        .fetch_events(
            Filter::new()
                .authors(vec![wallet, app.public_key()])
                .kinds(vec![
                    Kind::WalletConnectRequest,
                    Kind::WalletConnectResponse,
                ])
                .limit(100),
            Duration::from_secs(5),
        )
        .await?;
    let mut requests = BTreeSet::new();
    let mut responses = vec![];
    for event in events {
        event.verify()?;
        let Ok(text) = nostr::nips::nip44::decrypt(app.secret_key(), &wallet, &event.content)
        else {
            continue;
        };
        let Ok(payload) = serde_json::from_str::<Value>(&text) else {
            continue;
        };
        if event.pubkey == app.public_key()
            && event.kind == Kind::WalletConnectRequest
            && payload["method"] == "get_info"
        {
            requests.insert(event.id.to_hex());
        } else if event.pubkey == wallet
            && event.kind == Kind::WalletConnectResponse
            && payload["result_type"] == "get_info"
        {
            let request = event.tags.iter().find_map(|tag| {
                let v = tag.as_slice();
                (v.len() >= 2 && v[0] == "e").then(|| v[1].clone())
            });
            let recipient = event.tags.iter().any(|tag| {
                let v = tag.as_slice();
                v.len() >= 2 && v[0] == "p" && v[1] == app.public_key().to_hex()
            });
            if recipient {
                responses.push((request, payload));
            }
        }
    }
    let info = responses
        .into_iter()
        .find(|(id, _)| id.as_ref().is_some_and(|id| requests.contains(id)))
        .ok_or_else(|| {
            anyhow::anyhow!(
                "no authenticated get_info response paired with this application's request"
            )
        })?
        .1;
    let info = &info["result"];
    ensure!(info["network"] == "regtest", "NWC wallet is not regtest");
    ensure!(
        info["pubkey"].as_str()
            == Some(std::env::var("ARCADESTR_REGTEST_EXPECTED_LND_PUBKEY")?.as_str()),
        "NWC wallet is not the existing expected regtest LND node"
    );
    let name = std::env::var("ARCADESTR_REGTEST_BINDING_AUDIT_NAME")
        .unwrap_or_else(|_| "phase2-nwc-binding.json".into());
    ensure!(
        matches!(
            name.as_str(),
            "phase2-nwc-binding.json" | "phase2-nwc-binding-after-restart.json"
        ),
        "unexpected audit filename"
    );
    use std::os::unix::fs::OpenOptionsExt;
    let file = std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .mode(0o600)
        .open(root.join(name))?;
    serde_json::to_writer_pretty(
        file,
        &json!({"buyer":buyer.to_hex(),"wallet_pubkey":wallet.to_hex(),"application_pubkey":app.public_key().to_hex(),"payment_identity":identity,"stored_credential_matches_capture":true,"other_accounts_unconfigured":true,"wallet_network":"regtest","lnd_pubkey":info["pubkey"],"block_height":info["block_height"],"actual_get_info_response_verified":true,"payments":0}),
    )?;
    client.disconnect().await;
    println!("PASS stored buyer/wallet/application binding and signed regtest get_info response; other accounts unconfigured; no payment");
    Ok(())
}
