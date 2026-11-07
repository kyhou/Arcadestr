//! Existing connection, one standard get_info request, no payments or invoices.
use arcadestr_core::nwc_client::{NwcClient, NwcConnection};
use std::{path::PathBuf, time::Duration};

#[tokio::main]
async fn main() {
    if probe().await.is_err() {
        eprintln!("INCONCLUSIVE: read-only NWC capability probe unavailable; no payment, invoice or permission change attempted");
        std::process::exit(1)
    }
}
async fn probe() -> Result<(), Box<dyn std::error::Error>> {
    let root = PathBuf::from(std::env::var("ARCADESTR_REGTEST_INTEGRATION_ROOT")?);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if root.metadata()?.permissions().mode() & 0o077 != 0 {
            return Err("owner-only evidence directory required".into());
        }
    }
    let binding: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("phase2-nwc-binding.json"))?)?;
    let capture = std::fs::read_to_string(root.join("nwc-setup.env"))?;
    let (_, uri) = capture
        .trim()
        .split_once('=')
        .ok_or("invalid private capture")?;
    let connection = NwcConnection::parse(uri).map_err(|_| "invalid private connection")?;
    if connection
        .relay_urls()
        .iter()
        .any(|url| !url.starts_with("ws://127.0.0.1:") && !url.starts_with("ws://localhost:"))
    {
        return Err("only preserved loopback regtest connection allowed".into());
    }
    let client = NwcClient::new(connection).with_timeout(Duration::from_secs(7));
    if Some(client.payment_identity()?.as_str()) != binding["payment_identity"].as_str() {
        return Err("original wallet/application identity changed".into());
    }
    let info = client
        .capabilities()
        .await
        .map_err(|_| "get_info unavailable")?;
    let result = serde_json::json!({"standard_method":"get_info","original_connection_identity_verified":true,"network":info.network,"methods":info.methods,"payment_dispatches":0,"live_invoices_created":0,"permissions_changed":false,"fee_ceiling_established":false});
    let path = root.join(format!(
        "standard-nwc-capabilities-{}.json",
        nostr::Timestamp::now().as_secs()
    ));
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    serde_json::to_writer_pretty(options.open(path)?, &result)?;
    println!("PASS read-only standard NWC get_info; original connection retained; no payment/permission/invoice action");
    Ok(())
}
