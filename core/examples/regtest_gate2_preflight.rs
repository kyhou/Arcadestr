//! Read-only Gate 2 signer preflight. Never creates identities, ADP servers,
//! announcements, invoices, payment intents or reservations. No secret output.
use anyhow::{anyhow, ensure, Context, Result};
use arcadestr_core::auth::{Account, AccountManager};
use nostr::{Keys, PublicKey};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use zeroize::Zeroizing;

pub(crate) fn private_file(path: &Path) -> Result<Zeroizing<String>> {
    use std::os::unix::fs::PermissionsExt;
    let canonical = path
        .canonicalize()
        .context("private signer path unavailable")?;
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .context("workspace")?
        .canonicalize()?;
    let sibling = workspace
        .parent()
        .context("repository parent")?
        .join("adp-server");
    ensure!(
        !canonical.starts_with(&workspace) && !canonical.starts_with(sibling),
        "signer credentials must remain outside both repositories"
    );
    let metadata = std::fs::metadata(&canonical)?;
    ensure!(
        metadata.is_file() && metadata.permissions().mode() & 0o077 == 0 && metadata.len() <= 4096,
        "signer file must be small and owner-only"
    );
    let contents = Zeroizing::new(std::fs::read_to_string(&canonical)?);
    let lines: Vec<_> = contents
        .lines()
        .map(str::trim)
        .filter(|s| !s.is_empty() && !s.starts_with('#'))
        .collect();
    ensure!(
        lines.len() == 1,
        "signer file must contain one raw key or literal LSP_SEC assignment"
    );
    let line = lines[0].strip_prefix("export ").unwrap_or(lines[0]);
    let key = line
        .strip_prefix("LSP_SEC=")
        .or_else(|| line.strip_prefix("ADP_OPERATOR_NSEC="))
        .unwrap_or(line)
        .trim()
        .trim_matches(['\'', '"']);
    Ok(Zeroizing::new(key.to_owned()))
}
fn verify(secret: &str, expected: PublicKey) -> Result<()> {
    let keys = Keys::parse(secret).map_err(|_| {
        anyhow!("existing Dave signer is not a supported local key; secret suppressed")
    })?;
    ensure!(
        keys.public_key() == expected,
        "existing signer does not match established Dave public identity"
    );
    Ok(())
}
async fn inspect() -> Result<Value> {
    let expected = PublicKey::from_hex(&std::env::var("POLAR_DAVE_NOSTR_PUBKEY")?)?;
    for role in ["ALICE", "BOB", "CAROL"] {
        ensure!(
            expected.to_hex() != std::env::var(format!("POLAR_{role}_NOSTR_PUBKEY"))?,
            "Dave must be a distinct operator identity"
        );
    }
    if let Ok(path) = std::env::var("ARCADESTR_REGTEST_DAVE_SIGNER_FILE") {
        let secret = private_file(Path::new(&path))?;
        verify(&secret, expected)?;
        return Ok(
            json!({"status":"signer_verified","source":"external owner-only file","operator_pubkey":expected.to_hex(),"new_identities":0,"new_payments":0}),
        );
    }
    // An explicit persisted identity wins over an obsolete transient shell key.
    if let Ok(secret) = std::env::var("LSP_SEC") {
        let secret = Zeroizing::new(secret);
        verify(&secret, expected)?;
        return Ok(
            json!({"status":"signer_verified","source":"inherited LSP_SEC","operator_pubkey":expected.to_hex(),"new_identities":0,"new_payments":0}),
        );
    }
    let directory = PathBuf::from(
        std::env::var("ARCADESTR_REGTEST_DAVE_ACCOUNT_DIR")
            .or_else(|_| std::env::var("ARCADESTR_REGTEST_DATA_DIR"))?,
    );
    ensure!(
        directory.join("accounts.db").is_file() && directory.join(".master_key").is_file(),
        "existing account store required; refusing to create storage"
    );
    let exists = {
        let db = rusqlite::Connection::open_with_flags(
            directory.join("accounts.db"),
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )?;
        db.query_row(
            "SELECT COUNT(*) FROM accounts WHERE pubkey=?",
            [expected.to_hex()],
            |r| r.get::<_, i64>(0),
        )? > 0
    };
    if exists {
        let manager = AccountManager::new(&directory)
            .await
            .map_err(|_| anyhow!("existing Dave account could not be opened"))?;
        let account = sqlx::query_as::<_, Account>("SELECT * FROM accounts WHERE pubkey=?")
            .bind(expected.to_hex())
            .fetch_one(manager.db_pool())
            .await?;
        let secret = manager.get_nsec(&account).map_err(|_| {
            anyhow!("existing Dave account could not be unlocked; no replacement will be generated")
        })?;
        verify(secret.as_str(), expected)?;
        return Ok(
            json!({"status":"signer_verified","source":"existing account store","operator_pubkey":expected.to_hex(),"new_identities":0,"new_payments":0}),
        );
    }
    Ok(
        json!({"status":"blocked","reason":"established Dave signer unavailable; inherited LSP_SEC absent, no private file configured, no matching existing account",
        "operator_pubkey":expected.to_hex(),"new_identities":0,"new_payments":0,"secrets_printed":false}),
    )
}
#[tokio::main]
async fn main() {
    match inspect().await {
        Ok(report) => println!("{}", report),
        Err(_) => {
            eprintln!("Gate 2 signer preflight failed; private detail suppressed; no identity or payment created");
            std::process::exit(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    #[test]
    fn signer_file_must_be_owner_only_and_identity_must_match() {
        let dir = tempfile::tempdir().expect("private test directory");
        let path = dir.path().join("synthetic.key");
        let secret = "01".repeat(32);
        std::fs::write(&path, &secret).expect("synthetic fixture");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).expect("mode");
        assert!(private_file(&path).is_err());
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).expect("mode");
        let value = private_file(&path).expect("owner-only fixture");
        let keys = Keys::parse(&secret).expect("synthetic key");
        verify(&value, keys.public_key()).expect("same public identity");
        let other = Keys::parse(&"02".repeat(32)).expect("second synthetic identity");
        let error = verify(&value, other.public_key()).expect_err("identity mismatch");
        assert!(!error.to_string().contains(&secret));
    }
}
