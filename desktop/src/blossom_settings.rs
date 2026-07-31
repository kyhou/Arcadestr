//! Account-scoped persistence and deterministic selection of user-configured Blossom servers.

use crate::blossom_upload::BlossomAccountProvider;
use arcadestr_core::blossom::{validate_blossom_server_origin, BlossomServerOriginPolicy};
use arcadestr_core::storage::Database;
use nostr::PublicKey;
use sqlx::{Row, Sqlite, Transaction};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use thiserror::Error;

const MAX_LABEL_CHARS: usize = 100;
const DEVELOPMENT_DEFAULT_SERVER: &str = "http://localhost:9099/";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfiguredBlossomServer {
    pub origin: String,
    pub label: Option<String>,
    pub enabled: bool,
    pub created_at: u64,
    pub updated_at: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlossomServerSettings {
    pub publisher_pubkey: String,
    pub servers: Vec<ConfiguredBlossomServer>,
    pub preferred_server: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlossomServerConfigInput {
    pub origin: String,
    pub label: Option<String>,
    pub enabled: bool,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum BlossomSettingsError {
    #[error("no active Blossom account is available")]
    AccountUnavailable,
    #[error("the active Blossom account changed")]
    AccountMismatch,
    #[error("invalid Blossom server origin: {0}")]
    InvalidOrigin(String),
    #[error("invalid Blossom server label")]
    InvalidLabel,
    #[error("this Blossom server is already configured")]
    DuplicateServer,
    #[error("the Blossom server is not configured")]
    ServerNotFound,
    #[error("the preferred Blossom server must be enabled")]
    PreferredServerDisabled,
    #[error("the requested order must contain every configured server exactly once")]
    InvalidOrder,
    #[error("no Blossom servers are configured")]
    NoConfiguredServers,
    #[error("invalid persisted timestamp")]
    InvalidTimestamp,
    #[error("Blossom settings storage failed: {0}")]
    Storage(String),
}

pub struct BlossomServerSettingsRepository {
    database: Arc<Database>,
    provider: Arc<dyn BlossomAccountProvider>,
    origin_policy: BlossomServerOriginPolicy,
    default_server: Option<&'static str>,
}

impl BlossomServerSettingsRepository {
    pub fn production(database: Arc<Database>, provider: Arc<dyn BlossomAccountProvider>) -> Self {
        Self::new(
            database,
            provider,
            BlossomServerOriginPolicy::HttpsOnly,
            None,
        )
    }

    pub fn development_loopback(
        database: Arc<Database>,
        provider: Arc<dyn BlossomAccountProvider>,
    ) -> Self {
        Self::new(
            database,
            provider,
            BlossomServerOriginPolicy::AllowHttpLoopback,
            Some(DEVELOPMENT_DEFAULT_SERVER),
        )
    }

    pub fn new(
        database: Arc<Database>,
        provider: Arc<dyn BlossomAccountProvider>,
        origin_policy: BlossomServerOriginPolicy,
        default_server: Option<&'static str>,
    ) -> Self {
        Self {
            database,
            provider,
            origin_policy,
            default_server,
        }
    }

    pub async fn list(
        &self,
        expected_publisher: PublicKey,
    ) -> Result<BlossomServerSettings, BlossomSettingsError> {
        self.verify_account(expected_publisher).await?;
        let mut settings = self
            .list_for_publisher(&expected_publisher.to_hex())
            .await?;
        settings
            .servers
            .retain(|server| self.normalize_origin(&server.origin).is_ok());
        if settings.preferred_server.as_ref().is_some_and(|preferred| {
            !settings
                .servers
                .iter()
                .any(|server| &server.origin == preferred && server.enabled)
        }) {
            settings.preferred_server = None;
        }
        if settings.servers.is_empty() {
            if let Some(default_server) = self.default_server {
                match self
                    .add_server(
                        expected_publisher,
                        default_server,
                        Some("Local development"),
                    )
                    .await
                {
                    Ok(()) | Err(BlossomSettingsError::DuplicateServer) => {}
                    Err(error) => return Err(error),
                }
                settings = self
                    .list_for_publisher(&expected_publisher.to_hex())
                    .await?;
            }
        }
        if let Some(default_server) = self.default_server {
            let default_is_enabled = settings
                .servers
                .iter()
                .any(|server| server.origin == default_server && server.enabled);
            if settings.preferred_server.is_none() && default_is_enabled {
                self.set_preferred(expected_publisher, Some(default_server))
                    .await?;
                settings = self
                    .list_for_publisher(&expected_publisher.to_hex())
                    .await?;
            }
        }
        self.verify_account(expected_publisher).await?;
        Ok(settings)
    }

    pub async fn add_server(
        &self,
        expected_publisher: PublicKey,
        origin: &str,
        label: Option<&str>,
    ) -> Result<(), BlossomSettingsError> {
        self.verify_account(expected_publisher).await?;
        let publisher = expected_publisher.to_hex();
        let origin = self.normalize_origin(origin)?;
        let label = validate_label(label)?;
        let now = unix_now()?;
        let mut tx = self.begin().await?;
        let (sort_order,): (i64,) = sqlx::query_as(
            "SELECT COALESCE(MAX(sort_order), -1) + 1 FROM blossom_server_settings WHERE publisher_pubkey = ?",
        )
        .bind(&publisher)
        .fetch_one(&mut *tx)
        .await
        .map_err(storage)?;
        let result = sqlx::query(
            "INSERT INTO blossom_server_settings (publisher_pubkey, origin, label, enabled, sort_order, is_preferred, created_at, updated_at) VALUES (?, ?, ?, 1, ?, 0, ?, ?)",
        )
        .bind(&publisher)
        .bind(&origin)
        .bind(label)
        .bind(sort_order)
        .bind(now)
        .bind(now)
        .execute(&mut *tx)
        .await;
        if let Err(error) = result {
            tx.rollback().await.map_err(storage)?;
            return if is_unique_violation(&error) {
                Err(BlossomSettingsError::DuplicateServer)
            } else {
                Err(storage(error))
            };
        }
        self.commit_if_account_matches(tx, expected_publisher).await
    }

    pub async fn update_label(
        &self,
        expected_publisher: PublicKey,
        origin: &str,
        label: Option<&str>,
    ) -> Result<(), BlossomSettingsError> {
        self.verify_account(expected_publisher).await?;
        let origin = self.normalize_origin(origin)?;
        let label = validate_label(label)?;
        let publisher = expected_publisher.to_hex();
        let now = unix_now()?;
        let mut tx = self.begin().await?;
        let result = sqlx::query(
            "UPDATE blossom_server_settings SET label = ?, updated_at = ? WHERE publisher_pubkey = ? AND origin = ?",
        )
            .bind(label)
            .bind(now)
            .bind(&publisher)
            .bind(&origin)
            .execute(&mut *tx)
            .await
            .map_err(storage)?;
        if result.rows_affected() != 1 {
            tx.rollback().await.map_err(storage)?;
            return Err(BlossomSettingsError::ServerNotFound);
        }
        self.commit_if_account_matches(tx, expected_publisher).await
    }

    pub async fn set_enabled(
        &self,
        expected_publisher: PublicKey,
        origin: &str,
        enabled: bool,
    ) -> Result<(), BlossomSettingsError> {
        self.verify_account(expected_publisher).await?;
        let origin = self.normalize_origin(origin)?;
        let publisher = expected_publisher.to_hex();
        let now = unix_now()?;
        let mut tx = self.begin().await?;
        let result = sqlx::query(
            "UPDATE blossom_server_settings SET enabled = ?, is_preferred = CASE WHEN ? = 0 THEN 0 ELSE is_preferred END, updated_at = ? WHERE publisher_pubkey = ? AND origin = ?",
        )
            .bind(enabled)
            .bind(enabled)
            .bind(now)
            .bind(&publisher)
            .bind(&origin)
            .execute(&mut *tx)
            .await
            .map_err(storage)?;
        if result.rows_affected() != 1 {
            tx.rollback().await.map_err(storage)?;
            return Err(BlossomSettingsError::ServerNotFound);
        }
        self.commit_if_account_matches(tx, expected_publisher).await
    }

    pub async fn remove_server(
        &self,
        expected_publisher: PublicKey,
        origin: &str,
    ) -> Result<(), BlossomSettingsError> {
        self.verify_account(expected_publisher).await?;
        let origin = self.normalize_origin(origin)?;
        let publisher = expected_publisher.to_hex();
        let mut tx = self.begin().await?;
        let result = sqlx::query(
            "DELETE FROM blossom_server_settings WHERE publisher_pubkey = ? AND origin = ?",
        )
        .bind(&publisher)
        .bind(&origin)
        .execute(&mut *tx)
        .await
        .map_err(storage)?;
        if result.rows_affected() != 1 {
            tx.rollback().await.map_err(storage)?;
            return Err(BlossomSettingsError::ServerNotFound);
        }
        self.commit_if_account_matches(tx, expected_publisher).await
    }

    pub async fn reorder(
        &self,
        expected_publisher: PublicKey,
        ordered_origins: &[String],
    ) -> Result<(), BlossomSettingsError> {
        self.verify_account(expected_publisher).await?;
        let publisher = expected_publisher.to_hex();
        let normalized =
            self.normalize_unique_origins(ordered_origins, BlossomSettingsError::InvalidOrder)?;
        let mut tx = self.begin().await?;
        let rows =
            sqlx::query("SELECT origin FROM blossom_server_settings WHERE publisher_pubkey = ?")
                .bind(&publisher)
                .fetch_all(&mut *tx)
                .await
                .map_err(storage)?;
        let configured: HashSet<String> = rows
            .into_iter()
            .map(|row| row.get::<String, _>("origin"))
            .collect();
        if normalized.len() != configured.len()
            || normalized.iter().any(|origin| !configured.contains(origin))
        {
            tx.rollback().await.map_err(storage)?;
            return Err(BlossomSettingsError::InvalidOrder);
        }
        let now = unix_now()?;
        for (position, origin) in normalized.iter().enumerate() {
            let position =
                i64::try_from(position).map_err(|_| BlossomSettingsError::InvalidOrder)?;
            sqlx::query(
                "UPDATE blossom_server_settings SET sort_order = ?, updated_at = ? WHERE publisher_pubkey = ? AND origin = ?",
            )
                .bind(position)
                .bind(now)
                .bind(&publisher)
                .bind(origin)
                .execute(&mut *tx)
                .await
                .map_err(storage)?;
        }
        self.commit_if_account_matches(tx, expected_publisher).await
    }

    pub async fn set_preferred(
        &self,
        expected_publisher: PublicKey,
        origin: Option<&str>,
    ) -> Result<(), BlossomSettingsError> {
        self.verify_account(expected_publisher).await?;
        let publisher = expected_publisher.to_hex();
        let origin = origin
            .map(|value| self.normalize_origin(value))
            .transpose()?;
        let mut tx = self.begin().await?;
        if let Some(origin) = &origin {
            let row = sqlx::query(
                "SELECT enabled FROM blossom_server_settings WHERE publisher_pubkey = ? AND origin = ?",
            )
                .bind(&publisher)
                .bind(origin)
                .fetch_optional(&mut *tx)
                .await
                .map_err(storage)?
                .ok_or(BlossomSettingsError::ServerNotFound)?;
            if !row.get::<bool, _>("enabled") {
                tx.rollback().await.map_err(storage)?;
                return Err(BlossomSettingsError::PreferredServerDisabled);
            }
        }
        let now = unix_now()?;
        sqlx::query(
            "UPDATE blossom_server_settings SET is_preferred = 0, updated_at = ? WHERE publisher_pubkey = ? AND is_preferred = 1",
        )
            .bind(now)
            .bind(&publisher)
            .execute(&mut *tx)
            .await
            .map_err(storage)?;
        if let Some(origin) = &origin {
            sqlx::query(
                "UPDATE blossom_server_settings SET is_preferred = 1, updated_at = ? WHERE publisher_pubkey = ? AND origin = ?",
            )
                .bind(now)
                .bind(&publisher)
                .bind(origin)
                .execute(&mut *tx)
                .await
                .map_err(storage)?;
        }
        self.commit_if_account_matches(tx, expected_publisher).await
    }

    pub async fn replace(
        &self,
        expected_publisher: PublicKey,
        servers: &[BlossomServerConfigInput],
        preferred_server: Option<&str>,
    ) -> Result<(), BlossomSettingsError> {
        self.verify_account(expected_publisher).await?;
        let publisher = expected_publisher.to_hex();
        let mut normalized = Vec::with_capacity(servers.len());
        let mut seen = HashSet::with_capacity(servers.len());
        for server in servers {
            let origin = self.normalize_origin(&server.origin)?;
            if !seen.insert(origin.clone()) {
                return Err(BlossomSettingsError::DuplicateServer);
            }
            normalized.push((
                origin,
                validate_label(server.label.as_deref())?,
                server.enabled,
            ));
        }
        let preferred = preferred_server
            .map(|value| self.normalize_origin(value))
            .transpose()?;
        if let Some(preferred) = &preferred {
            match normalized.iter().find(|entry| &entry.0 == preferred) {
                None => return Err(BlossomSettingsError::ServerNotFound),
                Some((_, _, false)) => return Err(BlossomSettingsError::PreferredServerDisabled),
                Some(_) => {}
            }
        }
        let now = unix_now()?;
        let mut tx = self.begin().await?;
        let existing = sqlx::query(
            "SELECT origin, created_at FROM blossom_server_settings WHERE publisher_pubkey = ?",
        )
        .bind(&publisher)
        .fetch_all(&mut *tx)
        .await
        .map_err(storage)?
        .into_iter()
        .map(|row| {
            (
                row.get::<String, _>("origin"),
                row.get::<i64, _>("created_at"),
            )
        })
        .collect::<HashMap<_, _>>();
        sqlx::query("DELETE FROM blossom_server_settings WHERE publisher_pubkey = ?")
            .bind(&publisher)
            .execute(&mut *tx)
            .await
            .map_err(storage)?;
        for (position, (origin, label, enabled)) in normalized.iter().enumerate() {
            let position =
                i64::try_from(position).map_err(|_| BlossomSettingsError::InvalidOrder)?;
            let created_at = existing.get(origin).copied().unwrap_or(now);
            sqlx::query(
                "INSERT INTO blossom_server_settings (publisher_pubkey, origin, label, enabled, sort_order, is_preferred, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(&publisher)
            .bind(origin)
            .bind(label.as_deref())
            .bind(*enabled)
            .bind(position)
            .bind(preferred.as_ref() == Some(origin))
            .bind(created_at)
            .bind(now)
            .execute(&mut *tx)
            .await
            .map_err(storage)?;
        }
        self.commit_if_account_matches(tx, expected_publisher).await
    }

    async fn list_for_publisher(
        &self,
        publisher: &str,
    ) -> Result<BlossomServerSettings, BlossomSettingsError> {
        let rows = sqlx::query(
            "SELECT origin, label, enabled, is_preferred, created_at, updated_at FROM blossom_server_settings WHERE publisher_pubkey = ? ORDER BY sort_order ASC, origin ASC",
        )
        .bind(publisher)
        .fetch_all(self.database.pool())
        .await
        .map_err(storage)?;
        let mut preferred_server = None;
        let mut servers = Vec::with_capacity(rows.len());
        for row in rows {
            let origin = row.get::<String, _>("origin");
            if row.get::<bool, _>("is_preferred") {
                preferred_server = Some(origin.clone());
            }
            servers.push(ConfiguredBlossomServer {
                origin,
                label: row.get("label"),
                enabled: row.get("enabled"),
                created_at: timestamp_from_db(row.get("created_at"))?,
                updated_at: timestamp_from_db(row.get("updated_at"))?,
            });
        }
        Ok(BlossomServerSettings {
            publisher_pubkey: publisher.to_owned(),
            servers,
            preferred_server,
        })
    }

    async fn verify_account(
        &self,
        expected_publisher: PublicKey,
    ) -> Result<(), BlossomSettingsError> {
        let current = self
            .provider
            .current_publisher()
            .await
            .map_err(|_| BlossomSettingsError::AccountUnavailable)?;
        if current == expected_publisher {
            Ok(())
        } else {
            Err(BlossomSettingsError::AccountMismatch)
        }
    }

    fn normalize_origin(&self, origin: &str) -> Result<String, BlossomSettingsError> {
        validate_blossom_server_origin(origin, self.origin_policy)
            .map(|origin| origin.as_str().to_owned())
            .map_err(|error| BlossomSettingsError::InvalidOrigin(error.to_string()))
    }

    fn normalize_unique_origins(
        &self,
        origins: &[String],
        duplicate_error: BlossomSettingsError,
    ) -> Result<Vec<String>, BlossomSettingsError> {
        let mut normalized = Vec::with_capacity(origins.len());
        let mut seen = HashSet::with_capacity(origins.len());
        for origin in origins {
            let origin = self.normalize_origin(origin)?;
            if !seen.insert(origin.clone()) {
                return Err(duplicate_error);
            }
            normalized.push(origin);
        }
        Ok(normalized)
    }

    async fn begin(&self) -> Result<Transaction<'static, Sqlite>, BlossomSettingsError> {
        self.database.pool().begin().await.map_err(storage)
    }

    async fn commit_if_account_matches(
        &self,
        tx: Transaction<'static, Sqlite>,
        expected_publisher: PublicKey,
    ) -> Result<(), BlossomSettingsError> {
        if let Err(error) = self.verify_account(expected_publisher).await {
            tx.rollback().await.map_err(storage)?;
            return Err(error);
        }
        tx.commit().await.map_err(storage)
    }
}

pub fn resolve_blossom_server_candidates(
    settings: &BlossomServerSettings,
    explicit_server: Option<&str>,
    policy: BlossomServerOriginPolicy,
) -> Result<Vec<String>, BlossomSettingsError> {
    let normalize = |value: &str| {
        validate_blossom_server_origin(value, policy)
            .map(|origin| origin.as_str().to_owned())
            .map_err(|error| BlossomSettingsError::InvalidOrigin(error.to_string()))
    };
    let explicit = match explicit_server {
        Some(value) if !value.is_empty() => Some(normalize(value)?),
        _ => None,
    };
    let normalized_servers = settings
        .servers
        .iter()
        .map(|server| normalize(&server.origin).map(|origin| (origin, server.enabled)))
        .collect::<Result<Vec<_>, _>>()?;
    let preferred = settings
        .preferred_server
        .as_deref()
        .map(normalize)
        .transpose()?;
    let mut candidates = Vec::new();
    let mut seen = HashSet::new();
    if let Some(explicit) = explicit {
        seen.insert(explicit.clone());
        candidates.push(explicit);
    }
    if let Some(preferred) = preferred {
        if normalized_servers
            .iter()
            .any(|(origin, enabled)| origin == &preferred && *enabled)
            && seen.insert(preferred.clone())
        {
            candidates.push(preferred);
        }
    }
    for (origin, enabled) in normalized_servers {
        if enabled && seen.insert(origin.clone()) {
            candidates.push(origin);
        }
    }
    if candidates.is_empty() {
        Err(BlossomSettingsError::NoConfiguredServers)
    } else {
        Ok(candidates)
    }
}

fn validate_label(label: Option<&str>) -> Result<Option<String>, BlossomSettingsError> {
    label
        .map(|value| {
            let value = value.trim();
            if value.is_empty()
                || value.chars().count() > MAX_LABEL_CHARS
                || value.chars().any(char::is_control)
            {
                Err(BlossomSettingsError::InvalidLabel)
            } else {
                Ok(value.to_owned())
            }
        })
        .transpose()
}

fn unix_now() -> Result<i64, BlossomSettingsError> {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| BlossomSettingsError::InvalidTimestamp)?
        .as_secs();
    i64::try_from(seconds).map_err(|_| BlossomSettingsError::InvalidTimestamp)
}

fn timestamp_from_db(value: i64) -> Result<u64, BlossomSettingsError> {
    u64::try_from(value).map_err(|_| BlossomSettingsError::InvalidTimestamp)
}

fn storage(error: sqlx::Error) -> BlossomSettingsError {
    BlossomSettingsError::Storage(error.to_string())
}

fn is_unique_violation(error: &sqlx::Error) -> bool {
    error
        .as_database_error()
        .and_then(|error| error.code())
        .is_some_and(|code| code == "1555" || code == "2067")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::blossom_upload::BlossomUploadError;
    use arcadestr_core::signers::NostrSigner;
    use async_trait::async_trait;
    use nostr::Keys;
    use std::path::Path;
    use std::sync::Mutex;
    use tempfile::TempDir;

    struct TestProvider {
        state: Mutex<TestProviderState>,
    }

    struct TestProviderState {
        publisher: Option<PublicKey>,
        switch_on_call: Option<(usize, PublicKey)>,
        calls: usize,
    }

    impl TestProvider {
        fn new(publisher: PublicKey) -> Self {
            Self {
                state: Mutex::new(TestProviderState {
                    publisher: Some(publisher),
                    switch_on_call: None,
                    calls: 0,
                }),
            }
        }

        fn set_publisher(&self, publisher: Option<PublicKey>) {
            if let Ok(mut state) = self.state.lock() {
                state.publisher = publisher;
            }
        }

        fn switch_on_call(&self, call: usize, publisher: PublicKey) {
            if let Ok(mut state) = self.state.lock() {
                state.calls = 0;
                state.switch_on_call = Some((call, publisher));
            }
        }
    }

    #[async_trait]
    impl BlossomAccountProvider for TestProvider {
        async fn current_publisher(&self) -> Result<PublicKey, BlossomUploadError> {
            let mut state = self
                .state
                .lock()
                .map_err(|_| BlossomUploadError::AccountUnavailable)?;
            state.calls += 1;
            if let Some((call, publisher)) = state.switch_on_call {
                if state.calls == call {
                    state.publisher = Some(publisher);
                    state.switch_on_call = None;
                }
            }
            state
                .publisher
                .ok_or(BlossomUploadError::AccountUnavailable)
        }

        async fn current_signer(&self) -> Result<Arc<dyn NostrSigner>, BlossomUploadError> {
            Err(BlossomUploadError::AccountUnavailable)
        }
    }

    async fn database(path: &Path) -> Arc<Database> {
        Arc::new(
            Database::new(path)
                .await
                .expect("test database should initialize"),
        )
    }

    fn publisher() -> PublicKey {
        Keys::generate().public_key()
    }

    async fn fixture() -> (
        TempDir,
        Arc<Database>,
        Arc<TestProvider>,
        BlossomServerSettingsRepository,
        PublicKey,
    ) {
        let directory = tempfile::tempdir().expect("test directory should initialize");
        let database = database(&directory.path().join("settings.sqlite")).await;
        let publisher = publisher();
        let provider = Arc::new(TestProvider::new(publisher));
        let repository =
            BlossomServerSettingsRepository::production(database.clone(), provider.clone());
        (directory, database, provider, repository, publisher)
    }

    #[tokio::test]
    async fn blossom_settings_empty_and_logout_blocks_reads_without_erasing_rows() {
        let (_directory, _database, provider, repository, publisher) = fixture().await;
        let empty = repository.list(publisher).await.expect("empty list");
        assert!(empty.servers.is_empty());
        assert_eq!(empty.publisher_pubkey, publisher.to_hex());

        repository
            .add_server(publisher, "https://one.example", None)
            .await
            .expect("add server");
        provider.set_publisher(None);
        assert_eq!(
            repository.list(publisher).await,
            Err(BlossomSettingsError::AccountUnavailable)
        );
        provider.set_publisher(Some(publisher));
        assert_eq!(
            repository
                .list(publisher)
                .await
                .expect("persisted")
                .servers
                .len(),
            1
        );
    }

    #[tokio::test]
    async fn blossom_settings_account_isolation_uses_canonical_pubkey() {
        let (_directory, _database, provider, repository, first) = fixture().await;
        let second = publisher();
        repository
            .add_server(first, "https://first.example", None)
            .await
            .expect("first add");
        provider.set_publisher(Some(second));
        repository
            .add_server(second, "https://second.example", None)
            .await
            .expect("second add");
        let second_settings = repository.list(second).await.expect("second list");
        assert_eq!(second_settings.servers[0].origin, "https://second.example/");
        provider.set_publisher(Some(first));
        let first_settings = repository.list(first).await.expect("first list");
        assert_eq!(first_settings.servers[0].origin, "https://first.example/");
        assert_eq!(first_settings.publisher_pubkey, first.to_hex());
    }

    #[tokio::test]
    async fn blossom_settings_add_list_and_restart_persistence() {
        let (directory, db, provider, repository, publisher) = fixture().await;
        repository
            .add_server(publisher, "https://EXAMPLE.com:443", Some("  Primary  "))
            .await
            .expect("add server");
        let settings = repository.list(publisher).await.expect("list");
        assert_eq!(settings.servers[0].origin, "https://example.com/");
        assert_eq!(settings.servers[0].label.as_deref(), Some("Primary"));
        drop(repository);
        drop(db);

        let reopened = database(&directory.path().join("settings.sqlite")).await;
        let repository = BlossomServerSettingsRepository::production(reopened, provider);
        assert_eq!(
            repository.list(publisher).await.expect("reopen").servers,
            settings.servers
        );
    }

    #[tokio::test]
    async fn blossom_settings_duplicate_normalized_and_invalid_origin_are_typed() {
        let (_directory, _database, _provider, repository, publisher) = fixture().await;
        repository
            .add_server(publisher, "https://example.com", None)
            .await
            .expect("add server");
        assert_eq!(
            repository
                .add_server(publisher, "https://example.com:443", None)
                .await,
            Err(BlossomSettingsError::DuplicateServer)
        );
        assert!(matches!(
            repository
                .add_server(publisher, "http://example.com", None)
                .await,
            Err(BlossomSettingsError::InvalidOrigin(_))
        ));
    }

    #[tokio::test]
    async fn blossom_settings_development_seeds_preferred_local_server() {
        let (_directory, database, provider, production, publisher) = fixture().await;
        assert!(matches!(
            production
                .add_server(publisher, "http://127.0.0.1:3000", None)
                .await,
            Err(BlossomSettingsError::InvalidOrigin(_))
        ));
        let development = BlossomServerSettingsRepository::development_loopback(
            database.clone(),
            provider.clone(),
        );
        let settings = development
            .list(publisher)
            .await
            .expect("development defaults");
        assert_eq!(settings.servers.len(), 1);
        assert_eq!(settings.servers[0].origin, DEVELOPMENT_DEFAULT_SERVER);
        assert!(settings.servers[0].enabled);
        assert_eq!(
            settings.preferred_server.as_deref(),
            Some(DEVELOPMENT_DEFAULT_SERVER)
        );
        development
            .add_server(publisher, "http://127.0.0.1:3000", None)
            .await
            .expect("development loopback add");
        development
            .set_preferred(publisher, None)
            .await
            .expect("clear preferred");
        assert_eq!(
            development
                .list(publisher)
                .await
                .expect("repair development preference")
                .preferred_server
                .as_deref(),
            Some(DEVELOPMENT_DEFAULT_SERVER)
        );
        drop(development);
        let production = BlossomServerSettingsRepository::production(database, provider);
        assert!(production
            .list(publisher)
            .await
            .expect("production filters development origins")
            .servers
            .is_empty());
    }

    #[tokio::test]
    async fn blossom_settings_preferred_validation_disable_and_removal_clear() {
        let (_directory, _database, _provider, repository, publisher) = fixture().await;
        repository
            .add_server(publisher, "https://one.example", None)
            .await
            .expect("add one");
        repository
            .add_server(publisher, "https://two.example", None)
            .await
            .expect("add two");
        repository
            .set_enabled(publisher, "https://two.example", false)
            .await
            .expect("disable two");
        assert_eq!(
            repository
                .set_preferred(publisher, Some("https://two.example"))
                .await,
            Err(BlossomSettingsError::PreferredServerDisabled)
        );
        repository
            .set_preferred(publisher, Some("https://one.example"))
            .await
            .expect("prefer one");
        repository
            .set_enabled(publisher, "https://one.example", false)
            .await
            .expect("disable preferred");
        assert_eq!(
            repository
                .list(publisher)
                .await
                .expect("list")
                .preferred_server,
            None
        );
        repository
            .set_enabled(publisher, "https://one.example", true)
            .await
            .expect("enable one");
        repository
            .set_preferred(publisher, Some("https://one.example"))
            .await
            .expect("prefer one");
        repository
            .remove_server(publisher, "https://one.example")
            .await
            .expect("remove preferred");
        assert_eq!(
            repository
                .list(publisher)
                .await
                .expect("list")
                .preferred_server,
            None
        );
    }

    #[tokio::test]
    async fn blossom_settings_stable_order_reorder_and_enabled_filtering() {
        let (_directory, _database, _provider, repository, publisher) = fixture().await;
        for origin in [
            "https://one.example",
            "https://two.example",
            "https://three.example",
        ] {
            repository
                .add_server(publisher, origin, None)
                .await
                .expect("add");
        }
        repository
            .set_enabled(publisher, "https://two.example", false)
            .await
            .expect("disable");
        repository
            .reorder(
                publisher,
                &[
                    "https://three.example".into(),
                    "https://one.example".into(),
                    "https://two.example".into(),
                ],
            )
            .await
            .expect("reorder");
        let settings = repository.list(publisher).await.expect("list");
        assert_eq!(
            settings
                .servers
                .iter()
                .map(|server| server.origin.as_str())
                .collect::<Vec<_>>(),
            vec![
                "https://three.example/",
                "https://one.example/",
                "https://two.example/"
            ]
        );
        assert!(!settings.servers[2].enabled);
        assert_eq!(
            resolve_blossom_server_candidates(
                &settings,
                None,
                BlossomServerOriginPolicy::HttpsOnly
            )
            .expect("candidates"),
            vec!["https://three.example/", "https://one.example/"]
        );
    }

    #[tokio::test]
    async fn blossom_settings_invalid_reorder_and_replace_are_atomic() {
        let (_directory, _database, _provider, repository, publisher) = fixture().await;
        repository
            .add_server(publisher, "https://one.example", Some("One"))
            .await
            .expect("add");
        let before = repository.list(publisher).await.expect("before");
        assert_eq!(
            repository
                .reorder(publisher, &["https://missing.example".into()])
                .await,
            Err(BlossomSettingsError::InvalidOrder)
        );
        let invalid = vec![
            BlossomServerConfigInput {
                origin: "https://two.example".into(),
                label: None,
                enabled: true,
            },
            BlossomServerConfigInput {
                origin: "https://two.example:443".into(),
                label: None,
                enabled: true,
            },
        ];
        assert_eq!(
            repository.replace(publisher, &invalid, None).await,
            Err(BlossomSettingsError::DuplicateServer)
        );
        assert_eq!(repository.list(publisher).await.expect("after"), before);
    }

    #[tokio::test]
    async fn blossom_settings_replace_preserves_created_at_and_sets_complete_order() {
        let (_directory, _database, _provider, repository, publisher) = fixture().await;
        repository
            .add_server(publisher, "https://one.example", Some("Old"))
            .await
            .expect("add");
        let created_at = repository.list(publisher).await.expect("before").servers[0].created_at;
        let replacement = vec![
            BlossomServerConfigInput {
                origin: "https://two.example".into(),
                label: None,
                enabled: true,
            },
            BlossomServerConfigInput {
                origin: "https://one.example:443".into(),
                label: Some("New".into()),
                enabled: true,
            },
        ];
        repository
            .replace(publisher, &replacement, Some("https://one.example"))
            .await
            .expect("replace");
        let settings = repository.list(publisher).await.expect("after");
        assert_eq!(settings.servers[1].created_at, created_at);
        assert_eq!(settings.servers[1].label.as_deref(), Some("New"));
        assert_eq!(
            settings.preferred_server.as_deref(),
            Some("https://one.example/")
        );
    }

    #[tokio::test]
    async fn blossom_settings_stale_account_mutation_rolls_back() {
        let (_directory, _database, provider, repository, first) = fixture().await;
        let second = publisher();
        provider.switch_on_call(2, second);
        assert_eq!(
            repository
                .add_server(first, "https://one.example", None)
                .await,
            Err(BlossomSettingsError::AccountMismatch)
        );
        provider.set_publisher(Some(first));
        assert!(repository
            .list(first)
            .await
            .expect("rolled back")
            .servers
            .is_empty());
    }

    #[test]
    fn blossom_settings_candidates_explicit_preferred_rest_and_dedupe() {
        let server = |origin: &str, enabled| ConfiguredBlossomServer {
            origin: origin.into(),
            label: None,
            enabled,
            created_at: 1,
            updated_at: 1,
        };
        let settings = BlossomServerSettings {
            publisher_pubkey: "publisher".into(),
            servers: vec![
                server("https://one.example/", true),
                server("https://two.example/", true),
                server("https://ONE.example:443", true),
                server("https://off.example/", false),
            ],
            preferred_server: Some("https://two.example/".into()),
        };
        assert_eq!(
            resolve_blossom_server_candidates(
                &settings,
                Some("https://explicit.example:443"),
                BlossomServerOriginPolicy::HttpsOnly,
            )
            .expect("explicit candidates"),
            vec![
                "https://explicit.example/",
                "https://two.example/",
                "https://one.example/",
            ]
        );
        assert_eq!(
            resolve_blossom_server_candidates(
                &settings,
                Some("https://two.example"),
                BlossomServerOriginPolicy::HttpsOnly,
            )
            .expect("deduped candidates"),
            vec!["https://two.example/", "https://one.example/"]
        );
    }

    #[test]
    fn blossom_settings_no_configured_servers_and_no_defaults() {
        let settings = BlossomServerSettings {
            publisher_pubkey: "publisher".into(),
            servers: Vec::new(),
            preferred_server: None,
        };
        assert_eq!(
            resolve_blossom_server_candidates(
                &settings,
                None,
                BlossomServerOriginPolicy::HttpsOnly,
            ),
            Err(BlossomSettingsError::NoConfiguredServers)
        );
        assert_eq!(
            resolve_blossom_server_candidates(
                &settings,
                Some("https://outside.example"),
                BlossomServerOriginPolicy::HttpsOnly,
            )
            .expect("explicit outside persistence"),
            vec!["https://outside.example/"]
        );
    }
}
