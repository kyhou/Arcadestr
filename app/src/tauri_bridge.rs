// Typed IPC bridge wrappers for NIP-49 and NIP-05 desktop commands.

use crate::models::{
    EarnedBadgeSummary, GameListing, Nip05Status, Nip49ExportResult, Nip49ImportRequest,
    PlatformInfo, ProfileBadgeEntry,
};

/// Invoke desktop `get_platform_info` command.
#[cfg(not(feature = "web"))]
pub async fn invoke_get_platform_info() -> Result<PlatformInfo, String> {
    crate::tauri_invoke::invoke("get_platform_info", serde_json::json!({})).await
}

/// Web fallback for `get_platform_info`.
#[cfg(feature = "web")]
pub async fn invoke_get_platform_info() -> Result<PlatformInfo, String> {
    Err("Platform detection is only available in desktop builds.".to_string())
}

/// Invoke desktop `install_game` command.
#[cfg(not(feature = "web"))]
pub async fn invoke_install_game(listing: &GameListing) -> Result<(), String> {
    crate::tauri_invoke::invoke("install_game", serde_json::json!({ "listing": listing })).await
}

/// Web fallback for `install_game`.
#[cfg(feature = "web")]
pub async fn invoke_install_game(_listing: &GameListing) -> Result<(), String> {
    Err("Game installation is only available in desktop builds.".to_string())
}

/// Invoke desktop `ingest_receipt` command.
#[cfg(not(feature = "web"))]
pub async fn invoke_ingest_receipt(raw_event_json: String) -> Result<(), String> {
    crate::tauri_invoke::invoke(
        "ingest_receipt",
        serde_json::json!({ "rawEventJson": raw_event_json }),
    )
    .await
}

/// Web fallback for `ingest_receipt`.
#[cfg(feature = "web")]
pub async fn invoke_ingest_receipt(_raw_event_json: String) -> Result<(), String> {
    Err("Receipt ingestion is only available in desktop builds.".to_string())
}

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
pub async fn invoke_verify_nip05(
    identifier: String,
    expected_npub: String,
) -> Result<Nip05Status, String> {
    crate::tauri_invoke::invoke(
        "verify_nip05",
        serde_json::json!({ "identifier": identifier, "expectedNpub": expected_npub }),
    )
    .await
}

/// Web fallback for `verify_nip05`.
#[cfg(feature = "web")]
pub async fn invoke_verify_nip05(
    _identifier: String,
    _expected_npub: String,
) -> Result<Nip05Status, String> {
    Err("verify_nip05 is only available in desktop builds".to_string())
}

/// Invoke desktop `get_cached_earned_badges` command.
#[cfg(not(feature = "web"))]
pub async fn get_cached_earned_badges(
    profile_pubkey: String,
) -> Result<Vec<EarnedBadgeSummary>, String> {
    crate::tauri_invoke::invoke(
        "get_cached_earned_badges",
        serde_json::json!({ "profilePubkey": profile_pubkey }),
    )
    .await
}

/// Web fallback for `get_cached_earned_badges`.
#[cfg(feature = "web")]
pub async fn get_cached_earned_badges(
    _profile_pubkey: String,
) -> Result<Vec<EarnedBadgeSummary>, String> {
    Err("Badge relay display is not yet available on the web target.".to_string())
}

/// Invoke desktop `get_cached_profile_badges` command.
#[cfg(not(feature = "web"))]
pub async fn get_cached_profile_badges(
    profile_pubkey: String,
) -> Result<Vec<ProfileBadgeEntry>, String> {
    crate::tauri_invoke::invoke(
        "get_cached_profile_badges",
        serde_json::json!({ "profilePubkey": profile_pubkey }),
    )
    .await
}

/// Web fallback for `get_cached_profile_badges`.
#[cfg(feature = "web")]
pub async fn get_cached_profile_badges(
    _profile_pubkey: String,
) -> Result<Vec<ProfileBadgeEntry>, String> {
    Err("Badge relay display is not yet available on the web target.".to_string())
}

/// Invoke desktop `fetch_earned_badges` command.
#[cfg(not(feature = "web"))]
pub async fn fetch_earned_badges(
    profile_pubkey: String,
) -> Result<Vec<EarnedBadgeSummary>, String> {
    crate::tauri_invoke::invoke(
        "fetch_earned_badges",
        serde_json::json!({ "profilePubkey": profile_pubkey }),
    )
    .await
}

/// Web fallback for `fetch_earned_badges`.
#[cfg(feature = "web")]
pub async fn fetch_earned_badges(
    _profile_pubkey: String,
) -> Result<Vec<EarnedBadgeSummary>, String> {
    Err("Badge relay display is not yet available on the web target.".to_string())
}

/// Invoke desktop `fetch_profile_badges` command.
#[cfg(not(feature = "web"))]
pub async fn fetch_profile_badges(
    profile_pubkey: String,
) -> Result<Vec<ProfileBadgeEntry>, String> {
    crate::tauri_invoke::invoke(
        "fetch_profile_badges",
        serde_json::json!({ "profilePubkey": profile_pubkey }),
    )
    .await
}

/// Web fallback for `fetch_profile_badges`.
#[cfg(feature = "web")]
pub async fn fetch_profile_badges(
    _profile_pubkey: String,
) -> Result<Vec<ProfileBadgeEntry>, String> {
    Err("Badge relay display is not yet available on the web target.".to_string())
}
