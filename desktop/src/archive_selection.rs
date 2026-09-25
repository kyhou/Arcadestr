//! Immutable account-bound native archive staging and selection registry.

use std::collections::{HashMap, VecDeque};
use std::fs::{File, Metadata, OpenOptions};
use std::future::Future;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use arcadestr_core::archive_inspection::{
    detect_executable_candidates, inspect_detected_archive, ArchiveInspection, ArchiveWarning,
    CandidateConfidence, CandidateReason, DetectInspectError, EntryKind, InspectionError,
    ZipPreflightError,
};
use arcadestr_core::archive_inspection_contract::*;
use arcadestr_core::distribution::{
    normalize_arch, normalize_os, PackageFormat, PathError, PlatformTarget, RelativePath,
    Sha256Digest, WorkingDir,
};
use sha2::{Digest, Sha256};
use sysinfo::Disks;
use uuid::Uuid;

const SELECTION_TTL: Duration = Duration::from_secs(15 * 60);
const JANITOR_INTERVAL: Duration = Duration::from_secs(60);
const STORAGE_MARGIN: u64 = 512 * 1024 * 1024;
const MAX_LIVE_SELECTIONS: usize = 4;
/// Ephemeral lifecycle retention; records are deterministically evicted oldest-first.
const MAX_TOMBSTONES: usize = 4096;
const STAGED_PREFIX: &str = "arcadestr-archive-v1-";
const STAGED_SUFFIX: &str = ".stage";
const SESSION_PREFIX: &str = "session-v2-";
const SESSION_LOCK_NAME: &str = "session.lock";
const ROOT_CLEANUP_LOCK_NAME: &str = "cleanup.lock";

pub(crate) type AccountCheck = Arc<
    dyn Fn() -> Pin<Box<dyn Future<Output = Result<String, ArchiveSelectionError>> + Send>>
        + Send
        + Sync,
>;

#[derive(Debug, Clone, PartialEq, Eq)]
struct FileIdentity {
    size: u64,
    modified: Option<SystemTime>,
    #[cfg(unix)]
    dev: u64,
    #[cfg(unix)]
    ino: u64,
}

impl FileIdentity {
    fn new(metadata: &Metadata) -> Self {
        #[cfg(unix)]
        use std::os::unix::fs::MetadataExt;
        Self {
            size: metadata.len(),
            modified: metadata.modified().ok(),
            #[cfg(unix)]
            dev: metadata.dev(),
            #[cfg(unix)]
            ino: metadata.ino(),
        }
    }
    fn check(&self, actual: &Self) -> Result<(), ArchiveSelectionError> {
        if self.size != actual.size {
            return Err(ArchiveSelectionError::Mutation {
                failure: ArchiveMutationFailure::SizeChanged {
                    selected: self.size,
                    actual: actual.size,
                },
            });
        }
        if self.modified != actual.modified {
            return Err(ArchiveSelectionError::Mutation {
                failure: ArchiveMutationFailure::TimestampChanged,
            });
        }
        #[cfg(unix)]
        if self.dev != actual.dev || self.ino != actual.ino {
            return Err(ArchiveSelectionError::Mutation {
                failure: ArchiveMutationFailure::IdentityChanged,
            });
        }
        Ok(())
    }
}

#[derive(Debug)]
struct Selection {
    publisher: String,
    source_path: PathBuf,
    source_identity: FileIdentity,
    path: PathBuf,
    file: Arc<File>,
    identity: FileIdentity,
    filename: SafeArchiveFilename,
    last_access: Instant,
    generation: u64,
    in_progress: bool,
    publication_token: Option<Uuid>,
    ready: Option<Arc<ReadySnapshot>>,
    digest: Sha256Digest,
}

#[derive(Debug)]
struct ReadySnapshot {
    inspection: ArchiveInspection,
    summary: ArchiveSelectionSummary,
    files: Vec<SelectableArchiveFile>,
    directories: Vec<SelectableWorkingDirectory>,
    candidates: HashMap<String, Vec<ArchiveCandidate>>,
    hazards: HashMap<ArchiveHazardCategory, Vec<ArchiveHazardDetail>>,
    warnings: HashMap<ArchiveWarningCategory, Vec<ArchiveWarningDetail>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TombstoneState {
    Expired,
    Discarded,
}

#[derive(Debug)]
struct Tombstone {
    id: String,
    publisher: String,
    state: TombstoneState,
}

#[derive(Debug)]
struct Registry {
    selections: HashMap<String, Selection>,
    tombstones: VecDeque<Tombstone>,
    reservations: usize,
}

#[derive(Debug, Clone)]
pub struct ArchiveSelectionService {
    registry: Arc<Mutex<Registry>>,
    staging_dir: Arc<PathBuf>,
    _session_lock: Arc<File>,
}

#[derive(Debug)]
pub struct PublicationLease {
    service: ArchiveSelectionService,
    id: ArchiveSelectionId,
    token: Uuid,
    pub(crate) digest: Sha256Digest,
    pub(crate) file_size: u64,
    released: bool,
}

#[derive(Debug)]
pub(crate) enum LeaseUploadError {
    Archive(ArchiveSelectionError),
    Upload(arcadestr_core::adp_client::AdpClientError),
}

impl PublicationLease {
    pub(crate) async fn verify_digest(&self) -> Result<(), ArchiveSelectionError> {
        self.service.verify_lease_digest(self).await
    }

    pub(crate) async fn upload_with_progress<F>(
        &self,
        client: &arcadestr_core::adp_client::AdpClient,
        signer: &dyn arcadestr_core::signers::NostrSigner,
        listing: &nostr::Event,
        expected_publisher: &str,
        account_check: AccountCheck,
        progress: F,
    ) -> Result<arcadestr_core::adp_client::UploadResponse, LeaseUploadError>
    where
        F: Fn(u64, u64) + Send + Sync + 'static,
    {
        let (file, filename) = self
            .service
            .private_lease_file(self)
            .map_err(LeaseUploadError::Archive)?;
        verify_file_digest(Arc::clone(&file), &self.digest)
            .await
            .map_err(LeaseUploadError::Archive)?;
        let authorization = client
            .authorize_upload(signer)
            .await
            .map_err(LeaseUploadError::Upload)?;
        let active = (account_check)().await.map_err(LeaseUploadError::Archive)?;
        require_account(expected_publisher, &active).map_err(LeaseUploadError::Archive)?;
        client
            .upload_authorized_file_with_progress(authorization, listing, file, &filename, progress)
            .await
            .map_err(LeaseUploadError::Upload)
    }

    pub(crate) async fn finish(mut self) {
        let path = self.service.remove_selection_sync(&self.id, self.token);
        self.released = path.is_some();
        if let Some(path) = path {
            best_effort_delete(path).await;
        }
    }
}

impl Drop for PublicationLease {
    fn drop(&mut self) {
        if self.released {
            return;
        }
        let mut registry = match self.service.registry.lock() {
            Ok(registry) => registry,
            Err(poisoned) => poisoned.into_inner(),
        };
        if let Some(value) = registry.selections.get_mut(self.id.as_str()) {
            if value.publication_token == Some(self.token) {
                value.publication_token = None;
                value.last_access = Instant::now();
            }
        }
    }
}

struct PublicationReservationGuard {
    registry: Arc<Mutex<Registry>>,
    id: String,
    token: Uuid,
    held: bool,
}

impl PublicationReservationGuard {
    fn transfer(mut self) {
        self.held = false;
    }
}

impl Drop for PublicationReservationGuard {
    fn drop(&mut self) {
        if !self.held {
            return;
        }
        let mut registry = match self.registry.lock() {
            Ok(registry) => registry,
            Err(poisoned) => poisoned.into_inner(),
        };
        if let Some(entry) = registry.selections.get_mut(&self.id) {
            if entry.publication_token == Some(self.token) {
                entry.publication_token = None;
                entry.last_access = Instant::now();
            }
        }
    }
}

struct OperationGuard {
    registry: Arc<Mutex<Registry>>,
    id: String,
}

impl Drop for OperationGuard {
    fn drop(&mut self) {
        if let Ok(mut registry) = self.registry.lock() {
            if let Some(entry) = registry.selections.get_mut(&self.id) {
                entry.in_progress = false;
            }
        }
    }
}

struct ReservationGuard {
    registry: Arc<Mutex<Registry>>,
    held: bool,
}

impl ReservationGuard {
    fn release_into(&mut self, registry: &mut Registry) {
        if self.held {
            registry.reservations = registry.reservations.saturating_sub(1);
            self.held = false;
        }
    }

    fn release_fallback(&mut self) {
        if !self.held {
            return;
        }
        match self.registry.try_lock() {
            Ok(mut registry) => {
                registry.reservations = registry.reservations.saturating_sub(1);
                self.held = false;
            }
            Err(std::sync::TryLockError::Poisoned(error)) => {
                let mut registry = error.into_inner();
                registry.reservations = registry.reservations.saturating_sub(1);
                self.held = false;
            }
            Err(std::sync::TryLockError::WouldBlock) => {
                tracing::error!(
                    "archive selection reservation leaked to avoid a registry self-deadlock"
                );
            }
        }
    }
}

impl Drop for ReservationGuard {
    fn drop(&mut self) {
        self.release_fallback();
    }
}

impl ArchiveSelectionService {
    pub fn new() -> Result<Self, ArchiveSelectionError> {
        let root = dirs::cache_dir()
            .ok_or_else(registry_unavailable)?
            .join("arcadestr")
            .join("archive-selections");
        let (staging_dir, session_lock) = initialize_session(&root)?;
        let service = Self {
            registry: Arc::new(Mutex::new(Registry {
                selections: HashMap::new(),
                tombstones: VecDeque::new(),
                reservations: 0,
            })),
            staging_dir: Arc::new(staging_dir),
            _session_lock: Arc::new(session_lock),
        };
        service.start_janitor();
        Ok(service)
    }

    fn start_janitor(&self) {
        let service = self.clone();
        tauri::async_runtime::spawn(async move {
            loop {
                tokio::time::sleep(JANITOR_INTERVAL).await;
                service.prune_expired().await;
            }
        });
    }

    pub async fn stage_source(
        &self,
        source: PathBuf,
        expected_publisher: &str,
        account_check: AccountCheck,
    ) -> Result<ArchiveFileSelection, ArchiveSelectionError> {
        let filename = source
            .file_name()
            .and_then(|v| v.to_str())
            .and_then(|v| SafeArchiveFilename::parse(v).ok())
            .ok_or(ArchiveSelectionError::NonRegularFile)?;
        let mut reservation = {
            let mut registry = self.registry.lock().map_err(|_| registry_unavailable())?;
            let removed = prune_locked(&mut registry, Instant::now());
            delete_paths_async(removed);
            if registry.selections.len() + registry.reservations >= MAX_LIVE_SELECTIONS {
                return Err(ArchiveSelectionError::Registry {
                    failure: ArchiveRegistryFailure::CapacityExceeded,
                });
            }
            registry.reservations += 1;
            ReservationGuard {
                registry: Arc::clone(&self.registry),
                held: true,
            }
        };
        let (source_path, source_identity, path, file, identity, digest) =
            tokio::task::spawn_blocking({
                let staging = Arc::clone(&self.staging_dir);
                move || stage_file(&source, &staging)
            })
            .await
            .map_err(|_| registry_unavailable())??;
        let actual = match (account_check)().await {
            Ok(actual) => actual,
            Err(error) => {
                best_effort_delete(path).await;
                return Err(error);
            }
        };
        if let Err(error) = require_account(expected_publisher, &actual) {
            best_effort_delete(path).await;
            return Err(error);
        }
        let mut registry = self.registry.lock().map_err(|_| registry_unavailable())?;
        reservation.release_into(&mut registry);
        let id_text = Uuid::new_v4().to_string();
        let id = ArchiveSelectionId::parse(&id_text).map_err(|_| registry_unavailable())?;
        let now = Instant::now();
        registry.selections.insert(
            id_text,
            Selection {
                publisher: actual,
                source_path,
                source_identity,
                path,
                file: Arc::new(file),
                identity: identity.clone(),
                filename: filename.clone(),
                last_access: now,
                generation: 0,
                in_progress: false,
                publication_token: None,
                ready: None,
                digest,
            },
        );
        Ok(ArchiveFileSelection {
            selection_id: id,
            safe_filename: filename,
            file_size: identity.size,
            expires_at_unix: unix_now().saturating_add(SELECTION_TTL.as_secs()),
        })
    }

    pub async fn inspect(
        &self,
        request: InspectArchiveRequest,
        active: &str,
        account_check: AccountCheck,
    ) -> Result<InspectArchiveResponse, ArchiveSelectionError> {
        let id = request.selection_id.clone();
        let (file, generation, selection, cached, baseline, source) = {
            let mut registry = self.registry.lock().map_err(|_| registry_unavailable())?;
            let entry = bound(&mut registry, &id, &request.expected_publisher_npub, active)?;
            if entry.in_progress || entry.publication_token.is_some() {
                return Err(ArchiveSelectionError::Busy {
                    request_id: request.request_id,
                });
            }
            let cached = entry.ready.clone();
            entry.in_progress = true;
            (
                Arc::clone(&entry.file),
                entry.generation,
                selection_dto(&id, entry),
                cached,
                entry.digest.clone(),
                (entry.source_path.clone(), entry.source_identity.clone()),
            )
        };
        let _operation = OperationGuard {
            registry: Arc::clone(&self.registry),
            id: id.as_str().to_owned(),
        };
        let work = async {
            if let Some(ready) = cached {
                tokio::task::spawn_blocking(move || restat(&source.0, &source.1))
                    .await
                    .map_err(|_| registry_unavailable())??;
                verify_file_digest(file, &baseline).await?;
                let active_after_work = (account_check)().await?;
                require_account(&request.expected_publisher_npub, &active_after_work)?;
                return Ok((ready.summary.clone(), None, active_after_work));
            }
            let expected = baseline.clone();
            let (format, inspection) = tokio::task::spawn_blocking(move || {
                let mut file = file
                    .try_clone()
                    .map_err(|_| storage(ArchiveStorageOperation::Open))?;
                file.seek(SeekFrom::Start(0))
                    .map_err(|_| storage(ArchiveStorageOperation::Seek))?;
                let result = inspect_detected_archive(
                    file.try_clone()
                        .map_err(|_| storage(ArchiveStorageOperation::Open))?,
                )
                .map_err(map_detect)?;
                file.seek(SeekFrom::Start(0))
                    .map_err(|_| storage(ArchiveStorageOperation::Seek))?;
                verify_digest_text(hash_reader(&mut file)?, &expected)?;
                Ok(result)
            })
            .await
            .map_err(|_| registry_unavailable())??;
            let active_after_work = (account_check)().await?;
            require_account(&request.expected_publisher_npub, &active_after_work)?;
            let ready = build_ready(selection, format, request.expected_format, inspection);
            Ok((ready.summary.clone(), Some(ready), active_after_work))
        }
        .await;
        let (summary, result, active_after_work) = work?;
        let mut registry = self.registry.lock().map_err(|_| registry_unavailable())?;
        let entry = bound(
            &mut registry,
            &id,
            &active_after_work,
            &request.expected_publisher_npub,
        )?;
        if entry.generation != generation {
            return Err(unknown(id));
        }
        if let Some(ready) = result {
            entry.ready = Some(Arc::new(ready));
        }
        Ok(InspectArchiveResponse {
            summary: entry.ready.as_ref().map_or(summary, |v| v.summary.clone()),
        })
    }

    pub async fn hash(
        &self,
        request: HashSelectedArchiveRequest,
        active: &str,
        account_check: AccountCheck,
    ) -> Result<HashSelectedArchiveResponse, ArchiveSelectionError> {
        let id = request.selection_id.clone();
        let (file, generation, digest) = {
            let mut registry = self.registry.lock().map_err(|_| registry_unavailable())?;
            let entry = bound(&mut registry, &id, &request.expected_publisher_npub, active)?;
            if entry.in_progress || entry.publication_token.is_some() {
                return Err(ArchiveSelectionError::Busy {
                    request_id: request.request_id,
                });
            }
            entry.in_progress = true;
            (
                Arc::clone(&entry.file),
                entry.generation,
                entry.digest.clone(),
            )
        };
        let _operation = OperationGuard {
            registry: Arc::clone(&self.registry),
            id: id.as_str().to_owned(),
        };
        let work = async {
            verify_file_digest(file, &digest).await?;
            let active_after_work = (account_check)().await?;
            require_account(&request.expected_publisher_npub, &active_after_work)?;
            Ok((digest, active_after_work))
        }
        .await;
        let (digest, active_after_work) = work?;
        let mut registry = self.registry.lock().map_err(|_| registry_unavailable())?;
        let entry = bound(
            &mut registry,
            &id,
            &active_after_work,
            &request.expected_publisher_npub,
        )?;
        if entry.generation != generation {
            return Err(unknown(id));
        }
        Ok(HashSelectedArchiveResponse {
            selection_id: id,
            sha256: digest,
        })
    }

    pub async fn query_files(
        &self,
        r: QueryArchiveFilesRequest,
        active: &str,
    ) -> Result<QueryArchiveFilesResponse, ArchiveSelectionError> {
        let page = validate_page(r.page)?;
        let ready = self
            .ready(&r.selection_id, &r.expected_publisher_npub, active)
            .await?;
        Ok(QueryArchiveFilesResponse {
            page: page_filter(&ready.files, r.search.as_str(), page, |v| v.path.as_str())?,
        })
    }
    pub async fn query_working_directories(
        &self,
        r: QueryArchiveWorkingDirectoriesRequest,
        active: &str,
    ) -> Result<QueryArchiveWorkingDirectoriesResponse, ArchiveSelectionError> {
        let page = validate_page(r.page)?;
        let ready = self
            .ready(&r.selection_id, &r.expected_publisher_npub, active)
            .await?;
        Ok(QueryArchiveWorkingDirectoriesResponse {
            page: page_filter(&ready.directories, r.search.as_str(), page, |v| {
                v.directory.as_str()
            })?,
        })
    }
    pub async fn query_candidates(
        &self,
        r: QueryArchiveCandidatesRequest,
        active: &str,
    ) -> Result<QueryArchiveCandidatesResponse, ArchiveSelectionError> {
        let page = validate_page(r.page)?;
        let platform = canonical_platform(&r.platform)?;
        let ready = self
            .ready(&r.selection_id, &r.expected_publisher_npub, active)
            .await?;
        let key = format!("{}-{}", platform.os, platform.arch);
        let values = ready.candidates.get(&key).map(Vec::as_slice).unwrap_or(&[]);
        Ok(QueryArchiveCandidatesResponse {
            page: page_filter(values, r.search.as_str(), page, |v| v.path.as_str())?,
        })
    }
    pub async fn query_hazards(
        &self,
        r: QueryArchiveHazardsRequest,
        active: &str,
    ) -> Result<QueryArchiveHazardsResponse, ArchiveSelectionError> {
        let page = validate_page(r.page)?;
        let ready = self
            .ready(&r.selection_id, &r.expected_publisher_npub, active)
            .await?;
        Ok(QueryArchiveHazardsResponse {
            page: page_all(
                ready
                    .hazards
                    .get(&r.category)
                    .map(Vec::as_slice)
                    .unwrap_or(&[]),
                page,
            )?,
        })
    }
    pub async fn query_warnings(
        &self,
        r: QueryArchiveWarningsRequest,
        active: &str,
    ) -> Result<QueryArchiveWarningsResponse, ArchiveSelectionError> {
        let page = validate_page(r.page)?;
        let ready = self
            .ready(&r.selection_id, &r.expected_publisher_npub, active)
            .await?;
        Ok(QueryArchiveWarningsResponse {
            page: page_all(
                ready
                    .warnings
                    .get(&r.category)
                    .map(Vec::as_slice)
                    .unwrap_or(&[]),
                page,
            )?,
        })
    }

    pub async fn discard(
        &self,
        r: DiscardArchiveSelectionRequest,
        active: &str,
    ) -> Result<DiscardArchiveSelectionResponse, ArchiveSelectionError> {
        let value = {
            let mut registry = self.registry.lock().map_err(|_| registry_unavailable())?;
            let entry = bound(
                &mut registry,
                &r.selection_id,
                &r.expected_publisher_npub,
                active,
            )?;
            if entry.in_progress || entry.publication_token.is_some() {
                return Err(ArchiveSelectionError::Busy {
                    request_id: r.request_id,
                });
            }
            let value = registry.selections.remove(r.selection_id.as_str());
            if let Some(value) = value.as_ref() {
                tombstone(
                    &mut registry,
                    r.selection_id.as_str().to_owned(),
                    value.publisher.clone(),
                    TombstoneState::Discarded,
                );
            }
            value
        };
        if let Some(value) = value {
            best_effort_delete(value.path).await;
        }
        Ok(DiscardArchiveSelectionResponse {
            selection_id: r.selection_id,
            discarded: true,
        })
    }

    pub async fn publication_lease(
        &self,
        reference: &BuildSelectionRef,
        expected: &str,
        active: &str,
        account_check: AccountCheck,
    ) -> Result<PublicationLease, ArchiveSelectionError> {
        let token = Uuid::new_v4();
        let (file, generation, actual, file_size) = {
            let mut registry = self.registry.lock().map_err(|_| registry_unavailable())?;
            let entry = bound(&mut registry, &reference.selection_id, expected, active)?;
            if entry.in_progress || entry.publication_token.is_some() {
                let request_id = ArchiveRequestId::parse(&Uuid::new_v4().to_string())
                    .map_err(|_| registry_unavailable())?;
                return Err(ArchiveSelectionError::Busy { request_id });
            }
            let ready = entry.ready.as_ref().ok_or_else(registry_unavailable)?;
            if ready.summary.blocking_hazards {
                return Err(ArchiveSelectionError::BlockingHazards {
                    counts: ready.summary.counts.hazards.clone(),
                });
            }
            if entry.digest != reference.sha256 {
                return Err(ArchiveSelectionError::DigestMismatch {
                    expected: reference.sha256.clone(),
                    actual: entry.digest.clone(),
                });
            }
            entry.publication_token = Some(token);
            (
                Arc::clone(&entry.file),
                entry.generation,
                entry.digest.clone(),
                entry.identity.size,
            )
        };
        let reservation = PublicationReservationGuard {
            registry: Arc::clone(&self.registry),
            id: reference.selection_id.as_str().to_owned(),
            token,
            held: true,
        };
        verify_file_digest(file, &actual).await?;
        let active_after_work = (account_check)().await?;
        require_account(expected, &active_after_work)?;
        {
            let mut registry = self.registry.lock().map_err(|_| registry_unavailable())?;
            let entry = bound(
                &mut registry,
                &reference.selection_id,
                expected,
                &active_after_work,
            )?;
            if entry.generation != generation || entry.publication_token != Some(token) {
                return Err(unknown(reference.selection_id.clone()));
            }
        }
        reservation.transfer();
        Ok(PublicationLease {
            service: self.clone(),
            id: reference.selection_id.clone(),
            token,
            digest: reference.sha256.clone(),
            file_size,
            released: false,
        })
    }

    async fn ready(
        &self,
        id: &ArchiveSelectionId,
        expected: &str,
        active: &str,
    ) -> Result<Arc<ReadySnapshot>, ArchiveSelectionError> {
        let ready = {
            let mut registry = self.registry.lock().map_err(|_| registry_unavailable())?;
            let entry = bound(&mut registry, id, expected, active)?;
            entry.ready.clone().ok_or_else(registry_unavailable)?
        };
        Ok(ready)
    }
    fn remove_selection_sync(&self, id: &ArchiveSelectionId, token: Uuid) -> Option<PathBuf> {
        let mut registry = self.registry.lock().ok()?;
        let entry = registry.selections.get(id.as_str())?;
        if entry.publication_token != Some(token) {
            return None;
        }
        let value = registry.selections.remove(id.as_str())?;
        tombstone(
            &mut registry,
            id.as_str().to_owned(),
            value.publisher.clone(),
            TombstoneState::Discarded,
        );
        Some(value.path)
    }
    async fn prune_expired(&self) {
        let paths = self
            .registry
            .lock()
            .map(|mut registry| prune_locked(&mut registry, Instant::now()))
            .unwrap_or_default();
        delete_paths_await(paths).await;
    }
    fn private_lease_file(
        &self,
        lease: &PublicationLease,
    ) -> Result<(Arc<File>, SafeArchiveFilename), ArchiveSelectionError> {
        let registry = self.registry.lock().map_err(|_| registry_unavailable())?;
        let entry = registry
            .selections
            .get(lease.id.as_str())
            .ok_or_else(|| unknown(lease.id.clone()))?;
        if entry.publication_token != Some(lease.token) || entry.digest != lease.digest {
            return Err(unknown(lease.id.clone()));
        }
        Ok((Arc::clone(&entry.file), entry.filename.clone()))
    }
    async fn verify_lease_digest(
        &self,
        lease: &PublicationLease,
    ) -> Result<(), ArchiveSelectionError> {
        let (file, _) = self.private_lease_file(lease)?;
        verify_file_digest(file, &lease.digest).await
    }
}

fn stage_file(
    source: &Path,
    staging: &Path,
) -> Result<
    (
        PathBuf,
        FileIdentity,
        PathBuf,
        File,
        FileIdentity,
        Sha256Digest,
    ),
    ArchiveSelectionError,
> {
    let canonical = std::fs::canonicalize(source)
        .map_err(|_| storage(ArchiveStorageOperation::Canonicalize))?;
    let mut input = File::open(&canonical).map_err(|_| storage(ArchiveStorageOperation::Open))?;
    let before = input
        .metadata()
        .map_err(|_| storage(ArchiveStorageOperation::Metadata))?;
    if !before.is_file() {
        return Err(ArchiveSelectionError::NonRegularFile);
    }
    let source_identity = FileIdentity::new(&before);
    preflight_storage(staging, before.len())?;
    let path = staging.join(format!("{STAGED_PREFIX}{}{STAGED_SUFFIX}", Uuid::new_v4()));
    let mut options = OpenOptions::new();
    options.write(true).read(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut output = options
        .open(&path)
        .map_err(|error| copy_error(error, before.len(), staging))?;
    let copied = match std::io::copy(&mut input, &mut output) {
        Ok(v) => v,
        Err(error) => {
            let mapped = copy_error(error, before.len(), staging);
            let _ = std::fs::remove_file(&path);
            return Err(mapped);
        }
    };
    output
        .flush()
        .and_then(|_| output.sync_all())
        .map_err(|error| {
            let mapped = copy_error(error, before.len(), staging);
            let _ = std::fs::remove_file(&path);
            mapped
        })?;
    drop(input);
    if let Err(error) = restat(&canonical, &source_identity) {
        let _ = std::fs::remove_file(&path);
        return Err(error);
    }
    let metadata = output
        .metadata()
        .map_err(|_| storage(ArchiveStorageOperation::Metadata))?;
    let identity = FileIdentity::new(&metadata);
    output
        .seek(SeekFrom::Start(0))
        .map_err(|_| storage(ArchiveStorageOperation::Seek))?;
    let digest =
        Sha256Digest::parse(&hash_reader(&mut output)?).map_err(|_| registry_unavailable())?;
    output
        .seek(SeekFrom::Start(0))
        .map_err(|_| storage(ArchiveStorageOperation::Seek))?;
    let _ = copied;
    Ok((canonical, source_identity, path, output, identity, digest))
}

fn preflight_storage(staging: &Path, size: u64) -> Result<(), ArchiveSelectionError> {
    let required = size.saturating_add(STORAGE_MARGIN);
    let disks = Disks::new_with_refreshed_list();
    let found = disks
        .list()
        .iter()
        .filter(|disk| staging.starts_with(disk.mount_point()))
        .max_by_key(|disk| disk.mount_point().components().count());
    if let Some(disk) = found {
        let available = disk.available_space();
        if available < required {
            return Err(ArchiveSelectionError::InsufficientStorage {
                required,
                available,
            });
        }
    } else {
        tracing::warn!(name: "archive.staging.mount.unmatched", "archive staging mount was not reported; proceeding without preflight");
    }
    Ok(())
}
fn copy_error(error: std::io::Error, size: u64, staging: &Path) -> ArchiveSelectionError {
    if matches!(error.raw_os_error(), Some(28 | 112)) {
        let available = Disks::new_with_refreshed_list()
            .list()
            .iter()
            .filter(|d| staging.starts_with(d.mount_point()))
            .max_by_key(|d| d.mount_point().components().count())
            .map_or(0, |d| d.available_space());
        ArchiveSelectionError::InsufficientStorage {
            required: size.saturating_add(STORAGE_MARGIN),
            available,
        }
    } else {
        storage(ArchiveStorageOperation::CopySource)
    }
}
fn initialize_session(root: &Path) -> Result<(PathBuf, File), ArchiveSelectionError> {
    std::fs::create_dir_all(root)
        .map_err(|_| storage(ArchiveStorageOperation::CreateStagingDirectory))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(root, std::fs::Permissions::from_mode(0o700))
            .map_err(|_| storage(ArchiveStorageOperation::CreateStagingDirectory))?;
    }
    let root =
        std::fs::canonicalize(root).map_err(|_| storage(ArchiveStorageOperation::Canonicalize))?;
    let cleanup_path = root.join(ROOT_CLEANUP_LOCK_NAME);
    if cleanup_path
        .symlink_metadata()
        .is_ok_and(|metadata| !metadata.file_type().is_file())
    {
        return Err(storage(ArchiveStorageOperation::StartupCleanup));
    }
    let cleanup_lock = open_private_file(&cleanup_path, false)?;
    cleanup_lock
        .lock()
        .map_err(|_| storage(ArchiveStorageOperation::StartupCleanup))?;
    sweep_orphans(&root)?;

    for _ in 0..16 {
        let session_dir = root.join(format!("{SESSION_PREFIX}{}", Uuid::new_v4()));
        match std::fs::create_dir(&session_dir) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(_) => {
                return Err(storage(ArchiveStorageOperation::CreateStagingDirectory));
            }
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if std::fs::set_permissions(&session_dir, std::fs::Permissions::from_mode(0o700))
                .is_err()
            {
                let _ = std::fs::remove_dir(&session_dir);
                return Err(storage(ArchiveStorageOperation::CreateStagingDirectory));
            }
        }
        let lock_path = session_dir.join(SESSION_LOCK_NAME);
        let session_lock = match open_private_file(&lock_path, true) {
            Ok(file) => file,
            Err(error) => {
                let _ = std::fs::remove_file(&lock_path);
                let _ = std::fs::remove_dir(&session_dir);
                return Err(error);
            }
        };
        if session_lock.lock().is_err() {
            let _ = std::fs::remove_file(&lock_path);
            let _ = std::fs::remove_dir(&session_dir);
            return Err(storage(ArchiveStorageOperation::StartupCleanup));
        }
        return Ok((session_dir, session_lock));
    }
    Err(storage(ArchiveStorageOperation::CreateStagingDirectory))
}

fn open_private_file(path: &Path, create_new: bool) -> Result<File, ArchiveSelectionError> {
    let mut options = OpenOptions::new();
    options.read(true).write(true);
    if create_new {
        options.create_new(true);
    } else {
        options.create(true);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = options
        .open(path)
        .map_err(|_| storage(ArchiveStorageOperation::StartupCleanup))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(std::fs::Permissions::from_mode(0o600))
            .map_err(|_| storage(ArchiveStorageOperation::StartupCleanup))?;
    }
    Ok(file)
}

fn valid_session_name(name: &str) -> bool {
    name.strip_prefix(SESSION_PREFIX)
        .and_then(|value| Uuid::parse_str(value).ok().map(|uuid| (value, uuid)))
        .is_some_and(|(value, uuid)| value == uuid.to_string())
}

fn sweep_orphans(root: &Path) -> Result<(), ArchiveSelectionError> {
    for item in
        std::fs::read_dir(root).map_err(|_| storage(ArchiveStorageOperation::StartupCleanup))?
    {
        let item = item.map_err(|_| storage(ArchiveStorageOperation::StartupCleanup))?;
        let file_type = item
            .file_type()
            .map_err(|_| storage(ArchiveStorageOperation::StartupCleanup))?;
        let name = item.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        if file_type.is_file() && owned_name(name) {
            remove_file_benign(&item.path())?;
            continue;
        }
        if !file_type.is_dir() || !valid_session_name(name) {
            continue;
        }
        let lock_path = item.path().join(SESSION_LOCK_NAME);
        let lock_metadata = match std::fs::symlink_metadata(&lock_path) {
            Ok(metadata) if metadata.file_type().is_file() => metadata,
            Ok(_) => continue,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(_) => return Err(storage(ArchiveStorageOperation::StartupCleanup)),
        };
        let _ = lock_metadata;
        let session_lock = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&lock_path)
            .map_err(|_| storage(ArchiveStorageOperation::StartupCleanup))?;
        match session_lock.try_lock() {
            Ok(()) => {}
            Err(std::fs::TryLockError::WouldBlock) => continue,
            Err(std::fs::TryLockError::Error(_)) => {
                return Err(storage(ArchiveStorageOperation::StartupCleanup));
            }
        }
        for child in std::fs::read_dir(item.path())
            .map_err(|_| storage(ArchiveStorageOperation::StartupCleanup))?
        {
            let child = child.map_err(|_| storage(ArchiveStorageOperation::StartupCleanup))?;
            let child_type = child
                .file_type()
                .map_err(|_| storage(ArchiveStorageOperation::StartupCleanup))?;
            let child_name = child.file_name();
            if child_type.is_file() && child_name.to_str().is_some_and(|value| owned_name(value)) {
                remove_file_benign(&child.path())?;
            }
        }
        let has_preserved_entries = std::fs::read_dir(item.path())
            .map_err(|_| storage(ArchiveStorageOperation::StartupCleanup))?
            .filter_map(Result::ok)
            .any(|child| child.file_name() != SESSION_LOCK_NAME);
        if !has_preserved_entries {
            drop(session_lock);
            remove_file_benign(&lock_path)?;
            match std::fs::remove_dir(item.path()) {
                Ok(()) => {}
                Err(error)
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::NotFound | std::io::ErrorKind::DirectoryNotEmpty
                    ) => {}
                Err(_) => return Err(storage(ArchiveStorageOperation::StartupCleanup)),
            }
        }
    }
    Ok(())
}
fn owned_name(name: &str) -> bool {
    name.strip_prefix(STAGED_PREFIX)
        .and_then(|value| value.strip_suffix(STAGED_SUFFIX))
        .and_then(|value| Uuid::parse_str(value).ok().map(|uuid| (value, uuid)))
        .is_some_and(|(value, uuid)| value == uuid.to_string())
}
fn remove_file_benign(path: &Path) -> Result<(), ArchiveSelectionError> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err(storage(ArchiveStorageOperation::StartupCleanup)),
    }
}

fn delete_paths(paths: Vec<PathBuf>) {
    for path in paths {
        if let Err(error) = std::fs::remove_file(path) {
            if error.kind() != std::io::ErrorKind::NotFound {
                tracing::warn!(name: "archive.staging.cleanup.failed", "failed to remove owned staged archive");
            }
        }
    }
}
fn delete_paths_async(paths: Vec<PathBuf>) {
    if paths.is_empty() {
        return;
    }
    tauri::async_runtime::spawn_blocking(move || delete_paths(paths));
}
async fn delete_paths_await(paths: Vec<PathBuf>) {
    if paths.is_empty() {
        return;
    }
    if tokio::task::spawn_blocking(move || delete_paths(paths))
        .await
        .is_err()
    {
        tracing::warn!(name: "archive.staging.cleanup.failed", "staged archive cleanup task failed");
    }
}
async fn best_effort_delete(path: PathBuf) {
    let failed = tokio::task::spawn_blocking(move || match std::fs::remove_file(path) {
        Ok(()) => false,
        Err(error) => error.kind() != std::io::ErrorKind::NotFound,
    })
    .await
    .unwrap_or(true);
    if failed {
        tracing::warn!(name: "archive.staging.cleanup.failed", "failed to remove owned staged archive");
    }
}
fn verify_digest_text(text: String, expected: &Sha256Digest) -> Result<(), ArchiveSelectionError> {
    let actual = Sha256Digest::parse(&text).map_err(|_| registry_unavailable())?;
    if &actual == expected {
        Ok(())
    } else {
        Err(ArchiveSelectionError::DigestMismatch {
            expected: expected.clone(),
            actual,
        })
    }
}
async fn verify_file_digest(
    file: Arc<File>,
    expected: &Sha256Digest,
) -> Result<(), ArchiveSelectionError> {
    let expected = expected.clone();
    tokio::task::spawn_blocking(move || {
        let mut file = file
            .try_clone()
            .map_err(|_| storage(ArchiveStorageOperation::Open))?;
        file.seek(SeekFrom::Start(0))
            .map_err(|_| storage(ArchiveStorageOperation::Seek))?;
        verify_digest_text(hash_reader(&mut file)?, &expected)
    })
    .await
    .map_err(|_| registry_unavailable())?
}
fn prune_locked(registry: &mut Registry, now: Instant) -> Vec<PathBuf> {
    let ids = registry
        .selections
        .iter()
        .filter(|(_, value)| {
            !value.in_progress
                && value.publication_token.is_none()
                && now.duration_since(value.last_access) >= SELECTION_TTL
        })
        .map(|(id, _)| id.clone())
        .collect::<Vec<_>>();
    ids.into_iter()
        .filter_map(|id| {
            let value = registry.selections.remove(&id)?;
            tombstone(
                registry,
                id,
                value.publisher.clone(),
                TombstoneState::Expired,
            );
            Some(value.path)
        })
        .collect()
}
fn bound<'a>(
    registry: &'a mut Registry,
    id: &ArchiveSelectionId,
    expected: &str,
    active: &str,
) -> Result<&'a mut Selection, ArchiveSelectionError> {
    require_account(expected, active)?;
    if let Some(tombstone) = registry
        .tombstones
        .iter()
        .find(|value| value.id == id.as_str() && value.publisher == expected)
    {
        return match tombstone.state {
            TombstoneState::Expired => Err(ArchiveSelectionError::Expired {
                selection_id: id.clone(),
            }),
            TombstoneState::Discarded => Err(ArchiveSelectionError::Handle {
                selection_id: id.clone(),
                failure: ArchiveHandleFailure::Discarded,
            }),
        };
    }
    let value = registry
        .selections
        .get_mut(id.as_str())
        .ok_or_else(|| unknown(id.clone()))?;
    if value.publisher != expected {
        return Err(unknown(id.clone()));
    }
    value.last_access = Instant::now();
    Ok(value)
}
fn tombstone(registry: &mut Registry, id: String, publisher: String, state: TombstoneState) {
    registry.tombstones.push_back(Tombstone {
        id,
        publisher,
        state,
    });
    while registry.tombstones.len() > MAX_TOMBSTONES {
        registry.tombstones.pop_front();
    }
}
fn require_account(expected: &str, active: &str) -> Result<(), ArchiveSelectionError> {
    if expected == active {
        Ok(())
    } else {
        Err(account_mismatch())
    }
}
fn account_mismatch() -> ArchiveSelectionError {
    ArchiveSelectionError::Account {
        failure: ArchiveAccountFailure::AccountMismatch,
    }
}
fn restat(path: &Path, identity: &FileIdentity) -> Result<(), ArchiveSelectionError> {
    let metadata =
        std::fs::metadata(path).map_err(|_| storage(ArchiveStorageOperation::Metadata))?;
    if !metadata.is_file() {
        return Err(ArchiveSelectionError::NonRegularFile);
    }
    identity.check(&FileIdentity::new(&metadata))
}
fn selection_dto(id: &ArchiveSelectionId, value: &Selection) -> ArchiveFileSelection {
    ArchiveFileSelection {
        selection_id: id.clone(),
        safe_filename: value.filename.clone(),
        file_size: value.identity.size,
        expires_at_unix: unix_now().saturating_add(SELECTION_TTL.as_secs()),
    }
}

fn build_ready(
    selection: ArchiveFileSelection,
    format: PackageFormat,
    expected: Option<PackageFormat>,
    inspection: ArchiveInspection,
) -> ReadySnapshot {
    let extension = extension_format(selection.safe_filename.as_str());
    let mismatch = (expected.as_ref().is_some_and(|v| v != &format)
        || extension.as_ref().is_some_and(|v| v != &format))
    .then(|| ExtensionMismatchWarningDetail {
        extension_format: extension.clone(),
        detected_format: format.clone(),
    });
    let files = inspection
        .index
        .entries
        .iter()
        .filter(|v| matches!(v.kind, EntryKind::File))
        .filter_map(|v| {
            RelativePath::parse(&v.raw_path)
                .ok()
                .map(|path| SelectableArchiveFile {
                    path,
                    declared_size: v.declared_size,
                })
        })
        .collect::<Vec<_>>();
    let directories = inspection
        .index
        .working_directories()
        .into_iter()
        .filter_map(|v| {
            WorkingDir::parse(&v)
                .ok()
                .map(|directory| SelectableWorkingDirectory { directory })
        })
        .collect::<Vec<_>>();
    let hazards = map_hazards(&inspection);
    let warnings = map_warnings(&inspection, mismatch.clone());
    let hc = hazard_counts(&hazards);
    let wc = warning_counts(&warnings);
    let mut candidates = HashMap::new();
    for os in ["windows", "linux", "macos"] {
        for arch in ["x86_64", "aarch64"] {
            let platform = PlatformTarget {
                os: os.into(),
                arch: arch.into(),
            };
            candidates.insert(
                format!("{os}-{arch}"),
                detect_executable_candidates(&inspection.index, &platform)
                    .into_iter()
                    .map(map_candidate)
                    .collect(),
            );
        }
    }
    let counts = ArchiveInspectionCounts {
        logical_entries: inspection.index.entries.len() as u64
            + inspection.hazards.undecodable_paths.len() as u64,
        indexed_entries: inspection.index.entries.len() as u64,
        undecodable_entries: inspection.hazards.undecodable_paths.len() as u64,
        files: files.len() as u64,
        directory_entries: inspection
            .index
            .entries
            .iter()
            .filter(|v| matches!(v.kind, EntryKind::Directory))
            .count() as u64,
        symlinks: inspection.hazards.symlinks.len() as u64,
        hardlinks: inspection.hazards.hard_links.len() as u64,
        other_entry_kinds: inspection.hazards.other_entry_kinds.len() as u64,
        working_directories: directories.len() as u64,
        hazards: hc,
        warnings: wc,
    };
    ReadySnapshot {
        summary: ArchiveSelectionSummary {
            selection,
            detected_format: format,
            extension_format: extension,
            extension_mismatch_warning: mismatch,
            counts,
            blocking_hazards: !inspection.hazards.is_empty(),
        },
        inspection,
        files,
        directories,
        candidates,
        hazards,
        warnings,
    }
}

fn canonical_platform(value: &PlatformTarget) -> Result<PlatformTarget, ArchiveSelectionError> {
    let os = normalize_os(&value.os).ok_or_else(|| ArchiveSelectionError::InvalidPlatform {
        platform: value.clone(),
        failure: ArchivePlatformFailure::UnsupportedOs {
            os: safe(&value.os),
        },
    })?;
    let arch =
        normalize_arch(&value.arch).ok_or_else(|| ArchiveSelectionError::InvalidPlatform {
            platform: value.clone(),
            failure: ArchivePlatformFailure::UnsupportedArch {
                arch: safe(&value.arch),
            },
        })?;
    let canonical = PlatformTarget { os, arch };
    if &canonical != value {
        return Err(ArchiveSelectionError::InvalidPlatform {
            platform: value.clone(),
            failure: ArchivePlatformFailure::NonCanonical { canonical },
        });
    }
    Ok(canonical)
}
fn extension_format(v: &str) -> Option<PackageFormat> {
    let v = v.to_lowercase();
    if v.ends_with(".zip") {
        Some(PackageFormat::Zip)
    } else if v.ends_with(".tar.gz") || v.ends_with(".tgz") {
        Some(PackageFormat::TarGz)
    } else if v.ends_with(".tar.zst") || v.ends_with(".tzst") {
        Some(PackageFormat::TarZstd)
    } else {
        None
    }
}
fn validate_page(page: ArchivePageRequest) -> Result<ArchivePageRequest, ArchiveSelectionError> {
    page.validate()
        .map_err(|failure| ArchiveSelectionError::Pagination { failure })
}

fn page_filter<T: Clone, F: Fn(&T) -> &str>(
    items: &[T],
    search: &str,
    page: ArchivePageRequest,
    path: F,
) -> Result<ArchivePage<T>, ArchiveSelectionError> {
    let needle = search.to_lowercase();
    let values = items
        .iter()
        .filter(|v| path(v).to_lowercase().contains(&needle))
        .cloned()
        .collect::<Vec<_>>();
    page_all(&values, page)
}
fn page_all<T: Clone>(
    items: &[T],
    page: ArchivePageRequest,
) -> Result<ArchivePage<T>, ArchiveSelectionError> {
    let page = validate_page(page)?;
    let total = items.len() as u64;
    let offset = page.offset();
    if offset > total {
        return Err(ArchiveSelectionError::Pagination {
            failure: ArchivePaginationFailure::OffsetOutOfRange { offset, total },
        });
    }
    let start = offset as usize;
    let end = start.saturating_add(page.limit() as usize).min(items.len());
    Ok(ArchivePage {
        offset,
        total,
        next_offset: (end < items.len()).then_some(end as u64),
        items: items[start..end].to_vec(),
    })
}
fn hash_reader(file: &mut File) -> Result<String, ArchiveSelectionError> {
    file.seek(SeekFrom::Start(0))
        .map_err(|_| storage(ArchiveStorageOperation::Seek))?;
    let mut h = Sha256::new();
    let mut b = [0; 65536];
    loop {
        let n = file
            .read(&mut b)
            .map_err(|_| storage(ArchiveStorageOperation::Read))?;
        if n == 0 {
            break;
        }
        h.update(&b[..n]);
    }
    Ok(format!("{:x}", h.finalize()))
}
fn map_candidate(v: arcadestr_core::archive_inspection::ExecutableCandidate) -> ArchiveCandidate {
    ArchiveCandidate {
        path: v.path,
        confidence: match v.confidence {
            CandidateConfidence::High => ArchiveCandidateConfidence::High,
            CandidateConfidence::Medium => ArchiveCandidateConfidence::Medium,
            CandidateConfidence::Low => ArchiveCandidateConfidence::Low,
        },
        reason: match v.reason {
            CandidateReason::GodotPackageBesideBinary => {
                ArchiveCandidateReason::GodotPackageBesideBinary
            }
            CandidateReason::UnityDataDirectoryBesideBinary => {
                ArchiveCandidateReason::UnityDataDirectoryBesideBinary
            }
            CandidateReason::UnrealWin64Layout => ArchiveCandidateReason::UnrealWin64Layout,
            CandidateReason::WindowsExecutableExtension { archive_root } => {
                ArchiveCandidateReason::WindowsExecutableExtension { archive_root }
            }
            CandidateReason::UnixExecutableMode { archive_root } => {
                ArchiveCandidateReason::UnixExecutableMode { archive_root }
            }
        },
    }
}
fn map_hazards(i: &ArchiveInspection) -> HashMap<ArchiveHazardCategory, Vec<ArchiveHazardDetail>> {
    let h = &i.hazards;
    let mut m = HashMap::new();
    m.insert(
        ArchiveHazardCategory::PathRule,
        h.invalid_paths
            .iter()
            .map(|(p, e)| ArchiveHazardDetail::PathRule {
                display_path: safe(p),
                code: map_path(e),
            })
            .collect(),
    );
    m.insert(
        ArchiveHazardCategory::Symlink,
        h.symlinks
            .iter()
            .map(|p| ArchiveHazardDetail::Symlink {
                display_path: safe(p),
            })
            .collect(),
    );
    m.insert(
        ArchiveHazardCategory::Hardlink,
        h.hard_links
            .iter()
            .map(|p| ArchiveHazardDetail::Hardlink {
                display_path: safe(p),
            })
            .collect(),
    );
    m.insert(
        ArchiveHazardCategory::Collision,
        h.case_collisions
            .iter()
            .map(|p| ArchiveHazardDetail::Collision {
                display_paths: p.iter().map(|v| safe(v)).collect(),
            })
            .collect(),
    );
    m.insert(
        ArchiveHazardCategory::Other,
        h.other_entry_kinds
            .iter()
            .map(|p| ArchiveHazardDetail::Other {
                display_path: safe(p),
            })
            .collect(),
    );
    m.insert(
        ArchiveHazardCategory::Undecodable,
        h.undecodable_paths
            .iter()
            .map(|p| ArchiveHazardDetail::Undecodable {
                lossy: safe(&p.lossy),
                byte_length: p.bytes.len() as u64,
            })
            .collect(),
    );
    m.insert(
        ArchiveHazardCategory::TypeConflict,
        h.container_type_name_conflicts
            .iter()
            .map(|p| ArchiveHazardDetail::TypeConflict {
                display_path: safe(&p.entry),
                container_kind: match p.kind {
                    EntryKind::File => ArchiveEntryKind::File,
                    EntryKind::Directory => ArchiveEntryKind::Directory,
                    EntryKind::Symlink => ArchiveEntryKind::Symlink,
                    EntryKind::HardLink => ArchiveEntryKind::Hardlink,
                    EntryKind::Other => ArchiveEntryKind::Other,
                },
            })
            .collect(),
    );
    m.insert(
        ArchiveHazardCategory::ZipCountMismatch,
        h.zip_declared_entry_count_mismatch
            .iter()
            .map(|p| ArchiveHazardDetail::ZipCountMismatch {
                declared: p.declared,
                parsed: p.parsed,
            })
            .collect(),
    );
    m
}
fn map_warnings(
    i: &ArchiveInspection,
    x: Option<ExtensionMismatchWarningDetail>,
) -> HashMap<ArchiveWarningCategory, Vec<ArchiveWarningDetail>> {
    let mut m = HashMap::new();
    m.insert(
        ArchiveWarningCategory::EfsDivergence,
        i.warnings
            .entries
            .iter()
            .map(|v| match v {
                ArchiveWarning::ZipEfsUnsetUtf8Cp437Divergence { bytes, utf8, cp437 } => {
                    ArchiveWarningDetail::EfsDivergence(EfsDivergenceWarningDetail {
                        utf8: safe(utf8),
                        cp437: safe(cp437),
                        byte_length: bytes.len() as u64,
                    })
                }
            })
            .collect(),
    );
    m.insert(
        ArchiveWarningCategory::ExtensionMismatch,
        x.into_iter()
            .map(ArchiveWarningDetail::ExtensionMismatch)
            .collect(),
    );
    m
}
fn hazard_counts(
    m: &HashMap<ArchiveHazardCategory, Vec<ArchiveHazardDetail>>,
) -> ArchiveHazardCounts {
    let n = |k| m.get(&k).map_or(0, |v| v.len() as u64);
    ArchiveHazardCounts {
        path_rule: n(ArchiveHazardCategory::PathRule),
        symlink: n(ArchiveHazardCategory::Symlink),
        hardlink: n(ArchiveHazardCategory::Hardlink),
        collision: n(ArchiveHazardCategory::Collision),
        other: n(ArchiveHazardCategory::Other),
        undecodable: n(ArchiveHazardCategory::Undecodable),
        type_conflict: n(ArchiveHazardCategory::TypeConflict),
        zip_count_mismatch: n(ArchiveHazardCategory::ZipCountMismatch),
    }
}
fn warning_counts(
    m: &HashMap<ArchiveWarningCategory, Vec<ArchiveWarningDetail>>,
) -> ArchiveWarningCounts {
    ArchiveWarningCounts {
        efs_divergence: m
            .get(&ArchiveWarningCategory::EfsDivergence)
            .map_or(0, |v| v.len() as u64),
        extension_mismatch: m
            .get(&ArchiveWarningCategory::ExtensionMismatch)
            .map_or(0, |v| v.len() as u64),
    }
}
fn map_path(e: &PathError) -> ArchivePathRuleCode {
    match e {
        PathError::Rule1Absolute => ArchivePathRuleCode::Rule1Absolute,
        PathError::Rule2ParentSegment => ArchivePathRuleCode::Rule2ParentSegment,
        PathError::Rule3Backslash => ArchivePathRuleCode::Rule3Backslash,
        PathError::Rule4ColonOrPrefix => ArchivePathRuleCode::Rule4ColonOrPrefix,
        PathError::Rule5ControlCharacter => ArchivePathRuleCode::Rule5ControlCharacter,
        PathError::Rule6EmptyOrDotSegment => ArchivePathRuleCode::Rule6EmptyOrDotSegment,
        PathError::Rule7TrailingSpaceOrPeriod => ArchivePathRuleCode::Rule7TrailingSpaceOrPeriod,
        PathError::Rule8ReservedDeviceName => ArchivePathRuleCode::Rule8ReservedDeviceName,
        PathError::Rule9LengthExceeded => ArchivePathRuleCode::Rule9LengthExceeded,
    }
}
fn map_detect(e: DetectInspectError) -> ArchiveSelectionError {
    match e{DetectInspectError::Detection(v)=>ArchiveSelectionError::Detection{failure:match v{arcadestr_core::archive_inspection::ArchiveFormatDetectionError::UnknownFormat=>ArchiveDetectionFailure::UnknownFormat,arcadestr_core::archive_inspection::ArchiveFormatDetectionError::AmbiguousFormat=>ArchiveDetectionFailure::AmbiguousFormat,arcadestr_core::archive_inspection::ArchiveFormatDetectionError::TruncatedSignature=>ArchiveDetectionFailure::TruncatedSignature}},DetectInspectError::Inspection(v)=>ArchiveSelectionError::Inspection{failure:map_inspection(v)},DetectInspectError::Read=>storage(ArchiveStorageOperation::Seek)}
}
fn map_inspection(e: InspectionError) -> ArchiveInspectionFailure {
    match e {
        InspectionError::UnsupportedCompressionMethod { entry: _, method } => {
            ArchiveInspectionFailure::UnsupportedCompressionMethod {
                entry: "archive entry".into(),
                method,
            }
        }
        InspectionError::EntryLimitExceeded { limit } => {
            ArchiveInspectionFailure::EntryLimitExceeded { limit }
        }
        InspectionError::EntryCountOverflow => ArchiveInspectionFailure::EntryCountOverflow,
        InspectionError::ZipCentralDirectoryLimitExceeded {
            declared_size,
            limit,
        } => ArchiveInspectionFailure::ZipCentralDirectoryLimitExceeded {
            declared_size,
            limit,
        },
        InspectionError::ZipPreflightMalformed { reason } => {
            ArchiveInspectionFailure::ZipPreflightMalformed {
                reason: match reason {
                    ZipPreflightError::EndOfCentralDirectoryNotFound => {
                        ArchiveZipPreflightFailure::EndOfCentralDirectoryNotFound
                    }
                    ZipPreflightError::Zip64LocatorMissing => {
                        ArchiveZipPreflightFailure::Zip64LocatorMissing
                    }
                    ZipPreflightError::Zip64RecordsMalformed => {
                        ArchiveZipPreflightFailure::Zip64RecordsMalformed
                    }
                    ZipPreflightError::MultiDiskUnsupported => {
                        ArchiveZipPreflightFailure::MultiDiskUnsupported
                    }
                    ZipPreflightError::ArithmeticOverflow => {
                        ArchiveZipPreflightFailure::ArithmeticOverflow
                    }
                },
            }
        }
        InspectionError::MeasuredInputOverflow => ArchiveInspectionFailure::MeasuredInputOverflow,
        InspectionError::MeasuredOutputOverflow => ArchiveInspectionFailure::MeasuredOutputOverflow,
        InspectionError::ZeroMeasuredInput { measured_output } => {
            ArchiveInspectionFailure::ZeroMeasuredInput { measured_output }
        }
        InspectionError::CompressionRatioExceeded {
            measured_input,
            measured_output,
            maximum_ratio,
        } => ArchiveInspectionFailure::CompressionRatioExceeded {
            measured_input,
            measured_output,
            maximum_ratio,
        },
        InspectionError::MeasuredInputCounterRegressed => {
            ArchiveInspectionFailure::MeasuredInputCounterRegressed
        }
        InspectionError::TarMetadataLimitExceeded {
            measured_metadata,
            limit,
        } => ArchiveInspectionFailure::TarMetadataLimitExceeded {
            measured_metadata,
            limit,
        },
        InspectionError::TarDecoder { .. } => ArchiveInspectionFailure::TarDecoder {
            message: "archive decoder failed".into(),
        },
        InspectionError::TarParser { .. } => ArchiveInspectionFailure::TarParser {
            message: "archive parser failed".into(),
        },
        InspectionError::ZipParser { .. } => ArchiveInspectionFailure::ZipParser {
            message: "archive parser failed".into(),
        },
        InspectionError::ZipIo { .. } => ArchiveInspectionFailure::ZipIo {
            message: "archive read failed".into(),
        },
    }
}
fn safe(v: &str) -> String {
    v.chars().filter(|c| !c.is_control()).take(256).collect()
}
fn storage(operation: ArchiveStorageOperation) -> ArchiveSelectionError {
    ArchiveSelectionError::Storage { operation }
}
fn registry_unavailable() -> ArchiveSelectionError {
    ArchiveSelectionError::Registry {
        failure: ArchiveRegistryFailure::Unavailable,
    }
}
fn unknown(selection_id: ArchiveSelectionId) -> ArchiveSelectionError {
    ArchiveSelectionError::Handle {
        selection_id,
        failure: ArchiveHandleFailure::Unknown,
    }
}
fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |v| v.as_secs())
}

#[cfg(test)]
mod tests {
    use super::*;
    use arcadestr_core::archive_inspection::{
        ArchiveEntry, ArchiveHazards, ArchiveIndex, ArchiveWarnings,
    };
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tempfile::TempDir;

    const PUBLISHER: &str = "npub-test-publisher";

    fn service(root: &Path) -> ArchiveSelectionService {
        let (staging_dir, session_lock) = initialize_session(root).expect("initialize session");
        ArchiveSelectionService {
            registry: Arc::new(Mutex::new(Registry {
                selections: HashMap::new(),
                tombstones: VecDeque::new(),
                reservations: 0,
            })),
            staging_dir: Arc::new(staging_dir),
            _session_lock: Arc::new(session_lock),
        }
    }

    fn request_id() -> ArchiveRequestId {
        ArchiveRequestId::parse(&Uuid::new_v4().to_string()).expect("UUID v4 is a request ID")
    }

    fn account(value: &'static str) -> AccountCheck {
        Arc::new(move || Box::pin(async move { Ok(value.to_owned()) }))
    }

    #[test]
    fn reservation_release_under_held_guard_is_atomic_and_drop_is_noop() {
        let registry = Arc::new(Mutex::new(Registry {
            selections: HashMap::new(),
            tombstones: VecDeque::new(),
            reservations: 1,
        }));
        let mut reservation = ReservationGuard {
            registry: Arc::clone(&registry),
            held: true,
        };

        {
            let mut held_registry = registry.lock().expect("registry");
            reservation.release_into(&mut held_registry);
            assert_eq!(held_registry.reservations, 0);
        }
        drop(reservation);

        assert_eq!(registry.lock().expect("registry").reservations, 0);
    }

    #[test]
    fn reservation_drop_after_staging_failure_restores_capacity() {
        let registry = Arc::new(Mutex::new(Registry {
            selections: HashMap::new(),
            tombstones: VecDeque::new(),
            reservations: 1,
        }));
        let reservation = ReservationGuard {
            registry: Arc::clone(&registry),
            held: true,
        };

        drop(reservation);

        assert_eq!(registry.lock().expect("registry").reservations, 0);
    }

    fn crc32(bytes: &[u8]) -> u32 {
        let mut crc = !0_u32;
        for byte in bytes {
            crc ^= u32::from(*byte);
            for _ in 0..8 {
                crc = (crc >> 1) ^ (0xedb8_8320 & (0_u32.wrapping_sub(crc & 1)));
            }
        }
        !crc
    }

    fn zip(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut bytes = Vec::new();
        let mut central = Vec::new();
        for (name, data) in entries {
            let offset = bytes.len() as u32;
            let crc = crc32(data);
            bytes.extend_from_slice(&0x0403_4b50_u32.to_le_bytes());
            bytes.extend_from_slice(&[20, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
            bytes.extend_from_slice(&crc.to_le_bytes());
            bytes.extend_from_slice(&(data.len() as u32).to_le_bytes());
            bytes.extend_from_slice(&(data.len() as u32).to_le_bytes());
            bytes.extend_from_slice(&(name.len() as u16).to_le_bytes());
            bytes.extend_from_slice(&0_u16.to_le_bytes());
            bytes.extend_from_slice(name.as_bytes());
            bytes.extend_from_slice(data);

            central.extend_from_slice(&0x0201_4b50_u32.to_le_bytes());
            central.extend_from_slice(&[20, 0, 20, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
            central.extend_from_slice(&crc.to_le_bytes());
            central.extend_from_slice(&(data.len() as u32).to_le_bytes());
            central.extend_from_slice(&(data.len() as u32).to_le_bytes());
            central.extend_from_slice(&(name.len() as u16).to_le_bytes());
            central.extend_from_slice(&[0; 8]);
            central.extend_from_slice(&0_u32.to_le_bytes());
            central.extend_from_slice(&offset.to_le_bytes());
            central.extend_from_slice(name.as_bytes());
        }
        let central_offset = bytes.len() as u32;
        let central_size = central.len() as u32;
        bytes.extend_from_slice(&central);
        bytes.extend_from_slice(&0x0605_4b50_u32.to_le_bytes());
        bytes.extend_from_slice(&[0; 4]);
        bytes.extend_from_slice(&(entries.len() as u16).to_le_bytes());
        bytes.extend_from_slice(&(entries.len() as u16).to_le_bytes());
        bytes.extend_from_slice(&central_size.to_le_bytes());
        bytes.extend_from_slice(&central_offset.to_le_bytes());
        bytes.extend_from_slice(&0_u16.to_le_bytes());
        bytes
    }

    fn write_zip(root: &Path, name: &str, entries: &[(&str, &[u8])]) -> PathBuf {
        let path = root.join(name);
        std::fs::write(&path, zip(entries)).expect("write generated ZIP fixture");
        path
    }

    fn write_minimal_valid_archive(root: &Path, name: &str) -> PathBuf {
        write_zip(root, name, &[("game.exe", b"valid game bytes")])
    }

    fn inspect_request(selection_id: ArchiveSelectionId) -> InspectArchiveRequest {
        InspectArchiveRequest {
            request_id: request_id(),
            selection_id,
            expected_publisher_npub: PUBLISHER.into(),
            expected_format: Some(PackageFormat::Zip),
        }
    }

    async fn inspected(service: &ArchiveSelectionService, source: PathBuf) -> ArchiveFileSelection {
        let selected = service
            .stage_source(source, PUBLISHER, account(PUBLISHER))
            .await
            .expect("stage fixture");
        service
            .inspect(
                inspect_request(selected.selection_id.clone()),
                PUBLISHER,
                account(PUBLISHER),
            )
            .await
            .expect("inspect fixture");
        selected
    }

    fn hash_request(selection_id: ArchiveSelectionId) -> HashSelectedArchiveRequest {
        HashSelectedArchiveRequest {
            request_id: request_id(),
            selection_id,
            expected_publisher_npub: PUBLISHER.into(),
        }
    }

    async fn staged_path(service: &ArchiveSelectionService, id: &ArchiveSelectionId) -> PathBuf {
        service.registry.lock().expect("registry").selections[id.as_str()]
            .path
            .clone()
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a3_5_2_stage_is_private_and_isolated_from_source_mutation() {
        use std::os::unix::fs::PermissionsExt;

        // Arrange
        let temp = TempDir::new().expect("temp dir");
        std::fs::set_permissions(temp.path(), std::fs::Permissions::from_mode(0o700))
            .expect("set staging permissions");
        let source = write_zip(temp.path(), "source.zip", &[("bin/Game.EXE", b"original")]);
        let service = service(temp.path());

        // Act
        let selected = service
            .stage_source(source.clone(), PUBLISHER, account(PUBLISHER))
            .await
            .expect("stage");
        let path = staged_path(&service, &selected.selection_id).await;
        std::fs::write(source, b"changed source").expect("mutate original source");
        let inspection = service
            .inspect(
                inspect_request(selected.selection_id.clone()),
                PUBLISHER,
                account(PUBLISHER),
            )
            .await;
        let hash = service
            .hash(
                hash_request(selected.selection_id),
                PUBLISHER,
                account(PUBLISHER),
            )
            .await;

        // Assert
        assert_eq!(
            std::fs::metadata(temp.path())
                .expect("metadata")
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
        assert_eq!(
            std::fs::metadata(path)
                .expect("metadata")
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        assert!(inspection.is_ok());
        assert!(hash.is_ok());
    }

    #[tokio::test]
    async fn a3_5_2_account_binding_guessed_handles_and_tombstones_are_distinct() {
        // Arrange
        let temp = TempDir::new().expect("temp dir");
        let source = write_zip(temp.path(), "account.zip", &[("game.exe", b"x")]);
        let service = service(temp.path());
        let selected = inspected(&service, source).await;
        let guessed = ArchiveSelectionId::parse(&Uuid::new_v4().to_string()).expect("id");

        // Act
        let inspect_error = service
            .inspect(
                inspect_request(selected.selection_id.clone()),
                "other",
                account(PUBLISHER),
            )
            .await
            .expect_err("account mismatch");
        let hash_error = service
            .hash(
                hash_request(selected.selection_id.clone()),
                "other",
                account(PUBLISHER),
            )
            .await
            .expect_err("account mismatch");
        let query_error = service
            .query_files(
                QueryArchiveFilesRequest {
                    request_id: request_id(),
                    selection_id: selected.selection_id.clone(),
                    expected_publisher_npub: PUBLISHER.into(),
                    search: ArchiveSearch::parse("").expect("search"),
                    page: ArchivePageRequest::default(),
                },
                "other",
            )
            .await
            .expect_err("account mismatch");
        let directory_error = service
            .query_working_directories(
                QueryArchiveWorkingDirectoriesRequest {
                    request_id: request_id(),
                    selection_id: selected.selection_id.clone(),
                    expected_publisher_npub: PUBLISHER.into(),
                    search: ArchiveSearch::parse("").expect("search"),
                    page: ArchivePageRequest::default(),
                },
                "other",
            )
            .await
            .expect_err("account mismatch");
        let candidate_error = service
            .query_candidates(
                QueryArchiveCandidatesRequest {
                    request_id: request_id(),
                    selection_id: selected.selection_id.clone(),
                    expected_publisher_npub: PUBLISHER.into(),
                    search: ArchiveSearch::parse("").expect("search"),
                    platform: PlatformTarget {
                        os: "linux".into(),
                        arch: "x86_64".into(),
                    },
                    page: ArchivePageRequest::default(),
                },
                "other",
            )
            .await
            .expect_err("account mismatch");
        let hazard_error = service
            .query_hazards(
                QueryArchiveHazardsRequest {
                    request_id: request_id(),
                    selection_id: selected.selection_id.clone(),
                    expected_publisher_npub: PUBLISHER.into(),
                    category: ArchiveHazardCategory::Symlink,
                    page: ArchivePageRequest::default(),
                },
                "other",
            )
            .await
            .expect_err("account mismatch");
        let warning_error = service
            .query_warnings(
                QueryArchiveWarningsRequest {
                    request_id: request_id(),
                    selection_id: selected.selection_id.clone(),
                    expected_publisher_npub: PUBLISHER.into(),
                    category: ArchiveWarningCategory::ExtensionMismatch,
                    page: ArchivePageRequest::default(),
                },
                "other",
            )
            .await
            .expect_err("account mismatch");
        let discard_error = service
            .discard(
                DiscardArchiveSelectionRequest {
                    request_id: request_id(),
                    selection_id: selected.selection_id.clone(),
                    expected_publisher_npub: PUBLISHER.into(),
                },
                "other",
            )
            .await
            .expect_err("account mismatch");
        let cross_account_live = service
            .hash(
                HashSelectedArchiveRequest {
                    request_id: request_id(),
                    selection_id: selected.selection_id.clone(),
                    expected_publisher_npub: "other".into(),
                },
                "other",
                account("other"),
            )
            .await
            .expect_err("cross-account live handle is opaque");
        let guessed_error = service
            .hash(hash_request(guessed.clone()), PUBLISHER, account(PUBLISHER))
            .await
            .expect_err("unknown handle");
        service
            .discard(
                DiscardArchiveSelectionRequest {
                    request_id: request_id(),
                    selection_id: selected.selection_id.clone(),
                    expected_publisher_npub: PUBLISHER.into(),
                },
                PUBLISHER,
            )
            .await
            .expect("discard");
        let cross_account_discarded = service
            .hash(
                HashSelectedArchiveRequest {
                    request_id: request_id(),
                    selection_id: selected.selection_id.clone(),
                    expected_publisher_npub: "other".into(),
                },
                "other",
                account("other"),
            )
            .await
            .expect_err("cross-account discarded handle remains opaque");
        let discarded = service
            .hash(
                hash_request(selected.selection_id.clone()),
                PUBLISHER,
                account(PUBLISHER),
            )
            .await
            .expect_err("discarded");
        {
            let mut registry = service.registry.lock().expect("registry");
            tombstone(
                &mut registry,
                guessed.as_str().to_owned(),
                PUBLISHER.to_owned(),
                TombstoneState::Expired,
            );
        }
        let expired = service
            .hash(hash_request(guessed.clone()), PUBLISHER, account(PUBLISHER))
            .await
            .expect_err("expired");

        // Assert
        for error in [
            inspect_error,
            hash_error,
            query_error,
            directory_error,
            candidate_error,
            hazard_error,
            warning_error,
            discard_error,
        ] {
            assert_eq!(error, account_mismatch());
        }
        assert_eq!(cross_account_live, unknown(selected.selection_id.clone()));
        assert_eq!(
            cross_account_discarded,
            unknown(selected.selection_id.clone())
        );
        assert_eq!(guessed_error, unknown(guessed.clone()));
        assert!(matches!(
            discarded,
            ArchiveSelectionError::Handle {
                failure: ArchiveHandleFailure::Discarded,
                ..
            }
        ));
        assert!(matches!(expired, ArchiveSelectionError::Expired { .. }));
    }

    #[tokio::test]
    async fn a3_5_2_tombstones_preserve_state_beyond_legacy_churn_without_cross_account_leaks() {
        let temp = TempDir::new().expect("temp dir");
        let service = service(temp.path());
        let expired_id = ArchiveSelectionId::parse(&Uuid::new_v4().to_string()).expect("id");
        let discarded_id = ArchiveSelectionId::parse(&Uuid::new_v4().to_string()).expect("id");
        {
            let mut registry = service.registry.lock().expect("registry");
            tombstone(
                &mut registry,
                expired_id.as_str().to_owned(),
                PUBLISHER.to_owned(),
                TombstoneState::Expired,
            );
            tombstone(
                &mut registry,
                discarded_id.as_str().to_owned(),
                PUBLISHER.to_owned(),
                TombstoneState::Discarded,
            );
            for _ in 0..64 {
                tombstone(
                    &mut registry,
                    Uuid::new_v4().to_string(),
                    PUBLISHER.to_owned(),
                    TombstoneState::Discarded,
                );
            }
        }

        let expired = service
            .hash(
                hash_request(expired_id.clone()),
                PUBLISHER,
                account(PUBLISHER),
            )
            .await
            .expect_err("earliest expired state retained");
        let discarded = service
            .hash(
                hash_request(discarded_id.clone()),
                PUBLISHER,
                account(PUBLISHER),
            )
            .await
            .expect_err("earliest discarded state retained");
        let cross_account = service
            .hash(
                HashSelectedArchiveRequest {
                    request_id: request_id(),
                    selection_id: expired_id.clone(),
                    expected_publisher_npub: "other".into(),
                },
                "other",
                account("other"),
            )
            .await
            .expect_err("cross-account tombstone is opaque");

        assert!(matches!(expired, ArchiveSelectionError::Expired { .. }));
        assert!(matches!(
            discarded,
            ArchiveSelectionError::Handle {
                failure: ArchiveHandleFailure::Discarded,
                ..
            }
        ));
        assert_eq!(cross_account, unknown(expired_id));
    }

    #[tokio::test]
    async fn a3_5_2_cached_inspect_hash_and_lease_reject_staged_mutation() {
        // Arrange
        let temp = TempDir::new().expect("temp dir");
        let source = write_minimal_valid_archive(temp.path(), "mutation.zip");
        let service = service(temp.path());
        let selected = inspected(&service, source).await;
        let digest = service
            .hash(
                hash_request(selected.selection_id.clone()),
                PUBLISHER,
                account(PUBLISHER),
            )
            .await
            .expect("hash")
            .sha256;
        let path = staged_path(&service, &selected.selection_id).await;
        std::fs::write(path, b"mutated staged bytes").expect("mutate staged file");

        // Act
        let inspect_error = service
            .inspect(
                inspect_request(selected.selection_id.clone()),
                PUBLISHER,
                account(PUBLISHER),
            )
            .await
            .expect_err("cached inspect mutation");
        let hash_error = service
            .hash(
                hash_request(selected.selection_id.clone()),
                PUBLISHER,
                account(PUBLISHER),
            )
            .await
            .expect_err("hash mutation");
        let lease_error = service
            .publication_lease(
                &BuildSelectionRef {
                    selection_id: selected.selection_id,
                    sha256: digest,
                },
                PUBLISHER,
                PUBLISHER,
                account(PUBLISHER),
            )
            .await
            .expect_err("lease mutation");

        // Assert
        assert!(matches!(
            inspect_error,
            ArchiveSelectionError::DigestMismatch { .. }
        ));
        assert!(matches!(
            hash_error,
            ArchiveSelectionError::DigestMismatch { .. }
        ));
        assert!(matches!(
            lease_error,
            ArchiveSelectionError::DigestMismatch { .. }
        ));
    }

    #[tokio::test]
    async fn a3_5_2_cached_inspect_restated_source_reports_mutation() {
        // Arrange
        let temp = TempDir::new().expect("temp dir");
        let source = write_minimal_valid_archive(temp.path(), "source-mutation.zip");
        let service = service(temp.path());
        let selected = inspected(&service, source.clone()).await;
        std::fs::write(source, b"changed source bytes").expect("mutate source");

        // Act
        let error = service
            .inspect(
                inspect_request(selected.selection_id),
                PUBLISHER,
                account(PUBLISHER),
            )
            .await
            .expect_err("cached source mutation");

        // Assert
        assert!(matches!(
            error,
            ArchiveSelectionError::Mutation {
                failure: ArchiveMutationFailure::SizeChanged { .. }
            }
        ));
    }

    #[tokio::test]
    async fn a3_5_2_account_change_after_work_does_not_commit_and_clears_progress() {
        // Arrange
        let temp = TempDir::new().expect("temp dir");
        let source = write_zip(temp.path(), "changed.zip", &[("game.exe", b"x")]);
        let service = service(temp.path());
        let selected = service
            .stage_source(source, PUBLISHER, account(PUBLISHER))
            .await
            .expect("stage");
        let baseline_digest = service.registry.lock().expect("registry").selections
            [selected.selection_id.as_str()]
        .digest
        .clone();
        let calls = Arc::new(AtomicUsize::new(0));
        let check: AccountCheck = Arc::new({
            let calls = Arc::clone(&calls);
            move || {
                calls.fetch_add(1, Ordering::SeqCst);
                Box::pin(async { Ok("other".to_owned()) })
            }
        });

        // Act
        let inspect_error = service
            .inspect(
                inspect_request(selected.selection_id.clone()),
                PUBLISHER,
                check,
            )
            .await
            .expect_err("changed account");
        let after_inspect = service.registry.lock().expect("registry").selections
            [selected.selection_id.as_str()]
        .in_progress;
        let hash_error = service
            .hash(
                hash_request(selected.selection_id.clone()),
                PUBLISHER,
                account("other"),
            )
            .await
            .expect_err("changed account");
        let registry = service.registry.lock().expect("registry");
        let entry = &registry.selections[selected.selection_id.as_str()];

        // Assert
        assert_eq!(inspect_error, account_mismatch());
        assert_eq!(hash_error, account_mismatch());
        assert!(!after_inspect);
        assert!(!entry.in_progress);
        assert!(entry.ready.is_none());
        assert_eq!(entry.digest, baseline_digest);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn a3_5_2_capacity_counts_uninspected_selections() {
        // Arrange
        let temp = TempDir::new().expect("temp dir");
        let service = service(temp.path());

        // Act
        for index in 0..MAX_LIVE_SELECTIONS {
            let source = write_zip(temp.path(), &format!("{index}.zip"), &[("game", b"x")]);
            service
                .stage_source(source, PUBLISHER, account(PUBLISHER))
                .await
                .expect("first four fit");
        }
        let fifth = write_zip(temp.path(), "fifth.zip", &[("game", b"x")]);
        let error = service
            .stage_source(fifth, PUBLISHER, account(PUBLISHER))
            .await
            .expect_err("fifth rejected");

        // Assert
        assert!(matches!(
            error,
            ArchiveSelectionError::Registry {
                failure: ArchiveRegistryFailure::CapacityExceeded
            }
        ));
        assert_eq!(
            service.registry.lock().expect("registry").selections.len(),
            MAX_LIVE_SELECTIONS
        );
    }

    #[tokio::test]
    async fn a3_5_2_publication_leases_block_discard_survive_prune_drop_and_finish() {
        // Arrange
        let temp = TempDir::new().expect("temp dir");
        let source = write_minimal_valid_archive(temp.path(), "lease.zip");
        let service = service(temp.path());
        let selected = inspected(&service, source).await;
        let digest = service
            .hash(
                hash_request(selected.selection_id.clone()),
                PUBLISHER,
                account(PUBLISHER),
            )
            .await
            .expect("hash")
            .sha256;
        let reference = BuildSelectionRef {
            selection_id: selected.selection_id.clone(),
            sha256: digest.clone(),
        };
        let path = staged_path(&service, &selected.selection_id).await;
        let lease = service
            .publication_lease(&reference, PUBLISHER, PUBLISHER, account(PUBLISHER))
            .await
            .expect("lease");
        let forged = PublicationLease {
            file_size: 0,
            service: service.clone(),
            id: selected.selection_id.clone(),
            token: Uuid::new_v4(),
            digest: digest.clone(),
            released: true,
        };
        assert!(service.private_lease_file(&forged).is_err());
        let concurrent = service
            .publication_lease(&reference, PUBLISHER, PUBLISHER, account(PUBLISHER))
            .await
            .expect_err("publication lease is exclusive");
        let inspect_busy = service
            .inspect(
                inspect_request(selected.selection_id.clone()),
                PUBLISHER,
                account(PUBLISHER),
            )
            .await
            .expect_err("inspect cannot race publication");
        let hash_busy = service
            .hash(
                hash_request(selected.selection_id.clone()),
                PUBLISHER,
                account(PUBLISHER),
            )
            .await
            .expect_err("hash cannot race publication");
        assert!(matches!(concurrent, ArchiveSelectionError::Busy { .. }));
        assert!(matches!(inspect_busy, ArchiveSelectionError::Busy { .. }));
        assert!(matches!(hash_busy, ArchiveSelectionError::Busy { .. }));
        service
            .registry
            .lock()
            .expect("registry")
            .selections
            .get_mut(selected.selection_id.as_str())
            .expect("selection")
            .last_access = Instant::now() - SELECTION_TTL;

        // Act
        let busy = service
            .discard(
                DiscardArchiveSelectionRequest {
                    request_id: request_id(),
                    selection_id: selected.selection_id.clone(),
                    expected_publisher_npub: PUBLISHER.into(),
                },
                PUBLISHER,
            )
            .await
            .expect_err("leased selection is busy");
        service.prune_expired().await;
        let retained = service
            .registry
            .lock()
            .expect("registry")
            .selections
            .contains_key(selected.selection_id.as_str());
        drop(lease);
        let discarded = service
            .discard(
                DiscardArchiveSelectionRequest {
                    request_id: request_id(),
                    selection_id: selected.selection_id.clone(),
                    expected_publisher_npub: PUBLISHER.into(),
                },
                PUBLISHER,
            )
            .await
            .expect("drop releases lease");

        // Assert
        assert!(matches!(busy, ArchiveSelectionError::Busy { .. }));
        assert!(retained);
        assert!(discarded.discarded);
        assert!(!path.exists());

        // Arrange a second selection for explicit finish and negative lease checks.
        let source = write_zip(temp.path(), "finish.zip", &[("game.exe", b"y")]);
        let selected = inspected(&service, source).await;
        let digest = service
            .hash(
                hash_request(selected.selection_id.clone()),
                PUBLISHER,
                account(PUBLISHER),
            )
            .await
            .expect("hash")
            .sha256;
        let wrong =
            Sha256Digest::parse("0000000000000000000000000000000000000000000000000000000000000000")
                .expect("digest");

        // Act
        let mismatch = service
            .publication_lease(
                &BuildSelectionRef {
                    selection_id: selected.selection_id.clone(),
                    sha256: wrong,
                },
                PUBLISHER,
                PUBLISHER,
                account(PUBLISHER),
            )
            .await
            .expect_err("digest mismatch");
        let account_error = service
            .publication_lease(
                &BuildSelectionRef {
                    selection_id: selected.selection_id.clone(),
                    sha256: digest.clone(),
                },
                PUBLISHER,
                "other",
                account("other"),
            )
            .await
            .expect_err("account mismatch");
        let path = staged_path(&service, &selected.selection_id).await;
        let lease = service
            .publication_lease(
                &BuildSelectionRef {
                    selection_id: selected.selection_id,
                    sha256: digest,
                },
                PUBLISHER,
                PUBLISHER,
                account(PUBLISHER),
            )
            .await
            .expect("lease");
        lease.finish().await;

        // Assert
        assert!(matches!(
            mismatch,
            ArchiveSelectionError::DigestMismatch { .. }
        ));
        assert_eq!(account_error, account_mismatch());
        assert!(!path.exists());
    }

    #[tokio::test]
    async fn a3_5_2_cancelled_publication_acquisition_releases_exact_token() {
        let temp = TempDir::new().expect("temp dir");
        let source = write_minimal_valid_archive(temp.path(), "cancelled-lease.zip");
        let service = service(temp.path());
        let selected = inspected(&service, source).await;
        let digest = service
            .hash(
                hash_request(selected.selection_id.clone()),
                PUBLISHER,
                account(PUBLISHER),
            )
            .await
            .expect("hash")
            .sha256;
        let reference = BuildSelectionRef {
            selection_id: selected.selection_id.clone(),
            sha256: digest,
        };
        let pending_check: AccountCheck = Arc::new(|| Box::pin(std::future::pending()));
        let task_service = service.clone();
        let task = tokio::spawn(async move {
            task_service
                .publication_lease(&reference, PUBLISHER, PUBLISHER, pending_check)
                .await
        });

        for _ in 0..100 {
            let reserved = service
                .registry
                .lock()
                .expect("registry")
                .selections
                .get(selected.selection_id.as_str())
                .is_some_and(|entry| entry.publication_token.is_some());
            if reserved {
                break;
            }
            tokio::task::yield_now().await;
        }
        assert!(
            service.registry.lock().expect("registry").selections[selected.selection_id.as_str()]
                .publication_token
                .is_some(),
            "acquisition reserved a token"
        );
        task.abort();
        let _ = task.await;

        let entry_token = service.registry.lock().expect("registry").selections
            [selected.selection_id.as_str()]
        .publication_token;
        assert!(entry_token.is_none());
    }

    #[tokio::test]
    async fn a3_5_2_publication_lease_rejects_blocking_hazards_before_leasing() {
        // Arrange
        let temp = TempDir::new().expect("temp dir");
        let source = write_zip(temp.path(), "hazard.zip", &[("../escape", b"x")]);
        let service = service(temp.path());
        let selected = inspected(&service, source).await;
        let digest = service
            .hash(
                hash_request(selected.selection_id.clone()),
                PUBLISHER,
                account(PUBLISHER),
            )
            .await
            .expect("hash")
            .sha256;

        // Act
        let error = service
            .publication_lease(
                &BuildSelectionRef {
                    selection_id: selected.selection_id.clone(),
                    sha256: digest,
                },
                PUBLISHER,
                PUBLISHER,
                account(PUBLISHER),
            )
            .await
            .expect_err("hazards take precedence over staged mutation");
        let publication_token = service.registry.lock().expect("registry").selections
            [selected.selection_id.as_str()]
        .publication_token;

        // Assert
        assert!(matches!(
            error,
            ArchiveSelectionError::BlockingHazards { .. }
        ));
        assert!(publication_token.is_none());
    }

    #[tokio::test]
    async fn a3_5_2_janitor_helper_expires_only_idle_unleased_selection() {
        // Arrange
        let temp = TempDir::new().expect("temp dir");
        let service = service(temp.path());
        let old = service
            .stage_source(
                write_zip(temp.path(), "old.zip", &[("game", b"x")]),
                PUBLISHER,
                account(PUBLISHER),
            )
            .await
            .expect("stage old");
        let fresh = service
            .stage_source(
                write_zip(temp.path(), "fresh.zip", &[("game", b"y")]),
                PUBLISHER,
                account(PUBLISHER),
            )
            .await
            .expect("stage fresh");
        let old_path = staged_path(&service, &old.selection_id).await;
        service
            .registry
            .lock()
            .expect("registry")
            .selections
            .get_mut(old.selection_id.as_str())
            .expect("old")
            .last_access = Instant::now() - SELECTION_TTL;

        // Act
        service.prune_expired().await;
        let expired = service
            .hash(
                hash_request(old.selection_id.clone()),
                PUBLISHER,
                account(PUBLISHER),
            )
            .await
            .expect_err("expired tombstone");
        let registry = service.registry.lock().expect("registry");

        // Assert
        assert!(!old_path.exists());
        assert!(matches!(expired, ArchiveSelectionError::Expired { .. }));
        assert!(registry
            .selections
            .contains_key(fresh.selection_id.as_str()));
    }

    #[test]
    fn a3_5_2_paging_search_platform_directories_and_empty_candidates_are_exact() {
        // Arrange
        let files = vec![
            SelectableArchiveFile {
                path: RelativePath::parse("z/Game.EXE").expect("path"),
                declared_size: 1,
            },
            SelectableArchiveFile {
                path: RelativePath::parse("A/tool.bin").expect("path"),
                declared_size: 2,
            },
        ];
        let page = ArchivePageRequest::new(0, 1).expect("page");
        let inspection = ArchiveInspection {
            index: ArchiveIndex {
                format: PackageFormat::Zip,
                entries: vec![ArchiveEntry {
                    raw_path: "bin/tools/game".into(),
                    kind: EntryKind::File,
                    declared_size: 1,
                    unix_mode: None,
                }],
            },
            hazards: ArchiveHazards::default(),
            warnings: ArchiveWarnings::default(),
        };

        // Act
        let base_first = page_filter(&files, "", page, |v| v.path.as_str()).expect("base page");
        let reset = page_filter(
            &files,
            "",
            ArchivePageRequest::new(0, 2).expect("page"),
            |v| v.path.as_str(),
        )
        .expect("reset");
        let searched = page_filter(&files, "GAME.exe", ArchivePageRequest::default(), |v| {
            v.path.as_str()
        })
        .expect("search");
        let dirs = inspection.index.working_directories();
        let empty: ArchivePage<ArchiveCandidate> =
            page_all(&[], ArchivePageRequest::default()).expect("empty page");
        let noncanonical = canonical_platform(&PlatformTarget {
            os: "Linux".into(),
            arch: "x86_64".into(),
        })
        .expect_err("noncanonical");
        let unknown = canonical_platform(&PlatformTarget {
            os: "plan9".into(),
            arch: "x86_64".into(),
        })
        .expect_err("unknown");

        // Assert
        assert_eq!(base_first.items[0].path.as_str(), "z/Game.EXE");
        assert_eq!(reset.items, files);
        assert_eq!(searched.items.len(), 1);
        assert_eq!(dirs, vec![".", "bin", "bin/tools"]);
        assert!(empty.items.is_empty());
        assert!(matches!(
            noncanonical,
            ArchiveSelectionError::InvalidPlatform {
                failure: ArchivePlatformFailure::NonCanonical { .. },
                ..
            }
        ));
        assert!(
            matches!(unknown, ArchiveSelectionError::InvalidPlatform { failure: ArchivePlatformFailure::UnsupportedOs { ref os }, .. } if os == "plan9")
        );
        let invalid = serde_json::from_value::<ArchivePageRequest>(
            serde_json::json!({"offset": 0, "limit": 201}),
        )
        .expect("wire limit deserializes");
        assert!(matches!(
            page_all::<ArchiveCandidate>(&[], invalid),
            Err(ArchiveSelectionError::Pagination {
                failure: ArchivePaginationFailure::LimitExceeded {
                    requested: 201,
                    maximum: ARCHIVE_PAGE_SIZE_MAX,
                }
            })
        ));
        assert!(ArchivePageRequest::new(0, 201).is_err());
    }

    #[tokio::test]
    async fn a3_5_2_invalid_wire_pages_precede_all_query_context_checks() {
        let temp = TempDir::new().expect("temp dir");
        let service = service(temp.path());
        let selection_id = ArchiveSelectionId::parse(&Uuid::new_v4().to_string()).expect("id");
        let page = serde_json::from_value::<ArchivePageRequest>(
            serde_json::json!({"offset": 0, "limit": 0}),
        )
        .expect("wire zero deserializes");
        let search = ArchiveSearch::parse("").expect("search");

        let errors = vec![
            service
                .query_files(
                    QueryArchiveFilesRequest {
                        request_id: request_id(),
                        selection_id: selection_id.clone(),
                        expected_publisher_npub: PUBLISHER.into(),
                        search: search.clone(),
                        page,
                    },
                    "other",
                )
                .await
                .expect_err("files pagination"),
            service
                .query_working_directories(
                    QueryArchiveWorkingDirectoriesRequest {
                        request_id: request_id(),
                        selection_id: selection_id.clone(),
                        expected_publisher_npub: PUBLISHER.into(),
                        search: search.clone(),
                        page,
                    },
                    "other",
                )
                .await
                .expect_err("directories pagination"),
            service
                .query_candidates(
                    QueryArchiveCandidatesRequest {
                        request_id: request_id(),
                        selection_id: selection_id.clone(),
                        expected_publisher_npub: PUBLISHER.into(),
                        search,
                        platform: PlatformTarget {
                            os: "invalid".into(),
                            arch: "invalid".into(),
                        },
                        page,
                    },
                    "other",
                )
                .await
                .expect_err("candidates pagination"),
            service
                .query_hazards(
                    QueryArchiveHazardsRequest {
                        request_id: request_id(),
                        selection_id: selection_id.clone(),
                        expected_publisher_npub: PUBLISHER.into(),
                        category: ArchiveHazardCategory::Symlink,
                        page,
                    },
                    "other",
                )
                .await
                .expect_err("hazards pagination"),
            service
                .query_warnings(
                    QueryArchiveWarningsRequest {
                        request_id: request_id(),
                        selection_id: selection_id.clone(),
                        expected_publisher_npub: PUBLISHER.into(),
                        category: ArchiveWarningCategory::ExtensionMismatch,
                        page,
                    },
                    "other",
                )
                .await
                .expect_err("warnings pagination"),
        ];

        for error in errors {
            assert_eq!(
                error,
                ArchiveSelectionError::Pagination {
                    failure: ArchivePaginationFailure::ZeroLimit,
                }
            );
        }

        let excessive = serde_json::from_value::<ArchivePageRequest>(
            serde_json::json!({"offset": 0, "limit": 201}),
        )
        .expect("wire excessive limit deserializes");
        let error = service
            .query_files(
                QueryArchiveFilesRequest {
                    request_id: request_id(),
                    selection_id,
                    expected_publisher_npub: PUBLISHER.into(),
                    search: ArchiveSearch::parse("").expect("search"),
                    page: excessive,
                },
                "other",
            )
            .await
            .expect_err("excessive pagination precedes account and handle checks");
        assert_eq!(
            error,
            ArchiveSelectionError::Pagination {
                failure: ArchivePaginationFailure::LimitExceeded {
                    requested: 201,
                    maximum: ARCHIVE_PAGE_SIZE_MAX,
                },
            }
        );
    }

    #[tokio::test]
    async fn a3_5_2_detected_format_is_authoritative_and_responses_are_path_safe() {
        // Arrange
        let temp = TempDir::new().expect("temp dir");
        let source = write_zip(temp.path(), "actually-gzip.tgz", &[("bin/game", b"x")]);
        let native = temp.path().to_string_lossy().into_owned();
        let service = service(temp.path());

        // Act
        let selected = service
            .stage_source(source, PUBLISHER, account(PUBLISHER))
            .await
            .expect("stage");
        let response = service
            .inspect(
                InspectArchiveRequest {
                    expected_format: Some(PackageFormat::TarGz),
                    ..inspect_request(selected.selection_id)
                },
                PUBLISHER,
                account(PUBLISHER),
            )
            .await
            .expect("content detection wins");
        let json = serde_json::to_string(&response).expect("serialize response");

        // Assert
        assert_eq!(response.summary.detected_format, PackageFormat::Zip);
        assert_eq!(
            response.summary.extension_format,
            Some(PackageFormat::TarGz)
        );
        assert!(response.summary.extension_mismatch_warning.is_some());
        assert!(!json.contains(&native));
        assert!(!json.contains("arcadestr-archive-v1-"));
    }

    #[test]
    fn a3_5_2_orphan_sweep_skips_active_sessions_and_reclaims_stale_owned_files() {
        let temp = TempDir::new().expect("temp dir");
        let (active_dir, active_lock) = initialize_session(temp.path()).expect("active session");
        let active_owned =
            active_dir.join(format!("{STAGED_PREFIX}{}{STAGED_SUFFIX}", Uuid::new_v4()));
        std::fs::write(&active_owned, b"active").expect("active fixture");

        let stale_dir = temp
            .path()
            .join(format!("{SESSION_PREFIX}{}", Uuid::new_v4()));
        std::fs::create_dir(&stale_dir).expect("stale session");
        let stale_lock = stale_dir.join(SESSION_LOCK_NAME);
        std::fs::write(&stale_lock, b"").expect("stale lock");
        let stale_owned =
            stale_dir.join(format!("{STAGED_PREFIX}{}{STAGED_SUFFIX}", Uuid::new_v4()));
        let malformed = stale_dir.join("keep.stage");
        std::fs::write(&stale_owned, b"stale").expect("stale fixture");
        std::fs::write(&malformed, b"keep").expect("malformed fixture");
        #[cfg(unix)]
        let symlink = {
            let path = stale_dir.join(format!("{STAGED_PREFIX}{}{STAGED_SUFFIX}", Uuid::new_v4()));
            std::os::unix::fs::symlink(&malformed, &path).expect("symlink fixture");
            path
        };
        let old_root_owned = temp
            .path()
            .join(format!("{STAGED_PREFIX}{}{STAGED_SUFFIX}", Uuid::new_v4()));
        let malformed_dir = temp.path().join("session-v2-not-a-uuid");
        let root_file = temp.path().join("keep.stage");
        std::fs::write(&old_root_owned, b"old").expect("old fixture");
        std::fs::write(&root_file, b"keep").expect("root malformed fixture");
        std::fs::create_dir(&malformed_dir).expect("malformed directory");

        sweep_orphans(temp.path()).expect("sweep");

        assert!(active_owned.exists());
        assert!(!stale_owned.exists());
        assert!(malformed.exists());
        assert!(!old_root_owned.exists());
        assert!(root_file.exists());
        assert!(malformed_dir.exists());
        #[cfg(unix)]
        assert!(symlink.symlink_metadata().is_ok());
        drop(active_lock);
    }

    #[test]
    fn a3_5_2_enospc_mapping_is_exact_and_other_io_is_generic() {
        // Arrange
        let temp = TempDir::new().expect("temp dir");
        let size = 17;

        // Act
        let full = copy_error(std::io::Error::from_raw_os_error(28), size, temp.path());
        let other = copy_error(std::io::Error::from_raw_os_error(5), size, temp.path());

        // Assert
        assert!(
            matches!(full, ArchiveSelectionError::InsufficientStorage { required, .. } if required == size + STORAGE_MARGIN)
        );
        assert_eq!(other, storage(ArchiveStorageOperation::CopySource));
    }
}
