//! HTTP client abstraction for testable network-dependent modules.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

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

fn temp_sibling_path(dest: &Path) -> PathBuf {
    let filename = dest
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("download");
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    dest.with_file_name(format!(
        ".{filename}.{}.{nonce}.tmp",
        std::process::id()
    ))
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

    #[tokio::test]
    async fn failed_stream_removes_temp_and_leaves_destination_unchanged() {
        let dir = tempfile::tempdir().expect("temp dir should be created");
        let dest = dir.path().join("game.bin");
        std::fs::write(&dest, b"previous contents").expect("seed file should be written");

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("test server should bind");
        let addr = listener
            .local_addr()
            .expect("test server address should be available");
        let server = tokio::spawn(async move {
            use tokio::io::{AsyncReadExt, AsyncWriteExt};

            let (mut socket, _) = listener
                .accept()
                .await
                .expect("test server should accept one connection");
            let mut request = [0_u8; 1024];
            let _ = socket
                .read(&mut request)
                .await
                .expect("test server should read request");
            socket
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 64\r\n\r\npartial bytes")
                .await
                .expect("test server should write partial response");
        });

        let client = ReqwestHttpClient::new(Duration::from_secs(5))
            .expect("reqwest client should be created");
        let mut progress_events = Vec::new();
        let err = client
            .download_to_path(
                &format!("http://{addr}/game.bin"),
                Vec::new(),
                &dest,
                &mut |bytes, total| progress_events.push((bytes, total)),
            )
            .await
            .expect_err("truncated stream should fail");

        server.await.expect("test server task should finish");
        assert!(err.to_string().contains("request failed"));
        assert_eq!(
            std::fs::read(&dest).expect("destination should remain readable"),
            b"previous contents"
        );
        let entries = std::fs::read_dir(dir.path())
            .expect("temp dir should be readable")
            .count();
        assert_eq!(entries, 1, "partial temp file should be removed");
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
        let temp_dest = temp_sibling_path(dest);
        let write_result = async {
            let mut file = tokio::fs::File::create(&temp_dest)
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
            drop(file);

            tokio::fs::rename(&temp_dest, dest)
                .await
                .map_err(|e| HttpClientError::Request(e.to_string()))?;

            Ok(HttpDownloadOutcome { bytes_written })
        }
        .await;

        match write_result {
            Ok(outcome) => Ok(outcome),
            Err(err) => {
                let _ = tokio::fs::remove_file(&temp_dest).await;
                Err(err)
            }
        }
    }
}
