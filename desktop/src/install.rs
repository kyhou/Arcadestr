use std::fs::File;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use tokio::io::AsyncWriteExt;

use arcadestr_core::adp_storage::{
    AdpStorageError, InstallationId, InstallationRecord, InstallationRecoveryDiscriminator,
    InstallationState, InstallationsRepository,
};
use arcadestr_core::distribution::{LaunchProfile, PackageFormat, Sha256Digest};
use arcadestr_core::file_hash::sha256_file;
#[cfg(test)]
use arcadestr_core::package_extraction::extract_verified_package;
use arcadestr_core::package_extraction::{
    extract_verified_package_into_capability, ExtractionError, ExtractionOutcome, VerifiedPackage,
};
use thiserror::Error;
use uuid::Uuid;

const PLACEMENT_ATTEMPTS: usize = 16;

#[derive(Debug)]
pub(crate) struct PlacedInstallation {
    root: PathBuf,
    extraction: ExtractionOutcome,
    package_digest: Sha256Digest,
    package_format: PackageFormat,
    _placed_root: cap_std::fs::Dir,
    _operation_lock: OperationLock,
}

#[derive(Debug)]
struct PlacedTree {
    root: PathBuf,
    extraction: ExtractionOutcome,
    operation_lock: OperationLock,
}

/// Owned lease serializing installation operations for one final root.
#[derive(Debug, Clone)]
pub(crate) struct OperationLock {
    final_root: PathBuf,
    _file: Arc<File>,
    managed_directory: Arc<cap_std::fs::Dir>,
}

#[derive(Debug, Error)]
pub(crate) enum InstallationPlacementError {
    #[error("installation root must be an absolute non-root path: {0}")]
    InvalidFinalRoot(PathBuf),
    #[error("installation root already exists: {0}")]
    FinalRootExists(PathBuf),
    #[error("another installation operation holds the lock for {0}")]
    Busy(PathBuf),
    #[error("operation lock for {locked_root} cannot place installation at {requested_root}")]
    OperationLockRootMismatch {
        locked_root: PathBuf,
        requested_root: PathBuf,
    },
    #[error("package extraction failed: {0}")]
    Extraction(#[from] ExtractionError),
    #[error("filesystem operation failed while {operation} {path}: {source}")]
    Io {
        operation: &'static str,
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("operation failed ({operation}) and staging cleanup failed: {cleanup}")]
    Cleanup {
        operation: String,
        operation_insufficient_storage: bool,
        #[source]
        cleanup: std::io::Error,
    },
}

impl InstallationPlacementError {
    #[must_use]
    pub(crate) fn is_archive_policy(&self) -> bool {
        use arcadestr_core::archive_inspection::{DetectInspectError, InspectionError};

        match self {
            Self::Extraction(ExtractionError::Policy(error))
            | Self::Extraction(ExtractionError::Inspection(DetectInspectError::Inspection(
                error,
            ))) => matches!(
                error,
                InspectionError::CompressionRatioExceeded { .. }
                    | InspectionError::ZeroMeasuredInput { .. }
                    | InspectionError::EntryLimitExceeded { .. }
                    | InspectionError::EntryCountOverflow
                    | InspectionError::ZipCentralDirectoryLimitExceeded { .. }
                    | InspectionError::TarMetadataLimitExceeded { .. }
                    | InspectionError::MeasuredInputOverflow
                    | InspectionError::MeasuredOutputOverflow
            ),
            Self::Extraction(ExtractionError::DeclaredRegularFileSizeOverflow) => true,
            _ => false,
        }
    }

    #[must_use]
    pub(crate) fn is_insufficient_storage(&self) -> bool {
        match self {
            Self::Io { source, .. } => is_enospc(source),
            Self::Extraction(error) => error.is_insufficient_storage(),
            Self::Cleanup {
                operation_insufficient_storage,
                cleanup,
                ..
            } => *operation_insufficient_storage || is_enospc(cleanup),
            _ => false,
        }
    }
}

fn is_enospc(error: &std::io::Error) -> bool {
    #[cfg(unix)]
    if error.raw_os_error() == Some(28) {
        return true;
    }
    #[cfg(windows)]
    if error.raw_os_error() == Some(112) {
        return true;
    }
    false
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub(crate) struct RecoveryReport {
    pub examined: usize,
    pub marked_failed: usize,
    pub busy: usize,
    pub deferred_errors: usize,
    pub changed: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RecoveryMarkOutcome {
    MarkedFailed,
    Changed,
}

#[derive(Debug)]
pub(crate) struct InstallationCommitError {
    pub storage: AdpStorageError,
    pub cleanup_failure: Option<String>,
}

impl InstallationCommitError {
    #[must_use]
    pub fn recovery_required(&self) -> bool {
        true
    }
}

impl std::fmt::Display for InstallationCommitError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "installation commit failed and requires recovery: {}",
            self.storage
        )?;
        if let Some(cleanup) = &self.cleanup_failure {
            write!(formatter, "; placed-tree cleanup also failed: {cleanup}")?;
        }
        Ok(())
    }
}

impl std::error::Error for InstallationCommitError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.storage)
    }
}

pub(crate) fn extract_and_place_verified_package(
    package: &VerifiedPackage,
    final_root: &Path,
    launch_profiles: &[LaunchProfile],
    operation_lock: OperationLock,
) -> Result<PlacedInstallation, InstallationPlacementError> {
    operation_lock.require_root(final_root)?;
    let (parent, final_name) = validated_final_root(final_root)?;
    let managed_directory = operation_lock.managed_directory.clone();
    ensure_final_root_absent_in(&managed_directory, final_name, final_root)?;

    let staging_root = reserve_staging_path(&managed_directory, parent, final_name)?;
    let staging_name = staging_root
        .file_name()
        .ok_or_else(|| InstallationPlacementError::InvalidFinalRoot(staging_root.clone()))?;
    managed_directory
        .create_dir(staging_name)
        .map_err(|source| {
            placement_io("creating installation staging root", &staging_root, source)
        })?;
    let staging_directory = match open_managed_child_directory_no_follow(
        &managed_directory,
        staging_name,
        &staging_root,
    ) {
        Ok(directory) => directory,
        Err(error) => {
            cleanup_staging_after_error(
                &managed_directory,
                &staging_root,
                error.to_string(),
                error.is_insufficient_storage(),
            )?;
            return Err(error);
        }
    };

    let capability_extraction = match extract_verified_package_into_capability(
        package,
        staging_directory,
        &staging_root,
        launch_profiles,
    ) {
        Ok(extraction) => extraction,
        Err(error) => {
            cleanup_staging_after_error(
                &managed_directory,
                &staging_root,
                error.to_string(),
                error.is_insufficient_storage(),
            )?;
            return Err(InstallationPlacementError::Extraction(error));
        }
    };

    if let Err(error) = ensure_final_root_absent_in(&managed_directory, final_name, final_root) {
        let insufficient_storage = error.is_insufficient_storage();
        drop(capability_extraction);
        cleanup_staging_after_error(
            &managed_directory,
            &staging_root,
            error.to_string(),
            insufficient_storage,
        )?;
        return Err(error);
    }
    let expected_root_identity = match capability_extraction
        .root_file()
        .and_then(|file| file_identity(&file))
    {
        Ok(identity) => identity,
        Err(source) => {
            drop(capability_extraction);
            cleanup_staging_after_error(
                &managed_directory,
                &staging_root,
                source.to_string(),
                is_enospc(&source),
            )?;
            return Err(placement_io(
                "identifying completed staging root",
                &staging_root,
                source,
            ));
        }
    };
    let extraction = capability_extraction.outcome.clone();
    // Windows directory handles intentionally deny delete sharing. Release
    // them for rename, then prove the renamed object has the same file ID.
    drop(capability_extraction);
    if let Err(source) =
        managed_directory.rename(staging_name, &managed_directory, Path::new(final_name))
    {
        let operation = format!(
            "renaming completed staging tree {} to {}: {source}",
            staging_root.display(),
            final_root.display()
        );
        let insufficient_storage = is_enospc(&source);
        cleanup_staging_after_error(
            &managed_directory,
            &staging_root,
            operation,
            insufficient_storage,
        )?;
        return Err(InstallationPlacementError::Io {
            operation: "placing installation at",
            path: final_root.to_owned(),
            source,
        });
    }
    let placed_root = match open_managed_child_directory_no_follow(
        &managed_directory,
        std::ffi::OsStr::new(final_name),
        final_root,
    ) {
        Ok(directory) => directory,
        Err(error) => return Err(error),
    };
    let placed_identity = placed_root
        .try_clone()
        .map(cap_std::fs::Dir::into_std_file)
        .and_then(|file| file_identity(&file))
        .map_err(|source| {
            placement_io("identifying placed installation root", final_root, source)
        })?;
    if expected_root_identity != placed_identity {
        drop(placed_root);
        return Err(placement_io(
            "verifying placed installation root identity",
            final_root,
            std::io::Error::other("staging root was replaced during promotion"),
        ));
    }

    Ok(PlacedInstallation {
        root: final_root.to_owned(),
        extraction,
        package_digest: package.digest().clone(),
        package_format: package.format().clone(),
        _placed_root: placed_root,
        _operation_lock: operation_lock,
    })
}

/// Persists A4.2 output into an existing installing row. A4.4 owns recovery
/// for every returned error, whether placed-tree cleanup succeeds or fails.
pub(crate) async fn commit_placed_installation(
    repository: &InstallationsRepository,
    installation_id: &InstallationId,
    placed: PlacedInstallation,
    updated_at: i64,
) -> Result<PlacedInstallation, InstallationCommitError> {
    let commit = repository
        .commit_extraction(
            installation_id,
            &placed.root,
            &placed.package_digest,
            &placed.package_format,
            &placed.extraction.profiles,
            &placed.extraction.manifest,
            updated_at,
        )
        .await;
    let Err(storage) = commit else {
        return Ok(placed);
    };

    match repository.get_by_id(installation_id).await {
        Ok(Some(snapshot))
            if snapshot.installation.state
                == arcadestr_core::adp_storage::InstallationState::Ready
                && snapshot.installation.installation_root == placed.root
                && snapshot.installation.package_digest == placed.package_digest
                && snapshot.installation.package_format == placed.package_format
                && snapshot.profiles == placed.extraction.profiles
                && manifests_match(&snapshot.manifest, &placed.extraction.manifest) =>
        {
            return Ok(placed);
        }
        Ok(Some(snapshot))
            if snapshot.installation.state
                == arcadestr_core::adp_storage::InstallationState::Installing => {}
        Ok(None) => {}
        Ok(Some(_)) | Err(_) => {
            return Err(InstallationCommitError {
                storage,
                cleanup_failure: Some(
                    "commit outcome could not be proven uncommitted; placed tree preserved"
                        .to_string(),
                ),
            });
        }
    }

    Err(InstallationCommitError {
        storage,
        cleanup_failure: Some(
            "placed tree preserved for recovery because dropping its identity handle before recursive deletion would permit replacement races"
                .to_string(),
        ),
    })
}

/// Reuse extraction output and the repository's manifest validation for updates.
pub(crate) async fn commit_placed_update(
    repository: &InstallationsRepository,
    id: &str,
    package: &arcadestr_core::adp_storage::RetainedPackage,
    placed: PlacedInstallation,
    updated_at: i64,
) -> Result<(), String> {
    repository
        .commit_update(
            id,
            package,
            &placed.extraction.profiles,
            &placed.extraction.manifest,
            updated_at,
        )
        .await
        .map_err(|error| error.to_string())
}

/// Before commit: discard only the candidate. After commit: discard only the
/// retired version. Active snapshot and retained-package reference are checked
/// before deletion. Failed cleanup retains intent for the next startup/retry.
pub(crate) async fn recover_updates(
    repository: &InstallationsRepository,
    app_data: &Path,
    _updated_at: i64,
    locked_coordinate: Option<&str>,
) -> Result<(), String> {
    let mut first_error = None;
    for update in repository
        .pending_updates()
        .await
        .map_err(|error| error.to_string())?
    {
        let coordinate = &update.candidate.game_coordinate;
        if locked_coordinate.is_some_and(|locked| locked != coordinate) {
            continue;
        }
        let _coordinate_guard = if locked_coordinate.is_none() {
            match super::acquire_coordinate_lifecycle_lock(app_data, coordinate).await {
                Ok(lock) => Some(lock),
                Err(_) => continue,
            }
        } else {
            None
        };
        let result = async {
            let current = repository
                .get_by_coordinate(coordinate)
                .await
                .map_err(|error| error.to_string())?
                .ok_or_else(|| "update recovery has no active installation".to_string())?;
            let retained = repository
                .get_retained_package(coordinate)
                .await
                .map_err(|error| error.to_string())?;
            let (root, package) = if update.committed {
                if current.installation.id != update.candidate.id
                    || current.installation.state != InstallationState::Ready
                    || current.installation.installation_root != update.candidate.installation_root
                    || current.installation.package_digest != update.candidate.package_digest
                    || retained
                        .as_ref()
                        .is_none_or(|p| p.package_path != update.candidate_package_path)
                {
                    return Err("committed update recovery snapshot mismatch".into());
                }
                (
                    update.old.installation.installation_root.clone(),
                    update.old_retained.as_ref().map(|p| p.package_path.clone()),
                )
            } else {
                if current != update.old || retained != update.old_retained {
                    return Err("update rollback snapshot mismatch".into());
                }
                (
                    update.candidate.installation_root.clone(),
                    Some(update.candidate_package_path.clone()),
                )
            };
            if root == current.installation.installation_root
                || package
                    .as_ref()
                    .is_some_and(|path| retained.as_ref().is_some_and(|p| &p.package_path == path))
            {
                return Err("update cleanup would remove active files".into());
            }
            require_managed_direct_child(&app_data.join("installations"), &root)
                .map_err(|error| error.to_string())?;
            if let Some(path) = &package {
                require_managed_direct_child(&app_data.join("retained-packages"), path)
                    .map_err(|error| error.to_string())?;
            }
            let cleanup = tokio::task::spawn_blocking(move || -> Result<(), String> {
                // The stable root lock also excludes legacy recovery/uninstall.
                let lock = OperationLock::acquire_existing_for_recovery(&root)
                    .map_err(|error| error.to_string())?;
                cleanup_owned_staging_roots(&root, &lock).map_err(|error| error.to_string())?;
                lock.remove_root_no_follow()
                    .map_err(|error| error.to_string())?;
                if let Some(path) = package {
                    let parent = path
                        .parent()
                        .ok_or_else(|| "missing package parent".to_string())?;
                    remove_managed_direct_child_no_follow(parent, &path)
                        .map_err(|error| error.to_string())?;
                }
                Ok(())
            })
            .await
            .map_err(|error| error.to_string())?;
            cleanup?;
            repository
                .finish_update(&update.id)
                .await
                .map_err(|error| error.to_string())?;
            Ok::<(), String>(())
        }
        .await;
        if let Err(error) = result {
            tracing::warn!(%error,"update recovery deferred; other games remain independent");
            first_error.get_or_insert(error);
        }
    }
    first_error.map_or(Ok(()), Err)
}

fn cleanup_owned_staging_roots(root: &Path, lock: &OperationLock) -> Result<(), std::io::Error> {
    lock.require_root(root).map_err(std::io::Error::other)?;
    let name = root
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| std::io::Error::other("invalid update root"))?;
    let prefix = format!(".{name}.");
    for entry in lock.managed_directory.entries()? {
        let entry = entry?;
        let filename = entry.file_name();
        if let Some(suffix) = filename
            .to_str()
            .and_then(|name| name.strip_prefix(&prefix))
            .and_then(|name| name.strip_suffix(".tmp"))
        {
            if Uuid::parse_str(suffix).is_ok() {
                remove_managed_child_no_follow(&lock.managed_directory, &filename)?;
            }
        }
    }
    Ok(())
}

fn manifests_match(
    stored: &[arcadestr_core::adp_storage::InstallationManifestEntry],
    extracted: &[arcadestr_core::adp_storage::InstallationManifestEntry],
) -> bool {
    if stored.len() != extracted.len() {
        return false;
    }
    let stored = stored
        .iter()
        .map(|entry| (entry.path.as_str(), entry))
        .collect::<std::collections::HashMap<_, _>>();
    extracted.iter().all(|entry| {
        stored
            .get(entry.path.as_str())
            .is_some_and(|stored| *stored == entry)
    })
}

#[cfg(test)]
fn place_with_extractor(
    final_root: &Path,
    operation_lock: OperationLock,
    extractor: impl FnOnce(&Path) -> Result<ExtractionOutcome, ExtractionError>,
) -> Result<PlacedTree, InstallationPlacementError> {
    operation_lock.require_root(final_root)?;
    let (parent, final_name) = validated_final_root(final_root)?;
    let managed_directory = operation_lock.managed_directory.clone();
    ensure_final_root_absent_in(&managed_directory, final_name, final_root)?;

    let staging_root = reserve_staging_path(&managed_directory, parent, final_name)?;
    let staging_access_path = test_managed_child_access_path(&managed_directory, &staging_root)?;
    let extraction = match extractor(&staging_access_path) {
        Ok(extraction) => extraction,
        Err(error) => {
            cleanup_staging_after_error(
                &managed_directory,
                &staging_root,
                error.to_string(),
                error.is_insufficient_storage(),
            )?;
            return Err(InstallationPlacementError::Extraction(error));
        }
    };

    if let Err(error) = ensure_final_root_absent_in(&managed_directory, final_name, final_root) {
        let insufficient_storage = error.is_insufficient_storage();
        cleanup_staging_after_error(
            &managed_directory,
            &staging_root,
            error.to_string(),
            insufficient_storage,
        )?;
        return Err(error);
    }

    let staging_name = staging_root
        .file_name()
        .ok_or_else(|| InstallationPlacementError::InvalidFinalRoot(staging_root.clone()))?;
    if let Err(source) =
        managed_directory.rename(staging_name, &managed_directory, Path::new(final_name))
    {
        let operation = format!(
            "renaming completed staging tree {} to {}: {source}",
            staging_root.display(),
            final_root.display()
        );
        let insufficient_storage = is_enospc(&source);
        cleanup_staging_after_error(
            &managed_directory,
            &staging_root,
            operation.clone(),
            insufficient_storage,
        )?;
        return Err(InstallationPlacementError::Io {
            operation: "placing installation at",
            path: final_root.to_owned(),
            source,
        });
    }

    Ok(PlacedTree {
        root: final_root.to_owned(),
        extraction,
        operation_lock,
    })
}

fn secure_managed_directory(
    managed_directory: &Path,
    create: bool,
) -> Result<cap_std::fs::Dir, InstallationPlacementError> {
    let app_data = managed_directory
        .parent()
        .filter(|_| managed_directory.is_absolute())
        .ok_or_else(|| {
            InstallationPlacementError::InvalidFinalRoot(managed_directory.to_owned())
        })?;
    let managed_name = managed_directory
        .file_name()
        .filter(|name| !name.is_empty())
        .ok_or_else(|| {
            InstallationPlacementError::InvalidFinalRoot(managed_directory.to_owned())
        })?;
    if create {
        cap_std::fs::Dir::create_ambient_dir_all(app_data, cap_std::ambient_authority())
            .map_err(|source| placement_io("creating app data directory", app_data, source))?;
    }
    let app_data_directory =
        cap_std::fs::Dir::open_ambient_dir(app_data, cap_std::ambient_authority())
            .map_err(|source| placement_io("opening app data directory", app_data, source))?;
    if create {
        app_data_directory
            .create_dir_all(managed_name)
            .map_err(|source| {
                placement_io("creating managed directory", managed_directory, source)
            })?;
        #[cfg(unix)]
        {
            use cap_std::fs::PermissionsExt;

            app_data_directory
                .set_permissions(managed_name, cap_std::fs::Permissions::from_mode(0o700))
                .map_err(|source| {
                    placement_io("restricting managed directory", managed_directory, source)
                })?;
        }
    }
    open_managed_child_directory_no_follow(&app_data_directory, managed_name, managed_directory)
}

pub(crate) fn open_or_create_managed_directory(
    managed_directory: &Path,
) -> Result<cap_std::fs::Dir, InstallationPlacementError> {
    secure_managed_directory(managed_directory, true)
}

fn open_managed_child_directory_no_follow(
    managed_directory: &cap_std::fs::Dir,
    name: &std::ffi::OsStr,
    display_path: &Path,
) -> Result<cap_std::fs::Dir, InstallationPlacementError> {
    let parent = managed_directory
        .try_clone()
        .map_err(|source| {
            placement_io("cloning managed directory capability", display_path, source)
        })?
        .into_std_file();
    cap_primitives::fs::open_dir_nofollow(&parent, Path::new(name))
        .map(cap_std::fs::Dir::from_std_file)
        .map_err(|source| {
            placement_io(
                "opening managed child directory without following",
                display_path,
                source,
            )
        })
}

#[cfg(unix)]
fn file_identity(file: &File) -> Result<(u64, u64), std::io::Error> {
    use std::os::unix::fs::MetadataExt as _;

    let metadata = file.metadata()?;
    Ok((metadata.dev(), metadata.ino()))
}

#[cfg(windows)]
fn file_identity(file: &File) -> Result<(u64, u64), std::io::Error> {
    use std::ffi::c_void;
    use std::mem::MaybeUninit;
    use std::os::windows::io::AsRawHandle as _;

    #[repr(C)]
    struct FileTime {
        low: u32,
        high: u32,
    }

    #[repr(C)]
    struct ByHandleFileInformation {
        attributes: u32,
        creation_time: FileTime,
        last_access_time: FileTime,
        last_write_time: FileTime,
        volume_serial_number: u32,
        file_size_high: u32,
        file_size_low: u32,
        number_of_links: u32,
        file_index_high: u32,
        file_index_low: u32,
    }

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetFileInformationByHandle(
            file: *mut c_void,
            information: *mut ByHandleFileInformation,
        ) -> i32;
    }

    let mut information = MaybeUninit::<ByHandleFileInformation>::uninit();
    // SAFETY: the handle is live for the call and Windows initializes the
    // complete output structure when it reports success.
    let succeeded =
        unsafe { GetFileInformationByHandle(file.as_raw_handle(), information.as_mut_ptr()) };
    if succeeded == 0 {
        return Err(std::io::Error::last_os_error());
    }
    // SAFETY: successful GetFileInformationByHandle initialized the structure.
    let information = unsafe { information.assume_init() };
    let index =
        (u64::from(information.file_index_high) << 32) | u64::from(information.file_index_low);
    Ok((u64::from(information.volume_serial_number), index))
}

#[cfg(test)]
fn test_managed_child_access_path(
    managed_directory: &cap_std::fs::Dir,
    candidate: &Path,
) -> Result<PathBuf, InstallationPlacementError> {
    #[cfg(unix)]
    let name = candidate
        .file_name()
        .ok_or_else(|| InstallationPlacementError::InvalidFinalRoot(candidate.to_owned()))?;
    #[cfg(any(target_os = "linux", target_os = "android"))]
    {
        use std::os::fd::AsRawFd;

        return Ok(
            PathBuf::from(format!("/proc/self/fd/{}", managed_directory.as_raw_fd())).join(name),
        );
    }
    #[cfg(all(unix, not(any(target_os = "linux", target_os = "android"))))]
    {
        use std::os::fd::AsRawFd;

        return Ok(PathBuf::from(format!("/dev/fd/{}", managed_directory.as_raw_fd())).join(name));
    }
    #[cfg(windows)]
    {
        let _ = managed_directory;
        Ok(candidate.to_owned())
    }
}

#[derive(Debug)]
pub(crate) struct ManagedFileLease {
    file: File,
    managed_directory: cap_std::fs::Dir,
    name: std::ffi::OsString,
    path: PathBuf,
}

impl ManagedFileLease {
    pub(crate) fn download_file(&self) -> Result<File, InstallationPlacementError> {
        self.file
            .try_clone()
            .map_err(|source| placement_io("cloning managed child file", &self.path, source))
    }

    pub(crate) fn remove_if_owned(self) -> Result<(), std::io::Error> {
        let metadata = match self.managed_directory.symlink_metadata(&self.name) {
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(()),
            result => result?,
        };
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err(std::io::Error::other(
                "managed download candidate no longer names the created regular file",
            ));
        }
        let current = self.managed_directory.open(&self.name)?.into_std();
        if file_identity(&current)? != file_identity(&self.file)? {
            return Err(std::io::Error::other(
                "managed download candidate was replaced; preserving it",
            ));
        }
        drop(current);
        drop(self.file);
        self.managed_directory.remove_file(&self.name)
    }
}

pub(crate) fn create_managed_direct_child_file(
    managed_directory: &cap_std::fs::Dir,
    candidate: &Path,
) -> Result<ManagedFileLease, InstallationPlacementError> {
    let name = candidate
        .file_name()
        .ok_or_else(|| InstallationPlacementError::InvalidFinalRoot(candidate.to_owned()))?;
    let mut options = cap_std::fs::OpenOptions::new();
    options.read(true).write(true).create_new(true);
    #[cfg(unix)]
    {
        use cap_std::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    let file = managed_directory
        .open_with(name, &options)
        .map(cap_std::fs::File::into_std)
        .map_err(|source| placement_io("creating managed child file", candidate, source))?;
    let managed_directory = managed_directory
        .try_clone()
        .map_err(|source| placement_io("cloning managed directory", candidate, source))?;
    Ok(ManagedFileLease {
        file,
        managed_directory,
        name: name.to_owned(),
        path: candidate.to_owned(),
    })
}

fn ensure_final_root_absent_in(
    managed_directory: &cap_std::fs::Dir,
    final_name: &str,
    final_root: &Path,
) -> Result<(), InstallationPlacementError> {
    match managed_directory.symlink_metadata(final_name) {
        Ok(_) => Err(InstallationPlacementError::FinalRootExists(
            final_root.to_owned(),
        )),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Err(source) => Err(placement_io(
            "checking final installation root",
            final_root,
            source,
        )),
    }
}

pub(crate) fn remove_managed_direct_child_no_follow(
    managed_directory: &Path,
    candidate: &Path,
) -> Result<(), std::io::Error> {
    require_managed_direct_child(managed_directory, candidate).map_err(std::io::Error::other)?;
    let directory = secure_managed_directory(managed_directory, false).map_err(|error| {
        std::io::Error::other(format!("failed to open managed directory: {error}"))
    })?;
    let name = candidate
        .file_name()
        .ok_or_else(|| std::io::Error::other("managed child has no filename"))?;
    remove_managed_child_no_follow(&directory, name)
}

fn remove_managed_child_no_follow(
    directory: &cap_std::fs::Dir,
    name: &std::ffi::OsStr,
) -> Result<(), std::io::Error> {
    match directory.symlink_metadata(name) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            return Err(std::io::Error::other(
                "refusing to remove a symbolic link from a managed directory",
            ));
        }
        Ok(metadata) if metadata.is_dir() => directory.remove_dir_all(name)?,
        Ok(metadata) if metadata.is_file() => directory.remove_file(name)?,
        Ok(_) => {
            return Err(std::io::Error::other(
                "managed child is not a regular file or directory",
            ));
        }
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    }
    match directory.symlink_metadata(name) {
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Ok(_) => Err(std::io::Error::other(
            "managed child remained present after deletion",
        )),
        Err(error) => Err(error),
    }
}

impl OperationLock {
    pub(crate) fn root_exists_no_follow(&self) -> Result<bool, std::io::Error> {
        let name = self
            .final_root
            .file_name()
            .ok_or_else(|| std::io::Error::other("installation root has no filename"))?;
        match self.managed_directory.symlink_metadata(name) {
            Ok(_) => Ok(true),
            Err(error) if error.kind() == ErrorKind::NotFound => Ok(false),
            Err(error) => Err(error),
        }
    }

    pub(crate) fn remove_root_no_follow(&self) -> Result<(), std::io::Error> {
        let name = self
            .final_root
            .file_name()
            .ok_or_else(|| std::io::Error::other("installation root has no filename"))?;
        remove_managed_child_no_follow(&self.managed_directory, name)
    }

    pub(crate) fn acquire(final_root: &Path) -> Result<Self, InstallationPlacementError> {
        let (parent, final_name) = validated_final_root(final_root)?;
        let managed_directory = secure_managed_directory(parent, true)?;
        ensure_final_root_absent_in(&managed_directory, final_name, final_root)?;

        let lock_path = operation_lock_path(parent, final_name);
        let lock_name = operation_lock_name(final_name);
        let mut options = cap_std::fs::OpenOptions::new();
        options.read(true).write(true).create(true);
        #[cfg(unix)]
        {
            use cap_std::fs::OpenOptionsExt;

            options.mode(0o600);
        }
        let file = managed_directory
            .open_with(&lock_name, &options)
            .map_err(|source| placement_io("opening installation lock", &lock_path, source))?;
        let file = file.into_std();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;

            file.set_permissions(std::fs::Permissions::from_mode(0o600))
                .map_err(|source| {
                    placement_io("restricting installation lock", &lock_path, source)
                })?;
        }
        match file.try_lock() {
            Ok(()) => {}
            Err(std::fs::TryLockError::WouldBlock) => {
                return Err(InstallationPlacementError::Busy(final_root.to_owned()));
            }
            Err(std::fs::TryLockError::Error(source)) => {
                return Err(placement_io(
                    "locking installation operation",
                    &lock_path,
                    source,
                ));
            }
        }

        ensure_final_root_absent_in(&managed_directory, final_name, final_root)?;
        Ok(Self {
            final_root: final_root.to_owned(),
            _file: Arc::new(file),
            managed_directory: Arc::new(managed_directory),
        })
    }

    /// Acquires the stable lock left by a previously started operation. Unlike
    /// fresh acquisition this never creates or truncates the lock and permits
    /// any object to be present at the final root.
    pub(crate) fn acquire_existing_for_recovery(
        final_root: &Path,
    ) -> Result<Self, InstallationPlacementError> {
        let (parent, final_name) = validated_final_root(final_root)?;
        let managed_directory = secure_managed_directory(parent, false)?;
        let lock_path = operation_lock_path(parent, final_name);
        let mut options = cap_std::fs::OpenOptions::new();
        options.read(true).write(true);
        let file = managed_directory
            .open_with(operation_lock_name(final_name), &options)
            .map_err(|source| {
                placement_io("opening existing installation lock", &lock_path, source)
            })?
            .into_std();
        match file.try_lock() {
            Ok(()) => Ok(Self {
                final_root: final_root.to_owned(),
                _file: Arc::new(file),
                managed_directory: Arc::new(managed_directory),
            }),
            Err(std::fs::TryLockError::WouldBlock) => {
                Err(InstallationPlacementError::Busy(final_root.to_owned()))
            }
            Err(std::fs::TryLockError::Error(source)) => Err(placement_io(
                "locking existing installation operation",
                &lock_path,
                source,
            )),
        }
    }

    fn require_root(&self, final_root: &Path) -> Result<(), InstallationPlacementError> {
        if self.final_root == final_root {
            Ok(())
        } else {
            Err(InstallationPlacementError::OperationLockRootMismatch {
                locked_root: self.final_root.clone(),
                requested_root: final_root.to_owned(),
            })
        }
    }
}

fn operation_lock_path(parent: &Path, final_name: &str) -> PathBuf {
    parent.join(operation_lock_name(final_name))
}

fn operation_lock_name(final_name: &str) -> String {
    format!(".{final_name}.install.lock")
}

fn validated_final_root(final_root: &Path) -> Result<(&Path, &str), InstallationPlacementError> {
    let parent = final_root
        .parent()
        .filter(|_| final_root.is_absolute())
        .ok_or_else(|| InstallationPlacementError::InvalidFinalRoot(final_root.to_owned()))?;
    let final_name = final_root
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .ok_or_else(|| InstallationPlacementError::InvalidFinalRoot(final_root.to_owned()))?;
    Ok((parent, final_name))
}

fn reserve_staging_path(
    managed_directory: &cap_std::fs::Dir,
    parent: &Path,
    final_name: &str,
) -> Result<PathBuf, InstallationPlacementError> {
    for _ in 0..PLACEMENT_ATTEMPTS {
        let candidate = parent.join(format!(".{final_name}.{}.tmp", Uuid::new_v4()));
        let candidate_name = candidate
            .file_name()
            .ok_or_else(|| InstallationPlacementError::InvalidFinalRoot(candidate.clone()))?;
        match managed_directory.symlink_metadata(candidate_name) {
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(candidate),
            Ok(_) => continue,
            Err(source) => {
                return Err(placement_io(
                    "checking installation staging path",
                    &candidate,
                    source,
                ));
            }
        }
    }
    Err(placement_io(
        "reserving installation staging path",
        parent,
        std::io::Error::new(ErrorKind::AlreadyExists, "staging path attempts exhausted"),
    ))
}

fn cleanup_staging_after_error(
    managed_directory: &cap_std::fs::Dir,
    staging_root: &Path,
    operation: String,
    operation_insufficient_storage: bool,
) -> Result<(), InstallationPlacementError> {
    let name = staging_root
        .file_name()
        .ok_or_else(|| InstallationPlacementError::InvalidFinalRoot(staging_root.to_owned()))?;
    match managed_directory.remove_dir_all(name) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Err(cleanup) => Err(InstallationPlacementError::Cleanup {
            operation,
            operation_insufficient_storage,
            cleanup,
        }),
    }
}

fn placement_io(
    operation: &'static str,
    path: &Path,
    source: std::io::Error,
) -> InstallationPlacementError {
    InstallationPlacementError::Io {
        operation,
        path: path.to_owned(),
        source,
    }
}

/// Requires `candidate` to be an exact lexical direct child of the managed
/// directory. No canonicalization is used because the child may not exist and
/// following a hostile symlink would weaken the ownership boundary.
pub(crate) fn require_managed_direct_child(
    managed_directory: &Path,
    candidate: &Path,
) -> Result<(), String> {
    if !managed_directory.is_absolute()
        || !candidate.is_absolute()
        || candidate.parent() != Some(managed_directory)
        || candidate.file_name().is_none()
        || managed_directory.components().any(|component| {
            matches!(
                component,
                std::path::Component::CurDir | std::path::Component::ParentDir
            )
        })
        || candidate.components().any(|component| {
            matches!(
                component,
                std::path::Component::CurDir | std::path::Component::ParentDir
            )
        })
    {
        return Err(format!(
            "path {} is not an exact direct child of managed directory {}",
            candidate.display(),
            managed_directory.display()
        ));
    }
    Ok(())
}

/// Conservatively resolves interrupted installing rows. Lock and inspection
/// errors are deferred, and this routine never removes filesystem objects.
pub(crate) async fn recover_interrupted_installations(
    repository: &InstallationsRepository,
    managed_installations: &Path,
    updated_at: i64,
) -> Result<RecoveryReport, AdpStorageError> {
    let mut report = RecoveryReport::default();
    if let Some(app_data) = managed_installations.parent() {
        if let Err(error) = recover_updates(repository, app_data, updated_at, None).await {
            tracing::warn!(%error,"installation update recovery deferred");
            report.deferred_errors += 1;
        }
    }
    let snapshots = repository.list_installing().await?;
    for snapshot in snapshots {
        let _coordinate_guard = if let Some(app_data) = managed_installations.parent() {
            match super::acquire_coordinate_lifecycle_lock(
                app_data,
                &snapshot.installation.game_coordinate,
            )
            .await
            {
                Ok(lock) => Some(lock),
                Err(_) => {
                    report.busy += 1;
                    continue;
                }
            }
        } else {
            None
        };
        report.examined += 1;
        let recorded = snapshot.installation;
        if require_managed_direct_child(managed_installations, &recorded.installation_root).is_err()
        {
            match repository
                .mark_failed_for_recovery_if_unchanged(
                    &recorded.id,
                    recorded.updated_at,
                    InstallationRecoveryDiscriminator::FinalRootPresent,
                    updated_at,
                )
                .await
            {
                Ok(()) => report.marked_failed += 1,
                Err(AdpStorageError::InvalidTransition) => report.changed += 1,
                Err(error) => return Err(error),
            }
            continue;
        }
        let root = recorded.installation_root.clone();
        let lock_result = tokio::task::spawn_blocking(move || {
            OperationLock::acquire_existing_for_recovery(&root)
        })
        .await;
        let _lock = match lock_result {
            Ok(Ok(lock)) => lock,
            Ok(Err(InstallationPlacementError::Busy(_))) => {
                report.busy += 1;
                continue;
            }
            Ok(Err(_)) | Err(_) => {
                report.deferred_errors += 1;
                continue;
            }
        };

        match recover_locked_installation(repository, &recorded, updated_at).await? {
            RecoveryMarkOutcome::MarkedFailed => report.marked_failed += 1,
            RecoveryMarkOutcome::Changed => report.changed += 1,
        }
    }
    Ok(report)
}

async fn recover_locked_installation(
    repository: &InstallationsRepository,
    recorded: &InstallationRecord,
    updated_at: i64,
) -> Result<RecoveryMarkOutcome, AdpStorageError> {
    let Some(current) = repository.get_by_id(&recorded.id).await? else {
        return Ok(RecoveryMarkOutcome::Changed);
    };
    if current.installation.state != InstallationState::Installing
        || current.installation.updated_at != recorded.updated_at
        || current.installation.installation_root != recorded.installation_root
    {
        return Ok(RecoveryMarkOutcome::Changed);
    }
    let discriminator = match std::fs::symlink_metadata(&recorded.installation_root) {
        Err(error) if error.kind() == ErrorKind::NotFound => {
            InstallationRecoveryDiscriminator::FinalRootAbsent
        }
        Ok(_) | Err(_) => InstallationRecoveryDiscriminator::FinalRootPresent,
    };
    match repository
        .mark_failed_for_recovery_if_unchanged(
            &recorded.id,
            recorded.updated_at,
            discriminator,
            updated_at,
        )
        .await
    {
        Ok(()) => Ok(RecoveryMarkOutcome::MarkedFailed),
        Err(AdpStorageError::InvalidTransition) => Ok(RecoveryMarkOutcome::Changed),
        Err(error) => Err(error),
    }
}

#[allow(dead_code)]
pub(crate) async fn verify_downloaded_game(
    dest_path: &Path,
    expected_hash: &str,
) -> Result<(), String> {
    let actual_hash = sha256_file(dest_path).await.map_err(|err| {
        format!(
            "failed to compute downloaded file hash for {}: {err}",
            dest_path.display()
        )
    })?;

    if actual_hash != expected_hash {
        let quarantine_path = quarantine_corrupt_artifact(dest_path)
            .await
            .map_err(|err| {
                format!(
                "downloaded file hash mismatch: expected {expected_hash}, got {actual_hash}; {err}"
            )
            })?;
        return Err(format!(
            "downloaded file hash mismatch: expected {expected_hash}, got {actual_hash}; quarantined at {}",
            quarantine_path.display()
        ));
    }

    Ok(())
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

async fn quarantine_corrupt_artifact(dest_path: &Path) -> Result<PathBuf, String> {
    let primary = corrupt_artifact_path(dest_path);
    if let Some(quarantine_path) = copy_artifact_to_reserved_quarantine(dest_path, &primary).await?
    {
        return Ok(quarantine_path);
    }

    for index in 1..=1024 {
        let candidate = corrupt_artifact_path_with_suffix(dest_path, Some(index));
        match copy_artifact_to_reserved_quarantine(dest_path, &candidate).await? {
            Some(quarantine_path) => return Ok(quarantine_path),
            None => continue,
        }
    }

    Err(format!(
        "no available quarantine path for {} after 1024 attempts",
        dest_path.display()
    ))
}

async fn copy_artifact_to_reserved_quarantine(
    source_path: &Path,
    quarantine_path: &Path,
) -> Result<Option<PathBuf>, String> {
    let mut target = match tokio::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(quarantine_path)
        .await
    {
        Ok(file) => file,
        Err(err) if err.kind() == ErrorKind::AlreadyExists => return Ok(None),
        Err(err) => {
            return Err(format!(
                "failed to reserve quarantine path {}: {err}",
                quarantine_path.display()
            ));
        }
    };

    let mut source = match tokio::fs::File::open(source_path).await {
        Ok(file) => file,
        Err(err) => {
            let _ = tokio::fs::remove_file(quarantine_path).await;
            return Err(format!(
                "failed to open corrupt artifact {} for quarantine: {err}",
                source_path.display()
            ));
        }
    };

    if let Err(err) = tokio::io::copy(&mut source, &mut target).await {
        let _ = tokio::fs::remove_file(quarantine_path).await;
        return Err(format!(
            "failed to copy corrupt artifact to quarantine path {}: {err}",
            quarantine_path.display()
        ));
    }

    if let Err(err) = target.flush().await {
        let _ = tokio::fs::remove_file(quarantine_path).await;
        return Err(format!(
            "failed to flush quarantine path {}: {err}",
            quarantine_path.display()
        ));
    }

    if let Err(err) = target.sync_all().await {
        let _ = tokio::fs::remove_file(quarantine_path).await;
        return Err(format!(
            "failed to sync quarantine path {}: {err}",
            quarantine_path.display()
        ));
    }

    drop(target);
    drop(source);

    tokio::fs::remove_file(source_path).await.map_err(|err| {
        format!(
            "failed to remove original corrupt artifact {} after quarantine: {err}",
            source_path.display()
        )
    })?;

    Ok(Some(quarantine_path.to_path_buf()))
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::path::{Path, PathBuf};
    use std::sync::Arc;
    use std::time::{Instant, SystemTime, UNIX_EPOCH};

    use arcadestr_core::adp_storage::{
        InstallationProfile, InstallationState, ManifestEntryType, ProfileAvailability,
        ProfileUnavailableReason,
    };
    use arcadestr_core::archive_inspection::{DetectInspectError, InspectionError};
    use arcadestr_core::distribution::{
        LaunchType, PackageFormat, ProfileId, RelativePath, Sha256Digest, WorkingDir,
    };
    use arcadestr_core::file_hash::sha256_file;
    use arcadestr_core::storage::Database;
    use nostr::{EventBuilder, Keys, Kind, Tag};
    use tempfile::TempDir;

    fn unique_test_dir(test_name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "arcadestr-desktop-install-{test_name}-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system clock should be after epoch")
                .as_nanos()
        ))
    }

    #[test]
    fn archive_policy_classifies_typed_limits_in_both_extraction_passes() {
        for error in [
            InspectionError::CompressionRatioExceeded {
                measured_input: 1,
                measured_output: 1001,
                maximum_ratio: 1000,
            },
            InspectionError::ZeroMeasuredInput { measured_output: 1 },
            InspectionError::EntryLimitExceeded { limit: 1 },
            InspectionError::EntryCountOverflow,
            InspectionError::ZipCentralDirectoryLimitExceeded {
                declared_size: 2,
                limit: 1,
            },
            InspectionError::TarMetadataLimitExceeded {
                measured_metadata: 2,
                limit: 1,
            },
            InspectionError::MeasuredInputOverflow,
            InspectionError::MeasuredOutputOverflow,
        ] {
            for extraction in [
                ExtractionError::Policy(error.clone()),
                ExtractionError::Inspection(DetectInspectError::Inspection(error)),
            ] {
                let placement = InstallationPlacementError::Extraction(extraction);
                assert!(placement.is_archive_policy(), "{placement:?}");
                assert!(!placement.is_insufficient_storage());
            }
        }
        assert!(InstallationPlacementError::Extraction(
            ExtractionError::DeclaredRegularFileSizeOverflow
        )
        .is_archive_policy());
    }

    #[test]
    fn archive_policy_does_not_classify_unrelated_errors_by_text() {
        for extraction in [
            ExtractionError::Inspection(DetectInspectError::Read),
            ExtractionError::Policy(InspectionError::ZipIo {
                message: "CompressionRatioExceeded".into(),
            }),
            ExtractionError::DigestMismatch {
                expected: "archive_policy".into(),
                actual: "other".into(),
            },
        ] {
            assert!(!InstallationPlacementError::Extraction(extraction).is_archive_policy());
        }
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
    async fn verified_download_is_not_recorded_as_an_installation() {
        let dir = unique_test_dir("hash-match");
        tokio::fs::create_dir_all(&dir)
            .await
            .expect("test dir should be created");
        let expected_bytes = b"verified artifact bytes";
        let expected_hash = sha256_hex(expected_bytes, &dir).await;
        let final_install_path = dir.join("verified-game.zip");
        tokio::fs::write(&final_install_path, expected_bytes)
            .await
            .expect("downloaded artifact should be written");

        verify_downloaded_game(&final_install_path, &expected_hash)
            .await
            .expect("hash-matched artifact should verify");
        assert!(final_install_path.exists());

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
        let expected_bytes = b"verified artifact bytes";
        let expected_hash = sha256_hex(expected_bytes, &dir).await;
        let final_install_path = dir.join("corrupt-game.zip");
        let quarantine_path = corrupt_artifact_path(&final_install_path);
        tokio::fs::write(&final_install_path, b"corrupt bytes")
            .await
            .expect("corrupt artifact should be written");

        let err = verify_downloaded_game(&final_install_path, &expected_hash)
            .await
            .expect_err("hash mismatch should fail");

        assert!(err.contains("hash"));
        assert!(quarantine_path.exists());
        assert!(!final_install_path.exists());

        tokio::fs::remove_dir_all(&dir)
            .await
            .expect("test dir should be removed");
    }

    #[tokio::test]
    async fn install_preserves_existing_corrupt_artifact_when_quarantining_hash_mismatch() {
        let dir = unique_test_dir("hash-mismatch-existing-quarantine");
        tokio::fs::create_dir_all(&dir)
            .await
            .expect("test dir should be created");
        let expected_bytes = b"verified artifact bytes";
        let expected_hash = sha256_hex(expected_bytes, &dir).await;
        let final_install_path = dir.join("corrupt-game.zip");
        let quarantine_path = corrupt_artifact_path(&final_install_path);
        let suffixed_quarantine_path =
            corrupt_artifact_path_with_suffix(&final_install_path, Some(1));
        let existing_quarantine_bytes = b"previous corrupt artifact";
        let corrupt_bytes = b"new corrupt bytes";
        tokio::fs::write(&quarantine_path, existing_quarantine_bytes)
            .await
            .expect("existing quarantine artifact should be written");
        tokio::fs::write(&final_install_path, corrupt_bytes)
            .await
            .expect("corrupt artifact should be written");

        let err = verify_downloaded_game(&final_install_path, &expected_hash)
            .await
            .expect_err("hash mismatch should fail");

        assert!(err.contains("hash"));
        assert_eq!(
            tokio::fs::read(&quarantine_path)
                .await
                .expect("existing quarantine should remain readable"),
            existing_quarantine_bytes
        );
        assert_eq!(
            tokio::fs::read(&suffixed_quarantine_path)
                .await
                .expect("new quarantine should be readable"),
            corrupt_bytes
        );
        assert!(!final_install_path.exists());

        tokio::fs::remove_dir_all(&dir)
            .await
            .expect("test dir should be removed");
    }

    fn empty_extraction() -> ExtractionOutcome {
        ExtractionOutcome {
            manifest: Vec::new(),
            profiles: Vec::new(),
        }
    }

    fn operation_lock(final_root: &Path) -> OperationLock {
        OperationLock::acquire(final_root).expect("operation lock should be acquired")
    }

    fn staging_directories(parent: &Path) -> Vec<PathBuf> {
        std::fs::read_dir(parent)
            .expect("placement parent should be readable")
            .filter_map(Result::ok)
            .filter(|entry| {
                entry.file_type().is_ok_and(|kind| kind.is_dir())
                    && entry.file_name().to_string_lossy().ends_with(".tmp")
            })
            .map(|entry| entry.path())
            .collect()
    }

    fn fixture_path(name: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../tests/fixtures/a3-launch-profiles/generated")
            .join(name)
    }

    fn fixture_digest(name: &str) -> Sha256Digest {
        let sums_path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../tests/fixtures/a3-launch-profiles/SHA256SUMS");
        let sums =
            std::fs::read_to_string(&sums_path).expect("fixture checksums should be readable");
        let expected_path = format!("./{name}");
        let digest = sums.lines().find_map(|line| {
            let mut fields = line.split_whitespace();
            let digest = fields.next()?;
            (fields.next()? == expected_path).then_some(digest)
        });
        Sha256Digest::parse(digest.expect("fixture checksum should be recorded"))
            .expect("fixture checksum should be valid")
    }

    fn verified_fixture(name: &str, format: PackageFormat) -> VerifiedPackage {
        let path = fixture_path(name);
        let file = Arc::new(File::open(&path).expect("generated fixture should be readable"));
        VerifiedPackage::verify(file, fixture_digest(name), format)
            .expect("clean fixture should verify")
    }

    fn launch_profile(id: &str, executable: &str, working_dir: &str) -> LaunchProfile {
        LaunchProfile {
            id: ProfileId::parse(id).expect("profile id"),
            display_name: id.to_owned(),
            executable: RelativePath::parse(executable).expect("profile executable"),
            working_dir: WorkingDir::parse(working_dir).expect("profile working directory"),
            arguments: Vec::new(),
            launch_type: LaunchType::Game,
        }
    }

    fn initial_profiles(profiles: &[LaunchProfile]) -> Vec<InstallationProfile> {
        profiles
            .iter()
            .cloned()
            .map(|profile| InstallationProfile {
                profile,
                availability: ProfileAvailability::Unavailable(
                    ProfileUnavailableReason::ExecutableMissing,
                ),
            })
            .collect()
    }

    fn launch_type_text(launch_type: &LaunchType) -> &'static str {
        match launch_type {
            LaunchType::Game => "game",
            LaunchType::Config => "config",
            LaunchType::Server => "server",
            LaunchType::Editor => "editor",
            LaunchType::Tool => "tool",
        }
    }

    fn installing_record(
        d_tag: &str,
        root: &Path,
        package: &VerifiedPackage,
        profiles: &[LaunchProfile],
    ) -> InstallationRecord {
        assert!(
            !profiles.is_empty(),
            "installing fixtures require a launch profile matching the signed event"
        );
        let keys = Keys::generate();
        let package_format = match package.format() {
            PackageFormat::Zip => "zip",
            PackageFormat::TarGz => "tar.gz",
            PackageFormat::TarZstd => "tar.zst",
        };
        let mut tags = vec![
            Tag::parse(["d", d_tag]).expect("d tag"),
            Tag::parse(["file_hash", package.digest().as_str()]).expect("file hash tag"),
            Tag::parse(["server", "https://dist.example.com"]).expect("server tag"),
            Tag::parse(["package", package_format]).expect("package tag"),
            Tag::parse(["platform", "linux-x86_64"]).expect("platform tag"),
        ];
        for profile in profiles {
            tags.push(
                Tag::parse(vec![
                    "launch".to_owned(),
                    profile.id.as_str().to_owned(),
                    profile.executable.as_str().to_owned(),
                    profile.working_dir.as_str().to_owned(),
                    serde_json::to_string(&profile.arguments).expect("profile arguments JSON"),
                    launch_type_text(&profile.launch_type).to_owned(),
                    profile.display_name.clone(),
                ])
                .expect("launch tag"),
            );
        }
        let event = EventBuilder::new(Kind::Custom(30402), "listing")
            .tags(tags)
            .sign_with_keys(&keys)
            .expect("signed listing");
        InstallationRecord {
            id: InstallationId::generate(),
            game_coordinate: format!("30402:{}:{d_tag}", keys.public_key().to_hex()),
            installation_root: root.to_owned(),
            package_digest: package.digest().clone(),
            package_format: package.format().clone(),
            resolved_target: arcadestr_core::distribution::PlatformTarget {
                os: "linux".to_owned(),
                arch: "x86_64".to_owned(),
            },
            raw_event_json: serde_json::to_string(&event).expect("raw event JSON"),
            publisher_default_profile_id: None,
            state: InstallationState::Installing,
            recovery_discriminator: None,
            retained_package_coordinate: None,
            created_at: 1,
            updated_at: 1,
        }
    }

    async fn test_repository() -> (TempDir, Database, InstallationsRepository) {
        let temp = TempDir::new().expect("database temp directory");
        let database = Database::new(&temp.path().join("state.db"))
            .await
            .expect("test database should open");
        let repository = InstallationsRepository::new(database.pool().clone());
        (temp, database, repository)
    }

    #[tokio::test]
    async fn operation_lock_can_precede_database_begin_and_blocks_competitors() {
        let (_temp, _database, repository) = test_repository().await;
        let package = verified_fixture("windows-nested.zip", PackageFormat::Zip);
        let profiles = vec![launch_profile("play", "bin/Game.exe", "bin")];
        let final_root = _temp.path().join("installations").join("i-lock-first");
        let installation = installing_record("a4-4-lock-first", &final_root, &package, &profiles);

        let lease = OperationLock::acquire(&final_root)
            .expect("operation lock should be acquired before creating the database row");
        let competing = OperationLock::acquire(&final_root)
            .expect_err("competing operation should observe the pre-row lock");
        assert!(matches!(
            competing,
            InstallationPlacementError::Busy(path) if path == final_root
        ));

        repository
            .begin(&installation, &initial_profiles(&profiles))
            .await
            .expect("installing row should be created while the lease is held");
        drop(lease);
    }

    #[test]
    fn cloned_operation_lease_keeps_underlying_os_lock_alive() {
        let dir = unique_test_dir("shared-operation-lock");
        let final_root = dir.join("installations").join("i-shared");
        let first = OperationLock::acquire(&final_root).expect("first lease");
        let retained = first.clone();
        drop(first);
        assert!(matches!(
            OperationLock::acquire(&final_root),
            Err(InstallationPlacementError::Busy(path)) if path == final_root
        ));
        drop(retained);
        OperationLock::acquire_existing_for_recovery(&final_root)
            .expect("existing stable lock should become available after every lease drops");
        std::fs::remove_dir_all(dir).expect("shared lock test directory should be removed");
    }

    #[test]
    fn recovery_lock_accepts_present_root_but_never_creates_missing_lock() {
        let dir = unique_test_dir("recovery-present-root");
        let final_root = dir.join("installations").join("i-present");
        let fresh = OperationLock::acquire(&final_root).expect("fresh lock");
        drop(fresh);
        std::fs::create_dir(&final_root).expect("final root fixture");
        OperationLock::acquire_existing_for_recovery(&final_root)
            .expect("present final root should be recoverable");
        let missing_lock_root = dir.join("installations").join("i-no-lock");
        assert!(matches!(
            OperationLock::acquire_existing_for_recovery(&missing_lock_root),
            Err(InstallationPlacementError::Io { .. })
        ));
        std::fs::remove_dir_all(dir).expect("recovery lock test directory should be removed");
    }

    #[test]
    fn managed_containment_requires_an_exact_direct_child() {
        let managed = Path::new("/app/data/installations");
        assert!(
            require_managed_direct_child(managed, Path::new("/app/data/installations/id")).is_ok()
        );
        for rejected in [
            Path::new("/app/data/installations/id/nested"),
            Path::new("/app/data/other/id"),
            Path::new("/app/data/installations/../other"),
        ] {
            assert!(require_managed_direct_child(managed, rejected).is_err());
        }
    }

    #[cfg(unix)]
    #[test]
    fn operation_lock_rejects_symlinked_managed_directory() {
        use std::os::unix::fs::symlink;

        let app_data = unique_test_dir("managed-symlink");
        let outside = unique_test_dir("managed-symlink-outside");
        std::fs::create_dir_all(&app_data).expect("app data fixture");
        std::fs::create_dir_all(&outside).expect("outside fixture");
        let managed = app_data.join("installations");
        symlink(&outside, &managed).expect("managed directory symlink");

        let error = OperationLock::acquire(&managed.join("installation-id"))
            .expect_err("symlinked managed directory must be rejected");
        assert!(matches!(error, InstallationPlacementError::Io { .. }));
        assert!(std::fs::read_dir(&outside)
            .expect("outside directory")
            .next()
            .is_none());
    }

    #[cfg(unix)]
    #[test]
    fn managed_child_directory_open_does_not_follow_replacement_symlink() {
        use std::os::unix::fs::symlink;

        let managed_path = unique_test_dir("managed-child-nofollow");
        let outside = unique_test_dir("managed-child-nofollow-outside");
        std::fs::create_dir_all(&managed_path).expect("managed directory fixture");
        std::fs::create_dir_all(&outside).expect("outside directory fixture");
        let managed = secure_managed_directory(&managed_path, false).expect("managed capability");
        let child = managed_path.join("staging");
        symlink(&outside, &child).expect("replacement symlink fixture");

        assert!(open_managed_child_directory_no_follow(
            &managed,
            std::ffi::OsStr::new("staging"),
            &child,
        )
        .is_err());
        assert!(std::fs::read_dir(&outside)
            .expect("outside directory")
            .next()
            .is_none());
    }

    #[tokio::test]
    async fn recovery_marks_absent_and_present_roots_without_deleting_and_skips_live_lock() {
        let (_temp, _database, repository) = test_repository().await;
        let managed = _temp.path().join("installations");
        let package = verified_fixture("windows-nested.zip", PackageFormat::Zip);
        let profiles = vec![launch_profile("play", "bin/Game.exe", "bin")];

        let absent_root = managed.join("i-recover-absent");
        let absent = installing_record("recover-absent", &absent_root, &package, &profiles);
        let absent_lock = OperationLock::acquire(&absent_root).expect("absent stable lock");
        repository
            .begin(&absent, &initial_profiles(&profiles))
            .await
            .expect("absent row");
        drop(absent_lock);

        let present_root = managed.join("i-recover-present");
        let present = installing_record("recover-present", &present_root, &package, &profiles);
        let present_lock = OperationLock::acquire(&present_root).expect("present stable lock");
        repository
            .begin(&present, &initial_profiles(&profiles))
            .await
            .expect("present row");
        drop(present_lock);
        std::fs::create_dir(&present_root).expect("present root");
        std::fs::write(present_root.join("owned-marker"), b"preserve").expect("present marker");

        let live_root = managed.join("i-recover-live");
        let live = installing_record("recover-live", &live_root, &package, &profiles);
        let _live_lock = OperationLock::acquire(&live_root).expect("live lock");
        repository
            .begin(&live, &initial_profiles(&profiles))
            .await
            .expect("live row");

        let untrusted_root = _temp.path().join("outside-managed").join("root");
        std::fs::create_dir_all(&untrusted_root).expect("untrusted root fixture");
        let untrusted_marker = untrusted_root.join("preserve");
        std::fs::write(&untrusted_marker, b"untouched").expect("untrusted marker");
        let untrusted =
            installing_record("recover-untrusted", &untrusted_root, &package, &profiles);
        repository
            .begin(&untrusted, &initial_profiles(&profiles))
            .await
            .expect("untrusted row");

        let missing_lock_root = managed.join("i-recover-missing-lock");
        let missing_lock = installing_record(
            "recover-missing-lock",
            &missing_lock_root,
            &package,
            &profiles,
        );
        repository
            .begin(&missing_lock, &initial_profiles(&profiles))
            .await
            .expect("missing-lock row");

        let report = recover_interrupted_installations(&repository, &managed, 2)
            .await
            .expect("recovery scan");
        assert_eq!(report.marked_failed, 3);
        assert_eq!(report.busy, 1);
        assert_eq!(report.deferred_errors, 1);
        let absent_snapshot = repository
            .get_by_id(&absent.id)
            .await
            .expect("absent query")
            .expect("absent snapshot");
        assert_eq!(
            absent_snapshot.installation.recovery_discriminator,
            Some(InstallationRecoveryDiscriminator::FinalRootAbsent)
        );
        let present_snapshot = repository
            .get_by_id(&present.id)
            .await
            .expect("present query")
            .expect("present snapshot");
        assert_eq!(
            present_snapshot.installation.recovery_discriminator,
            Some(InstallationRecoveryDiscriminator::FinalRootPresent)
        );
        assert_eq!(
            std::fs::read(present_root.join("owned-marker")).expect("marker survives"),
            b"preserve"
        );
        assert_eq!(
            repository
                .get_by_id(&live.id)
                .await
                .expect("live query")
                .expect("live snapshot")
                .installation
                .state,
            InstallationState::Installing
        );
        let untrusted_snapshot = repository
            .get_by_id(&untrusted.id)
            .await
            .expect("untrusted query")
            .expect("untrusted snapshot");
        assert_eq!(
            untrusted_snapshot.installation.state,
            InstallationState::Failed
        );
        assert_eq!(
            untrusted_snapshot.installation.recovery_discriminator,
            Some(InstallationRecoveryDiscriminator::FinalRootPresent)
        );
        assert_eq!(
            std::fs::read(untrusted_marker).expect("untrusted marker survives"),
            b"untouched"
        );
        assert_eq!(
            repository
                .get_by_id(&missing_lock.id)
                .await
                .expect("missing-lock query")
                .expect("missing-lock snapshot")
                .installation
                .state,
            InstallationState::Installing
        );
    }

    #[tokio::test]
    async fn recovery_locked_reread_leaves_row_changed_from_installing_untouched() {
        let (_temp, _database, repository) = test_repository().await;
        let package = verified_fixture("windows-nested.zip", PackageFormat::Zip);
        let profiles = vec![launch_profile("play", "bin/Game.exe", "bin")];
        let root = _temp.path().join("installations").join("i-row-changed");
        let recorded = installing_record("recover-row-changed", &root, &package, &profiles);
        repository
            .begin(&recorded, &initial_profiles(&profiles))
            .await
            .expect("installing row");
        repository
            .mark_failed(&recorded.id, 2)
            .await
            .expect("concurrent state change");

        assert_eq!(
            recover_locked_installation(&repository, &recorded, 3)
                .await
                .expect("changed recovery reread"),
            RecoveryMarkOutcome::Changed
        );
        let current = repository
            .get_by_id(&recorded.id)
            .await
            .expect("changed row query")
            .expect("changed row");
        assert_eq!(current.installation.state, InstallationState::Failed);
        assert_eq!(current.installation.updated_at, 2);
        assert_eq!(current.installation.recovery_discriminator, None);
    }

    #[derive(Clone, Copy)]
    enum HostileRejection {
        BlockingHazard,
        RatioLimit,
    }

    fn assert_hostile_fixture_aborts_placement(
        name: &str,
        format: PackageFormat,
        expected: HostileRejection,
    ) {
        let fixture = fixture_path(name);
        let file = Arc::new(File::open(&fixture).expect("hostile fixture should be readable"));
        let digest = fixture_digest(name);
        let dir = unique_test_dir(name);
        let final_root = dir.join("installations").join("i-hostile");

        let lock = operation_lock(&final_root);
        let error = place_with_extractor(&final_root, lock, move |staging_root| {
            let package = VerifiedPackage::verify(file, digest, format)?;
            extract_verified_package(&package, staging_root, &[])
        })
        .expect_err("hostile fixture must not be placed");

        match expected {
            HostileRejection::BlockingHazard => assert!(
                matches!(
                    &error,
                    InstallationPlacementError::Extraction(ExtractionError::BlockingHazards(_))
                ),
                "unexpected hostile-fixture error: {error:?}"
            ),
            HostileRejection::RatioLimit => assert!(
                matches!(
                    &error,
                    InstallationPlacementError::Extraction(ExtractionError::Inspection(
                        DetectInspectError::Inspection(
                            InspectionError::CompressionRatioExceeded { .. }
                        )
                    ))
                ),
                "unexpected ratio-fixture error: {error:?}"
            ),
        }
        if matches!(expected, HostileRejection::RatioLimit) {
            let InstallationPlacementError::Extraction(extraction) = &error else {
                panic!("expected typed extraction error");
            };
            let rejection =
                arcadestr_core::install_contract::InstallCommandError::from_extraction(extraction);
            assert_eq!(
                serde_json::to_value(rejection).expect("serialize actual ratio fixture rejection"),
                serde_json::json!({"code": "compression_ratio_exceeded"}),
            );
        }
        assert!(!final_root.exists(), "final installation root was modified");
        assert!(
            staging_directories(final_root.parent().expect("final parent")).is_empty(),
            "adjacent temporary directory survived rejection"
        );
        std::fs::remove_dir_all(dir).expect("hostile fixture test directory should be removed");
    }

    fn regular_files(root: &Path) -> Vec<PathBuf> {
        let mut pending = vec![root.to_owned()];
        let mut files = Vec::new();
        while let Some(directory) = pending.pop() {
            for entry in
                std::fs::read_dir(&directory).expect("fixture directory should be readable")
            {
                let entry = entry.expect("fixture entry should be readable");
                let file_type = entry
                    .file_type()
                    .expect("fixture entry type should be readable");
                if file_type.is_dir() {
                    pending.push(entry.path());
                } else if file_type.is_file() {
                    files.push(entry.path());
                }
            }
        }
        files
    }

    #[test]
    fn reserved_device_fixture_aborts_placement_without_residue() {
        assert_hostile_fixture_aborts_placement(
            "reserved-device.zip",
            PackageFormat::Zip,
            HostileRejection::BlockingHazard,
        );
    }

    #[test]
    fn case_collision_fixture_aborts_placement_without_residue() {
        assert_hostile_fixture_aborts_placement(
            "case-collision.zip",
            PackageFormat::Zip,
            HostileRejection::BlockingHazard,
        );
    }

    #[test]
    fn escaping_symlink_fixture_aborts_placement_without_residue() {
        assert_hostile_fixture_aborts_placement(
            "escaping-symlink.zip",
            PackageFormat::Zip,
            HostileRejection::BlockingHazard,
        );
    }

    #[test]
    fn hardlink_fixture_aborts_placement_without_residue() {
        assert_hostile_fixture_aborts_placement(
            "hardlink.tar.zst",
            PackageFormat::TarZstd,
            HostileRejection::BlockingHazard,
        );
    }

    #[test]
    fn traversal_fixture_aborts_placement_without_residue() {
        assert_hostile_fixture_aborts_placement(
            "traversal.zip",
            PackageFormat::Zip,
            HostileRejection::BlockingHazard,
        );
    }

    #[test]
    fn absolute_path_fixture_aborts_placement_without_residue() {
        assert_hostile_fixture_aborts_placement(
            "absolute-path.zip",
            PackageFormat::Zip,
            HostileRejection::BlockingHazard,
        );
    }

    #[test]
    fn backslash_name_fixture_aborts_placement_without_residue() {
        assert_hostile_fixture_aborts_placement(
            "backslash-name.zip",
            PackageFormat::Zip,
            HostileRejection::BlockingHazard,
        );
    }

    #[test]
    fn ratio_bomb_fixture_aborts_placement_without_residue() {
        assert_hostile_fixture_aborts_placement(
            "ratio-bomb.zip",
            PackageFormat::Zip,
            HostileRejection::RatioLimit,
        );
    }

    #[test]
    fn windows_nested_fixture_preserves_every_leading_component() {
        let package = verified_fixture("windows-nested.zip", PackageFormat::Zip);
        let dir = unique_test_dir("windows-nested-fixture");
        let final_root = dir.join("installations").join("i-windows");

        let lock = operation_lock(&final_root);
        let placed = extract_and_place_verified_package(&package, &final_root, &[], lock)
            .expect("nested Windows fixture should extract");

        for path in [
            "bin/Game.exe",
            "data/config.json",
            "data/maps/tutorial.dat",
            "data/audio/theme.dat",
            "data/assets.pack",
        ] {
            assert!(final_root.join(path).is_file(), "missing exact path {path}");
        }
        assert!(!final_root.join("Game.exe").exists());
        assert!(!final_root.join("windows").exists());
        assert!(placed
            .extraction
            .manifest
            .iter()
            .any(|entry| entry.path.as_str() == "bin/Game.exe"));
        assert!(staging_directories(final_root.parent().expect("final parent")).is_empty());
        std::fs::remove_dir_all(dir).expect("Windows fixture test directory should be removed");
    }

    #[cfg(unix)]
    #[test]
    fn linux_layout_fixture_marks_only_profile_executable_as_executable() {
        use std::os::unix::fs::PermissionsExt;

        let package = verified_fixture("linux-layout.tar.zst", PackageFormat::TarZstd);
        let profile = LaunchProfile {
            id: ProfileId::parse("play").expect("profile id"),
            display_name: "Play".to_owned(),
            executable: RelativePath::parse("bin/game").expect("profile executable"),
            working_dir: WorkingDir::parse("bin").expect("profile working directory"),
            arguments: Vec::new(),
            launch_type: LaunchType::Game,
        };
        let dir = unique_test_dir("linux-layout-fixture");
        let final_root = dir.join("installations").join("i-linux");

        let lock = operation_lock(&final_root);
        let placed = extract_and_place_verified_package(&package, &final_root, &[profile], lock)
            .expect("Linux fixture should extract");

        assert_eq!(
            placed.extraction.profiles[0].availability,
            ProfileAvailability::Available
        );
        for file in regular_files(&final_root) {
            let relative = file.strip_prefix(&final_root).expect("file below root");
            let execute_bits = std::fs::metadata(&file)
                .expect("fixture metadata should be readable")
                .permissions()
                .mode()
                & 0o111;
            if relative == Path::new("bin/game") {
                assert_ne!(execute_bits, 0, "profile executable lacks execute bit");
            } else {
                assert_eq!(execute_bits, 0, "unexpected executable file {relative:?}");
            }
        }
        assert!(staging_directories(final_root.parent().expect("final parent")).is_empty());
        std::fs::remove_dir_all(dir).expect("Linux fixture test directory should be removed");
    }

    #[test]
    fn large_index_5000_fixture_extracts_and_reports_timing() {
        let package = verified_fixture("large-index-5000.zip", PackageFormat::Zip);
        let dir = unique_test_dir("large-index-5000-fixture");
        let final_root = dir.join("installations").join("i-large");
        let started = Instant::now();

        let lock = operation_lock(&final_root);
        let placed = extract_and_place_verified_package(&package, &final_root, &[], lock)
            .expect("5,000-entry fixture should extract");
        let elapsed = started.elapsed();
        eprintln!("large-index-5000.zip extraction time: {elapsed:.3?}");

        assert!(final_root.join("bin/Game.exe").is_file());
        assert!(final_root.join("assets/maps/map-0001.dat").is_file());
        assert!(final_root.join("assets/maps/map-5000.dat").is_file());
        assert_eq!(
            placed
                .extraction
                .manifest
                .iter()
                .filter(|entry| entry.entry_type == ManifestEntryType::File)
                .count(),
            5_002
        );
        assert!(staging_directories(final_root.parent().expect("final parent")).is_empty());
        std::fs::remove_dir_all(dir).expect("large fixture test directory should be removed");
    }

    #[tokio::test]
    async fn real_extraction_outcome_commits_as_exact_ready_snapshot() {
        let (_temp, _database, repository) = test_repository().await;
        let package = verified_fixture("linux-layout.tar.zst", PackageFormat::TarZstd);
        let profiles = vec![
            launch_profile("play", "bin/game", "bin"),
            launch_profile("missing", "bin/missing", "bin"),
        ];
        let final_root = _temp.path().join("installations").join("i-round-trip");
        let installation = installing_record("a4-3-round-trip", &final_root, &package, &profiles);
        let initial = initial_profiles(&profiles);
        repository
            .begin(&installation, &initial)
            .await
            .expect("installing row should be created");
        let lock = operation_lock(&final_root);
        let placed = extract_and_place_verified_package(&package, &final_root, &profiles, lock)
            .expect("fixture should be placed");
        let expected_extraction = placed.extraction.clone();

        commit_placed_installation(&repository, &installation.id, placed, 2)
            .await
            .expect("completed extraction should commit");

        let snapshot = repository
            .get_by_id(&installation.id)
            .await
            .expect("snapshot query should succeed")
            .expect("committed snapshot should exist");
        let mut expected_installation = installation;
        expected_installation.state = InstallationState::Ready;
        expected_installation.updated_at = 2;
        assert_eq!(snapshot.installation, expected_installation);
        assert_eq!(snapshot.profiles, expected_extraction.profiles);
        assert_eq!(snapshot.manifest, expected_extraction.manifest);
        assert!(snapshot
            .manifest
            .iter()
            .filter(|entry| entry.entry_type == ManifestEntryType::Directory)
            .all(|entry| entry.file_digest.is_none()));
        assert!(snapshot
            .manifest
            .iter()
            .filter(|entry| entry.entry_type == ManifestEntryType::File)
            .all(|entry| entry.file_digest.is_some()));
        assert_eq!(
            snapshot.profiles[0].availability,
            ProfileAvailability::Available
        );
        assert_eq!(
            snapshot.profiles[1].availability,
            ProfileAvailability::Unavailable(ProfileUnavailableReason::ExecutableMissing)
        );
        assert!(snapshot.any_profile_available());
        assert!(repository
            .any_profile_available(&snapshot.installation.id)
            .await
            .expect("availability query should succeed"));
    }

    #[tokio::test]
    async fn all_unavailable_profiles_commit_ready_and_remain_queryable() {
        let (_temp, _database, repository) = test_repository().await;
        let package = verified_fixture("windows-nested.zip", PackageFormat::Zip);
        let profiles = vec![
            launch_profile("missing", "bin/Missing.exe", "."),
            launch_profile("bad-working-dir", "bin/Game.exe", "bin/Game.exe"),
        ];
        let final_root = _temp.path().join("installations").join("i-unavailable");
        let installation =
            installing_record("a4-3-all-unavailable", &final_root, &package, &profiles);
        repository
            .begin(&installation, &initial_profiles(&profiles))
            .await
            .expect("installing row should be created");
        let lock = operation_lock(&final_root);
        let placed = extract_and_place_verified_package(&package, &final_root, &profiles, lock)
            .expect("fixture should be placed despite unavailable profiles");

        commit_placed_installation(&repository, &installation.id, placed, 2)
            .await
            .expect("all-unavailable installation should commit");

        let snapshot = repository
            .get_by_id(&installation.id)
            .await
            .expect("snapshot query should succeed")
            .expect("committed snapshot should exist");
        assert_eq!(snapshot.installation.state, InstallationState::Ready);
        assert_eq!(
            snapshot.profiles[0].availability,
            ProfileAvailability::Unavailable(ProfileUnavailableReason::ExecutableMissing)
        );
        assert_eq!(
            snapshot.profiles[1].availability,
            ProfileAvailability::Unavailable(
                ProfileUnavailableReason::WorkingDirectoryNotDirectory
            )
        );
        assert!(!snapshot.any_profile_available());
        assert!(!repository
            .any_profile_available(&installation.id)
            .await
            .expect("availability query should succeed"));
    }

    #[tokio::test]
    async fn commit_failure_rolls_back_database_and_preserves_placed_tree_for_recovery() {
        let (_temp, database, repository) = test_repository().await;
        let package = verified_fixture("linux-layout.tar.zst", PackageFormat::TarZstd);
        let profiles = vec![launch_profile("play", "bin/game", "bin")];
        let final_root = _temp.path().join("installations").join("i-rollback");
        let installation = installing_record("a4-3-rollback", &final_root, &package, &profiles);
        let initial = initial_profiles(&profiles);
        repository
            .begin(&installation, &initial)
            .await
            .expect("installing row should be created");
        let lock = operation_lock(&final_root);
        let placed = extract_and_place_verified_package(&package, &final_root, &profiles, lock)
            .expect("fixture should be placed");
        sqlx::query(
            r#"CREATE TRIGGER fail_desktop_installation_ready
               BEFORE UPDATE OF state ON installations
               WHEN NEW.state = 'ready'
               BEGIN SELECT RAISE(ABORT, 'injected ready failure'); END"#,
        )
        .execute(database.pool())
        .await
        .expect("failure trigger should be installed");

        let error = commit_placed_installation(&repository, &installation.id, placed, 2)
            .await
            .expect_err("injected commit failure should propagate");

        assert!(error.recovery_required());
        assert!(error.cleanup_failure.is_some());
        assert!(final_root.exists());
        let snapshot = repository
            .get_by_id(&installation.id)
            .await
            .expect("snapshot query should succeed")
            .expect("installing row should remain");
        assert_eq!(snapshot.installation.state, InstallationState::Installing);
        assert_eq!(snapshot.profiles, initial);
        assert!(snapshot.manifest.is_empty());
    }

    #[tokio::test]
    async fn matching_ready_retry_is_idempotent_and_preserves_tree() {
        let (_temp, _database, repository) = test_repository().await;
        let package = verified_fixture("linux-layout.tar.zst", PackageFormat::TarZstd);
        let profiles = vec![launch_profile("play", "bin/game", "bin")];
        let final_root = _temp.path().join("installations").join("i-ready-retry");
        let installation = installing_record("a4-3-ready-retry", &final_root, &package, &profiles);
        repository
            .begin(&installation, &initial_profiles(&profiles))
            .await
            .expect("installing row should be created");
        let lock = operation_lock(&final_root);
        let placed = extract_and_place_verified_package(&package, &final_root, &profiles, lock)
            .expect("fixture should be placed");
        repository
            .commit_extraction(
                &installation.id,
                &placed.root,
                &placed.package_digest,
                &placed.package_format,
                &placed.extraction.profiles,
                &placed.extraction.manifest,
                2,
            )
            .await
            .expect("first commit should succeed");

        commit_placed_installation(&repository, &installation.id, placed, 2)
            .await
            .expect("matching ready retry should be accepted");

        assert!(final_root.join("bin/game").is_file());
        assert_eq!(
            repository
                .get_by_id(&installation.id)
                .await
                .expect("snapshot query should succeed")
                .expect("ready snapshot should exist")
                .installation
                .state,
            InstallationState::Ready
        );
    }

    #[tokio::test]
    async fn missing_installing_row_rejects_commit_and_preserves_tree() {
        let (_temp, _database, repository) = test_repository().await;
        let package = verified_fixture("linux-layout.tar.zst", PackageFormat::TarZstd);
        let profiles = vec![launch_profile("play", "bin/game", "bin")];
        let final_root = _temp.path().join("installations").join("i-missing-row");
        let lock = operation_lock(&final_root);
        let placed = extract_and_place_verified_package(&package, &final_root, &profiles, lock)
            .expect("fixture should be placed");

        let error = commit_placed_installation(&repository, &InstallationId::generate(), placed, 2)
            .await
            .expect_err("missing row should reject commit");

        assert!(error.recovery_required());
        assert!(error.cleanup_failure.is_some());
        assert!(final_root.exists());
    }

    #[tokio::test]
    async fn placed_package_identity_mismatch_rejects_commit_and_preserves_tree() {
        let (_temp, _database, repository) = test_repository().await;
        let recorded_package = verified_fixture("windows-nested.zip", PackageFormat::Zip);
        let placed_package = verified_fixture("large-index-5000.zip", PackageFormat::Zip);
        let profiles = vec![launch_profile("play", "bin/Game.exe", "bin")];
        let final_root = _temp
            .path()
            .join("installations")
            .join("i-package-mismatch");
        let installation = installing_record(
            "a4-3-package-mismatch",
            &final_root,
            &recorded_package,
            &profiles,
        );
        repository
            .begin(&installation, &initial_profiles(&profiles))
            .await
            .expect("installing row should be created");
        let lock = operation_lock(&final_root);
        let placed =
            extract_and_place_verified_package(&placed_package, &final_root, &profiles, lock)
                .expect("different fixture should be placed");

        let error = commit_placed_installation(&repository, &installation.id, placed, 2)
            .await
            .expect_err("bound package mismatch should reject commit");

        assert!(error.recovery_required());
        assert!(error.cleanup_failure.is_some());
        assert!(final_root.exists());
        assert_eq!(
            repository
                .get_by_id(&installation.id)
                .await
                .expect("snapshot query should succeed")
                .expect("installing row should remain")
                .installation
                .state,
            InstallationState::Installing
        );
    }

    #[tokio::test]
    async fn large_manifest_commit_reports_timing() {
        let (_temp, _database, repository) = test_repository().await;
        let package = verified_fixture("large-index-5000.zip", PackageFormat::Zip);
        let profiles = vec![launch_profile("play", "bin/Game.exe", "bin")];
        let final_root = _temp.path().join("installations").join("i-large-commit");
        let installation = installing_record("a4-3-large", &final_root, &package, &profiles);
        repository
            .begin(&installation, &initial_profiles(&profiles))
            .await
            .expect("installing row should be created");
        let lock = operation_lock(&final_root);
        let placed = extract_and_place_verified_package(&package, &final_root, &profiles, lock)
            .expect("large fixture should be placed");
        assert_eq!(placed.extraction.manifest.len(), 5_006);
        let started = Instant::now();

        commit_placed_installation(&repository, &installation.id, placed, 2)
            .await
            .expect("large manifest should commit");
        let elapsed = started.elapsed();
        eprintln!("5,000-entry manifest commit time: {elapsed:.3?}");

        let snapshot = repository
            .get_by_id(&installation.id)
            .await
            .expect("snapshot query should succeed")
            .expect("large snapshot should exist");
        assert_eq!(snapshot.installation.state, InstallationState::Ready);
        assert_eq!(snapshot.manifest.len(), 5_006);
    }

    #[test]
    fn placement_exposes_only_complete_tree() {
        let dir = unique_test_dir("atomic-placement");
        let final_root = dir.join("installations").join("i-123");

        let lock = operation_lock(&final_root);
        let placed = place_with_extractor(&final_root, lock, |staging_root| {
            assert!(!final_root.exists());
            std::fs::create_dir(staging_root).expect("staging root should be created");
            std::fs::write(staging_root.join("game.bin"), b"complete")
                .expect("staged game should be written");
            assert!(!final_root.exists());
            Ok(empty_extraction())
        })
        .expect("completed staging tree should be placed");

        assert_eq!(placed.root, final_root);
        assert!(placed.extraction.manifest.is_empty());
        assert_eq!(
            std::fs::read(final_root.join("game.bin")).expect("placed game should be readable"),
            b"complete"
        );
        assert!(staging_directories(final_root.parent().expect("final parent")).is_empty());
        std::fs::remove_dir_all(dir).expect("test directory should be removed");
    }

    #[test]
    fn placement_failure_removes_partial_staging_and_leaves_final_absent() {
        let dir = unique_test_dir("failed-placement");
        let final_root = dir.join("installations").join("i-456");

        let lock = operation_lock(&final_root);
        let error = place_with_extractor(&final_root, lock, |staging_root| {
            std::fs::create_dir(staging_root).expect("staging root should be created");
            std::fs::write(staging_root.join("partial.bin"), b"partial")
                .expect("partial game should be written");
            Err(ExtractionError::InvalidPath {
                path: "../escape".to_owned(),
                message: "test rejection".to_owned(),
            })
        })
        .expect_err("failed extraction must not be placed");

        assert!(matches!(error, InstallationPlacementError::Extraction(_)));
        assert!(!final_root.exists());
        assert!(staging_directories(final_root.parent().expect("final parent")).is_empty());
        std::fs::remove_dir_all(dir).expect("test directory should be removed");
    }

    #[test]
    fn placement_preserves_existing_final_root() {
        let dir = unique_test_dir("existing-final");
        let final_root = dir.join("installations").join("i-789");
        std::fs::create_dir_all(&final_root).expect("existing root should be created");
        std::fs::write(final_root.join("buyer-save"), b"keep")
            .expect("existing data should be written");

        let error = OperationLock::acquire(&final_root)
            .expect_err("existing final root must prevent an installation lease");

        assert!(matches!(
            error,
            InstallationPlacementError::FinalRootExists(path) if path == final_root
        ));
        assert_eq!(
            std::fs::read(final_root.join("buyer-save")).expect("existing data should remain"),
            b"keep"
        );
        std::fs::remove_dir_all(dir).expect("test directory should be removed");
    }

    #[test]
    fn placement_does_not_replace_a_final_root_created_during_extraction() {
        let dir = unique_test_dir("late-final");
        let final_root = dir.join("installations").join("i-late");

        let lock = operation_lock(&final_root);
        let error = place_with_extractor(&final_root, lock, |staging_root| {
            std::fs::create_dir(staging_root).expect("staging root should be created");
            std::fs::write(staging_root.join("game.bin"), b"package")
                .expect("staged game should be written");
            std::fs::create_dir(&final_root).expect("late final root should be created");
            std::fs::write(final_root.join("foreign-data"), b"keep")
                .expect("late final data should be written");
            Ok(empty_extraction())
        })
        .expect_err("late final root must not be replaced");

        assert!(matches!(
            error,
            InstallationPlacementError::FinalRootExists(path) if path == final_root
        ));
        assert_eq!(
            std::fs::read(final_root.join("foreign-data")).expect("late data should remain"),
            b"keep"
        );
        assert!(staging_directories(final_root.parent().expect("final parent")).is_empty());
        std::fs::remove_dir_all(dir).expect("test directory should be removed");
    }

    #[test]
    fn placement_reports_lock_contention_without_creating_staging() {
        let dir = unique_test_dir("placement-lock");
        let final_root = dir.join("installations").join("i-lock");
        let parent = final_root.parent().expect("final parent");
        let _held_lock = OperationLock::acquire(&final_root)
            .expect("first operation should acquire lock and create its parent");

        let error = OperationLock::acquire(&final_root)
            .expect_err("second operation should observe contention");

        assert!(matches!(
            error,
            InstallationPlacementError::Busy(path) if path == final_root
        ));
        assert!(!final_root.exists());
        assert!(staging_directories(parent).is_empty());
        std::fs::remove_dir_all(dir).expect("test directory should be removed");
    }

    #[test]
    fn placement_rejects_lease_for_a_different_final_root() {
        let dir = unique_test_dir("placement-lock-root-mismatch");
        let locked_root = dir.join("installations").join("i-locked");
        let requested_root = dir.join("installations").join("i-requested");
        let parent = locked_root.parent().expect("final parent");
        let lock = operation_lock(&locked_root);

        let error = place_with_extractor(&requested_root, lock, |_| Ok(empty_extraction()))
            .expect_err("lease for another final root must be rejected");

        assert!(matches!(
            error,
            InstallationPlacementError::OperationLockRootMismatch {
                locked_root: actual_locked,
                requested_root: actual_requested,
            } if actual_locked == locked_root && actual_requested == requested_root
        ));
        assert!(!requested_root.exists());
        assert!(staging_directories(parent).is_empty());
        std::fs::remove_dir_all(dir).expect("test directory should be removed");
    }

    #[test]
    fn managed_file_lease_removes_the_exact_created_candidate() {
        use std::io::Write as _;

        let dir = unique_test_dir("managed-file-lease-remove");
        let retained = dir.join("retained-packages");
        let capability =
            open_or_create_managed_directory(&retained).expect("retained directory capability");
        let candidate = retained.join("candidate.package");
        let lease = create_managed_direct_child_file(&capability, &candidate)
            .expect("managed candidate lease");
        lease
            .download_file()
            .expect("download file clone")
            .write_all(b"partial")
            .expect("partial download");

        lease.remove_if_owned().expect("owned candidate removal");

        assert!(!candidate.exists());
        std::fs::remove_dir_all(dir).expect("test directory should be removed");
    }

    #[cfg(unix)]
    #[test]
    fn managed_file_lease_preserves_a_replacement_candidate() {
        let dir = unique_test_dir("managed-file-lease-replacement");
        let retained = dir.join("retained-packages");
        let capability =
            open_or_create_managed_directory(&retained).expect("retained directory capability");
        let candidate = retained.join("candidate.package");
        let original = retained.join("original.package");
        let lease = create_managed_direct_child_file(&capability, &candidate)
            .expect("managed candidate lease");
        std::fs::rename(&candidate, &original).expect("move leased candidate");
        std::fs::write(&candidate, b"replacement").expect("replacement candidate");

        lease
            .remove_if_owned()
            .expect_err("identity mismatch must preserve replacement");

        assert_eq!(
            std::fs::read(&candidate).expect("replacement survives"),
            b"replacement"
        );
        std::fs::remove_dir_all(dir).expect("test directory should be removed");
    }
}
