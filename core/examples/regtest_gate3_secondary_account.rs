//! Reuse an existing local identity from a private account-store copy. Never pays.
use anyhow::{ensure, Result};
use arcadestr_core::auth::{Account, AccountManager};
use nostr::Keys;
use std::{io::Write, path::PathBuf};

#[tokio::main]
async fn main() -> Result<()> {
    let root = PathBuf::from(std::env::var("ARCADESTR_REGTEST_EXISTING_LOCAL_COPY")?);
    ensure!(
        root.join("accounts.db").is_file() && root.join(".master_key").is_file(),
        "private existing-account copy required"
    );
    let manager = AccountManager::new(&root).await?;
    let public = "b33e5cb14ec49b9c309679ae0ee1aeed1f44827a1040edb82df32750d354eec8";
    let account = sqlx::query_as::<_, Account>("SELECT * FROM accounts WHERE pubkey=?")
        .bind(public)
        .fetch_one(manager.db_pool())
        .await?;
    let secret = manager.get_nsec(&account)?;
    ensure!(
        Keys::parse(secret.as_str())?.public_key().to_hex() == public,
        "existing secondary identity mismatch"
    );
    use std::os::unix::fs::OpenOptionsExt;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(root.join("secondary-identity-private"))?;
    file.write_all(secret.as_bytes())?;
    println!("Existing secondary public identity verified; protected private input retained; no payment or original-store writes");
    Ok(())
}
