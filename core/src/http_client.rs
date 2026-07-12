//! HTTP client abstraction for testable network-dependent modules.

use std::path::Path;
use std::time::Duration;

use async_trait::async_trait;
use futures::StreamExt;
use serde_json::Value;
use thiserror::Error;
use tokio::io::AsyncWriteExt;

/// Result metadata for a streamed HTTP download.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HttpDownloadOutcome {
    pub bytes_written: u64,
}

/// Errors returned by [`HttpClient`] implementations.
#[derive(Debug, Clone, Error)]
pub enum HttpClientError {
    #[error("HTTP client build failed: {0}")]
    Build(String),
    #[error("HTTP request failed: {0}")]
    Request(String),
    #[error("HTTP redirect blocked: {0}")]
    RedirectBlocked(String),
    #[error("HTTP status not successful: {0}")]
    Status(u16),
    #[error("HTTP status not successful: {status} body: {body}")]
    StatusWithBody { status: u16, body: String },
    #[error("HTTP JSON decode failed: {0}")]
    Json(String),
}

/// Minimal HTTP client contract used by protocol validators.
#[async_trait]
pub trait HttpClient: Send + Sync {
    /// Fetch URL and decode JSON payload.
    async fn get_json(&self, url: &str) -> Result<Value, HttpClientError>;

    /// Fetch URL and decode JSON payload while forbidding redirect following.
    async fn get_json_no_redirects(&self, url: &str) -> Result<Value, HttpClientError>;

    /// Post JSON to a URL and decode the JSON response.
    async fn post_json(
        &self,
        url: &str,
        body: Value,
        headers: Vec<(String, String)>,
    ) -> Result<Value, HttpClientError>;

    /// Stream a URL body to `dest` while reporting cumulative bytes.
    async fn download_to_path(
        &self,
        url: &str,
        headers: Vec<(String, String)>,
        dest: &Path,
        on_progress: &mut (dyn FnMut(u64, Option<u64>) + Send),
    ) -> Result<HttpDownloadOutcome, HttpClientError>;
}

/// Production HTTP client backed by `reqwest`.
pub struct ReqwestHttpClient {
    client: reqwest::Client,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_error_displays_response_body_when_available() {
        let err = HttpClientError::StatusWithBody {
            status: 400,
            body: "{\"error\":\"missing tag\"}".to_string(),
        };

        assert_eq!(
            err.to_string(),
            "HTTP status not successful: 400 body: {\"error\":\"missing tag\"}"
        );
    }
}

impl ReqwestHttpClient {
    /// Build client with redirect policy disabled for NIP-05 compliance.
    pub fn new(timeout: Duration) -> Result<Self, HttpClientError> {
        let client = reqwest::Client::builder()
            .timeout(timeout)
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|e| HttpClientError::Build(e.to_string()))?;

        Ok(Self { client })
    }
}

#[async_trait]
impl HttpClient for ReqwestHttpClient {
    async fn get_json(&self, url: &str) -> Result<Value, HttpClientError> {
        let response = self
            .client
            .get(url)
            .send()
            .await
            .map_err(|e| HttpClientError::Request(e.to_string()))?;

        if response.status().is_redirection() {
            return Err(HttpClientError::RedirectBlocked(
                response.status().to_string(),
            ));
        }

        if !response.status().is_success() {
            let status = response.status().as_u16();
            let body = response
                .text()
                .await
                .unwrap_or_else(|err| format!("<failed to read error body: {err}>"));
            return Err(HttpClientError::StatusWithBody { status, body });
        }

        response
            .json::<Value>()
            .await
            .map_err(|e| HttpClientError::Json(e.to_string()))
    }

    async fn get_json_no_redirects(&self, url: &str) -> Result<Value, HttpClientError> {
        self.get_json(url).await
    }

    async fn post_json(
        &self,
        url: &str,
        body: Value,
        headers: Vec<(String, String)>,
    ) -> Result<Value, HttpClientError> {
        let mut request = self.client.post(url).json(&body);
        for (name, value) in headers {
            request = request.header(name, value);
        }

        let response = request
            .send()
            .await
            .map_err(|e| HttpClientError::Request(e.to_string()))?;

        if response.status().is_redirection() {
            return Err(HttpClientError::RedirectBlocked(
                response.status().to_string(),
            ));
        }

        if !response.status().is_success() {
            let status = response.status().as_u16();
            let body = response
                .text()
                .await
                .unwrap_or_else(|err| format!("<failed to read error body: {err}>"));
            return Err(HttpClientError::StatusWithBody { status, body });
        }

        response
            .json::<Value>()
            .await
            .map_err(|e| HttpClientError::Json(e.to_string()))
    }

    async fn download_to_path(
        &self,
        url: &str,
        headers: Vec<(String, String)>,
        dest: &Path,
        on_progress: &mut (dyn FnMut(u64, Option<u64>) + Send),
    ) -> Result<HttpDownloadOutcome, HttpClientError> {
        let mut request = self.client.get(url);
        for (name, value) in headers {
            request = request.header(name, value);
        }

        let response = request
            .send()
            .await
            .map_err(|e| HttpClientError::Request(e.to_string()))?;

        if response.status().is_redirection() {
            return Err(HttpClientError::RedirectBlocked(
                response.status().to_string(),
            ));
        }

        if !response.status().is_success() {
            let status = response.status().as_u16();
            let body = response
                .text()
                .await
                .unwrap_or_else(|err| format!("<failed to read error body: {err}>"));
            return Err(HttpClientError::StatusWithBody { status, body });
        }

        let total = response.content_length();
        let mut file = tokio::fs::File::create(dest)
            .await
            .map_err(|e| HttpClientError::Request(e.to_string()))?;
        let mut stream = response.bytes_stream();
        let mut bytes_written = 0_u64;

        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|e| HttpClientError::Request(e.to_string()))?;
            file.write_all(&chunk)
                .await
                .map_err(|e| HttpClientError::Request(e.to_string()))?;
            bytes_written += chunk.len() as u64;
            on_progress(bytes_written, total);
        }

        file.flush()
            .await
            .map_err(|e| HttpClientError::Request(e.to_string()))?;

        Ok(HttpDownloadOutcome { bytes_written })
    }
}
