//! HTTP client abstraction for testable network-dependent modules.

use std::error::Error as _;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::Arc;
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

/// A private candidate download and the still-open handle that received it.
#[derive(Debug, Clone)]
pub struct HttpRetainedDownload {
    pub file: Arc<File>,
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
    #[error("insufficient storage while downloading: {0}")]
    InsufficientStorage(String),
    #[error("filesystem file-size limit reached while downloading: {0}")]
    FilesystemFileSizeLimit(String),
    #[error("local download file operation failed: {0}")]
    DownloadIo(String),
}

/// Minimal HTTP client contract used by protocol validators.
#[async_trait]
pub trait HttpClient: Send + Sync {
    /// Native streamed upload boundary. Never follow redirects or reflect server errors.
    async fn post_multipart(
        &self,
        _url: &str,
        _authorization: String,
        _form: reqwest::multipart::Form,
    ) -> Result<Value, HttpClientError> {
        Err(HttpClientError::Request(
            "streamed uploads unsupported".into(),
        ))
    }
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

    /// Exclusively creates a private candidate and retains its written handle.
    async fn download_to_retained_file(
        &self,
        url: &str,
        headers: Vec<(String, String)>,
        candidate: &Path,
        on_progress: &mut (dyn FnMut(u64, Option<u64>) + Send),
    ) -> Result<HttpRetainedDownload, HttpClientError> {
        let mut allow_download = |_| Ok(());
        self.download_to_retained_file_with_preflight(
            url,
            headers,
            candidate,
            &mut allow_download,
            on_progress,
        )
        .await
    }

    /// Performs a retained download with a callback after successful response
    /// headers and content-length discovery, but before candidate creation or
    /// response-body polling.
    async fn download_to_retained_file_with_preflight(
        &self,
        _url: &str,
        _headers: Vec<(String, String)>,
        _candidate: &Path,
        _on_preflight: &mut (dyn FnMut(Option<u64>) -> Result<(), HttpClientError> + Send),
        _on_progress: &mut (dyn FnMut(u64, Option<u64>) + Send),
    ) -> Result<HttpRetainedDownload, HttpClientError> {
        Err(HttpClientError::Request(
            "retained downloads are unsupported by this HTTP client".to_string(),
        ))
    }

    /// Streams into a caller-created capability-owned file. The caller retains
    /// responsibility for capability-rooted cleanup on failure.
    async fn download_to_open_retained_file_with_preflight(
        &self,
        _url: &str,
        _headers: Vec<(String, String)>,
        _candidate: File,
        _on_preflight: &mut (dyn FnMut(Option<u64>) -> Result<(), HttpClientError> + Send),
        _on_progress: &mut (dyn FnMut(u64, Option<u64>) + Send),
    ) -> Result<HttpRetainedDownload, HttpClientError> {
        Err(HttpClientError::Request(
            "open-file retained downloads are unsupported by this HTTP client".to_string(),
        ))
    }
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
    dest.with_file_name(format!(".{filename}.{}.{nonce}.tmp", std::process::id()))
}

/// Publication endpoints require TLS, except explicitly addressed development loopback.
/// Reject credentials, queries and fragments before signing or sending anything.
pub fn validate_publication_endpoint(endpoint: &str) -> Result<(), HttpClientError> {
    let invalid = || HttpClientError::Request("unsafe publication endpoint".into());
    let url = url::Url::parse(endpoint).map_err(|_| invalid())?;
    let loopback = match url.host() {
        Some(url::Host::Domain("localhost")) => true,
        Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
        Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
        _ => false,
    };
    if !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || url.host().is_none()
        || !(url.scheme() == "https" || (url.scheme() == "http" && loopback))
    {
        return Err(invalid());
    }
    Ok(())
}

struct RetainedCandidateFile {
    file: Option<tokio::fs::File>,
    path: PathBuf,
    remove_on_drop: bool,
}

impl RetainedCandidateFile {
    fn new(file: tokio::fs::File, path: &Path) -> Self {
        Self {
            file: Some(file),
            path: path.to_owned(),
            remove_on_drop: true,
        }
    }

    fn file_mut(&mut self) -> Result<&mut tokio::fs::File, HttpClientError> {
        self.file.as_mut().ok_or_else(|| {
            HttpClientError::Request("retained candidate handle is unavailable".to_string())
        })
    }

    fn retain(mut self) -> Result<tokio::fs::File, HttpClientError> {
        self.remove_on_drop = false;
        self.file.take().ok_or_else(|| {
            HttpClientError::Request("retained candidate handle is unavailable".to_string())
        })
    }

    fn cleanup(mut self) -> Result<(), std::io::Error> {
        self.file.take();
        self.remove_on_drop = false;
        match std::fs::remove_file(&self.path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error),
        }
    }
}

impl Drop for RetainedCandidateFile {
    fn drop(&mut self) {
        self.file.take();
        if self.remove_on_drop {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

fn request_error(error: reqwest::Error) -> HttpClientError {
    let mut details = if error.is_timeout() {
        format!("request timed out: {error}")
    } else {
        error.to_string()
    };
    let mut source = error.source();
    while let Some(cause) = source {
        let cause_text = cause.to_string();
        if !details.contains(&cause_text) {
            details.push_str(": ");
            details.push_str(&cause_text);
        }
        source = cause.source();
    }
    HttpClientError::Request(details)
}

pub(crate) fn download_io_error(error: std::io::Error) -> HttpClientError {
    let raw = error.raw_os_error();
    #[cfg(unix)]
    if raw == Some(28) {
        return HttpClientError::InsufficientStorage(error.to_string());
    }
    #[cfg(windows)]
    if raw == Some(112) {
        return HttpClientError::InsufficientStorage(error.to_string());
    }
    #[cfg(unix)]
    if raw == Some(27) {
        return HttpClientError::FilesystemFileSizeLimit(error.to_string());
    }
    #[cfg(windows)]
    if raw == Some(223) {
        return HttpClientError::FilesystemFileSizeLimit(error.to_string());
    }
    HttpClientError::DownloadIo(error.to_string())
}

fn add_cleanup_failure(
    error: HttpClientError,
    candidate: &Path,
    cleanup: std::io::Error,
) -> HttpClientError {
    let detail = format!(
        "{error}; failed to remove retained candidate {}: {cleanup}",
        candidate.display()
    );
    match error {
        HttpClientError::InsufficientStorage(_) => HttpClientError::InsufficientStorage(detail),
        HttpClientError::FilesystemFileSizeLimit(_) => {
            HttpClientError::FilesystemFileSizeLimit(detail)
        }
        HttpClientError::DownloadIo(_) => HttpClientError::DownloadIo(detail),
        _ => HttpClientError::Request(detail),
    }
}

/// Production HTTP client backed by `reqwest`.
pub struct ReqwestHttpClient {
    client: reqwest::Client,
    upload_client: reqwest::Client,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn publication_endpoints_reject_secrets_and_insecure_remote_hosts() {
        for endpoint in [
            "http://example.com/upload",
            "ftp://example.com/upload",
            "https://user:secret@example.com/upload",
            "https://example.com/?secret=token",
            "https://example.com/#secret",
            "file:///tmp/build",
        ] {
            let error = validate_publication_endpoint(endpoint).expect_err("unsafe endpoint");
            assert_eq!(
                error.to_string(),
                "HTTP request failed: unsafe publication endpoint"
            );
        }
        for endpoint in [
            "https://example.com/upload",
            "http://127.0.0.1:8080/upload",
            "http://[::1]:8080/upload",
            "http://localhost:8080/upload",
        ] {
            validate_publication_endpoint(endpoint).expect("TLS or explicit development loopback");
        }
    }

    async fn upload_fixture(response: Vec<u8>, stall: bool) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let address = listener.local_addr().expect("address");
        tokio::spawn(async move {
            use tokio::io::{AsyncReadExt, AsyncWriteExt};
            let (mut socket, _) = listener.accept().await.expect("accept");
            let mut request = [0; 4096];
            let _ = socket.read(&mut request).await;
            if stall {
                let _ = socket.write_all(&response).await;
                std::future::pending::<()>().await;
            }
            let _ = socket.write_all(&response).await;
        });
        format!("http://{address}/upload")
    }

    #[tokio::test]
    async fn multipart_upload_blocks_redirects_and_never_reflects_error_bodies() {
        let client = ReqwestHttpClient::new(Duration::from_secs(2)).expect("client");
        let target = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("target");
        let location = target.local_addr().expect("address");
        let url = upload_fixture(format!("HTTP/1.1 307 Temporary Redirect\r\nLocation: http://{location}/stolen\r\nContent-Length: 0\r\n\r\n").into_bytes(), false).await;
        let error = client
            .post_multipart(
                &url,
                "Nostr secret-header".into(),
                reqwest::multipart::Form::new().text("file", "bytes"),
            )
            .await
            .expect_err("redirect blocked");
        assert!(matches!(error, HttpClientError::RedirectBlocked(_)));
        assert!(
            tokio::time::timeout(Duration::from_millis(30), target.accept())
                .await
                .is_err()
        );
        let url = upload_fixture(b"HTTP/1.1 500 Internal Server Error\r\nContent-Length: 999999999\r\n\r\nNostr secret-header user:password".to_vec(), false).await;
        let error = client
            .post_multipart(
                &url,
                "Nostr secret-header".into(),
                reqwest::multipart::Form::new(),
            )
            .await
            .expect_err("status");
        assert_eq!(error.to_string(), "HTTP status not successful: 500");
    }

    #[tokio::test]
    async fn multipart_upload_bounds_success_bodies_and_times_out_stalls() {
        let body = "x".repeat(65537);
        let url = upload_fixture(
            format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{body}",
                body.len()
            )
            .into_bytes(),
            false,
        )
        .await;
        let client = ReqwestHttpClient::new(Duration::from_secs(2)).expect("client");
        let error = client
            .post_multipart(&url, "secret".into(), reqwest::multipart::Form::new())
            .await
            .expect_err("bounded");
        assert!(error.to_string().contains("exceeds limit"));
        let url = upload_fixture(Vec::new(), true).await;
        let client = ReqwestHttpClient::new(Duration::from_millis(40)).expect("client");
        let error = client
            .post_multipart(&url, "secret".into(), reqwest::multipart::Form::new())
            .await
            .expect_err("timeout");
        assert_eq!(error.to_string(), "HTTP request failed: upload timed out");
        let url = upload_fixture(
            b"HTTP/1.1 200 OK\r\nContent-Length: 10\r\n\r\n{}".to_vec(),
            true,
        )
        .await;
        let error = client
            .post_multipart(&url, "secret".into(), reqwest::multipart::Form::new())
            .await
            .expect_err("response body stall");
        assert_eq!(error.to_string(), "HTTP request failed: upload timed out");
    }

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

    #[test]
    fn download_io_errors_map_storage_categories() {
        assert!(matches!(
            download_io_error(std::io::Error::from(std::io::ErrorKind::PermissionDenied)),
            HttpClientError::DownloadIo(_)
        ));
        #[cfg(unix)]
        assert!(matches!(
            download_io_error(std::io::Error::from_raw_os_error(28)),
            HttpClientError::InsufficientStorage(_)
        ));
        #[cfg(windows)]
        assert!(matches!(
            download_io_error(std::io::Error::from_raw_os_error(112)),
            HttpClientError::InsufficientStorage(_)
        ));
        #[cfg(unix)]
        assert!(matches!(
            download_io_error(std::io::Error::from_raw_os_error(27)),
            HttpClientError::FilesystemFileSizeLimit(_)
        ));
        #[cfg(windows)]
        assert!(matches!(
            download_io_error(std::io::Error::from_raw_os_error(223)),
            HttpClientError::FilesystemFileSizeLimit(_)
        ));
    }

    #[tokio::test]
    async fn retained_preflight_runs_after_headers_before_candidate_creation() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let dir = tempfile::tempdir().expect("temp dir should be created");
        let candidate = dir.path().join("candidate.bin");
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("test server should bind");
        let addr = listener
            .local_addr()
            .expect("test server address should be available");
        let server = tokio::spawn(async move {
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
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 9\r\n\r\nnot-read!")
                .await
                .expect("test server should write response");
        });
        let client = ReqwestHttpClient::new(Duration::from_secs(5))
            .expect("reqwest client should be created");
        let mut calls = 0;

        let error = client
            .download_to_retained_file_with_preflight(
                &format!("http://{addr}/game.bin"),
                Vec::new(),
                &candidate,
                &mut |length| {
                    calls += 1;
                    assert_eq!(length, Some(9));
                    assert!(!candidate.exists());
                    Err(HttpClientError::InsufficientStorage(
                        "capacity preflight rejected".to_string(),
                    ))
                },
                &mut |_bytes, _total| panic!("body progress must not run"),
            )
            .await
            .expect_err("preflight should reject response");

        server.await.expect("test server task should finish");
        assert!(matches!(error, HttpClientError::InsufficientStorage(_)));
        assert_eq!(calls, 1);
        assert!(!candidate.exists());
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

    #[tokio::test]
    async fn retained_download_preserves_an_existing_candidate() {
        let dir = tempfile::tempdir().expect("temp dir should be created");
        let candidate = dir.path().join("candidate.bin");
        std::fs::write(&candidate, b"existing candidate").expect("candidate should be seeded");
        let client = ReqwestHttpClient::new(Duration::from_secs(5))
            .expect("reqwest client should be created");

        let err = client
            .download_to_retained_file(
                "http://127.0.0.1:1/not-requested",
                Vec::new(),
                &candidate,
                &mut |_bytes, _total| {},
            )
            .await
            .expect_err("create_new should reject an existing candidate");

        assert!(err.to_string().contains("request failed"));
        assert_eq!(
            std::fs::read(&candidate).expect("candidate should remain readable"),
            b"existing candidate"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn retained_download_handle_survives_candidate_replacement() {
        use std::os::unix::fs::FileExt;
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let expected = b"retained response bytes";
        let dir = tempfile::tempdir().expect("temp dir should be created");
        let candidate = dir.path().join("candidate.bin");
        let moved = dir.path().join("moved.bin");
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("test server should bind");
        let addr = listener
            .local_addr()
            .expect("test server address should be available");
        let server = tokio::spawn(async move {
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
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 23\r\n\r\nretained response bytes")
                .await
                .expect("test server should write response");
        });
        let client = ReqwestHttpClient::new(Duration::from_secs(5))
            .expect("reqwest client should be created");

        let outcome = client
            .download_to_retained_file(
                &format!("http://{addr}/game.bin"),
                Vec::new(),
                &candidate,
                &mut |_bytes, _total| {},
            )
            .await
            .expect("retained download should succeed");
        server.await.expect("test server task should finish");
        std::fs::rename(&candidate, &moved).expect("candidate should be renamed");
        std::fs::write(&candidate, b"replacement").expect("candidate should be replaced");
        let mut retained = vec![0_u8; expected.len()];
        let read = outcome
            .file
            .read_at(&mut retained, 0)
            .expect("retained handle should remain readable");

        assert_eq!(read, expected.len());
        assert_eq!(retained, expected);
        assert_eq!(outcome.bytes_written, expected.len() as u64);
    }

    #[tokio::test]
    async fn retained_download_removes_candidate_after_truncated_stream() {
        let dir = tempfile::tempdir().expect("temp dir should be created");
        let candidate = dir.path().join("candidate.bin");
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

        let err = client
            .download_to_retained_file(
                &format!("http://{addr}/game.bin"),
                Vec::new(),
                &candidate,
                &mut |_bytes, _total| {},
            )
            .await
            .expect_err("truncated stream should fail");

        server.await.expect("test server task should finish");
        assert!(err.to_string().contains("request failed"));
        assert!(
            !candidate.exists(),
            "partial retained candidate should be removed"
        );
    }

    #[tokio::test]
    async fn cancelled_retained_download_removes_candidate() {
        let dir = tempfile::tempdir().expect("temp dir should be created");
        let candidate = dir.path().join("candidate.bin");
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
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 1024\r\n\r\npartial")
                .await
                .expect("test server should write partial response");
            tokio::time::sleep(Duration::from_secs(60)).await;
        });
        let client = ReqwestHttpClient::new(Duration::from_secs(60))
            .expect("reqwest client should be created");
        let task_candidate = candidate.clone();
        let download = tokio::spawn(async move {
            client
                .download_to_retained_file(
                    &format!("http://{addr}/game.bin"),
                    Vec::new(),
                    &task_candidate,
                    &mut |_bytes, _total| {},
                )
                .await
        });

        for _ in 0..100 {
            if candidate.exists() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        assert!(
            candidate.exists(),
            "candidate should be reserved before cancellation"
        );
        download.abort();
        let _ = download.await;
        server.abort();
        let _ = server.await;

        assert!(!candidate.exists(), "cancelled candidate should be removed");
    }

    #[tokio::test]
    async fn localhost_connects_to_ipv4_without_changing_host_header() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("test server should bind");
        let port = listener
            .local_addr()
            .expect("test server address should be available")
            .port();
        let server = tokio::spawn(async move {
            use tokio::io::{AsyncReadExt, AsyncWriteExt};

            let (mut socket, _) = listener
                .accept()
                .await
                .expect("test server should accept one connection");
            let mut request = [0_u8; 1024];
            let bytes = socket
                .read(&mut request)
                .await
                .expect("test server should read request");
            let request = String::from_utf8_lossy(&request[..bytes]);
            assert!(request.contains(&format!("host: localhost:{port}")));
            socket
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 11\r\n\r\n{\"ok\":true}")
                .await
                .expect("test server should write response");
        });

        let client = ReqwestHttpClient::new(Duration::from_secs(5))
            .expect("reqwest client should be created");
        let response = client
            .get_json(&format!("http://localhost:{port}/health"))
            .await
            .expect("localhost should connect through IPv4");

        server.await.expect("test server task should finish");
        assert_eq!(response, serde_json::json!({ "ok": true }));
    }
}

impl ReqwestHttpClient {
    /// Build client with redirect policy disabled for NIP-05 compliance.
    pub fn new(timeout: Duration) -> Result<Self, HttpClientError> {
        let client = reqwest::Client::builder()
            .timeout(timeout)
            .redirect(reqwest::redirect::Policy::none())
            .resolve("localhost", std::net::SocketAddr::from(([127, 0, 0, 1], 0)))
            .build()
            .map_err(|e| HttpClientError::Build(e.to_string()))?;

        // Large streamed builds need a longer total deadline than metadata requests,
        // while connect/read stalls retain the caller's configured short timeout.
        let upload_client = reqwest::Client::builder()
            .timeout(Duration::from_secs(30 * 60))
            .connect_timeout(timeout.min(Duration::from_secs(30)))
            .read_timeout(timeout.min(Duration::from_secs(30)))
            .redirect(reqwest::redirect::Policy::none())
            .resolve("localhost", std::net::SocketAddr::from(([127, 0, 0, 1], 0)))
            .build()
            .map_err(|_| HttpClientError::Build("upload client unavailable".into()))?;
        Ok(Self {
            client,
            upload_client,
        })
    }
}

#[async_trait]
impl HttpClient for ReqwestHttpClient {
    async fn post_multipart(
        &self,
        url: &str,
        authorization: String,
        form: reqwest::multipart::Form,
    ) -> Result<Value, HttpClientError> {
        validate_publication_endpoint(url)?;
        let sanitized = |error: reqwest::Error| {
            HttpClientError::Request(
                if error.is_timeout() {
                    "upload timed out"
                } else {
                    "upload transport failed"
                }
                .into(),
            )
        };
        let response = self
            .upload_client
            .post(url)
            .header("Authorization", authorization)
            .multipart(form)
            .send()
            .await
            .map_err(sanitized)?;
        if response.status().is_redirection() {
            return Err(HttpClientError::RedirectBlocked(
                "upload redirect refused".into(),
            ));
        }
        if !response.status().is_success() {
            return Err(HttpClientError::Status(response.status().as_u16()));
        }
        const MAX_UPLOAD_RESPONSE: usize = 64 * 1024;
        let mut body = Vec::new();
        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(sanitized)?;
            if chunk.len() > MAX_UPLOAD_RESPONSE.saturating_sub(body.len()) {
                return Err(HttpClientError::Json(
                    "upload response exceeds limit".into(),
                ));
            }
            body.extend_from_slice(&chunk);
        }
        serde_json::from_slice(&body)
            .map_err(|_| HttpClientError::Json("invalid upload response".into()))
    }
    async fn get_json(&self, url: &str) -> Result<Value, HttpClientError> {
        let response = self.client.get(url).send().await.map_err(request_error)?;

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

        let response = request.send().await.map_err(request_error)?;

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
        let mut request = self.client.get(url).timeout(Duration::from_secs(30 * 60));
        for (name, value) in headers {
            request = request.header(name, value);
        }

        let response = request.send().await.map_err(request_error)?;

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
                .map_err(download_io_error)?;
            let mut stream = response.bytes_stream();
            let mut bytes_written = 0_u64;

            while let Some(chunk) = stream.next().await {
                let chunk = chunk.map_err(|e| HttpClientError::Request(e.to_string()))?;
                file.write_all(&chunk).await.map_err(download_io_error)?;
                bytes_written += chunk.len() as u64;
                on_progress(bytes_written, total);
            }

            file.flush().await.map_err(download_io_error)?;
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

    async fn download_to_retained_file_with_preflight(
        &self,
        url: &str,
        headers: Vec<(String, String)>,
        candidate: &Path,
        on_preflight: &mut (dyn FnMut(Option<u64>) -> Result<(), HttpClientError> + Send),
        on_progress: &mut (dyn FnMut(u64, Option<u64>) + Send),
    ) -> Result<HttpRetainedDownload, HttpClientError> {
        let mut request = self.client.get(url).timeout(Duration::from_secs(30 * 60));
        for (name, value) in headers {
            request = request.header(name, value);
        }

        let response = request.send().await.map_err(request_error)?;

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
        on_preflight(total)?;

        let mut options = tokio::fs::OpenOptions::new();
        options.read(true).write(true).create_new(true);
        #[cfg(unix)]
        options.mode(0o600);

        let file = options.open(candidate).await.map_err(download_io_error)?;
        let mut candidate_file = RetainedCandidateFile::new(file, candidate);

        let download_result = async {
            let mut stream = response.bytes_stream();
            let mut bytes_written = 0_u64;

            while let Some(chunk) = stream.next().await {
                let chunk = chunk.map_err(|e| HttpClientError::Request(e.to_string()))?;
                candidate_file
                    .file_mut()?
                    .write_all(&chunk)
                    .await
                    .map_err(download_io_error)?;
                bytes_written += chunk.len() as u64;
                on_progress(bytes_written, total);
            }

            candidate_file
                .file_mut()?
                .flush()
                .await
                .map_err(download_io_error)?;
            candidate_file
                .file_mut()?
                .sync_all()
                .await
                .map_err(download_io_error)?;
            Ok(bytes_written)
        }
        .await;

        match download_result {
            Ok(bytes_written) => {
                let file = candidate_file.retain()?.into_std().await;
                Ok(HttpRetainedDownload {
                    file: Arc::new(file),
                    bytes_written,
                })
            }
            Err(err) => {
                if let Err(cleanup) = candidate_file.cleanup() {
                    return Err(add_cleanup_failure(err, candidate, cleanup));
                }
                Err(err)
            }
        }
    }

    async fn download_to_open_retained_file_with_preflight(
        &self,
        url: &str,
        headers: Vec<(String, String)>,
        candidate: File,
        on_preflight: &mut (dyn FnMut(Option<u64>) -> Result<(), HttpClientError> + Send),
        on_progress: &mut (dyn FnMut(u64, Option<u64>) + Send),
    ) -> Result<HttpRetainedDownload, HttpClientError> {
        let mut request = self.client.get(url).timeout(Duration::from_secs(30 * 60));
        for (name, value) in headers {
            request = request.header(name, value);
        }
        let response = request.send().await.map_err(request_error)?;
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
        on_preflight(total)?;

        let mut file = tokio::fs::File::from_std(candidate);
        let mut stream = response.bytes_stream();
        let mut bytes_written = 0_u64;
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|error| HttpClientError::Request(error.to_string()))?;
            file.write_all(&chunk).await.map_err(download_io_error)?;
            bytes_written += chunk.len() as u64;
            on_progress(bytes_written, total);
        }
        file.flush().await.map_err(download_io_error)?;
        file.sync_all().await.map_err(download_io_error)?;
        let file = file.into_std().await;
        Ok(HttpRetainedDownload {
            file: Arc::new(file),
            bytes_written,
        })
    }
}
