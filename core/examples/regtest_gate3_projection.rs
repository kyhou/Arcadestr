//! Explicit, offline acceptance check for the backed-up Gate 2 client projection.
//! No wallet, signer, HTTP client, or payment API is constructed.
use arcadestr_core::storage::Database;
use sqlx::{sqlite::SqliteConnectOptions, SqlitePool};
use std::{error::Error, path::PathBuf};

type Quotes = Vec<(String, String, String, Option<String>, String)>;
type Attempts = Vec<(String, String, String, Vec<u8>, Option<Vec<u8>>, String)>;

async fn evidence(pool: &SqlitePool) -> Result<(Quotes, Attempts), sqlx::Error> {
    let quotes = sqlx::query_as("SELECT account_pubkey,quote_id,quote_event_json,receipt_event_json,state FROM settlement_quotes ORDER BY account_pubkey,quote_id").fetch_all(pool).await?;
    let attempts = sqlx::query_as("SELECT account_pubkey,quote_id,attempt_id,encrypted_attempt,encrypted_proof,state FROM settlement_invoice_attempts ORDER BY account_pubkey,quote_id,attempt_id").fetch_all(pool).await?;
    Ok((quotes, attempts))
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.len() != 2 || args[0] != "--apply-backed-up-client" {
        return Err("usage: regtest_gate3_projection --apply-backed-up-client PATH".into());
    }
    let path = PathBuf::from(&args[1]).canonicalize()?;
    if !path.is_file() {
        return Err("existing client database required".into());
    }
    let readonly =
        SqlitePool::connect_with(SqliteConnectOptions::new().filename(&path).read_only(true))
            .await?;
    let before = evidence(&readonly).await?;
    let before_pending: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM settlement_legs WHERE state='pending'")
            .fetch_one(&readonly)
            .await?;
    readonly.close().await;
    let db = Database::new(&path).await?;
    if evidence(db.pool()).await? != before {
        return Err("signed quote, receipt, attempt, proof, or authoritative state changed".into());
    }
    let pending: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM settlement_legs WHERE state='pending'")
            .fetch_one(db.pool())
            .await?;
    let settled: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM settlement_legs WHERE state='settled'")
            .fetch_one(db.pool())
            .await?;
    println!("PASS original quote/receipt/proof bytes and authoritative states unchanged");
    println!("Projection: pending {before_pending} -> {pending}; settled {settled}; payment operations 0");
    db.pool().close().await;
    Ok(())
}
