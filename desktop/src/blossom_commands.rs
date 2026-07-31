//! Safe Tauri command boundary for account-scoped Blossom uploads and settings.

use crate::adp_commands::resolve_active_signer;
use crate::blossom_settings::{
    resolve_blossom_server_candidates as resolve_candidates, BlossomServerConfigInput,
    BlossomServerSettings, BlossomServerSettingsRepository, BlossomSettingsError,
    ConfiguredBlossomServer,
};
use crate::blossom_upload::{
    BlossomAccountProvider, BlossomProgressCallback, BlossomUploadError, BlossomUploadPhase,
    BlossomUploadProgress, BlossomUploadRequest, BlossomUploadService,
};
use arcadestr_core::auth::AuthState;
use arcadestr_core::blossom::{validate_blossom_server_origin, BlossomServerOriginPolicy};
use arcadestr_core::nip46::AppSignerState;
use arcadestr_core::signers::NostrSigner;
use async_trait::async_trait;
use base64::Engine;
use futures_util::future::join_all;
use nostr::PublicKey;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tauri::Emitter;
use tauri_plugin_dialog::DialogExt;
use tokio::sync::Mutex as AsyncMutex;
use uuid::Uuid;

const MAX_REQUEST_ID_CHARS: usize = 128;

pub struct DesktopBlossomAccountProvider {
    auth: Arc<AsyncMutex<AuthState>>,
    signer_state: Arc<AsyncMutex<AppSignerState>>,
}

impl DesktopBlossomAccountProvider {
    pub fn new(
        auth: Arc<AsyncMutex<AuthState>>,
        signer_state: Arc<AsyncMutex<AppSignerState>>,
    ) -> Self {
        Self { auth, signer_state }
    }

    async fn signer(&self) -> Result<Arc<dyn NostrSigner>, BlossomUploadError> {
        // Never retain the auth guard while a remote signer can await user interaction.
        let auth = { self.auth.lock().await.clone() };
        let expected = auth
            .public_key()
            .ok_or(BlossomUploadError::AccountUnavailable)?;
        let signer = resolve_active_signer(&self.signer_state, &auth)
            .await
            .map_err(|_| BlossomUploadError::AccountUnavailable)?;
        let actual = signer
            .get_public_key()
            .await
            .map_err(|_| BlossomUploadError::AccountUnavailable)?;
        if actual != expected {
            return Err(BlossomUploadError::AccountMismatch);
        }
        Ok(signer)
    }

    async fn publisher(&self) -> Result<PublicKey, BlossomUploadError> {
        self.auth
            .lock()
            .await
            .public_key()
            .ok_or(BlossomUploadError::AccountUnavailable)
    }
}

#[async_trait]
impl BlossomAccountProvider for DesktopBlossomAccountProvider {
    async fn current_publisher(&self) -> Result<PublicKey, BlossomUploadError> {
        self.publisher().await
    }

    async fn current_signer(&self) -> Result<Arc<dyn NostrSigner>, BlossomUploadError> {
        self.signer().await
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct BlossomOperationContext {
    selection_id: Uuid,
    request_id: String,
    publisher_hex: String,
}

struct BlossomOperationGuard {
    upload_id: Uuid,
    operations: Arc<Mutex<HashMap<Uuid, BlossomOperationContext>>>,
}

impl Drop for BlossomOperationGuard {
    fn drop(&mut self) {
        if let Ok(mut operations) = self.operations.lock() {
            operations.remove(&self.upload_id);
        }
    }
}

pub struct BlossomManagedState {
    pub upload_service: Arc<BlossomUploadService>,
    pub settings: Arc<BlossomServerSettingsRepository>,
    operations: Arc<Mutex<HashMap<Uuid, BlossomOperationContext>>>,
}

impl BlossomManagedState {
    pub fn new(
        upload_service: Arc<BlossomUploadService>,
        settings: Arc<BlossomServerSettingsRepository>,
    ) -> Self {
        Self {
            upload_service,
            settings,
            operations: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BlossomCommandError {
    pub code: String,
    pub message: String,
}

impl BlossomCommandError {
    fn new(code: &str, message: &str) -> Self {
        Self {
            code: code.to_owned(),
            message: message.to_owned(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExpectedBlossomPublisherRequest {
    pub expected_publisher_hex: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BlossomMediaSelectionDto {
    pub selection_id: String,
    pub filename: String,
    pub detected_mime: String,
    pub size: u64,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub preview_data_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StartBlossomUploadRequest {
    pub selection_id: String,
    pub expected_publisher_hex: String,
    pub selected_server: Option<String>,
    pub preflight: bool,
    pub request_id: String,
}

pub type RetryBlossomUploadRequest = StartBlossomUploadRequest;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BlossomUploadResponse {
    pub upload_id: String,
    pub url: String,
    pub sha256: String,
    pub mime_type: String,
    pub size: u64,
    pub uploaded: u64,
    pub was_existing: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BlossomUploadProgressDto {
    pub upload_id: String,
    pub selection_id: String,
    pub request_id: String,
    pub publisher_pubkey: String,
    pub phase: String,
    pub bytes_completed: u64,
    pub total_bytes: u64,
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CancelBlossomUploadRequest {
    pub upload_id: String,
    pub expected_publisher_hex: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CancelBlossomUploadResponse {
    pub cancelled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DiscardBlossomMediaRequest {
    pub selection_id: String,
    pub expected_publisher_hex: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DiscardBlossomMediaResponse {
    pub cancelled_uploads: usize,
    pub selection_removed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BlossomServerDto {
    pub origin: String,
    pub label: Option<String>,
    pub enabled: bool,
    pub created_at: u64,
    pub updated_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BlossomServerSettingsDto {
    pub publisher_pubkey: String,
    pub servers: Vec<BlossomServerDto>,
    pub preferred_server: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BlossomServerHealthDto {
    pub origin: String,
    pub status: String,
    pub latency_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BlossomServerHealthResponse {
    pub publisher_pubkey: String,
    pub servers: Vec<BlossomServerHealthDto>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BlossomServerInputDto {
    pub origin: String,
    pub label: Option<String>,
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReplaceBlossomServerSettingsRequest {
    pub expected_publisher_hex: String,
    pub servers: Vec<BlossomServerInputDto>,
    pub preferred_server: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AddBlossomServerRequest {
    pub expected_publisher_hex: String,
    pub origin: String,
    pub label: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct UpdateBlossomServerRequest {
    pub expected_publisher_hex: String,
    pub origin: String,
    pub label: Option<String>,
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BlossomServerOriginRequest {
    pub expected_publisher_hex: String,
    pub origin: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReorderBlossomServersRequest {
    pub expected_publisher_hex: String,
    pub ordered_origins: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SetPreferredBlossomServerRequest {
    pub expected_publisher_hex: String,
    pub origin: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ResolveBlossomServerCandidatesRequest {
    pub expected_publisher_hex: String,
    pub explicit_server: Option<String>,
}

#[cfg(debug_assertions)]
fn origin_policy() -> BlossomServerOriginPolicy {
    BlossomServerOriginPolicy::AllowHttpLoopback
}

#[cfg(not(debug_assertions))]
fn origin_policy() -> BlossomServerOriginPolicy {
    BlossomServerOriginPolicy::HttpsOnly
}

fn parse_publisher(value: &str) -> Result<PublicKey, BlossomCommandError> {
    let publisher = PublicKey::from_hex(value)
        .map_err(|_| BlossomCommandError::new("invalid_request", "Invalid publisher key."))?;
    if value.len() != 64 || publisher.to_hex() != value {
        return Err(BlossomCommandError::new(
            "invalid_request",
            "Publisher key must be canonical lowercase hexadecimal.",
        ));
    }
    Ok(publisher)
}

fn parse_uuid(value: &str, field: &str) -> Result<Uuid, BlossomCommandError> {
    let parsed = Uuid::parse_str(value)
        .map_err(|_| BlossomCommandError::new("invalid_request", &format!("Invalid {field}.")))?;
    if parsed.to_string() != value {
        return Err(BlossomCommandError::new(
            "invalid_request",
            &format!("Invalid {field}."),
        ));
    }
    Ok(parsed)
}

fn validate_request_id(value: &str) -> Result<(), BlossomCommandError> {
    let chars = value.chars().count();
    if chars == 0
        || chars > MAX_REQUEST_ID_CHARS
        || value.trim() != value
        || value.chars().any(char::is_control)
    {
        return Err(BlossomCommandError::new(
            "invalid_request",
            "Request id is invalid.",
        ));
    }
    Ok(())
}

async fn verify_publisher(
    service: &BlossomUploadService,
    expected: PublicKey,
) -> Result<(), BlossomCommandError> {
    let current = service
        .current_publisher()
        .await
        .map_err(map_upload_error)?;
    if current != expected {
        return Err(BlossomCommandError::new(
            "account_changed",
            "The active account changed.",
        ));
    }
    Ok(())
}

fn map_upload_error(error: BlossomUploadError) -> BlossomCommandError {
    use BlossomUploadError::*;
    match error {
        SelectionUnavailable => BlossomCommandError::new(
            "selection_unavailable",
            "The media selection is unavailable.",
        ),
        AccountMismatch => {
            BlossomCommandError::new("account_changed", "The active account changed.")
        }
        AccountUnavailable => BlossomCommandError::new(
            "account_unavailable",
            "No active account signer is available.",
        ),
        UploadAlreadyActive => {
            BlossomCommandError::new("invalid_request", "The upload is already active.")
        }
        CancelledRemotePartialPossible => {
            BlossomCommandError::new("cancelled", "The upload was cancelled.")
        }
        InvalidMedia(message)
            if message.to_ascii_lowercase().contains("size")
                || message.to_ascii_lowercase().contains("large")
                || message.to_ascii_lowercase().contains("limit") =>
        {
            BlossomCommandError::new("file_too_large", "The selected file is too large.")
        }
        InvalidMedia(_) => {
            BlossomCommandError::new("unsupported_file", "The selected file is unsupported.")
        }
        FileChanged => BlossomCommandError::new(
            "integrity_mismatch",
            "The selected file changed during processing.",
        ),
        Io(_) => {
            BlossomCommandError::new("storage_failure", "The selected file could not be read.")
        }
        Destination(_) => {
            BlossomCommandError::new("invalid_server", "The Blossom server is invalid.")
        }
        ForbiddenAddress(_) | RedirectRejected => BlossomCommandError::new(
            "unsafe_destination",
            "The Blossom destination is not permitted.",
        ),
        Server { status: 401, .. } => BlossomCommandError::new(
            "authorization_rejected",
            "The Blossom server rejected the upload authorization.",
        ),
        Server { status: 413, .. } => BlossomCommandError::new(
            "file_too_large",
            "The Blossom server rejected the file size.",
        ),
        Server { status: 415, .. } => BlossomCommandError::new(
            "unsupported_file",
            "The Blossom server rejected the media type.",
        ),
        Server { status: 429, .. } => BlossomCommandError::new(
            "rate_limited",
            "The Blossom server is temporarily rate limiting uploads.",
        ),
        ResponseTooLarge => BlossomCommandError::new(
            "malformed_descriptor",
            "The Blossom server response was too large.",
        ),
        Dns(_) | Http(_) | Server { .. } => {
            BlossomCommandError::new("network_failure", "The Blossom server request failed.")
        }
        SignerRejected(_) => {
            BlossomCommandError::new("signer_rejected", "The signer rejected the upload.")
        }
        SignerTimeout => {
            BlossomCommandError::new("signer_timeout", "The signer did not respond in time.")
        }
        InvalidSignedAuthorization(_) => BlossomCommandError::new(
            "signer_rejected",
            "The signer returned an invalid authorization.",
        ),
        PaymentRequired => {
            BlossomCommandError::new("payment_required", "The Blossom server requires payment.")
        }
        InvalidDescriptor(message)
            if ["hash", "size", "mime", "type"]
                .iter()
                .any(|part| message.to_ascii_lowercase().contains(part)) =>
        {
            BlossomCommandError::new(
                "integrity_mismatch",
                "The Blossom response did not match the uploaded file.",
            )
        }
        InvalidDescriptor(_) => BlossomCommandError::new(
            "malformed_descriptor",
            "The Blossom server returned an invalid descriptor.",
        ),
    }
}

fn map_settings_error(error: BlossomSettingsError) -> BlossomCommandError {
    use BlossomSettingsError::*;
    match error {
        AccountUnavailable => BlossomCommandError::new(
            "account_unavailable",
            "No active account signer is available.",
        ),
        AccountMismatch => {
            BlossomCommandError::new("account_changed", "The active account changed.")
        }
        InvalidOrigin(_) => {
            BlossomCommandError::new("invalid_server", "The Blossom server is invalid.")
        }
        NoConfiguredServers => BlossomCommandError::new(
            "no_configured_servers",
            "No enabled Blossom servers are configured.",
        ),
        Storage(_) | InvalidTimestamp => {
            BlossomCommandError::new("storage_failure", "Blossom settings could not be saved.")
        }
        InvalidLabel
        | DuplicateServer
        | ServerNotFound
        | PreferredServerDisabled
        | InvalidOrder => BlossomCommandError::new(
            "invalid_request",
            "The Blossom settings request is invalid.",
        ),
    }
}

fn settings_dto(settings: BlossomServerSettings) -> BlossomServerSettingsDto {
    BlossomServerSettingsDto {
        publisher_pubkey: settings.publisher_pubkey,
        servers: settings.servers.into_iter().map(server_dto).collect(),
        preferred_server: settings.preferred_server,
    }
}

fn server_dto(server: ConfiguredBlossomServer) -> BlossomServerDto {
    BlossomServerDto {
        origin: server.origin,
        label: server.label,
        enabled: server.enabled,
        created_at: server.created_at,
        updated_at: server.updated_at,
    }
}

fn phase_name(phase: BlossomUploadPhase) -> &'static str {
    match phase {
        BlossomUploadPhase::Inspect => "inspect",
        BlossomUploadPhase::Hash => "hash",
        BlossomUploadPhase::Sign => "sign",
        BlossomUploadPhase::Preflight => "preflight",
        BlossomUploadPhase::Upload => "upload",
        BlossomUploadPhase::Verify => "verify",
        BlossomUploadPhase::Complete => "complete",
    }
}

fn progress_payload(
    context: &BlossomOperationContext,
    progress: BlossomUploadProgress,
) -> BlossomUploadProgressDto {
    BlossomUploadProgressDto {
        upload_id: progress.upload_id.to_string(),
        selection_id: context.selection_id.to_string(),
        request_id: context.request_id.clone(),
        publisher_pubkey: context.publisher_hex.clone(),
        phase: phase_name(progress.phase).to_owned(),
        bytes_completed: progress.bytes,
        total_bytes: progress.total,
        message: progress.message,
    }
}

fn emit_if_current<F>(
    operations: &Mutex<HashMap<Uuid, BlossomOperationContext>>,
    upload_id: Uuid,
    progress: BlossomUploadProgress,
    mut emit: F,
) where
    F: FnMut(BlossomUploadProgressDto) -> Result<(), ()>,
{
    if progress.upload_id != upload_id {
        return;
    }
    let context = operations
        .lock()
        .ok()
        .and_then(|operations| operations.get(&upload_id).cloned());
    if let Some(context) = context {
        let _ = emit(progress_payload(&context, progress));
    }
}

fn emit_terminal(
    app: &tauri::AppHandle,
    operations: &Mutex<HashMap<Uuid, BlossomOperationContext>>,
    upload_id: Uuid,
    phase: &str,
    message: String,
) {
    let context = operations
        .lock()
        .ok()
        .and_then(|operations| operations.get(&upload_id).cloned());
    if let Some(context) = context {
        emit_terminal_context(app, upload_id, context, phase, message);
    }
}

fn emit_terminal_context(
    app: &tauri::AppHandle,
    upload_id: Uuid,
    context: BlossomOperationContext,
    phase: &str,
    message: String,
) {
    let payload = BlossomUploadProgressDto {
        upload_id: upload_id.to_string(),
        selection_id: context.selection_id.to_string(),
        request_id: context.request_id,
        publisher_pubkey: context.publisher_hex,
        phase: phase.to_owned(),
        bytes_completed: 0,
        total_bytes: 0,
        message: Some(message),
    };
    let _ = app.emit("blossom-upload-progress", payload);
}

fn selected_path(path: Option<tauri_plugin_dialog::FilePath>) -> Option<PathBuf> {
    path.and_then(|value| value.into_path().ok())
}

#[tauri::command]
pub async fn select_blossom_media_file(
    app: tauri::AppHandle,
    state: tauri::State<'_, BlossomManagedState>,
    request: ExpectedBlossomPublisherRequest,
) -> Result<Option<BlossomMediaSelectionDto>, BlossomCommandError> {
    let publisher = parse_publisher(&request.expected_publisher_hex)?;
    verify_publisher(&state.upload_service, publisher).await?;
    let (sender, receiver) = tokio::sync::oneshot::channel();
    app.dialog()
        .file()
        .add_filter(
            "Blossom media",
            &["png", "jpg", "jpeg", "webp", "mp4", "webm"],
        )
        .pick_file(move |path| {
            let _ = sender.send(path);
        });
    let path = receiver.await.ok().and_then(selected_path);
    let Some(path) = path else {
        return Ok(None);
    };
    verify_publisher(&state.upload_service, publisher).await?;
    let preview_path = path.clone();
    let selected = state
        .upload_service
        .register_file(path, publisher)
        .map_err(map_upload_error)?;
    match state
        .upload_service
        .inspect_selection(selected.selection_id)
        .await
    {
        Ok(inspected) => {
            let preview_data_url = if inspected.mime_type.starts_with("image/") {
                let bytes = match tokio::fs::read(preview_path).await {
                    Ok(bytes) => bytes,
                    Err(_) => {
                        let _ = state
                            .upload_service
                            .cleanup_selection(inspected.selection_id);
                        return Err(BlossomCommandError::new(
                            "storage_failure",
                            "The local media preview failed.",
                        ));
                    }
                };
                Some(format!(
                    "data:{};base64,{}",
                    inspected.mime_type,
                    base64::engine::general_purpose::STANDARD.encode(bytes)
                ))
            } else {
                None
            };
            Ok(Some(BlossomMediaSelectionDto {
                selection_id: inspected.selection_id.to_string(),
                filename: inspected.filename,
                detected_mime: inspected.mime_type,
                size: inspected.size,
                width: inspected.width,
                height: inspected.height,
                preview_data_url,
            }))
        }
        Err(error) => {
            let _ = state
                .upload_service
                .cleanup_selection(selected.selection_id);
            Err(map_upload_error(error))
        }
    }
}

async fn run_upload(
    app: tauri::AppHandle,
    state: &BlossomManagedState,
    request: StartBlossomUploadRequest,
) -> Result<BlossomUploadResponse, BlossomCommandError> {
    validate_request_id(&request.request_id)?;
    let publisher = parse_publisher(&request.expected_publisher_hex)?;
    verify_publisher(&state.upload_service, publisher).await?;
    let selection_id = parse_uuid(&request.selection_id, "selection id")?;
    let settings = state
        .settings
        .list(publisher)
        .await
        .map_err(map_settings_error)?;
    let server = resolve_candidates(
        &settings,
        request.selected_server.as_deref(),
        origin_policy(),
    )
    .map_err(map_settings_error)?
    .into_iter()
    .next()
    .ok_or_else(|| {
        BlossomCommandError::new(
            "no_configured_servers",
            "No enabled Blossom servers are configured.",
        )
    })?;
    let upload_id = Uuid::new_v4();
    let context = BlossomOperationContext {
        selection_id,
        request_id: request.request_id,
        publisher_hex: publisher.to_hex(),
    };
    {
        let mut operations = state.operations.lock().map_err(|_| {
            BlossomCommandError::new("storage_failure", "Upload state is unavailable.")
        })?;
        if operations
            .values()
            .any(|operation| operation.selection_id == selection_id)
        {
            return Err(BlossomCommandError::new(
                "selection_busy",
                "This file already has an active upload.",
            ));
        }
        operations.insert(upload_id, context);
    }
    let _operation_guard = BlossomOperationGuard {
        upload_id,
        operations: Arc::clone(&state.operations),
    };

    let app_for_progress = app.clone();
    let operations = state.operations.clone();
    let callback: BlossomProgressCallback = Arc::new(move |progress| {
        emit_if_current(&operations, upload_id, progress, |payload| {
            app_for_progress
                .emit("blossom-upload-progress", payload)
                .map_err(|_| ())
        });
    });
    let result = state
        .upload_service
        .upload(
            BlossomUploadRequest {
                selection_id,
                server,
                upload_id: Some(upload_id),
                preflight: request.preflight,
            },
            callback,
        )
        .await;
    let response = match result {
        Ok(result) => Ok(BlossomUploadResponse {
            upload_id: result.upload_id.to_string(),
            url: result.descriptor.url,
            sha256: result.sha256,
            mime_type: result.mime_type,
            size: result.size,
            uploaded: result.descriptor.uploaded,
            was_existing: result.was_existing,
        }),
        Err(error) => {
            let mapped = map_upload_error(error);
            let phase = if mapped.code == "cancelled" {
                "cancelled"
            } else {
                "failed"
            };
            emit_terminal(
                &app,
                &state.operations,
                upload_id,
                phase,
                mapped.message.clone(),
            );
            Err(mapped)
        }
    };
    response
}

#[tauri::command]
pub async fn start_blossom_upload(
    app: tauri::AppHandle,
    state: tauri::State<'_, BlossomManagedState>,
    request: StartBlossomUploadRequest,
) -> Result<BlossomUploadResponse, BlossomCommandError> {
    run_upload(app, state.inner(), request).await
}

#[tauri::command]
pub async fn retry_blossom_upload(
    app: tauri::AppHandle,
    state: tauri::State<'_, BlossomManagedState>,
    request: RetryBlossomUploadRequest,
) -> Result<BlossomUploadResponse, BlossomCommandError> {
    run_upload(app, state.inner(), request).await
}

#[tauri::command]
pub async fn cancel_blossom_upload(
    state: tauri::State<'_, BlossomManagedState>,
    request: CancelBlossomUploadRequest,
) -> Result<CancelBlossomUploadResponse, BlossomCommandError> {
    let publisher = parse_publisher(&request.expected_publisher_hex)?;
    let upload_id = parse_uuid(&request.upload_id, "upload id")?;
    if let Some(context) = state
        .operations
        .lock()
        .map_err(|_| BlossomCommandError::new("storage_failure", "Upload state is unavailable."))?
        .get(&upload_id)
    {
        if context.publisher_hex != publisher.to_hex() {
            return Err(BlossomCommandError::new(
                "account_changed",
                "The upload belongs to another account.",
            ));
        }
    }
    Ok(CancelBlossomUploadResponse {
        cancelled: state
            .upload_service
            .cancel(upload_id)
            .map_err(map_upload_error)?,
    })
}

#[tauri::command]
pub async fn discard_blossom_media_selection(
    app: tauri::AppHandle,
    state: tauri::State<'_, BlossomManagedState>,
    request: DiscardBlossomMediaRequest,
) -> Result<DiscardBlossomMediaResponse, BlossomCommandError> {
    let publisher = parse_publisher(&request.expected_publisher_hex)?;
    let selection_id = parse_uuid(&request.selection_id, "selection id")?;
    let upload_ids = state
        .operations
        .lock()
        .map_err(|_| BlossomCommandError::new("storage_failure", "Upload state is unavailable."))?
        .iter()
        .filter_map(|(id, context)| {
            (context.selection_id == selection_id && context.publisher_hex == publisher.to_hex())
                .then_some(*id)
        })
        .collect::<Vec<_>>();
    let mut cancelled_uploads = 0;
    for upload_id in &upload_ids {
        if state
            .upload_service
            .cancel(*upload_id)
            .map_err(map_upload_error)?
        {
            cancelled_uploads += 1;
        }
    }
    let removed = state
        .operations
        .lock()
        .map_err(|_| BlossomCommandError::new("storage_failure", "Upload state is unavailable."))
        .map(|mut operations| {
            upload_ids
                .into_iter()
                .filter_map(|upload_id| {
                    operations
                        .remove(&upload_id)
                        .map(|context| (upload_id, context))
                })
                .collect::<Vec<_>>()
        })?;
    for (upload_id, context) in removed {
        emit_terminal_context(
            &app,
            upload_id,
            context,
            "cancelled",
            "The upload was cancelled.".to_string(),
        );
    }
    Ok(DiscardBlossomMediaResponse {
        cancelled_uploads,
        selection_removed: state
            .upload_service
            .cleanup_selection_for(selection_id, publisher)
            .map_err(map_upload_error)?,
    })
}

async fn load_settings(
    state: &BlossomManagedState,
    publisher: PublicKey,
) -> Result<BlossomServerSettingsDto, BlossomCommandError> {
    state
        .settings
        .list(publisher)
        .await
        .map(settings_dto)
        .map_err(map_settings_error)
}

#[tauri::command]
pub async fn get_blossom_server_settings(
    state: tauri::State<'_, BlossomManagedState>,
    request: ExpectedBlossomPublisherRequest,
) -> Result<BlossomServerSettingsDto, BlossomCommandError> {
    load_settings(
        state.inner(),
        parse_publisher(&request.expected_publisher_hex)?,
    )
    .await
}

#[tauri::command]
pub async fn probe_blossom_server_health(
    state: tauri::State<'_, BlossomManagedState>,
    request: ExpectedBlossomPublisherRequest,
) -> Result<BlossomServerHealthResponse, BlossomCommandError> {
    let publisher = parse_publisher(&request.expected_publisher_hex)?;
    verify_publisher(&state.upload_service, publisher).await?;
    let settings = state
        .settings
        .list(publisher)
        .await
        .map_err(map_settings_error)?;
    let probes = settings.servers.into_iter().map(|server| {
        let service = Arc::clone(&state.upload_service);
        async move {
            let result = service.probe_server(&server.origin).await;
            match result {
                Ok(elapsed) => {
                    let latency_ms = u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX);
                    BlossomServerHealthDto {
                        origin: server.origin,
                        status: if latency_ms > 1_000 { "slow" } else { "online" }.into(),
                        latency_ms: Some(latency_ms),
                    }
                }
                Err(_) => BlossomServerHealthDto {
                    origin: server.origin,
                    status: "offline".into(),
                    latency_ms: None,
                },
            }
        }
    });
    let servers = join_all(probes).await;
    verify_publisher(&state.upload_service, publisher).await?;
    Ok(BlossomServerHealthResponse {
        publisher_pubkey: publisher.to_hex(),
        servers,
    })
}

#[tauri::command]
pub async fn replace_blossom_server_settings(
    state: tauri::State<'_, BlossomManagedState>,
    request: ReplaceBlossomServerSettingsRequest,
) -> Result<BlossomServerSettingsDto, BlossomCommandError> {
    let publisher = parse_publisher(&request.expected_publisher_hex)?;
    let servers = request
        .servers
        .into_iter()
        .map(|server| BlossomServerConfigInput {
            origin: server.origin,
            label: server.label,
            enabled: server.enabled,
        })
        .collect::<Vec<_>>();
    state
        .settings
        .replace(publisher, &servers, request.preferred_server.as_deref())
        .await
        .map_err(map_settings_error)?;
    load_settings(state.inner(), publisher).await
}

#[tauri::command]
pub async fn add_blossom_server(
    state: tauri::State<'_, BlossomManagedState>,
    request: AddBlossomServerRequest,
) -> Result<BlossomServerSettingsDto, BlossomCommandError> {
    let publisher = parse_publisher(&request.expected_publisher_hex)?;
    state
        .settings
        .add_server(publisher, &request.origin, request.label.as_deref())
        .await
        .map_err(map_settings_error)?;
    load_settings(state.inner(), publisher).await
}

#[tauri::command]
pub async fn update_blossom_server(
    state: tauri::State<'_, BlossomManagedState>,
    request: UpdateBlossomServerRequest,
) -> Result<BlossomServerSettingsDto, BlossomCommandError> {
    let publisher = parse_publisher(&request.expected_publisher_hex)?;
    let origin = validate_blossom_server_origin(&request.origin, origin_policy())
        .map_err(|_| BlossomCommandError::new("invalid_server", "The Blossom server is invalid."))?
        .as_str()
        .to_owned();
    let current = state
        .settings
        .list(publisher)
        .await
        .map_err(map_settings_error)?;
    if !current.servers.iter().any(|server| server.origin == origin) {
        return Err(BlossomCommandError::new(
            "invalid_request",
            "The Blossom server is not configured.",
        ));
    }
    let servers = current
        .servers
        .iter()
        .map(|server| BlossomServerConfigInput {
            origin: server.origin.clone(),
            label: if server.origin == origin {
                request.label.clone()
            } else {
                server.label.clone()
            },
            enabled: if server.origin == origin {
                request.enabled
            } else {
                server.enabled
            },
        })
        .collect::<Vec<_>>();
    let preferred = current
        .preferred_server
        .filter(|preferred| preferred != &origin || request.enabled);
    state
        .settings
        .replace(publisher, &servers, preferred.as_deref())
        .await
        .map_err(map_settings_error)?;
    load_settings(state.inner(), publisher).await
}

#[tauri::command]
pub async fn remove_blossom_server(
    state: tauri::State<'_, BlossomManagedState>,
    request: BlossomServerOriginRequest,
) -> Result<BlossomServerSettingsDto, BlossomCommandError> {
    let publisher = parse_publisher(&request.expected_publisher_hex)?;
    state
        .settings
        .remove_server(publisher, &request.origin)
        .await
        .map_err(map_settings_error)?;
    load_settings(state.inner(), publisher).await
}

#[tauri::command]
pub async fn reorder_blossom_servers(
    state: tauri::State<'_, BlossomManagedState>,
    request: ReorderBlossomServersRequest,
) -> Result<BlossomServerSettingsDto, BlossomCommandError> {
    let publisher = parse_publisher(&request.expected_publisher_hex)?;
    state
        .settings
        .reorder(publisher, &request.ordered_origins)
        .await
        .map_err(map_settings_error)?;
    load_settings(state.inner(), publisher).await
}

#[tauri::command]
pub async fn set_preferred_blossom_server(
    state: tauri::State<'_, BlossomManagedState>,
    request: SetPreferredBlossomServerRequest,
) -> Result<BlossomServerSettingsDto, BlossomCommandError> {
    let publisher = parse_publisher(&request.expected_publisher_hex)?;
    state
        .settings
        .set_preferred(publisher, request.origin.as_deref())
        .await
        .map_err(map_settings_error)?;
    load_settings(state.inner(), publisher).await
}

#[tauri::command]
pub async fn resolve_blossom_server_candidates(
    state: tauri::State<'_, BlossomManagedState>,
    request: ResolveBlossomServerCandidatesRequest,
) -> Result<Vec<String>, BlossomCommandError> {
    let publisher = parse_publisher(&request.expected_publisher_hex)?;
    let settings = state
        .settings
        .list(publisher)
        .await
        .map_err(map_settings_error)?;
    resolve_candidates(
        &settings,
        request.explicit_server.as_deref(),
        origin_policy(),
    )
    .map_err(map_settings_error)
}

#[cfg(test)]
mod tests {
    use super::*;
    use arcadestr_core::signers::LocalSigner;
    use nostr::Keys;

    struct ManagedTestProvider {
        publisher: PublicKey,
        signer: Arc<dyn NostrSigner>,
    }

    #[async_trait]
    impl BlossomAccountProvider for ManagedTestProvider {
        async fn current_publisher(&self) -> Result<PublicKey, BlossomUploadError> {
            Ok(self.publisher)
        }

        async fn current_signer(&self) -> Result<Arc<dyn NostrSigner>, BlossomUploadError> {
            Ok(self.signer.clone())
        }
    }

    #[tokio::test]
    async fn blossom_account_provider_requires_current_authenticated_identity() {
        let keys = Keys::generate();
        let auth = Arc::new(AsyncMutex::new(AuthState::new()));
        auth.lock()
            .await
            .connect_with_key(&keys.secret_key().to_secret_hex())
            .expect("connect local account");
        let provider = DesktopBlossomAccountProvider::new(
            Arc::clone(&auth),
            Arc::new(AsyncMutex::new(AppSignerState::new())),
        );
        assert_eq!(
            provider.current_publisher().await.expect("publisher"),
            keys.public_key()
        );
        assert_eq!(
            provider
                .current_signer()
                .await
                .expect("authenticated signer")
                .get_public_key()
                .await
                .expect("signer publisher"),
            keys.public_key()
        );

        auth.lock().await.disconnect();
        assert!(matches!(
            provider.current_publisher().await,
            Err(BlossomUploadError::AccountUnavailable)
        ));
        assert!(matches!(
            provider.current_signer().await,
            Err(BlossomUploadError::AccountUnavailable)
        ));
    }

    #[tokio::test]
    async fn blossom_managed_state_constructs_with_shared_services() {
        let keys = Keys::generate();
        let publisher = keys.public_key();
        let signer = LocalSigner::from_hex(&keys.secret_key().to_secret_hex())
            .expect("construct local signer");
        let provider: Arc<dyn BlossomAccountProvider> = Arc::new(ManagedTestProvider {
            publisher,
            signer: Arc::new(signer),
        });
        let directory = tempfile::tempdir().expect("temporary directory");
        let database = Arc::new(
            arcadestr_core::storage::Database::new(&directory.path().join("blossom.db"))
                .await
                .expect("database"),
        );
        let service = Arc::new(BlossomUploadService::new(
            provider.clone(),
            crate::blossom_upload::BlossomUploadConfig::default(),
        ));
        let settings = Arc::new(BlossomServerSettingsRepository::development_loopback(
            database, provider,
        ));
        let state = BlossomManagedState::new(service.clone(), settings.clone());
        assert!(Arc::ptr_eq(&state.upload_service, &service));
        assert!(Arc::ptr_eq(&state.settings, &settings));
        assert!(state.operations.lock().expect("operations").is_empty());
    }

    #[test]
    fn blossom_safe_dtos_never_serialize_backend_file_or_auth_data() {
        let dto = BlossomMediaSelectionDto {
            selection_id: Uuid::nil().to_string(),
            filename: "cover.png".into(),
            detected_mime: "image/png".into(),
            size: 42,
            width: Some(1),
            height: Some(1),
            preview_data_url: Some("data:image/png;base64,AA==".into()),
        };
        let json = serde_json::to_string(&dto).expect("serialize safe selection");
        for forbidden in ["path", "authorization", "secret", "bytes"] {
            assert!(!json.to_ascii_lowercase().contains(forbidden));
        }
    }

    #[test]
    fn blossom_error_mapping_is_stable_and_does_not_expose_details() {
        let cases = [
            (BlossomUploadError::PaymentRequired, "payment_required"),
            (BlossomUploadError::SignerTimeout, "signer_timeout"),
            (BlossomUploadError::AccountMismatch, "account_changed"),
            (
                BlossomUploadError::Destination("private detail".into()),
                "invalid_server",
            ),
            (
                BlossomUploadError::ForbiddenAddress("127.0.0.1".parse().expect("IP")),
                "unsafe_destination",
            ),
            (
                BlossomUploadError::InvalidDescriptor("hash mismatch: secret".into()),
                "integrity_mismatch",
            ),
            (
                BlossomUploadError::Http("Authorization: secret".into()),
                "network_failure",
            ),
        ];
        for (error, code) in cases {
            let mapped = map_upload_error(error);
            assert_eq!(mapped.code, code);
            assert!(!mapped.message.contains("secret"));
        }
        assert_eq!(
            map_settings_error(BlossomSettingsError::NoConfiguredServers).code,
            "no_configured_servers"
        );
        assert_eq!(
            map_settings_error(BlossomSettingsError::Storage("secret".into())).code,
            "storage_failure"
        );
    }

    #[test]
    fn blossom_operation_registry_correlates_and_suppresses_late_events() {
        let upload_id = Uuid::new_v4();
        let selection_id = Uuid::new_v4();
        let operations = Mutex::new(HashMap::from([(
            upload_id,
            BlossomOperationContext {
                selection_id,
                request_id: "request-1".into(),
                publisher_hex: "ab".repeat(32),
            },
        )]));
        let progress = BlossomUploadProgress {
            upload_id,
            phase: BlossomUploadPhase::Hash,
            bytes: 3,
            total: 5,
            message: None,
        };
        let mut emitted = Vec::new();
        emit_if_current(&operations, upload_id, progress.clone(), |value| {
            emitted.push(value);
            Ok(())
        });
        assert_eq!(emitted[0].request_id, "request-1");
        operations.lock().expect("operations").remove(&upload_id);
        emit_if_current(&operations, upload_id, progress.clone(), |value| {
            emitted.push(value);
            Ok(())
        });
        assert_eq!(emitted.len(), 1);
        emit_if_current(&operations, upload_id, progress.clone(), |_| {
            panic!("emitter must not run for removed operation")
        });
        operations.lock().expect("operations").insert(
            upload_id,
            BlossomOperationContext {
                selection_id,
                request_id: "request-1".into(),
                publisher_hex: "ab".repeat(32),
            },
        );
        emit_if_current(&operations, upload_id, progress, |_| Err(()));
    }

    #[test]
    fn blossom_picker_cancellation_helper_is_none() {
        assert_eq!(selected_path(None), None);
    }

    #[test]
    fn blossom_request_validation_requires_canonical_account_and_bounded_id() {
        assert!(parse_publisher(&"AB".repeat(32)).is_err());
        assert!(validate_request_id("").is_err());
        assert!(validate_request_id(&"x".repeat(MAX_REQUEST_ID_CHARS + 1)).is_err());
        assert!(validate_request_id("ui-request-1").is_ok());
    }
}
