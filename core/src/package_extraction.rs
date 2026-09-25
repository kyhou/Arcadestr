//! Verified, policy-gated package extraction into a fresh staging directory.

use std::{
    collections::{BTreeMap, BTreeSet},
    error::Error as _,
    fs::{self, File, OpenOptions},
    io::{self, Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    sync::Arc,
};

use sha2::{Digest, Sha256};
use tar::Archive;
use thiserror::Error;
use unicode_normalization::UnicodeNormalization;
use zip::read::HasZipMetadata;

#[cfg(unix)]
use cap_std::fs::OpenOptionsExt as _;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt as _;

use crate::{
    adp_storage::{
        InstallationManifestEntry, InstallationProfile, ManifestEntryType, ProfileAvailability,
        ProfileUnavailableReason,
    },
    archive_inspection::{
        self, decode_entry_name, ArchiveHazards, ArchiveInspection, DetectInspectError, EntryKind,
    },
    distribution::{LaunchProfile, PackageFormat, RelativePath, Sha256Digest},
};

const COPY_BUFFER_BYTES: usize = 64 * 1024;
const ZIP_EFS_FLAG: u16 = 1 << 11;

/// A retained package handle whose digest, detected format, and full archive
/// policy inspection have all succeeded.
#[derive(Debug)]
pub struct VerifiedPackage {
    file: Arc<File>,
    digest: Sha256Digest,
    format: PackageFormat,
    declared_regular_file_size_bytes: u64,
}

impl VerifiedPackage {
    /// Verifies and inspects a retained handle without creating filesystem output.
    pub fn verify(
        file: Arc<File>,
        expected_digest: Sha256Digest,
        expected_format: PackageFormat,
    ) -> Result<Self, ExtractionError> {
        let inspection = preflight(&file, &expected_digest, &expected_format)?;
        let declared_regular_file_size_bytes = declared_regular_file_size(&inspection)?;
        Ok(Self {
            file,
            digest: expected_digest,
            format: expected_format,
            declared_regular_file_size_bytes,
        })
    }

    #[must_use]
    pub fn digest(&self) -> &Sha256Digest {
        &self.digest
    }

    #[must_use]
    pub fn format(&self) -> &PackageFormat {
        &self.format
    }

    /// Advisory sum of container-declared regular-file sizes from the
    /// successful inspection. Extraction still enforces measured accounting.
    #[must_use]
    pub fn declared_regular_file_size_bytes(&self) -> u64 {
        self.declared_regular_file_size_bytes
    }
}

/// Filesystem-independent extraction products. The caller owns promotion or
/// persistence of the staging root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtractionOutcome {
    pub manifest: Vec<InstallationManifestEntry>,
    pub profiles: Vec<InstallationProfile>,
}

/// A typed failure from verification, policy preflight, extraction, or cleanup.
#[derive(Debug, Error)]
pub enum ExtractionError {
    #[error("package digest mismatch: expected {expected}, got {actual}")]
    DigestMismatch { expected: String, actual: String },
    #[error("archive detection or inspection failed: {0:?}")]
    Inspection(DetectInspectError),
    #[error("archive extraction policy failed: {0}")]
    Policy(archive_inspection::InspectionError),
    #[error("package format mismatch: expected {expected:?}, detected {detected:?}")]
    FormatMismatch {
        expected: PackageFormat,
        detected: PackageFormat,
    },
    #[error("archive contains blocking hazards: {0:?}")]
    BlockingHazards(ArchiveHazards),
    #[error("sum of archive-declared regular-file sizes overflowed u64")]
    DeclaredRegularFileSizeOverflow,
    #[error("staging root already exists: {0}")]
    StagingRootExists(PathBuf),
    #[error("invalid archive path {path:?}: {message}")]
    InvalidPath { path: String, message: String },
    #[error("archive path occurs more than once: {0}")]
    DuplicatePath(String),
    #[error("archive paths collide without case: {first} and {second}")]
    CaseCollision { first: String, second: String },
    #[error("archive file/directory ancestry conflict at {0}")]
    AncestryConflict(String),
    #[error("archive contains unsupported entry type at {0}")]
    UnsupportedEntryType(String),
    #[error("archive entry name is not valid UTF-8: {0}")]
    UndecodablePath(String),
    #[error("ZIP error: {0}")]
    Zip(#[from] zip::result::ZipError),
    #[error("I/O error while {operation} {path}: {source}")]
    Io {
        operation: &'static str,
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("could not encode extracted SHA-256 digest: {0}")]
    InvalidDigest(String),
    #[error("extraction failed ({extraction}) and staging cleanup failed: {cleanup}")]
    Cleanup {
        extraction: String,
        extraction_insufficient_storage: bool,
        #[source]
        cleanup: io::Error,
    },
}

impl ExtractionError {
    #[must_use]
    pub fn is_insufficient_storage(&self) -> bool {
        match self {
            Self::Io { source, .. } => is_enospc(source),
            Self::Cleanup {
                extraction_insufficient_storage,
                cleanup,
                ..
            } => *extraction_insufficient_storage || is_enospc(cleanup),
            _ => self
                .source()
                .and_then(|source| source.downcast_ref::<io::Error>())
                .is_some_and(is_enospc),
        }
    }
}

fn is_enospc(error: &io::Error) -> bool {
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

/// Extracts a verified package into an absent caller-selected staging root.
///
/// The retained handle is reverified and fully reinspected before the root is
/// created. Once creation succeeds, every later error recursively removes the
/// owned staging root.
pub fn extract_verified_package(
    package: &VerifiedPackage,
    staging_root: &Path,
    launch_profiles: &[LaunchProfile],
) -> Result<ExtractionOutcome, ExtractionError> {
    ensure_absent(staging_root)?;

    // This second pass is deliberately adjacent to extraction. It both limits
    // the verification-to-use window and ensures policy rejection precedes output.
    let inspection = preflight(&package.file, &package.digest, &package.format)?;
    validate_inspected_tree(&inspection)?;
    // Rehash after the streaming inspection so the final verification is
    // immediately adjacent to creation and extraction.
    verify_digest(&package.file, &package.digest)?;

    create_staging_root(staging_root)?;
    let result = extract_after_creation(package, staging_root, launch_profiles);
    match result {
        Ok(outcome) => Ok(outcome),
        Err(error) => match fs::remove_dir_all(staging_root) {
            Ok(()) => Err(error),
            Err(cleanup) => Err(ExtractionError::Cleanup {
                extraction_insufficient_storage: error.is_insufficient_storage(),
                extraction: error.to_string(),
                cleanup,
            }),
        },
    }
}

/// Capability-rooted extraction result. Directory handles are retained through
/// extraction so Windows directories cannot be replaced by junctions or other
/// reparse points while they are used as output parents.
#[derive(Debug)]
pub struct CapabilityExtraction {
    pub outcome: ExtractionOutcome,
    _directories: BTreeMap<String, Arc<cap_std::fs::Dir>>,
}

impl CapabilityExtraction {
    /// Clones the held staging root for platform file-identity inspection.
    pub fn root_file(&self) -> io::Result<File> {
        self._directories
            .get("")
            .ok_or_else(|| io::Error::other("staging root capability is unavailable"))?
            .try_clone()
            .map(cap_std::fs::Dir::into_std_file)
    }
}

/// Extracts into an already-created staging directory capability.
///
/// Every descendant is created relative to a retained directory handle. Each
/// newly created directory is reopened without following a symlink/reparse
/// point before it can be used as a parent.
pub fn extract_verified_package_into_capability(
    package: &VerifiedPackage,
    staging_root: cap_std::fs::Dir,
    staging_display_path: &Path,
    launch_profiles: &[LaunchProfile],
) -> Result<CapabilityExtraction, ExtractionError> {
    let inspection = preflight(&package.file, &package.digest, &package.format)?;
    validate_inspected_tree(&inspection)?;
    verify_digest(&package.file, &package.digest)?;

    let mut target = CapabilityTarget::new(
        staging_root,
        staging_display_path.to_owned(),
        launch_profiles,
    )?;
    let mut manifest = Vec::new();
    let mut tree = TreeState::default();
    match package.format.clone() {
        PackageFormat::Zip => extract_zip_capability(
            PositionalReader::new(Arc::clone(&package.file))?,
            &mut target,
            &mut tree,
            &mut manifest,
        )?,
        PackageFormat::TarGz => {
            let reader = PositionalReader::new(Arc::clone(&package.file))?;
            let (reader, accounting) = archive_inspection::extraction_tar_gz_reader(reader);
            extract_tar_capability(reader, accounting, &mut target, &mut tree, &mut manifest)?;
        }
        PackageFormat::TarZstd => {
            let reader = PositionalReader::new(Arc::clone(&package.file))?;
            let (reader, accounting) = archive_inspection::extraction_tar_zstd_reader(reader)
                .map_err(ExtractionError::Policy)?;
            extract_tar_capability(reader, accounting, &mut target, &mut tree, &mut manifest)?;
        }
    }

    target.apply_exact_executable_permissions(launch_profiles)?;
    let profiles = validate_profiles_from_tree(&tree, launch_profiles);
    Ok(CapabilityExtraction {
        outcome: ExtractionOutcome { manifest, profiles },
        _directories: target.directories,
    })
}

struct CapabilityTarget {
    display_root: PathBuf,
    directories: BTreeMap<String, Arc<cap_std::fs::Dir>>,
    profile_executable_paths: BTreeSet<String>,
    profile_executables: BTreeMap<String, File>,
}

impl CapabilityTarget {
    fn new(
        root: cap_std::fs::Dir,
        display_root: PathBuf,
        profiles: &[LaunchProfile],
    ) -> Result<Self, ExtractionError> {
        #[cfg(unix)]
        root.set_permissions(
            ".",
            cap_std::fs::Permissions::from_std(fs::Permissions::from_mode(0o700)),
        )
        .map_err(|source| io_error("restricting staging root", &display_root, source))?;
        let mut directories = BTreeMap::new();
        directories.insert(String::new(), Arc::new(root));
        Ok(Self {
            display_root,
            directories,
            profile_executable_paths: profiles
                .iter()
                .map(|profile| profile.executable.as_str().to_owned())
                .collect(),
            profile_executables: BTreeMap::new(),
        })
    }

    fn parent_and_name<'a>(&self, relative: &'a str) -> (&'a str, &'a str) {
        relative.rsplit_once('/').unwrap_or(("", relative))
    }

    fn parent<'directory, 'path>(
        &'directory self,
        relative: &'path str,
    ) -> Result<(&'directory Arc<cap_std::fs::Dir>, &'path str), ExtractionError> {
        let (parent, name) = self.parent_and_name(relative);
        self.directories
            .get(parent)
            .map(|directory| (directory, name))
            .ok_or_else(|| ExtractionError::AncestryConflict(relative.to_owned()))
    }

    fn create_directory(&mut self, path: &RelativePath) -> Result<(), ExtractionError> {
        let relative = path.as_str();
        let (parent, name) = self.parent(relative)?;
        let display = self.display_root.join(relative);
        parent
            .create_dir(name)
            .map_err(|source| io_error("creating extracted directory", &display, source))?;
        #[cfg(unix)]
        parent
            .set_permissions(
                name,
                cap_std::fs::Permissions::from_std(fs::Permissions::from_mode(0o700)),
            )
            .map_err(|source| io_error("restricting extracted directory", &display, source))?;
        let parent_file = parent
            .try_clone()
            .map_err(|source| io_error("cloning extraction parent capability", &display, source))?
            .into_std_file();
        let opened = cap_primitives::fs::open_dir_nofollow(&parent_file, Path::new(name)).map_err(
            |source| {
                io_error(
                    "opening extracted directory without following",
                    &display,
                    source,
                )
            },
        )?;
        self.directories.insert(
            relative.to_owned(),
            Arc::new(cap_std::fs::Dir::from_std_file(opened)),
        );
        Ok(())
    }

    fn create_file(&self, path: &RelativePath) -> Result<File, ExtractionError> {
        let relative = path.as_str();
        let (parent, name) = self.parent(relative)?;
        let display = self.display_root.join(relative);
        let mut options = cap_std::fs::OpenOptions::new();
        options.read(true).write(true).create_new(true);
        #[cfg(unix)]
        options.mode(0o600);
        parent
            .open_with(name, &options)
            .map(cap_std::fs::File::into_std)
            .map_err(|source| io_error("creating extracted file", &display, source))
    }

    fn retain_profile_executable(&mut self, path: &RelativePath, file: File) {
        if self.profile_executable_paths.contains(path.as_str()) {
            self.profile_executables
                .insert(path.as_str().to_owned(), file);
        }
    }

    fn apply_exact_executable_permissions(
        &self,
        profiles: &[LaunchProfile],
    ) -> Result<(), ExtractionError> {
        #[cfg(windows)]
        let _ = profiles;
        #[cfg(unix)]
        {
            for relative in profiles
                .iter()
                .map(|profile| profile.executable.as_str())
                .collect::<std::collections::BTreeSet<_>>()
            {
                if let Some(file) = self.profile_executables.get(relative) {
                    file.set_permissions(fs::Permissions::from_mode(0o700))
                        .map_err(|source| {
                            io_error(
                                "marking profile executable",
                                &self.display_root.join(relative),
                                source,
                            )
                        })?;
                }
            }
        }
        Ok(())
    }
}

fn extract_zip_capability<R: Read + Seek>(
    reader: R,
    target: &mut CapabilityTarget,
    tree: &mut TreeState,
    manifest: &mut Vec<InstallationManifestEntry>,
) -> Result<(), ExtractionError> {
    let (mut archive, mut accounting) =
        archive_inspection::extraction_zip_archive(reader).map_err(ExtractionError::Policy)?;
    for index in 0..archive.len() {
        let mut entry = archive_inspection::extraction_zip_entry(&mut archive, index, &accounting)
            .map_err(ExtractionError::Policy)?;
        let metadata = entry.get_metadata();
        let kind =
            archive_inspection::zip_entry_kind(metadata.external_attributes, entry.unix_mode());
        let decoded = archive_inspection::decode_zip_name(
            entry.name_raw(),
            metadata.flags & ZIP_EFS_FLAG != 0,
        )
        .ok_or_else(|| ExtractionError::UndecodablePath(entry.name().to_owned()))?;
        let path = parse_entry_name(&decoded, &kind)?;
        let new_directories = tree.register(&path, &kind)?;
        create_capability_directories(target, new_directories, manifest)?;
        match kind {
            EntryKind::Directory => {}
            EntryKind::File => {
                let unix_mode = entry.unix_mode();
                accounting
                    .begin_regular_entry()
                    .map_err(ExtractionError::Policy)?;
                let mut output = target.create_file(&path)?;
                let mut hasher = Sha256::new();
                let mut buffer = [0_u8; COPY_BUFFER_BYTES];
                loop {
                    let read = accounting
                        .read_regular_output(&mut entry, &mut buffer)
                        .map_err(ExtractionError::Policy)?;
                    if read == 0 {
                        break;
                    }
                    output.write_all(&buffer[..read]).map_err(|source| {
                        io_error(
                            "writing extracted file",
                            &target.display_root.join(path.as_str()),
                            source,
                        )
                    })?;
                    hasher.update(&buffer[..read]);
                }
                finish_capability_file(target, output, path, hasher, unix_mode, manifest)?;
            }
            EntryKind::Symlink | EntryKind::HardLink | EntryKind::Other => {
                return Err(ExtractionError::UnsupportedEntryType(decoded));
            }
        }
    }
    Ok(())
}

fn extract_tar_capability<R: Read>(
    reader: R,
    mut accounting: archive_inspection::TarExtractionAccounting,
    target: &mut CapabilityTarget,
    tree: &mut TreeState,
    manifest: &mut Vec<InstallationManifestEntry>,
) -> Result<(), ExtractionError> {
    let mut archive = Archive::new(reader);
    {
        let entries = archive.entries().map_err(|source| {
            ExtractionError::Policy(archive_inspection::map_tar_extraction_io(source))
        })?;
        for entry in entries {
            let mut entry = entry.map_err(|source| {
                ExtractionError::Policy(archive_inspection::map_tar_extraction_io(source))
            })?;
            accounting.begin_entry().map_err(ExtractionError::Policy)?;
            let kind = archive_inspection::tar_entry_kind(entry.header().entry_type().as_byte());
            let raw_bytes = entry.path_bytes();
            let decoded = std::str::from_utf8(raw_bytes.as_ref())
                .map_err(|_| {
                    ExtractionError::UndecodablePath(String::from_utf8_lossy(&raw_bytes).into())
                })?
                .to_owned();
            let path = parse_entry_name(&decoded, &kind)?;
            let new_directories = tree.register(&path, &kind)?;
            create_capability_directories(target, new_directories, manifest)?;
            match kind {
                EntryKind::Directory => {}
                EntryKind::File => {
                    let unix_mode = entry
                        .header()
                        .mode()
                        .map_err(archive_inspection::map_tar_extraction_io)
                        .map_err(ExtractionError::Policy)?;
                    let mut output = target.create_file(&path)?;
                    let mut hasher = Sha256::new();
                    accounting
                        .stream_regular_payload(&mut entry, |buffer| {
                            output.write_all(buffer)?;
                            hasher.update(buffer);
                            Ok(())
                        })
                        .map_err(|error| match error {
                            archive_inspection::TarExtractionStreamError::Policy(error) => {
                                ExtractionError::Policy(error)
                            }
                            archive_inspection::TarExtractionStreamError::Output(source) => {
                                io_error(
                                    "writing extracted file",
                                    &target.display_root.join(path.as_str()),
                                    source,
                                )
                            }
                        })?;
                    finish_capability_file(
                        target,
                        output,
                        path,
                        hasher,
                        Some(unix_mode),
                        manifest,
                    )?;
                }
                EntryKind::Symlink | EntryKind::HardLink | EntryKind::Other => {
                    return Err(ExtractionError::UnsupportedEntryType(decoded));
                }
            }
        }
    }
    let mut decoder = archive.into_inner();
    accounting
        .finish(&mut decoder)
        .map_err(ExtractionError::Policy)
}

fn create_capability_directories(
    target: &mut CapabilityTarget,
    directories: Vec<RelativePath>,
    manifest: &mut Vec<InstallationManifestEntry>,
) -> Result<(), ExtractionError> {
    for directory in directories {
        target.create_directory(&directory)?;
        manifest.push(InstallationManifestEntry {
            path: directory,
            entry_type: ManifestEntryType::Directory,
            file_digest: None,
        });
    }
    Ok(())
}

fn finish_capability_file(
    target: &mut CapabilityTarget,
    mut output: File,
    path: RelativePath,
    hasher: Sha256,
    unix_mode: Option<u32>,
    manifest: &mut Vec<InstallationManifestEntry>,
) -> Result<(), ExtractionError> {
    output.flush().map_err(|source| {
        io_error(
            "flushing extracted file",
            &target.display_root.join(path.as_str()),
            source,
        )
    })?;
    let digest_text = format!("{:x}", hasher.finalize());
    let file_digest = Sha256Digest::parse(&digest_text)
        .map_err(|error| ExtractionError::InvalidDigest(error.to_string()))?;
    preserve_executable_permissions(&output, &target.display_root.join(path.as_str()), unix_mode)?;
    target.retain_profile_executable(&path, output);
    manifest.push(InstallationManifestEntry {
        path,
        entry_type: ManifestEntryType::File,
        file_digest: Some(file_digest),
    });
    Ok(())
}

fn validate_profiles_from_tree(
    tree: &TreeState,
    profiles: &[LaunchProfile],
) -> Vec<InstallationProfile> {
    profiles
        .iter()
        .cloned()
        .map(|profile| {
            let executable = tree_node_for_platform_path(tree, profile.executable.as_str());
            let availability = if executable.is_none() {
                ProfileAvailability::Unavailable(ProfileUnavailableReason::ExecutableMissing)
            } else if !matches!(executable.map(|node| node.kind), Some(TreeKind::File)) {
                ProfileAvailability::Unavailable(ProfileUnavailableReason::ExecutableNotFile)
            } else if !profile.working_dir.is_root()
                && tree_node_for_platform_path(tree, profile.working_dir.as_str()).is_none()
            {
                ProfileAvailability::Unavailable(ProfileUnavailableReason::WorkingDirectoryMissing)
            } else if !profile.working_dir.is_root()
                && !matches!(
                    tree_node_for_platform_path(tree, profile.working_dir.as_str())
                        .map(|node| node.kind),
                    Some(TreeKind::Directory)
                )
            {
                ProfileAvailability::Unavailable(
                    ProfileUnavailableReason::WorkingDirectoryNotDirectory,
                )
            } else {
                ProfileAvailability::Available
            };
            InstallationProfile {
                profile,
                availability,
            }
        })
        .collect()
}

fn tree_node_for_platform_path<'a>(tree: &'a TreeState, path: &str) -> Option<&'a TreeNode> {
    let node = tree.nodes.get(&manifest_path_key(path))?;
    #[cfg(not(windows))]
    if node.path != path {
        return None;
    }
    Some(node)
}

fn extract_after_creation(
    package: &VerifiedPackage,
    staging_root: &Path,
    launch_profiles: &[LaunchProfile],
) -> Result<ExtractionOutcome, ExtractionError> {
    let mut manifest = Vec::new();
    let mut tree = TreeState::default();
    match package.format.clone() {
        PackageFormat::Zip => extract_zip(
            PositionalReader::new(Arc::clone(&package.file))?,
            staging_root,
            &mut tree,
            &mut manifest,
        )?,
        PackageFormat::TarGz => {
            let reader = PositionalReader::new(Arc::clone(&package.file))?;
            let (reader, accounting) = archive_inspection::extraction_tar_gz_reader(reader);
            extract_tar(reader, accounting, staging_root, &mut tree, &mut manifest)?;
        }
        PackageFormat::TarZstd => {
            let reader = PositionalReader::new(Arc::clone(&package.file))?;
            let (reader, accounting) = archive_inspection::extraction_tar_zstd_reader(reader)
                .map_err(ExtractionError::Policy)?;
            extract_tar(reader, accounting, staging_root, &mut tree, &mut manifest)?;
        }
    }

    apply_exact_executable_permissions(staging_root, launch_profiles)?;
    let profiles = validate_profiles(staging_root, launch_profiles)?;
    Ok(ExtractionOutcome { manifest, profiles })
}

fn preflight(
    file: &Arc<File>,
    expected_digest: &Sha256Digest,
    expected_format: &PackageFormat,
) -> Result<ArchiveInspection, ExtractionError> {
    verify_digest(file, expected_digest)?;

    let reader = PositionalReader::new(Arc::clone(file))?;
    let (detected, inspection) = archive_inspection::inspect_detected_archive(reader)
        .map_err(ExtractionError::Inspection)?;
    if &detected != expected_format {
        return Err(ExtractionError::FormatMismatch {
            expected: expected_format.clone(),
            detected,
        });
    }
    if !inspection.hazards.is_empty() {
        return Err(ExtractionError::BlockingHazards(inspection.hazards));
    }
    Ok(inspection)
}

fn declared_regular_file_size(inspection: &ArchiveInspection) -> Result<u64, ExtractionError> {
    inspection
        .index
        .entries
        .iter()
        .filter(|entry| matches!(entry.kind, EntryKind::File))
        .try_fold(0_u64, |total, entry| {
            total
                .checked_add(entry.declared_size)
                .ok_or(ExtractionError::DeclaredRegularFileSizeOverflow)
        })
}

fn verify_digest(file: &Arc<File>, expected_digest: &Sha256Digest) -> Result<(), ExtractionError> {
    let actual = hash_reader(PositionalReader::new(Arc::clone(file))?)?;
    if actual == expected_digest.as_str() {
        return Ok(());
    }
    Err(ExtractionError::DigestMismatch {
        expected: expected_digest.as_str().to_owned(),
        actual,
    })
}

fn hash_reader(mut reader: impl Read) -> Result<String, ExtractionError> {
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; COPY_BUFFER_BYTES];
    loop {
        let read = reader
            .read(&mut buffer)
            .map_err(|source| ExtractionError::Io {
                operation: "verifying retained package",
                path: PathBuf::from("<retained-package>"),
                source,
            })?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn validate_inspected_tree(inspection: &ArchiveInspection) -> Result<(), ExtractionError> {
    let mut tree = TreeState::default();
    for entry in &inspection.index.entries {
        let path = parse_entry_name(&entry.raw_path, &entry.kind)?;
        tree.register(&path, &entry.kind)?;
    }
    Ok(())
}

fn extract_zip<R: Read + Seek>(
    reader: R,
    root: &Path,
    tree: &mut TreeState,
    manifest: &mut Vec<InstallationManifestEntry>,
) -> Result<(), ExtractionError> {
    let (mut archive, mut accounting) =
        archive_inspection::extraction_zip_archive(reader).map_err(ExtractionError::Policy)?;
    for index in 0..archive.len() {
        let mut entry = archive_inspection::extraction_zip_entry(&mut archive, index, &accounting)
            .map_err(ExtractionError::Policy)?;
        let metadata = entry.get_metadata();
        let kind =
            archive_inspection::zip_entry_kind(metadata.external_attributes, entry.unix_mode());
        let decoded = archive_inspection::decode_zip_name(
            entry.name_raw(),
            metadata.flags & ZIP_EFS_FLAG != 0,
        )
        .ok_or_else(|| ExtractionError::UndecodablePath(entry.name().to_owned()))?;
        let path = parse_entry_name(&decoded, &kind)?;
        let new_directories = tree.register(&path, &kind)?;
        create_registered_directories(root, new_directories, manifest)?;

        match kind {
            EntryKind::Directory => {}
            EntryKind::File => {
                let unix_mode = entry.unix_mode();
                accounting
                    .begin_regular_entry()
                    .map_err(ExtractionError::Policy)?;
                extract_regular_zip_file(
                    &mut entry,
                    &mut accounting,
                    root,
                    path,
                    unix_mode,
                    manifest,
                )?;
            }
            EntryKind::Symlink | EntryKind::HardLink | EntryKind::Other => {
                return Err(ExtractionError::UnsupportedEntryType(decoded));
            }
        }
    }
    Ok(())
}

fn extract_tar<R: Read>(
    reader: R,
    mut accounting: archive_inspection::TarExtractionAccounting,
    root: &Path,
    tree: &mut TreeState,
    manifest: &mut Vec<InstallationManifestEntry>,
) -> Result<(), ExtractionError> {
    let mut archive = Archive::new(reader);
    {
        let entries = archive.entries().map_err(|source| {
            ExtractionError::Policy(archive_inspection::map_tar_extraction_io(source))
        })?;
        for entry in entries {
            let mut entry = entry.map_err(|source| {
                ExtractionError::Policy(archive_inspection::map_tar_extraction_io(source))
            })?;
            accounting.begin_entry().map_err(ExtractionError::Policy)?;
            let raw_type = entry.header().entry_type().as_byte();
            let kind = archive_inspection::tar_entry_kind(raw_type);
            let raw_bytes = entry.path_bytes();
            let decoded = std::str::from_utf8(raw_bytes.as_ref())
                .map_err(|_| {
                    ExtractionError::UndecodablePath(String::from_utf8_lossy(&raw_bytes).into())
                })?
                .to_owned();
            let path = parse_entry_name(&decoded, &kind)?;
            let new_directories = tree.register(&path, &kind)?;
            create_registered_directories(root, new_directories, manifest)?;

            match kind {
                EntryKind::Directory => {}
                EntryKind::File => {
                    let unix_mode = entry
                        .header()
                        .mode()
                        .map_err(archive_inspection::map_tar_extraction_io)
                        .map_err(ExtractionError::Policy)?;
                    extract_regular_tar_file(
                        &mut entry,
                        &mut accounting,
                        root,
                        path,
                        Some(unix_mode),
                        manifest,
                    )?
                }
                EntryKind::Symlink | EntryKind::HardLink | EntryKind::Other => {
                    return Err(ExtractionError::UnsupportedEntryType(decoded));
                }
            }
        }
    }
    let mut decoder = archive.into_inner();
    accounting
        .finish(&mut decoder)
        .map_err(ExtractionError::Policy)
}

fn parse_entry_name(raw: &str, kind: &EntryKind) -> Result<RelativePath, ExtractionError> {
    let decoded = decode_entry_name(raw, kind);
    if decoded.conflict.is_some() {
        return Err(ExtractionError::UnsupportedEntryType(raw.to_owned()));
    }
    RelativePath::parse(&decoded.raw_path).map_err(|error| ExtractionError::InvalidPath {
        path: decoded.raw_path,
        message: error.to_string(),
    })
}

fn create_registered_directories(
    root: &Path,
    directories: Vec<RelativePath>,
    manifest: &mut Vec<InstallationManifestEntry>,
) -> Result<(), ExtractionError> {
    for directory in directories {
        let output = root.join(directory.as_str());
        create_directory(&output, "creating extracted directory")?;
        manifest.push(InstallationManifestEntry {
            path: directory,
            entry_type: ManifestEntryType::Directory,
            file_digest: None,
        });
    }
    Ok(())
}

fn extract_regular_zip_file(
    reader: &mut impl Read,
    accounting: &mut archive_inspection::ZipExtractionAccounting,
    root: &Path,
    path: RelativePath,
    unix_mode: Option<u32>,
    manifest: &mut Vec<InstallationManifestEntry>,
) -> Result<(), ExtractionError> {
    let output_path = root.join(path.as_str());
    let mut output = create_new_file(&output_path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; COPY_BUFFER_BYTES];
    loop {
        let read = accounting
            .read_regular_output(reader, &mut buffer)
            .map_err(ExtractionError::Policy)?;
        if read == 0 {
            break;
        }
        output
            .write_all(&buffer[..read])
            .map_err(|source| io_error("writing extracted file", &output_path, source))?;
        hasher.update(&buffer[..read]);
    }
    finish_manifest_file(output, output_path, path, hasher, unix_mode, manifest)
}

fn extract_regular_tar_file<R: Read>(
    entry: &mut tar::Entry<'_, R>,
    accounting: &mut archive_inspection::TarExtractionAccounting,
    root: &Path,
    path: RelativePath,
    unix_mode: Option<u32>,
    manifest: &mut Vec<InstallationManifestEntry>,
) -> Result<(), ExtractionError> {
    let output_path = root.join(path.as_str());
    let mut output = create_new_file(&output_path)?;
    let mut hasher = Sha256::new();
    accounting
        .stream_regular_payload(entry, |buffer| {
            output.write_all(buffer)?;
            hasher.update(buffer);
            Ok(())
        })
        .map_err(|error| match error {
            archive_inspection::TarExtractionStreamError::Policy(error) => {
                ExtractionError::Policy(error)
            }
            archive_inspection::TarExtractionStreamError::Output(source) => {
                io_error("writing extracted file", &output_path, source)
            }
        })?;
    finish_manifest_file(output, output_path, path, hasher, unix_mode, manifest)
}

fn finish_manifest_file(
    mut output: File,
    output_path: PathBuf,
    path: RelativePath,
    hasher: Sha256,
    unix_mode: Option<u32>,
    manifest: &mut Vec<InstallationManifestEntry>,
) -> Result<(), ExtractionError> {
    output
        .flush()
        .map_err(|source| io_error("flushing extracted file", &output_path, source))?;
    let digest_text = format!("{:x}", hasher.finalize());
    let file_digest = Sha256Digest::parse(&digest_text)
        .map_err(|error| ExtractionError::InvalidDigest(error.to_string()))?;
    preserve_executable_permissions(&output, &output_path, unix_mode)?;
    manifest.push(InstallationManifestEntry {
        path,
        entry_type: ManifestEntryType::File,
        file_digest: Some(file_digest),
    });
    Ok(())
}

#[cfg(unix)]
fn preserve_executable_permissions(
    file: &File,
    path: &Path,
    archive_mode: Option<u32>,
) -> Result<(), ExtractionError> {
    use std::os::unix::fs::PermissionsExt;

    let mode = 0o600 | (archive_mode.unwrap_or(0) & 0o111);
    file.set_permissions(fs::Permissions::from_mode(mode))
        .map_err(|source| io_error("preserving executable permissions", path, source))
}

#[cfg(not(unix))]
fn preserve_executable_permissions(
    _file: &File,
    _path: &Path,
    _archive_mode: Option<u32>,
) -> Result<(), ExtractionError> {
    Ok(())
}

fn ensure_absent(path: &Path) -> Result<(), ExtractionError> {
    match fs::symlink_metadata(path) {
        Ok(_) => Err(ExtractionError::StagingRootExists(path.to_owned())),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(io_error("checking staging root", path, source)),
    }
}

fn create_staging_root(path: &Path) -> Result<(), ExtractionError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::{DirBuilderExt, PermissionsExt};

        let mut builder = fs::DirBuilder::new();
        builder.mode(0o700);
        builder
            .create(path)
            .map_err(|source| io_error("creating staging root", path, source))?;
        if let Err(source) = fs::set_permissions(path, fs::Permissions::from_mode(0o700)) {
            let extraction = io_error("restricting staging root", path, source);
            return match fs::remove_dir_all(path) {
                Ok(()) => Err(extraction),
                Err(cleanup) => Err(ExtractionError::Cleanup {
                    extraction_insufficient_storage: extraction.is_insufficient_storage(),
                    extraction: extraction.to_string(),
                    cleanup,
                }),
            };
        }
    }
    #[cfg(not(unix))]
    fs::create_dir(path).map_err(|source| io_error("creating staging root", path, source))?;
    Ok(())
}

fn create_directory(path: &Path, operation: &'static str) -> Result<(), ExtractionError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::{DirBuilderExt, PermissionsExt};

        let mut builder = fs::DirBuilder::new();
        builder.mode(0o700);
        builder
            .create(path)
            .map_err(|source| io_error(operation, path, source))?;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))
            .map_err(|source| io_error("restricting extracted directory", path, source))?;
    }
    #[cfg(not(unix))]
    fs::create_dir(path).map_err(|source| io_error(operation, path, source))?;
    Ok(())
}

fn create_new_file(path: &Path) -> Result<File, ExtractionError> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = options
        .open(path)
        .map_err(|source| io_error("creating extracted file", path, source))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(fs::Permissions::from_mode(0o600))
            .map_err(|source| io_error("restricting extracted file", path, source))?;
    }
    Ok(file)
}

#[cfg(unix)]
fn apply_exact_executable_permissions(
    root: &Path,
    profiles: &[LaunchProfile],
) -> Result<(), ExtractionError> {
    use std::os::unix::fs::PermissionsExt;

    let mut paths = std::collections::BTreeSet::new();
    for profile in profiles {
        paths.insert(profile.executable.as_str());
    }
    for relative in paths {
        let path = root.join(relative);
        let metadata = match fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::NotFound | io::ErrorKind::NotADirectory
                ) =>
            {
                continue;
            }
            Err(source) => return Err(io_error("checking profile executable", &path, source)),
        };
        if metadata.file_type().is_file() {
            fs::set_permissions(&path, fs::Permissions::from_mode(0o700))
                .map_err(|source| io_error("marking profile executable", &path, source))?;
        }
    }
    Ok(())
}

#[cfg(not(unix))]
fn apply_exact_executable_permissions(
    _root: &Path,
    _profiles: &[LaunchProfile],
) -> Result<(), ExtractionError> {
    Ok(())
}

fn validate_profiles(
    root: &Path,
    profiles: &[LaunchProfile],
) -> Result<Vec<InstallationProfile>, ExtractionError> {
    let canonical_root = fs::canonicalize(root)
        .map_err(|source| io_error("canonicalizing staging root", root, source))?;
    profiles
        .iter()
        .cloned()
        .map(|profile| {
            let availability = profile_availability(&canonical_root, &profile)?;
            Ok(InstallationProfile {
                profile,
                availability,
            })
        })
        .collect()
}

fn profile_availability(
    canonical_root: &Path,
    profile: &LaunchProfile,
) -> Result<ProfileAvailability, ExtractionError> {
    let executable = canonical_root.join(profile.executable.as_str());
    let executable_metadata = match fs::symlink_metadata(&executable) {
        Ok(metadata) => metadata,
        Err(error)
            if matches!(
                error.kind(),
                io::ErrorKind::NotFound | io::ErrorKind::NotADirectory
            ) =>
        {
            return Ok(ProfileAvailability::Unavailable(
                ProfileUnavailableReason::ExecutableMissing,
            ));
        }
        Err(source) => return Err(io_error("checking profile executable", &executable, source)),
    };
    if !executable_metadata.file_type().is_file() {
        return Ok(ProfileAvailability::Unavailable(
            ProfileUnavailableReason::ExecutableNotFile,
        ));
    }
    let canonical_executable = fs::canonicalize(&executable)
        .map_err(|source| io_error("canonicalizing profile executable", &executable, source))?;
    if !canonical_executable.starts_with(canonical_root) {
        return Ok(ProfileAvailability::Unavailable(
            ProfileUnavailableReason::OutsideInstallationRoot,
        ));
    }

    let working_directory = if profile.working_dir.is_root() {
        canonical_root.to_owned()
    } else {
        canonical_root.join(profile.working_dir.as_str())
    };
    let working_metadata = match fs::symlink_metadata(&working_directory) {
        Ok(metadata) => metadata,
        Err(error)
            if matches!(
                error.kind(),
                io::ErrorKind::NotFound | io::ErrorKind::NotADirectory
            ) =>
        {
            return Ok(ProfileAvailability::Unavailable(
                ProfileUnavailableReason::WorkingDirectoryMissing,
            ));
        }
        Err(source) => {
            return Err(io_error(
                "checking profile working directory",
                &working_directory,
                source,
            ));
        }
    };
    if !working_metadata.file_type().is_dir() {
        return Ok(ProfileAvailability::Unavailable(
            ProfileUnavailableReason::WorkingDirectoryNotDirectory,
        ));
    }
    let canonical_working = fs::canonicalize(&working_directory).map_err(|source| {
        io_error(
            "canonicalizing profile working directory",
            &working_directory,
            source,
        )
    })?;
    if !canonical_working.starts_with(canonical_root) {
        return Ok(ProfileAvailability::Unavailable(
            ProfileUnavailableReason::OutsideInstallationRoot,
        ));
    }
    Ok(ProfileAvailability::Available)
}

fn io_error(operation: &'static str, path: &Path, source: io::Error) -> ExtractionError {
    ExtractionError::Io {
        operation,
        path: path.to_owned(),
        source,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TreeKind {
    File,
    Directory,
}

#[derive(Debug)]
struct TreeNode {
    path: String,
    kind: TreeKind,
    explicit: bool,
}

#[derive(Debug, Default)]
struct TreeState {
    nodes: BTreeMap<String, TreeNode>,
}

impl TreeState {
    fn register(
        &mut self,
        path: &RelativePath,
        kind: &EntryKind,
    ) -> Result<Vec<RelativePath>, ExtractionError> {
        let target_kind = match kind {
            EntryKind::File => TreeKind::File,
            EntryKind::Directory => TreeKind::Directory,
            EntryKind::Symlink | EntryKind::HardLink | EntryKind::Other => {
                return Err(ExtractionError::UnsupportedEntryType(
                    path.as_str().to_owned(),
                ));
            }
        };
        let mut created_directories = Vec::new();
        let mut prefix_end = 0;
        while let Some(relative) = path.as_str()[prefix_end..].find('/') {
            prefix_end += relative;
            let parent = &path.as_str()[..prefix_end];
            self.register_directory(parent, false, &mut created_directories)?;
            prefix_end += 1;
        }

        match target_kind {
            TreeKind::Directory => {
                self.register_directory(path.as_str(), true, &mut created_directories)?;
            }
            TreeKind::File => self.register_file(path.as_str())?,
        }
        Ok(created_directories)
    }

    fn register_directory(
        &mut self,
        path: &str,
        explicit: bool,
        created: &mut Vec<RelativePath>,
    ) -> Result<(), ExtractionError> {
        let key = manifest_path_key(path);
        if let Some(existing) = self.nodes.get_mut(&key) {
            if existing.path != path {
                return Err(ExtractionError::CaseCollision {
                    first: existing.path.clone(),
                    second: path.to_owned(),
                });
            }
            if existing.kind != TreeKind::Directory {
                return Err(ExtractionError::AncestryConflict(path.to_owned()));
            }
            if explicit && existing.explicit {
                return Err(ExtractionError::DuplicatePath(path.to_owned()));
            }
            existing.explicit |= explicit;
            return Ok(());
        }
        let relative = RelativePath::parse(path).map_err(|error| ExtractionError::InvalidPath {
            path: path.to_owned(),
            message: error.to_string(),
        })?;
        self.nodes.insert(
            key,
            TreeNode {
                path: path.to_owned(),
                kind: TreeKind::Directory,
                explicit,
            },
        );
        created.push(relative);
        Ok(())
    }

    fn register_file(&mut self, path: &str) -> Result<(), ExtractionError> {
        let key = manifest_path_key(path);
        if let Some(existing) = self.nodes.get(&key) {
            if existing.path != path {
                return Err(ExtractionError::CaseCollision {
                    first: existing.path.clone(),
                    second: path.to_owned(),
                });
            }
            return if existing.kind == TreeKind::File && existing.explicit {
                Err(ExtractionError::DuplicatePath(path.to_owned()))
            } else {
                Err(ExtractionError::AncestryConflict(path.to_owned()))
            };
        }
        self.nodes.insert(
            key,
            TreeNode {
                path: path.to_owned(),
                kind: TreeKind::File,
                explicit: true,
            },
        );
        Ok(())
    }
}

fn manifest_path_key(path: &str) -> String {
    path.chars()
        .flat_map(char::to_uppercase)
        .flat_map(char::to_lowercase)
        .collect::<String>()
        .nfc()
        .collect()
}

#[derive(Debug)]
struct PositionalReader {
    file: Arc<File>,
    position: u64,
    length: u64,
}

impl PositionalReader {
    fn new(file: Arc<File>) -> Result<Self, ExtractionError> {
        let length = file
            .metadata()
            .map_err(|source| {
                io_error(
                    "reading retained package metadata",
                    Path::new("<retained-package>"),
                    source,
                )
            })?
            .len();
        Ok(Self {
            file,
            position: 0,
            length,
        })
    }
}

impl Read for PositionalReader {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let read = read_file_at(&self.file, buffer, self.position)?;
        self.position = self
            .position
            .checked_add(read as u64)
            .ok_or_else(|| io::Error::other("positional package offset overflowed"))?;
        Ok(read)
    }
}

impl Seek for PositionalReader {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        let next = match position {
            SeekFrom::Start(offset) => i128::from(offset),
            SeekFrom::End(offset) => i128::from(self.length) + i128::from(offset),
            SeekFrom::Current(offset) => i128::from(self.position) + i128::from(offset),
        };
        self.position = u64::try_from(next)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "invalid package seek"))?;
        Ok(self.position)
    }
}

#[cfg(unix)]
fn read_file_at(file: &File, buffer: &mut [u8], offset: u64) -> io::Result<usize> {
    use std::os::unix::fs::FileExt;

    file.read_at(buffer, offset)
}

#[cfg(windows)]
fn read_file_at(file: &File, buffer: &mut [u8], offset: u64) -> io::Result<usize> {
    use std::os::windows::fs::FileExt;

    file.seek_read(buffer, offset)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    use flate2::{write::GzEncoder, Compression};
    use tar::{Builder, EntryType, Header};
    use tempfile::TempDir;
    use zip::{write::SimpleFileOptions, CompressionMethod, ZipWriter};

    #[derive(Clone, Copy)]
    enum TestEntry<'a> {
        File(&'a str, &'a [u8]),
        Executable(&'a str, &'a [u8]),
        Directory(&'a str),
        Symlink(&'a str, &'a str),
        HardLink(&'a str, &'a str),
    }

    fn digest(bytes: &[u8]) -> Sha256Digest {
        Sha256Digest::parse(&format!("{:x}", Sha256::digest(bytes))).expect("test digest")
    }

    fn write_package(temp: &TempDir, name: &str, bytes: &[u8]) -> Arc<File> {
        let path = temp.path().join(name);
        fs::write(&path, bytes).expect("write package");
        Arc::new(File::open(path).expect("open package"))
    }

    fn zip_bytes(entries: &[TestEntry<'_>]) -> Vec<u8> {
        let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
        let options = SimpleFileOptions::default()
            .compression_method(CompressionMethod::Deflated)
            .last_modified_time(zip::DateTime::default());
        for entry in entries {
            match entry {
                TestEntry::File(path, bytes) => {
                    writer.start_file(*path, options).expect("start ZIP file");
                    writer.write_all(bytes).expect("write ZIP file");
                }
                TestEntry::Executable(path, bytes) => {
                    writer
                        .start_file(*path, options.unix_permissions(0o7755))
                        .expect("start executable ZIP file");
                    writer.write_all(bytes).expect("write executable ZIP file");
                }
                TestEntry::Directory(path) => writer
                    .add_directory(*path, options)
                    .expect("add ZIP directory"),
                TestEntry::Symlink(path, target) => writer
                    .add_symlink(*path, *target, options)
                    .expect("add ZIP symlink"),
                TestEntry::HardLink(_, _) => panic!("ZIP test hard links are unsupported"),
            }
        }
        writer.finish().expect("finish ZIP").into_inner()
    }

    fn append_tar_entries<W: Write>(builder: &mut Builder<W>, entries: &[TestEntry<'_>]) {
        for entry in entries {
            let (path, kind, bytes, link, mode) = match entry {
                TestEntry::File(path, bytes) => (*path, EntryType::Regular, *bytes, None, 0o644),
                TestEntry::Executable(path, bytes) => {
                    (*path, EntryType::Regular, *bytes, None, 0o7755)
                }
                TestEntry::Directory(path) => (*path, EntryType::Directory, &[][..], None, 0o755),
                TestEntry::Symlink(path, target) => {
                    (*path, EntryType::Symlink, &[][..], Some(*target), 0o777)
                }
                TestEntry::HardLink(path, target) => {
                    (*path, EntryType::Link, &[][..], Some(*target), 0o644)
                }
            };
            let mut header = Header::new_gnu();
            header.set_entry_type(kind);
            header.set_mode(mode);
            header.set_uid(0);
            header.set_gid(0);
            header.set_mtime(0);
            header.set_size(bytes.len() as u64);
            if let Some(target) = link {
                header.set_link_name(target).expect("set tar link target");
            }
            header.set_cksum();
            builder
                .append_data(&mut header, path, bytes)
                .expect("append tar entry");
        }
        builder.finish().expect("finish tar");
    }

    fn tar_gz_bytes(entries: &[TestEntry<'_>]) -> Vec<u8> {
        let encoder = GzEncoder::new(Vec::new(), Compression::default());
        let mut builder = Builder::new(encoder);
        append_tar_entries(&mut builder, entries);
        builder
            .into_inner()
            .expect("tar gzip encoder")
            .finish()
            .expect("gzip finish")
    }

    fn tar_zstd_bytes(entries: &[TestEntry<'_>]) -> Vec<u8> {
        let encoder = zstd::stream::write::Encoder::new(Vec::new(), 1).expect("zstd encoder");
        let mut builder = Builder::new(encoder);
        append_tar_entries(&mut builder, entries);
        builder
            .into_inner()
            .expect("tar zstd encoder")
            .finish()
            .expect("zstd finish")
    }

    fn extract_test_package(
        bytes: &[u8],
        format: PackageFormat,
        profiles: &[LaunchProfile],
    ) -> (TempDir, PathBuf, ExtractionOutcome) {
        let temp = TempDir::new().expect("temp dir");
        let package = VerifiedPackage::verify(
            write_package(&temp, "package", bytes),
            digest(bytes),
            format,
        )
        .expect("verify package");
        let root = temp.path().join("stage");
        let outcome = extract_verified_package(&package, &root, profiles).expect("extract package");
        (temp, root, outcome)
    }

    #[test]
    fn extracts_valid_zip_tar_gz_and_tar_zstd() {
        let entries = [
            TestEntry::Directory("game/"),
            TestEntry::File("game/run.bin", b"game"),
        ];
        for (bytes, format) in [
            (zip_bytes(&entries), PackageFormat::Zip),
            (tar_gz_bytes(&entries), PackageFormat::TarGz),
            (tar_zstd_bytes(&entries), PackageFormat::TarZstd),
        ] {
            let (_temp, root, outcome) = extract_test_package(&bytes, format, &[]);
            assert_eq!(fs::read(root.join("game/run.bin")).expect("read"), b"game");
            assert_eq!(outcome.manifest.len(), 2);
        }
    }

    #[test]
    fn verified_package_exposes_declared_regular_file_size_sum() {
        let entries = [
            TestEntry::Directory("game/"),
            TestEntry::File("game/run.bin", b"game"),
            TestEntry::File("game/data.bin", b"metadata"),
        ];
        for (bytes, format) in [
            (zip_bytes(&entries), PackageFormat::Zip),
            (tar_gz_bytes(&entries), PackageFormat::TarGz),
            (tar_zstd_bytes(&entries), PackageFormat::TarZstd),
        ] {
            let temp = TempDir::new().expect("temp dir");
            let package = VerifiedPackage::verify(
                write_package(&temp, "package", &bytes),
                digest(&bytes),
                format,
            )
            .expect("package should verify");

            assert_eq!(
                package.declared_regular_file_size_bytes(),
                (b"game".len() + b"metadata".len()) as u64
            );
        }
    }

    #[test]
    fn declared_regular_file_size_overflow_is_typed() {
        use crate::archive_inspection::{
            ArchiveEntry, ArchiveHazards, ArchiveIndex, ArchiveWarnings,
        };

        let inspection = ArchiveInspection {
            index: ArchiveIndex {
                format: PackageFormat::Zip,
                entries: vec![
                    ArchiveEntry {
                        raw_path: "first".to_string(),
                        kind: EntryKind::File,
                        declared_size: u64::MAX,
                        unix_mode: None,
                    },
                    ArchiveEntry {
                        raw_path: "second".to_string(),
                        kind: EntryKind::File,
                        declared_size: 1,
                        unix_mode: None,
                    },
                ],
            },
            hazards: ArchiveHazards::default(),
            warnings: ArchiveWarnings::default(),
        };

        assert!(matches!(
            declared_regular_file_size(&inspection),
            Err(ExtractionError::DeclaredRegularFileSizeOverflow)
        ));
    }

    #[test]
    fn cleanup_failure_preserves_primary_insufficient_storage_classification() {
        let error = ExtractionError::Cleanup {
            extraction: "write failed".to_string(),
            extraction_insufficient_storage: true,
            cleanup: std::io::Error::other("cleanup failed"),
        };

        assert!(error.is_insufficient_storage());
    }

    #[test]
    fn rejects_traversal_absolute_and_backslash_before_staging_creation() {
        for hostile in ["../escape", "/absolute", "dir\\escape"] {
            let bytes = zip_bytes(&[TestEntry::File(hostile, b"bad")]);
            let temp = TempDir::new().expect("temp dir");
            let root = temp.path().join("stage");
            let result = VerifiedPackage::verify(
                write_package(&temp, "bad.zip", &bytes),
                digest(&bytes),
                PackageFormat::Zip,
            );
            assert!(matches!(result, Err(ExtractionError::BlockingHazards(_))));
            assert!(!root.exists());
        }
    }

    #[test]
    fn rejects_symlink_and_hardlink_before_staging_creation() {
        for (bytes, format) in [
            (
                zip_bytes(&[TestEntry::Symlink("link", "target")]),
                PackageFormat::Zip,
            ),
            (
                tar_gz_bytes(&[TestEntry::HardLink("link", "target")]),
                PackageFormat::TarGz,
            ),
        ] {
            let temp = TempDir::new().expect("temp dir");
            let root = temp.path().join("stage");
            let result = VerifiedPackage::verify(
                write_package(&temp, "bad", &bytes),
                digest(&bytes),
                format,
            );
            assert!(matches!(result, Err(ExtractionError::BlockingHazards(_))));
            assert!(!root.exists());
        }
    }

    #[test]
    fn rejects_format_and_digest_mismatches() {
        let bytes = zip_bytes(&[TestEntry::File("run", b"ok")]);
        let temp = TempDir::new().expect("temp dir");
        let file = write_package(&temp, "package", &bytes);
        assert!(matches!(
            VerifiedPackage::verify(Arc::clone(&file), digest(&bytes), PackageFormat::TarGz),
            Err(ExtractionError::FormatMismatch { .. })
        ));
        assert!(matches!(
            VerifiedPackage::verify(
                file,
                Sha256Digest::parse(&"0".repeat(64)).expect("zero digest"),
                PackageFormat::Zip,
            ),
            Err(ExtractionError::DigestMismatch { .. })
        ));
    }

    #[test]
    fn rejects_case_duplicates_and_ancestry_conflicts() {
        for entries in [
            vec![
                TestEntry::File("Game.exe", b"one"),
                TestEntry::File("game.exe", b"two"),
            ],
            vec![
                TestEntry::File("data", b"file"),
                TestEntry::File("data/child", b"child"),
            ],
        ] {
            let bytes = zip_bytes(&entries);
            let temp = TempDir::new().expect("temp dir");
            let root = temp.path().join("stage");
            let result = VerifiedPackage::verify(
                write_package(&temp, "package", &bytes),
                digest(&bytes),
                PackageFormat::Zip,
            );
            match result {
                Err(ExtractionError::BlockingHazards(_)) => {}
                Ok(package) => assert!(matches!(
                    extract_verified_package(&package, &root, &[]),
                    Err(ExtractionError::AncestryConflict(_))
                )),
                Err(error) => panic!("unexpected error: {error}"),
            }
            assert!(!root.exists());
        }
    }

    #[test]
    fn rejects_exact_duplicate_paths() {
        let bytes = tar_gz_bytes(&[
            TestEntry::File("same", b"one"),
            TestEntry::File("same", b"two"),
        ]);
        let temp = TempDir::new().expect("temp dir");
        let root = temp.path().join("stage");
        let result = VerifiedPackage::verify(
            write_package(&temp, "package", &bytes),
            digest(&bytes),
            PackageFormat::TarGz,
        );
        assert!(matches!(result, Err(ExtractionError::BlockingHazards(_))));
        assert!(!root.exists());
    }

    #[test]
    fn preserves_exact_paths_and_records_hashes_and_implicit_directories() {
        let bytes = zip_bytes(&[TestEntry::File("outer/bin/Game.exe", b"payload")]);
        let (_temp, root, outcome) = extract_test_package(&bytes, PackageFormat::Zip, &[]);
        assert!(root.join("outer/bin/Game.exe").is_file());
        assert!(!root.join("bin/Game.exe").exists());
        let entries = outcome
            .manifest
            .iter()
            .map(|entry| {
                (
                    entry.path.as_str(),
                    entry.entry_type,
                    entry.file_digest.as_ref(),
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(entries[0], ("outer", ManifestEntryType::Directory, None));
        assert_eq!(
            entries[1],
            ("outer/bin", ManifestEntryType::Directory, None)
        );
        assert_eq!(entries[2].0, "outer/bin/Game.exe");
        assert_eq!(entries[2].1, ManifestEntryType::File);
        assert_eq!(
            entries[2].2.map(Sha256Digest::as_str),
            Some(digest(b"payload").as_str())
        );
        assert!(outcome
            .manifest
            .iter()
            .all(|entry| entry.path.as_str() != "."));
    }

    #[test]
    fn ratio_bomb_preflight_leaves_no_staging_and_writer_orders_check_before_emit() {
        let payload =
            vec![0_u8; archive_inspection::RATIO_ENFORCEMENT_OUTPUT_BYTES as usize + 64 * 1024];
        let bytes = tar_gz_bytes(&[TestEntry::File("bomb", &payload)]);
        let temp = TempDir::new().expect("temp dir");
        let root = temp.path().join("stage");
        // Construct directly inside the private test module so this specifically
        // exercises extract_verified_package's mandatory second preflight.
        let package = VerifiedPackage {
            file: write_package(&temp, "bomb.tar.gz", &bytes),
            digest: digest(&bytes),
            format: PackageFormat::TarGz,
            declared_regular_file_size_bytes: payload.len() as u64,
        };
        assert!(matches!(
            extract_verified_package(&package, &root, &[]),
            Err(ExtractionError::Inspection(DetectInspectError::Inspection(
                archive_inspection::InspectionError::CompressionRatioExceeded { .. }
            )))
        ));
        assert!(!root.exists());

        let (reader, mut accounting) =
            archive_inspection::extraction_tar_gz_reader(Cursor::new(bytes));
        let mut archive = Archive::new(reader);
        let mut entries = archive.entries().expect("tar entries");
        let mut entry = entries
            .next()
            .expect("bomb entry")
            .expect("parse bomb entry");
        accounting.begin_entry().expect("first entry");
        let mut emitted = 0_u64;
        let error = accounting
            .stream_regular_payload(&mut entry, |buffer| {
                emitted += buffer.len() as u64;
                Ok(())
            })
            .expect_err("ratio bomb must fail during writing pass");
        let measured_output = match error {
            archive_inspection::TarExtractionStreamError::Policy(
                archive_inspection::InspectionError::CompressionRatioExceeded {
                    measured_output,
                    ..
                },
            ) => measured_output,
            other => panic!("unexpected extraction error: {other:?}"),
        };
        assert!(emitted < archive_inspection::RATIO_ENFORCEMENT_OUTPUT_BYTES);
        assert!(measured_output >= archive_inspection::RATIO_ENFORCEMENT_OUTPUT_BYTES);
        assert!(
            emitted < measured_output,
            "the violating buffer was emitted"
        );
    }

    #[test]
    fn profiles_below_a_regular_file_are_unavailable_without_failing_extraction() {
        use crate::distribution::{LaunchType, ProfileId, WorkingDir};

        let bytes = zip_bytes(&[TestEntry::File("bin", b"not a directory")]);
        let profiles = [
            LaunchProfile {
                id: ProfileId::parse("missing-executable").expect("profile id"),
                display_name: "Missing executable".to_owned(),
                executable: RelativePath::parse("bin/run").expect("executable"),
                working_dir: WorkingDir::parse(".").expect("working dir"),
                arguments: Vec::new(),
                launch_type: LaunchType::Game,
            },
            LaunchProfile {
                id: ProfileId::parse("missing-working-dir").expect("profile id"),
                display_name: "Missing working directory".to_owned(),
                executable: RelativePath::parse("bin").expect("executable"),
                working_dir: WorkingDir::parse("bin/work").expect("working dir"),
                arguments: Vec::new(),
                launch_type: LaunchType::Game,
            },
        ];

        let (_temp, _root, outcome) = extract_test_package(&bytes, PackageFormat::Zip, &profiles);

        assert_eq!(
            outcome.profiles[0].availability,
            ProfileAvailability::Unavailable(ProfileUnavailableReason::ExecutableMissing)
        );
        assert_eq!(
            outcome.profiles[1].availability,
            ProfileAvailability::Unavailable(ProfileUnavailableReason::WorkingDirectoryMissing)
        );
    }

    #[test]
    fn capability_extraction_retains_only_profile_executables() {
        use crate::distribution::{LaunchType, ProfileId, WorkingDir};

        let temp = TempDir::new().expect("temp dir");
        let root = temp.path().join("stage");
        fs::create_dir(&root).expect("create staging root");
        let root_capability =
            cap_std::fs::Dir::open_ambient_dir(&root, cap_std::ambient_authority())
                .expect("open staging root");
        let profile = LaunchProfile {
            id: ProfileId::parse("default").expect("profile id"),
            display_name: "Play".to_owned(),
            executable: RelativePath::parse("file-1000.bin").expect("executable"),
            working_dir: WorkingDir::parse(".").expect("working dir"),
            arguments: Vec::new(),
            launch_type: LaunchType::Game,
        };
        let mut target =
            CapabilityTarget::new(root_capability, root, &[profile]).expect("capability target");
        let mut manifest = Vec::new();

        for index in 0..2_100 {
            let path = RelativePath::parse(&format!("file-{index}.bin")).expect("file path");
            let output = target.create_file(&path).expect("create extracted file");
            finish_capability_file(
                &mut target,
                output,
                path,
                Sha256::new(),
                None,
                &mut manifest,
            )
            .expect("finish extracted file");
        }

        assert_eq!(manifest.len(), 2_100);
        assert_eq!(target.profile_executables.len(), 1);
        assert!(target.profile_executables.contains_key("file-1000.bin"));
    }

    #[cfg(unix)]
    #[test]
    fn applies_execute_permissions_only_to_exact_profile_executable() {
        use std::os::unix::fs::PermissionsExt;

        use crate::distribution::{LaunchType, ProfileId, WorkingDir};

        let bytes = zip_bytes(&[
            TestEntry::File("bin/run", b"run"),
            TestEntry::File("bin/helper", b"helper"),
        ]);
        let profile = LaunchProfile {
            id: ProfileId::parse("default").expect("profile id"),
            display_name: "Play".to_owned(),
            executable: RelativePath::parse("bin/run").expect("executable"),
            working_dir: WorkingDir::parse("bin").expect("working dir"),
            arguments: Vec::new(),
            launch_type: LaunchType::Game,
        };
        let (_temp, root, outcome) = extract_test_package(&bytes, PackageFormat::Zip, &[profile]);
        let run_mode = fs::metadata(root.join("bin/run"))
            .expect("run metadata")
            .permissions()
            .mode()
            & 0o777;
        let helper_mode = fs::metadata(root.join("bin/helper"))
            .expect("helper metadata")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(run_mode, 0o700);
        assert_eq!(helper_mode, 0o600);
        assert_eq!(
            outcome.profiles[0].availability,
            ProfileAvailability::Available
        );
    }

    #[cfg(unix)]
    #[test]
    fn capability_extraction_preserves_packaged_execute_bits() {
        use std::os::unix::fs::PermissionsExt;

        use crate::distribution::{LaunchType, ProfileId, WorkingDir};

        let profile = LaunchProfile {
            id: ProfileId::parse("default").expect("profile id"),
            display_name: "Play".to_owned(),
            executable: RelativePath::parse("bin/run").expect("executable"),
            working_dir: WorkingDir::parse("bin").expect("working dir"),
            arguments: Vec::new(),
            launch_type: LaunchType::Game,
        };
        let entries = [
            TestEntry::File("bin/run", b"#!/bin/sh\n"),
            TestEntry::Executable("bin/helper", b"helper"),
            TestEntry::File("bin/data", b"data"),
        ];
        for (bytes, format) in [
            (zip_bytes(&entries), PackageFormat::Zip),
            (tar_gz_bytes(&entries), PackageFormat::TarGz),
            (tar_zstd_bytes(&entries), PackageFormat::TarZstd),
        ] {
            let mode = |root: &Path, name: &str| {
                fs::metadata(root.join(name))
                    .expect("file metadata")
                    .permissions()
                    .mode()
                    & 0o7777
            };
            let (_ambient_temp, ambient_root, _) =
                extract_test_package(&bytes, format.clone(), std::slice::from_ref(&profile));
            assert_eq!(
                mode(&ambient_root, "bin/run"),
                0o700,
                "ambient signed entrypoint normalization"
            );
            assert_eq!(
                mode(&ambient_root, "bin/helper"),
                0o711,
                "ambient packaged execute bits without special bits"
            );
            assert_eq!(
                mode(&ambient_root, "bin/data"),
                0o600,
                "ambient unmarked package file"
            );

            let temp = TempDir::new().expect("temp dir");
            let package = VerifiedPackage::verify(
                write_package(&temp, "package", &bytes),
                digest(&bytes),
                format,
            )
            .expect("verify package");
            let root = temp.path().join("stage");
            fs::create_dir(&root).expect("create staging root");
            let capability =
                cap_std::fs::Dir::open_ambient_dir(&root, cap_std::ambient_authority())
                    .expect("open staging root");
            extract_verified_package_into_capability(
                &package,
                capability,
                &root,
                std::slice::from_ref(&profile),
            )
            .expect("extract package");

            assert_eq!(
                mode(&root, "bin/run"),
                0o700,
                "signed entrypoint normalization"
            );
            assert_eq!(
                mode(&root, "bin/helper"),
                0o711,
                "packaged execute bits without special bits"
            );
            assert_eq!(mode(&root, "bin/data"), 0o600, "unmarked package file");
        }
    }
}
