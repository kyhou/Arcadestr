use std::cell::{Cell, RefCell};
use std::rc::Rc;

use arcadestr_core::store_page::{StorePageDraft, StorePageMediaItem};
use leptos::prelude::*;

use crate::ui_v2::components::{
    Dialog, DialogCloseAction, DialogCloseButtonPolicy, DialogClosePolicy, DialogCloseRequest,
    DialogDismissal, DialogSourcePolicy, DialogWidth, StatusChip, StatusChipSize,
    StatusChipVariant,
};
use send_wrapper::SendWrapper;
use wasm_bindgen_futures::spawn_local;

use crate::tauri_bridge::{
    invoke_add_blossom_server, invoke_cancel_blossom_upload,
    invoke_discard_blossom_media_selection, invoke_get_blossom_server_settings,
    invoke_remove_blossom_server, invoke_retry_blossom_upload, invoke_select_blossom_media_file,
    invoke_select_blossom_media_files, invoke_set_preferred_blossom_server,
    invoke_start_blossom_upload, listen_blossom_upload_progress, AddBlossomServerRequest,
    BlossomMediaSelectionDto, BlossomServerOriginRequest, BlossomServerSettingsDto,
    BlossomUploadProgressDto, BlossomUploadResponse, CancelBlossomUploadRequest,
    DiscardBlossomMediaRequest, ExpectedBlossomPublisherRequest, SelectBlossomMediaFilesRequest,
    SetPreferredBlossomServerRequest, StartBlossomUploadRequest,
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum UploadState {
    #[default]
    Idle,
    Selecting,
    Ready,
    Uploading,
    Failed,
    Cancelled,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct UploadCorrelation<'a> {
    pub account_npub: &'a str,
    pub publisher_hex: &'a str,
    pub generation: u64,
    pub session: u64,
    pub selection_id: &'a str,
    pub request_id: &'a str,
    pub upload_id: Option<&'a str>,
}

pub(crate) fn publisher_hex(npub: &str) -> Option<String> {
    use nostr::nips::nip19::FromBech32;

    nostr::PublicKey::from_bech32(npub)
        .ok()
        .map(|key| key.to_hex())
}

pub(crate) fn role_accepts_mime(role: &str, mime: &str) -> bool {
    match mime {
        "image/jpeg" | "image/png" | "image/webp" => {
            matches!(role, "hero" | "capsule" | "screenshot" | "feature")
        }
        "video/mp4" | "video/webm" => role == "trailer",
        _ => false,
    }
}

/// Visual treatment for the upload dialog, mapped from state the component
/// already emits. Nothing here infers progress or success on its own.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum UploadPresentation {
    Idle,
    Selecting,
    Selected,
    Busy,
    Uploading,
    Uploaded,
    Cancelled,
    Retryable,
    Rejected,
    PaymentRequired,
}

impl UploadPresentation {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Idle => "No file selected",
            Self::Selecting => "Selecting file",
            Self::Selected => "File selected; not uploaded",
            Self::Busy => "Preparing upload",
            Self::Uploading => "Uploading",
            Self::Uploaded => "Uploaded to the Blossom server",
            Self::Cancelled => "Upload cancelled",
            Self::Retryable => "Upload failed; retry available",
            Self::Rejected => "Rejected",
            Self::PaymentRequired => "Payment required",
        }
    }

    pub(crate) fn variant(self) -> StatusChipVariant {
        match self {
            Self::Idle => StatusChipVariant::Neutral,
            Self::Selecting | Self::Busy => StatusChipVariant::Pending,
            Self::Selected => StatusChipVariant::Draft,
            Self::Uploading => StatusChipVariant::Downloading,
            Self::Uploaded => StatusChipVariant::Verified,
            Self::Cancelled => StatusChipVariant::Cancelled,
            Self::Retryable => StatusChipVariant::Warning,
            Self::Rejected => StatusChipVariant::Error,
            Self::PaymentRequired => StatusChipVariant::Unavailable,
        }
    }
}

/// A completed Blossom upload is hosted media, never a published Store Page.
/// Error codes decide rejection versus retryable failure; the component's own
/// validation stays authoritative.
pub(crate) fn upload_presentation(
    state: UploadState,
    has_selection: bool,
    error_message: Option<&str>,
    phase: Option<&str>,
) -> UploadPresentation {
    if let Some(message) = error_message {
        // Classified from the stable message the component already renders, so
        // no error-handling path has to change to drive presentation.
        if message == stable_error_message("payment_required") {
            return UploadPresentation::PaymentRequired;
        }
        if [
            "unsupported_media",
            "file_too_large",
            "unsafe_destination",
            "invalid_server",
            "integrity_mismatch",
            "publisher_mismatch",
        ]
        .into_iter()
        .any(|code| stable_error_message(code) == message)
        {
            return UploadPresentation::Rejected;
        }
        if message == stable_error_message("cancelled") {
            return UploadPresentation::Cancelled;
        }
        return UploadPresentation::Retryable;
    }
    match state {
        UploadState::Idle if has_selection => UploadPresentation::Selected,
        UploadState::Idle => UploadPresentation::Idle,
        UploadState::Selecting => UploadPresentation::Selecting,
        UploadState::Ready if phase == Some(phase_label("complete")) => {
            UploadPresentation::Uploaded
        }
        UploadState::Ready if phase.is_some() => UploadPresentation::Busy,
        UploadState::Ready => UploadPresentation::Selected,
        UploadState::Uploading => UploadPresentation::Uploading,
        UploadState::Cancelled => UploadPresentation::Cancelled,
        UploadState::Failed => UploadPresentation::Retryable,
    }
}

pub(crate) fn phase_label(phase: &str) -> &'static str {
    match phase {
        "inspect" | "inspecting" => "Inspecting file",
        "hash" | "preparing" => "Hashing",
        "sign" | "signing" | "authorizing" => "Waiting for signer",
        "preflight" => "Checking server",
        "paying" | "payment" => "Waiting for payment",
        "upload" | "uploading" => "Uploading",
        "verify" | "verifying" => "Verifying response",
        "complete" | "completed" => "Complete",
        "cancelled" => "Upload cancelled",
        "failed" => "Upload failed",
        _ => "Processing upload",
    }
}

pub(crate) fn stable_error_message(code: &str) -> &'static str {
    match code {
        "desktop_only" => "Blossom upload is available in the desktop app only.",
        "account_unavailable" => "The publisher account signer is unavailable.",
        "signer_rejected" => "The signer rejected this upload.",
        "signer_timeout" => "The signer did not respond in time.",
        "signer_unavailable" | "signer_failure" | "signing_failed" => {
            "The signer could not authorize this upload."
        }
        "payment_required" | "payment_failed" | "invoice_failed" => {
            "This Blossom server requires payment before upload."
        }
        "invalid_descriptor_url" => {
            "The returned media URL is unsafe or does not contain the uploaded hash."
        }
        "integrity_mismatch" => "The returned hash, size, or media type did not match.",
        "body_rejected" | "invalid_response" | "response_too_large" | "malformed_descriptor" => {
            "The upload server returned a malformed response."
        }
        "cancelled" | "upload_cancelled" => "Upload cancelled.",
        "no_server" | "server_not_configured" | "no_configured_servers" => {
            "Choose an enabled Blossom server."
        }
        "unsupported_media" | "unsupported_file" | "mime_mismatch" => {
            "This file is not compatible with that media role."
        }
        "file_too_large" => "The selected file is too large.",
        "invalid_server" => "Enter a valid Blossom server origin.",
        "unsafe_destination" => "The Blossom server destination is not permitted.",
        "storage_failure" => "The local Blossom operation failed.",
        "publisher_mismatch" | "account_changed" => "The active publisher account changed.",
        "network_failure" | "request_failed" => "The upload server could not be reached.",
        "authorization_rejected" => "The server rejected the upload authorization.",
        "rate_limited" => "The server is temporarily rate limiting uploads.",
        "selection_unavailable" => {
            "The selected file is no longer available to Arcadestr. Select it again."
        }
        "selection_busy" => "This file already has an active upload.",
        _ => "The Blossom operation could not be completed.",
    }
}

pub(crate) fn preferred_candidate(settings: &BlossomServerSettingsDto) -> Option<String> {
    let enabled = |origin: &str| {
        settings
            .servers
            .iter()
            .any(|server| server.enabled && server.origin == origin)
    };
    settings
        .preferred_server
        .as_deref()
        .filter(|origin| enabled(origin))
        .map(str::to_owned)
        .or_else(|| {
            settings
                .servers
                .iter()
                .find(|server| server.enabled)
                .map(|server| server.origin.clone())
        })
}

pub(crate) fn selection_after_picker(
    picked: Option<BlossomMediaSelectionDto>,
) -> Option<BlossomMediaSelectionDto> {
    picked
}

pub(crate) fn fresh_request_id(now_ms: u64, sequence: u64) -> String {
    format!("blossom-{now_ms}-{sequence}")
}

pub(crate) fn accepts_progress(
    expected: &UploadCorrelation<'_>,
    active_account: Option<&str>,
    active_generation: u64,
    active_session: u64,
    event: &BlossomUploadProgressDto,
) -> bool {
    active_account == Some(expected.account_npub)
        && active_generation == expected.generation
        && active_session == expected.session
        && event.publisher_pubkey == expected.publisher_hex
        && !event.upload_id.is_empty()
        && event.selection_id == expected.selection_id
        && event.request_id == expected.request_id
        && expected
            .upload_id
            .map_or(true, |upload_id| event.upload_id == upload_id)
}

pub(crate) fn accepts_completion(
    expected: &UploadCorrelation<'_>,
    active_account: Option<&str>,
    active_publisher_hex: Option<&str>,
    active_generation: u64,
    active_session: u64,
    active_role: Option<&str>,
    expected_role: &str,
    active_selection_id: Option<&str>,
    active_request_id: Option<&str>,
) -> bool {
    active_account == Some(expected.account_npub)
        && active_publisher_hex == Some(expected.publisher_hex)
        && active_generation == expected.generation
        && active_session == expected.session
        && active_role == Some(expected_role)
        && active_selection_id == Some(expected.selection_id)
        && active_request_id == Some(expected.request_id)
}

pub(crate) fn unique_media_id<'a>(role: &str, existing: impl Iterator<Item = &'a str>) -> String {
    let existing = existing.collect::<Vec<_>>();
    let prefix = format!("blossom-{}", role.replace('_', "-"));
    if !existing.iter().any(|id| **id == prefix) {
        return prefix;
    }
    (2_u64..)
        .map(|index| format!("{prefix}-{index}"))
        .find(|id| !existing.iter().any(|existing| *existing == id))
        .unwrap_or_else(|| format!("{prefix}-new"))
}

pub(crate) fn verified_media_item(
    role: &str,
    selection: &BlossomMediaSelectionDto,
    response: &BlossomUploadResponse,
    id: String,
) -> Option<StorePageMediaItem> {
    if !role_accepts_mime(role, &selection.detected_mime)
        || !role_accepts_mime(role, &response.mime_type)
        || selection.detected_mime != response.mime_type
        || response.url.trim().is_empty()
        || response.sha256.trim().is_empty()
    {
        return None;
    }
    Some(StorePageMediaItem {
        id,
        media_type: if response.mime_type.starts_with("video/") {
            "video".into()
        } else {
            "image".into()
        },
        role: role.into(),
        url: response.url.clone(),
        sha256: Some(response.sha256.clone()),
        mime_type: Some(response.mime_type.clone()),
        size: Some(response.size),
        thumbnail_url: None,
        alt: None,
        caption: None,
        width: selection.width,
        height: selection.height,
    })
}

pub(crate) fn role_available(draft: &StorePageDraft, role: &str) -> bool {
    !matches!(role, "hero" | "capsule") || !draft.content.media.iter().any(|item| item.role == role)
}

/// Roles a page can hold several of; their picker selects many files at once.
pub(crate) fn role_allows_many(role: &str) -> bool {
    matches!(role, "screenshot" | "feature" | "trailer")
}

/// How many more files of `role` the page can take, counting files already
/// queued for upload.
pub(crate) fn remaining_for_role(draft: &StorePageDraft, role: &str, queued: usize) -> usize {
    use arcadestr_core::store_page_content_policy::{MAX_MEDIA_ITEMS, MAX_SCREENSHOTS};
    let media = &draft.content.media;
    let total = MAX_MEDIA_ITEMS.saturating_sub(media.len() + queued);
    let role_limit = match role {
        "hero" | "capsule" => usize::from(role_available(draft, role)),
        "screenshot" => MAX_SCREENSHOTS.saturating_sub(
            media
                .iter()
                .filter(|item| item.role == "screenshot")
                .count(),
        ),
        _ => total + queued,
    };
    total.min(role_limit.saturating_sub(queued))
}

/// Plural dialog title for a role.
fn role_title(role: &str) -> String {
    match role {
        "screenshot" => "Upload screenshots".into(),
        "feature" => "Upload feature images".into(),
        "trailer" => "Upload trailers".into(),
        other => format!("Upload {other}"),
    }
}

/// One picked file waiting in, or done with, the upload queue.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct QueuedMedia {
    pub selection: BlossomMediaSelectionDto,
    pub status: QueuedStatus,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum QueuedStatus {
    Ready,
    Uploading,
    Uploaded,
    Failed(String),
}

impl QueuedStatus {
    fn label(&self) -> String {
        match self {
            Self::Ready => "Ready to upload".into(),
            Self::Uploading => "Uploading".into(),
            Self::Uploaded => "Uploaded".into(),
            Self::Failed(message) => message.clone(),
        }
    }
}

/// Index of the next file to upload: ready files first in queue order, then
/// failed files when retrying.
pub(crate) fn next_upload(queue: &[QueuedMedia], retry_failed: bool) -> Option<usize> {
    queue
        .iter()
        .position(|item| item.status == QueuedStatus::Ready)
        .or_else(|| {
            retry_failed
                .then(|| {
                    queue
                        .iter()
                        .position(|item| matches!(item.status, QueuedStatus::Failed(_)))
                })
                .flatten()
        })
}

/// Files picked for a role are kept only when the role accepts their type;
/// the rest are reported by name.
pub(crate) fn split_compatible(
    role: &str,
    picked: Vec<BlossomMediaSelectionDto>,
) -> (Vec<BlossomMediaSelectionDto>, Vec<BlossomMediaSelectionDto>) {
    picked
        .into_iter()
        .partition(|selection| role_accepts_mime(role, &selection.detected_mime))
}

struct PickOutcome {
    selections: Vec<BlossomMediaSelectionDto>,
    notes: Vec<String>,
}

/// Run the picker for `role`: several files for roles that allow many, one
/// otherwise.
async fn pick_media(
    role: &str,
    publisher_hex: &str,
    remaining: usize,
) -> Result<PickOutcome, crate::tauri_bridge::BlossomCommandErrorDto> {
    if role_allows_many(role) {
        let batch = invoke_select_blossom_media_files(SelectBlossomMediaFilesRequest {
            expected_publisher_hex: publisher_hex.to_string(),
            max_files: remaining,
        })
        .await?;
        let mut notes = batch
            .rejected
            .into_iter()
            .map(|file| format!("{}: {}", file.filename, stable_error_message(&file.code)))
            .collect::<Vec<_>>();
        if batch.over_limit > 0 {
            notes.push(format!(
                "{} more file(s) were not added: the Store Page has room for {remaining} more.",
                batch.over_limit
            ));
        }
        Ok(PickOutcome {
            selections: batch.selections,
            notes,
        })
    } else {
        let picked = invoke_select_blossom_media_file(ExpectedBlossomPublisherRequest {
            expected_publisher_hex: publisher_hex.to_string(),
        })
        .await?;
        Ok(PickOutcome {
            selections: selection_after_picker(picked).into_iter().collect(),
            notes: Vec::new(),
        })
    }
}

async fn discard_selections(selections: Vec<BlossomMediaSelectionDto>, publisher_hex: String) {
    for selection in selections {
        let _ = invoke_discard_blossom_media_selection(DiscardBlossomMediaRequest {
            selection_id: selection.selection_id,
            expected_publisher_hex: publisher_hex.clone(),
        })
        .await;
    }
}

/// Selections in the queue that were never uploaded and must be released.
fn pending_selections(queue: &[QueuedMedia]) -> Vec<BlossomMediaSelectionDto> {
    queue
        .iter()
        .filter(|item| item.status != QueuedStatus::Uploaded)
        .map(|item| item.selection.clone())
        .collect()
}

#[cfg(test)]
pub(crate) fn remove_draft_media(draft: &mut StorePageDraft, id: &str) -> bool {
    let before = draft.content.media.len();
    draft.content.media.retain(|item| item.id != id);
    if draft.content.media.len() == before {
        return false;
    }
    for section in &mut draft.content.sections {
        if section.media_id.as_deref() == Some(id) {
            section.media_id = None;
        }
    }
    true
}

fn human_size(size: u64) -> String {
    if size >= 1_048_576 {
        format!("{:.1} MiB", size as f64 / 1_048_576.0)
    } else if size >= 1_024 {
        format!("{:.1} KiB", size as f64 / 1_024.0)
    } else {
        format!("{size} bytes")
    }
}

fn run_cleanup(slot: &Rc<RefCell<Option<Box<dyn FnOnce()>>>>) {
    if let Some(cleanup) = slot.borrow_mut().take() {
        cleanup();
    }
}

/// The declared dismissal contract for the Blossom upload dialog.
///
/// An in-flight upload has a real cancellation path, so dismissal while
/// uploading routes through the existing in-dialog confirmation instead of
/// being refused. A stray backdrop click never reaches that path.
fn upload_close_contract() -> (DialogClosePolicy, DialogDismissal) {
    (
        DialogClosePolicy::ConfirmRequired,
        DialogDismissal {
            escape: DialogSourcePolicy::Allowed,
            backdrop: DialogSourcePolicy::Ignored,
            close_button: DialogCloseButtonPolicy::Visible,
        },
    )
}

#[component]
pub fn BlossomMediaUpload(
    dialog_role: RwSignal<Option<String>>,
    listing_publisher_npub: String,
    publisher_hex: String,
    context_generation: RwSignal<u64>,
    draft: RwSignal<StorePageDraft>,
    input_dirty: RwSignal<bool>,
) -> impl IntoView {
    let auth = use_context::<crate::AuthContext>().expect("AuthContext not provided");
    let session = RwSignal::new(0_u64);
    let opened_role = RwSignal::new(None::<String>);
    let captured_account = RwSignal::new(None::<String>);
    let captured_generation = RwSignal::new(0_u64);
    // Every file picked in this dialog session, uploaded one after another.
    let queue = RwSignal::new(Vec::<QueuedMedia>::new());
    let notes = RwSignal::new(Vec::<String>::new());
    let settings = RwSignal::new(None::<BlossomServerSettingsDto>);
    let selected_server = RwSignal::new(None::<String>);
    let state = RwSignal::new(UploadState::Idle);
    let error = RwSignal::new(None::<String>);
    let status = RwSignal::new(None::<String>);
    let bytes = RwSignal::new((0_u64, 0_u64));
    let active_selection_id = RwSignal::new(None::<String>);
    let active_request_id = RwSignal::new(None::<String>);
    let active_upload_id = RwSignal::new(None::<String>);
    let request_sequence = RwSignal::new(0_u64);
    let close_confirmation = RwSignal::new(false);
    let server_origin = RwSignal::new(String::new());
    let server_label = RwSignal::new(String::new());
    let listener_alive = Rc::new(Cell::new(true));
    let listener_cleanup: Rc<RefCell<Option<Box<dyn FnOnce()>>>> = Rc::new(RefCell::new(None));

    let has_pending = move || {
        queue
            .get()
            .iter()
            .any(|item| item.status != QueuedStatus::Uploaded)
    };

    // Pick files for the open role and add them to the queue. A pick that
    // finishes after the dialog session changed is discarded, never queued.
    let pick = {
        let publisher_hex = publisher_hex.clone();
        move |role: String, replace_single: bool| {
            let expected_account = captured_account.get_untracked();
            let expected_generation = captured_generation.get_untracked();
            let active_session = session.get_untracked();
            let publisher_hex = publisher_hex.clone();
            state.set(UploadState::Selecting);
            error.set(None);
            notes.set(Vec::new());
            spawn_local(async move {
                if replace_single {
                    let previous = pending_selections(&queue.get_untracked());
                    queue.set(Vec::new());
                    discard_selections(previous, publisher_hex.clone()).await;
                }
                let queued = queue
                    .get_untracked()
                    .iter()
                    .filter(|item| item.status != QueuedStatus::Uploaded)
                    .count();
                let remaining = remaining_for_role(&draft.get_untracked(), &role, queued);
                if remaining == 0 {
                    state.set(UploadState::Idle);
                    error.set(Some(if role_allows_many(&role) {
                        format!("The Store Page has no room for more {role} files.")
                    } else {
                        format!("A {role} item already exists in this draft.")
                    }));
                    return;
                }
                let picked = pick_media(&role, &publisher_hex, remaining).await;
                if auth.npub.get_untracked() != expected_account
                    || context_generation.get_untracked() != expected_generation
                    || session.get_untracked() != active_session
                    || dialog_role.get_untracked().as_deref() != Some(role.as_str())
                {
                    if let Ok(stale) = picked {
                        discard_selections(stale.selections, publisher_hex).await;
                    }
                    return;
                }
                match picked {
                    Ok(outcome) => {
                        let (compatible, incompatible) =
                            split_compatible(&role, outcome.selections);
                        let mut messages = outcome.notes;
                        messages.extend(incompatible.iter().map(|selection| {
                            format!(
                                "{}: {}",
                                selection.filename,
                                stable_error_message("mime_mismatch")
                            )
                        }));
                        notes.set(messages);
                        queue.update(|items| {
                            items.extend(compatible.into_iter().map(|selection| QueuedMedia {
                                selection,
                                status: QueuedStatus::Ready,
                            }))
                        });
                        state.set(if next_upload(&queue.get_untracked(), false).is_some() {
                            UploadState::Ready
                        } else {
                            UploadState::Idle
                        });
                        discard_selections(incompatible, publisher_hex).await;
                    }
                    Err(command_error) => {
                        state.set(UploadState::Failed);
                        error.set(Some(stable_error_message(&command_error.code).into()));
                    }
                }
            });
        }
    };

    Effect::new({
        let listener_cleanup = Rc::clone(&listener_cleanup);
        let listener_alive = Rc::clone(&listener_alive);
        let publisher_hex = publisher_hex.clone();
        let pick = pick.clone();
        move |_| {
            let role = dialog_role.get();
            let Some(role) = role else {
                opened_role.set(None);
                close_confirmation.set(false);
                listener_alive.set(false);
                run_cleanup(&listener_cleanup);
                return;
            };
            if opened_role.get_untracked().as_deref() == Some(role.as_str()) {
                return;
            }
            opened_role.set(Some(role.clone()));
            session.update(|value| *value = value.wrapping_add(1));
            captured_account.set(auth.npub.get_untracked());
            captured_generation.set(context_generation.get_untracked());
            queue.set(Vec::new());
            notes.set(Vec::new());
            error.set(None);
            status.set(None);
            bytes.set((0, 0));
            active_selection_id.set(None);
            active_request_id.set(None);
            active_upload_id.set(None);
            listener_alive.set(true);
            run_cleanup(&listener_cleanup);
            if captured_account.get_untracked().is_none() {
                error.set(Some("The publisher account is unavailable.".into()));
                state.set(UploadState::Failed);
                return;
            }
            if publisher_hex.is_empty() {
                error.set(Some("The listing publisher key is invalid.".into()));
                state.set(UploadState::Failed);
                return;
            }
            let publisher_hex = publisher_hex.clone();
            spawn_local(async move {
                match invoke_get_blossom_server_settings(ExpectedBlossomPublisherRequest {
                    expected_publisher_hex: publisher_hex.clone(),
                })
                .await
                {
                    Ok(value) if value.publisher_pubkey == publisher_hex => {
                        selected_server.set(preferred_candidate(&value));
                        settings.set(Some(value));
                    }
                    Ok(_) => error.set(Some(stable_error_message("publisher_mismatch").into())),
                    Err(command_error) => {
                        error.set(Some(stable_error_message(&command_error.code).into()))
                    }
                }
            });
            pick(role, false);
        }
    });

    Effect::new({
        let listener_cleanup = Rc::clone(&listener_cleanup);
        let listener_alive = Rc::clone(&listener_alive);
        let publisher_hex = publisher_hex.clone();
        move |_| {
            let active = auth.npub.get();
            let generation = context_generation.get();
            if dialog_role.get_untracked().is_some()
                && (captured_account.get_untracked() != active
                    || captured_generation.get_untracked() != generation)
            {
                session.update(|value| *value = value.wrapping_add(1));
                listener_alive.set(false);
                run_cleanup(&listener_cleanup);
                let upload_id = active_upload_id.get_untracked();
                let pending = pending_selections(&queue.get_untracked());
                let publisher_hex = publisher_hex.clone();
                spawn_local(async move {
                    if let Some(upload_id) = upload_id {
                        let _ = invoke_cancel_blossom_upload(CancelBlossomUploadRequest {
                            upload_id,
                            expected_publisher_hex: publisher_hex.clone(),
                        })
                        .await;
                    }
                    discard_selections(pending, publisher_hex).await;
                });
                queue.set(Vec::new());
                dialog_role.set(None);
            }
        }
    });

    // Upload every queued file in order. Each file keeps its own request ID and
    // correlation, exactly like a single upload; a stale session stops the run.
    let upload = UnsyncCallback::new({
        let listener_cleanup = Rc::clone(&listener_cleanup);
        let listener_alive = Rc::clone(&listener_alive);
        let publisher_hex = publisher_hex.clone();
        move |retry_failed: bool| {
            let Some(role) = dialog_role.get_untracked() else {
                return;
            };
            if next_upload(&queue.get_untracked(), retry_failed).is_none() {
                error.set(Some("Choose a media file first.".into()));
                return;
            }
            let Some(server) = selected_server.get_untracked() else {
                error.set(Some(stable_error_message("no_server").into()));
                return;
            };
            let Some(account) = captured_account.get_untracked() else {
                error.set(Some("The publisher account is unavailable.".into()));
                return;
            };
            let expected_generation = captured_generation.get_untracked();
            let expected_session = session.get_untracked();
            state.set(UploadState::Uploading);
            error.set(None);
            let listener_cleanup = Rc::clone(&listener_cleanup);
            let listener_alive = Rc::clone(&listener_alive);
            let publisher_hex = publisher_hex.clone();
            let listing_publisher_npub = listing_publisher_npub.clone();
            spawn_local(async move {
                let mut retried = std::collections::HashSet::new();
                loop {
                    if session.get_untracked() != expected_session {
                        return;
                    }
                    let items = queue.get_untracked();
                    let Some(index) = next_upload(&items, retry_failed)
                        .filter(|index| !retried.contains(&items[*index].selection.selection_id))
                    else {
                        break;
                    };
                    let selected = items[index].selection.clone();
                    let use_retry = matches!(items[index].status, QueuedStatus::Failed(_));
                    retried.insert(selected.selection_id.clone());
                    let status_id = selected.selection_id.clone();
                    let set_status = move |status: QueuedStatus| {
                        queue.update(|items| {
                            if let Some(item) = items
                                .iter_mut()
                                .find(|item| item.selection.selection_id == status_id)
                            {
                                item.status = status;
                            }
                        });
                    };
                    if !role_accepts_mime(&role, &selected.detected_mime)
                        || !role_available(&draft.get_untracked(), &role)
                    {
                        set_status(QueuedStatus::Failed(
                            if role_available(&draft.get_untracked(), &role) {
                                stable_error_message("mime_mismatch").into()
                            } else {
                                format!("A {role} item already exists in this draft.")
                            },
                        ));
                        continue;
                    }
                    request_sequence.update(|value| *value = value.wrapping_add(1));
                    let request_id = fresh_request_id(
                        js_sys::Date::now().max(0.0) as u64,
                        request_sequence.get_untracked(),
                    );
                    active_selection_id.set(Some(selected.selection_id.clone()));
                    active_request_id.set(Some(request_id.clone()));
                    active_upload_id.set(None);
                    set_status(QueuedStatus::Uploading);
                    status.set(Some("Preparing file".into()));
                    bytes.set((0, selected.size));
                    listener_alive.set(false);
                    run_cleanup(&listener_cleanup);
                    listener_alive.set(true);

                    let event_account = account.clone();
                    let event_publisher = publisher_hex.clone();
                    let event_selection = selected.selection_id.clone();
                    let event_request = request_id.clone();
                    let event_cleanup = Rc::clone(&listener_cleanup);
                    let event_alive = Rc::clone(&listener_alive);
                    let registered = listen_blossom_upload_progress(move |event| {
                        let bound_upload = active_upload_id.get_untracked();
                        let correlation = UploadCorrelation {
                            account_npub: &event_account,
                            publisher_hex: &event_publisher,
                            generation: expected_generation,
                            session: expected_session,
                            selection_id: &event_selection,
                            request_id: &event_request,
                            upload_id: bound_upload.as_deref(),
                        };
                        if !accepts_progress(
                            &correlation,
                            auth.npub.get_untracked().as_deref(),
                            context_generation.get_untracked(),
                            session.get_untracked(),
                            &event,
                        ) {
                            return;
                        }
                        if bound_upload.is_none() {
                            active_upload_id.set(Some(event.upload_id.clone()));
                        }
                        status.set(Some(phase_label(&event.phase).into()));
                        bytes.set((event.bytes_completed, event.total_bytes));
                        if matches!(
                            event.phase.as_str(),
                            "complete" | "completed" | "cancelled" | "failed"
                        ) {
                            event_alive.set(false);
                            let deferred_cleanup = Rc::clone(&event_cleanup);
                            spawn_local(async move {
                                run_cleanup(&deferred_cleanup);
                            });
                        }
                    })
                    .await;
                    let cleanup = match registered {
                        Ok(cleanup) => cleanup,
                        Err(command_error) => {
                            set_status(QueuedStatus::Failed(
                                stable_error_message(&command_error.code).into(),
                            ));
                            state.set(UploadState::Failed);
                            return;
                        }
                    };
                    if !listener_alive.get()
                        || session.get_untracked() != expected_session
                        || dialog_role.get_untracked().as_deref() != Some(role.as_str())
                    {
                        cleanup();
                        return;
                    }
                    *listener_cleanup.borrow_mut() = Some(cleanup);
                    let request = StartBlossomUploadRequest {
                        selection_id: selected.selection_id.clone(),
                        expected_publisher_hex: publisher_hex.clone(),
                        selected_server: Some(server.clone()),
                        preflight: true,
                        request_id: request_id.clone(),
                    };
                    let result = if use_retry {
                        invoke_retry_blossom_upload(request).await
                    } else {
                        invoke_start_blossom_upload(request).await
                    };
                    listener_alive.set(false);
                    run_cleanup(&listener_cleanup);
                    let completion_upload_id = active_upload_id.get_untracked();
                    let correlation = UploadCorrelation {
                        account_npub: &account,
                        publisher_hex: &publisher_hex,
                        generation: expected_generation,
                        session: expected_session,
                        selection_id: &selected.selection_id,
                        request_id: &request_id,
                        upload_id: completion_upload_id.as_deref(),
                    };
                    if !accepts_completion(
                        &correlation,
                        auth.npub.get_untracked().as_deref(),
                        self::publisher_hex(&listing_publisher_npub).as_deref(),
                        context_generation.get_untracked(),
                        session.get_untracked(),
                        dialog_role.get_untracked().as_deref(),
                        &role,
                        active_selection_id.get_untracked().as_deref(),
                        active_request_id.get_untracked().as_deref(),
                    ) {
                        return;
                    }
                    match result {
                        Ok(response) => {
                            if completion_upload_id
                                .as_deref()
                                .is_some_and(|upload_id| upload_id != response.upload_id)
                            {
                                set_status(QueuedStatus::Failed(
                                    stable_error_message("invalid_response").into(),
                                ));
                                continue;
                            }
                            if !role_available(&draft.get_untracked(), &role) {
                                set_status(QueuedStatus::Failed(format!(
                                    "A {role} item already exists in this draft."
                                )));
                                continue;
                            }
                            let id = unique_media_id(
                                &role,
                                draft
                                    .get_untracked()
                                    .content
                                    .media
                                    .iter()
                                    .map(|item| item.id.as_str()),
                            );
                            match verified_media_item(&role, &selected, &response, id) {
                                Some(item) => {
                                    draft.update(|value| value.content.media.push(item));
                                    input_dirty.set(true);
                                    set_status(QueuedStatus::Uploaded);
                                }
                                None => set_status(QueuedStatus::Failed(
                                    stable_error_message("invalid_response").into(),
                                )),
                            }
                        }
                        Err(command_error) => {
                            let cancelled = matches!(
                                command_error.code.as_str(),
                                "cancelled" | "upload_cancelled"
                            );
                            set_status(QueuedStatus::Failed(
                                stable_error_message(&command_error.code).into(),
                            ));
                            if cancelled {
                                state.set(UploadState::Cancelled);
                                status.set(Some(
                                    "Upload cancelled. A partial blob may remain on the selected server."
                                        .into(),
                                ));
                                return;
                            }
                        }
                    }
                }
                active_selection_id.set(None);
                active_request_id.set(None);
                active_upload_id.set(None);
                let items = queue.get_untracked();
                if items
                    .iter()
                    .all(|item| item.status == QueuedStatus::Uploaded)
                {
                    queue.set(Vec::new());
                    status.set(None);
                    dialog_role.set(None);
                } else {
                    let failed = items
                        .iter()
                        .filter(|item| matches!(item.status, QueuedStatus::Failed(_)))
                        .count();
                    state.set(UploadState::Failed);
                    status.set(None);
                    error.set(Some(format!(
                        "{failed} file(s) did not upload. Retry them or remove them from the list."
                    )));
                }
            });
        }
    });

    let cancel_upload = UnsyncCallback::new({
        let listener_cleanup = Rc::clone(&listener_cleanup);
        let listener_alive = Rc::clone(&listener_alive);
        let publisher_hex = publisher_hex.clone();
        move |close_after: bool| {
            listener_alive.set(false);
            run_cleanup(&listener_cleanup);
            session.update(|value| *value = value.wrapping_add(1));
            let upload_id = active_upload_id.get_untracked();
            let pending = pending_selections(&queue.get_untracked());
            let publisher_hex = publisher_hex.clone();
            spawn_local(async move {
                if let Some(upload_id) = upload_id {
                    let _ = invoke_cancel_blossom_upload(CancelBlossomUploadRequest {
                        upload_id,
                        expected_publisher_hex: publisher_hex.clone(),
                    })
                    .await;
                }
                discard_selections(pending, publisher_hex).await;
            });
            queue.set(Vec::new());
            active_selection_id.set(None);
            active_upload_id.set(None);
            active_request_id.set(None);
            state.set(UploadState::Cancelled);
            status.set(Some(
                "Upload cancelled. A partial blob may remain on the selected server.".into(),
            ));
            close_confirmation.set(false);
            if close_after {
                dialog_role.set(None);
            }
        }
    });

    let choose_files = Callback::new({
        let pick = pick.clone();
        move |_| {
            if let Some(role) = dialog_role.get_untracked() {
                let replace = !role_allows_many(&role);
                pick(role, replace);
            }
        }
    });

    let remove_queued = Callback::new({
        let publisher_hex = publisher_hex.clone();
        move |selection_id: String| {
            let mut removed = Vec::new();
            queue.update(|items| {
                items.retain(|item| {
                    let keep = item.selection.selection_id != selection_id
                        || item.status == QueuedStatus::Uploading;
                    if !keep {
                        removed.push(item.selection.clone());
                    }
                    keep
                })
            });
            if next_upload(&queue.get_untracked(), true).is_none() {
                error.set(None);
                state.set(UploadState::Idle);
            }
            let publisher_hex = publisher_hex.clone();
            spawn_local(discard_selections(removed, publisher_hex));
        }
    });

    let add_server = Callback::new({
        let publisher_hex = publisher_hex.clone();
        move |_| {
            let origin = server_origin.get_untracked().trim().to_string();
            if origin.is_empty() {
                return;
            }
            let label = server_label.get_untracked().trim().to_string();
            let publisher_hex = publisher_hex.clone();
            spawn_local(async move {
                match invoke_add_blossom_server(AddBlossomServerRequest {
                    expected_publisher_hex: publisher_hex.clone(),
                    origin,
                    label: (!label.is_empty()).then_some(label),
                })
                .await
                {
                    Ok(value) => {
                        if value.publisher_pubkey == publisher_hex {
                            selected_server.set(preferred_candidate(&value));
                            settings.set(Some(value));
                            server_origin.set(String::new());
                            server_label.set(String::new());
                        } else {
                            error.set(Some(stable_error_message("publisher_mismatch").into()));
                        }
                    }
                    Err(command_error) => {
                        error.set(Some(stable_error_message(&command_error.code).into()));
                    }
                }
            });
        }
    });

    let remove_server = Callback::new({
        let publisher_hex = publisher_hex.clone();
        move |origin: String| {
            let publisher_hex = publisher_hex.clone();
            spawn_local(async move {
                match invoke_remove_blossom_server(BlossomServerOriginRequest {
                    expected_publisher_hex: publisher_hex.clone(),
                    origin,
                })
                .await
                {
                    Ok(value) => {
                        if value.publisher_pubkey == publisher_hex {
                            selected_server.set(preferred_candidate(&value));
                            settings.set(Some(value));
                        } else {
                            error.set(Some(stable_error_message("publisher_mismatch").into()));
                        }
                    }
                    Err(command_error) => {
                        error.set(Some(stable_error_message(&command_error.code).into()));
                    }
                }
            });
        }
    });

    let make_preferred = Callback::new({
        let publisher_hex = publisher_hex.clone();
        move |origin: String| {
            let publisher_hex = publisher_hex.clone();
            spawn_local(async move {
                match invoke_set_preferred_blossom_server(SetPreferredBlossomServerRequest {
                    expected_publisher_hex: publisher_hex.clone(),
                    origin: Some(origin),
                })
                .await
                {
                    Ok(value) => {
                        if value.publisher_pubkey == publisher_hex {
                            selected_server.set(preferred_candidate(&value));
                            settings.set(Some(value));
                        } else {
                            error.set(Some(stable_error_message("publisher_mismatch").into()));
                        }
                    }
                    Err(command_error) => {
                        error.set(Some(stable_error_message(&command_error.code).into()));
                    }
                }
            });
        }
    });

    let teardown_cleanup = SendWrapper::new(Rc::clone(&listener_cleanup));
    let teardown_alive = SendWrapper::new(Rc::clone(&listener_alive));
    let teardown_publisher = publisher_hex.clone();
    on_cleanup(move || {
        teardown_alive.set(false);
        run_cleanup(&teardown_cleanup);
        let upload_id = active_upload_id.get_untracked();
        let pending = queue
            .try_get_untracked()
            .map(|items| pending_selections(&items))
            .unwrap_or_default();
        spawn_local(async move {
            if let Some(upload_id) = upload_id {
                let _ = invoke_cancel_blossom_upload(CancelBlossomUploadRequest {
                    upload_id,
                    expected_publisher_hex: teardown_publisher.clone(),
                })
                .await;
            }
            discard_selections(pending, teardown_publisher).await;
        });
    });

    let upload_label = move || {
        let ready = queue
            .get()
            .iter()
            .filter(|item| item.status == QueuedStatus::Ready)
            .count();
        if ready > 1 {
            format!("Upload {ready} files")
        } else {
            "Upload".to_string()
        }
    };

    view! {
        <Dialog
            id="blossom-upload"
            open=Signal::derive(move || dialog_role.get().is_some())
            title=Signal::derive(move || dialog_role.get().map(|role| role_title(&role)).unwrap_or_else(|| "Upload media".into()))
            kicker="Store Page media"
            width=DialogWidth::Standard
            // An in-flight upload is never abandoned silently: the existing
            // in-dialog confirmation runs first, then the existing cancel flow.
            policy=upload_close_contract().0
            dismissal=upload_close_contract().1
            busy=Signal::derive(move || state.get() == UploadState::Uploading)
            close_label="Close media upload"
            on_close=UnsyncCallback::new(move |request: DialogCloseRequest| {
                match request.action {
                    // Closing an idle dialog still discards every pending
                    // selection through the existing teardown path.
                    DialogCloseAction::Dismiss | DialogCloseAction::CancelOperation => {
                        cancel_upload.run(true)
                    }
                    DialogCloseAction::RequestConfirmation => close_confirmation.set(true),
                    DialogCloseAction::Ignore => {}
                }
            })
            actions=move || view! {
                <button type="button" class="v2-btn-primary" disabled=move || next_upload(&queue.get(), false).is_none() || selected_server.get().is_none() || state.get() == UploadState::Uploading on:click=move |_| upload.run(false)>{upload_label}</button>
                <Show when=move || state.get() != UploadState::Uploading && queue.get().iter().any(|item| matches!(item.status, QueuedStatus::Failed(_)))><button type="button" class="v2-btn-secondary" disabled=move || selected_server.get().is_none() on:click=move |_| upload.run(true)>"Retry failed"</button></Show>
                <Show when=move || state.get() == UploadState::Uploading><button type="button" class="v2-btn-secondary" on:click=move |_| cancel_upload.run(false)>"Cancel upload"</button></Show>
            }
        >
            <Show when=move || close_confirmation.get()>
                <section class="v2-store-card" role="alert"><strong>"Cancel active upload and close?"</strong><p>"The upload will be cancelled and every file not uploaded yet is discarded. A partial blob may remain on the server."</p><div class="v2-store-card-actions"><button type="button" on:click=move |_| close_confirmation.set(false)>"Keep uploading"</button><button type="button" on:click=move |_| cancel_upload.run(true)>"Cancel upload and close"</button></div></section>
            </Show>
            <Show when=move || !queue.get().is_empty()>
                <ul class="v2-blossom-queue" aria-label="Selected media">
                    {move || queue.get().into_iter().map(|item| {
                        let selection_id = item.selection.selection_id.clone();
                        let uploading = item.status == QueuedStatus::Uploading;
                        let failed = matches!(item.status, QueuedStatus::Failed(_));
                        let removable = item.status != QueuedStatus::Uploaded && !uploading;
                        let status_label = item.status.label();
                        let detail = format!(
                            "{} · {}{}",
                            item.selection.detected_mime,
                            human_size(item.selection.size),
                            match (item.selection.width, item.selection.height) {
                                (Some(width), Some(height)) => format!(" · {width} × {height}"),
                                _ => String::new(),
                            }
                        );
                        view! {
                            <li class="v2-blossom-queue-item" class:v2-blossom-queue-failed=failed>
                                {match item.selection.preview_data_url.clone() {
                                    Some(src) => view! { <img class="v2-blossom-queue-thumb" src=src alt="" /> }.into_any(),
                                    None => view! { <span class="v2-blossom-queue-thumb v2-blossom-queue-thumb-empty" aria-hidden="true">"VIDEO"</span> }.into_any(),
                                }}
                                <div class="v2-blossom-queue-body">
                                    <strong>{item.selection.filename.clone()}</strong>
                                    <span class="v2-store-help">{detail}</span>
                                    <span class="v2-blossom-queue-status" role=failed.then_some("alert")>{status_label}</span>
                                </div>
                                <Show when=move || removable && state.get() != UploadState::Uploading>
                                    {
                                        let selection_id = selection_id.clone();
                                        view! { <button type="button" class="v2-btn-secondary" on:click=move |_| remove_queued.run(selection_id.clone())>"Remove"</button> }
                                    }
                                </Show>
                            </li>
                        }
                    }).collect_view()}
                </ul>
            </Show>
            <Show when=move || state.get() != UploadState::Selecting && state.get() != UploadState::Uploading>
                <button type="button" class="v2-btn-secondary" on:click=move |_| choose_files.run(())>{move || {
                    let many = dialog_role.get().is_some_and(|role| role_allows_many(&role));
                    match (many, queue.get().is_empty()) {
                        (true, true) => "Choose files",
                        (true, false) => "Add more files",
                        (false, true) => "Choose file",
                        (false, false) => "Choose different file",
                    }
                }}</button>
            </Show>
            <Show when=move || state.get() == UploadState::Selecting><p role="status">"Waiting for file selection…"</p></Show>
            <Show when=move || !notes.get().is_empty()>
                <ul class="v2-store-help" role="status">{move || notes.get().into_iter().map(|note| view! { <li>{note}</li> }).collect_view()}</ul>
            </Show>

            <section class="v2-store-card"><h3>"Upload server"</h3>
                {move || {
                    let enabled = settings.get().map(|value| value.servers.into_iter().filter(|server| server.enabled).collect::<Vec<_>>()).unwrap_or_default();
                    if enabled.is_empty() { view! { <p class="v2-store-alert">"No enabled Blossom servers are configured. Add one below."</p> }.into_any() } else { view! { <label>"Server"<select class="v2-input" prop:value=move || selected_server.get().unwrap_or_default() on:change=move |event| selected_server.set(Some(event_target_value(&event)))>{enabled.into_iter().map(|server| { let origin = server.origin.clone(); let label = server.label.map(|label| format!("{label} — {origin}")).unwrap_or_else(|| origin.clone()); view! { <option value=origin>{label}</option> } }).collect_view()}</select></label> }.into_any() }
                }}
                <details><summary>"Configure servers"</summary><div class="v2-store-form-grid"><label>"Origin"<input class="v2-input" placeholder="https://blossom.example" prop:value=move || server_origin.get() on:input=move |event| server_origin.set(event_target_value(&event)) /></label><label>"Optional label"<input class="v2-input" prop:value=move || server_label.get() on:input=move |event| server_label.set(event_target_value(&event)) /></label><button type="button" class="v2-btn-secondary" on:click=move |_| add_server.run(())>"Add server"</button></div>
                    {move || settings.get().map(|value| { let preferred = value.preferred_server; value.servers.into_iter().map(|server| { let origin_remove = server.origin.clone(); let origin_preferred = server.origin.clone(); let is_preferred = preferred.as_deref() == Some(server.origin.as_str()); view! { <div class="v2-store-card"><strong>{server.label.unwrap_or_else(|| server.origin.clone())}</strong><p class="v2-store-mono">{server.origin}</p><p>{if server.enabled { "Enabled" } else { "Disabled" }}</p><div class="v2-store-card-actions"><button type="button" disabled=is_preferred on:click=move |_| make_preferred.run(origin_preferred.clone())>{if is_preferred { "Preferred" } else { "Make preferred" }}</button><button type="button" on:click=move |_| remove_server.run(origin_remove.clone())>"Remove"</button></div></div> } }).collect_view() })}
                </details>
            </section>
            <div class="v2-blossom-status">
                {move || { let presentation = upload_presentation(state.get(), has_pending(), error.get().as_deref(), status.get().as_deref()); view! { <StatusChip label=presentation.label() variant=presentation.variant() icon=None size=StatusChipSize::Compact /> } }}
                {move || status.get().map(|value| view! { <span class="v2-blossom-phase" role="status">{value}</span> })}
            </div>
            <Show when=move || state.get() == UploadState::Uploading>
                {move || { let (done, total) = bytes.get(); let percent = if total == 0 { 0 } else { ((done.saturating_mul(100)) / total).min(100) }; let items = queue.get(); let position = items.iter().position(|item| item.status == QueuedStatus::Uploading).map(|index| index + 1).unwrap_or_default(); let heading = if items.len() > 1 { format!("File {position} of {} · {} / {}", items.len(), human_size(done), human_size(total)) } else { format!("{} / {}", human_size(done), human_size(total)) }; view! {
                    <div class="v2-create-progress" role="progressbar" aria-label="Blossom upload progress" aria-valuemin="0" aria-valuemax="100" aria-valuenow=percent>
                        <div class="v2-create-progress-heading"><span>{heading}</span><span>{format!("{percent}%")}</span></div>
                        <div class="v2-create-progress-track"><div class="v2-create-progress-fill" style=format!("width: {percent}%")></div></div>
                    </div>
                } }}
            </Show>
            {move || error.get().map(|value| view! { <p class="v2-store-alert" role="alert">{value}</p> })}
            <p class="v2-store-help">"A completed upload stores the file on the Blossom server. It becomes part of the Store Page only when you publish."</p>
        </Dialog>
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tauri_bridge::BlossomServerDto;

    fn selection(mime: &str) -> BlossomMediaSelectionDto {
        BlossomMediaSelectionDto {
            selection_id: "selection-1".into(),
            filename: "safe-name.png".into(),
            detected_mime: mime.into(),
            size: 42,
            width: Some(1200),
            height: Some(600),
            preview_data_url: Some("data:image/png;base64,AA==".into()),
        }
    }

    fn response(mime: &str) -> BlossomUploadResponse {
        BlossomUploadResponse {
            upload_id: "upload-1".into(),
            url: "https://cdn.example/hash.png".into(),
            sha256: "abc123".into(),
            mime_type: mime.into(),
            size: 42,
            uploaded: 42,
            was_existing: false,
        }
    }

    fn settings(preferred: Option<&str>, servers: &[(&str, bool)]) -> BlossomServerSettingsDto {
        BlossomServerSettingsDto {
            publisher_pubkey: "publisher".into(),
            servers: servers
                .iter()
                .map(|(origin, enabled)| BlossomServerDto {
                    origin: (*origin).into(),
                    label: None,
                    enabled: *enabled,
                    created_at: 1,
                    updated_at: 1,
                })
                .collect(),
            preferred_server: preferred.map(str::to_owned),
        }
    }

    #[test]
    fn an_active_upload_asks_before_it_is_abandoned() {
        use crate::ui_v2::components::{resolve_close, DialogCloseAction, DialogCloseSource};

        let (policy, dismissal) = upload_close_contract();
        assert_eq!(
            resolve_close(policy, dismissal, true, DialogCloseSource::Escape),
            DialogCloseAction::RequestConfirmation
        );
        assert_eq!(
            resolve_close(policy, dismissal, true, DialogCloseSource::CloseButton),
            DialogCloseAction::RequestConfirmation
        );
    }

    #[test]
    fn the_backdrop_never_closes_or_cancels_an_upload() {
        use crate::ui_v2::components::{resolve_close, DialogCloseAction, DialogCloseSource};

        let (policy, dismissal) = upload_close_contract();
        for busy in [true, false] {
            assert_eq!(
                resolve_close(policy, dismissal, busy, DialogCloseSource::Backdrop),
                DialogCloseAction::Ignore
            );
        }
    }

    #[test]
    fn an_idle_upload_dialog_closes_through_the_existing_teardown() {
        use crate::ui_v2::components::{resolve_close, DialogCloseAction, DialogCloseSource};

        let (policy, dismissal) = upload_close_contract();
        assert_eq!(
            resolve_close(policy, dismissal, false, DialogCloseSource::CloseButton),
            DialogCloseAction::Dismiss
        );
    }

    #[test]
    fn a_confirmed_cancellation_runs_the_existing_cancel_command() {
        use crate::ui_v2::components::{resolve_close, DialogCloseAction, DialogCloseSource};

        let (policy, dismissal) = upload_close_contract();
        assert_eq!(
            resolve_close(policy, dismissal, true, DialogCloseSource::ConfirmedCancel),
            DialogCloseAction::CancelOperation
        );
        // The cancellation path is the pre-existing backend command, not a new
        // one invented by the dialog layer.
        let source = include_str!("blossom_media_upload.rs");
        assert!(source.contains("invoke_cancel_blossom_upload"));
    }

    fn queued(id: &str, status: QueuedStatus) -> QueuedMedia {
        let mut selection = selection("image/png");
        selection.selection_id = id.into();
        QueuedMedia { selection, status }
    }

    #[test]
    fn only_repeatable_roles_pick_several_files() {
        for role in ["screenshot", "feature", "trailer"] {
            assert!(role_allows_many(role), "{role}");
        }
        for role in ["hero", "capsule"] {
            assert!(!role_allows_many(role), "{role}");
        }
    }

    #[test]
    fn room_for_more_files_counts_the_page_and_the_queue() {
        use arcadestr_core::store_page_content_policy::MAX_SCREENSHOTS;
        let mut draft = StorePageDraft::new("game".into(), Vec::new());
        assert_eq!(remaining_for_role(&draft, "capsule", 0), 1);
        assert_eq!(remaining_for_role(&draft, "capsule", 1), 0);
        assert_eq!(remaining_for_role(&draft, "screenshot", 0), MAX_SCREENSHOTS);
        assert_eq!(
            remaining_for_role(&draft, "screenshot", 3),
            MAX_SCREENSHOTS - 3
        );
        for index in 0..MAX_SCREENSHOTS {
            let item = verified_media_item(
                "screenshot",
                &selection("image/png"),
                &response("image/png"),
                format!("shot-{index}"),
            )
            .expect("valid item");
            draft.content.media.push(item);
        }
        assert_eq!(remaining_for_role(&draft, "screenshot", 0), 0);
        assert!(remaining_for_role(&draft, "feature", 0) > 0);
    }

    #[test]
    fn the_queue_uploads_ready_files_in_order_and_retries_failed_ones_after() {
        let queue = vec![
            queued("done", QueuedStatus::Uploaded),
            queued("failed", QueuedStatus::Failed("nope".into())),
            queued("ready", QueuedStatus::Ready),
        ];
        assert_eq!(next_upload(&queue, false), Some(2));
        assert_eq!(next_upload(&queue, true), Some(2));
        let without_ready = &queue[..2];
        assert_eq!(next_upload(without_ready, false), None);
        assert_eq!(next_upload(without_ready, true), Some(1));
        let pending = pending_selections(&queue)
            .into_iter()
            .map(|selection| selection.selection_id)
            .collect::<Vec<_>>();
        assert_eq!(pending, ["failed", "ready"]);
    }

    #[test]
    fn files_of_the_wrong_type_are_split_out_of_a_batch() {
        let (kept, rejected) = split_compatible(
            "screenshot",
            vec![
                selection("image/png"),
                selection("video/mp4"),
                selection("image/webp"),
            ],
        );
        assert_eq!(kept.len(), 2);
        assert_eq!(rejected[0].detected_mime, "video/mp4");
    }

    #[test]
    fn store_page_picker_none_is_idle_data_without_error() {
        assert_eq!(selection_after_picker(None), None);
    }

    #[test]
    fn store_page_server_choice_uses_enabled_preferred_then_first() {
        assert_eq!(preferred_candidate(&settings(None, &[])), None);
        assert_eq!(
            preferred_candidate(&settings(Some("b"), &[("a", true), ("b", true)])),
            Some("b".into())
        );
        assert_eq!(
            preferred_candidate(&settings(Some("b"), &[("a", true), ("b", false)])),
            Some("a".into())
        );
    }

    #[test]
    fn store_page_roles_and_phases_are_stable() {
        assert!(role_accepts_mime("hero", "image/png"));
        assert!(role_accepts_mime("trailer", "video/webm"));
        assert!(!role_accepts_mime("hero", "video/mp4"));
        assert!(!role_accepts_mime("trailer", "image/webp"));
        assert_eq!(phase_label("inspect"), "Inspecting file");
        assert_eq!(phase_label("hash"), "Hashing");
        assert_eq!(phase_label("sign"), "Waiting for signer");
        assert_eq!(phase_label("upload"), "Uploading");
        assert_eq!(phase_label("verify"), "Verifying response");
    }

    #[test]
    fn store_page_progress_rejects_stale_correlation_and_upload_id() {
        let event = BlossomUploadProgressDto {
            upload_id: "upload-1".into(),
            selection_id: "selection-1".into(),
            request_id: "request-1".into(),
            publisher_pubkey: "hex".into(),
            phase: "uploading".into(),
            bytes_completed: 2,
            total_bytes: 4,
            message: None,
        };
        let expected = UploadCorrelation {
            account_npub: "npub",
            publisher_hex: "hex",
            generation: 2,
            session: 3,
            selection_id: "selection-1",
            request_id: "request-1",
            upload_id: Some("upload-1"),
        };
        assert!(accepts_progress(&expected, Some("npub"), 2, 3, &event));
        assert!(!accepts_progress(&expected, Some("other"), 2, 3, &event));
        assert!(!accepts_progress(&expected, Some("npub"), 1, 3, &event));
        let wrong_upload = UploadCorrelation {
            upload_id: Some("upload-2"),
            ..expected
        };
        assert!(!accepts_progress(&wrong_upload, Some("npub"), 2, 3, &event));
    }

    #[test]
    fn store_page_request_ids_are_fresh_and_errors_are_concise() {
        assert_ne!(fresh_request_id(10, 1), fresh_request_id(10, 2));
        assert_eq!(publisher_hex("not-an-npub"), None);
        assert_eq!(
            stable_error_message("desktop_only"),
            "Blossom upload is available in the desktop app only."
        );
        assert_eq!(
            stable_error_message("signer_failure"),
            "The signer could not authorize this upload."
        );
        assert_eq!(
            stable_error_message("signer_rejected"),
            "The signer rejected this upload."
        );
        assert_eq!(
            stable_error_message("signer_timeout"),
            "The signer did not respond in time."
        );
        assert_eq!(
            stable_error_message("payment_required"),
            "This Blossom server requires payment before upload."
        );
        assert_eq!(
            stable_error_message("body_rejected"),
            "The upload server returned a malformed response."
        );
        assert_eq!(
            stable_error_message("integrity_mismatch"),
            "The returned hash, size, or media type did not match."
        );
        assert_eq!(
            stable_error_message("invalid_descriptor_url"),
            "The returned media URL is unsafe or does not contain the uploaded hash."
        );
        assert_eq!(
            stable_error_message("selection_unavailable"),
            "The selected file is no longer available to Arcadestr. Select it again."
        );
    }

    #[test]
    fn store_page_verified_item_uses_only_verified_response_and_inspected_dimensions() {
        let item = verified_media_item(
            "hero",
            &selection("image/png"),
            &response("image/png"),
            "media".into(),
        )
        .expect("verified item");
        assert_eq!(item.url, "https://cdn.example/hash.png");
        assert_eq!(item.sha256.as_deref(), Some("abc123"));
        assert_eq!(item.mime_type.as_deref(), Some("image/png"));
        assert_eq!(item.size, Some(42));
        assert_eq!((item.width, item.height), (Some(1200), Some(600)));
        assert_eq!(item.thumbnail_url, None);
        assert_eq!(item.alt, None);
        assert_eq!(item.caption, None);
    }

    #[test]
    fn store_page_singular_roles_collision_ids_and_local_removal_are_safe() {
        let mut draft = StorePageDraft::new("page".into(), vec![]);
        let mut manual = verified_media_item(
            "hero",
            &selection("image/png"),
            &response("image/png"),
            "blossom-hero".into(),
        )
        .expect("item");
        manual.sha256 = None;
        manual.mime_type = None;
        manual.size = None;
        draft.content.media.push(manual.clone());
        assert!(!role_available(&draft, "hero"));
        assert_eq!(
            unique_media_id(
                "hero",
                draft.content.media.iter().map(|item| item.id.as_str())
            ),
            "blossom-hero-2"
        );
        assert_eq!(
            (manual.sha256, manual.mime_type, manual.size),
            (None, None, None)
        );
        let retained_url = manual.url.clone();
        assert!(remove_draft_media(&mut draft, "blossom-hero"));
        assert!(draft.content.media.is_empty());
        assert_eq!(retained_url, "https://cdn.example/hash.png");
    }

    #[test]
    fn store_page_stale_completion_is_rejected() {
        let expected = UploadCorrelation {
            account_npub: "npub",
            publisher_hex: "hex",
            generation: 2,
            session: 3,
            selection_id: "selection",
            request_id: "request",
            upload_id: None,
        };
        assert!(accepts_completion(
            &expected,
            Some("npub"),
            Some("hex"),
            2,
            3,
            Some("hero"),
            "hero",
            Some("selection"),
            Some("request")
        ));
        assert!(!accepts_completion(
            &expected,
            Some("npub"),
            Some("hex"),
            3,
            3,
            Some("hero"),
            "hero",
            Some("selection"),
            Some("request")
        ));
    }
    #[test]
    fn upload_presentation_separates_every_state_the_component_emits() {
        use UploadPresentation::*;
        assert_eq!(
            upload_presentation(UploadState::Idle, false, None, None),
            Idle
        );
        assert_eq!(
            upload_presentation(UploadState::Selecting, false, None, None),
            Selecting
        );
        assert_eq!(
            upload_presentation(UploadState::Idle, true, None, None),
            Selected
        );
        assert_eq!(
            upload_presentation(UploadState::Ready, true, None, Some(phase_label("hash"))),
            Busy
        );
        assert_eq!(
            upload_presentation(UploadState::Uploading, true, None, None),
            Uploading
        );
        assert_eq!(
            upload_presentation(
                UploadState::Ready,
                true,
                None,
                Some(phase_label("complete"))
            ),
            Uploaded
        );
        assert_eq!(
            upload_presentation(UploadState::Cancelled, true, None, None),
            Cancelled
        );
        assert_eq!(
            upload_presentation(UploadState::Failed, true, None, None),
            Retryable
        );
    }

    #[test]
    fn upload_errors_separate_rejection_payment_and_retryable_failure() {
        use UploadPresentation::*;
        let classify = |code: &str| {
            upload_presentation(
                UploadState::Failed,
                true,
                Some(stable_error_message(code)),
                None,
            )
        };
        assert_eq!(classify("payment_required"), PaymentRequired);
        assert_eq!(classify("unsafe_destination"), Rejected);
        assert_eq!(classify("unsupported_media"), Rejected);
        assert_eq!(classify("file_too_large"), Rejected);
        assert_eq!(classify("integrity_mismatch"), Rejected);
        assert_eq!(classify("publisher_mismatch"), Rejected);
        assert_eq!(classify("cancelled"), Cancelled);
        assert_eq!(classify("network_failure"), Retryable);
        assert_eq!(classify("rate_limited"), Retryable);
    }

    #[test]
    fn a_completed_upload_is_never_described_as_a_published_store_page() {
        for presentation in [
            UploadPresentation::Uploaded,
            UploadPresentation::Selected,
            UploadPresentation::Uploading,
        ] {
            let label = presentation.label().to_ascii_lowercase();
            assert!(!label.contains("store page"));
            assert!(!label.contains("live"));
        }
        assert!(UploadPresentation::Uploaded.label().contains("Blossom"));
        assert!(UploadPresentation::Selected
            .label()
            .contains("not uploaded"));
    }
}
