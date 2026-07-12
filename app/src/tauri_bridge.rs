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

/// ADP server metadata returned by `check_adp_server`.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct AdpServerInfo {
    pub adp_version: String,
    pub pubkey: String,
    pub name: Option<String>,
    pub url: Option<String>,
}

/// Request payload for the ADP publish command.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct PublishAdpListingRequest {
    pub d_tag: String,
    pub title: String,
    pub description: String,
    pub price_sats: u64,
    pub lud16: Option<String>,
    pub server_url: String,
    pub file_path: String,
    pub version: String,
    pub platforms: Vec<String>,
}

/// Upload response nested in ADP publish results.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct UploadResponse {
    pub game_coordinate: String,
    pub file_hash: String,
    pub download_url: String,
}

/// Result returned by the ADP publish command.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct PublishAdpListingResult {
    pub event_id: String,
    pub acceptance_event_id: String,
    pub fulfillment_pubkey: String,
    pub file_hash: String,
    pub upload: UploadResponse,
}

/// Progress event payload emitted during ADP publishing.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct PublishProgressPayload {
    pub step: String,
    pub status: String,
    pub message: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct RequestLnurlInvoiceRequest {
    pub lud16: String,
    pub amount_sats: u64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct RequestLnurlInvoiceResponse {
    pub bolt11: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct ConnectNwcWalletRequest {
    pub connection_string: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct ConnectNwcWalletResponse {
    pub wallet_pubkey: String,
    pub relays: Vec<String>,
    pub lud16: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct PayNwcInvoiceRequest {
    pub bolt11: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct PayNwcInvoiceResponse {
    pub preimage: String,
    pub fees_paid_msat: Option<u64>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct ConfirmPurchaseRequest {
    pub publisher_npub: String,
    pub listing_id: String,
    pub server_url: String,
    pub bolt11: String,
    pub preimage: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct ConfirmPurchaseResponse {
    pub receipt: serde_json::Value,
    pub download_token: String,
    pub token_expires_at: i64,
}

/// Invoke desktop `check_adp_server` command.
#[cfg(not(feature = "web"))]
pub async fn invoke_check_adp_server(server_url: String) -> Result<AdpServerInfo, String> {
    crate::tauri_invoke::invoke(
        "check_adp_server",
        serde_json::json!({ "serverUrl": server_url }),
    )
    .await
}

/// Web fallback for `check_adp_server`.
#[cfg(feature = "web")]
pub async fn invoke_check_adp_server(_server_url: String) -> Result<AdpServerInfo, String> {
    Err("ADP server checks are only available in desktop builds.".to_string())
}

/// Invoke desktop `publish_adp_listing` command.
#[cfg(not(feature = "web"))]
pub async fn invoke_publish_adp_listing(
    request: PublishAdpListingRequest,
) -> Result<PublishAdpListingResult, String> {
    crate::tauri_invoke::invoke(
        "publish_adp_listing",
        serde_json::json!({ "request": request }),
    )
    .await
}

/// Web fallback for `publish_adp_listing`.
#[cfg(feature = "web")]
pub async fn invoke_publish_adp_listing(
    _request: PublishAdpListingRequest,
) -> Result<PublishAdpListingResult, String> {
    Err("ADP publishing is only available in desktop builds.".to_string())
}

#[cfg(not(feature = "web"))]
pub async fn invoke_connect_nwc_wallet(
    request: ConnectNwcWalletRequest,
) -> Result<ConnectNwcWalletResponse, String> {
    crate::tauri_invoke::invoke(
        "connect_nwc_wallet",
        serde_json::json!({ "request": request }),
    )
    .await
}

#[cfg(feature = "web")]
pub async fn invoke_connect_nwc_wallet(
    _request: ConnectNwcWalletRequest,
) -> Result<ConnectNwcWalletResponse, String> {
    Err("NWC wallet connection is only available in desktop builds.".to_string())
}

#[cfg(not(feature = "web"))]
pub async fn invoke_request_lnurl_invoice(
    request: RequestLnurlInvoiceRequest,
) -> Result<RequestLnurlInvoiceResponse, String> {
    crate::tauri_invoke::invoke(
        "request_lnurl_invoice",
        serde_json::json!({ "request": request }),
    )
    .await
}

#[cfg(feature = "web")]
pub async fn invoke_request_lnurl_invoice(
    _request: RequestLnurlInvoiceRequest,
) -> Result<RequestLnurlInvoiceResponse, String> {
    Err("ADP invoice requests are only available in desktop builds.".to_string())
}

#[cfg(not(feature = "web"))]
pub async fn invoke_pay_nwc_invoice(
    request: PayNwcInvoiceRequest,
) -> Result<PayNwcInvoiceResponse, String> {
    crate::tauri_invoke::invoke("pay_nwc_invoice", serde_json::json!({ "request": request })).await
}

#[cfg(feature = "web")]
pub async fn invoke_pay_nwc_invoice(
    _request: PayNwcInvoiceRequest,
) -> Result<PayNwcInvoiceResponse, String> {
    Err("NWC payments are only available in desktop builds.".to_string())
}

#[cfg(not(feature = "web"))]
pub async fn invoke_confirm_purchase(
    request: ConfirmPurchaseRequest,
) -> Result<ConfirmPurchaseResponse, String> {
    crate::tauri_invoke::invoke("confirm_purchase", serde_json::json!({ "request": request })).await
}

#[cfg(feature = "web")]
pub async fn invoke_confirm_purchase(
    _request: ConfirmPurchaseRequest,
) -> Result<ConfirmPurchaseResponse, String> {
    Err("ADP purchase confirmation is only available in desktop builds.".to_string())
}

/// Listen for desktop `publish-progress` events.
#[cfg(not(feature = "web"))]
pub async fn listen_publish_progress<F>(mut callback: F) -> Result<Box<dyn FnOnce()>, String>
where
    F: FnMut(PublishProgressPayload) + 'static,
{
    let cleanup = crate::tauri_invoke::listen("publish-progress", move |value| {
        if let Ok(payload) = serde_json::from_value::<PublishProgressPayload>(value) {
            callback(payload);
        }
    })
    .await?;
    Ok(Box::new(cleanup))
}

/// Web fallback for `publish-progress` events.
#[cfg(feature = "web")]
pub async fn listen_publish_progress<F>(_callback: F) -> Result<Box<dyn FnOnce()>, String>
where
    F: FnMut(PublishProgressPayload) + 'static,
{
    Err("ADP publishing is only available in desktop builds.".to_string())
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
