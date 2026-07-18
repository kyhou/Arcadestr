//! SQLite repositories for ADP client-side state.

use std::path::PathBuf;

use sqlx::{Row, SqlitePool};
use thiserror::Error;

/// Errors returned by ADP storage repositories.
#[derive(Debug, Error)]
pub enum AdpStorageError {
    /// SQLite operation failed.
    #[error("ADP storage SQL error: {0}")]
    Sql(#[from] sqlx::Error),
}

/// Stored ADP provisioning relationship.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdpProvisioning {
    pub id: String,
    pub developer_npub: String,
    pub server_url: String,
    pub operator_pubkey: String,
    pub scope: Option<String>,
    pub fulfillment_pubkey: String,
    pub attestation_event_id: String,
    pub acceptance_event_id: String,
    pub valid_from: i64,
    pub revoked_at: Option<i64>,
    pub created_at: i64,
}

/// Cached ADP download token.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DownloadToken {
    pub game_coordinate: String,
    pub server_url: String,
    pub token: String,
    pub expires_at: i64,
}

/// Installed ADP game record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstalledGame {
    pub game_coordinate: String,
    pub file_path: PathBuf,
    pub file_hash: String,
    pub version: Option<String>,
    pub server_url: String,
    pub installed_at: i64,
}

/// Repository for `adp_provisioning` rows.
pub struct AdpProvisioningRepository {
    pool: SqlitePool,
}

impl AdpProvisioningRepository {
    /// Creates a repository backed by `pool`.
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// Inserts or replaces a provisioning row.
    ///
    /// # Errors
    /// Returns [`AdpStorageError`] if SQLite fails.
    pub async fn upsert(&self, entry: &AdpProvisioning) -> Result<(), AdpStorageError> {
        sqlx::query(
            r#"INSERT OR REPLACE INTO adp_provisioning
            (id, developer_npub, server_url, operator_pubkey, scope, fulfillment_pubkey,
             attestation_event_id, acceptance_event_id, valid_from, revoked_at, created_at)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)"#,
        )
        .bind(&entry.id)
        .bind(&entry.developer_npub)
        .bind(&entry.server_url)
        .bind(&entry.operator_pubkey)
        .bind(&entry.scope)
        .bind(&entry.fulfillment_pubkey)
        .bind(&entry.attestation_event_id)
        .bind(&entry.acceptance_event_id)
        .bind(entry.valid_from)
        .bind(entry.revoked_at)
        .bind(entry.created_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Finds an active provisioning row for `(developer, server, scope)`.
    ///
    /// # Errors
    /// Returns [`AdpStorageError`] if SQLite fails.
    pub async fn active_for_scope(
        &self,
        developer_npub: &str,
        server_url: &str,
        scope: Option<&str>,
    ) -> Result<Option<AdpProvisioning>, AdpStorageError> {
        let row = sqlx::query(
            r#"SELECT id, developer_npub, server_url, operator_pubkey, scope,
               fulfillment_pubkey, attestation_event_id, acceptance_event_id,
               valid_from, revoked_at, created_at
               FROM adp_provisioning
               WHERE developer_npub = ? AND server_url = ?
                 AND COALESCE(scope, '') = COALESCE(?, '')
                 AND revoked_at IS NULL"#,
        )
        .bind(developer_npub)
        .bind(server_url)
        .bind(scope)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(adp_provisioning_from_row))
    }

    /// Finds all provisioning rows matching a developer, fulfillment key, and scope.
    ///
    /// Revoked rows are included so callers can resolve historical listing metadata.
    ///
    /// # Errors
    /// Returns [`AdpStorageError`] if SQLite fails.
    pub async fn for_fulfillment_scope(
        &self,
        developer_npub: &str,
        fulfillment_pubkey: &str,
        scope: &str,
    ) -> Result<Vec<AdpProvisioning>, AdpStorageError> {
        let rows = sqlx::query(
            r#"SELECT id, developer_npub, server_url, operator_pubkey, scope,
               fulfillment_pubkey, attestation_event_id, acceptance_event_id,
               valid_from, revoked_at, created_at
               FROM adp_provisioning
               WHERE developer_npub = ? AND fulfillment_pubkey = ? AND scope = ?"#,
        )
        .bind(developer_npub)
        .bind(fulfillment_pubkey)
        .bind(scope)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows.into_iter().map(adp_provisioning_from_row).collect())
    }

    /// Marks all rows for `fulfillment_pubkey` revoked.
    ///
    /// # Errors
    /// Returns [`AdpStorageError`] if SQLite fails.
    pub async fn mark_revoked(
        &self,
        fulfillment_pubkey: &str,
        revoked_at: i64,
    ) -> Result<(), AdpStorageError> {
        sqlx::query("UPDATE adp_provisioning SET revoked_at = ? WHERE fulfillment_pubkey = ?")
            .bind(revoked_at)
            .bind(fulfillment_pubkey)
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}

/// Repository for `download_tokens` rows.
pub struct DownloadTokensRepository {
    pool: SqlitePool,
}

impl DownloadTokensRepository {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn upsert(&self, token: &DownloadToken) -> Result<(), AdpStorageError> {
        sqlx::query(
            r#"INSERT OR REPLACE INTO download_tokens
            (game_coordinate, server_url, token, expires_at) VALUES (?, ?, ?, ?)"#,
        )
        .bind(&token.game_coordinate)
        .bind(&token.server_url)
        .bind(&token.token)
        .bind(token.expires_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn valid_token(
        &self,
        game_coordinate: &str,
        server_url: &str,
        now: i64,
    ) -> Result<Option<DownloadToken>, AdpStorageError> {
        let row = sqlx::query(
            "SELECT game_coordinate, server_url, token, expires_at FROM download_tokens \
             WHERE game_coordinate = ? AND server_url = ? AND expires_at > ?",
        )
        .bind(game_coordinate)
        .bind(server_url)
        .bind(now)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(download_token_from_row))
    }

    pub async fn delete(
        &self,
        game_coordinate: &str,
        server_url: &str,
    ) -> Result<(), AdpStorageError> {
        sqlx::query("DELETE FROM download_tokens WHERE game_coordinate = ? AND server_url = ?")
            .bind(game_coordinate)
            .bind(server_url)
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}

/// Repository for `installed_games` rows.
pub struct InstalledGamesRepository {
    pool: SqlitePool,
}

impl InstalledGamesRepository {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn record(&self, entry: &InstalledGame) -> Result<(), AdpStorageError> {
        sqlx::query(
            r#"INSERT OR REPLACE INTO installed_games
            (game_coordinate, file_path, file_hash, version, server_url, installed_at)
            VALUES (?, ?, ?, ?, ?, ?)"#,
        )
        .bind(&entry.game_coordinate)
        .bind(entry.file_path.to_string_lossy().as_ref())
        .bind(&entry.file_hash)
        .bind(&entry.version)
        .bind(&entry.server_url)
        .bind(entry.installed_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn get(&self, coordinate: &str) -> Result<Option<InstalledGame>, AdpStorageError> {
        let row = sqlx::query(
            "SELECT game_coordinate, file_path, file_hash, version, server_url, installed_at \
             FROM installed_games WHERE game_coordinate = ?",
        )
        .bind(coordinate)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(installed_game_from_row))
    }

    pub async fn list(&self) -> Result<Vec<InstalledGame>, AdpStorageError> {
        let rows = sqlx::query(
            "SELECT game_coordinate, file_path, file_hash, version, server_url, installed_at \
             FROM installed_games ORDER BY installed_at DESC",
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.into_iter().map(installed_game_from_row).collect())
    }
}

fn adp_provisioning_from_row(row: sqlx::sqlite::SqliteRow) -> AdpProvisioning {
    AdpProvisioning {
        id: row.get("id"),
        developer_npub: row.get("developer_npub"),
        server_url: row.get("server_url"),
        operator_pubkey: row.get("operator_pubkey"),
        scope: row.get("scope"),
        fulfillment_pubkey: row.get("fulfillment_pubkey"),
        attestation_event_id: row.get("attestation_event_id"),
        acceptance_event_id: row.get("acceptance_event_id"),
        valid_from: row.get("valid_from"),
        revoked_at: row.get("revoked_at"),
        created_at: row.get("created_at"),
    }
}

fn download_token_from_row(row: sqlx::sqlite::SqliteRow) -> DownloadToken {
    DownloadToken {
        game_coordinate: row.get("game_coordinate"),
        server_url: row.get("server_url"),
        token: row.get("token"),
        expires_at: row.get("expires_at"),
    }
}

fn installed_game_from_row(row: sqlx::sqlite::SqliteRow) -> InstalledGame {
    let file_path: String = row.get("file_path");
    InstalledGame {
        game_coordinate: row.get("game_coordinate"),
        file_path: PathBuf::from(file_path),
        file_hash: row.get("file_hash"),
        version: row.get("version"),
        server_url: row.get("server_url"),
        installed_at: row.get("installed_at"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::storage::Database;

    async fn test_db() -> Database {
        let path = std::env::temp_dir().join(format!(
            "arcadestr-adp-storage-{}.db",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system clock should be after epoch")
                .as_nanos()
        ));
        Database::new(&path)
            .await
            .expect("test database should open")
    }

    fn provisioning(scope: Option<&str>) -> AdpProvisioning {
        AdpProvisioning {
            id: format!("prov-{}", scope.unwrap_or("none")),
            developer_npub: "developer".to_string(),
            server_url: "https://dist.example.com".to_string(),
            operator_pubkey: "operator".to_string(),
            scope: scope.map(str::to_string),
            fulfillment_pubkey: format!("fulfillment-{}", scope.unwrap_or("none")),
            attestation_event_id: "attestation".to_string(),
            acceptance_event_id: "acceptance".to_string(),
            valid_from: 10,
            revoked_at: None,
            created_at: 11,
        }
    }

    #[tokio::test]
    async fn provisioning_insert_lookup_and_revoke() {
        let db = test_db().await;
        let repo = AdpProvisioningRepository::new(db.pool().clone());
        let entry = provisioning(Some("game"));

        repo.upsert(&entry).await.expect("insert should succeed");
        let found = repo
            .active_for_scope("developer", "https://dist.example.com", Some("game"))
            .await
            .expect("lookup should succeed")
            .expect("active row should exist");
        assert_eq!(found.fulfillment_pubkey, "fulfillment-game");

        repo.mark_revoked("fulfillment-game", 99)
            .await
            .expect("revoke should succeed");
        let revoked = repo
            .active_for_scope("developer", "https://dist.example.com", Some("game"))
            .await
            .expect("lookup should succeed");
        assert!(revoked.is_none());
    }

    #[tokio::test]
    async fn provisioning_unscoped_lookup_uses_null_scope_idempotency() {
        let db = test_db().await;
        let repo = AdpProvisioningRepository::new(db.pool().clone());
        let entry = provisioning(None);

        repo.upsert(&entry).await.expect("insert should succeed");
        let found = repo
            .active_for_scope("developer", "https://dist.example.com", None)
            .await
            .expect("lookup should succeed")
            .expect("unscoped active row should exist");

        assert_eq!(found.scope, None);
        assert_eq!(found.fulfillment_pubkey, "fulfillment-none");
    }

    #[tokio::test]
    async fn provisioning_scoped_active_lookup_is_idempotent_returns_same_row() {
        let db = test_db().await;
        let repo = AdpProvisioningRepository::new(db.pool().clone());
        let entry = provisioning(Some("game"));

        repo.upsert(&entry).await.expect("insert should succeed");
        let first = repo
            .active_for_scope("developer", "https://dist.example.com", Some("game"))
            .await
            .expect("first lookup should succeed")
            .expect("first lookup should find active row");
        let second = repo
            .active_for_scope("developer", "https://dist.example.com", Some("game"))
            .await
            .expect("second lookup should succeed")
            .expect("second lookup should find active row");

        assert_eq!(first.id, second.id);
        assert_eq!(first.fulfillment_pubkey, second.fulfillment_pubkey);
        assert_eq!(first.scope, second.scope);
    }

    #[tokio::test]
    async fn fulfillment_scope_lookup_returns_zero_exact_matches() {
        let db = test_db().await;
        let repo = AdpProvisioningRepository::new(db.pool().clone());
        repo.upsert(&provisioning(Some("other-game")))
            .await
            .expect("insert should succeed");

        let found = repo
            .for_fulfillment_scope("developer", "fulfillment-game", "game")
            .await
            .expect("lookup should succeed");

        assert!(found.is_empty());
    }

    #[tokio::test]
    async fn fulfillment_scope_lookup_returns_one_revoked_exact_match() {
        let db = test_db().await;
        let repo = AdpProvisioningRepository::new(db.pool().clone());
        let mut entry = provisioning(Some("game"));
        entry.revoked_at = Some(99);
        repo.upsert(&entry).await.expect("insert should succeed");

        let found = repo
            .for_fulfillment_scope("developer", "fulfillment-game", "game")
            .await
            .expect("lookup should succeed");

        assert_eq!(found, vec![entry]);
    }

    #[tokio::test]
    async fn fulfillment_scope_lookup_returns_multiple_exact_matches() {
        let db = test_db().await;
        let repo = AdpProvisioningRepository::new(db.pool().clone());
        let first = provisioning(Some("game"));
        let mut second = first.clone();
        second.id = "prov-game-second".to_string();
        second.server_url = "https://operator-two.example.com".to_string();
        repo.upsert(&first)
            .await
            .expect("first insert should succeed");
        repo.upsert(&second)
            .await
            .expect("second insert should succeed");

        let found = repo
            .for_fulfillment_scope("developer", "fulfillment-game", "game")
            .await
            .expect("lookup should succeed");

        assert_eq!(found.len(), 2);
        assert!(found.iter().any(|entry| entry.id == first.id));
        assert!(found.iter().any(|entry| entry.id == second.id));
    }

    #[tokio::test]
    async fn download_tokens_upsert_valid_expired_and_delete() {
        let db = test_db().await;
        let repo = DownloadTokensRepository::new(db.pool().clone());
        let token = DownloadToken {
            game_coordinate: "30402:dev:game".to_string(),
            server_url: "https://dist.example.com".to_string(),
            token: "token-1".to_string(),
            expires_at: 100,
        };

        repo.upsert(&token).await.expect("insert should succeed");
        assert_eq!(
            repo.valid_token("30402:dev:game", "https://dist.example.com", 50)
                .await
                .expect("lookup should succeed")
                .expect("token should be valid")
                .token,
            "token-1"
        );
        assert!(repo
            .valid_token("30402:dev:game", "https://dist.example.com", 101)
            .await
            .expect("lookup should succeed")
            .is_none());

        repo.delete("30402:dev:game", "https://dist.example.com")
            .await
            .expect("delete should succeed");
        assert!(repo
            .valid_token("30402:dev:game", "https://dist.example.com", 50)
            .await
            .expect("lookup should succeed")
            .is_none());
    }

    #[tokio::test]
    async fn download_tokens_upsert_replaces_existing_token() {
        let db = test_db().await;
        let repo = DownloadTokensRepository::new(db.pool().clone());
        let original = DownloadToken {
            game_coordinate: "30402:dev:game".to_string(),
            server_url: "https://dist.example.com".to_string(),
            token: "token-1".to_string(),
            expires_at: 100,
        };
        let replacement = DownloadToken {
            game_coordinate: "30402:dev:game".to_string(),
            server_url: "https://dist.example.com".to_string(),
            token: "token-2".to_string(),
            expires_at: 200,
        };

        repo.upsert(&original)
            .await
            .expect("first upsert should succeed");
        repo.upsert(&replacement)
            .await
            .expect("replacement upsert should succeed");
        let found = repo
            .valid_token("30402:dev:game", "https://dist.example.com", 150)
            .await
            .expect("lookup should succeed")
            .expect("replacement token should be valid");

        assert_eq!(found.token, "token-2");
        assert_eq!(found.expires_at, 200);
    }

    #[tokio::test]
    async fn installed_games_record_get_and_list() {
        let db = test_db().await;
        let repo = InstalledGamesRepository::new(db.pool().clone());
        let entry = InstalledGame {
            game_coordinate: "30402:dev:game".to_string(),
            file_path: PathBuf::from("/tmp/game.zip"),
            file_hash: "hash".to_string(),
            version: Some("1.0.0".to_string()),
            server_url: "https://dist.example.com".to_string(),
            installed_at: 123,
        };

        repo.record(&entry).await.expect("record should succeed");
        let found = repo
            .get("30402:dev:game")
            .await
            .expect("get should succeed")
            .expect("game should exist");
        assert_eq!(found.file_path, PathBuf::from("/tmp/game.zip"));
        assert_eq!(repo.list().await.expect("list should succeed").len(), 1);
    }
}
