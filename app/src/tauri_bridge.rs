// Typed IPC bridge wrappers for NIP-49 and NIP-05 desktop commands.

use crate::models::{Nip05Status, Nip49ExportResult, Nip49ImportRequest};

/// Invoke desktop `nip49_import` command.
#[cfg(not(feature = "web"))]
pub async fn invoke_nip49_import(request: Nip49ImportRequest) -> Result<String, String> {
    crate::tauri_invoke::invoke("nip49_import", serde_json::json!({ "request": request })).await
}

/// Web fallback for `nip49_import`.
#[cfg(feature = "web")]
pub async fn invoke_nip49_import(_request: Nip49ImportRequest) -> Result<String, String> {
    Err("nip49_import is only available in desktop builds".to_string())
}

/// Invoke desktop `nip49_export` command.
#[cfg(not(feature = "web"))]
pub async fn invoke_nip49_export(
    npub: String,
    password: String,
) -> Result<Nip49ExportResult, String> {
    crate::tauri_invoke::invoke(
        "nip49_export",
        serde_json::json!({
            "npub": npub,
            "password": password,
        }),
    )
    .await
}

/// Web fallback for `nip49_export`.
#[cfg(feature = "web")]
pub async fn invoke_nip49_export(
    _npub: String,
    _password: String,
) -> Result<Nip49ExportResult, String> {
    Err("nip49_export is only available in desktop builds".to_string())
}

/// Invoke desktop `verify_nip05` command.
#[cfg(not(feature = "web"))]
pub async fn invoke_verify_nip05(identifier: String) -> Result<Nip05Status, String> {
    crate::tauri_invoke::invoke(
        "verify_nip05",
        serde_json::json!({ "identifier": identifier }),
    )
    .await
}

/// Web fallback for `verify_nip05`.
#[cfg(feature = "web")]
pub async fn invoke_verify_nip05(_identifier: String) -> Result<Nip05Status, String> {
    Err("verify_nip05 is only available in desktop builds".to_string())
}
