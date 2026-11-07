//! Update transactions on the same installation repository and manifest verifier.
use super::*;

#[derive(Debug, Clone)]
pub struct InstallationUpdate {
    pub id: String,
    pub old: InstallationSnapshot,
    pub old_retained: Option<RetainedPackage>,
    pub candidate: InstallationRecord,
    pub candidate_package_path: PathBuf,
    pub committed: bool,
}

impl InstallationsRepository {
    pub async fn pending_updates(&self) -> Result<Vec<InstallationUpdate>, AdpStorageError> {
        let rows = sqlx::query("SELECT * FROM installation_updates")
            .fetch_all(&self.pool)
            .await?;
        rows.into_iter()
            .map(|row| {
                Ok(InstallationUpdate {
                    id: row.try_get("id")?,
                    old: serde_json::from_str(&row.try_get::<String, _>("old_snapshot_json")?)?,
                    old_retained: row
                        .try_get::<Option<String>, _>("old_retained_json")?
                        .map(|json| serde_json::from_str(&json))
                        .transpose()?,
                    candidate: serde_json::from_str(
                        &row.try_get::<String, _>("candidate_record_json")?,
                    )?,
                    candidate_package_path: PathBuf::from(
                        row.try_get::<String, _>("candidate_package_path")?,
                    ),
                    committed: row.try_get::<String, _>("state")? == "committed",
                })
            })
            .collect()
    }

    /// Persist intent before any candidate file is created. Caller holds the
    /// coordinate lifecycle lock; the old ready snapshot remains launchable.
    pub async fn begin_update(
        &self,
        old: &InstallationSnapshot,
        candidate: &InstallationRecord,
        profiles: &[InstallationProfile],
        package_path: &Path,
    ) -> Result<String, AdpStorageError> {
        if old.installation.state != InstallationState::Ready
            || candidate.state != InstallationState::Installing
            || candidate.id != old.installation.id
            || candidate.game_coordinate != old.installation.game_coordinate
            || candidate.installation_root == old.installation.installation_root
        {
            return Err(AdpStorageError::InvalidTransition);
        }
        validate_resolved_snapshot(candidate, profiles)?;
        let previous_event: Event = serde_json::from_str(&old.installation.raw_event_json)?;
        let candidate_event: Event = serde_json::from_str(&candidate.raw_event_json)?;
        if !crate::replaceable_event::is_replaceable_event_newer(
            candidate_event.created_at.as_secs(),
            Some(&candidate_event.id.to_hex()),
            previous_event.created_at.as_secs(),
            Some(&previous_event.id.to_hex()),
        ) {
            return Err(AdpStorageError::InvalidData(
                "update listing does not supersede the installed signed revision".into(),
            ));
        }
        validate_absolute_path(&candidate.installation_root, "update root")?;
        validate_absolute_path(package_path, "update package")?;
        let mut tx = self.pool.begin().await?;
        let row = sqlx::query(&format!("{INSTALLATION_SELECT} WHERE id = ?"))
            .bind(old.installation.id.as_str())
            .fetch_optional(&mut *tx)
            .await?
            .ok_or(AdpStorageError::NotFound)?;
        if self.snapshot_from_row(row, &mut tx).await? != *old {
            return Err(AdpStorageError::InvalidTransition);
        }
        let retained = sqlx::query(&format!(
            "{RETAINED_PACKAGE_SELECT} WHERE game_coordinate = ?"
        ))
        .bind(&candidate.game_coordinate)
        .fetch_optional(&mut *tx)
        .await?
        .map(retained_package_from_row)
        .transpose()?;
        let id = Uuid::new_v4().simple().to_string();
        sqlx::query("INSERT INTO installation_updates (id,game_coordinate,old_snapshot_json,old_retained_json,candidate_record_json,candidate_package_path,state) VALUES (?,?,?,?,?,?,'preparing')")
            .bind(&id).bind(&candidate.game_coordinate).bind(serde_json::to_string(old)?)
            .bind(retained.as_ref().map(serde_json::to_string).transpose()?)
            .bind(serde_json::to_string(candidate)?).bind(path_text(package_path,"update package")?)
            .execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(id)
    }

    /// Atomically replace the snapshot, manifest and retained-package reference.
    /// A failed transaction leaves every old row unchanged.
    pub async fn commit_update(
        &self,
        id: &str,
        package: &RetainedPackage,
        profiles: &[InstallationProfile],
        entries: &[InstallationManifestEntry],
        updated_at: i64,
    ) -> Result<(), AdpStorageError> {
        validate_retained_package(package)?;
        let mut tx = self.pool.begin().await?;
        let journal =
            sqlx::query("SELECT * FROM installation_updates WHERE id = ? AND state = 'preparing'")
                .bind(id)
                .fetch_optional(&mut *tx)
                .await?
                .ok_or(AdpStorageError::InvalidTransition)?;
        let old: InstallationSnapshot =
            serde_json::from_str(&journal.try_get::<String, _>("old_snapshot_json")?)?;
        let candidate: InstallationRecord =
            serde_json::from_str(&journal.try_get::<String, _>("candidate_record_json")?)?;
        let row = sqlx::query(&format!("{INSTALLATION_SELECT} WHERE id = ?"))
            .bind(old.installation.id.as_str())
            .fetch_optional(&mut *tx)
            .await?
            .ok_or(AdpStorageError::NotFound)?;
        if self.snapshot_from_row(row, &mut tx).await? != old {
            return Err(AdpStorageError::InvalidTransition);
        }
        if old
            .profiles
            .iter()
            .any(|profile| profile.availability == ProfileAvailability::Available)
            && !profiles
                .iter()
                .any(|profile| profile.availability == ProfileAvailability::Available)
        {
            return Err(AdpStorageError::InvalidData(
                "replacement has no available launch profile; keeping the working installation"
                    .into(),
            ));
        }
        if package.game_coordinate != candidate.game_coordinate
            || package.package_digest != candidate.package_digest
            || package.package_format != candidate.package_format
            || package.reinstall.raw_event_json != candidate.raw_event_json
            || package.reinstall.resolved_target != candidate.resolved_target
            || package.reinstall.publisher_default_profile_id
                != candidate.publisher_default_profile_id
            || package.reinstall.launch_profiles
                != profiles
                    .iter()
                    .map(|p| p.profile.clone())
                    .collect::<Vec<_>>()
            || path_text(&package.package_path, "update package")?
                != journal.try_get::<String, _>("candidate_package_path")?
        {
            return Err(AdpStorageError::InvalidTransition);
        }
        // Detach only inside this transaction; no observer sees an unready version.
        sqlx::query("UPDATE installations SET installation_root=?, package_digest=?, package_format=?,resolved_os=?,resolved_arch=?,raw_event_json=?,publisher_default_profile_id=?,state='installing',retained_package_coordinate=NULL WHERE id=?")
            .bind(path_text(&candidate.installation_root,"update root")?).bind(candidate.package_digest.as_str())
            .bind(package_format_text(&candidate.package_format)).bind(&candidate.resolved_target.os).bind(&candidate.resolved_target.arch)
            .bind(&candidate.raw_event_json).bind(candidate.publisher_default_profile_id.as_ref().map(ProfileId::as_str))
            .bind(candidate.id.as_str()).execute(&mut *tx).await?;
        sqlx::query("DELETE FROM retained_packages WHERE game_coordinate=?")
            .bind(&candidate.game_coordinate)
            .execute(&mut *tx)
            .await?;
        sqlx::query("INSERT INTO retained_packages (game_coordinate,package_id,package_path,package_digest,package_format,raw_event_json,resolved_os,resolved_arch,profiles_json,publisher_default_profile_id,size_bytes,created_at,updated_at) VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?)")
            .bind(&package.game_coordinate).bind(package.package_id.as_str()).bind(path_text(&package.package_path,"update package")?)
            .bind(package.package_digest.as_str()).bind(package_format_text(&package.package_format)).bind(&package.reinstall.raw_event_json)
            .bind(&package.reinstall.resolved_target.os).bind(&package.reinstall.resolved_target.arch)
            .bind(serde_json::to_string(&package.reinstall.launch_profiles)?)
            .bind(package.reinstall.publisher_default_profile_id.as_ref().map(ProfileId::as_str))
            .bind(i64::try_from(package.size_bytes).map_err(|_|AdpStorageError::OutOfRange("package size"))?)
            .bind(package.created_at).bind(updated_at).execute(&mut *tx).await?;
        sqlx::query("UPDATE installations SET retained_package_coordinate=? WHERE id=?")
            .bind(&candidate.game_coordinate)
            .bind(candidate.id.as_str())
            .execute(&mut *tx)
            .await?;
        self.commit_extraction_in(
            &mut tx,
            &candidate.id,
            &candidate.installation_root,
            &candidate.package_digest,
            &candidate.package_format,
            profiles,
            entries,
            updated_at,
        )
        .await?;
        sqlx::query("UPDATE installation_updates SET state='committed' WHERE id=?")
            .bind(id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(())
    }

    /// Called only after guarded cleanup, under the same lifecycle lock.
    pub async fn finish_update(&self, id: &str) -> Result<(), AdpStorageError> {
        sqlx::query("DELETE FROM installation_updates WHERE id=?")
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}
