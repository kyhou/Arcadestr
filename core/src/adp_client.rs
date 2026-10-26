//! ADP-01 HTTP client for Arcadestr distribution servers.

use std::fs::File;
use std::io;
use std::path::Path;
use std::sync::Arc;

use nostr::Event;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::archive_inspection_contract::SafeArchiveFilename;
use crate::http_client::{HttpClient, HttpClientError, HttpRetainedDownload};
use crate::nip98_client::{build_nip98_auth_header, Nip98ClientError};
use crate::signers::NostrSigner;

/// Errors returned by ADP HTTP client operations.
#[derive(Debug, Error)]
pub enum AdpClientError {
    /// The HTTP layer failed.
    #[error("ADP HTTP request failed: {0}")]
    Http(#[from] HttpClientError),

    /// The ADP response did not match the expected schema.
    #[error("ADP response decode failed: {0}")]
    Decode(#[from] serde_json::Error),

    /// NIP-98 authentication could not be built.
    #[error("ADP authentication failed: {0}")]
    Auth(#[from] Nip98ClientError),

    /// Local file access failed.
    #[error("ADP file I/O failed: {0}")]
    Io(#[from] std::io::Error),

    /// Direct reqwest operation failed.
    #[error("ADP reqwest request failed: {0}")]
    Reqwest(#[from] reqwest::Error),

    /// The requested operation belongs to a later implementation gate.
    #[error("ADP operation is not implemented in Gate 1: {0}")]
    NotImplemented(&'static str),

    /// The server rejected the buyer's ownership proof.
    #[error("ADP download ownership/auth failure: {0}")]
    DownloadOwnership(String),

    /// The server is no longer authorized to distribute the listing.
    #[error("ADP download server no longer distributes this listing: {0}")]
    DownloadDistribution(String),

    /// The server returned a protocol-level download error.
    #[error("ADP download protocol error: {0}")]
    DownloadProtocol(String),
    #[error("ADP download verification evidence is unavailable: {0}")]
    DownloadUnavailable(String),

    #[error("campaign has not started: {0}")]
    ClaimCampaignNotStarted(String),
    #[error("campaign has ended: {0}")]
    ClaimCampaignEnded(String),
    #[error("campaign is cancelled: {0}")]
    ClaimCampaignCancelled(String),
    #[error("campaign is invalid: {0}")]
    ClaimInvalidCampaign(String),
    #[error("claim coordinate mismatch: {0}")]
    ClaimCoordinateMismatch(String),
    #[error("claim response contains an invalid grant: {0}")]
    ClaimInvalidGrantResponse(String),
    #[error("server is not authorized to distribute this game: {0}")]
    ClaimDistributionUnauthorized(String),
    #[error("claim protocol failure: {0}")]
    ClaimProtocol(String),
    #[error("ADP authorization evidence is unavailable: {0}")]
    AuthorizationUnavailable(String),
    #[error("ADP server has no held authorized signer: {0}")]
    NoHeldAuthorizedSigner(String),
    #[error("purchase protocol failure: {0}")]
    PurchaseProtocol(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct AdpEventKinds {
    pub entitlement_grant: u16,
    pub adp_campaign: u16,
    #[serde(default)]
    pub commercial_terms: Option<u16>,
    #[serde(default)]
    pub private_quote: Option<u16>,
    #[serde(default)]
    pub recovery_evidence: Option<u16>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct AdpSettlementInfo {
    pub version: u16,
    pub split_enabled: bool,
    #[serde(default)]
    pub terms_event_id: Option<String>,
    pub fee: crate::adp_settlement::FeePolicy,
    pub legacy_paid_sales: String,
}

/// Public metadata returned by `GET /.well-known/adp`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct AdpServerInfo {
    pub adp_version: String,
    /// Maximum file bytes; absent on legacy servers (unknown, not unlimited).
    #[serde(default)]
    pub max_upload_bytes: Option<u64>,
    pub pubkey: String,
    pub name: Option<String>,
    pub url: Option<String>,
    #[serde(default)]
    pub event_kinds: Option<AdpEventKinds>,
    #[serde(default)]
    pub event_kind_status: Option<String>,
    #[serde(default)]
    pub settlement: Option<AdpSettlementInfo>,
}

/// Response returned by `POST /provision`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct ProvisionResponse {
    pub fulfillment_pubkey: String,
    pub attestation_event_id: String,
    #[serde(default)]
    pub attestation: Option<Event>,
    pub scope: Option<String>,
}

/// Response returned by `POST /provision/revoke`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct RevokeResponse {
    pub fulfillment_pubkey: String,
    pub revoked_at: Option<i64>,
}

/// Placeholder upload response for the Gate 3 implementation.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct UploadResponse {
    pub game_coordinate: String,
    pub file_hash: String,
    pub download_url: String,
}

/// Request sent to `POST /purchase/confirm` in a later gate.
#[derive(Debug, Clone, Serialize)]
pub struct PurchaseConfirmRequest {
    pub game_coordinate: String,
    pub listing_event: Event,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub zap_receipt_event: Option<Event>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bolt11: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preimage: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quote_id: Option<String>,
}

/// Response returned by `POST /purchase/confirm` in a later gate.
#[derive(Debug, Clone, Deserialize)]
pub struct PurchaseConfirmResponse {
    pub receipt: Event,
    pub download_token: String,
    pub token_expires_at: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct CreateSettlementQuoteRequest {
    pub purchase_id: String,
    pub listing_event: Event,
    pub terms_events: Vec<Event>,
    pub refund_destination: crate::adp_settlement::RefundDestination,
    #[serde(default, skip_serializing_if = "std::collections::HashMap::is_empty")]
    pub zap_requests: std::collections::HashMap<String, Event>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SettlementQuoteResponse {
    pub quote: Event,
    pub state: crate::adp_settlement::QuoteState,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SettlementStatusResponse {
    pub quote_id: String,
    pub state: crate::adp_settlement::QuoteState,
    pub total_legs: u64,
    pub disclosed_legs: u64,
    pub settled_legs: u64,
    pub failure_at: Option<u64>,
    pub refund_due_at: Option<u64>,
    pub receipt_event_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SettlementInvoiceResponse {
    pub quote_id: String,
    pub attempt_id: String,
    pub leg_id: String,
    pub amount_msat: u64,
    pub payment_hash: String,
    pub bolt11: String,
    pub expires_at: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct SettlementProofRequest {
    pub attempt_id: String,
    pub preimage: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SettlementProofResponse {
    pub state: String,
    pub settled_leg: Option<String>,
    pub receipt: Option<Event>,
    pub download_token: Option<String>,
    pub token_expires_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ReplaceRefundDestinationRequest {
    pub destination: crate::adp_settlement::RefundDestination,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecoveryPreparationResponse {
    pub claimant: String,
    pub event: Option<Event>,
    pub plaintext: Option<String>,
    pub evidence: crate::adp_settlement::RecoveryEvidence,
}

#[derive(Debug, Clone, Serialize)]
pub struct SubmitRecoveryRequest {
    pub event: Event,
    pub plaintext: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RefundInvoiceResponse {
    pub attempt_id: String,
    pub leg_id: String,
    pub amount_msat: u64,
    pub bolt11: String,
    pub expires_at: u64,
}

/// Request sent to `POST /entitlement/claim`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EntitlementClaimRequest {
    pub game_coordinate: String,
    pub campaign_event_id: String,
}

/// Signed grant and short-lived download authorization returned by a claim.
#[derive(Debug, Clone, Deserialize)]
pub struct EntitlementClaimResponse {
    pub grant: Event,
    pub download_token: String,
    pub token_expires_at: i64,
    #[serde(default)]
    pub already_claimed: bool,
}

/// Download authentication strategy for game archive fetches.
pub enum DownloadAuth<'a> {
    Token(String),
    Nip98 { signer: &'a dyn NostrSigner },
}

/// Result of an ADP download operation in a later gate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DownloadOutcome {
    pub bytes_written: u64,
}

/// Result of a private candidate download whose written handle stays open.
#[derive(Debug, Clone)]
pub struct RetainedDownloadOutcome {
    pub file: Arc<File>,
    pub bytes_written: u64,
}

impl From<HttpRetainedDownload> for RetainedDownloadOutcome {
    fn from(outcome: HttpRetainedDownload) -> Self {
        Self {
            file: outcome.file,
            bytes_written: outcome.bytes_written,
        }
    }
}

const UPLOAD_CHUNK_BYTES: usize = 64 * 1024;

type UploadProgressCallback = Arc<dyn Fn(u64, u64) + Send + Sync>;

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

fn blocking_task_error(error: tokio::task::JoinError) -> io::Error {
    io::Error::other(format!("ADP upload read task failed: {error}"))
}

fn positional_upload_stream(
    file: Arc<File>,
    total_bytes: u64,
    on_progress: UploadProgressCallback,
) -> impl futures::Stream<Item = io::Result<Vec<u8>>> + Send {
    futures::stream::try_unfold(
        (file, 0_u64, on_progress),
        move |(file, uploaded, on_progress)| async move {
            if uploaded == total_bytes {
                return Ok(None);
            }

            let remaining = total_bytes - uploaded;
            let requested = remaining.min(UPLOAD_CHUNK_BYTES as u64) as usize;
            let read_file = Arc::clone(&file);
            let chunk = tokio::task::spawn_blocking(move || {
                let mut chunk = vec![0_u8; requested];
                let read = read_file_at(&read_file, &mut chunk, uploaded)?;
                chunk.truncate(read);
                Ok::<_, io::Error>(chunk)
            })
            .await
            .map_err(blocking_task_error)??;

            if chunk.is_empty() {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    format!("upload file ended after {uploaded} of {total_bytes} advertised bytes"),
                ));
            }

            let uploaded = uploaded + chunk.len() as u64;
            on_progress(uploaded, total_bytes);
            Ok(Some((chunk, (file, uploaded, on_progress))))
        },
    )
}

/// Typed client for one ADP server base URL.
pub struct AdpClient {
    base_url: String,
    http: Arc<dyn HttpClient>,
}

/// NIP-98 authorization bound to one client's upload endpoint.
pub struct UploadAuthorization {
    header: String,
}

impl AdpClient {
    /// Creates an ADP client for `base_url`.
    pub fn new(base_url: impl Into<String>, http: Arc<dyn HttpClient>) -> Self {
        Self {
            base_url: normalize_base_url(base_url.into()),
            http,
        }
    }

    /// Fetches public ADP server metadata.
    ///
    /// # Errors
    /// Returns [`AdpClientError`] if the server cannot be reached or decoded.
    pub async fn well_known(&self) -> Result<AdpServerInfo, AdpClientError> {
        let value = self.http.get_json(&self.url("/.well-known/adp")).await?;
        Ok(serde_json::from_value(value)?)
    }

    /// Requests or reuses a provisioned fulfillment key.
    ///
    /// # Errors
    /// Currently returns [`AdpClientError::NotImplemented`] until Gate 1 POST support lands.
    pub async fn provision(
        &self,
        signer: &dyn NostrSigner,
        scope: Option<&str>,
    ) -> Result<ProvisionResponse, AdpClientError> {
        let url = self.url("/provision");
        let auth_header = build_nip98_auth_header(signer, &url, "POST").await?;
        let body = match scope {
            Some(scope) => serde_json::json!({ "scope": scope }),
            None => serde_json::json!({}),
        };
        let value = self
            .http
            .post_json(&url, body, vec![("Authorization".to_string(), auth_header)])
            .await?;

        Ok(serde_json::from_value(value)?)
    }

    /// Revokes a provisioned fulfillment key on the operator.
    ///
    /// # Errors
    /// Currently returns [`AdpClientError::NotImplemented`] until Gate 1 POST support lands.
    pub async fn provision_revoke(
        &self,
        signer: &dyn NostrSigner,
        fulfillment_pubkey: &str,
    ) -> Result<RevokeResponse, AdpClientError> {
        let url = self.url("/provision/revoke");
        let auth_header = build_nip98_auth_header(signer, &url, "POST").await?;
        let body = serde_json::json!({ "fulfillment_pubkey": fulfillment_pubkey });
        let value = self
            .http
            .post_json(&url, body, vec![("Authorization".to_string(), auth_header)])
            .await?;

        Ok(serde_json::from_value(value)?)
    }

    /// Uploads a build file for an already-published listing.
    ///
    /// This path-based compatibility API opens the path once, then delegates to the handle-based
    /// upload API. The request stream never reopens the path.
    ///
    /// # Errors
    /// Returns an error if the file cannot be read, authentication fails, or the server rejects
    /// the upload.
    pub async fn upload(
        &self,
        signer: &dyn NostrSigner,
        listing_event: &Event,
        file_path: &Path,
    ) -> Result<UploadResponse, AdpClientError> {
        self.upload_with_progress(signer, listing_event, file_path, |_, _| {})
            .await
    }

    /// Uploads a build path and reports file bytes as they are streamed into the request body.
    ///
    /// This path-based compatibility API opens the path once, then delegates to
    /// [`Self::upload_file_with_progress`].
    pub async fn upload_with_progress<F>(
        &self,
        signer: &dyn NostrSigner,
        listing_event: &Event,
        file_path: &Path,
        on_progress: F,
    ) -> Result<UploadResponse, AdpClientError>
    where
        F: Fn(u64, u64) + Send + Sync + 'static,
    {
        let safe_filename = match file_path
            .file_name()
            .and_then(|name| name.to_str())
            .and_then(|name| SafeArchiveFilename::parse(name).ok())
        {
            Some(filename) => filename,
            None => SafeArchiveFilename::parse("game-build")
                .map_err(|error| io::Error::other(error.to_string()))?,
        };
        let path = file_path.to_owned();
        let file = tokio::task::spawn_blocking(move || File::open(path))
            .await
            .map_err(blocking_task_error)??;

        self.upload_file_with_progress(
            signer,
            listing_event,
            Arc::new(file),
            &safe_filename,
            on_progress,
        )
        .await
    }

    /// Uploads an already-open build file without consulting or reopening its source path.
    ///
    /// # Errors
    /// Returns an error if the file cannot be read, authentication fails, or the server rejects
    /// the upload.
    pub async fn upload_file(
        &self,
        signer: &dyn NostrSigner,
        listing_event: &Event,
        file: Arc<File>,
        safe_filename: &SafeArchiveFilename,
    ) -> Result<UploadResponse, AdpClientError> {
        self.upload_file_with_progress(signer, listing_event, file, safe_filename, |_, _| {})
            .await
    }

    /// Uploads an open file with positional reads and reports streamed file bytes.
    ///
    /// The length advertised to multipart is captured from the supplied handle. Streaming stops
    /// at exactly that length and reports [`io::ErrorKind::UnexpectedEof`] if the handle becomes
    /// shorter. Every blocking read uses an explicit offset, independent of any shared cursor.
    pub async fn upload_file_with_progress<F>(
        &self,
        signer: &dyn NostrSigner,
        listing_event: &Event,
        file: Arc<File>,
        safe_filename: &SafeArchiveFilename,
        on_progress: F,
    ) -> Result<UploadResponse, AdpClientError>
    where
        F: Fn(u64, u64) + Send + Sync + 'static,
    {
        let authorization = self.authorize_upload(signer).await?;
        self.upload_authorized_file_with_progress(
            authorization,
            listing_event,
            file,
            safe_filename,
            on_progress,
        )
        .await
    }

    /// Signs NIP-98 authorization for this client's upload endpoint without sending a request.
    pub async fn authorize_upload(
        &self,
        signer: &dyn NostrSigner,
    ) -> Result<UploadAuthorization, AdpClientError> {
        let header = build_nip98_auth_header(signer, &self.url("/upload"), "POST").await?;
        Ok(UploadAuthorization { header })
    }

    /// Uploads from an open file using previously signed endpoint authorization.
    ///
    /// Separating authorization from transmission lets native callers revalidate account state
    /// after a remote signer returns and before the HTTP side effect begins.
    pub async fn upload_authorized_file_with_progress<F>(
        &self,
        authorization: UploadAuthorization,
        listing_event: &Event,
        file: Arc<File>,
        safe_filename: &SafeArchiveFilename,
        on_progress: F,
    ) -> Result<UploadResponse, AdpClientError>
    where
        F: Fn(u64, u64) + Send + Sync + 'static,
    {
        let url = self.url("/upload");
        let listing_json = serde_json::to_string(listing_event)?;
        let metadata_file = Arc::clone(&file);
        let total_bytes = tokio::task::spawn_blocking(move || metadata_file.metadata())
            .await
            .map_err(blocking_task_error)??
            .len();
        let on_progress: UploadProgressCallback = Arc::new(on_progress);
        on_progress(0, total_bytes);
        let stream = positional_upload_stream(file, total_bytes, on_progress);
        let file_part = reqwest::multipart::Part::stream_with_length(
            reqwest::Body::wrap_stream(stream),
            total_bytes,
        )
        .file_name(safe_filename.as_str().to_owned());
        let form = reqwest::multipart::Form::new()
            .text("listing_event", listing_json)
            .part("file", file_part);

        let response = reqwest::Client::new()
            .post(url)
            .header("Authorization", authorization.header)
            .multipart(form)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status().as_u16();
            let body = response
                .text()
                .await
                .unwrap_or_else(|err| format!("<failed to read error body: {err}>"));
            return Err(AdpClientError::Http(HttpClientError::StatusWithBody {
                status,
                body,
            }));
        }

        Ok(response.json::<UploadResponse>().await?)
    }

    /// Confirms a purchase and returns receipt/token data.
    ///
    /// # Errors
    /// Returns an error if authentication, HTTP transport, or response decoding fails.
    pub async fn purchase_confirm(
        &self,
        signer: &dyn NostrSigner,
        req: PurchaseConfirmRequest,
    ) -> Result<PurchaseConfirmResponse, AdpClientError> {
        let url = self.url("/purchase/confirm");
        let auth_header = build_nip98_auth_header(signer, &url, "POST").await?;
        let body = serde_json::to_value(req)?;
        let value = self
            .http
            .post_json(&url, body, vec![("Authorization".to_string(), auth_header)])
            .await
            .map_err(map_purchase_error)?;

        Ok(serde_json::from_value(value)?)
    }

    pub async fn create_settlement_quote(
        &self,
        signer: &dyn NostrSigner,
        req: CreateSettlementQuoteRequest,
    ) -> Result<SettlementQuoteResponse, AdpClientError> {
        let url = self.url("/settlement/quote");
        let auth = build_nip98_auth_header(signer, &url, "POST").await?;
        let value = self
            .http
            .post_json(
                &url,
                serde_json::to_value(req)?,
                vec![("Authorization".into(), auth)],
            )
            .await
            .map_err(map_purchase_error)?;
        Ok(serde_json::from_value(value)?)
    }

    pub async fn settlement_status(
        &self,
        signer: &dyn NostrSigner,
        quote_id: &str,
    ) -> Result<SettlementStatusResponse, AdpClientError> {
        let url = self.url(&format!("/settlement/{quote_id}/status"));
        let auth = build_nip98_auth_header(signer, &url, "POST").await?;
        let value = self
            .http
            .post_json(
                &url,
                serde_json::json!({}),
                vec![("Authorization".into(), auth)],
            )
            .await
            .map_err(map_purchase_error)?;
        Ok(serde_json::from_value(value)?)
    }

    pub async fn settlement_next_invoice(
        &self,
        signer: &dyn NostrSigner,
        quote_id: &str,
    ) -> Result<SettlementInvoiceResponse, AdpClientError> {
        let url = self.url(&format!("/settlement/{quote_id}/next"));
        let auth = build_nip98_auth_header(signer, &url, "POST").await?;
        let value = self
            .http
            .post_json(
                &url,
                serde_json::json!({}),
                vec![("Authorization".into(), auth)],
            )
            .await
            .map_err(map_purchase_error)?;
        Ok(serde_json::from_value(value)?)
    }

    pub async fn submit_settlement_proof(
        &self,
        signer: &dyn NostrSigner,
        quote_id: &str,
        req: SettlementProofRequest,
    ) -> Result<SettlementProofResponse, AdpClientError> {
        let url = self.url(&format!("/settlement/{quote_id}/proof"));
        let auth = build_nip98_auth_header(signer, &url, "POST").await?;
        let value = self
            .http
            .post_json(
                &url,
                serde_json::to_value(req)?,
                vec![("Authorization".into(), auth)],
            )
            .await
            .map_err(map_purchase_error)?;
        Ok(serde_json::from_value(value)?)
    }

    pub async fn replace_refund_destination(
        &self,
        signer: &dyn NostrSigner,
        quote_id: &str,
        destination: crate::adp_settlement::RefundDestination,
    ) -> Result<serde_json::Value, AdpClientError> {
        let url = self.url(&format!("/settlement/{quote_id}/refund-destination"));
        let auth = build_nip98_auth_header(signer, &url, "POST").await?;
        self.http
            .post_json(
                &url,
                serde_json::to_value(ReplaceRefundDestinationRequest { destination })?,
                vec![("Authorization".into(), auth)],
            )
            .await
            .map_err(map_purchase_error)
    }

    pub async fn prepare_recovery(
        &self,
        signer: &dyn NostrSigner,
        quote_id: &str,
    ) -> Result<RecoveryPreparationResponse, AdpClientError> {
        let url = self.url(&format!("/settlement/{quote_id}/recovery"));
        let auth = build_nip98_auth_header(signer, &url, "POST").await?;
        let value = self
            .http
            .post_json(
                &url,
                serde_json::json!({}),
                vec![("Authorization".into(), auth)],
            )
            .await
            .map_err(map_purchase_error)?;
        Ok(serde_json::from_value(value)?)
    }

    pub async fn submit_recovery(
        &self,
        signer: &dyn NostrSigner,
        quote_id: &str,
        event: Event,
        plaintext: String,
    ) -> Result<serde_json::Value, AdpClientError> {
        let url = self.url(&format!("/settlement/{quote_id}/recovery/submit"));
        let auth = build_nip98_auth_header(signer, &url, "POST").await?;
        self.http
            .post_json(
                &url,
                serde_json::to_value(SubmitRecoveryRequest { event, plaintext })?,
                vec![("Authorization".into(), auth)],
            )
            .await
            .map_err(map_purchase_error)
    }

    pub async fn prepare_refund(
        &self,
        signer: &dyn NostrSigner,
        quote_id: &str,
        leg_id: &str,
    ) -> Result<RefundInvoiceResponse, AdpClientError> {
        let url = self.url(&format!("/settlement/{quote_id}/refund/{leg_id}/prepare"));
        let auth = build_nip98_auth_header(signer, &url, "POST").await?;
        let value = self
            .http
            .post_json(
                &url,
                serde_json::json!({}),
                vec![("Authorization".into(), auth)],
            )
            .await
            .map_err(map_purchase_error)?;
        Ok(serde_json::from_value(value)?)
    }

    pub async fn confirm_refund(
        &self,
        signer: &dyn NostrSigner,
        quote_id: &str,
        leg_id: &str,
        attempt_id: &str,
        preimage: &str,
    ) -> Result<serde_json::Value, AdpClientError> {
        let url = self.url(&format!("/settlement/{quote_id}/refund/{leg_id}/confirm"));
        let auth = build_nip98_auth_header(signer, &url, "POST").await?;
        self.http
            .post_json(
                &url,
                serde_json::json!({ "attempt_id": attempt_id, "preimage": preimage }),
                vec![("Authorization".into(), auth)],
            )
            .await
            .map_err(map_purchase_error)
    }

    /// Claims an active publisher campaign using NIP-98 bound to this exact endpoint.
    pub async fn entitlement_claim(
        &self,
        signer: &dyn NostrSigner,
        request: EntitlementClaimRequest,
    ) -> Result<EntitlementClaimResponse, AdpClientError> {
        let url = self.url("/entitlement/claim");
        let auth_header = build_nip98_auth_header(signer, &url, "POST").await?;
        let body = serde_json::to_value(request)?;
        let value = self
            .http
            .post_json(&url, body, vec![("Authorization".to_string(), auth_header)])
            .await
            .map_err(map_claim_error)?;
        serde_json::from_value(value)
            .map_err(|error| AdpClientError::ClaimInvalidGrantResponse(error.to_string()))
    }

    /// Downloads a game archive to `dest`.
    ///
    /// Builds either a token-authenticated download URL or a NIP-98
    /// `Authorization` header before streaming the response to disk.
    ///
    /// # Errors
    /// Returns [`AdpClientError::Auth`] when NIP-98 authentication cannot be
    /// built, [`AdpClientError::DownloadOwnership`] for HTTP 403 responses,
    /// [`AdpClientError::DownloadDistribution`] for HTTP 451 responses,
    /// [`AdpClientError::DownloadProtocol`] for other unsuccessful download
    /// statuses, or [`AdpClientError::Http`] when the HTTP stream or local file
    /// write fails.
    pub async fn download(
        &self,
        game_coordinate: &str,
        auth: DownloadAuth<'_>,
        dest: &Path,
        mut on_progress: impl FnMut(u64, Option<u64>) + Send,
    ) -> Result<DownloadOutcome, AdpClientError> {
        let encoded_coordinate = urlencoding::encode(game_coordinate);
        let (url, headers) = match auth {
            DownloadAuth::Token(token) => {
                let encoded_token = urlencoding::encode(&token);
                (
                    self.url(&format!("/game/{encoded_coordinate}?token={encoded_token}")),
                    Vec::new(),
                )
            }
            DownloadAuth::Nip98 { signer } => {
                let url = self.url(&format!("/game/{encoded_coordinate}"));
                let auth_header = build_nip98_auth_header(signer, &url, "GET").await?;
                (url, vec![("Authorization".to_string(), auth_header)])
            }
        };

        let outcome = self
            .http
            .download_to_path(&url, headers, dest, &mut on_progress)
            .await
            .map_err(map_download_error)?;

        Ok(DownloadOutcome {
            bytes_written: outcome.bytes_written,
        })
    }

    /// Downloads into an exclusively created private candidate and retains the
    /// exact open handle that received the response bytes.
    pub async fn download_retained(
        &self,
        game_coordinate: &str,
        auth: DownloadAuth<'_>,
        candidate: &Path,
        on_progress: impl FnMut(u64, Option<u64>) + Send,
    ) -> Result<RetainedDownloadOutcome, AdpClientError> {
        self.download_retained_with_preflight(
            game_coordinate,
            auth,
            candidate,
            |_| Ok(()),
            on_progress,
        )
        .await
    }

    /// Downloads into a retained candidate after allowing the caller to reject
    /// the successful response's optional content length. The preflight runs
    /// before candidate creation and response-body polling.
    pub async fn download_retained_with_preflight(
        &self,
        game_coordinate: &str,
        auth: DownloadAuth<'_>,
        candidate: &Path,
        mut on_preflight: impl FnMut(Option<u64>) -> Result<(), HttpClientError> + Send,
        mut on_progress: impl FnMut(u64, Option<u64>) + Send,
    ) -> Result<RetainedDownloadOutcome, AdpClientError> {
        let encoded_coordinate = urlencoding::encode(game_coordinate);
        let (url, headers) = match auth {
            DownloadAuth::Token(token) => {
                let encoded_token = urlencoding::encode(&token);
                (
                    self.url(&format!("/game/{encoded_coordinate}?token={encoded_token}")),
                    Vec::new(),
                )
            }
            DownloadAuth::Nip98 { signer } => {
                let url = self.url(&format!("/game/{encoded_coordinate}"));
                let auth_header = build_nip98_auth_header(signer, &url, "GET").await?;
                (url, vec![("Authorization".to_string(), auth_header)])
            }
        };

        self.http
            .download_to_retained_file_with_preflight(
                &url,
                headers,
                candidate,
                &mut on_preflight,
                &mut on_progress,
            )
            .await
            .map(RetainedDownloadOutcome::from)
            .map_err(map_download_error)
    }

    /// Downloads into a caller-created capability-owned file.
    pub async fn download_retained_to_open_file_with_preflight(
        &self,
        game_coordinate: &str,
        auth: DownloadAuth<'_>,
        candidate: File,
        mut on_preflight: impl FnMut(Option<u64>) -> Result<(), HttpClientError> + Send,
        mut on_progress: impl FnMut(u64, Option<u64>) + Send,
    ) -> Result<RetainedDownloadOutcome, AdpClientError> {
        let encoded_coordinate = urlencoding::encode(game_coordinate);
        let (url, headers) = match auth {
            DownloadAuth::Token(token) => {
                let encoded_token = urlencoding::encode(&token);
                (
                    self.url(&format!("/game/{encoded_coordinate}?token={encoded_token}")),
                    Vec::new(),
                )
            }
            DownloadAuth::Nip98 { signer } => {
                let url = self.url(&format!("/game/{encoded_coordinate}"));
                let auth_header = build_nip98_auth_header(signer, &url, "GET").await?;
                (url, vec![("Authorization".to_string(), auth_header)])
            }
        };
        self.http
            .download_to_open_retained_file_with_preflight(
                &url,
                headers,
                candidate,
                &mut on_preflight,
                &mut on_progress,
            )
            .await
            .map(RetainedDownloadOutcome::from)
            .map_err(map_download_error)
    }

    fn url(&self, path: &str) -> String {
        format!("{}{}", self.base_url, path)
    }
}

fn map_download_error(err: HttpClientError) -> AdpClientError {
    match err {
        HttpClientError::Status(413) | HttpClientError::StatusWithBody { status: 413, .. } => {
            AdpClientError::Http(err)
        }
        HttpClientError::Status(403) => {
            AdpClientError::DownloadOwnership("ownership check failed".to_string())
        }
        HttpClientError::StatusWithBody { status: 403, body } => {
            AdpClientError::DownloadOwnership(body)
        }
        HttpClientError::Status(451) => AdpClientError::DownloadDistribution(
            "server is not authorized to distribute this file".to_string(),
        ),
        HttpClientError::StatusWithBody { status: 451, body } => {
            AdpClientError::DownloadDistribution(body)
        }
        HttpClientError::Status(503) => {
            AdpClientError::DownloadUnavailable("relay evidence unavailable".into())
        }
        HttpClientError::StatusWithBody { status: 503, body } => {
            AdpClientError::DownloadUnavailable(body)
        }
        HttpClientError::Status(status) => {
            AdpClientError::DownloadProtocol(format!("HTTP status {status}"))
        }
        HttpClientError::StatusWithBody { status, body } => {
            AdpClientError::DownloadProtocol(format!("HTTP status {status}: {body}"))
        }
        other => AdpClientError::Http(other),
    }
}

fn map_claim_error(error: HttpClientError) -> AdpClientError {
    let (status, body) = match error {
        HttpClientError::Status(status) => (status, format!("HTTP status {status}")),
        HttpClientError::StatusWithBody { status, body } => (status, body),
        other => return AdpClientError::Http(other),
    };
    let normalized = body.to_ascii_lowercase();
    match status {
        400 if normalized.contains("coordinate") => AdpClientError::ClaimCoordinateMismatch(body),
        400 => AdpClientError::ClaimInvalidCampaign(body),
        403 if normalized.contains("not started") => AdpClientError::ClaimCampaignNotStarted(body),
        403 if normalized.contains("ended") || normalized.contains("expired") => {
            AdpClientError::ClaimCampaignEnded(body)
        }
        403 if normalized.contains("cancel") => AdpClientError::ClaimCampaignCancelled(body),
        403 => AdpClientError::ClaimInvalidCampaign(body),
        451 => AdpClientError::ClaimDistributionUnauthorized(body),
        500 if normalized.contains("no held key") || normalized.contains("authorized signer") => {
            AdpClientError::NoHeldAuthorizedSigner(body)
        }
        503 => AdpClientError::AuthorizationUnavailable(body),
        _ => AdpClientError::ClaimProtocol(format!("HTTP status {status}: {body}")),
    }
}

fn map_purchase_error(error: HttpClientError) -> AdpClientError {
    let (status, body) = match error {
        HttpClientError::Status(status) => (status, format!("HTTP status {status}")),
        HttpClientError::StatusWithBody { status, body } => (status, body),
        other => return AdpClientError::Http(other),
    };
    let normalized = body.to_ascii_lowercase();
    match status {
        451 => AdpClientError::DownloadDistribution(body),
        500 if normalized.contains("no held key") || normalized.contains("authorized signer") => {
            AdpClientError::NoHeldAuthorizedSigner(body)
        }
        503 => AdpClientError::AuthorizationUnavailable(body),
        _ => AdpClientError::PurchaseProtocol(format!("HTTP status {status}: {body}")),
    }
}

fn normalize_base_url(mut base_url: String) -> String {
    while base_url.ends_with('/') {
        base_url.pop();
    }
    base_url
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    use base64::Engine;
    use serde_json::json;
    use serde_json::Value;

    use crate::signers::LocalSigner;
    use crate::test_helpers::http_mocks::MockHttpClient;

    #[tokio::test]
    async fn well_known_fetches_server_info_from_normalized_base_url() {
        let http = Arc::new(MockHttpClient::new().with_json_response(
            "https://dist.example.com/.well-known/adp",
            json!({
                "adp_version": "0.2.0",
                "pubkey": "operator-pubkey",
                "name": "Arcadestr Official Distribution",
                "url": "https://dist.example.com",
                "event_kinds": { "entitlement_grant": 1030, "adp_campaign": 1031 },
                "event_kind_status": "provisional"
            }),
        ));
        let client = AdpClient::new("https://dist.example.com/", http.clone());

        let info = client.well_known().await.expect("server info should load");

        assert_eq!(info.adp_version, "0.2.0");
        assert_eq!(info.pubkey, "operator-pubkey");
        assert_eq!(
            info.name.as_deref(),
            Some("Arcadestr Official Distribution")
        );
        assert_eq!(info.url.as_deref(), Some("https://dist.example.com"));
        assert_eq!(info.event_kind_status.as_deref(), Some("provisional"));
        assert_eq!(
            info.event_kinds
                .as_ref()
                .map(|kinds| kinds.entitlement_grant),
            Some(1030)
        );
        assert_eq!(
            http.call_count("https://dist.example.com/.well-known/adp"),
            1
        );
    }

    #[test]
    fn upload_capacity_metadata_is_optional_and_exact() {
        let mut value = json!({"adp_version":"0.2.0", "pubkey":"operator"});
        assert_eq!(
            serde_json::from_value::<AdpServerInfo>(value.clone())
                .expect("legacy")
                .max_upload_bytes,
            None
        );
        value["max_upload_bytes"] = json!(8_u64 * 1024 * 1024 * 1024);
        assert_eq!(
            serde_json::from_value::<AdpServerInfo>(value.clone())
                .expect("large limit")
                .max_upload_bytes,
            Some(8_u64 * 1024 * 1024 * 1024)
        );
        value["max_upload_bytes"] = json!(-1);
        assert!(serde_json::from_value::<AdpServerInfo>(value).is_err());
    }

    #[tokio::test]
    async fn download_with_token_streams_file_and_reports_progress() {
        let expected_bytes = b"downloaded artifact bytes";
        let token = "download-token";
        let coordinate = "30406:publisher:test-game";
        let http = Arc::new(MockHttpClient::new().with_download_response(
            "https://dist.example.com/game/30406%3Apublisher%3Atest-game?token=download-token",
            expected_bytes.as_slice(),
        ));
        let client = AdpClient::new("https://dist.example.com", http);
        let dest = temp_download_path("token-streams");
        let mut progress_events = Vec::new();

        let outcome = client
            .download(
                coordinate,
                DownloadAuth::Token(token.to_string()),
                &dest,
                |bytes, total| progress_events.push((bytes, total)),
            )
            .await
            .expect("download should stream to disk");

        assert_eq!(
            std::fs::read(&dest).expect("downloaded file should exist"),
            expected_bytes
        );
        assert_eq!(outcome.bytes_written, expected_bytes.len() as u64);
        assert!(progress_events
            .iter()
            .any(|event| event.0 == expected_bytes.len() as u64));

        let _ = std::fs::remove_file(dest);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn retained_token_download_handle_survives_candidate_replacement() {
        use std::os::unix::fs::FileExt;

        let expected_bytes = b"retained artifact bytes";
        let url =
            "https://dist.example.com/game/30406%3Apublisher%3Atest-game?token=download-token";
        let http = Arc::new(MockHttpClient::new().with_download_response(url, expected_bytes));
        let client = AdpClient::new("https://dist.example.com", http.clone());
        let candidate = temp_download_path("retained-token");
        let moved = temp_download_path("retained-token-moved");
        let mut progress_events = Vec::new();

        let outcome = client
            .download_retained(
                "30406:publisher:test-game",
                DownloadAuth::Token("download-token".to_string()),
                &candidate,
                |bytes, total| progress_events.push((bytes, total)),
            )
            .await
            .expect("retained download should succeed");

        std::fs::rename(&candidate, &moved).expect("candidate should be renamed");
        std::fs::write(&candidate, b"replacement path contents")
            .expect("candidate path should be replaced");
        let mut retained_bytes = vec![0_u8; expected_bytes.len()];
        let read = outcome
            .file
            .read_at(&mut retained_bytes, 0)
            .expect("retained handle should remain readable");

        assert_eq!(read, expected_bytes.len());
        assert_eq!(retained_bytes, expected_bytes);
        assert_eq!(outcome.bytes_written, expected_bytes.len() as u64);
        assert_eq!(http.call_count(url), 1);
        assert!(progress_events
            .iter()
            .any(|event| event.0 == expected_bytes.len() as u64));

        let _ = std::fs::remove_file(candidate);
        let _ = std::fs::remove_file(moved);
    }

    #[tokio::test]
    async fn retained_mock_download_preserves_an_existing_candidate() {
        let url =
            "https://dist.example.com/game/30406%3Apublisher%3Atest-game?token=download-token";
        let http = Arc::new(MockHttpClient::new().with_download_response(url, b"new bytes"));
        let client = AdpClient::new("https://dist.example.com", http.clone());
        let candidate = temp_download_path("retained-existing");
        std::fs::write(&candidate, b"existing bytes").expect("candidate should be seeded");

        client
            .download_retained(
                "30406:publisher:test-game",
                DownloadAuth::Token("download-token".to_string()),
                &candidate,
                |_bytes, _total| {},
            )
            .await
            .expect_err("existing candidate should be rejected");

        assert_eq!(
            std::fs::read(&candidate).expect("candidate should remain readable"),
            b"existing bytes"
        );
        assert_eq!(http.call_count(url), 1);

        let _ = std::fs::remove_file(candidate);
    }

    #[tokio::test]
    async fn retained_preflight_rejection_creates_no_candidate_or_body_write() {
        let url =
            "https://dist.example.com/game/30406%3Apublisher%3Atest-game?token=download-token";
        let expected = b"body must not be written";
        let http = Arc::new(MockHttpClient::new().with_download_response(url, expected));
        let client = AdpClient::new("https://dist.example.com", http.clone());
        let candidate = temp_download_path("retained-preflight-rejection");
        let mut discovered_length = None;
        let mut progress_called = false;

        let error = client
            .download_retained_with_preflight(
                "30406:publisher:test-game",
                DownloadAuth::Token("download-token".to_string()),
                &candidate,
                |length| {
                    discovered_length = length;
                    Err(HttpClientError::InsufficientStorage(
                        "preflight capacity check failed".to_string(),
                    ))
                },
                |_bytes, _total| progress_called = true,
            )
            .await
            .expect_err("preflight should reject the retained download");

        assert!(matches!(
            error,
            AdpClientError::Http(HttpClientError::InsufficientStorage(_))
        ));
        assert_eq!(discovered_length, Some(expected.len() as u64));
        assert!(!candidate.exists());
        assert!(!progress_called);
        assert_eq!(http.download_body_write_count(url), 0);
        assert_eq!(http.call_count(url), 1);
    }

    #[tokio::test]
    async fn download_percent_encodes_coordinate_and_token() {
        let coordinate = "30406:publisher/game name";
        let token = "token with spaces&scope=bad";
        let expected_url = "https://dist.example.com/game/30406%3Apublisher%2Fgame%20name?token=token%20with%20spaces%26scope%3Dbad";
        let http = Arc::new(
            MockHttpClient::new().with_download_response(expected_url, b"encoded download"),
        );
        let client = AdpClient::new("https://dist.example.com", http.clone());
        let dest = temp_download_path("encoded-url");

        client
            .download(
                coordinate,
                DownloadAuth::Token(token.to_string()),
                &dest,
                |_bytes, _total| {},
            )
            .await
            .expect("download should encode URL components");

        assert_eq!(http.call_count(expected_url), 1);
        assert_eq!(http.last_requested_url().as_deref(), Some(expected_url));

        let _ = std::fs::remove_file(dest);
    }

    #[tokio::test]
    async fn download_with_nip98_sets_authorization_header() {
        let coordinate = "30406:publisher:test-game";
        let url = "https://dist.example.com/game/30406%3Apublisher%3Atest-game";
        let http = Arc::new(MockHttpClient::new().with_download_response(url, b"nip98 bytes"));
        let client = AdpClient::new("https://dist.example.com", http.clone());
        let signer = LocalSigner::from_hex(
            "0000000000000000000000000000000000000000000000000000000000000001",
        )
        .expect("test signer should be valid");
        let dest = temp_download_path("nip98-auth");

        let _outcome = client
            .download(
                coordinate,
                DownloadAuth::Nip98 { signer: &signer },
                &dest,
                |_bytes, _total| {},
            )
            .await
            .expect("download should use NIP-98 auth");

        let headers = http
            .last_download_headers(url)
            .expect("download headers should be captured");
        let auth = headers
            .iter()
            .find(|(name, _)| name == "Authorization")
            .map(|(_, value)| value.as_str())
            .expect("Authorization header should be set");
        assert!(auth.starts_with("Nostr "));
        let encoded = auth
            .strip_prefix("Nostr ")
            .expect("Authorization should use Nostr scheme");
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(encoded)
            .expect("Authorization should contain base64 event JSON");
        let event: Value = serde_json::from_slice(&decoded).expect("event JSON should decode");
        assert_eq!(
            event["tags"],
            json!([
                [
                    "u",
                    "https://dist.example.com/game/30406%3Apublisher%3Atest-game"
                ],
                ["method", "GET"]
            ])
        );
        assert_eq!(http.call_count(url), 1);
        assert_eq!(http.last_requested_url().as_deref(), Some(url));

        let _ = std::fs::remove_file(dest);
    }

    #[tokio::test]
    async fn retained_download_with_nip98_sets_authorization_header() {
        let coordinate = "30406:publisher:test-game";
        let url = "https://dist.example.com/game/30406%3Apublisher%3Atest-game";
        let http = Arc::new(MockHttpClient::new().with_download_response(url, b"retained nip98"));
        let client = AdpClient::new("https://dist.example.com", http.clone());
        let signer = LocalSigner::from_hex(
            "0000000000000000000000000000000000000000000000000000000000000001",
        )
        .expect("test signer should be valid");
        let candidate = temp_download_path("retained-nip98-auth");

        let outcome = client
            .download_retained(
                coordinate,
                DownloadAuth::Nip98 { signer: &signer },
                &candidate,
                |_bytes, _total| {},
            )
            .await
            .expect("retained download should use NIP-98 auth");

        let headers = http
            .last_download_headers(url)
            .expect("download headers should be captured");
        let auth = headers
            .iter()
            .find(|(name, _)| name == "Authorization")
            .map(|(_, value)| value.as_str())
            .expect("Authorization header should be set");
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(
                auth.strip_prefix("Nostr ")
                    .expect("Authorization should use Nostr scheme"),
            )
            .expect("Authorization should contain base64 event JSON");
        let event: Value = serde_json::from_slice(&decoded).expect("event JSON should decode");
        assert_eq!(
            event["tags"],
            json!([
                [
                    "u",
                    "https://dist.example.com/game/30406%3Apublisher%3Atest-game"
                ],
                ["method", "GET"]
            ])
        );
        assert_eq!(outcome.bytes_written, b"retained nip98".len() as u64);
        assert_eq!(http.call_count(url), 1);

        let _ = std::fs::remove_file(candidate);
    }

    #[test]
    fn download_archive_policy_preserves_typed_size_errors() {
        assert!(matches!(
            map_download_error(HttpClientError::Status(413)),
            AdpClientError::Http(HttpClientError::Status(413))
        ));
        assert!(matches!(
            map_download_error(HttpClientError::StatusWithBody {
                status: 413,
                body: "oversized".into(),
            }),
            AdpClientError::Http(HttpClientError::StatusWithBody { status: 413, .. })
        ));
        assert!(matches!(
            map_download_error(HttpClientError::FilesystemFileSizeLimit("limit".into())),
            AdpClientError::Http(HttpClientError::FilesystemFileSizeLimit(_))
        ));
        assert!(matches!(
            map_download_error(HttpClientError::Request("HTTP 413".into())),
            AdpClientError::Http(HttpClientError::Request(_))
        ));
        assert!(matches!(
            map_download_error(HttpClientError::Status(503)),
            AdpClientError::DownloadUnavailable(_)
        ));
    }

    #[tokio::test]
    async fn download_403_returns_ownership_error() {
        let coordinate = "30406:publisher:test-game";
        let url = "https://dist.example.com/game/30406%3Apublisher%3Atest-game?token=expired-token";
        let http = Arc::new(MockHttpClient::new().with_download_error(
            url,
            HttpClientError::StatusWithBody {
                status: 403,
                body: "ownership check failed".to_string(),
            },
        ));
        let client = AdpClient::new("https://dist.example.com", http);
        let dest = temp_download_path("ownership-error");

        let err = client
            .download(
                coordinate,
                DownloadAuth::Token("expired-token".to_string()),
                &dest,
                |_bytes, _total| {},
            )
            .await
            .expect_err("403 should map to ownership/auth failure");

        assert!(err.to_string().contains("ownership/auth failure"));
        let _ = std::fs::remove_file(dest);
    }

    #[tokio::test]
    async fn download_451_returns_distribution_error() {
        let coordinate = "30406:publisher:test-game";
        let url = "https://dist.example.com/game/30406%3Apublisher%3Atest-game?token=valid-token";
        let http = Arc::new(MockHttpClient::new().with_download_error(
            url,
            HttpClientError::StatusWithBody {
                status: 451,
                body: "server not authorized".to_string(),
            },
        ));
        let client = AdpClient::new("https://dist.example.com", http);
        let dest = temp_download_path("distribution-error");

        let err = client
            .download(
                coordinate,
                DownloadAuth::Token("valid-token".to_string()),
                &dest,
                |_bytes, _total| {},
            )
            .await
            .expect_err("451 should map to distribution authorization failure");

        assert!(
            err.to_string().contains("no longer distributes")
                || err.to_string().contains("authorized")
        );
        let _ = std::fs::remove_file(dest);
    }

    fn temp_download_path(label: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "arcadestr-adp-{label}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system clock should be after unix epoch")
                .as_nanos()
        ))
    }

    #[tokio::test]
    async fn provision_posts_scope_with_nip98_authorization_and_decodes_response() {
        let http = Arc::new(MockHttpClient::new().with_json_post_response(
            "https://dist.example.com/provision",
            json!({
                "fulfillment_pubkey": "fulfillment-key",
                "attestation_event_id": "attestation-id",
                "scope": "my-game"
            }),
        ));
        let signer = crate::signers::LocalSigner::from_hex(
            "0000000000000000000000000000000000000000000000000000000000000001",
        )
        .expect("test private key should be valid");
        let client = AdpClient::new("https://dist.example.com", http.clone());

        let response = client
            .provision(&signer, Some("my-game"))
            .await
            .expect("provision should succeed");

        assert_eq!(response.fulfillment_pubkey, "fulfillment-key");
        assert_eq!(response.attestation_event_id, "attestation-id");
        assert_eq!(response.scope.as_deref(), Some("my-game"));
        assert_eq!(
            http.post_call_count("https://dist.example.com/provision"),
            1
        );
        assert_eq!(
            http.last_json_post_body("https://dist.example.com/provision"),
            Some(json!({ "scope": "my-game" }))
        );
        let headers = http
            .last_json_post_headers("https://dist.example.com/provision")
            .expect("headers should be recorded");
        assert!(headers
            .iter()
            .any(|(name, value)| { name == "Authorization" && value.starts_with("Nostr ") }));
    }

    #[tokio::test]
    async fn provision_revoke_posts_fulfillment_pubkey_with_nip98_authorization() {
        let http = Arc::new(MockHttpClient::new().with_json_post_response(
            "https://dist.example.com/provision/revoke",
            json!({
                "fulfillment_pubkey": "fulfillment-key",
                "revoked_at": 1_725_000_000
            }),
        ));
        let signer = crate::signers::LocalSigner::from_hex(
            "0000000000000000000000000000000000000000000000000000000000000001",
        )
        .expect("test private key should be valid");
        let client = AdpClient::new("https://dist.example.com", http.clone());

        let response = client
            .provision_revoke(&signer, "fulfillment-key")
            .await
            .expect("revoke should succeed");

        assert_eq!(response.fulfillment_pubkey, "fulfillment-key");
        assert_eq!(response.revoked_at, Some(1_725_000_000));
        assert_eq!(
            http.last_json_post_body("https://dist.example.com/provision/revoke"),
            Some(json!({ "fulfillment_pubkey": "fulfillment-key" }))
        );
        let headers = http
            .last_json_post_headers("https://dist.example.com/provision/revoke")
            .expect("headers should be recorded");
        assert!(headers
            .iter()
            .any(|(name, value)| { name == "Authorization" && value.starts_with("Nostr ") }));
    }

    #[tokio::test]
    async fn purchase_confirm_posts_payment_proof_with_nip98_authorization() {
        let buyer = crate::signers::LocalSigner::from_hex(
            "0000000000000000000000000000000000000000000000000000000000000001",
        )
        .expect("test private key should be valid");
        let merchant_keys = nostr::Keys::generate();
        let listing_event = nostr::EventBuilder::new(nostr::Kind::Custom(30402), "listing")
            .tag(nostr::Tag::custom(nostr::TagKind::d(), ["game-1"]))
            .sign_with_keys(&merchant_keys)
            .unwrap();
        let receipt_event = nostr::EventBuilder::new(nostr::Kind::Custom(1020), "receipt")
            .sign_with_keys(&merchant_keys)
            .unwrap();
        let http = Arc::new(MockHttpClient::new().with_json_post_response(
            "https://dist.example.com/purchase/confirm",
            json!({
                "receipt": receipt_event,
                "download_token": "download-token-1",
                "token_expires_at": 1_800_000_000i64
            }),
        ));
        let client = AdpClient::new("https://dist.example.com", http.clone());

        let response = client
            .purchase_confirm(
                &buyer,
                PurchaseConfirmRequest {
                    game_coordinate: "30402:merchant:game-1".to_string(),
                    listing_event: listing_event.clone(),
                    zap_receipt_event: None,
                    bolt11: Some("lnbc1fixedamountinvoice".to_string()),
                    preimage: Some("feedface".to_string()),
                    quote_id: None,
                },
            )
            .await
            .expect("purchase confirm should succeed");

        assert_eq!(response.download_token, "download-token-1");
        assert_eq!(response.token_expires_at, 1_800_000_000);
        assert_eq!(response.receipt.id, receipt_event.id);
        assert_eq!(
            http.last_json_post_body("https://dist.example.com/purchase/confirm"),
            Some(json!({
                "game_coordinate": "30402:merchant:game-1",
                "listing_event": listing_event,
                "bolt11": "lnbc1fixedamountinvoice",
                "preimage": "feedface"
            }))
        );
        let headers = http
            .last_json_post_headers("https://dist.example.com/purchase/confirm")
            .expect("headers should be recorded");
        assert!(headers
            .iter()
            .any(|(name, value)| { name == "Authorization" && value.starts_with("Nostr ") }));
    }

    #[tokio::test]
    #[ignore = "requires ADP_TEST_SERVER_URL pointing at a live adp-server"]
    async fn live_well_known_and_provision_round_trip_is_idempotent() {
        let server_url = std::env::var("ADP_TEST_SERVER_URL")
            .expect("ADP_TEST_SERVER_URL must point at a live adp-server");
        let http = Arc::new(
            crate::http_client::ReqwestHttpClient::new(std::time::Duration::from_secs(10))
                .expect("reqwest client should build"),
        );
        let client = AdpClient::new(server_url, http);
        let signer = crate::signers::LocalSigner::from_hex(
            "0000000000000000000000000000000000000000000000000000000000000001",
        )
        .expect("test private key should be valid");
        let scope = format!(
            "gate1-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system clock should be after unix epoch")
                .as_nanos()
        );

        let info = client.well_known().await.expect("well-known should load");
        assert!(!info.adp_version.is_empty());
        assert!(!info.pubkey.is_empty());

        let first = client
            .provision(&signer, Some(&scope))
            .await
            .expect("first provision should succeed");
        let second = client
            .provision(&signer, Some(&scope))
            .await
            .expect("second provision should succeed");

        assert!(!first.fulfillment_pubkey.is_empty());
        assert!(!first.attestation_event_id.is_empty());
        assert_eq!(first.scope.as_deref(), Some(scope.as_str()));
        assert_eq!(second.scope.as_deref(), Some(scope.as_str()));
        assert_eq!(first.fulfillment_pubkey, second.fulfillment_pubkey);
    }

    async fn collect_positional_upload(
        file: Arc<File>,
        advertised_bytes: u64,
    ) -> io::Result<Vec<u8>> {
        use futures::TryStreamExt;

        positional_upload_stream(file, advertised_bytes, Arc::new(|_, _| {}))
            .try_fold(Vec::new(), |mut collected, chunk| async move {
                collected.extend_from_slice(&chunk);
                Ok(collected)
            })
            .await
    }

    #[tokio::test]
    async fn concurrent_upload_streams_share_handle_without_sharing_cursor() {
        use std::io::{Seek, SeekFrom, Write};

        let expected: Vec<u8> = (0..(UPLOAD_CHUNK_BYTES * 2 + 17_123))
            .map(|index| (index % 251) as u8)
            .collect();
        let mut temporary = tempfile::NamedTempFile::new().expect("temporary upload file");
        temporary
            .write_all(&expected)
            .expect("write upload fixture");
        temporary.flush().expect("flush upload fixture");
        let mut open_file = temporary.reopen().expect("reopen upload fixture once");
        open_file
            .seek(SeekFrom::End(-37))
            .expect("set an irrelevant shared cursor position");
        let shared_file = Arc::new(open_file);

        let first = collect_positional_upload(Arc::clone(&shared_file), expected.len() as u64);
        let second = collect_positional_upload(Arc::clone(&shared_file), expected.len() as u64);
        let (first, second) = tokio::join!(first, second);

        assert_eq!(first.expect("first stream reads exactly"), expected);
        assert_eq!(second.expect("second stream reads exactly"), expected);
    }

    #[tokio::test]
    async fn positional_upload_stream_rejects_shorter_than_advertised_file() {
        use std::io::Write;

        let expected = vec![7_u8; UPLOAD_CHUNK_BYTES + 29];
        let mut temporary = tempfile::NamedTempFile::new().expect("temporary upload file");
        temporary
            .write_all(&expected)
            .expect("write upload fixture");
        temporary.flush().expect("flush upload fixture");
        let shared_file = Arc::new(temporary.reopen().expect("open upload fixture"));

        let error = collect_positional_upload(shared_file, expected.len() as u64 + 1)
            .await
            .expect_err("short file must not satisfy advertised length");

        assert_eq!(error.kind(), io::ErrorKind::UnexpectedEof);
    }

    #[tokio::test]
    async fn upload_no_longer_returns_gate1_not_implemented_for_missing_file() {
        let http = Arc::new(MockHttpClient::new());
        let client = AdpClient::new("https://dist.example.com", http);
        let signer = crate::signers::LocalSigner::from_hex(
            "0000000000000000000000000000000000000000000000000000000000000001",
        )
        .expect("test private key should be valid");
        let listing = nostr::EventBuilder::new(nostr::Kind::Custom(30402), "test")
            .sign_with_keys(signer.keys())
            .expect("listing should sign");
        let missing = std::path::PathBuf::from("/tmp/arcadestr-missing-upload-file.zip");

        let err = client
            .upload(&signer, &listing, &missing)
            .await
            .expect_err("missing file should fail before upload succeeds");

        assert!(!matches!(err, AdpClientError::NotImplemented("upload")));
    }

    #[tokio::test]
    async fn entitlement_claim_posts_exact_request_with_nip98() {
        let buyer = crate::signers::LocalSigner::from_hex(
            "0000000000000000000000000000000000000000000000000000000000000001",
        )
        .expect("test private key should be valid");
        let issuer = nostr::Keys::generate();
        let grant = nostr::EventBuilder::new(
            nostr::Kind::Custom(crate::adp_protocol::ENTITLEMENT_GRANT_KIND),
            "",
        )
        .sign_with_keys(&issuer)
        .expect("grant signs");
        let url = "https://dist.example.com/entitlement/claim";
        let http = Arc::new(MockHttpClient::new().with_json_post_response(
            url,
            json!({
                "grant": grant,
                "download_token": "claim-token",
                "token_expires_at": 1_800_000_000i64,
                "already_claimed": true
            }),
        ));
        let client = AdpClient::new("https://dist.example.com", http.clone());

        let response = client
            .entitlement_claim(
                &buyer,
                EntitlementClaimRequest {
                    game_coordinate: "30402:publisher:game".into(),
                    campaign_event_id: "aa".repeat(32),
                },
            )
            .await
            .expect("claim succeeds");

        assert_eq!(response.download_token, "claim-token");
        assert!(response.already_claimed);
        assert_eq!(
            http.last_json_post_body(url),
            Some(json!({
                "game_coordinate": "30402:publisher:game",
                "campaign_event_id": "aa".repeat(32)
            }))
        );
        let headers = http
            .last_json_post_headers(url)
            .expect("claim headers recorded");
        assert!(headers
            .iter()
            .any(|(name, value)| { name == "Authorization" && value.starts_with("Nostr ") }));
    }

    #[tokio::test]
    async fn entitlement_claim_maps_distribution_error() {
        let buyer = crate::signers::LocalSigner::from_hex(
            "0000000000000000000000000000000000000000000000000000000000000001",
        )
        .expect("test private key should be valid");
        let url = "https://dist.example.com/entitlement/claim";
        let http = Arc::new(MockHttpClient::new().with_post_error_response(
            url,
            HttpClientError::StatusWithBody {
                status: 451,
                body: "server is not authorized".into(),
            },
        ));
        let client = AdpClient::new("https://dist.example.com", http);

        let error = client
            .entitlement_claim(
                &buyer,
                EntitlementClaimRequest {
                    game_coordinate: "30402:publisher:game".into(),
                    campaign_event_id: "aa".repeat(32),
                },
            )
            .await
            .expect_err("claim must fail");

        assert!(matches!(
            error,
            AdpClientError::ClaimDistributionUnauthorized(_)
        ));
    }

    #[test]
    fn updated_server_authorization_errors_are_typed() {
        assert!(matches!(
            map_claim_error(HttpClientError::StatusWithBody {
                status: 503,
                body: r#"{"error":"authorization evidence unavailable"}"#.into(),
            }),
            AdpClientError::AuthorizationUnavailable(_)
        ));
        assert!(matches!(
            map_claim_error(HttpClientError::StatusWithBody {
                status: 500,
                body: r#"{"error":"no held key has current listing and authorization authority"}"#
                    .into(),
            }),
            AdpClientError::NoHeldAuthorizedSigner(_)
        ));
        assert!(matches!(
            map_purchase_error(HttpClientError::StatusWithBody {
                status: 451,
                body: r#"{"error":"this server is not named in the current listing"}"#.into(),
            }),
            AdpClientError::DownloadDistribution(_)
        ));
        assert!(matches!(
            map_purchase_error(HttpClientError::StatusWithBody {
                status: 503,
                body: r#"{"error":"listing lookup unavailable"}"#.into(),
            }),
            AdpClientError::AuthorizationUnavailable(_)
        ));
    }
}
