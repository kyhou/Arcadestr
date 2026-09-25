//! Tauri command adapters for the account-bound archive registry.

use std::sync::Arc;

use arcadestr_core::archive_inspection_contract::*;
use arcadestr_core::nip46::AppSignerState;
use arcadestr_core::signers::NostrSigner;
use nostr::nips::nip19::ToBech32;
use tauri::State;
use tauri_plugin_dialog::DialogExt;
use tokio::sync::oneshot;

use crate::adp_commands::resolve_active_signer;
use crate::archive_selection::ArchiveSelectionService;
use crate::AppState;

pub(crate) fn account_check(
    state: &State<'_, AppState>,
    signer_state: &State<'_, Arc<tokio::sync::Mutex<AppSignerState>>>,
) -> crate::archive_selection::AccountCheck {
    let auth = Arc::clone(&state.auth);
    let signer_state = Arc::clone(signer_state.inner());
    Arc::new(move || {
        let auth = Arc::clone(&auth);
        let signer_state = Arc::clone(&signer_state);
        Box::pin(async move {
            let snapshot = { auth.lock().await.clone() };
            let signer = resolve_active_signer(&signer_state, &snapshot)
                .await
                .map_err(|_| ArchiveSelectionError::Account {
                    failure: ArchiveAccountFailure::NotAuthenticated,
                })?;
            signer
                .get_public_key()
                .await
                .map_err(|_| ArchiveSelectionError::Account {
                    failure: ArchiveAccountFailure::NotAuthenticated,
                })?
                .to_bech32()
                .map_err(|_| ArchiveSelectionError::Account {
                    failure: ArchiveAccountFailure::InvalidHandle,
                })
        })
    })
}

async fn active_publisher(
    state: &State<'_, AppState>,
    signer_state: &State<'_, Arc<tokio::sync::Mutex<AppSignerState>>>,
) -> Result<String, ArchiveSelectionError> {
    let auth = { state.auth.lock().await.clone() };
    let signer = resolve_active_signer(signer_state.inner(), &auth)
        .await
        .map_err(|_| ArchiveSelectionError::Account {
            failure: ArchiveAccountFailure::NotAuthenticated,
        })?;
    signer
        .get_public_key()
        .await
        .map_err(|_| ArchiveSelectionError::Account {
            failure: ArchiveAccountFailure::NotAuthenticated,
        })?
        .to_bech32()
        .map_err(|_| ArchiveSelectionError::Account {
            failure: ArchiveAccountFailure::InvalidHandle,
        })
}

fn verify_expected(expected: &str, active: &str) -> Result<(), ArchiveSelectionError> {
    if expected == active {
        Ok(())
    } else {
        Err(ArchiveSelectionError::Account {
            failure: ArchiveAccountFailure::AccountMismatch,
        })
    }
}

#[tauri::command]
pub async fn select_archive_build<R: tauri::Runtime>(
    request: SelectArchiveRequest,
    app: tauri::AppHandle<R>,
    state: State<'_, AppState>,
    signer_state: State<'_, Arc<tokio::sync::Mutex<AppSignerState>>>,
    service: State<'_, ArchiveSelectionService>,
) -> Result<SelectArchiveResponse, ArchiveSelectionError> {
    let before = active_publisher(&state, &signer_state).await?;
    verify_expected(&request.expected_publisher_npub, &before)?;
    let (sender, receiver) = oneshot::channel();
    app.dialog()
        .file()
        .add_filter("Archives", &["zip", "gz", "tgz", "zst", "tzst"])
        .pick_file(move |selected| {
            let _ = sender.send(selected);
        });
    let selected = receiver
        .await
        .map_err(|_| ArchiveSelectionError::PickerFailed)?;
    let after = active_publisher(&state, &signer_state).await?;
    verify_expected(&request.expected_publisher_npub, &after)?;
    let selection = match selected {
        Some(path) => Some(
            service
                .stage_source(
                    path.into_path()
                        .map_err(|_| ArchiveSelectionError::PickerFailed)?,
                    &after,
                    account_check(&state, &signer_state),
                )
                .await?,
        ),
        None => None,
    };
    Ok(SelectArchiveResponse { selection })
}

macro_rules! archive_command {
    ($name:ident, $method:ident, $request:ty, $response:ty) => {
        #[tauri::command]
        pub async fn $name(
            request: $request,
            state: State<'_, AppState>,
            signer_state: State<'_, Arc<tokio::sync::Mutex<AppSignerState>>>,
            service: State<'_, ArchiveSelectionService>,
        ) -> Result<$response, ArchiveSelectionError> {
            let active = active_publisher(&state, &signer_state).await?;
            service.$method(request, &active).await
        }
    };
}

#[tauri::command]
pub async fn inspect_archive(
    request: InspectArchiveRequest,
    state: State<'_, AppState>,
    signer_state: State<'_, Arc<tokio::sync::Mutex<AppSignerState>>>,
    service: State<'_, ArchiveSelectionService>,
) -> Result<InspectArchiveResponse, ArchiveSelectionError> {
    let active = active_publisher(&state, &signer_state).await?;
    service
        .inspect(request, &active, account_check(&state, &signer_state))
        .await
}

#[tauri::command]
pub async fn hash_selected_archive(
    request: HashSelectedArchiveRequest,
    state: State<'_, AppState>,
    signer_state: State<'_, Arc<tokio::sync::Mutex<AppSignerState>>>,
    service: State<'_, ArchiveSelectionService>,
) -> Result<HashSelectedArchiveResponse, ArchiveSelectionError> {
    let active = active_publisher(&state, &signer_state).await?;
    service
        .hash(request, &active, account_check(&state, &signer_state))
        .await
}
archive_command!(
    query_archive_files,
    query_files,
    QueryArchiveFilesRequest,
    QueryArchiveFilesResponse
);
archive_command!(
    query_archive_working_directories,
    query_working_directories,
    QueryArchiveWorkingDirectoriesRequest,
    QueryArchiveWorkingDirectoriesResponse
);
archive_command!(
    query_archive_candidates,
    query_candidates,
    QueryArchiveCandidatesRequest,
    QueryArchiveCandidatesResponse
);
archive_command!(
    query_archive_hazards,
    query_hazards,
    QueryArchiveHazardsRequest,
    QueryArchiveHazardsResponse
);
archive_command!(
    query_archive_warnings,
    query_warnings,
    QueryArchiveWarningsRequest,
    QueryArchiveWarningsResponse
);
archive_command!(
    discard_archive_selection,
    discard,
    DiscardArchiveSelectionRequest,
    DiscardArchiveSelectionResponse
);
