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

/// Installed ADP game returned by `get_installed_games`.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct InstalledGame {
    pub game_coordinate: String,
    pub file_path: String,
    pub file_hash: String,
    pub version: Option<String>,
    pub server_url: String,
    pub installed_at: i64,
}

/// Completion payload emitted after an ADP install finishes.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct DownloadCompletePayload {
    pub game_coordinate: String,
    pub listing_id: String,
    pub file_path: String,
}

/// Invoke desktop `get_installed_games` command.
#[cfg(not(feature = "web"))]
pub async fn invoke_get_installed_games() -> Result<Vec<InstalledGame>, String> {
    crate::tauri_invoke::invoke("get_installed_games", serde_json::json!({})).await
}

/// Web fallback for `get_installed_games`.
#[cfg(feature = "web")]
pub async fn invoke_get_installed_games() -> Result<Vec<InstalledGame>, String> {
    Ok(Vec::new())
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
#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FulfillmentMode {
    None,
    Direct,
    Delegate,
}

/// Request payload for the ADP publish command.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct PublishAdpListingRequest {
    pub d_tag: String,
    pub title: String,
    pub description: String,
    pub price_sats: u64,
    pub lud16: Option<String>,
    pub tags: Vec<String>,
    pub images: Vec<String>,
    pub fulfillment_mode: FulfillmentMode,
    pub operator_url: Option<String>,
    pub servers: Vec<String>,
    pub file_path: Option<String>,
    pub existing_file_hash: Option<String>,
    pub existing_fulfillment_pubkey: Option<String>,
    pub existing_fulfillment_valid_from: Option<u64>,
    pub existing_fulfillment_revoked_at: Option<u64>,
    pub version: Option<String>,
    pub acquisition: crate::models::AcquisitionPolicy,
    pub platforms: Vec<String>,
}

/// Request payload for resolving an edited listing's delegated operator.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct ResolveAdpOperatorRequest {
    pub publisher_npub: String,
    pub fulfillment_pubkey: String,
    pub scope: String,
}

/// Upload response nested in ADP publish results.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct UploadResponse {
    pub game_coordinate: String,
    pub file_hash: String,
    pub download_url: String,
}

/// Per-server upload status returned by ADP publish results.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct PublishServerUploadResult {
    pub server_url: String,
    pub status: String,
    pub error: Option<String>,
    pub upload: Option<UploadResponse>,
}

/// Result returned by the ADP publish command.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct PublishAdpListingResult {
    pub event_id: String,
    pub acceptance_event_id: Option<String>,
    pub fulfillment_pubkey: Option<String>,
    pub file_hash: Option<String>,
    pub uploads: Vec<PublishServerUploadResult>,
}

/// Progress event payload emitted during ADP publishing.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct PublishProgressPayload {
    pub step: String,
    pub status: String,
    pub server_url: Option<String>,
    pub message: Option<String>,
}

/// ADP server announcement discovered from relays.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct AdpServerAnnouncement {
    pub pubkey: String,
    pub url: String,
    pub name: Option<String>,
    pub supported_adp: Option<String>,
    pub contact: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct HashBuildFileRequest {
    pub file_path: String,
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

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct ClaimEntitlementRequest {
    pub publisher_npub: String,
    pub listing_id: String,
    pub campaign_event_id: String,
    pub server_url: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct ClaimEntitlementResponse {
    pub grant: serde_json::Value,
    pub download_token: String,
    pub token_expires_at: i64,
    pub already_claimed: bool,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct DiscoverCampaignsRequest {
    pub publisher_npub: String,
    pub listing_id: String,
    pub pointers: Vec<CampaignPointerInput>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct CampaignPointerInput {
    pub root_event_id: String,
    pub relay_hint: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct DiscoverCampaignSummariesRequest {
    pub publisher_npub: String,
    pub listings: Vec<CampaignSummaryListingInput>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct CampaignSummaryListingInput {
    pub listing_id: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct CampaignSummary {
    pub listing_id: String,
    pub active: usize,
    pub upcoming: usize,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct DiscoveredCampaign {
    pub root_event_id: String,
    pub campaign_id: String,
    pub starts_at: u64,
    pub ends_at: u64,
    pub classification: String,
    #[serde(default)]
    pub event_id: Option<String>,
    #[serde(default)]
    pub predecessor_event_id: Option<String>,
    #[serde(default)]
    pub mode: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct PublishCampaignRequest {
    pub publisher_npub: String,
    pub listing_id: String,
    pub campaign_id: String,
    pub starts_at: Option<u64>,
    pub ends_at: Option<u64>,
    pub predecessor_event_id: Option<String>,
    pub cancel: bool,
    pub update_listing_pointer: bool,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct PublishCampaignResponse {
    pub event_id: String,
    pub root_event_id: String,
    pub listing_event_id: Option<String>,
    #[serde(default)]
    pub pointer_update_error: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct UpdateCampaignPointerRequest {
    pub publisher_npub: String,
    pub listing_id: String,
    pub campaign_root_id: String,
    pub remove: bool,
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

/// Invoke desktop `discover_adp_servers` command.
#[cfg(not(feature = "web"))]
pub async fn invoke_discover_adp_servers() -> Result<Vec<AdpServerAnnouncement>, String> {
    crate::tauri_invoke::invoke("discover_adp_servers", serde_json::json!({})).await
}

/// Web fallback for `discover_adp_servers`.
#[cfg(feature = "web")]
pub async fn invoke_discover_adp_servers() -> Result<Vec<AdpServerAnnouncement>, String> {
    Ok(Vec::new())
}

/// Invoke desktop `select_build_file` command.
#[cfg(not(feature = "web"))]
pub async fn invoke_select_build_file() -> Result<Option<String>, String> {
    crate::tauri_invoke::invoke("select_build_file", serde_json::json!({})).await
}

/// Web fallback for `select_build_file`.
#[cfg(feature = "web")]
pub async fn invoke_select_build_file() -> Result<Option<String>, String> {
    Err("Build file selection is only available in desktop builds.".to_string())
}

/// Invoke desktop `hash_build_file` command.
#[cfg(not(feature = "web"))]
pub async fn invoke_hash_build_file(request: HashBuildFileRequest) -> Result<String, String> {
    crate::tauri_invoke::invoke("hash_build_file", serde_json::json!({ "request": request })).await
}

/// Web fallback for `hash_build_file`.
#[cfg(feature = "web")]
pub async fn invoke_hash_build_file(_request: HashBuildFileRequest) -> Result<String, String> {
    Err("Build file hashing is only available in desktop builds.".to_string())
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

/// Invoke desktop `resolve_adp_operator` command.
#[cfg(not(feature = "web"))]
pub async fn invoke_resolve_adp_operator(
    request: ResolveAdpOperatorRequest,
) -> Result<Option<String>, String> {
    crate::tauri_invoke::invoke(
        "resolve_adp_operator",
        serde_json::json!({ "request": request }),
    )
    .await
}

/// Web fallback for `resolve_adp_operator`.
#[cfg(feature = "web")]
pub async fn invoke_resolve_adp_operator(
    _request: ResolveAdpOperatorRequest,
) -> Result<Option<String>, String> {
    Ok(None)
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
    crate::tauri_invoke::invoke(
        "confirm_purchase",
        serde_json::json!({ "request": request }),
    )
    .await
}

#[cfg(feature = "web")]
pub async fn invoke_confirm_purchase(
    _request: ConfirmPurchaseRequest,
) -> Result<ConfirmPurchaseResponse, String> {
    Err("ADP purchase confirmation is only available in desktop builds.".to_string())
}

#[cfg(not(feature = "web"))]
pub async fn invoke_claim_entitlement(
    request: ClaimEntitlementRequest,
) -> Result<ClaimEntitlementResponse, String> {
    crate::tauri_invoke::invoke(
        "claim_entitlement",
        serde_json::json!({ "request": request }),
    )
    .await
}

#[cfg(feature = "web")]
pub async fn invoke_claim_entitlement(
    _request: ClaimEntitlementRequest,
) -> Result<ClaimEntitlementResponse, String> {
    Err("ADP entitlement claims are only available in desktop builds.".to_string())
}

#[cfg(not(feature = "web"))]
pub async fn invoke_discover_campaigns(
    request: DiscoverCampaignsRequest,
) -> Result<Vec<DiscoveredCampaign>, String> {
    crate::tauri_invoke::invoke(
        "discover_campaigns",
        serde_json::json!({ "request": request }),
    )
    .await
}

#[cfg(not(feature = "web"))]
pub async fn invoke_discover_campaign_summaries(
    request: DiscoverCampaignSummariesRequest,
) -> Result<Vec<CampaignSummary>, String> {
    crate::tauri_invoke::invoke(
        "discover_campaign_summaries",
        serde_json::json!({ "request": request }),
    )
    .await
}

#[cfg(feature = "web")]
pub async fn invoke_discover_campaign_summaries(
    _request: DiscoverCampaignSummariesRequest,
) -> Result<Vec<CampaignSummary>, String> {
    Ok(Vec::new())
}

#[cfg(feature = "web")]
pub async fn invoke_discover_campaigns(
    _request: DiscoverCampaignsRequest,
) -> Result<Vec<DiscoveredCampaign>, String> {
    Ok(Vec::new())
}

#[cfg(not(feature = "web"))]
pub async fn invoke_publish_campaign(
    request: PublishCampaignRequest,
) -> Result<PublishCampaignResponse, String> {
    crate::tauri_invoke::invoke(
        "publish_campaign",
        serde_json::json!({ "request": request }),
    )
    .await
}

#[cfg(not(feature = "web"))]
pub async fn invoke_update_campaign_pointer(
    request: UpdateCampaignPointerRequest,
) -> Result<String, String> {
    crate::tauri_invoke::invoke(
        "update_campaign_pointer",
        serde_json::json!({ "request": request }),
    )
    .await
}

#[cfg(feature = "web")]
pub async fn invoke_update_campaign_pointer(
    _request: UpdateCampaignPointerRequest,
) -> Result<String, String> {
    Err("Campaign pointer updates are only available in desktop builds.".to_string())
}

#[cfg(feature = "web")]
pub async fn invoke_publish_campaign(
    _request: PublishCampaignRequest,
) -> Result<PublishCampaignResponse, String> {
    Err("Campaign publishing is only available in desktop builds.".to_string())
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

/// Listen for desktop `download-complete` events.
#[cfg(not(feature = "web"))]
pub async fn listen_download_complete<F>(mut callback: F) -> Result<Box<dyn FnOnce()>, String>
where
    F: FnMut(DownloadCompletePayload) + 'static,
{
    let cleanup = crate::tauri_invoke::listen("download-complete", move |value| {
        if let Ok(payload) = serde_json::from_value::<DownloadCompletePayload>(value) {
            callback(payload);
        }
    })
    .await?;
    Ok(Box::new(cleanup))
}

/// Web fallback for `download-complete` events.
#[cfg(feature = "web")]
pub async fn listen_download_complete<F>(_callback: F) -> Result<Box<dyn FnOnce()>, String>
where
    F: FnMut(DownloadCompletePayload) + 'static,
{
    Err("ADP installation events are only available in desktop builds.".to_string())
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn publish_request_serialization_preserves_existing_fulfillment_metadata() {
        let request = PublishAdpListingRequest {
            d_tag: "game".to_string(),
            title: "Game".to_string(),
            description: "Description".to_string(),
            price_sats: 0,
            lud16: None,
            tags: vec![],
            images: vec![],
            fulfillment_mode: FulfillmentMode::Delegate,
            operator_url: Some("https://operator.example.com".to_string()),
            servers: vec!["https://dist.example.com".to_string()],
            file_path: None,
            existing_file_hash: Some("hash".to_string()),
            existing_fulfillment_pubkey: Some("delegated-key".to_string()),
            existing_fulfillment_valid_from: Some(123),
            existing_fulfillment_revoked_at: Some(456),
            version: Some("1.0.0".to_string()),
            acquisition: crate::models::AcquisitionPolicy::TimedAccess {
                starts_at: 100,
                ends_at: 200,
            },
            platforms: vec!["linux-x86_64".to_string()],
        };

        let value = serde_json::to_value(request).expect("request should serialize");

        assert_eq!(value["existing_fulfillment_pubkey"], "delegated-key");
        assert_eq!(value["existing_fulfillment_valid_from"], 123);
        assert_eq!(value["existing_fulfillment_revoked_at"], 456);
        assert_eq!(value["acquisition"]["TimedAccess"]["starts_at"], 100);
        assert_eq!(value["acquisition"]["TimedAccess"]["ends_at"], 200);
    }
}
