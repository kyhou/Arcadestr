//! ADP-01 HTTP client for Arcadestr distribution servers.

use std::path::Path;
use std::sync::Arc;

use nostr::Event;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::http_client::{HttpClient, HttpClientError};
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
}

/// Public metadata returned by `GET /.well-known/adp`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct AdpServerInfo {
    pub adp_version: String,
    pub pubkey: String,
    pub name: Option<String>,
    pub url: Option<String>,
}

/// Response returned by `POST /provision`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct ProvisionResponse {
    pub fulfillment_pubkey: String,
    pub attestation_event_id: String,
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
    pub bolt11: String,
    pub preimage: String,
}

/// Response returned by `POST /purchase/confirm` in a later gate.
#[derive(Debug, Clone, Deserialize)]
pub struct PurchaseConfirmResponse {
    pub receipt: Event,
    pub download_token: String,
    pub token_expires_at: i64,
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

/// Typed client for one ADP server base URL.
pub struct AdpClient {
    base_url: String,
    http: Arc<dyn HttpClient>,
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
    /// # Errors
    /// Always returns [`AdpClientError::NotImplemented`] in Gate 1.
    pub async fn upload(
        &self,
        signer: &dyn NostrSigner,
        listing_event: &Event,
        file_path: &Path,
    ) -> Result<UploadResponse, AdpClientError> {
        let url = self.url("/upload");
        let auth_header = build_nip98_auth_header(signer, &url, "POST").await?;
        let listing_json = serde_json::to_string(listing_event)?;
        let file_name = file_path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("game-build")
            .to_string();
        let file_bytes = tokio::fs::read(file_path).await?;
        let file_part = reqwest::multipart::Part::bytes(file_bytes).file_name(file_name);
        let form = reqwest::multipart::Form::new()
            .text("listing_event", listing_json)
            .part("file", file_part);

        let response = reqwest::Client::new()
            .post(url)
            .header("Authorization", auth_header)
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
            .await?;

        Ok(serde_json::from_value(value)?)
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
                    self.url(&format!(
                        "/game/{encoded_coordinate}?token={encoded_token}"
                    )),
                    Vec::new(),
                )
            }
            DownloadAuth::Nip98 { signer } => {
                let url = self.url(&format!("/game/{encoded_coordinate}"));
                let auth_url = self.url(&format!("/game/{game_coordinate}"));
                let auth_header = build_nip98_auth_header(signer, &auth_url, "GET").await?;
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

    fn url(&self, path: &str) -> String {
        format!("{}{}", self.base_url, path)
    }
}

fn map_download_error(err: HttpClientError) -> AdpClientError {
    match err {
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
        HttpClientError::Status(status) => {
            AdpClientError::DownloadProtocol(format!("HTTP status {status}"))
        }
        HttpClientError::StatusWithBody { status, body } => {
            AdpClientError::DownloadProtocol(format!("HTTP status {status}: {body}"))
        }
        other => AdpClientError::Http(other),
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
                "url": "https://dist.example.com"
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
        assert_eq!(
            http.call_count("https://dist.example.com/.well-known/adp"),
            1
        );
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

    #[tokio::test]
    async fn download_percent_encodes_coordinate_and_token() {
        let coordinate = "30406:publisher/game name";
        let token = "token with spaces&scope=bad";
        let expected_url = "https://dist.example.com/game/30406%3Apublisher%2Fgame%20name?token=token%20with%20spaces%26scope%3Dbad";
        let http = Arc::new(MockHttpClient::new().with_download_response(
            expected_url,
            b"encoded download",
        ));
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
                ["u", "https://dist.example.com/game/30406:publisher:test-game"],
                ["method", "GET"]
            ])
        );
        assert_eq!(http.call_count(url), 1);
        assert_eq!(http.last_requested_url().as_deref(), Some(url));

        let _ = std::fs::remove_file(dest);
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
                    bolt11: "lnbc1fixedamountinvoice".to_string(),
                    preimage: "feedface".to_string(),
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
}
