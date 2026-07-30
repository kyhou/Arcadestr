//! Native, UI-independent Blossom upload service.
//!
//! Inspection deliberately validates only the media container envelope needed to reject
//! obvious spoofing and malformed files. It is not a codec decoder: video dimensions are
//! optional, MP4 nested boxes/codecs are not decoded, and WebM clusters are not validated.
//! Callers must still treat uploaded media as untrusted when rendering it.
//! `infer` supplies advisory magic detection, while custom container checks remain authoritative.
//! `imagesize` extracts image dimensions from the bounded 1 MiB inspection prefix; an unusually
//! late JPEG dimension marker is rejected rather than causing an unbounded pre-upload read.

use arcadestr_core::blossom::{
    build_upload_authorization, encode_blossom_authorization_header, parse_blob_descriptor,
    validate_blob_descriptor, validate_blossom_media, validate_blossom_server_origin,
    BlossomBlobDescriptor, BlossomBlobExpectation, BlossomServerOrigin, BlossomServerOriginPolicy,
    UploadAuthorizationInput, BLOSSOM_UPLOAD_AUTHORIZATION_KIND,
};
use arcadestr_core::signers::NostrSigner;
use async_trait::async_trait;
use futures_util::{stream, StreamExt};
use nostr::{Event, PublicKey};
use reqwest::header::{AUTHORIZATION, CONTENT_LENGTH, CONTENT_TYPE};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fs::Metadata;
use std::io::{Read, Seek, SeekFrom};
use std::net::{IpAddr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use thiserror::Error;
use tokio::io::AsyncReadExt;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

const INSPECTION_PREFIX_BYTES: usize = 1024 * 1024;
const IO_CHUNK_BYTES: usize = 64 * 1024;
const MAX_RESPONSE_BYTES: usize = 64 * 1024;
const MAX_PROGRESS_MESSAGE_CHARS: usize = 160;
const AUTH_LIFETIME_SECS: u64 = 600;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectedMediaFile {
    pub selection_id: Uuid,
    pub filename: String,
    pub size: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InspectedMediaFile {
    pub selection_id: Uuid,
    pub filename: String,
    pub size: u64,
    pub mime_type: String,
    pub width: Option<u32>,
    pub height: Option<u32>,
}

#[derive(Debug, Clone)]
pub struct BlossomUploadRequest {
    pub selection_id: Uuid,
    pub server: String,
    pub upload_id: Option<Uuid>,
    pub preflight: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlossomUploadResult {
    pub upload_id: Uuid,
    pub sha256: String,
    pub size: u64,
    pub mime_type: String,
    pub descriptor: BlossomBlobDescriptor,
    /// `true` when the server reported that the blob already existed (HTTP 200).
    pub was_existing: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlossomUploadPhase {
    Inspect,
    Hash,
    Sign,
    Preflight,
    Upload,
    Verify,
    Complete,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlossomUploadProgress {
    pub upload_id: Uuid,
    pub phase: BlossomUploadPhase,
    pub bytes: u64,
    pub total: u64,
    pub message: Option<String>,
}

pub type BlossomProgressCallback = Arc<dyn Fn(BlossomUploadProgress) + Send + Sync>;

/// Cancellation can leave a partial remote blob when the server received body bytes before the
/// local request was dropped. Blossom does not provide a transactional upload abort operation.
#[derive(Debug, Error)]
pub enum BlossomUploadError {
    #[error("media selection is missing or expired")]
    SelectionUnavailable,
    #[error("media selection belongs to another account")]
    AccountMismatch,
    #[error("no active account or signer is available")]
    AccountUnavailable,
    #[error("an upload with this id is already active")]
    UploadAlreadyActive,
    #[error("upload was cancelled; the remote server may retain a partial body")]
    CancelledRemotePartialPossible,
    #[error("unsupported or spoofed media: {0}")]
    InvalidMedia(String),
    #[error("selected file changed after registration or inspection")]
    FileChanged,
    #[error("I/O failure: {0}")]
    Io(String),
    #[error("invalid Blossom destination: {0}")]
    Destination(String),
    #[error("destination resolves to a forbidden address: {0}")]
    ForbiddenAddress(IpAddr),
    #[error("DNS resolution failed: {0}")]
    Dns(String),
    #[error("signer rejected the authorization: {0}")]
    SignerRejected(String),
    #[error("signing timed out")]
    SignerTimeout,
    #[error("signer returned an invalid authorization event: {0}")]
    InvalidSignedAuthorization(String),
    #[error("HTTP request failed: {0}")]
    Http(String),
    #[error("Blossom server redirected the request")]
    RedirectRejected,
    #[error("Blossom server requires payment")]
    PaymentRequired,
    #[error("Blossom server returned status {status}: {body}")]
    Server { status: u16, body: String },
    #[error("Blossom response exceeded 65536 bytes")]
    ResponseTooLarge,
    #[error("invalid Blossom descriptor: {0}")]
    InvalidDescriptor(String),
}

#[async_trait]
pub trait BlossomAccountProvider: Send + Sync {
    async fn current_publisher(&self) -> Result<PublicKey, BlossomUploadError>;
    async fn current_signer(&self) -> Result<Arc<dyn NostrSigner>, BlossomUploadError>;
}

#[derive(Debug, Clone)]
pub struct BlossomUploadConfig {
    pub selection_ttl: Duration,
    pub max_selections: usize,
    pub allow_loopback: bool,
    pub connect_timeout: Duration,
    pub request_timeout: Duration,
    pub signer_timeout: Duration,
}

impl Default for BlossomUploadConfig {
    fn default() -> Self {
        Self {
            selection_ttl: Duration::from_secs(15 * 60),
            max_selections: 32,
            allow_loopback: false,
            connect_timeout: Duration::from_secs(10),
            request_timeout: Duration::from_secs(10 * 60),
            signer_timeout: Duration::from_secs(90),
        }
    }
}

#[derive(Clone)]
struct SelectionEntry {
    path: PathBuf,
    filename: String,
    publisher: PublicKey,
    len: u64,
    modified: Option<SystemTime>,
    created_at: Instant,
    created_wall: SystemTime,
}

#[derive(Default)]
struct SelectionRegistry {
    entries: HashMap<Uuid, SelectionEntry>,
}

#[derive(Clone)]
pub struct BlossomUploadService {
    provider: Arc<dyn BlossomAccountProvider>,
    config: BlossomUploadConfig,
    registry: Arc<Mutex<SelectionRegistry>>,
    active: Arc<Mutex<HashMap<Uuid, CancellationToken>>>,
}

struct ActiveUploadGuard {
    upload_id: Uuid,
    token: CancellationToken,
    active: Arc<Mutex<HashMap<Uuid, CancellationToken>>>,
}

impl Drop for ActiveUploadGuard {
    fn drop(&mut self) {
        self.token.cancel();
        if let Ok(mut active) = self.active.lock() {
            active.remove(&self.upload_id);
        }
    }
}

impl BlossomUploadService {
    pub fn new(provider: Arc<dyn BlossomAccountProvider>, config: BlossomUploadConfig) -> Self {
        Self {
            provider,
            config,
            registry: Arc::new(Mutex::new(SelectionRegistry::default())),
            active: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Resolve the active publisher without exposing the account provider to command layers.
    pub async fn current_publisher(&self) -> Result<PublicKey, BlossomUploadError> {
        self.provider.current_publisher().await
    }

    pub fn register_file(
        &self,
        path: impl AsRef<Path>,
        expected_publisher: PublicKey,
    ) -> Result<SelectedMediaFile, BlossomUploadError> {
        let path = std::fs::canonicalize(path).map_err(io_error)?;
        let metadata = std::fs::metadata(&path).map_err(io_error)?;
        if !metadata.is_file() {
            return Err(BlossomUploadError::InvalidMedia(
                "not a regular file".into(),
            ));
        }
        let filename = path
            .file_name()
            .and_then(|name| name.to_str())
            .filter(|name| !name.is_empty())
            .ok_or_else(|| BlossomUploadError::InvalidMedia("invalid filename".into()))?
            .to_owned();
        let now = Instant::now();
        let mut registry = self.registry.lock().map_err(lock_error)?;
        prune_registry(&mut registry, now, self.config.selection_ttl);
        if self.config.max_selections == 0 {
            return Err(BlossomUploadError::SelectionUnavailable);
        }
        while registry.entries.len() >= self.config.max_selections {
            let oldest = registry
                .entries
                .iter()
                .min_by_key(|(_, entry)| entry.created_at)
                .map(|(id, _)| *id);
            if let Some(id) = oldest {
                registry.entries.remove(&id);
            } else {
                break;
            }
        }
        let selection_id = Uuid::new_v4();
        registry.entries.insert(
            selection_id,
            SelectionEntry {
                path,
                filename: filename.clone(),
                publisher: expected_publisher,
                len: metadata.len(),
                modified: metadata.modified().ok(),
                created_at: now,
                created_wall: SystemTime::now(),
            },
        );
        Ok(SelectedMediaFile {
            selection_id,
            filename,
            size: metadata.len(),
        })
    }

    pub fn prune(&self) -> Result<usize, BlossomUploadError> {
        let mut registry = self.registry.lock().map_err(lock_error)?;
        let before = registry.entries.len();
        prune_registry(&mut registry, Instant::now(), self.config.selection_ttl);
        Ok(before - registry.entries.len())
    }

    pub fn cleanup_selection(&self, selection_id: Uuid) -> Result<bool, BlossomUploadError> {
        Ok(self
            .registry
            .lock()
            .map_err(lock_error)?
            .entries
            .remove(&selection_id)
            .is_some())
    }

    pub fn cleanup_selection_for(
        &self,
        selection_id: Uuid,
        expected_publisher: PublicKey,
    ) -> Result<bool, BlossomUploadError> {
        let mut registry = self.registry.lock().map_err(lock_error)?;
        let Some(entry) = registry.entries.get(&selection_id) else {
            return Ok(false);
        };
        if entry.publisher != expected_publisher {
            return Err(BlossomUploadError::AccountMismatch);
        }
        Ok(registry.entries.remove(&selection_id).is_some())
    }

    pub fn cancel(&self, upload_id: Uuid) -> Result<bool, BlossomUploadError> {
        let token = self
            .active
            .lock()
            .map_err(lock_error)?
            .get(&upload_id)
            .cloned();
        if let Some(token) = token {
            token.cancel();
            Ok(true)
        } else {
            Ok(false)
        }
    }

    pub async fn inspect_selection(
        &self,
        selection_id: Uuid,
    ) -> Result<InspectedMediaFile, BlossomUploadError> {
        let publisher = self.provider.current_publisher().await?;
        let entry = self.selection(selection_id, publisher)?;
        inspect_entry(selection_id, &entry).await
    }

    /// Explicit retries call this same method. The hash and authorization are intentionally
    /// regenerated; there is no automatic retry once a PUT body may have started.
    pub async fn upload(
        &self,
        request: BlossomUploadRequest,
        progress: BlossomProgressCallback,
    ) -> Result<BlossomUploadResult, BlossomUploadError> {
        let upload_id = request.upload_id.unwrap_or_else(Uuid::new_v4);
        let token = CancellationToken::new();
        {
            let mut active = self.active.lock().map_err(lock_error)?;
            if active.contains_key(&upload_id) {
                return Err(BlossomUploadError::UploadAlreadyActive);
            }
            active.insert(upload_id, token.clone());
        }
        let _guard = ActiveUploadGuard {
            upload_id,
            token: token.clone(),
            active: Arc::clone(&self.active),
        };
        self.upload_inner(upload_id, request, progress, token).await
    }

    async fn upload_inner(
        &self,
        upload_id: Uuid,
        request: BlossomUploadRequest,
        progress: BlossomProgressCallback,
        token: CancellationToken,
    ) -> Result<BlossomUploadResult, BlossomUploadError> {
        let publisher = self.provider.current_publisher().await?;
        let entry = self.selection(request.selection_id, publisher)?;
        emit(
            &progress,
            upload_id,
            BlossomUploadPhase::Inspect,
            0,
            entry.len,
            None,
        );
        let inspected = cancelled(&token, inspect_entry(request.selection_id, &entry)).await??;
        emit(
            &progress,
            upload_id,
            BlossomUploadPhase::Hash,
            0,
            entry.len,
            None,
        );
        let sha256 = hash_file(&entry, upload_id, &progress, &token).await?;
        ensure_account(&*self.provider, publisher).await?;

        let policy = if self.config.allow_loopback {
            BlossomServerOriginPolicy::AllowHttpLoopback
        } else {
            BlossomServerOriginPolicy::HttpsOnly
        };
        let origin = validate_blossom_server_origin(&request.server, policy)
            .map_err(|error| BlossomUploadError::Destination(error.to_string()))?;
        let pinned = resolve_destination(
            &origin,
            self.config.allow_loopback,
            &token,
            self.config.connect_timeout,
        )
        .await?;
        let signer = self.provider.current_signer().await?;
        let signer_pubkey = cancelled(&token, signer.get_public_key())
            .await?
            .map_err(|error| BlossomUploadError::SignerRejected(error.to_string()))?;
        if signer_pubkey != publisher {
            return Err(BlossomUploadError::AccountMismatch);
        }
        ensure_account(&*self.provider, publisher).await?;
        emit(
            &progress,
            upload_id,
            BlossomUploadPhase::Sign,
            0,
            entry.len,
            None,
        );
        let now = unix_now()?.saturating_sub(1);
        let authorization_content = format!("Upload media to Blossom ({})", Uuid::new_v4());
        let unsigned = build_upload_authorization(&UploadAuthorizationInput {
            publisher,
            sha256: sha256.clone(),
            created_at: now,
            expiration: now + AUTH_LIFETIME_SECS,
            server: Some(origin.clone()),
            content: authorization_content.clone(),
        })
        .map_err(|error| BlossomUploadError::InvalidSignedAuthorization(error.to_string()))?;
        let event = cancelled(
            &token,
            tokio::time::timeout(self.config.signer_timeout, signer.sign_event(unsigned)),
        )
        .await?
        .map_err(|_| BlossomUploadError::SignerTimeout)?
        .map_err(|error| BlossomUploadError::SignerRejected(error.to_string()))?;
        verify_authorization(
            &event,
            publisher,
            &sha256,
            now,
            now + AUTH_LIFETIME_SECS,
            &origin,
            &authorization_content,
        )?;
        ensure_account(&*self.provider, publisher).await?;
        let authorization = encode_blossom_authorization_header(&event)
            .map_err(|error| BlossomUploadError::InvalidSignedAuthorization(error.to_string()))?;
        let client = pinned_client(&origin, &pinned, &self.config)?;
        let endpoint = format!("{}upload", origin.as_str());

        if request.preflight {
            // BUD-11 scopes both HEAD and PUT /upload to the same action and hash, so one
            // short-lived authorization can safely cover both requests in this attempt.
            emit(
                &progress,
                upload_id,
                BlossomUploadPhase::Preflight,
                0,
                entry.len,
                None,
            );
            let response = cancelled(
                &token,
                client
                    .head(&endpoint)
                    .header(AUTHORIZATION, &authorization)
                    .header("X-SHA-256", &sha256)
                    .header("X-Content-Type", &inspected.mime_type)
                    .header("X-Content-Length", entry.len)
                    .send(),
            )
            .await?;
            let response = match response {
                Ok(response) => response,
                Err(_) if token.is_cancelled() => {
                    return Err(BlossomUploadError::CancelledRemotePartialPossible)
                }
                Err(error) => return Err(http_error(error)),
            };
            match response.status().as_u16() {
                200 | 404 | 405 | 501 => {}
                402 => return Err(BlossomUploadError::PaymentRequired),
                status if (300..400).contains(&status) => {
                    return Err(BlossomUploadError::RedirectRejected)
                }
                status => {
                    let body = bounded_response(response, &token).await?;
                    return Err(BlossomUploadError::Server { status, body });
                }
            }
        }

        ensure_account(&*self.provider, publisher).await?;
        ensure_unchanged(&entry, &std::fs::metadata(&entry.path).map_err(io_error)?)?;
        emit(
            &progress,
            upload_id,
            BlossomUploadPhase::Upload,
            0,
            entry.len,
            None,
        );
        let file = tokio::fs::File::open(&entry.path).await.map_err(io_error)?;
        ensure_unchanged(&entry, &file.metadata().await.map_err(io_error)?)?;
        let streamed_bytes = Arc::new(AtomicU64::new(0));
        let streamed_hasher = Arc::new(Mutex::new(Sha256::new()));
        let state = UploadStreamState {
            file,
            sent: 0,
            total: entry.len,
            upload_id,
            progress: progress.clone(),
            token: token.clone(),
            streamed_bytes: streamed_bytes.clone(),
            streamed_hasher: Arc::clone(&streamed_hasher),
        };
        let body_stream = stream::try_unfold(state, |mut state| async move {
            if state.sent == state.total {
                return Ok(None);
            }
            if state.token.is_cancelled() {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::Interrupted,
                    "cancelled",
                ));
            }
            let remaining = (state.total - state.sent).min(IO_CHUNK_BYTES as u64) as usize;
            let mut buffer = vec![0_u8; remaining];
            let count = state.file.read(&mut buffer).await?;
            if count == 0 {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::UnexpectedEof,
                    "file truncated",
                ));
            }
            buffer.truncate(count);
            state
                .streamed_hasher
                .lock()
                .map_err(|_| std::io::Error::other("upload hash state unavailable"))?
                .update(&buffer);
            state.sent += count as u64;
            state.streamed_bytes.store(state.sent, Ordering::Release);
            emit(
                &state.progress,
                state.upload_id,
                BlossomUploadPhase::Upload,
                state.sent,
                state.total,
                None,
            );
            Ok(Some((buffer, state)))
        });
        let send = client
            .put(&endpoint)
            .header(AUTHORIZATION, authorization)
            .header(CONTENT_TYPE, &inspected.mime_type)
            .header(CONTENT_LENGTH, entry.len)
            .header("X-SHA-256", &sha256)
            .body(reqwest::Body::wrap_stream(body_stream))
            .send();
        let response = cancelled(&token, send).await?;
        let response = match response {
            Ok(response) => response,
            Err(_) if token.is_cancelled() => {
                return Err(BlossomUploadError::CancelledRemotePartialPossible)
            }
            Err(error) => return Err(http_error(error)),
        };
        if streamed_bytes.load(Ordering::Acquire) != entry.len {
            return Err(BlossomUploadError::FileChanged);
        }
        let uploaded_sha256 = streamed_hasher
            .lock()
            .map_err(lock_error)?
            .clone()
            .finalize();
        if format!("{uploaded_sha256:x}") != sha256 {
            return Err(BlossomUploadError::FileChanged);
        }
        ensure_account(&*self.provider, publisher).await?;
        let status = response.status().as_u16();
        if (300..400).contains(&status) {
            return Err(BlossomUploadError::RedirectRejected);
        }
        if status == 402 {
            return Err(BlossomUploadError::PaymentRequired);
        }
        if status != 200 && status != 201 {
            let body = bounded_response(response, &token).await?;
            return Err(BlossomUploadError::Server { status, body });
        }
        emit(
            &progress,
            upload_id,
            BlossomUploadPhase::Verify,
            entry.len,
            entry.len,
            None,
        );
        let body = bounded_response(response, &token).await?;
        let descriptor = parse_blob_descriptor(&body)
            .and_then(|descriptor| {
                validate_blob_descriptor(
                    descriptor,
                    BlossomBlobExpectation {
                        sha256: &sha256,
                        size: entry.len,
                        mime_type: &inspected.mime_type,
                    },
                )
            })
            .map_err(|error| BlossomUploadError::InvalidDescriptor(error.to_string()))?;
        ensure_account(&*self.provider, publisher).await?;
        ensure_unchanged(&entry, &std::fs::metadata(&entry.path).map_err(io_error)?)?;
        self.cleanup_selection(request.selection_id)?;
        emit(
            &progress,
            upload_id,
            BlossomUploadPhase::Complete,
            entry.len,
            entry.len,
            None,
        );
        Ok(BlossomUploadResult {
            upload_id,
            sha256,
            size: entry.len,
            mime_type: inspected.mime_type,
            descriptor,
            was_existing: status == 200,
        })
    }

    fn selection(
        &self,
        selection_id: Uuid,
        publisher: PublicKey,
    ) -> Result<SelectionEntry, BlossomUploadError> {
        let now = Instant::now();
        let mut registry = self.registry.lock().map_err(lock_error)?;
        prune_registry(&mut registry, now, self.config.selection_ttl);
        let entry = registry
            .entries
            .get(&selection_id)
            .cloned()
            .ok_or(BlossomUploadError::SelectionUnavailable)?;
        if entry.publisher != publisher {
            return Err(BlossomUploadError::AccountMismatch);
        }
        Ok(entry)
    }
}

struct UploadStreamState {
    file: tokio::fs::File,
    sent: u64,
    total: u64,
    upload_id: Uuid,
    progress: BlossomProgressCallback,
    token: CancellationToken,
    streamed_bytes: Arc<AtomicU64>,
    streamed_hasher: Arc<Mutex<Sha256>>,
}

fn prune_registry(registry: &mut SelectionRegistry, now: Instant, ttl: Duration) {
    registry
        .entries
        .retain(|_, entry| now.saturating_duration_since(entry.created_at) <= ttl);
}

async fn inspect_entry(
    selection_id: Uuid,
    entry: &SelectionEntry,
) -> Result<InspectedMediaFile, BlossomUploadError> {
    let entry = entry.clone();
    tokio::task::spawn_blocking(move || inspect_entry_sync(selection_id, &entry))
        .await
        .map_err(|error| BlossomUploadError::Io(error.to_string()))?
}

fn inspect_entry_sync(
    selection_id: Uuid,
    entry: &SelectionEntry,
) -> Result<InspectedMediaFile, BlossomUploadError> {
    let metadata = std::fs::metadata(&entry.path).map_err(io_error)?;
    ensure_unchanged(entry, &metadata)?;
    let mut file = std::fs::File::open(&entry.path).map_err(io_error)?;
    let mut prefix = vec![0_u8; (entry.len as usize).min(INSPECTION_PREFIX_BYTES)];
    file.read_exact(&mut prefix).map_err(io_error)?;
    let detected_mime = infer::get(&prefix)
        .map(|kind| kind.mime_type())
        .or_else(|| {
            prefix
                .starts_with(&[0x1a, 0x45, 0xdf, 0xa3])
                .then_some("video/webm")
        })
        .ok_or_else(|| BlossomUploadError::InvalidMedia("unrecognized file content".into()))?;
    validate_blossom_media(detected_mime, entry.len)
        .map_err(|error| BlossomUploadError::InvalidMedia(error.to_string()))?;
    let dimensions = match detected_mime {
        "image/jpeg" => {
            validate_jpeg(&mut file, entry.len, &prefix)?;
            image_dimensions(&prefix)?
        }
        "image/png" => {
            validate_png(&mut file, entry.len)?;
            image_dimensions(&prefix)?
        }
        "image/webp" => {
            validate_webp(&prefix, entry.len)?;
            image_dimensions(&prefix)?
        }
        "video/mp4" => {
            validate_mp4(&mut file, entry.len)?;
            None
        }
        "video/webm" => {
            validate_webm(&prefix)?;
            None
        }
        _ => {
            return Err(BlossomUploadError::InvalidMedia(
                "unsupported media content".into(),
            ))
        }
    };
    Ok(InspectedMediaFile {
        selection_id,
        filename: entry.filename.clone(),
        size: entry.len,
        mime_type: detected_mime.into(),
        width: dimensions.map(|value| value.0),
        height: dimensions.map(|value| value.1),
    })
}

fn image_dimensions(bytes: &[u8]) -> Result<Option<(u32, u32)>, BlossomUploadError> {
    let size = imagesize::blob_size(bytes).map_err(|error| {
        BlossomUploadError::InvalidMedia(format!("invalid image dimensions: {error}"))
    })?;
    let width = u32::try_from(size.width)
        .map_err(|_| BlossomUploadError::InvalidMedia("image width is too large".into()))?;
    let height = u32::try_from(size.height)
        .map_err(|_| BlossomUploadError::InvalidMedia("image height is too large".into()))?;
    if width == 0 || height == 0 {
        return Err(BlossomUploadError::InvalidMedia(
            "zero image dimension".into(),
        ));
    }
    Ok(Some((width, height)))
}

fn validate_jpeg(
    file: &mut std::fs::File,
    len: u64,
    prefix: &[u8],
) -> Result<(), BlossomUploadError> {
    if len < 4 || !prefix.starts_with(&[0xff, 0xd8, 0xff]) {
        return Err(BlossomUploadError::InvalidMedia(
            "invalid JPEG markers".into(),
        ));
    }
    file.seek(SeekFrom::End(-2)).map_err(io_error)?;
    let mut end = [0_u8; 2];
    file.read_exact(&mut end).map_err(io_error)?;
    if end != [0xff, 0xd9] {
        return Err(BlossomUploadError::InvalidMedia(
            "JPEG end marker is missing".into(),
        ));
    }
    Ok(())
}

fn validate_png(file: &mut std::fs::File, len: u64) -> Result<(), BlossomUploadError> {
    file.seek(SeekFrom::Start(0)).map_err(io_error)?;
    let mut signature = [0_u8; 8];
    file.read_exact(&mut signature).map_err(io_error)?;
    if signature != [137, 80, 78, 71, 13, 10, 26, 10] {
        return Err(BlossomUploadError::InvalidMedia(
            "invalid PNG signature".into(),
        ));
    }
    let mut offset = 8_u64;
    let mut first = true;
    let mut ended = false;
    while offset < len {
        if len - offset < 12 {
            return Err(BlossomUploadError::InvalidMedia(
                "truncated PNG chunk".into(),
            ));
        }
        let mut header = [0_u8; 8];
        file.read_exact(&mut header).map_err(io_error)?;
        let chunk_len = u32::from_be_bytes(header[..4].try_into().map_err(|_| malformed())?) as u64;
        let chunk_type = &header[4..8];
        let total = 12_u64.checked_add(chunk_len).ok_or_else(malformed)?;
        if total > len - offset {
            return Err(BlossomUploadError::InvalidMedia(
                "PNG chunk exceeds file".into(),
            ));
        }
        if first && (chunk_type != b"IHDR" || chunk_len != 13) {
            return Err(BlossomUploadError::InvalidMedia(
                "PNG must begin with a 13-byte IHDR".into(),
            ));
        }
        if chunk_type == b"IEND" {
            if chunk_len != 0 || offset + total != len {
                return Err(BlossomUploadError::InvalidMedia("invalid PNG IEND".into()));
            }
            ended = true;
        }
        file.seek(SeekFrom::Current((chunk_len + 4) as i64))
            .map_err(io_error)?;
        offset += total;
        first = false;
    }
    if !ended {
        return Err(BlossomUploadError::InvalidMedia(
            "PNG IEND is missing".into(),
        ));
    }
    Ok(())
}

fn validate_webp(prefix: &[u8], len: u64) -> Result<(), BlossomUploadError> {
    if prefix.len() < 16 || &prefix[..4] != b"RIFF" || &prefix[8..12] != b"WEBP" {
        return Err(BlossomUploadError::InvalidMedia(
            "invalid WebP envelope".into(),
        ));
    }
    let declared = u32::from_le_bytes(prefix[4..8].try_into().map_err(|_| malformed())?) as u64 + 8;
    if declared != len || !matches!(&prefix[12..16], b"VP8 " | b"VP8L" | b"VP8X") {
        return Err(BlossomUploadError::InvalidMedia(
            "invalid WebP length or primary chunk".into(),
        ));
    }
    Ok(())
}

fn validate_mp4(file: &mut std::fs::File, len: u64) -> Result<(), BlossomUploadError> {
    file.seek(SeekFrom::Start(0)).map_err(io_error)?;
    let mut offset = 0_u64;
    let mut valid_ftyp = false;
    while offset < len {
        if len - offset < 8 {
            return Err(BlossomUploadError::InvalidMedia("truncated MP4 box".into()));
        }
        let mut header = [0_u8; 16];
        file.read_exact(&mut header[..8]).map_err(io_error)?;
        let size32 = u32::from_be_bytes(header[..4].try_into().map_err(|_| malformed())?);
        let kind: [u8; 4] = header[4..8].try_into().map_err(|_| malformed())?;
        let (size, header_len) = if size32 == 1 {
            file.read_exact(&mut header[8..16]).map_err(io_error)?;
            (
                u64::from_be_bytes(header[8..16].try_into().map_err(|_| malformed())?),
                16_u64,
            )
        } else if size32 == 0 {
            (len - offset, 8_u64)
        } else {
            (size32 as u64, 8_u64)
        };
        if size < header_len || size > len - offset {
            return Err(BlossomUploadError::InvalidMedia(
                "MP4 box exceeds file".into(),
            ));
        }
        if kind == *b"ftyp" {
            if size < header_len + 8 || size > 1024 {
                return Err(BlossomUploadError::InvalidMedia("invalid MP4 ftyp".into()));
            }
            let mut payload = vec![0_u8; (size - header_len) as usize];
            file.read_exact(&mut payload).map_err(io_error)?;
            let brands = payload[..4]
                .chunks_exact(4)
                .chain(payload.get(8..).unwrap_or_default().chunks_exact(4));
            valid_ftyp = brands.into_iter().any(acceptable_mp4_brand);
        } else {
            file.seek(SeekFrom::Current((size - header_len) as i64))
                .map_err(io_error)?;
        }
        offset += size;
    }
    if !valid_ftyp {
        return Err(BlossomUploadError::InvalidMedia(
            "acceptable MP4 ftyp brand is missing".into(),
        ));
    }
    Ok(())
}

fn acceptable_mp4_brand(brand: &[u8]) -> bool {
    matches!(
        brand,
        b"isom" | b"iso2" | b"mp41" | b"mp42" | b"avc1" | b"M4V " | b"dash"
    )
}

fn validate_webm(prefix: &[u8]) -> Result<(), BlossomUploadError> {
    if !prefix.starts_with(&[0x1a, 0x45, 0xdf, 0xa3]) {
        return Err(BlossomUploadError::InvalidMedia(
            "invalid WebM EBML header".into(),
        ));
    }
    let mut index = 4;
    let (header_size, header_len) = parse_ebml_vint(prefix.get(index..).ok_or_else(malformed)?)?;
    index += header_len;
    let header_end = index
        .checked_add(header_size as usize)
        .filter(|end| *end <= prefix.len())
        .ok_or_else(malformed)?;
    let mut cursor = index;
    let mut valid_doctype = false;
    while cursor + 3 <= header_end {
        if prefix[cursor..].starts_with(&[0x42, 0x82]) {
            let (size, size_len) = parse_ebml_vint(&prefix[cursor + 2..])?;
            let start = cursor + 2 + size_len;
            let end = start.checked_add(size as usize).ok_or_else(malformed)?;
            if end <= header_end && prefix.get(start..end) == Some(b"webm") {
                valid_doctype = true;
                break;
            }
            return Err(BlossomUploadError::InvalidMedia(
                "EBML DocType is not webm".into(),
            ));
        }
        cursor += 1;
    }
    if !valid_doctype {
        return Err(BlossomUploadError::InvalidMedia(
            "WebM DocType is missing".into(),
        ));
    }
    if prefix.get(header_end..header_end + 4) != Some(&[0x18, 0x53, 0x80, 0x67]) {
        return Err(BlossomUploadError::InvalidMedia(
            "WebM Segment is missing".into(),
        ));
    }
    Ok(())
}

fn parse_ebml_vint(bytes: &[u8]) -> Result<(u64, usize), BlossomUploadError> {
    let first = *bytes.first().ok_or_else(malformed)?;
    let width = first.leading_zeros() as usize + 1;
    if width > 8 || bytes.len() < width {
        return Err(malformed());
    }
    let mut value = (first & (0xff >> width)) as u64;
    for byte in &bytes[1..width] {
        value = (value << 8) | *byte as u64;
    }
    Ok((value, width))
}

async fn hash_file(
    entry: &SelectionEntry,
    upload_id: Uuid,
    progress: &BlossomProgressCallback,
    token: &CancellationToken,
) -> Result<String, BlossomUploadError> {
    ensure_unchanged(
        entry,
        &tokio::fs::metadata(&entry.path).await.map_err(io_error)?,
    )?;
    let mut file = tokio::fs::File::open(&entry.path).await.map_err(io_error)?;
    ensure_unchanged(entry, &file.metadata().await.map_err(io_error)?)?;
    let mut hash = Sha256::new();
    let mut count = 0_u64;
    let mut buffer = vec![0_u8; IO_CHUNK_BYTES];
    loop {
        if token.is_cancelled() {
            return Err(BlossomUploadError::CancelledRemotePartialPossible);
        }
        let read = file.read(&mut buffer).await.map_err(io_error)?;
        if read == 0 {
            break;
        }
        count += read as u64;
        if count > entry.len {
            return Err(BlossomUploadError::FileChanged);
        }
        hash.update(&buffer[..read]);
        emit(
            progress,
            upload_id,
            BlossomUploadPhase::Hash,
            count,
            entry.len,
            None,
        );
    }
    if count != entry.len {
        return Err(BlossomUploadError::FileChanged);
    }
    ensure_unchanged(
        entry,
        &tokio::fs::metadata(&entry.path).await.map_err(io_error)?,
    )?;
    Ok(format!("{:x}", hash.finalize()))
}

fn ensure_unchanged(entry: &SelectionEntry, metadata: &Metadata) -> Result<(), BlossomUploadError> {
    if !metadata.is_file()
        || metadata.len() != entry.len
        || metadata.modified().ok() != entry.modified
        || SystemTime::now()
            .duration_since(entry.created_wall)
            .is_err()
    {
        return Err(BlossomUploadError::FileChanged);
    }
    Ok(())
}

async fn ensure_account(
    provider: &dyn BlossomAccountProvider,
    expected: PublicKey,
) -> Result<(), BlossomUploadError> {
    if provider.current_publisher().await? == expected {
        Ok(())
    } else {
        Err(BlossomUploadError::AccountMismatch)
    }
}

fn verify_authorization(
    event: &Event,
    publisher: PublicKey,
    sha256: &str,
    created_at: u64,
    expiration: u64,
    origin: &BlossomServerOrigin,
    content: &str,
) -> Result<(), BlossomUploadError> {
    if event.verify().is_err()
        || event.pubkey != publisher
        || event.kind.as_u16() != BLOSSOM_UPLOAD_AUTHORIZATION_KIND
        || event.created_at.as_u64() != created_at
        || event.content != content
    {
        return Err(BlossomUploadError::InvalidSignedAuthorization(
            "signature, publisher, kind, timestamp, or content mismatch".into(),
        ));
    }
    let expected_tag_count = if origin.authorization_domain().is_some() {
        4
    } else {
        3
    };
    if event.tags.len() != expected_tag_count {
        return Err(BlossomUploadError::InvalidSignedAuthorization(
            "authorization contains unexpected tags".into(),
        ));
    }
    let required = [
        ("t", "upload".to_owned()),
        ("expiration", expiration.to_string()),
        ("x", sha256.to_owned()),
    ];
    for (name, value) in required {
        if count_exact_tag(event, name, &value) != 1 {
            return Err(BlossomUploadError::InvalidSignedAuthorization(format!(
                "missing or duplicate {name} tag"
            )));
        }
    }
    if let Some(domain) = origin.authorization_domain() {
        if count_exact_tag(event, "server", domain) != 1 {
            return Err(BlossomUploadError::InvalidSignedAuthorization(
                "missing or duplicate server tag".into(),
            ));
        }
    } else if event
        .tags
        .iter()
        .any(|tag| tag.as_slice().first().map(String::as_str) == Some("server"))
    {
        return Err(BlossomUploadError::InvalidSignedAuthorization(
            "unexpected server tag".into(),
        ));
    }
    Ok(())
}

fn count_exact_tag(event: &Event, name: &str, value: &str) -> usize {
    event
        .tags
        .iter()
        .filter(|tag| matches!(tag.as_slice(), [tag_name, tag_value] if tag_name == name && tag_value == value))
        .count()
}

async fn resolve_destination(
    origin: &BlossomServerOrigin,
    allow_loopback: bool,
    token: &CancellationToken,
    timeout: Duration,
) -> Result<Vec<SocketAddr>, BlossomUploadError> {
    let url = reqwest::Url::parse(origin.as_str())
        .map_err(|error| BlossomUploadError::Destination(error.to_string()))?;
    let host = url
        .host_str()
        .ok_or_else(|| BlossomUploadError::Destination("host missing".into()))?;
    let port = url
        .port_or_known_default()
        .ok_or_else(|| BlossomUploadError::Destination("port missing".into()))?;
    let lookup = cancelled(
        token,
        tokio::time::timeout(timeout, tokio::net::lookup_host((host, port))),
    )
    .await?
    .map_err(|_| BlossomUploadError::Dns("resolution timed out".into()))?
    .map_err(|error| BlossomUploadError::Dns(error.to_string()))?;
    let addresses: Vec<_> = lookup.collect();
    if addresses.is_empty() {
        return Err(BlossomUploadError::Dns("no addresses".into()));
    }
    for address in &addresses {
        if !is_allowed_destination(address.ip(), allow_loopback) {
            return Err(BlossomUploadError::ForbiddenAddress(address.ip()));
        }
    }
    Ok(addresses)
}

fn is_allowed_destination(ip: IpAddr, allow_loopback: bool) -> bool {
    if ip.is_loopback() {
        return allow_loopback;
    }
    match ip {
        IpAddr::V4(ip) => {
            let [a, b, c, _] = ip.octets();
            !(ip.is_private()
                || ip.is_link_local()
                || ip.is_multicast()
                || ip.is_unspecified()
                || ip.is_broadcast()
                || ip.is_documentation()
                || a == 0
                || a >= 240
                || (a == 100 && (64..=127).contains(&b))
                || (a == 192 && b == 0 && c == 0)
                || (a == 192 && b == 88 && c == 99)
                || (a == 198 && (b == 18 || b == 19))
                || (a == 169 && b == 254))
        }
        IpAddr::V6(ip) => {
            let segments = ip.segments();
            if let Some(v4) = ip.to_ipv4() {
                return is_allowed_destination(IpAddr::V4(v4), allow_loopback);
            }
            (segments[0] & 0xe000) == 0x2000
                && !(ip.is_unspecified()
                    || ip.is_multicast()
                    || (segments[0] & 0xfe00) == 0xfc00
                    || (segments[0] & 0xffc0) == 0xfe80
                    || (segments[0] == 0x2001
                        && (segments[1] == 0
                            || segments[1] == 2
                            || (0x10..=0x2f).contains(&segments[1])
                            || segments[1] == 0x0db8))
                    || segments[0] == 0x2002)
        }
    }
}

fn pinned_client(
    origin: &BlossomServerOrigin,
    addresses: &[SocketAddr],
    config: &BlossomUploadConfig,
) -> Result<reqwest::Client, BlossomUploadError> {
    let url = reqwest::Url::parse(origin.as_str())
        .map_err(|error| BlossomUploadError::Destination(error.to_string()))?;
    let host = url
        .host_str()
        .ok_or_else(|| BlossomUploadError::Destination("host missing".into()))?;
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .no_proxy()
        .connect_timeout(config.connect_timeout)
        .timeout(config.request_timeout)
        .resolve_to_addrs(host, addresses)
        .build()
        .map_err(http_error)
}

async fn bounded_response(
    response: reqwest::Response,
    token: &CancellationToken,
) -> Result<String, BlossomUploadError> {
    let mut stream = response.bytes_stream();
    let mut body = Vec::new();
    while let Some(chunk) = cancelled(token, stream.next()).await? {
        let chunk = chunk.map_err(http_error)?;
        if body.len().saturating_add(chunk.len()) > MAX_RESPONSE_BYTES {
            return Err(BlossomUploadError::ResponseTooLarge);
        }
        body.extend_from_slice(&chunk);
    }
    Ok(String::from_utf8_lossy(&body).into_owned())
}

async fn cancelled<F, T>(token: &CancellationToken, future: F) -> Result<T, BlossomUploadError>
where
    F: std::future::Future<Output = T>,
{
    tokio::select! {
        _ = token.cancelled() => Err(BlossomUploadError::CancelledRemotePartialPossible),
        value = future => Ok(value),
    }
}

fn emit(
    callback: &BlossomProgressCallback,
    upload_id: Uuid,
    phase: BlossomUploadPhase,
    bytes: u64,
    total: u64,
    message: Option<String>,
) {
    let message = message.map(|value| value.chars().take(MAX_PROGRESS_MESSAGE_CHARS).collect());
    callback(BlossomUploadProgress {
        upload_id,
        phase,
        bytes,
        total,
        message,
    });
}

fn unix_now() -> Result<u64, BlossomUploadError> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .map_err(|error| BlossomUploadError::Io(error.to_string()))
}

fn malformed() -> BlossomUploadError {
    BlossomUploadError::InvalidMedia("malformed container".into())
}

fn io_error(error: impl std::fmt::Display) -> BlossomUploadError {
    BlossomUploadError::Io(error.to_string())
}

fn http_error(error: impl std::fmt::Display) -> BlossomUploadError {
    BlossomUploadError::Http(error.to_string())
}

fn lock_error<T>(error: std::sync::PoisonError<T>) -> BlossomUploadError {
    BlossomUploadError::Io(format!("internal state lock poisoned: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use arcadestr_core::signers::{LocalSigner, SignerError};
    use nostr::{Keys, UnsignedEvent};
    use std::net::{Ipv4Addr, Ipv6Addr};
    use tempfile::NamedTempFile;
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt};
    use tokio::sync::oneshot;

    struct TestProvider {
        publisher: Arc<Mutex<PublicKey>>,
        signer: Arc<dyn NostrSigner>,
    }

    #[async_trait]
    impl BlossomAccountProvider for TestProvider {
        async fn current_publisher(&self) -> Result<PublicKey, BlossomUploadError> {
            self.publisher
                .lock()
                .map(|value| *value)
                .map_err(lock_error)
        }

        async fn current_signer(&self) -> Result<Arc<dyn NostrSigner>, BlossomUploadError> {
            Ok(self.signer.clone())
        }
    }

    struct RejectSigner(PublicKey);

    #[async_trait]
    impl NostrSigner for RejectSigner {
        async fn get_public_key(&self) -> Result<PublicKey, SignerError> {
            Ok(self.0)
        }
        async fn sign_event(&self, _: UnsignedEvent) -> Result<Event, SignerError> {
            Err(SignerError::SigningFailed("rejected".into()))
        }
    }

    struct SlowSigner(PublicKey);

    #[async_trait]
    impl NostrSigner for SlowSigner {
        async fn get_public_key(&self) -> Result<PublicKey, SignerError> {
            Ok(self.0)
        }
        async fn sign_event(&self, _: UnsignedEvent) -> Result<Event, SignerError> {
            std::future::pending().await
        }
    }

    struct AccountSwitchSigner {
        signer: LocalSigner,
        publisher: Arc<Mutex<PublicKey>>,
        replacement: PublicKey,
    }

    #[async_trait]
    impl NostrSigner for AccountSwitchSigner {
        async fn get_public_key(&self) -> Result<PublicKey, SignerError> {
            self.signer.get_public_key().await
        }

        async fn sign_event(&self, unsigned: UnsignedEvent) -> Result<Event, SignerError> {
            let event = self.signer.sign_event(unsigned).await?;
            *self
                .publisher
                .lock()
                .map_err(|error| SignerError::SigningFailed(error.to_string()))? = self.replacement;
            Ok(event)
        }
    }

    fn provider() -> (Arc<TestProvider>, PublicKey) {
        let keys = Keys::generate();
        let publisher = keys.public_key();
        let secret = keys.secret_key().to_secret_hex();
        let signer = LocalSigner::from_hex(&secret).expect("test key must construct signer");
        (
            Arc::new(TestProvider {
                publisher: Arc::new(Mutex::new(publisher)),
                signer: Arc::new(signer),
            }),
            publisher,
        )
    }

    fn file_with_suffix(bytes: &[u8], suffix: &str) -> NamedTempFile {
        let mut file = tempfile::Builder::new()
            .suffix(suffix)
            .tempfile()
            .expect("temp file");
        std::io::Write::write_all(&mut file, bytes).expect("write fixture");
        file
    }

    fn png_fixture() -> Vec<u8> {
        // 1x1 RGBA PNG, including a structurally exact IEND.
        vec![
            137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 1, 0, 0, 0, 1,
            8, 6, 0, 0, 0, 31, 21, 196, 137, 0, 0, 0, 13, 73, 68, 65, 84, 8, 215, 99, 248, 207,
            192, 240, 31, 0, 5, 0, 1, 255, 137, 153, 61, 29, 0, 0, 0, 0, 73, 69, 78, 68, 174, 66,
            96, 130,
        ]
    }

    fn jpeg_fixture() -> Vec<u8> {
        vec![
            0xff, 0xd8, 0xff, 0xe0, 0x00, 0x10, b'J', b'F', b'I', b'F', 0, 1, 1, 0, 0, 1, 0, 1, 0,
            0, 0xff, 0xc0, 0x00, 0x0b, 8, 0, 1, 0, 1, 1, 1, 0x11, 0, 0xff, 0xd9,
        ]
    }

    fn webp_fixture() -> Vec<u8> {
        vec![
            b'R', b'I', b'F', b'F', 22, 0, 0, 0, b'W', b'E', b'B', b'P', b'V', b'P', b'8', b'X',
            10, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        ]
    }

    fn mp4_fixture() -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&20_u32.to_be_bytes());
        bytes.extend_from_slice(b"ftypisom");
        bytes.extend_from_slice(&0_u32.to_be_bytes());
        bytes.extend_from_slice(b"mp42");
        bytes
    }

    fn webm_fixture() -> Vec<u8> {
        vec![
            0x1a, 0x45, 0xdf, 0xa3, 0x87, 0x42, 0x82, 0x84, b'w', b'e', b'b', b'm', 0x18, 0x53,
            0x80, 0x67, 0xff,
        ]
    }

    #[derive(Clone, Copy)]
    enum FixtureBody {
        Valid,
        WrongHash,
        WrongSize,
        WrongMime,
        Oversized,
        Empty,
    }

    async fn blossom_fixture(
        responses: Vec<(u16, FixtureBody)>,
    ) -> (String, oneshot::Receiver<Vec<String>>) {
        let listener = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .await
            .expect("bind fixture");
        let address = listener.local_addr().expect("fixture address");
        let (sender, receiver) = oneshot::channel();
        tokio::spawn(async move {
            let mut requests = Vec::new();
            for (status, body_mode) in responses {
                let (mut socket, _) = listener.accept().await.expect("accept fixture request");
                let mut request = Vec::new();
                let header_end = loop {
                    let mut chunk = [0_u8; 4096];
                    let count = socket.read(&mut chunk).await.expect("read fixture request");
                    assert!(count > 0, "request ended before headers");
                    request.extend_from_slice(&chunk[..count]);
                    if let Some(position) =
                        request.windows(4).position(|window| window == b"\r\n\r\n")
                    {
                        break position + 4;
                    }
                };
                let headers = String::from_utf8_lossy(&request[..header_end]).into_owned();
                let content_length = headers
                    .lines()
                    .find_map(|line| {
                        line.to_ascii_lowercase()
                            .strip_prefix("content-length: ")
                            .map(str::to_owned)
                    })
                    .and_then(|value| value.trim().parse::<usize>().ok())
                    .unwrap_or(0);
                while request.len() - header_end < content_length {
                    let mut chunk = [0_u8; 4096];
                    let count = socket.read(&mut chunk).await.expect("read fixture body");
                    assert!(count > 0, "request body truncated");
                    request.extend_from_slice(&chunk[..count]);
                }
                requests.push(headers.clone());
                let hash = headers
                    .lines()
                    .find_map(|line| {
                        line.strip_prefix("x-sha-256: ")
                            .or_else(|| line.strip_prefix("X-SHA-256: "))
                    })
                    .unwrap_or_default();
                let mime = headers
                    .lines()
                    .find_map(|line| {
                        line.strip_prefix("content-type: ")
                            .or_else(|| line.strip_prefix("Content-Type: "))
                    })
                    .unwrap_or("image/png");
                let descriptor_hash = if matches!(body_mode, FixtureBody::WrongHash) {
                    "0".repeat(64)
                } else {
                    hash.to_owned()
                };
                let descriptor_size = if matches!(body_mode, FixtureBody::WrongSize) {
                    content_length + 1
                } else {
                    content_length
                };
                let descriptor_mime = if matches!(body_mode, FixtureBody::WrongMime) {
                    "image/webp"
                } else {
                    mime
                };
                let body = match body_mode {
                    FixtureBody::Valid
                    | FixtureBody::WrongHash
                    | FixtureBody::WrongSize
                    | FixtureBody::WrongMime => serde_json::json!({
                        "url": format!("https://cdn.example/{descriptor_hash}"),
                        "sha256": descriptor_hash,
                        "size": descriptor_size,
                        "type": descriptor_mime,
                        "uploaded": 1
                    })
                    .to_string(),
                    FixtureBody::Oversized => "x".repeat(MAX_RESPONSE_BYTES + 1),
                    FixtureBody::Empty => String::new(),
                };
                let reason = match status {
                    200 => "OK",
                    201 => "Created",
                    302 => "Found",
                    402 => "Payment Required",
                    405 => "Method Not Allowed",
                    500 => "Internal Server Error",
                    _ => "Test",
                };
                let response = format!(
                    "HTTP/1.1 {status} {reason}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                socket
                    .write_all(response.as_bytes())
                    .await
                    .expect("write fixture response");
            }
            let _ = sender.send(requests);
        });
        (format!("http://{address}/"), receiver)
    }

    fn development_service(provider: Arc<TestProvider>) -> BlossomUploadService {
        BlossomUploadService::new(
            provider,
            BlossomUploadConfig {
                allow_loopback: true,
                signer_timeout: Duration::from_secs(1),
                ..BlossomUploadConfig::default()
            },
        )
    }

    fn no_progress() -> BlossomProgressCallback {
        Arc::new(|_| {})
    }

    #[test]
    fn blossom_upload_destination_policy_rejects_non_global_and_allows_explicit_loopback() {
        assert!(!is_allowed_destination(
            IpAddr::V4(Ipv4Addr::new(169, 254, 169, 254)),
            false
        ));
        assert!(!is_allowed_destination(
            IpAddr::V4(Ipv4Addr::LOCALHOST),
            false
        ));
        assert!(is_allowed_destination(
            IpAddr::V4(Ipv4Addr::LOCALHOST),
            true
        ));
        assert!(!is_allowed_destination(
            IpAddr::V6(Ipv6Addr::LOCALHOST),
            false
        ));
        assert!(is_allowed_destination(
            IpAddr::V4(Ipv4Addr::new(8, 8, 8, 8)),
            false
        ));
        for rejected in [
            "64:ff9b::a9fe:a9fe",
            "2001:db8::1",
            "2002:a9fe:a9fe::1",
            "fec0::1",
        ] {
            assert!(!is_allowed_destination(
                IpAddr::V6(rejected.parse().expect("IPv6 fixture")),
                false
            ));
        }
        assert!(is_allowed_destination(
            IpAddr::V6("2001:4860:4860::8888".parse().expect("global IPv6")),
            false
        ));
    }

    #[tokio::test]
    async fn blossom_upload_detection_uses_content_and_rejects_gif_spoofing() {
        let (provider, publisher) = provider();
        let service = BlossomUploadService::new(provider, BlossomUploadConfig::default());
        let png = file_with_suffix(&png_fixture(), ".png");
        let selected = service
            .register_file(png.path(), publisher)
            .expect("register PNG");
        let inspected = service
            .inspect_selection(selected.selection_id)
            .await
            .expect("inspect PNG");
        assert_eq!(inspected.mime_type, "image/png");
        assert_eq!((inspected.width, inspected.height), (Some(1), Some(1)));

        let gif = file_with_suffix(b"GIF89a........", ".png");
        let selected = service
            .register_file(gif.path(), publisher)
            .expect("register GIF");
        assert!(matches!(
            service.inspect_selection(selected.selection_id).await,
            Err(BlossomUploadError::InvalidMedia(_))
        ));
        let spoof = file_with_suffix(&png_fixture(), ".jpg");
        let selected = service
            .register_file(spoof.path(), publisher)
            .expect("register spoof");
        let inspected = service
            .inspect_selection(selected.selection_id)
            .await
            .expect("content determines MIME");
        assert_eq!(inspected.mime_type, "image/png");
    }

    #[tokio::test]
    async fn blossom_upload_detects_every_supported_format_without_trusting_extensions() {
        let (provider, publisher) = provider();
        let service = BlossomUploadService::new(provider, BlossomUploadConfig::default());
        for (bytes, mime_type) in [
            (jpeg_fixture(), "image/jpeg"),
            (png_fixture(), "image/png"),
            (webp_fixture(), "image/webp"),
            (mp4_fixture(), "video/mp4"),
            (webm_fixture(), "video/webm"),
        ] {
            let file = file_with_suffix(&bytes, ".untrusted");
            let selected = service
                .register_file(file.path(), publisher)
                .expect("register");
            let inspected = service
                .inspect_selection(selected.selection_id)
                .await
                .unwrap_or_else(|error| panic!("failed to inspect {mime_type}: {error}"));
            assert_eq!(inspected.mime_type, mime_type);
        }
    }

    #[test]
    fn blossom_upload_detects_jpeg_webp_mp4_and_webm_envelopes() {
        let jpeg = [
            0xff, 0xd8, 0xff, 0xe0, 0x00, 0x10, b'J', b'F', b'I', b'F', 0, 1, 1, 0, 0, 1, 0, 1, 0,
            0, 0xff, 0xc0, 0x00, 0x0b, 8, 0, 1, 0, 1, 1, 1, 0x11, 0, 0xff, 0xd9,
        ];
        let mut jpeg_file = file_with_suffix(&jpeg, ".jpg");
        assert!(validate_jpeg(jpeg_file.as_file_mut(), jpeg.len() as u64, &jpeg).is_ok());
        assert_eq!(
            image_dimensions(&jpeg).expect("JPEG dimensions"),
            Some((1, 1))
        );

        let webp = [
            b'R', b'I', b'F', b'F', 22, 0, 0, 0, b'W', b'E', b'B', b'P', b'V', b'P', b'8', b'X',
            10, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        ];
        assert!(validate_webp(&webp, webp.len() as u64).is_ok());
        assert_eq!(
            image_dimensions(&webp).expect("WebP dimensions"),
            Some((1, 1))
        );

        let mut mp4 = Vec::new();
        mp4.extend_from_slice(&20_u32.to_be_bytes());
        mp4.extend_from_slice(b"ftypisom");
        mp4.extend_from_slice(&0_u32.to_be_bytes());
        mp4.extend_from_slice(b"mp42");
        let mp4_file = file_with_suffix(&mp4, ".mp4");
        assert!(validate_mp4(
            &mut std::fs::File::open(mp4_file.path()).expect("open MP4"),
            mp4.len() as u64
        )
        .is_ok());
        assert!(validate_webm(&webm_fixture()).is_ok());
    }

    #[tokio::test]
    async fn blossom_upload_registry_enforces_account_expiry_and_mutation() {
        let (provider, publisher) = provider();
        let config = BlossomUploadConfig {
            selection_ttl: Duration::ZERO,
            ..BlossomUploadConfig::default()
        };
        let service = BlossomUploadService::new(provider.clone(), config);
        let file = file_with_suffix(&png_fixture(), ".png");
        let selected = service
            .register_file(file.path(), publisher)
            .expect("register");
        tokio::time::sleep(Duration::from_millis(1)).await;
        assert!(matches!(
            service.inspect_selection(selected.selection_id).await,
            Err(BlossomUploadError::SelectionUnavailable)
        ));

        let service = BlossomUploadService::new(provider.clone(), BlossomUploadConfig::default());
        let selected = service
            .register_file(file.path(), publisher)
            .expect("register");
        *provider.publisher.lock().expect("publisher lock") = Keys::generate().public_key();
        assert!(matches!(
            service.inspect_selection(selected.selection_id).await,
            Err(BlossomUploadError::AccountMismatch)
        ));
        *provider.publisher.lock().expect("publisher lock") = publisher;
        std::io::Write::write_all(
            &mut std::fs::OpenOptions::new()
                .append(true)
                .open(file.path())
                .expect("open"),
            b"growth",
        )
        .expect("grow");
        assert!(matches!(
            service.inspect_selection(selected.selection_id).await,
            Err(BlossomUploadError::FileChanged)
        ));

        let truncated = file_with_suffix(&png_fixture(), ".png");
        let selected = service
            .register_file(truncated.path(), publisher)
            .expect("register truncation");
        std::fs::OpenOptions::new()
            .write(true)
            .open(truncated.path())
            .expect("open for truncation")
            .set_len(8)
            .expect("truncate");
        assert!(matches!(
            service.inspect_selection(selected.selection_id).await,
            Err(BlossomUploadError::FileChanged)
        ));
    }

    #[tokio::test]
    async fn blossom_upload_hash_is_lowercase_exact_and_cancellable() {
        let file = file_with_suffix(b"abc", ".mp4");
        let metadata = std::fs::metadata(file.path()).expect("metadata");
        let entry = SelectionEntry {
            path: std::fs::canonicalize(file.path()).expect("canonical"),
            filename: "a.mp4".into(),
            publisher: Keys::generate().public_key(),
            len: 3,
            modified: metadata.modified().ok(),
            created_at: Instant::now(),
            created_wall: SystemTime::now(),
        };
        let callback: BlossomProgressCallback = Arc::new(|_| {});
        let hash = hash_file(&entry, Uuid::new_v4(), &callback, &CancellationToken::new())
            .await
            .expect("hash");
        assert_eq!(
            hash,
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        let token = CancellationToken::new();
        token.cancel();
        assert!(matches!(
            hash_file(&entry, Uuid::new_v4(), &callback, &token).await,
            Err(BlossomUploadError::CancelledRemotePartialPossible)
        ));
    }

    #[tokio::test]
    async fn blossom_upload_local_signer_authorization_verifies_and_rejection_is_typed() {
        let (provider, publisher) = provider();
        let signer = provider.current_signer().await.expect("signer");
        let origin = validate_blossom_server_origin(
            "https://cdn.example/",
            BlossomServerOriginPolicy::HttpsOnly,
        )
        .expect("origin");
        let hash = "a".repeat(64);
        let unsigned = build_upload_authorization(&UploadAuthorizationInput {
            publisher,
            sha256: hash.clone(),
            created_at: 10,
            expiration: 610,
            server: Some(origin.clone()),
            content: "Upload media to Blossom".into(),
        })
        .expect("authorization");
        let event = signer.sign_event(unsigned).await.expect("sign");
        verify_authorization(
            &event,
            publisher,
            &hash,
            10,
            610,
            &origin,
            "Upload media to Blossom",
        )
        .expect("verify");
        let reject = RejectSigner(publisher);
        let unsigned = build_upload_authorization(&UploadAuthorizationInput {
            publisher,
            sha256: hash,
            created_at: 10,
            expiration: 610,
            server: Some(origin),
            content: "Upload media to Blossom".into(),
        })
        .expect("authorization");
        assert!(reject.sign_event(unsigned).await.is_err());
    }

    #[tokio::test]
    async fn blossom_upload_mock_signer_rejection_and_timeout_are_typed() {
        for timeout in [false, true] {
            let publisher = Keys::generate().public_key();
            let signer: Arc<dyn NostrSigner> = if timeout {
                Arc::new(SlowSigner(publisher))
            } else {
                Arc::new(RejectSigner(publisher))
            };
            let provider = Arc::new(TestProvider {
                publisher: Arc::new(Mutex::new(publisher)),
                signer,
            });
            let service = BlossomUploadService::new(
                provider,
                BlossomUploadConfig {
                    allow_loopback: true,
                    signer_timeout: Duration::from_millis(5),
                    ..BlossomUploadConfig::default()
                },
            );
            let file = file_with_suffix(&png_fixture(), ".png");
            let selected = service
                .register_file(file.path(), publisher)
                .expect("register");
            let error = service
                .upload(
                    BlossomUploadRequest {
                        selection_id: selected.selection_id,
                        server: "http://127.0.0.1:9/".into(),
                        upload_id: None,
                        preflight: false,
                    },
                    no_progress(),
                )
                .await
                .expect_err("signing fails before HTTP");
            if timeout {
                assert!(matches!(error, BlossomUploadError::SignerTimeout));
            } else {
                assert!(matches!(error, BlossomUploadError::SignerRejected(_)));
            }
        }
    }

    #[tokio::test]
    async fn blossom_upload_active_cancellation_stops_signing_and_cleans_active_state() {
        let publisher = Keys::generate().public_key();
        let provider = Arc::new(TestProvider {
            publisher: Arc::new(Mutex::new(publisher)),
            signer: Arc::new(SlowSigner(publisher)),
        });
        let service = BlossomUploadService::new(
            provider,
            BlossomUploadConfig {
                allow_loopback: true,
                signer_timeout: Duration::from_secs(30),
                ..BlossomUploadConfig::default()
            },
        );
        let file = file_with_suffix(&png_fixture(), ".png");
        let selected = service
            .register_file(file.path(), publisher)
            .expect("register");
        let upload_id = Uuid::new_v4();
        let task_service = service.clone();
        let task = tokio::spawn(async move {
            task_service
                .upload(
                    BlossomUploadRequest {
                        selection_id: selected.selection_id,
                        server: "http://127.0.0.1:9/".into(),
                        upload_id: Some(upload_id),
                        preflight: false,
                    },
                    no_progress(),
                )
                .await
        });
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert!(service.cancel(upload_id).expect("cancel active upload"));
        assert!(matches!(
            task.await.expect("join upload"),
            Err(BlossomUploadError::CancelledRemotePartialPossible)
        ));
        assert!(!service.cancel(upload_id).expect("active state cleaned"));
    }

    #[tokio::test]
    async fn blossom_upload_aborted_future_cleans_active_state() {
        let publisher = Keys::generate().public_key();
        let provider = Arc::new(TestProvider {
            publisher: Arc::new(Mutex::new(publisher)),
            signer: Arc::new(SlowSigner(publisher)),
        });
        let service = Arc::new(BlossomUploadService::new(
            provider,
            BlossomUploadConfig {
                allow_loopback: true,
                signer_timeout: Duration::from_secs(60),
                ..BlossomUploadConfig::default()
            },
        ));
        let file = file_with_suffix(&png_fixture(), ".png");
        let selected = service
            .register_file(file.path(), publisher)
            .expect("register");
        let upload_id = Uuid::new_v4();
        let task_service = Arc::clone(&service);
        let task = tokio::spawn(async move {
            task_service
                .upload(
                    BlossomUploadRequest {
                        selection_id: selected.selection_id,
                        server: "http://127.0.0.1:9/".into(),
                        upload_id: Some(upload_id),
                        preflight: false,
                    },
                    no_progress(),
                )
                .await
        });
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(service
            .active
            .lock()
            .expect("active uploads")
            .contains_key(&upload_id));
        task.abort();
        let _ = task.await;
        assert!(!service
            .active
            .lock()
            .expect("active uploads")
            .contains_key(&upload_id));
    }

    #[tokio::test]
    async fn blossom_upload_cancellation_during_body_stream_is_typed() {
        let (provider, publisher) = provider();
        let service = development_service(provider);
        let mut bytes = mp4_fixture();
        let payload_len = IO_CHUNK_BYTES * 4;
        bytes.extend_from_slice(&((payload_len + 8) as u32).to_be_bytes());
        bytes.extend_from_slice(b"mdat");
        bytes.resize(bytes.len() + payload_len, 0);
        let file = file_with_suffix(&bytes, ".mp4");
        let selected = service
            .register_file(file.path(), publisher)
            .expect("register");
        let listener = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .await
            .expect("bind slow fixture");
        let address = listener.local_addr().expect("fixture address");
        let server_task = tokio::spawn(async move {
            let (_socket, _) = listener.accept().await.expect("accept upload");
            tokio::time::sleep(Duration::from_secs(5)).await;
        });
        let upload_id = Uuid::new_v4();
        let cancel_service = service.clone();
        let progress: BlossomProgressCallback = Arc::new(move |progress| {
            if progress.phase == BlossomUploadPhase::Upload && progress.bytes > 0 {
                let _ = cancel_service.cancel(upload_id);
            }
        });
        let result = tokio::time::timeout(
            Duration::from_secs(2),
            service.upload(
                BlossomUploadRequest {
                    selection_id: selected.selection_id,
                    server: format!("http://{address}/"),
                    upload_id: Some(upload_id),
                    preflight: false,
                },
                progress,
            ),
        )
        .await
        .expect("cancellation should be prompt");
        assert!(
            matches!(
                result,
                Err(BlossomUploadError::CancelledRemotePartialPossible)
            ),
            "unexpected cancellation result: {result:?}"
        );
        server_task.abort();
    }

    #[tokio::test]
    async fn blossom_upload_rechecks_account_after_signing() {
        let keys = Keys::generate();
        let publisher = keys.public_key();
        let signer = LocalSigner::from_hex(&keys.secret_key().to_secret_hex())
            .expect("construct switching signer");
        let shared_publisher = Arc::new(Mutex::new(publisher));
        let provider = Arc::new(TestProvider {
            publisher: shared_publisher.clone(),
            signer: Arc::new(AccountSwitchSigner {
                signer,
                publisher: shared_publisher,
                replacement: Keys::generate().public_key(),
            }),
        });
        let service = development_service(provider);
        let file = file_with_suffix(&png_fixture(), ".png");
        let selected = service
            .register_file(file.path(), publisher)
            .expect("register");
        let error = service
            .upload(
                BlossomUploadRequest {
                    selection_id: selected.selection_id,
                    server: "http://127.0.0.1:9/".into(),
                    upload_id: None,
                    preflight: false,
                },
                no_progress(),
            )
            .await
            .expect_err("account switch rejected");
        assert!(matches!(error, BlossomUploadError::AccountMismatch));
    }

    #[test]
    fn blossom_upload_size_boundaries_use_phase_one_policy_without_large_files() {
        assert!(validate_blossom_media(
            "image/png",
            arcadestr_core::blossom::BLOSSOM_IMAGE_MAX_BYTES
        )
        .is_ok());
        assert!(validate_blossom_media(
            "image/png",
            arcadestr_core::blossom::BLOSSOM_IMAGE_MAX_BYTES + 1
        )
        .is_err());
        assert!(validate_blossom_media(
            "video/mp4",
            arcadestr_core::blossom::BLOSSOM_VIDEO_MAX_BYTES
        )
        .is_ok());
    }

    #[test]
    fn blossom_upload_structural_video_fixtures() {
        let mut mp4 = Vec::new();
        mp4.extend_from_slice(&20_u32.to_be_bytes());
        mp4.extend_from_slice(b"ftyp");
        mp4.extend_from_slice(b"isom");
        mp4.extend_from_slice(&0_u32.to_be_bytes());
        mp4.extend_from_slice(b"mp42");
        let file = file_with_suffix(&mp4, ".mp4");
        let mut handle = std::fs::File::open(file.path()).expect("open");
        assert!(validate_mp4(&mut handle, mp4.len() as u64).is_ok());
        assert!(validate_webm(&webm_fixture()).is_ok());
        assert!(validate_webm(&[
            0x1a, 0x45, 0xdf, 0xa3, 0x86, 0x42, 0x82, 0x83, b'm', b'a', b't', 0x18, 0x53, 0x80,
            0x67, 0xff,
        ])
        .is_err());
    }

    #[tokio::test]
    async fn blossom_upload_http_accepts_201_and_200_and_removes_successful_selection() {
        for status in [201, 200] {
            let (provider, publisher) = provider();
            let service = development_service(provider);
            let file = file_with_suffix(&png_fixture(), ".png");
            let selected = service
                .register_file(file.path(), publisher)
                .expect("register");
            let (server, captured) = blossom_fixture(vec![(status, FixtureBody::Valid)]).await;
            let result = service
                .upload(
                    BlossomUploadRequest {
                        selection_id: selected.selection_id,
                        server,
                        upload_id: None,
                        preflight: false,
                    },
                    no_progress(),
                )
                .await
                .expect("upload succeeds");
            assert_eq!(result.size, png_fixture().len() as u64);
            assert_eq!(result.was_existing, status == 200);
            assert!(matches!(
                service.inspect_selection(selected.selection_id).await,
                Err(BlossomUploadError::SelectionUnavailable)
            ));
            assert_eq!(captured.await.expect("captured").len(), 1);
        }
    }

    #[tokio::test]
    async fn blossom_upload_preflight_unsupported_continues_to_put() {
        let (provider, publisher) = provider();
        let service = development_service(provider);
        let file = file_with_suffix(&png_fixture(), ".png");
        let selected = service
            .register_file(file.path(), publisher)
            .expect("register");
        let (server, captured) =
            blossom_fixture(vec![(405, FixtureBody::Empty), (201, FixtureBody::Valid)]).await;
        service
            .upload(
                BlossomUploadRequest {
                    selection_id: selected.selection_id,
                    server,
                    upload_id: None,
                    preflight: true,
                },
                no_progress(),
            )
            .await
            .expect("unsupported preflight continues");
        let requests = captured.await.expect("captured");
        assert_eq!(requests.len(), 2);
        let preflight = requests[0].to_ascii_lowercase();
        assert!(preflight.contains("x-content-type: image/png"));
        assert!(preflight.contains(&format!("x-content-length: {}", png_fixture().len())));
    }

    #[tokio::test]
    async fn blossom_upload_http_types_payment_redirect_large_body_and_descriptor_mismatch() {
        for (status, mode, expected) in [
            (402, FixtureBody::Empty, "payment"),
            (302, FixtureBody::Empty, "redirect"),
            (200, FixtureBody::Oversized, "large"),
            (200, FixtureBody::WrongHash, "descriptor"),
            (200, FixtureBody::WrongSize, "descriptor"),
            (200, FixtureBody::WrongMime, "descriptor"),
        ] {
            let (provider, publisher) = provider();
            let service = development_service(provider);
            let file = file_with_suffix(&png_fixture(), ".png");
            let selected = service
                .register_file(file.path(), publisher)
                .expect("register");
            let (server, _) = blossom_fixture(vec![(status, mode)]).await;
            let error = service
                .upload(
                    BlossomUploadRequest {
                        selection_id: selected.selection_id,
                        server,
                        upload_id: None,
                        preflight: false,
                    },
                    no_progress(),
                )
                .await
                .expect_err("upload must fail");
            assert!(match expected {
                "payment" => matches!(error, BlossomUploadError::PaymentRequired),
                "redirect" => matches!(error, BlossomUploadError::RedirectRejected),
                "large" => matches!(error, BlossomUploadError::ResponseTooLarge),
                _ => matches!(error, BlossomUploadError::InvalidDescriptor(_)),
            });
            assert!(
                service
                    .inspect_selection(selected.selection_id)
                    .await
                    .is_ok(),
                "failed upload keeps selection"
            );
        }
    }

    #[tokio::test]
    async fn blossom_upload_explicit_retry_uses_fresh_authorization() {
        let (provider, publisher) = provider();
        let service = development_service(provider);
        let file = file_with_suffix(&png_fixture(), ".png");
        let selected = service
            .register_file(file.path(), publisher)
            .expect("register");
        let (server, captured) =
            blossom_fixture(vec![(500, FixtureBody::Empty), (201, FixtureBody::Valid)]).await;
        let request = BlossomUploadRequest {
            selection_id: selected.selection_id,
            server,
            upload_id: None,
            preflight: false,
        };
        assert!(matches!(
            service.upload(request.clone(), no_progress()).await,
            Err(BlossomUploadError::Server { status: 500, .. })
        ));
        service
            .upload(request, no_progress())
            .await
            .expect("explicit retry succeeds");
        let requests = captured.await.expect("captured requests");
        let authorizations = requests
            .iter()
            .filter_map(|headers| {
                headers.lines().find_map(|line| {
                    line.strip_prefix("authorization: ")
                        .or_else(|| line.strip_prefix("Authorization: "))
                })
            })
            .collect::<Vec<_>>();
        assert_eq!(authorizations.len(), 2);
        assert_ne!(authorizations[0], authorizations[1]);
    }
}
