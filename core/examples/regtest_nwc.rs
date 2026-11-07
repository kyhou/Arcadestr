//! Read-only verification of the existing account-scoped NWC connection.
//! No connection creation, payment command, credentials in argv or raw errors.
use arcadestr_core::nwc_client::{load_nwc_connection, NwcClient, NwcClientError};
use nostr::PublicKey;
use std::time::Duration;

fn reason(error: NwcClientError) -> &'static str {
    match error {
        NwcClientError::NotConfigured => "not_configured",
        NwcClientError::Keyring(_) => "keyring_locked_or_unavailable",
        NwcClientError::WalletTimeout => "wallet_timeout",
        NwcClientError::Relay(_) => "relay_unavailable",
        NwcClientError::VerificationUnsupported => "verification_unsupported",
        NwcClientError::WalletError { .. } => "wallet_rejected_request",
        _ => "invalid_connection_or_response",
    }
}

#[tokio::main]
async fn main() {
    let result = async {
        let account =
            std::env::var("ARCADESTR_REGTEST_BUYER_PUBKEY").map_err(|_| "buyer_pubkey_missing")?;
        let account = PublicKey::from_hex(&account).map_err(|_| "buyer_pubkey_invalid")?;
        let connection = load_nwc_connection(&account).map_err(reason)?;
        let client = NwcClient::new(connection).with_timeout(Duration::from_secs(10));
        if std::env::args().nth(1).as_deref() == Some("lookup") {
            use sha2::Digest;
            let hash = std::env::var("ARCADESTR_REGTEST_LOOKUP_HASH")
                .map_err(|_| "lookup_hash_missing")?;
            let amount: u64 = std::env::var("ARCADESTR_REGTEST_LOOKUP_AMOUNT_MSAT")
                .map_err(|_| "lookup_amount_missing")?
                .parse()
                .map_err(|_| "lookup_amount_invalid")?;
            let result = client
                .lookup_payment(&hash, amount)
                .await
                .map_err(reason)?
                .ok_or("lookup_unresolved")?;
            let bytes = hex::decode(result.preimage).map_err(|_| "lookup_proof_invalid")?;
            if bytes.len() != 32 || hex::encode(sha2::Sha256::digest(bytes)) != hash {
                return Err("lookup_proof_invalid");
            }
            return Ok(());
        }
        client.verify().await.map_err(reason)
    }
    .await;
    match result {
        Ok(()) => println!("{}", serde_json::json!({"status":"verified"})),
        Err(reason) => {
            println!(
                "{}",
                serde_json::json!({"status":"blocked", "reason":reason})
            );
            std::process::exit(1);
        }
    }
}
