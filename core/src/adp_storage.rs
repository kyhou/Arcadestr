//! SQLite repositories for ADP client-side state.

use std::collections::HashSet;
use std::path::{Component, Path, PathBuf};

use nostr::Event;
use serde::{Deserialize, Deserializer, Serialize};
use sqlx::{Row, Sqlite, SqlitePool, Transaction};
use unicode_normalization::UnicodeNormalization;
use uuid::{Uuid, Version};

use crate::distribution::{
    resolve_install_plan, LaunchProfile, PackageFormat, PlatformTarget, ProfileId, RelativePath,
    Sha256Digest,
};
use thiserror::Error;

/// Errors returned by ADP storage repositories.
#[derive(Debug, Error)]
pub enum AdpStorageError {
    /// SQLite operation failed.
    #[error("ADP storage SQL error: {0}")]
    Sql(#[from] sqlx::Error),
    #[error("ADP storage JSON error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("invalid installation storage data: {0}")]
    InvalidData(String),
    #[error("installation not found")]
    NotFound,
    #[error("invalid installation state transition")]
    InvalidTransition,
    #[error("value cannot be represented by SQLite: {0}")]
    OutOfRange(&'static str),
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
    pub authorization_root_event_id: Option<String>,
    pub authorization_capabilities: Vec<String>,
    pub authorization_profile_version: i64,
    pub valid_from: i64,
    pub revoked_at: Option<i64>,
    pub created_at: i64,
}

/// Cached ADP download token.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DownloadToken {
    pub buyer_pubkey: String,
    pub game_coordinate: String,
    pub server_url: String,
    pub token: String,
    pub expires_at: i64,
}

/// Canonical lowercase, hyphenless UUID v4 used for installation identities.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
#[serde(transparent)]
pub struct InstallationId(String);

impl InstallationId {
    pub fn generate() -> Self {
        Self(Uuid::new_v4().simple().to_string())
    }

    pub fn parse(value: &str) -> Result<Self, AdpStorageError> {
        validate_uuid_v4(value, "installation id")?;
        Ok(Self(value.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Canonical opaque identity for one immutable retained package file.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
#[serde(transparent)]
pub struct RetainedPackageId(String);

impl RetainedPackageId {
    pub fn generate() -> Self {
        Self(Uuid::new_v4().simple().to_string())
    }

    pub fn parse(value: &str) -> Result<Self, AdpStorageError> {
        validate_uuid_v4(value, "retained package id")?;
        Ok(Self(value.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl<'de> Deserialize<'de> for RetainedPackageId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::parse(&value).map_err(serde::de::Error::custom)
    }
}

impl<'de> Deserialize<'de> for InstallationId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::parse(&value).map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InstallationState {
    Installing,
    Ready,
    Failed,
    Uninstalling,
}

/// Persisted filesystem outcome used to present recovery for a failed installation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InstallationRecoveryDiscriminator {
    FinalRootAbsent,
    FinalRootPresent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ManifestEntryType {
    File,
    Directory,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstallationManifestEntry {
    pub path: RelativePath,
    pub entry_type: ManifestEntryType,
    pub file_digest: Option<Sha256Digest>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProfileUnavailableReason {
    ExecutableMissing,
    ExecutableNotFile,
    WorkingDirectoryMissing,
    WorkingDirectoryNotDirectory,
    OutsideInstallationRoot,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "availability", content = "reason", rename_all = "snake_case")]
pub enum ProfileAvailability {
    Available,
    Unavailable(ProfileUnavailableReason),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstallationProfile {
    pub profile: LaunchProfile,
    pub availability: ProfileAvailability,
}

/// Installation metadata persisted together with ordered profiles and a manifest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallationRecord {
    pub id: InstallationId,
    pub game_coordinate: String,
    pub installation_root: PathBuf,
    pub package_digest: Sha256Digest,
    pub package_format: PackageFormat,
    pub resolved_target: PlatformTarget,
    pub raw_event_json: String,
    pub publisher_default_profile_id: Option<ProfileId>,
    pub state: InstallationState,
    pub recovery_discriminator: Option<InstallationRecoveryDiscriminator>,
    pub retained_package_coordinate: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallationSnapshot {
    pub installation: InstallationRecord,
    pub profiles: Vec<InstallationProfile>,
    pub manifest: Vec<InstallationManifestEntry>,
}

impl InstallationSnapshot {
    #[must_use]
    pub fn any_profile_available(&self) -> bool {
        self.profiles
            .iter()
            .any(|profile| profile.availability == ProfileAvailability::Available)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RetainedPackageReinstallSnapshot {
    pub raw_event_json: String,
    pub resolved_target: PlatformTarget,
    pub launch_profiles: Vec<LaunchProfile>,
    #[serde(default)]
    pub publisher_default_profile_id: Option<ProfileId>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RetainedPackage {
    pub game_coordinate: String,
    pub package_id: RetainedPackageId,
    pub package_path: PathBuf,
    pub package_digest: Sha256Digest,
    pub package_format: PackageFormat,
    pub reinstall: RetainedPackageReinstallSnapshot,
    pub size_bytes: u64,
    pub created_at: i64,
    pub updated_at: i64,
}

/// A game explicitly saved to an account's library.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LibraryGame {
    pub buyer_pubkey: String,
    pub game_coordinate: String,
    pub added_at: i64,
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
             attestation_event_id, acceptance_event_id, authorization_root_event_id,
             authorization_capabilities_json, authorization_profile_version,
             valid_from, revoked_at, created_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)"#,
        )
        .bind(&entry.id)
        .bind(&entry.developer_npub)
        .bind(&entry.server_url)
        .bind(&entry.operator_pubkey)
        .bind(&entry.scope)
        .bind(&entry.fulfillment_pubkey)
        .bind(&entry.attestation_event_id)
        .bind(&entry.acceptance_event_id)
        .bind(&entry.authorization_root_event_id)
        .bind(serde_json::to_string(&entry.authorization_capabilities)?)
        .bind(entry.authorization_profile_version)
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
               authorization_root_event_id, authorization_capabilities_json,
               authorization_profile_version, valid_from, revoked_at, created_at
               FROM adp_provisioning
               WHERE developer_npub = ? AND server_url = ?
                  AND COALESCE(scope, '') = COALESCE(?, '')
                  AND revoked_at IS NULL
                  AND authorization_profile_version >= 2
                  AND authorization_root_event_id IS NOT NULL"#,
        )
        .bind(developer_npub)
        .bind(server_url)
        .bind(scope)
        .fetch_optional(&self.pool)
        .await?;

        row.map(adp_provisioning_from_row).transpose()
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
               authorization_root_event_id, authorization_capabilities_json,
               authorization_profile_version, valid_from, revoked_at, created_at
               FROM adp_provisioning
               WHERE developer_npub = ? AND fulfillment_pubkey = ? AND scope = ?"#,
        )
        .bind(developer_npub)
        .bind(fulfillment_pubkey)
        .bind(scope)
        .fetch_all(&self.pool)
        .await?;

        rows.into_iter().map(adp_provisioning_from_row).collect()
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
            (buyer_pubkey, game_coordinate, server_url, token, expires_at) VALUES (?, ?, ?, ?, ?)"#,
        )
        .bind(&token.buyer_pubkey)
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
        buyer_pubkey: &str,
        game_coordinate: &str,
        server_url: &str,
        now: i64,
    ) -> Result<Option<DownloadToken>, AdpStorageError> {
        let row = sqlx::query(
            "SELECT buyer_pubkey, game_coordinate, server_url, token, expires_at FROM download_tokens \
             WHERE buyer_pubkey = ? AND game_coordinate = ? AND server_url = ? AND expires_at > ?",
        )
        .bind(buyer_pubkey)
        .bind(game_coordinate)
        .bind(server_url)
        .bind(now)
        .fetch_optional(&self.pool)
        .await?;
        row.map(download_token_from_row).transpose()
    }

    pub async fn delete(
        &self,
        buyer_pubkey: &str,
        game_coordinate: &str,
        server_url: &str,
    ) -> Result<(), AdpStorageError> {
        sqlx::query(
            "DELETE FROM download_tokens WHERE buyer_pubkey = ? AND game_coordinate = ? AND server_url = ?",
        )
            .bind(buyer_pubkey)
            .bind(game_coordinate)
            .bind(server_url)
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}

/// Repository for installation snapshots and retained packages.
pub struct InstallationsRepository {
    pool: SqlitePool,
}

/// Device-local launch preference storage, deliberately independent of an
/// installation row so preferences survive uninstall and reinstall.
pub struct LaunchProfilePreferencesRepository {
    pool: SqlitePool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchProfilePreference {
    pub profile_id: Option<String>,
    pub updated_at: i64,
}

impl LaunchProfilePreferencesRepository {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn get(
        &self,
        game_coordinate: &str,
    ) -> Result<Option<LaunchProfilePreference>, AdpStorageError> {
        let value: Option<(Option<String>, i64)> = sqlx::query_as(
            "SELECT profile_id, updated_at FROM launch_profile_preferences WHERE game_coordinate = ?",
        )
        .bind(game_coordinate)
        .fetch_optional(&self.pool)
        .await?;
        Ok(
            value.map(|(profile_id, updated_at)| LaunchProfilePreference {
                profile_id,
                updated_at,
            }),
        )
    }

    pub async fn set(
        &self,
        game_coordinate: &str,
        profile_id: &ProfileId,
        updated_at: i64,
    ) -> Result<(), AdpStorageError> {
        if !(0..i64::MAX).contains(&updated_at) {
            return Err(AdpStorageError::InvalidData(
                "launch profile preference revision is out of range".to_string(),
            ));
        }
        sqlx::query(
            r#"INSERT INTO launch_profile_preferences (game_coordinate, profile_id, updated_at)
               VALUES (?, ?, ?)
               ON CONFLICT(game_coordinate) DO UPDATE SET
                    profile_id = excluded.profile_id,
                    updated_at = excluded.updated_at
                WHERE excluded.updated_at > launch_profile_preferences.updated_at"#,
        )
        .bind(game_coordinate)
        .bind(profile_id.as_str())
        .bind(updated_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn clear(
        &self,
        game_coordinate: &str,
        updated_at: i64,
    ) -> Result<(), AdpStorageError> {
        if !(0..i64::MAX).contains(&updated_at) {
            return Err(AdpStorageError::InvalidData(
                "launch profile preference revision is out of range".to_string(),
            ));
        }
        sqlx::query(
            r#"INSERT INTO launch_profile_preferences (game_coordinate, profile_id, updated_at)
               VALUES (?, NULL, ?)
               ON CONFLICT(game_coordinate) DO UPDATE SET
                    profile_id = NULL,
                    updated_at = excluded.updated_at
               WHERE excluded.updated_at > launch_profile_preferences.updated_at"#,
        )
        .bind(game_coordinate)
        .bind(updated_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }
}

/// Repository for account-scoped `library_games` rows.
pub struct LibraryGamesRepository {
    pool: SqlitePool,
}

impl LibraryGamesRepository {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn add(&self, entry: &LibraryGame) -> Result<(), AdpStorageError> {
        sqlx::query(
            r#"INSERT INTO library_games (buyer_pubkey, game_coordinate, added_at)
               VALUES (?, ?, ?)
               ON CONFLICT (buyer_pubkey, game_coordinate) DO NOTHING"#,
        )
        .bind(&entry.buyer_pubkey)
        .bind(&entry.game_coordinate)
        .bind(entry.added_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn contains(
        &self,
        buyer_pubkey: &str,
        game_coordinate: &str,
    ) -> Result<bool, AdpStorageError> {
        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM library_games WHERE buyer_pubkey = ? AND game_coordinate = ?",
        )
        .bind(buyer_pubkey)
        .bind(game_coordinate)
        .fetch_one(&self.pool)
        .await?;
        Ok(count > 0)
    }

    pub async fn list(&self, buyer_pubkey: &str) -> Result<Vec<LibraryGame>, AdpStorageError> {
        let rows = sqlx::query(
            "SELECT buyer_pubkey, game_coordinate, added_at FROM library_games \
             WHERE buyer_pubkey = ? ORDER BY added_at DESC, game_coordinate ASC",
        )
        .bind(buyer_pubkey)
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(|row| {
                Ok(LibraryGame {
                    buyer_pubkey: row.try_get("buyer_pubkey")?,
                    game_coordinate: row.try_get("game_coordinate")?,
                    added_at: row.try_get("added_at")?,
                })
            })
            .collect()
    }
}

impl InstallationsRepository {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }

    /// Creates an installing snapshot and its ordered profiles atomically.
    pub async fn begin(
        &self,
        installation: &InstallationRecord,
        profiles: &[InstallationProfile],
    ) -> Result<(), AdpStorageError> {
        if installation.state != InstallationState::Installing {
            return Err(AdpStorageError::InvalidData(
                "new installations must start in installing state".to_string(),
            ));
        }
        validate_recovery_discriminator(installation.state, installation.recovery_discriminator)?;
        validate_absolute_path(&installation.installation_root, "installation root")?;
        validate_platform_target(&installation.resolved_target)?;
        validate_retained_reference(
            installation.retained_package_coordinate.as_deref(),
            &installation.game_coordinate,
        )?;
        validate_resolved_snapshot(installation, profiles)?;

        let mut transaction = self.pool.begin().await?;
        if let Some(coordinate) = &installation.retained_package_coordinate {
            let retained = sqlx::query(&format!(
                "{RETAINED_PACKAGE_SELECT} WHERE game_coordinate = ?"
            ))
            .bind(coordinate)
            .fetch_optional(&mut *transaction)
            .await?
            .map(retained_package_from_row)
            .transpose()?;
            let installation_profiles = profiles
                .iter()
                .map(|profile| &profile.profile)
                .collect::<Vec<_>>();
            if retained.as_ref().is_none_or(|package| {
                package.package_digest != installation.package_digest
                    || package.package_format != installation.package_format
                    || package.reinstall.raw_event_json != installation.raw_event_json
                    || package.reinstall.resolved_target != installation.resolved_target
                    || package.reinstall.publisher_default_profile_id
                        != installation.publisher_default_profile_id
                    || package.reinstall.launch_profiles.iter().collect::<Vec<_>>()
                        != installation_profiles
            }) {
                return Err(AdpStorageError::InvalidData(
                    "retained package must match the installation snapshot".to_string(),
                ));
            }
        }
        sqlx::query(
            r#"INSERT INTO installations (
                id, game_coordinate, installation_root, package_digest, package_format,
                resolved_os, resolved_arch, raw_event_json, publisher_default_profile_id, state,
                recovery_discriminator, retained_package_coordinate, created_at, updated_at
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)"#,
        )
        .bind(installation.id.as_str())
        .bind(&installation.game_coordinate)
        .bind(path_text(
            &installation.installation_root,
            "installation root",
        )?)
        .bind(installation.package_digest.as_str())
        .bind(package_format_text(&installation.package_format))
        .bind(&installation.resolved_target.os)
        .bind(&installation.resolved_target.arch)
        .bind(&installation.raw_event_json)
        .bind(
            installation
                .publisher_default_profile_id
                .as_ref()
                .map(ProfileId::as_str),
        )
        .bind(installation_state_text(installation.state))
        .bind(
            installation
                .recovery_discriminator
                .map(recovery_discriminator_text),
        )
        .bind(&installation.retained_package_coordinate)
        .bind(installation.created_at)
        .bind(installation.updated_at)
        .execute(&mut *transaction)
        .await?;

        for (order, profile) in profiles.iter().enumerate() {
            let order =
                i64::try_from(order).map_err(|_| AdpStorageError::OutOfRange("profile order"))?;
            let profile_json = serde_json::to_string(&profile.profile)?;
            let (availability, reason) = profile_availability_text(&profile.availability);
            sqlx::query(
                r#"INSERT INTO installation_profiles
                   (installation_id, profile_order, profile_json, availability, unavailable_reason)
                   VALUES (?, ?, ?, ?, ?)"#,
            )
            .bind(installation.id.as_str())
            .bind(order)
            .bind(profile_json)
            .bind(availability)
            .bind(reason)
            .execute(&mut *transaction)
            .await?;
        }
        transaction.commit().await?;
        Ok(())
    }

    /// Replaces the complete manifest in one transaction.
    pub async fn replace_manifest(
        &self,
        id: &InstallationId,
        entries: &[InstallationManifestEntry],
    ) -> Result<(), AdpStorageError> {
        for entry in entries {
            validate_manifest_entry(entry)?;
        }
        let mut path_keys = HashSet::with_capacity(entries.len());
        if entries
            .iter()
            .any(|entry| !path_keys.insert(manifest_path_key(&entry.path)))
        {
            return Err(AdpStorageError::InvalidData(
                "manifest paths must be unique under case-insensitive comparison".to_string(),
            ));
        }
        let mut transaction = self.pool.begin().await?;
        let state: Option<String> =
            sqlx::query_scalar("SELECT state FROM installations WHERE id = ?")
                .bind(id.as_str())
                .fetch_optional(&mut *transaction)
                .await?;
        match state.as_deref() {
            None => return Err(AdpStorageError::NotFound),
            Some("installing") => {}
            Some(_) => return Err(AdpStorageError::InvalidTransition),
        }
        sqlx::query("DELETE FROM installation_manifest_entries WHERE installation_id = ?")
            .bind(id.as_str())
            .execute(&mut *transaction)
            .await?;
        for entry in entries {
            sqlx::query(
                r#"INSERT INTO installation_manifest_entries
                   (installation_id, path, path_key, entry_type, file_digest)
                   VALUES (?, ?, ?, ?, ?)"#,
            )
            .bind(id.as_str())
            .bind(entry.path.as_str())
            .bind(manifest_path_key(&entry.path))
            .bind(manifest_entry_type_text(entry.entry_type))
            .bind(entry.file_digest.as_ref().map(Sha256Digest::as_str))
            .execute(&mut *transaction)
            .await?;
        }
        transaction.commit().await?;
        Ok(())
    }

    /// Atomically records completed extraction output and marks an existing
    /// installing row ready.
    pub async fn commit_extraction(
        &self,
        id: &InstallationId,
        installation_root: &Path,
        package_digest: &Sha256Digest,
        package_format: &PackageFormat,
        profiles: &[InstallationProfile],
        entries: &[InstallationManifestEntry],
        updated_at: i64,
    ) -> Result<(), AdpStorageError> {
        validate_absolute_path(installation_root, "installation root")?;
        for entry in entries {
            validate_manifest_entry(entry)?;
        }
        let mut path_keys = HashSet::with_capacity(entries.len());
        if entries
            .iter()
            .any(|entry| !path_keys.insert(manifest_path_key(&entry.path)))
        {
            return Err(AdpStorageError::InvalidData(
                "manifest paths must be unique under case-insensitive comparison".to_string(),
            ));
        }

        let mut transaction = self.pool.begin().await?;
        let row = sqlx::query(&format!("{INSTALLATION_SELECT} WHERE id = ?"))
            .bind(id.as_str())
            .fetch_optional(&mut *transaction)
            .await?
            .ok_or(AdpStorageError::NotFound)?;
        let mut installation = installation_from_row(row)?;
        if installation.state != InstallationState::Installing {
            return Err(AdpStorageError::InvalidTransition);
        }
        if installation.installation_root != installation_root
            || &installation.package_digest != package_digest
            || &installation.package_format != package_format
        {
            return Err(AdpStorageError::InvalidData(
                "placed extraction does not match the installing snapshot".to_string(),
            ));
        }
        validate_resolved_snapshot(&installation, profiles)?;

        sqlx::query("DELETE FROM installation_profiles WHERE installation_id = ?")
            .bind(id.as_str())
            .execute(&mut *transaction)
            .await?;
        for (order, profile) in profiles.iter().enumerate() {
            let order =
                i64::try_from(order).map_err(|_| AdpStorageError::OutOfRange("profile order"))?;
            let profile_json = serde_json::to_string(&profile.profile)?;
            let (availability, reason) = profile_availability_text(&profile.availability);
            sqlx::query(
                r#"INSERT INTO installation_profiles
                   (installation_id, profile_order, profile_json, availability, unavailable_reason)
                   VALUES (?, ?, ?, ?, ?)"#,
            )
            .bind(id.as_str())
            .bind(order)
            .bind(profile_json)
            .bind(availability)
            .bind(reason)
            .execute(&mut *transaction)
            .await?;
        }

        sqlx::query("DELETE FROM installation_manifest_entries WHERE installation_id = ?")
            .bind(id.as_str())
            .execute(&mut *transaction)
            .await?;
        for entry in entries {
            sqlx::query(
                r#"INSERT INTO installation_manifest_entries
                   (installation_id, path, path_key, entry_type, file_digest)
                   VALUES (?, ?, ?, ?, ?)"#,
            )
            .bind(id.as_str())
            .bind(entry.path.as_str())
            .bind(manifest_path_key(&entry.path))
            .bind(manifest_entry_type_text(entry.entry_type))
            .bind(entry.file_digest.as_ref().map(Sha256Digest::as_str))
            .execute(&mut *transaction)
            .await?;
        }

        installation.state = InstallationState::Ready;
        installation.updated_at = updated_at;
        let snapshot = InstallationSnapshot {
            installation,
            profiles: profiles.to_vec(),
            manifest: entries.to_vec(),
        };
        validate_complete_manifest(&snapshot)?;

        let result = sqlx::query(
            "UPDATE installations SET state = 'ready', updated_at = ? \
             WHERE id = ? AND state = 'installing'",
        )
        .bind(updated_at)
        .bind(id.as_str())
        .execute(&mut *transaction)
        .await?;
        if result.rows_affected() != 1 {
            return Err(AdpStorageError::InvalidTransition);
        }
        transaction.commit().await?;
        Ok(())
    }

    /// Returns whether the stored profile set contains a launchable profile.
    pub async fn any_profile_available(
        &self,
        id: &InstallationId,
    ) -> Result<bool, AdpStorageError> {
        let available: Option<bool> = sqlx::query_scalar(
            r#"SELECT EXISTS (
                   SELECT 1 FROM installation_profiles
                   WHERE installation_id = installations.id AND availability = 'available'
               )
               FROM installations WHERE id = ?"#,
        )
        .bind(id.as_str())
        .fetch_optional(&self.pool)
        .await?;
        available.ok_or(AdpStorageError::NotFound)
    }

    pub async fn get_by_id(
        &self,
        id: &InstallationId,
    ) -> Result<Option<InstallationSnapshot>, AdpStorageError> {
        let mut transaction = self.pool.begin().await?;
        let row = sqlx::query(&format!("{INSTALLATION_SELECT} WHERE id = ?"))
            .bind(id.as_str())
            .fetch_optional(&mut *transaction)
            .await?;
        let snapshot = match row {
            Some(row) => Some(self.snapshot_from_row(row, &mut transaction).await?),
            None => None,
        };
        transaction.commit().await?;
        Ok(snapshot)
    }

    pub async fn get_by_coordinate(
        &self,
        coordinate: &str,
    ) -> Result<Option<InstallationSnapshot>, AdpStorageError> {
        let mut transaction = self.pool.begin().await?;
        let row = sqlx::query(&format!("{INSTALLATION_SELECT} WHERE game_coordinate = ?"))
            .bind(coordinate)
            .fetch_optional(&mut *transaction)
            .await?;
        let snapshot = match row {
            Some(row) => Some(self.snapshot_from_row(row, &mut transaction).await?),
            None => None,
        };
        transaction.commit().await?;
        Ok(snapshot)
    }

    pub async fn list(&self) -> Result<Vec<InstallationSnapshot>, AdpStorageError> {
        let mut transaction = self.pool.begin().await?;
        let rows = sqlx::query(&format!(
            "{INSTALLATION_SELECT} ORDER BY created_at DESC, id"
        ))
        .fetch_all(&mut *transaction)
        .await?;
        let mut snapshots = Vec::with_capacity(rows.len());
        for row in rows {
            snapshots.push(self.snapshot_from_row(row, &mut transaction).await?);
        }
        transaction.commit().await?;
        Ok(snapshots)
    }

    /// Lists only incomplete installs. Recovery deliberately does not inspect or
    /// mutate ready installations whose roots have vanished; launch-time
    /// validation owns that A5 decision.
    pub async fn list_installing(&self) -> Result<Vec<InstallationSnapshot>, AdpStorageError> {
        let mut transaction = self.pool.begin().await?;
        let rows = sqlx::query(&format!(
            "{INSTALLATION_SELECT} WHERE state = 'installing' ORDER BY created_at, id"
        ))
        .fetch_all(&mut *transaction)
        .await?;
        let mut snapshots = Vec::with_capacity(rows.len());
        for row in rows {
            snapshots.push(self.snapshot_from_row(row, &mut transaction).await?);
        }
        transaction.commit().await?;
        Ok(snapshots)
    }

    pub async fn manifest_entry(
        &self,
        id: &InstallationId,
        path: &RelativePath,
    ) -> Result<Option<InstallationManifestEntry>, AdpStorageError> {
        let row = sqlx::query(
            r#"SELECT path, entry_type, file_digest
               FROM installation_manifest_entries
               WHERE installation_id = ? AND path_key = ?"#,
        )
        .bind(id.as_str())
        .bind(manifest_path_key(path))
        .fetch_optional(&self.pool)
        .await?;
        row.map(manifest_entry_from_row).transpose()
    }

    pub async fn transition(
        &self,
        id: &InstallationId,
        next: InstallationState,
        updated_at: i64,
    ) -> Result<(), AdpStorageError> {
        let allowed_from = match next {
            InstallationState::Ready | InstallationState::Failed => InstallationState::Installing,
            InstallationState::Uninstalling => InstallationState::Ready,
            InstallationState::Installing => return Err(AdpStorageError::InvalidTransition),
        };
        let mut transaction = self.pool.begin().await?;
        if next == InstallationState::Ready {
            let row = sqlx::query(&format!("{INSTALLATION_SELECT} WHERE id = ?"))
                .bind(id.as_str())
                .fetch_optional(&mut *transaction)
                .await?
                .ok_or(AdpStorageError::NotFound)?;
            let snapshot = self.snapshot_from_row(row, &mut transaction).await?;
            if snapshot.installation.state != allowed_from {
                return Err(AdpStorageError::InvalidTransition);
            }
            validate_complete_manifest(&snapshot)?;
        }
        let result = sqlx::query(
            "UPDATE installations SET state = ?, recovery_discriminator = NULL, updated_at = ? \
             WHERE id = ? AND state = ?",
        )
        .bind(installation_state_text(next))
        .bind(updated_at)
        .bind(id.as_str())
        .bind(installation_state_text(allowed_from))
        .execute(&mut *transaction)
        .await?;
        if result.rows_affected() == 1 {
            transaction.commit().await?;
            Ok(())
        } else {
            Err(AdpStorageError::InvalidTransition)
        }
    }

    /// Marks an installing row failed without presenting a recovery outcome.
    pub async fn mark_failed(
        &self,
        id: &InstallationId,
        updated_at: i64,
    ) -> Result<(), AdpStorageError> {
        self.transition(id, InstallationState::Failed, updated_at)
            .await
    }

    /// Atomically marks an installing row failed and records its filesystem recovery outcome.
    pub async fn mark_failed_for_recovery(
        &self,
        id: &InstallationId,
        discriminator: InstallationRecoveryDiscriminator,
        updated_at: i64,
    ) -> Result<(), AdpStorageError> {
        let result = sqlx::query(
            "UPDATE installations SET state = 'failed', recovery_discriminator = ?, updated_at = ? \
             WHERE id = ? AND state = 'installing'",
        )
        .bind(recovery_discriminator_text(discriminator))
        .bind(updated_at)
        .bind(id.as_str())
        .execute(&self.pool)
        .await?;
        if result.rows_affected() == 1 {
            Ok(())
        } else {
            Err(AdpStorageError::InvalidTransition)
        }
    }

    /// Recovery compare-and-swap which refuses to overwrite a row changed after
    /// the recovery scan snapshot was read.
    pub async fn mark_failed_for_recovery_if_unchanged(
        &self,
        id: &InstallationId,
        expected_updated_at: i64,
        discriminator: InstallationRecoveryDiscriminator,
        updated_at: i64,
    ) -> Result<(), AdpStorageError> {
        let result = sqlx::query(
            "UPDATE installations SET state = 'failed', recovery_discriminator = ?, updated_at = ? \
             WHERE id = ? AND state = 'installing' AND updated_at = ?",
        )
        .bind(recovery_discriminator_text(discriminator))
        .bind(updated_at)
        .bind(id.as_str())
        .bind(expected_updated_at)
        .execute(&self.pool)
        .await?;
        if result.rows_affected() == 1 {
            Ok(())
        } else {
            Err(AdpStorageError::InvalidTransition)
        }
    }

    pub async fn delete(&self, id: &InstallationId) -> Result<(), AdpStorageError> {
        let result =
            sqlx::query("DELETE FROM installations WHERE id = ? AND state = 'uninstalling'")
                .bind(id.as_str())
                .execute(&self.pool)
                .await?;
        if result.rows_affected() == 1 {
            Ok(())
        } else {
            Err(AdpStorageError::InvalidTransition)
        }
    }

    /// Removes a failed row before an explicit retry or dismissal.
    pub async fn delete_failed(&self, id: &InstallationId) -> Result<(), AdpStorageError> {
        let result = sqlx::query("DELETE FROM installations WHERE id = ? AND state = 'failed'")
            .bind(id.as_str())
            .execute(&self.pool)
            .await?;
        if result.rows_affected() == 1 {
            Ok(())
        } else {
            Err(AdpStorageError::InvalidTransition)
        }
    }

    /// Atomically points a coordinate at a newly completed immutable package.
    ///
    /// The returned old row can be deleted after commit. If the process dies on either
    /// side of the commit, the database still points at one complete package.
    pub async fn replace_retained_package(
        &self,
        package: &RetainedPackage,
    ) -> Result<Option<RetainedPackage>, AdpStorageError> {
        validate_retained_package(package)?;
        let size = i64::try_from(package.size_bytes)
            .map_err(|_| AdpStorageError::OutOfRange("retained package size"))?;
        let profiles_json = serde_json::to_string(&package.reinstall.launch_profiles)?;
        let mut transaction = self.pool.begin().await?;
        let installed_package: Option<(String, String, String, String, String, String, Option<String>)> =
            sqlx::query_as(
                "SELECT id, package_digest, package_format, raw_event_json, resolved_os, resolved_arch, \
                        publisher_default_profile_id \
                 FROM installations WHERE game_coordinate = ?",
            )
            .bind(&package.game_coordinate)
            .fetch_optional(&mut *transaction)
            .await?;
        if let Some((id, digest, format, raw, os, arch, publisher_default)) = installed_package {
            let profile_json: Vec<String> = sqlx::query_scalar(
                "SELECT profile_json FROM installation_profiles WHERE installation_id = ? \
                 ORDER BY profile_order",
            )
            .bind(id)
            .fetch_all(&mut *transaction)
            .await?;
            let installed_profiles = profile_json
                .into_iter()
                .map(|json| serde_json::from_str::<LaunchProfile>(&json))
                .collect::<Result<Vec<_>, _>>()?;
            if digest != package.package_digest.as_str()
                || format != package_format_text(&package.package_format)
                || raw != package.reinstall.raw_event_json
                || os != package.reinstall.resolved_target.os
                || arch != package.reinstall.resolved_target.arch
                || publisher_default.as_deref()
                    != package
                        .reinstall
                        .publisher_default_profile_id
                        .as_ref()
                        .map(ProfileId::as_str)
                || installed_profiles != package.reinstall.launch_profiles
            {
                return Err(AdpStorageError::InvalidData(
                    "retained package must match the current installation snapshot".to_string(),
                ));
            }
        }
        let previous = sqlx::query(&format!(
            "{RETAINED_PACKAGE_SELECT} WHERE game_coordinate = ?"
        ))
        .bind(&package.game_coordinate)
        .fetch_optional(&mut *transaction)
        .await?
        .map(retained_package_from_row)
        .transpose()?;
        sqlx::query(
            r#"INSERT INTO retained_packages (
                game_coordinate, package_id, package_path, package_digest, package_format,
                raw_event_json, resolved_os, resolved_arch, profiles_json,
                publisher_default_profile_id, size_bytes, created_at, updated_at
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            ON CONFLICT(game_coordinate) DO UPDATE SET
                package_id = excluded.package_id,
                package_path = excluded.package_path,
                package_digest = excluded.package_digest,
                package_format = excluded.package_format,
                raw_event_json = excluded.raw_event_json,
                resolved_os = excluded.resolved_os,
                resolved_arch = excluded.resolved_arch,
                profiles_json = excluded.profiles_json,
                publisher_default_profile_id = excluded.publisher_default_profile_id,
                size_bytes = excluded.size_bytes,
                updated_at = excluded.updated_at"#,
        )
        .bind(&package.game_coordinate)
        .bind(package.package_id.as_str())
        .bind(path_text(&package.package_path, "retained package path")?)
        .bind(package.package_digest.as_str())
        .bind(package_format_text(&package.package_format))
        .bind(&package.reinstall.raw_event_json)
        .bind(&package.reinstall.resolved_target.os)
        .bind(&package.reinstall.resolved_target.arch)
        .bind(profiles_json)
        .bind(
            package
                .reinstall
                .publisher_default_profile_id
                .as_ref()
                .map(ProfileId::as_str),
        )
        .bind(size)
        .bind(package.created_at)
        .bind(package.updated_at)
        .execute(&mut *transaction)
        .await?;
        transaction.commit().await?;
        Ok(previous)
    }

    /// Atomically stores an immutable retained package and attaches it to the
    /// matching installing row. This ordering lets install orchestration create
    /// its recovery row before writing package bytes without ever exposing an
    /// installation reference to a missing retained row.
    pub async fn retain_and_attach_installing(
        &self,
        installation_id: &InstallationId,
        package: &RetainedPackage,
    ) -> Result<(), AdpStorageError> {
        validate_retained_package(package)?;
        let size = i64::try_from(package.size_bytes)
            .map_err(|_| AdpStorageError::OutOfRange("retained package size"))?;
        let profiles_json = serde_json::to_string(&package.reinstall.launch_profiles)?;
        let mut transaction = self.pool.begin().await?;
        let row: Option<(
            String,
            String,
            String,
            String,
            String,
            String,
            Option<String>,
        )> = sqlx::query_as(
            "SELECT game_coordinate, package_digest, package_format, raw_event_json, \
                    resolved_os, resolved_arch, publisher_default_profile_id FROM installations \
             WHERE id = ? AND state = 'installing' AND retained_package_coordinate IS NULL",
        )
        .bind(installation_id.as_str())
        .fetch_optional(&mut *transaction)
        .await?;
        let Some((
            coordinate,
            digest,
            format,
            raw_event,
            resolved_os,
            resolved_arch,
            publisher_default,
        )) = row
        else {
            return Err(AdpStorageError::InvalidTransition);
        };
        let stored_profile_json: Vec<String> = sqlx::query_scalar(
            "SELECT profile_json FROM installation_profiles WHERE installation_id = ? \
             ORDER BY profile_order",
        )
        .bind(installation_id.as_str())
        .fetch_all(&mut *transaction)
        .await?;
        let stored_profiles = stored_profile_json
            .into_iter()
            .map(|json| serde_json::from_str::<LaunchProfile>(&json))
            .collect::<Result<Vec<_>, _>>()?;
        if coordinate != package.game_coordinate
            || digest != package.package_digest.as_str()
            || format != package_format_text(&package.package_format)
            || raw_event != package.reinstall.raw_event_json
            || resolved_os != package.reinstall.resolved_target.os
            || resolved_arch != package.reinstall.resolved_target.arch
            || publisher_default.as_deref()
                != package
                    .reinstall
                    .publisher_default_profile_id
                    .as_ref()
                    .map(ProfileId::as_str)
            || stored_profiles != package.reinstall.launch_profiles
        {
            return Err(AdpStorageError::InvalidData(
                "retained package must match the installing snapshot".to_string(),
            ));
        }
        sqlx::query(
            r#"INSERT INTO retained_packages (
                game_coordinate, package_id, package_path, package_digest, package_format,
                raw_event_json, resolved_os, resolved_arch, profiles_json,
                publisher_default_profile_id, size_bytes, created_at, updated_at
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)"#,
        )
        .bind(&package.game_coordinate)
        .bind(package.package_id.as_str())
        .bind(path_text(&package.package_path, "retained package path")?)
        .bind(package.package_digest.as_str())
        .bind(package_format_text(&package.package_format))
        .bind(&package.reinstall.raw_event_json)
        .bind(&package.reinstall.resolved_target.os)
        .bind(&package.reinstall.resolved_target.arch)
        .bind(profiles_json)
        .bind(
            package
                .reinstall
                .publisher_default_profile_id
                .as_ref()
                .map(ProfileId::as_str),
        )
        .bind(size)
        .bind(package.created_at)
        .bind(package.updated_at)
        .execute(&mut *transaction)
        .await?;
        let result = sqlx::query(
            "UPDATE installations SET retained_package_coordinate = ?, updated_at = ? \
             WHERE id = ? AND state = 'installing' AND retained_package_coordinate IS NULL",
        )
        .bind(&package.game_coordinate)
        .bind(package.updated_at)
        .bind(installation_id.as_str())
        .execute(&mut *transaction)
        .await?;
        if result.rows_affected() != 1 {
            return Err(AdpStorageError::InvalidTransition);
        }
        transaction.commit().await?;
        Ok(())
    }

    /// Attaches an already persisted immutable package to a matching installing
    /// row without replacing either record.
    pub async fn attach_existing_retained_installing(
        &self,
        installation_id: &InstallationId,
        package: &RetainedPackage,
        updated_at: i64,
    ) -> Result<(), AdpStorageError> {
        validate_retained_package(package)?;
        let mut transaction = self.pool.begin().await?;
        let retained = sqlx::query(&format!(
            "{RETAINED_PACKAGE_SELECT} WHERE game_coordinate = ?"
        ))
        .bind(&package.game_coordinate)
        .fetch_optional(&mut *transaction)
        .await?
        .map(retained_package_from_row)
        .transpose()?;
        if retained.as_ref() != Some(package) {
            return Err(AdpStorageError::InvalidData(
                "existing retained package changed before attachment".to_string(),
            ));
        }
        let installation: Option<(String, String, String, Option<String>)> = sqlx::query_as(
            "SELECT raw_event_json, resolved_os, resolved_arch, publisher_default_profile_id FROM installations \
             WHERE id = ? AND state = 'installing'",
        )
        .bind(installation_id.as_str())
        .fetch_optional(&mut *transaction)
        .await?;
        let stored_profile_json: Vec<String> = sqlx::query_scalar(
            "SELECT profile_json FROM installation_profiles WHERE installation_id = ? \
             ORDER BY profile_order",
        )
        .bind(installation_id.as_str())
        .fetch_all(&mut *transaction)
        .await?;
        let stored_profiles = stored_profile_json
            .into_iter()
            .map(|json| serde_json::from_str::<LaunchProfile>(&json))
            .collect::<Result<Vec<_>, _>>()?;
        let Some((raw, os, arch, publisher_default)) = installation else {
            return Err(AdpStorageError::InvalidData(
                "installing row does not match retained reinstall snapshot".to_string(),
            ));
        };
        let retained_publisher_default = package
            .reinstall
            .publisher_default_profile_id
            .as_ref()
            .map(ProfileId::as_str);
        if raw != package.reinstall.raw_event_json
            || os != package.reinstall.resolved_target.os
            || arch != package.reinstall.resolved_target.arch
            || retained_publisher_default.is_some()
                && publisher_default.as_deref() != retained_publisher_default
            || stored_profiles != package.reinstall.launch_profiles
        {
            return Err(AdpStorageError::InvalidData(
                "installing row does not match retained reinstall snapshot".to_string(),
            ));
        }
        // Migration 16 left pre-existing retained snapshots null. The new
        // installation has already revalidated the same signed event and target,
        // so pin its resolved scalar into the legacy retained row atomically.
        if retained_publisher_default.is_none() {
            sqlx::query(
                "UPDATE retained_packages SET publisher_default_profile_id = ?, updated_at = ? \
                 WHERE game_coordinate = ? AND publisher_default_profile_id IS NULL",
            )
            .bind(&publisher_default)
            .bind(updated_at)
            .bind(&package.game_coordinate)
            .execute(&mut *transaction)
            .await?;
        }
        let result = sqlx::query(
            "UPDATE installations SET retained_package_coordinate = ?, updated_at = ? \
             WHERE id = ? AND state = 'installing' AND game_coordinate = ? \
             AND package_digest = ? AND package_format = ? \
             AND retained_package_coordinate IS NULL",
        )
        .bind(&package.game_coordinate)
        .bind(updated_at)
        .bind(installation_id.as_str())
        .bind(&package.game_coordinate)
        .bind(package.package_digest.as_str())
        .bind(package_format_text(&package.package_format))
        .execute(&mut *transaction)
        .await?;
        if result.rows_affected() != 1 {
            return Err(AdpStorageError::InvalidTransition);
        }
        transaction.commit().await?;
        Ok(())
    }

    pub async fn get_retained_package(
        &self,
        coordinate: &str,
    ) -> Result<Option<RetainedPackage>, AdpStorageError> {
        let row = sqlx::query(&format!(
            "{RETAINED_PACKAGE_SELECT} WHERE game_coordinate = ?"
        ))
        .bind(coordinate)
        .fetch_optional(&self.pool)
        .await?;
        row.map(retained_package_from_row).transpose()
    }

    pub async fn list_retained_packages(&self) -> Result<Vec<RetainedPackage>, AdpStorageError> {
        sqlx::query(&format!(
            "{RETAINED_PACKAGE_SELECT} ORDER BY game_coordinate"
        ))
        .fetch_all(&self.pool)
        .await?
        .into_iter()
        .map(retained_package_from_row)
        .collect()
    }

    pub async fn delete_retained_package(&self, coordinate: &str) -> Result<(), AdpStorageError> {
        let mut transaction = self.pool.begin().await?;
        let existing = sqlx::query(&format!(
            "{RETAINED_PACKAGE_SELECT} WHERE game_coordinate = ?"
        ))
        .bind(coordinate)
        .fetch_optional(&mut *transaction)
        .await?
        .map(retained_package_from_row)
        .transpose()?;
        if existing.is_none() {
            return Ok(());
        }
        sqlx::query("DELETE FROM retained_packages WHERE game_coordinate = ?")
            .bind(coordinate)
            .execute(&mut *transaction)
            .await?;
        transaction.commit().await?;
        Ok(())
    }

    async fn snapshot_from_row(
        &self,
        row: sqlx::sqlite::SqliteRow,
        transaction: &mut Transaction<'_, Sqlite>,
    ) -> Result<InstallationSnapshot, AdpStorageError> {
        let installation = installation_from_row(row)?;
        let profile_rows = sqlx::query(
            r#"SELECT profile_order, profile_json, availability, unavailable_reason
               FROM installation_profiles WHERE installation_id = ? ORDER BY profile_order"#,
        )
        .bind(installation.id.as_str())
        .fetch_all(&mut **transaction)
        .await?;
        let mut profiles = Vec::with_capacity(profile_rows.len());
        for (expected_order, row) in profile_rows.into_iter().enumerate() {
            let stored_order: i64 = row.try_get("profile_order")?;
            let expected_order = i64::try_from(expected_order)
                .map_err(|_| AdpStorageError::OutOfRange("profile order"))?;
            if stored_order != expected_order {
                return Err(AdpStorageError::InvalidData(
                    "stored profile order is not contiguous".to_string(),
                ));
            }
            profiles.push(installation_profile_from_row(row)?);
        }
        let manifest = sqlx::query(
            r#"SELECT path, entry_type, file_digest FROM installation_manifest_entries
               WHERE installation_id = ? ORDER BY path_key"#,
        )
        .bind(installation.id.as_str())
        .fetch_all(&mut **transaction)
        .await?
        .into_iter()
        .map(manifest_entry_from_row)
        .collect::<Result<Vec<_>, _>>()?;
        let snapshot = InstallationSnapshot {
            installation,
            profiles,
            manifest,
        };
        if let Some(coordinate) = &snapshot.installation.retained_package_coordinate {
            let retained = sqlx::query(&format!(
                "{RETAINED_PACKAGE_SELECT} WHERE game_coordinate = ?"
            ))
            .bind(coordinate)
            .fetch_optional(&mut **transaction)
            .await?
            .ok_or_else(|| {
                AdpStorageError::InvalidData(
                    "installation references a missing retained package".to_string(),
                )
            })
            .and_then(retained_package_from_row)?;
            let installation_profiles = snapshot
                .profiles
                .iter()
                .map(|profile| &profile.profile)
                .collect::<Vec<_>>();
            if retained.package_digest != snapshot.installation.package_digest
                || retained.package_format != snapshot.installation.package_format
                || retained.reinstall.raw_event_json != snapshot.installation.raw_event_json
                || retained.reinstall.resolved_target != snapshot.installation.resolved_target
                || retained.reinstall.publisher_default_profile_id
                    != snapshot.installation.publisher_default_profile_id
                || retained
                    .reinstall
                    .launch_profiles
                    .iter()
                    .collect::<Vec<_>>()
                    != installation_profiles
            {
                return Err(AdpStorageError::InvalidData(
                    "retained package does not match installation snapshot".to_string(),
                ));
            }
        }
        Ok(snapshot)
    }
}

const INSTALLATION_SELECT: &str = r#"SELECT id, game_coordinate, installation_root,
    package_digest, package_format, resolved_os, resolved_arch, raw_event_json, state,
    recovery_discriminator, retained_package_coordinate, publisher_default_profile_id,
    created_at, updated_at FROM installations"#;
const RETAINED_PACKAGE_SELECT: &str = r#"SELECT game_coordinate, package_id, package_path,
    package_digest, package_format, raw_event_json, resolved_os, resolved_arch, profiles_json,
    publisher_default_profile_id, size_bytes, created_at, updated_at
    FROM retained_packages"#;

fn installation_from_row(
    row: sqlx::sqlite::SqliteRow,
) -> Result<InstallationRecord, AdpStorageError> {
    let id = InstallationId::parse(&row.try_get::<String, _>("id")?)?;
    let game_coordinate: String = row.try_get("game_coordinate")?;
    let installation_root = PathBuf::from(row.try_get::<String, _>("installation_root")?);
    validate_absolute_path(&installation_root, "installation root")?;
    let package_digest = Sha256Digest::parse(&row.try_get::<String, _>("package_digest")?)
        .map_err(|error| AdpStorageError::InvalidData(error.to_string()))?;
    let package_format = parse_package_format(&row.try_get::<String, _>("package_format")?)?;
    let raw_event_json: String = row.try_get("raw_event_json")?;
    validate_signed_listing(&raw_event_json, &game_coordinate)?;
    let retained_package_coordinate: Option<String> = row.try_get("retained_package_coordinate")?;
    validate_retained_reference(retained_package_coordinate.as_deref(), &game_coordinate)?;
    let resolved_target = PlatformTarget {
        os: row.try_get("resolved_os")?,
        arch: row.try_get("resolved_arch")?,
    };
    validate_platform_target(&resolved_target)?;
    let state = parse_installation_state(&row.try_get::<String, _>("state")?)?;
    let recovery_discriminator = row
        .try_get::<Option<String>, _>("recovery_discriminator")?
        .map(|value| parse_recovery_discriminator(&value))
        .transpose()?;
    validate_recovery_discriminator(state, recovery_discriminator)?;
    let publisher_default_profile_id = row
        .try_get::<Option<String>, _>("publisher_default_profile_id")?
        .map(|value| {
            ProfileId::parse(&value).map_err(|error| {
                AdpStorageError::InvalidData(format!(
                    "invalid stored publisher default profile id: {error}"
                ))
            })
        })
        .transpose()?;
    Ok(InstallationRecord {
        id,
        game_coordinate,
        installation_root,
        package_digest,
        package_format,
        resolved_target,
        raw_event_json,
        publisher_default_profile_id,
        state,
        recovery_discriminator,
        retained_package_coordinate,
        created_at: row.try_get("created_at")?,
        updated_at: row.try_get("updated_at")?,
    })
}

fn installation_profile_from_row(
    row: sqlx::sqlite::SqliteRow,
) -> Result<InstallationProfile, AdpStorageError> {
    let profile_json: String = row.try_get("profile_json")?;
    let profile = serde_json::from_str(&profile_json)?;
    let availability: String = row.try_get("availability")?;
    let reason: Option<String> = row.try_get("unavailable_reason")?;
    let availability = match (availability.as_str(), reason.as_deref()) {
        ("available", None) => ProfileAvailability::Available,
        ("unavailable", Some(reason)) => {
            ProfileAvailability::Unavailable(parse_unavailable_reason(reason)?)
        }
        _ => {
            return Err(AdpStorageError::InvalidData(
                "invalid stored profile availability".to_string(),
            ))
        }
    };
    Ok(InstallationProfile {
        profile,
        availability,
    })
}

fn manifest_entry_from_row(
    row: sqlx::sqlite::SqliteRow,
) -> Result<InstallationManifestEntry, AdpStorageError> {
    let path = RelativePath::parse(&row.try_get::<String, _>("path")?)
        .map_err(|error| AdpStorageError::InvalidData(error.to_string()))?;
    let entry_type = parse_manifest_entry_type(&row.try_get::<String, _>("entry_type")?)?;
    let digest = row.try_get::<Option<String>, _>("file_digest")?;
    let file_digest = digest
        .map(|value| {
            Sha256Digest::parse(&value)
                .map_err(|error| AdpStorageError::InvalidData(error.to_string()))
        })
        .transpose()?;
    let entry = InstallationManifestEntry {
        path,
        entry_type,
        file_digest,
    };
    validate_manifest_entry(&entry)?;
    Ok(entry)
}

fn retained_package_from_row(
    row: sqlx::sqlite::SqliteRow,
) -> Result<RetainedPackage, AdpStorageError> {
    let package_path = PathBuf::from(row.try_get::<String, _>("package_path")?);
    validate_absolute_path(&package_path, "retained package path")?;
    let stored_size: i64 = row.try_get("size_bytes")?;
    let size_bytes = u64::try_from(stored_size)
        .map_err(|_| AdpStorageError::InvalidData("negative retained package size".to_string()))?;
    let package_id = RetainedPackageId::parse(&row.try_get::<String, _>("package_id")?)?;
    let reinstall = RetainedPackageReinstallSnapshot {
        raw_event_json: row.try_get("raw_event_json")?,
        resolved_target: PlatformTarget {
            os: row.try_get("resolved_os")?,
            arch: row.try_get("resolved_arch")?,
        },
        launch_profiles: serde_json::from_str(&row.try_get::<String, _>("profiles_json")?)?,
        publisher_default_profile_id: row
            .try_get::<Option<String>, _>("publisher_default_profile_id")?
            .map(|value| {
                ProfileId::parse(&value).map_err(|error| {
                    AdpStorageError::InvalidData(format!(
                        "invalid retained publisher default profile id: {error}"
                    ))
                })
            })
            .transpose()?,
    };
    let package = RetainedPackage {
        game_coordinate: row.try_get("game_coordinate")?,
        package_id,
        package_path,
        package_digest: Sha256Digest::parse(&row.try_get::<String, _>("package_digest")?)
            .map_err(|error| AdpStorageError::InvalidData(error.to_string()))?,
        package_format: parse_package_format(&row.try_get::<String, _>("package_format")?)?,
        reinstall,
        size_bytes,
        created_at: row.try_get("created_at")?,
        updated_at: row.try_get("updated_at")?,
    };
    validate_retained_package_boundary(&package)?;
    Ok(package)
}

fn validate_retained_reference(
    retained_coordinate: Option<&str>,
    installation_coordinate: &str,
) -> Result<(), AdpStorageError> {
    if retained_coordinate.is_some_and(|coordinate| coordinate != installation_coordinate) {
        return Err(AdpStorageError::InvalidData(
            "retained package coordinate must match the installation coordinate".to_string(),
        ));
    }
    Ok(())
}

fn validate_uuid_v4(value: &str, name: &'static str) -> Result<(), AdpStorageError> {
    let uuid = Uuid::parse_str(value)
        .map_err(|error| AdpStorageError::InvalidData(format!("invalid {name}: {error}")))?;
    if uuid.get_version() != Some(Version::Random) || uuid.simple().to_string() != value {
        return Err(AdpStorageError::InvalidData(format!(
            "{name} must be a canonical lowercase hyphenless UUID v4"
        )));
    }
    Ok(())
}

fn validate_signed_listing(raw: &str, coordinate: &str) -> Result<Event, AdpStorageError> {
    let event: Event = serde_json::from_str(raw)?;
    if event.kind.as_u16() != crate::adp_protocol::NIP99_LISTING_KIND || event.verify().is_err() {
        return Err(AdpStorageError::InvalidData(
            "raw event must be a valid signed kind:30402 event".to_string(),
        ));
    }
    let identifier = event
        .tags
        .identifier()
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            AdpStorageError::InvalidData("raw event is missing its d tag".to_string())
        })?;
    let event_coordinate = format!("30402:{}:{identifier}", event.pubkey.to_hex());
    if event_coordinate != coordinate {
        return Err(AdpStorageError::InvalidData(
            "raw event does not match installation coordinate".to_string(),
        ));
    }
    Ok(event)
}

fn validate_resolved_snapshot(
    installation: &InstallationRecord,
    profiles: &[InstallationProfile],
) -> Result<(), AdpStorageError> {
    let event =
        validate_signed_listing(&installation.raw_event_json, &installation.game_coordinate)?;
    let resolved =
        resolve_install_plan(&event, &installation.resolved_target).map_err(|error| {
            AdpStorageError::InvalidData(format!("listing resolution failed: {error}"))
        })?;
    let coordinate = format!(
        "{}:{}:{}",
        resolved.plan.game_coordinate.kind,
        resolved.plan.game_coordinate.publisher_pubkey_hex,
        resolved.plan.game_coordinate.d_tag
    );
    let resolved_profiles = profiles
        .iter()
        .map(|profile| &profile.profile)
        .collect::<Vec<_>>();
    if coordinate != installation.game_coordinate
        || resolved.plan.artifact.digest != installation.package_digest
        || resolved.plan.artifact.package_format != installation.package_format
        || resolved.plan.launch_profiles.iter().collect::<Vec<_>>() != resolved_profiles
        || installation.publisher_default_profile_id.is_some()
            && resolved.plan.publisher_default_profile_id
                != installation.publisher_default_profile_id
    {
        return Err(AdpStorageError::InvalidData(
            "installation snapshot does not match its signed listing event".to_string(),
        ));
    }
    Ok(())
}

fn validate_retained_package(package: &RetainedPackage) -> Result<(), AdpStorageError> {
    let event = validate_retained_package_boundary(package)?;
    let resolved =
        resolve_install_plan(&event, &package.reinstall.resolved_target).map_err(|error| {
            AdpStorageError::InvalidData(format!(
                "retained package listing resolution failed: {error}"
            ))
        })?;
    let coordinate = format!(
        "{}:{}:{}",
        resolved.plan.game_coordinate.kind,
        resolved.plan.game_coordinate.publisher_pubkey_hex,
        resolved.plan.game_coordinate.d_tag
    );
    if coordinate != package.game_coordinate
        || resolved.plan.artifact.digest != package.package_digest
        || resolved.plan.artifact.package_format != package.package_format
        || resolved.plan.launch_profiles != package.reinstall.launch_profiles
        || package.reinstall.publisher_default_profile_id.is_some()
            && resolved.plan.publisher_default_profile_id
                != package.reinstall.publisher_default_profile_id
    {
        return Err(AdpStorageError::InvalidData(
            "retained reinstall snapshot does not match its signed listing event".to_string(),
        ));
    }
    Ok(())
}

fn validate_retained_package_boundary(package: &RetainedPackage) -> Result<Event, AdpStorageError> {
    validate_absolute_path(&package.package_path, "retained package path")?;
    validate_platform_target(&package.reinstall.resolved_target)?;
    validate_signed_listing(&package.reinstall.raw_event_json, &package.game_coordinate)
}

fn validate_absolute_path(path: &Path, name: &'static str) -> Result<(), AdpStorageError> {
    if !path.is_absolute()
        || path.to_str().is_none()
        || path
            .components()
            .any(|component| matches!(component, Component::CurDir | Component::ParentDir))
    {
        return Err(AdpStorageError::InvalidData(format!(
            "{name} must be an absolute normalized UTF-8 path"
        )));
    }
    Ok(())
}

fn validate_platform_target(target: &PlatformTarget) -> Result<(), AdpStorageError> {
    if !matches!(target.os.as_str(), "windows" | "linux" | "macos")
        || !matches!(target.arch.as_str(), "x86_64" | "aarch64")
    {
        return Err(AdpStorageError::InvalidData(
            "resolved target must use canonical OS and architecture values".to_string(),
        ));
    }
    Ok(())
}

fn path_text<'a>(path: &'a Path, name: &'static str) -> Result<&'a str, AdpStorageError> {
    path.to_str()
        .ok_or_else(|| AdpStorageError::InvalidData(format!("{name} must be UTF-8")))
}

fn validate_manifest_entry(entry: &InstallationManifestEntry) -> Result<(), AdpStorageError> {
    match (entry.entry_type, entry.file_digest.is_some()) {
        (ManifestEntryType::File, true) | (ManifestEntryType::Directory, false) => Ok(()),
        _ => Err(AdpStorageError::InvalidData(
            "files require a digest and directories forbid one".to_string(),
        )),
    }
}

fn manifest_path_key(path: &RelativePath) -> String {
    manifest_path_key_text(path.as_str())
}

fn manifest_path_key_text(path: &str) -> String {
    path.chars()
        .flat_map(char::to_uppercase)
        .flat_map(char::to_lowercase)
        .collect::<String>()
        .nfc()
        .collect()
}

fn validate_complete_manifest(snapshot: &InstallationSnapshot) -> Result<(), AdpStorageError> {
    if snapshot.manifest.is_empty() {
        return Err(AdpStorageError::InvalidData(
            "a ready installation requires a manifest".to_string(),
        ));
    }
    for profile in &snapshot.profiles {
        if profile.availability != ProfileAvailability::Available {
            continue;
        }
        let executable_key = manifest_path_key(&profile.profile.executable);
        if !snapshot.manifest.iter().any(|entry| {
            manifest_path_key(&entry.path) == executable_key
                && entry.entry_type == ManifestEntryType::File
        }) {
            return Err(AdpStorageError::InvalidData(format!(
                "available profile {} has no regular-file manifest entry",
                profile.profile.id.as_str()
            )));
        }
        if !profile.profile.working_dir.is_root() {
            let working_directory_key =
                manifest_path_key_text(profile.profile.working_dir.as_str());
            if !snapshot.manifest.iter().any(|entry| {
                manifest_path_key(&entry.path) == working_directory_key
                    && entry.entry_type == ManifestEntryType::Directory
            }) {
                return Err(AdpStorageError::InvalidData(format!(
                    "available profile {} has no directory manifest entry",
                    profile.profile.id.as_str()
                )));
            }
        }
    }
    Ok(())
}

fn installation_state_text(state: InstallationState) -> &'static str {
    match state {
        InstallationState::Installing => "installing",
        InstallationState::Ready => "ready",
        InstallationState::Failed => "failed",
        InstallationState::Uninstalling => "uninstalling",
    }
}

fn parse_installation_state(value: &str) -> Result<InstallationState, AdpStorageError> {
    match value {
        "installing" => Ok(InstallationState::Installing),
        "ready" => Ok(InstallationState::Ready),
        "failed" => Ok(InstallationState::Failed),
        "uninstalling" => Ok(InstallationState::Uninstalling),
        _ => Err(AdpStorageError::InvalidData(
            "invalid installation state".to_string(),
        )),
    }
}

fn recovery_discriminator_text(discriminator: InstallationRecoveryDiscriminator) -> &'static str {
    match discriminator {
        InstallationRecoveryDiscriminator::FinalRootAbsent => "final_root_absent",
        InstallationRecoveryDiscriminator::FinalRootPresent => "final_root_present",
    }
}

fn parse_recovery_discriminator(
    value: &str,
) -> Result<InstallationRecoveryDiscriminator, AdpStorageError> {
    match value {
        "final_root_absent" => Ok(InstallationRecoveryDiscriminator::FinalRootAbsent),
        "final_root_present" => Ok(InstallationRecoveryDiscriminator::FinalRootPresent),
        _ => Err(AdpStorageError::InvalidData(
            "invalid installation recovery discriminator".to_string(),
        )),
    }
}

fn validate_recovery_discriminator(
    state: InstallationState,
    discriminator: Option<InstallationRecoveryDiscriminator>,
) -> Result<(), AdpStorageError> {
    if discriminator.is_some() && state != InstallationState::Failed {
        return Err(AdpStorageError::InvalidData(
            "installation recovery discriminator requires failed state".to_string(),
        ));
    }
    Ok(())
}

fn manifest_entry_type_text(entry_type: ManifestEntryType) -> &'static str {
    match entry_type {
        ManifestEntryType::File => "file",
        ManifestEntryType::Directory => "directory",
    }
}

fn parse_manifest_entry_type(value: &str) -> Result<ManifestEntryType, AdpStorageError> {
    match value {
        "file" => Ok(ManifestEntryType::File),
        "directory" => Ok(ManifestEntryType::Directory),
        _ => Err(AdpStorageError::InvalidData(
            "invalid manifest entry type".to_string(),
        )),
    }
}

fn package_format_text(format: &PackageFormat) -> &'static str {
    match format {
        PackageFormat::Zip => "zip",
        PackageFormat::TarZstd => "tar_zstd",
        PackageFormat::TarGz => "tar_gz",
    }
}

fn parse_package_format(value: &str) -> Result<PackageFormat, AdpStorageError> {
    match value {
        "zip" => Ok(PackageFormat::Zip),
        "tar_zstd" => Ok(PackageFormat::TarZstd),
        "tar_gz" => Ok(PackageFormat::TarGz),
        _ => Err(AdpStorageError::InvalidData(
            "invalid package format".to_string(),
        )),
    }
}

fn profile_availability_text(
    availability: &ProfileAvailability,
) -> (&'static str, Option<&'static str>) {
    match availability {
        ProfileAvailability::Available => ("available", None),
        ProfileAvailability::Unavailable(reason) => ("unavailable", Some(reason_text(*reason))),
    }
}

fn reason_text(reason: ProfileUnavailableReason) -> &'static str {
    match reason {
        ProfileUnavailableReason::ExecutableMissing => "executable_missing",
        ProfileUnavailableReason::ExecutableNotFile => "executable_not_file",
        ProfileUnavailableReason::WorkingDirectoryMissing => "working_directory_missing",
        ProfileUnavailableReason::WorkingDirectoryNotDirectory => "working_directory_not_directory",
        ProfileUnavailableReason::OutsideInstallationRoot => "outside_installation_root",
    }
}

fn parse_unavailable_reason(value: &str) -> Result<ProfileUnavailableReason, AdpStorageError> {
    match value {
        "executable_missing" => Ok(ProfileUnavailableReason::ExecutableMissing),
        "executable_not_file" => Ok(ProfileUnavailableReason::ExecutableNotFile),
        "working_directory_missing" => Ok(ProfileUnavailableReason::WorkingDirectoryMissing),
        "working_directory_not_directory" => {
            Ok(ProfileUnavailableReason::WorkingDirectoryNotDirectory)
        }
        "outside_installation_root" => Ok(ProfileUnavailableReason::OutsideInstallationRoot),
        _ => Err(AdpStorageError::InvalidData(
            "invalid profile unavailability reason".to_string(),
        )),
    }
}

fn adp_provisioning_from_row(
    row: sqlx::sqlite::SqliteRow,
) -> Result<AdpProvisioning, AdpStorageError> {
    let authorization_capabilities = row
        .try_get::<Option<String>, _>("authorization_capabilities_json")?
        .map(|json| serde_json::from_str(&json))
        .transpose()?
        .unwrap_or_default();
    Ok(AdpProvisioning {
        id: row.try_get("id")?,
        developer_npub: row.try_get("developer_npub")?,
        server_url: row.try_get("server_url")?,
        operator_pubkey: row.try_get("operator_pubkey")?,
        scope: row.try_get("scope")?,
        fulfillment_pubkey: row.try_get("fulfillment_pubkey")?,
        attestation_event_id: row.try_get("attestation_event_id")?,
        acceptance_event_id: row.try_get("acceptance_event_id")?,
        authorization_root_event_id: row.try_get("authorization_root_event_id")?,
        authorization_capabilities,
        authorization_profile_version: row.try_get("authorization_profile_version")?,
        valid_from: row.try_get("valid_from")?,
        revoked_at: row.try_get("revoked_at")?,
        created_at: row.try_get("created_at")?,
    })
}

fn download_token_from_row(row: sqlx::sqlite::SqliteRow) -> Result<DownloadToken, AdpStorageError> {
    Ok(DownloadToken {
        buyer_pubkey: row.try_get("buyer_pubkey")?,
        game_coordinate: row.try_get("game_coordinate")?,
        server_url: row.try_get("server_url")?,
        token: row.try_get("token")?,
        expires_at: row.try_get("expires_at")?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::distribution::{LaunchType, ProfileId, WorkingDir};
    use crate::storage::Database;
    use nostr::{EventBuilder, Keys, Kind, Tag};

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

    fn digest() -> Sha256Digest {
        Sha256Digest::parse(&"ab".repeat(32)).expect("valid digest")
    }

    fn installation(d_tag: &str, state: InstallationState) -> InstallationRecord {
        let keys = Keys::generate();
        let event = EventBuilder::new(Kind::Custom(30402), "listing")
            .tags([
                Tag::parse(["d", d_tag]).expect("d tag"),
                Tag::parse(["file_hash", &"ab".repeat(32)]).expect("file hash tag"),
                Tag::parse(["server", "https://dist.example.com"]).expect("server tag"),
                Tag::parse(["package", "tar.zst"]).expect("package tag"),
                Tag::parse(["platform", "linux-x86_64"]).expect("platform tag"),
                Tag::parse(["platform", "linux-aarch64"]).expect("alternate platform tag"),
                Tag::parse([
                    "launch",
                    "play",
                    "bin/game",
                    ".",
                    "[\"--fullscreen\"]",
                    "game",
                    "Play",
                ])
                .expect("game launch tag"),
                Tag::parse([
                    "launch",
                    "editor",
                    "tools/editor",
                    "tools",
                    "[]",
                    "editor",
                    "Editor",
                ])
                .expect("editor launch tag"),
                Tag::parse(["launch_default", "play"]).expect("launch default tag"),
            ])
            .sign_with_keys(&keys)
            .expect("signed listing");
        InstallationRecord {
            id: InstallationId::generate(),
            game_coordinate: format!("30402:{}:{d_tag}", keys.public_key().to_hex()),
            installation_root: PathBuf::from(format!("/games/{d_tag}")),
            package_digest: digest(),
            package_format: PackageFormat::TarZstd,
            resolved_target: PlatformTarget {
                os: "linux".to_string(),
                arch: "x86_64".to_string(),
            },
            raw_event_json: serde_json::to_string(&event).expect("event JSON"),
            publisher_default_profile_id: Some(ProfileId::parse("play").expect("default id")),
            state,
            recovery_discriminator: None,
            retained_package_coordinate: None,
            created_at: 1,
            updated_at: 1,
        }
    }

    fn profiles() -> Vec<InstallationProfile> {
        vec![
            InstallationProfile {
                profile: LaunchProfile {
                    id: ProfileId::parse("play").expect("profile id"),
                    display_name: "Play".to_string(),
                    executable: RelativePath::parse("bin/game").expect("executable"),
                    working_dir: WorkingDir::parse(".").expect("working dir"),
                    arguments: vec!["--fullscreen".to_string()],
                    launch_type: LaunchType::Game,
                },
                availability: ProfileAvailability::Available,
            },
            InstallationProfile {
                profile: LaunchProfile {
                    id: ProfileId::parse("editor").expect("profile id"),
                    display_name: "Editor".to_string(),
                    executable: RelativePath::parse("tools/editor").expect("executable"),
                    working_dir: WorkingDir::parse("tools").expect("working dir"),
                    arguments: Vec::new(),
                    launch_type: LaunchType::Editor,
                },
                availability: ProfileAvailability::Unavailable(
                    ProfileUnavailableReason::WorkingDirectoryMissing,
                ),
            },
        ]
    }

    fn reinstall_snapshot(installation: &InstallationRecord) -> RetainedPackageReinstallSnapshot {
        RetainedPackageReinstallSnapshot {
            raw_event_json: installation.raw_event_json.clone(),
            resolved_target: installation.resolved_target.clone(),
            launch_profiles: profiles()
                .into_iter()
                .map(|profile| profile.profile)
                .collect(),
            publisher_default_profile_id: installation.publisher_default_profile_id.clone(),
        }
    }

    fn ready_manifest() -> Vec<InstallationManifestEntry> {
        vec![InstallationManifestEntry {
            path: RelativePath::parse("bin/game").expect("valid file path"),
            entry_type: ManifestEntryType::File,
            file_digest: Some(digest()),
        }]
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
            authorization_root_event_id: Some("authorization-root".to_string()),
            authorization_capabilities: vec!["upload_build".to_string()],
            authorization_profile_version: 2,
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
    async fn legacy_provisioning_row_is_audit_only_and_not_reusable() {
        let db = test_db().await;
        let repo = AdpProvisioningRepository::new(db.pool().clone());
        let mut entry = provisioning(Some("game"));
        entry.authorization_root_event_id = None;
        entry.authorization_capabilities.clear();
        entry.authorization_profile_version = 1;
        repo.upsert(&entry).await.expect("legacy row persists");

        assert!(repo
            .active_for_scope("developer", "https://dist.example.com", Some("game"))
            .await
            .expect("lookup")
            .is_none());
        assert_eq!(
            repo.for_fulfillment_scope("developer", "fulfillment-game", "game")
                .await
                .expect("audit lookup")
                .len(),
            1
        );
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
            buyer_pubkey: "buyer-1".to_string(),
            game_coordinate: "30402:dev:game".to_string(),
            server_url: "https://dist.example.com".to_string(),
            token: "token-1".to_string(),
            expires_at: 100,
        };

        repo.upsert(&token).await.expect("insert should succeed");
        assert_eq!(
            repo.valid_token("buyer-1", "30402:dev:game", "https://dist.example.com", 50,)
                .await
                .expect("lookup should succeed")
                .expect("token should be valid")
                .token,
            "token-1"
        );
        assert!(repo
            .valid_token("buyer-1", "30402:dev:game", "https://dist.example.com", 101,)
            .await
            .expect("lookup should succeed")
            .is_none());

        repo.delete("buyer-1", "30402:dev:game", "https://dist.example.com")
            .await
            .expect("delete should succeed");
        assert!(repo
            .valid_token("buyer-1", "30402:dev:game", "https://dist.example.com", 50,)
            .await
            .expect("lookup should succeed")
            .is_none());
    }

    #[tokio::test]
    async fn download_tokens_upsert_replaces_existing_token() {
        let db = test_db().await;
        let repo = DownloadTokensRepository::new(db.pool().clone());
        let original = DownloadToken {
            buyer_pubkey: "buyer-1".to_string(),
            game_coordinate: "30402:dev:game".to_string(),
            server_url: "https://dist.example.com".to_string(),
            token: "token-1".to_string(),
            expires_at: 100,
        };
        let replacement = DownloadToken {
            buyer_pubkey: "buyer-1".to_string(),
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
            .valid_token("buyer-1", "30402:dev:game", "https://dist.example.com", 150)
            .await
            .expect("lookup should succeed")
            .expect("replacement token should be valid");

        assert_eq!(found.token, "token-2");
        assert_eq!(found.expires_at, 200);
    }

    #[tokio::test]
    async fn download_tokens_are_isolated_by_buyer() {
        let db = test_db().await;
        let repo = DownloadTokensRepository::new(db.pool().clone());
        let token = DownloadToken {
            buyer_pubkey: "buyer-1".to_string(),
            game_coordinate: "30402:dev:game".to_string(),
            server_url: "https://dist.example.com".to_string(),
            token: "buyer-1-token".to_string(),
            expires_at: 200,
        };

        repo.upsert(&token).await.expect("insert should succeed");

        assert!(repo
            .valid_token("buyer-2", "30402:dev:game", "https://dist.example.com", 100,)
            .await
            .expect("lookup should succeed")
            .is_none());
        assert_eq!(
            repo.valid_token("buyer-1", "30402:dev:game", "https://dist.example.com", 100,)
                .await
                .expect("lookup should succeed")
                .expect("owner token should remain available")
                .token,
            "buyer-1-token"
        );
    }

    #[tokio::test]
    async fn installation_snapshot_round_trips_profiles_manifest_and_signed_event() {
        let db = test_db().await;
        let repo = InstallationsRepository::new(db.pool().clone());
        let installation = installation("round-trip", InstallationState::Installing);
        let profiles = profiles();
        let manifest = vec![
            InstallationManifestEntry {
                path: RelativePath::parse("bin").expect("valid directory path"),
                entry_type: ManifestEntryType::Directory,
                file_digest: None,
            },
            InstallationManifestEntry {
                path: RelativePath::parse("bin/game").expect("valid file path"),
                entry_type: ManifestEntryType::File,
                file_digest: Some(digest()),
            },
        ];

        repo.begin(&installation, &profiles)
            .await
            .expect("begin should succeed");
        repo.replace_manifest(&installation.id, &manifest)
            .await
            .expect("manifest should be recorded");

        let snapshot = repo
            .get_by_id(&installation.id)
            .await
            .expect("snapshot query should succeed")
            .expect("snapshot should exist");
        assert_eq!(snapshot.installation, installation);
        assert_eq!(snapshot.profiles, profiles);
        assert_eq!(snapshot.manifest, manifest);
        let event: Event = serde_json::from_str(&snapshot.installation.raw_event_json)
            .expect("stored event should parse");
        assert!(event.verify().is_ok());
        assert_eq!(
            repo.get_by_coordinate(&installation.game_coordinate)
                .await
                .expect("coordinate query should succeed"),
            Some(snapshot.clone())
        );
        assert_eq!(
            repo.list().await.expect("list should succeed"),
            vec![snapshot]
        );
    }

    #[tokio::test]
    async fn launch_preference_survives_uninstall_and_new_installation_identity() {
        let db = test_db().await;
        let installations = InstallationsRepository::new(db.pool().clone());
        let preferences = LaunchProfilePreferencesRepository::new(db.pool().clone());
        let first = installation("preference-reinstall", InstallationState::Installing);
        let coordinate = first.game_coordinate.clone();
        let preferred = ProfileId::parse("play").expect("profile id");

        installations
            .begin(&first, &profiles())
            .await
            .expect("first installation");
        preferences
            .set(&coordinate, &preferred, 10)
            .await
            .expect("save preference");
        preferences
            .set(&coordinate, &preferred, 20)
            .await
            .expect("update preference timestamp");
        let updated_at: i64 = sqlx::query_scalar(
            "SELECT updated_at FROM launch_profile_preferences WHERE game_coordinate = ?",
        )
        .bind(&coordinate)
        .fetch_one(db.pool())
        .await
        .expect("preference timestamp");
        assert_eq!(updated_at, 20);

        installations
            .replace_manifest(&first.id, &ready_manifest())
            .await
            .expect("ready manifest");
        installations
            .transition(&first.id, InstallationState::Ready, 21)
            .await
            .expect("ready transition");
        installations
            .transition(&first.id, InstallationState::Uninstalling, 22)
            .await
            .expect("uninstall transition");
        installations
            .delete(&first.id)
            .await
            .expect("uninstall delete");

        let mut second = first.clone();
        second.id = InstallationId::generate();
        second.installation_root = PathBuf::from("/games/preference-reinstall-new");
        second.state = InstallationState::Installing;
        second.updated_at = 23;
        installations
            .begin(&second, &profiles())
            .await
            .expect("reinstallation with a new identity");
        assert_ne!(first.id, second.id);
        assert_eq!(
            preferences.get(&coordinate).await.expect("load preference"),
            Some(LaunchProfilePreference {
                profile_id: Some(preferred.as_str().to_owned()),
                updated_at: 20,
            })
        );

        let stale = ProfileId::parse("stale").expect("stale profile id");
        preferences
            .set(&coordinate, &stale, 15)
            .await
            .expect("stale write is ignored");
        assert_eq!(
            preferences
                .get(&coordinate)
                .await
                .expect("newer value wins"),
            Some(LaunchProfilePreference {
                profile_id: Some(preferred.as_str().to_owned()),
                updated_at: 20,
            })
        );

        sqlx::query(
            "UPDATE launch_profile_preferences SET profile_id = 'Bad' WHERE game_coordinate = ?",
        )
        .bind(&coordinate)
        .execute(db.pool())
        .await
        .expect("corrupt preference fixture");
        assert_eq!(
            preferences
                .get(&coordinate)
                .await
                .expect("raw corrupt value"),
            Some(LaunchProfilePreference {
                profile_id: Some("Bad".into()),
                updated_at: 20,
            })
        );
        preferences
            .clear(&coordinate, 30)
            .await
            .expect("clear preference");
        assert_eq!(
            preferences.get(&coordinate).await.expect("cleared"),
            Some(LaunchProfilePreference {
                profile_id: None,
                updated_at: 30,
            })
        );
        preferences
            .set(&coordinate, &preferred, 25)
            .await
            .expect("stale set after clear is ignored");
        assert_eq!(
            preferences
                .get(&coordinate)
                .await
                .expect("clear tombstone wins"),
            Some(LaunchProfilePreference {
                profile_id: None,
                updated_at: 30,
            })
        );
    }

    #[tokio::test]
    async fn installation_roots_are_unique_without_case_aliases() {
        let database = test_db().await;
        let repository = InstallationsRepository::new(database.pool().clone());
        let first = installation("root-owner-a", InstallationState::Installing);
        repository
            .begin(&first, &profiles())
            .await
            .expect("first installation root claim");

        let mut alias = installation("root-owner-b", InstallationState::Installing);
        alias.installation_root = PathBuf::from(
            first
                .installation_root
                .to_string_lossy()
                .to_ascii_uppercase(),
        );
        let error = repository
            .begin(&alias, &profiles())
            .await
            .expect_err("case-aliased installation root must be rejected");
        assert!(matches!(error, AdpStorageError::Sql(_)));
    }

    #[tokio::test]
    async fn commit_extraction_is_atomic_when_ready_transition_fails() {
        let db = test_db().await;
        let repo = InstallationsRepository::new(db.pool().clone());
        let installation = installation("atomic-commit", InstallationState::Installing);
        let initial_profiles = profiles();
        repo.begin(&installation, &initial_profiles)
            .await
            .expect("begin should succeed");
        sqlx::query(
            r#"CREATE TRIGGER fail_installation_ready
               BEFORE UPDATE OF state ON installations
               WHEN NEW.state = 'ready'
               BEGIN SELECT RAISE(ABORT, 'injected ready failure'); END"#,
        )
        .execute(db.pool())
        .await
        .expect("failure trigger should be installed");
        let mut committed_profiles = initial_profiles.clone();
        committed_profiles[0].availability =
            ProfileAvailability::Unavailable(ProfileUnavailableReason::ExecutableMissing);

        assert!(matches!(
            repo.commit_extraction(
                &installation.id,
                &installation.installation_root,
                &installation.package_digest,
                &installation.package_format,
                &committed_profiles,
                &ready_manifest(),
                2,
            )
            .await,
            Err(AdpStorageError::Sql(_))
        ));

        let snapshot = repo
            .get_by_id(&installation.id)
            .await
            .expect("snapshot query should succeed")
            .expect("installing snapshot should remain");
        assert_eq!(snapshot.installation.state, InstallationState::Installing);
        assert_eq!(snapshot.profiles, initial_profiles);
        assert!(snapshot.manifest.is_empty());
    }

    #[tokio::test]
    async fn manifest_lookup_uses_normalized_key_for_sharp_s() {
        let db = test_db().await;
        let repo = InstallationsRepository::new(db.pool().clone());
        let installation = installation("unicode-manifest-lookup", InstallationState::Installing);
        repo.begin(&installation, &profiles()).await.expect("begin");
        repo.replace_manifest(
            &installation.id,
            &[InstallationManifestEntry {
                path: RelativePath::parse("Straße").expect("stored manifest path"),
                entry_type: ManifestEntryType::Directory,
                file_digest: None,
            }],
        )
        .await
        .expect("manifest replacement");

        let entry = repo
            .manifest_entry(
                &installation.id,
                &RelativePath::parse("STRASSE").expect("lookup path"),
            )
            .await
            .expect("normalized manifest lookup")
            .expect("manifest entry");
        assert_eq!(entry.path.as_str(), "Straße");
    }

    #[tokio::test]
    async fn hostile_manifest_relative_path_fails_read() {
        let db = test_db().await;
        let repo = InstallationsRepository::new(db.pool().clone());
        let installation = installation("hostile", InstallationState::Installing);
        repo.begin(&installation, &profiles()).await.expect("begin");
        repo.replace_manifest(
            &installation.id,
            &[InstallationManifestEntry {
                path: RelativePath::parse("safe").expect("safe path"),
                entry_type: ManifestEntryType::Directory,
                file_digest: None,
            }],
        )
        .await
        .expect("manifest");
        sqlx::query(
            "UPDATE installation_manifest_entries SET path = '../escape' WHERE installation_id = ?",
        )
        .bind(installation.id.as_str())
        .execute(db.pool())
        .await
        .expect("hostile row injection");

        assert!(matches!(
            repo.get_by_id(&installation.id).await,
            Err(AdpStorageError::InvalidData(_))
        ));
    }

    #[tokio::test]
    async fn stored_profile_read_does_not_reresolve_the_signed_snapshot() {
        let db = test_db().await;
        let repo = InstallationsRepository::new(db.pool().clone());
        let installation = installation("hostile-profile", InstallationState::Installing);
        repo.begin(&installation, &profiles()).await.expect("begin");
        let mut changed = profiles()[0].profile.clone();
        changed.executable = RelativePath::parse("bin/other").expect("alternate executable");
        sqlx::query(
            "UPDATE installation_profiles SET profile_json = ? \
             WHERE installation_id = ? AND profile_order = 0",
        )
        .bind(serde_json::to_string(&changed).expect("profile JSON"))
        .bind(installation.id.as_str())
        .execute(db.pool())
        .await
        .expect("hostile profile injection");

        let snapshot = repo
            .get_by_id(&installation.id)
            .await
            .expect("typed pinned snapshot should remain readable")
            .expect("installation should exist");
        assert_eq!(snapshot.profiles[0].profile, changed);
    }

    #[tokio::test]
    async fn installation_state_machine_allows_only_declared_transitions() {
        let db = test_db().await;
        let repo = InstallationsRepository::new(db.pool().clone());

        let ready = installation("ready", InstallationState::Installing);
        repo.begin(&ready, &profiles())
            .await
            .expect("begin ready path");
        assert!(matches!(
            repo.transition(&ready.id, InstallationState::Uninstalling, 2)
                .await,
            Err(AdpStorageError::InvalidTransition)
        ));
        repo.replace_manifest(&ready.id, &ready_manifest())
            .await
            .expect("ready manifest");
        repo.transition(&ready.id, InstallationState::Ready, 2)
            .await
            .expect("installing to ready");
        assert!(matches!(
            repo.transition(&ready.id, InstallationState::Failed, 3)
                .await,
            Err(AdpStorageError::InvalidTransition)
        ));
        assert!(matches!(
            repo.delete(&ready.id).await,
            Err(AdpStorageError::InvalidTransition)
        ));
        repo.transition(&ready.id, InstallationState::Uninstalling, 4)
            .await
            .expect("ready to uninstalling");
        repo.delete(&ready.id).await.expect("delete uninstalling");

        let failed = installation("failed", InstallationState::Installing);
        repo.begin(&failed, &profiles())
            .await
            .expect("begin failed path");
        repo.transition(&failed.id, InstallationState::Failed, 2)
            .await
            .expect("installing to failed");
        assert!(matches!(
            repo.transition(&failed.id, InstallationState::Ready, 3)
                .await,
            Err(AdpStorageError::InvalidTransition)
        ));
        repo.delete_failed(&failed.id)
            .await
            .expect("failed row can be dismissed before retry");
    }

    #[tokio::test]
    async fn recovery_failure_transition_persists_both_final_root_outcomes() {
        let db = test_db().await;
        let repo = InstallationsRepository::new(db.pool().clone());

        for (index, discriminator) in [
            InstallationRecoveryDiscriminator::FinalRootAbsent,
            InstallationRecoveryDiscriminator::FinalRootPresent,
        ]
        .into_iter()
        .enumerate()
        {
            let installation = installation(
                &format!("recovery-outcome-{index}"),
                InstallationState::Installing,
            );
            repo.begin(&installation, &profiles())
                .await
                .expect("begin recovery path");
            repo.mark_failed_for_recovery(&installation.id, discriminator, 2)
                .await
                .expect("installing row should become a recovery failure");

            let stored = repo
                .get_by_id(&installation.id)
                .await
                .expect("recovery failure query")
                .expect("recovery failure should exist");
            assert_eq!(stored.installation.state, InstallationState::Failed);
            assert_eq!(
                stored.installation.recovery_discriminator,
                Some(discriminator)
            );
            assert_eq!(stored.installation.updated_at, 2);
        }
    }

    #[tokio::test]
    async fn recovery_failure_transition_rejects_no_longer_installing_rows() {
        let db = test_db().await;
        let repo = InstallationsRepository::new(db.pool().clone());
        let installation = installation("recovery-stale", InstallationState::Installing);
        repo.begin(&installation, &profiles()).await.expect("begin");
        repo.mark_failed(&installation.id, 2)
            .await
            .expect("ordinary failure");

        assert!(matches!(
            repo.mark_failed_for_recovery(
                &installation.id,
                InstallationRecoveryDiscriminator::FinalRootPresent,
                3,
            )
            .await,
            Err(AdpStorageError::InvalidTransition)
        ));
        let stored = repo
            .get_by_id(&installation.id)
            .await
            .expect("ordinary failure query")
            .expect("ordinary failure should exist");
        assert_eq!(stored.installation.recovery_discriminator, None);
        assert_eq!(stored.installation.updated_at, 2);
    }

    #[tokio::test]
    async fn malformed_persisted_recovery_discriminator_fails_read() {
        let db = test_db().await;
        let repo = InstallationsRepository::new(db.pool().clone());
        let installation = installation("malformed-recovery", InstallationState::Installing);
        repo.begin(&installation, &profiles()).await.expect("begin");
        let mut connection = db.pool().acquire().await.expect("test connection");
        sqlx::query("PRAGMA ignore_check_constraints = ON")
            .execute(&mut *connection)
            .await
            .expect("disable checks for hostile row injection");
        sqlx::query(
            "UPDATE installations SET state = 'failed', recovery_discriminator = 'unknown' \
             WHERE id = ?",
        )
        .bind(installation.id.as_str())
        .execute(&mut *connection)
        .await
        .expect("hostile recovery discriminator injection");
        sqlx::query("PRAGMA ignore_check_constraints = OFF")
            .execute(&mut *connection)
            .await
            .expect("restore checks");
        drop(connection);

        assert!(matches!(
            repo.get_by_id(&installation.id).await,
            Err(AdpStorageError::InvalidData(_))
        ));
    }

    #[tokio::test]
    async fn exact_manifest_lookup_uses_index_at_scale() {
        let db = test_db().await;
        let repo = InstallationsRepository::new(db.pool().clone());
        let installation = installation("manifest-scale", InstallationState::Installing);
        repo.begin(&installation, &profiles()).await.expect("begin");
        let entries = (0..5_000)
            .map(|index| InstallationManifestEntry {
                path: RelativePath::parse(&format!("files/{index:04}.bin"))
                    .expect("generated path should be valid"),
                entry_type: ManifestEntryType::File,
                file_digest: Some(digest()),
            })
            .collect::<Vec<_>>();
        repo.replace_manifest(&installation.id, &entries)
            .await
            .expect("large manifest");

        let plan = sqlx::query(
            r#"EXPLAIN QUERY PLAN SELECT path, entry_type, file_digest
               FROM installation_manifest_entries
               WHERE installation_id = ? AND path_key = ?"#,
        )
        .bind(installation.id.as_str())
        .bind("files/4999.bin")
        .fetch_all(db.pool())
        .await
        .expect("query plan");
        let details = plan
            .into_iter()
            .map(|row| row.try_get::<String, _>("detail").expect("plan detail"))
            .collect::<Vec<_>>()
            .join(" ");
        assert!(
            details.contains("USING INDEX")
                && !details.contains("SCAN installation_manifest_entries"),
            "unexpected query plan: {details}"
        );
        assert_eq!(
            repo.manifest_entry(
                &installation.id,
                &RelativePath::parse("FILES/4999.BIN").expect("lookup path"),
            )
            .await
            .expect("exact lookup")
            .expect("entry")
            .path
            .as_str(),
            "files/4999.bin"
        );
    }

    #[tokio::test]
    async fn manifest_rejects_unicode_case_fold_collisions() {
        let db = test_db().await;
        let repo = InstallationsRepository::new(db.pool().clone());
        let installation = installation("unicode-collision", InstallationState::Installing);
        repo.begin(&installation, &profiles()).await.expect("begin");
        let entries = ["Σ.exe", "ς.exe"].map(|path| InstallationManifestEntry {
            path: RelativePath::parse(path).expect("portable Unicode path"),
            entry_type: ManifestEntryType::File,
            file_digest: Some(digest()),
        });

        assert!(matches!(
            repo.replace_manifest(&installation.id, &entries).await,
            Err(AdpStorageError::InvalidData(_))
        ));
    }

    #[tokio::test]
    async fn retained_packages_survive_installation_deletion_and_check_u64_bounds() {
        let db = test_db().await;
        let repo = InstallationsRepository::new(db.pool().clone());
        let mut installation = installation("retained", InstallationState::Installing);
        let package = RetainedPackage {
            game_coordinate: installation.game_coordinate.clone(),
            package_id: RetainedPackageId::generate(),
            package_path: PathBuf::from("/immutable/packages/game.zip"),
            package_digest: digest(),
            package_format: PackageFormat::TarZstd,
            reinstall: reinstall_snapshot(&installation),
            size_bytes: i64::MAX as u64,
            created_at: 1,
            updated_at: 2,
        };
        repo.replace_retained_package(&package)
            .await
            .expect("upsert retained package");
        installation.retained_package_coordinate = Some(package.game_coordinate.clone());
        repo.begin(&installation, &profiles())
            .await
            .expect("begin referenced installation");
        repo.replace_manifest(&installation.id, &ready_manifest())
            .await
            .expect("record manifest");
        repo.transition(&installation.id, InstallationState::Ready, 2)
            .await
            .expect("ready");
        repo.transition(&installation.id, InstallationState::Uninstalling, 3)
            .await
            .expect("uninstalling");
        repo.delete(&installation.id)
            .await
            .expect("delete installation");

        assert_eq!(
            repo.get_retained_package(&package.game_coordinate)
                .await
                .expect("get retained package"),
            Some(package.clone())
        );
        assert_eq!(
            repo.list_retained_packages().await.expect("list retained"),
            vec![package.clone()]
        );

        let mut oversized = package.clone();
        oversized.package_id = RetainedPackageId::generate();
        oversized.size_bytes = (i64::MAX as u64) + 1;
        assert!(matches!(
            repo.replace_retained_package(&oversized).await,
            Err(AdpStorageError::OutOfRange(_))
        ));
        repo.delete_retained_package(&package.game_coordinate)
            .await
            .expect("delete retained package");
    }

    #[tokio::test]
    async fn installing_row_atomically_attaches_new_matching_retained_package() {
        let db = test_db().await;
        let repo = InstallationsRepository::new(db.pool().clone());
        let installation = installation("attach-retained", InstallationState::Installing);
        repo.begin(&installation, &profiles())
            .await
            .expect("begin install");
        let package = RetainedPackage {
            game_coordinate: installation.game_coordinate.clone(),
            package_id: RetainedPackageId::generate(),
            package_path: PathBuf::from("/immutable/packages/attached.tar.zst"),
            package_digest: installation.package_digest.clone(),
            package_format: installation.package_format.clone(),
            reinstall: reinstall_snapshot(&installation),
            size_bytes: 42,
            created_at: 1,
            updated_at: 2,
        };

        repo.retain_and_attach_installing(&installation.id, &package)
            .await
            .expect("matching retained package should attach");

        let stored = repo
            .get_by_id(&installation.id)
            .await
            .expect("installation query")
            .expect("installation exists");
        assert_eq!(
            stored.installation.retained_package_coordinate,
            Some(installation.game_coordinate)
        );
        assert_eq!(
            repo.get_retained_package(&package.game_coordinate)
                .await
                .expect("retained query"),
            Some(package)
        );
    }

    #[tokio::test]
    async fn existing_retained_attachment_validates_immutable_row_identity() {
        let db = test_db().await;
        let repo = InstallationsRepository::new(db.pool().clone());
        let installation = installation("attach-existing-retained", InstallationState::Installing);
        let package = RetainedPackage {
            game_coordinate: installation.game_coordinate.clone(),
            package_id: RetainedPackageId::generate(),
            package_path: PathBuf::from("/immutable/packages/existing.tar.zst"),
            package_digest: installation.package_digest.clone(),
            package_format: installation.package_format.clone(),
            reinstall: reinstall_snapshot(&installation),
            size_bytes: 42,
            created_at: 1,
            updated_at: 1,
        };
        repo.replace_retained_package(&package)
            .await
            .expect("persist retained package");
        repo.begin(&installation, &profiles())
            .await
            .expect("begin install");
        let mut altered = package.clone();
        altered.package_path = PathBuf::from("/immutable/packages/different.tar.zst");

        assert!(matches!(
            repo.attach_existing_retained_installing(&installation.id, &altered, 2)
                .await,
            Err(AdpStorageError::InvalidData(_))
        ));
        assert_eq!(
            repo.get_by_id(&installation.id)
                .await
                .expect("installation query")
                .expect("installation row")
                .installation
                .retained_package_coordinate,
            None
        );

        repo.attach_existing_retained_installing(&installation.id, &package, 2)
            .await
            .expect("attach exact retained row");
        assert_eq!(
            repo.get_by_id(&installation.id)
                .await
                .expect("installation query")
                .expect("installation row")
                .installation
                .retained_package_coordinate,
            Some(package.game_coordinate)
        );
    }

    #[tokio::test]
    async fn existing_legacy_retained_attachment_pins_resolved_publisher_default() {
        let db = test_db().await;
        let repo = InstallationsRepository::new(db.pool().clone());
        let installation = installation("attach-legacy-retained", InstallationState::Installing);
        let expected_default = installation.publisher_default_profile_id.clone();
        assert!(expected_default.is_some());
        let mut package = RetainedPackage {
            game_coordinate: installation.game_coordinate.clone(),
            package_id: RetainedPackageId::generate(),
            package_path: PathBuf::from("/immutable/packages/legacy.tar.zst"),
            package_digest: installation.package_digest.clone(),
            package_format: installation.package_format.clone(),
            reinstall: reinstall_snapshot(&installation),
            size_bytes: 42,
            created_at: 1,
            updated_at: 1,
        };
        package.reinstall.publisher_default_profile_id = None;
        repo.replace_retained_package(&package)
            .await
            .expect("persist legacy retained package");
        repo.begin(&installation, &profiles())
            .await
            .expect("begin reinstall");

        repo.attach_existing_retained_installing(&installation.id, &package, 2)
            .await
            .expect("legacy retained package should be upgraded and attached");

        let upgraded = repo
            .get_retained_package(&package.game_coordinate)
            .await
            .expect("retained query")
            .expect("retained package");
        assert_eq!(
            upgraded.reinstall.publisher_default_profile_id,
            expected_default
        );
        assert_eq!(upgraded.updated_at, 2);
    }

    #[tokio::test]
    async fn attached_retained_snapshot_must_still_match_after_restart_read() {
        let db = test_db().await;
        let repo = InstallationsRepository::new(db.pool().clone());
        let mut installation =
            installation("attached-snapshot-mismatch", InstallationState::Installing);
        let package = RetainedPackage {
            game_coordinate: installation.game_coordinate.clone(),
            package_id: RetainedPackageId::generate(),
            package_path: PathBuf::from("/immutable/packages/attached-snapshot-mismatch.tar.zst"),
            package_digest: installation.package_digest.clone(),
            package_format: installation.package_format.clone(),
            reinstall: reinstall_snapshot(&installation),
            size_bytes: 42,
            created_at: 1,
            updated_at: 1,
        };
        repo.replace_retained_package(&package)
            .await
            .expect("persist retained package");
        installation.retained_package_coordinate = Some(package.game_coordinate.clone());
        repo.begin(&installation, &profiles())
            .await
            .expect("attach matching retained package");

        sqlx::query(
            "UPDATE retained_packages SET resolved_arch = 'aarch64' WHERE game_coordinate = ?",
        )
        .bind(&package.game_coordinate)
        .execute(db.pool())
        .await
        .expect("inject independently valid but mismatched target");

        assert!(matches!(
            repo.get_by_id(&installation.id).await,
            Err(AdpStorageError::InvalidData(_))
        ));
    }

    #[tokio::test]
    async fn retained_reinstall_snapshot_rejects_malformed_event_target_and_profiles_on_read() {
        for (suffix, column, value) in [
            ("event", "raw_event_json", "{}"),
            ("target", "resolved_os", "plan9"),
            (
                "profiles",
                "profiles_json",
                r#"[{"id":"play","display_name":"Play","executable":"../escape","working_dir":".","arguments":[],"launch_type":"Game"}]"#,
            ),
        ] {
            let db = test_db().await;
            let repo = InstallationsRepository::new(db.pool().clone());
            let installation = installation(
                &format!("retained-hostile-{suffix}"),
                InstallationState::Installing,
            );
            let package = RetainedPackage {
                game_coordinate: installation.game_coordinate.clone(),
                package_id: RetainedPackageId::generate(),
                package_path: PathBuf::from(format!(
                    "/immutable/packages/retained-hostile-{suffix}.tar.zst"
                )),
                package_digest: installation.package_digest.clone(),
                package_format: installation.package_format.clone(),
                reinstall: reinstall_snapshot(&installation),
                size_bytes: 42,
                created_at: 1,
                updated_at: 1,
            };
            repo.replace_retained_package(&package)
                .await
                .expect("persist valid retained snapshot");
            sqlx::query(&format!(
                "UPDATE retained_packages SET {column} = ? WHERE game_coordinate = ?"
            ))
            .bind(value)
            .bind(&package.game_coordinate)
            .execute(db.pool())
            .await
            .expect("inject hostile retained snapshot");

            assert!(matches!(
                repo.get_retained_package(&package.game_coordinate).await,
                Err(AdpStorageError::InvalidData(_)) | Err(AdpStorageError::Json(_))
            ));
        }
    }

    #[tokio::test]
    async fn retained_read_does_not_reresolve_pinned_profiles() {
        let db = test_db().await;
        let repo = InstallationsRepository::new(db.pool().clone());
        let installation = installation("retained-pinned-read", InstallationState::Installing);
        let package = RetainedPackage {
            game_coordinate: installation.game_coordinate.clone(),
            package_id: RetainedPackageId::generate(),
            package_path: PathBuf::from("/immutable/packages/retained-pinned-read.tar.zst"),
            package_digest: installation.package_digest.clone(),
            package_format: installation.package_format.clone(),
            reinstall: reinstall_snapshot(&installation),
            size_bytes: 42,
            created_at: 1,
            updated_at: 1,
        };
        repo.replace_retained_package(&package)
            .await
            .expect("persist valid retained snapshot");
        sqlx::query("UPDATE retained_packages SET profiles_json = '[]' WHERE game_coordinate = ?")
            .bind(&package.game_coordinate)
            .execute(db.pool())
            .await
            .expect("replace pinned profiles with a typed value");

        let stored = repo
            .get_retained_package(&package.game_coordinate)
            .await
            .expect("typed pinned retained snapshot should remain readable")
            .expect("retained package should exist");
        assert!(stored.reinstall.launch_profiles.is_empty());
    }

    #[tokio::test]
    async fn retained_reinstall_snapshot_round_trips_after_database_restart() {
        let path = std::env::temp_dir().join(format!(
            "arcadestr-retained-restart-{}.db",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system clock")
                .as_nanos()
        ));
        let installation = installation("retained-restart", InstallationState::Installing);
        let package = RetainedPackage {
            game_coordinate: installation.game_coordinate.clone(),
            package_id: RetainedPackageId::generate(),
            package_path: PathBuf::from("/immutable/packages/restart.tar.zst"),
            package_digest: installation.package_digest.clone(),
            package_format: installation.package_format.clone(),
            reinstall: reinstall_snapshot(&installation),
            size_bytes: 42,
            created_at: 1,
            updated_at: 1,
        };
        {
            let db = Database::new(&path).await.expect("initial database");
            InstallationsRepository::new(db.pool().clone())
                .replace_retained_package(&package)
                .await
                .expect("persist retained snapshot");
        }
        let reopened = Database::new(&path).await.expect("reopened database");
        assert_eq!(
            InstallationsRepository::new(reopened.pool().clone())
                .get_retained_package(&package.game_coordinate)
                .await
                .expect("restarted retained query"),
            Some(package)
        );
    }

    #[tokio::test]
    async fn library_games_are_idempotent_and_account_scoped() {
        let db = test_db().await;
        let repo = LibraryGamesRepository::new(db.pool().clone());
        let entry = LibraryGame {
            buyer_pubkey: "buyer-1".to_string(),
            game_coordinate: "30402:dev:game".to_string(),
            added_at: 123,
        };

        repo.add(&entry).await.expect("insert should succeed");
        repo.add(&entry)
            .await
            .expect("repeat insert should succeed");

        assert!(repo
            .contains("buyer-1", "30402:dev:game")
            .await
            .expect("lookup should succeed"));
        assert!(!repo
            .contains("buyer-2", "30402:dev:game")
            .await
            .expect("lookup should succeed"));
        assert_eq!(
            repo.list("buyer-1").await.expect("list should succeed"),
            vec![entry]
        );
        assert!(repo
            .list("buyer-2")
            .await
            .expect("list should succeed")
            .is_empty());
    }
}
