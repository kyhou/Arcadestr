use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use arcadestr_core::adp_storage::{InstalledGame, InstalledGamesRepository};
use arcadestr_core::file_hash::sha256_file;

#[allow(dead_code)]
pub(crate) async fn verify_and_record_downloaded_game(
    installed_games: &InstalledGamesRepository,
    game_coordinate: &str,
    dest_path: &Path,
    expected_hash: &str,
    version: Option<String>,
    server_url: &str,
) -> Result<(), String> {
    let actual_hash = sha256_file(dest_path).await.map_err(|err| {
        format!(
            "failed to compute downloaded file hash for {}: {err}",
            dest_path.display()
        )
    })?;

    if actual_hash != expected_hash {
        let quarantine_path = available_corrupt_artifact_path(dest_path).await?;
        tokio::fs::rename(dest_path, &quarantine_path)
            .await
            .map_err(|err| {
                format!(
                    "downloaded file hash mismatch: expected {expected_hash}, got {actual_hash}; failed to quarantine at {}: {err}",
                    quarantine_path.display()
                )
            })?;
        return Err(format!(
            "downloaded file hash mismatch: expected {expected_hash}, got {actual_hash}; quarantined at {}",
            quarantine_path.display()
        ));
    }

    let installed_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|err| err.to_string())?
        .as_secs() as i64;

    installed_games
        .record(&InstalledGame {
            game_coordinate: game_coordinate.to_string(),
            file_path: dest_path.to_path_buf(),
            file_hash: expected_hash.to_string(),
            version,
            server_url: server_url.to_string(),
            installed_at,
        })
        .await
        .map_err(|err| err.to_string())
}

#[allow(dead_code)]
pub(crate) fn corrupt_artifact_path(dest_path: &Path) -> PathBuf {
    corrupt_artifact_path_with_suffix(dest_path, None)
}

fn corrupt_artifact_path_with_suffix(dest_path: &Path, suffix: Option<usize>) -> PathBuf {
    let corrupt_name = dest_path
        .file_name()
        .map(|name| match suffix {
            Some(index) => format!("{}.corrupt.{index}", name.to_string_lossy()),
            None => format!("{}.corrupt", name.to_string_lossy()),
        })
        .unwrap_or_else(|| "artifact.corrupt".to_string());
    dest_path.with_file_name(corrupt_name)
}

async fn available_corrupt_artifact_path(dest_path: &Path) -> Result<PathBuf, String> {
    let primary = corrupt_artifact_path(dest_path);
    if !tokio::fs::try_exists(&primary)
        .await
        .map_err(|err| format!("failed to inspect quarantine path {}: {err}", primary.display()))?
    {
        return Ok(primary);
    }

    for index in 1..=1024 {
        let candidate = corrupt_artifact_path_with_suffix(dest_path, Some(index));
        if !tokio::fs::try_exists(&candidate).await.map_err(|err| {
            format!(
                "failed to inspect quarantine path {}: {err}",
                candidate.display()
            )
        })? {
            return Ok(candidate);
        }
    }

    Err(format!(
        "no available quarantine path for {} after 1024 attempts",
        dest_path.display()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::path::{Path, PathBuf};
    use std::time::{SystemTime, UNIX_EPOCH};

    use arcadestr_core::adp_storage::InstalledGamesRepository;
    use arcadestr_core::file_hash::sha256_file;
    use arcadestr_core::storage::Database;

    fn unique_test_dir(test_name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "arcadestr-desktop-install-{test_name}-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system clock should be after epoch")
                .as_nanos()
        ))
    }

    async fn test_db(dir: &Path) -> Database {
        Database::new(&dir.join("install.db"))
            .await
            .expect("test database should open")
    }

    async fn sha256_hex(bytes: &[u8], dir: &Path) -> String {
        let path = dir.join("expected.bin");
        tokio::fs::write(&path, bytes)
            .await
            .expect("expected bytes should be written");
        sha256_file(&path)
            .await
            .expect("expected hash should compute")
    }

    #[tokio::test]
    async fn install_records_game_when_hash_matches() {
        let dir = unique_test_dir("hash-match");
        tokio::fs::create_dir_all(&dir)
            .await
            .expect("test dir should be created");
        let db = test_db(&dir).await;
        let repo = InstalledGamesRepository::new(db.pool().clone());
        let expected_bytes = b"verified artifact bytes";
        let expected_hash = sha256_hex(expected_bytes, &dir).await;
        let coordinate = "30402:publisher:verified-game";
        let server_url = "https://dist.example.com";
        let final_install_path = dir.join("verified-game.zip");
        tokio::fs::write(&final_install_path, expected_bytes)
            .await
            .expect("downloaded artifact should be written");

        verify_and_record_downloaded_game(
            &repo,
            coordinate,
            &final_install_path,
            &expected_hash,
            Some("1.0.0".to_string()),
            server_url,
        )
        .await
        .expect("hash-matched artifact should record");

        let installed = repo
            .get(coordinate)
            .await
            .expect("lookup should work")
            .expect("installed game should be recorded");
        assert_eq!(installed.file_hash, expected_hash);
        assert_eq!(installed.server_url, server_url);
        assert!(installed.file_path.exists());

        db.close().await;
        tokio::fs::remove_dir_all(&dir)
            .await
            .expect("test dir should be removed");
    }

    #[tokio::test]
    async fn install_quarantines_hash_mismatch_without_recording_installed_game() {
        let dir = unique_test_dir("hash-mismatch");
        tokio::fs::create_dir_all(&dir)
            .await
            .expect("test dir should be created");
        let db = test_db(&dir).await;
        let repo = InstalledGamesRepository::new(db.pool().clone());
        let expected_bytes = b"verified artifact bytes";
        let expected_hash = sha256_hex(expected_bytes, &dir).await;
        let coordinate = "30402:publisher:corrupt-game";
        let final_install_path = dir.join("corrupt-game.zip");
        let quarantine_path = corrupt_artifact_path(&final_install_path);
        tokio::fs::write(&final_install_path, b"corrupt bytes")
            .await
            .expect("corrupt artifact should be written");

        let err = verify_and_record_downloaded_game(
            &repo,
            coordinate,
            &final_install_path,
            &expected_hash,
            None,
            "https://dist.example.com",
        )
        .await
        .expect_err("hash mismatch should fail");

        assert!(err.contains("hash"));
        assert!(repo
            .get(coordinate)
            .await
            .expect("lookup should work")
            .is_none());
        assert!(quarantine_path.exists());
        assert!(!final_install_path.exists());

        db.close().await;
        tokio::fs::remove_dir_all(&dir)
            .await
            .expect("test dir should be removed");
    }
}
