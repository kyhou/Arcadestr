//! Loopback-only Blossom fixture for local smoke testing.
//!
//! `cargo run -p arcadestr-desktop --example blossom_fixture -- --port 3030 --mode store`

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use arcadestr_core::blossom::{
    build_upload_authorization, encode_blossom_authorization_header,
    validate_blossom_server_origin, BlossomServerOriginPolicy, UploadAuthorizationInput,
};
use axum::body::Body;
use axum::extract::State;
use axum::http::header::{AUTHORIZATION, CONTENT_LENGTH, CONTENT_TYPE, HOST, LOCATION};
use axum::http::{HeaderMap, Response, StatusCode};
use axum::routing::head;
use axum::Router;
use base64::Engine as _;
use futures_util::{stream, StreamExt};
use nostr::{Event, Keys};
use serde_json::json;
use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;
use uuid::Uuid;

const UPLOAD_AUTHORIZATION_KIND: u16 = 24_242;
const MAX_BYTES: u64 = 500 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    Store,
    Malformed,
    WrongHash,
    WrongSize,
    WrongMime,
    UnsafeUrl,
    Oversized,
    Delayed,
    Drop,
    Redirect,
    Payment,
}

impl Mode {
    fn parse(value: &str) -> anyhow::Result<Self> {
        match value {
            "store" => Ok(Self::Store),
            "malformed" => Ok(Self::Malformed),
            "wrong-hash" => Ok(Self::WrongHash),
            "wrong-size" => Ok(Self::WrongSize),
            "wrong-mime" => Ok(Self::WrongMime),
            "unsafe-url" => Ok(Self::UnsafeUrl),
            "oversized" => Ok(Self::Oversized),
            "delayed" => Ok(Self::Delayed),
            "drop" => Ok(Self::Drop),
            "redirect" => Ok(Self::Redirect),
            "payment" => Ok(Self::Payment),
            _ => anyhow::bail!("unsupported fixture mode: {value}"),
        }
    }
}

#[derive(Clone)]
struct FixtureState {
    root: Arc<tempfile::TempDir>,
    mode: Mode,
    descriptor_port: u16,
}

struct UploadHeaders {
    hash: String,
    mime_type: String,
    size: u64,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut port = 3030_u16;
    let mut mode = Mode::Store;
    let mut self_test = false;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--port" => {
                port = args
                    .next()
                    .ok_or_else(|| anyhow::anyhow!("--port requires a value"))?
                    .parse()?;
            }
            "--mode" => {
                mode = Mode::parse(
                    &args
                        .next()
                        .ok_or_else(|| anyhow::anyhow!("--mode requires a value"))?,
                )?;
            }
            "--self-test" => self_test = true,
            _ => anyhow::bail!("unknown argument: {arg}"),
        }
    }

    let address = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port);
    let listener = tokio::net::TcpListener::bind(address).await?;
    let local = listener.local_addr()?;
    let state = FixtureState {
        root: Arc::new(tempfile::tempdir()?),
        mode,
        descriptor_port: local.port(),
    };
    let app = Router::new()
        .route("/upload", head(preflight).put(upload))
        .with_state(state);
    println!(
        "Blossom fixture mode={mode:?} origin=http://localhost:{}/",
        local.port()
    );
    if self_test {
        let server = tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        let result = run_self_test(local.port()).await;
        server.abort();
        result?;
        println!("fixture self-test passed: 201 new blob, 200 existing blob");
        return Ok(());
    }
    axum::serve(listener, app).await?;
    Ok(())
}

async fn run_self_test(port: u16) -> anyhow::Result<()> {
    let bytes = [
        0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a, 0, 0, 0, 13, b'I', b'H', b'D', b'R', 0, 0,
        0, 1, 0, 0, 0, 1, 8, 6, 0, 0, 0,
    ];
    let hash = format!("{:x}", Sha256::digest(bytes));
    let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
    let keys = Keys::generate();
    let origin = validate_blossom_server_origin(
        &format!("http://localhost:{port}/"),
        BlossomServerOriginPolicy::AllowHttpLoopback,
    )?;
    let event = build_upload_authorization(&UploadAuthorizationInput {
        publisher: keys.public_key(),
        sha256: hash.clone(),
        created_at: now.saturating_sub(1),
        expiration: now.saturating_add(600),
        server: Some(origin),
        content: "Blossom fixture self-test".into(),
    })?
    .sign(&keys)
    .await?;
    let authorization = encode_blossom_authorization_header(&event)?;
    let endpoint = format!("http://localhost:{port}/upload");
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()?;
    let preflight = client
        .head(&endpoint)
        .header(AUTHORIZATION, &authorization)
        .header("X-SHA-256", &hash)
        .header("X-Content-Type", "image/png")
        .header("X-Content-Length", bytes.len())
        .send()
        .await?;
    anyhow::ensure!(preflight.status() == StatusCode::OK, "preflight failed");
    for expected_status in [StatusCode::CREATED, StatusCode::OK] {
        let response = client
            .put(&endpoint)
            .header(AUTHORIZATION, &authorization)
            .header(CONTENT_TYPE, "image/png")
            .header(CONTENT_LENGTH, bytes.len())
            .header("X-SHA-256", &hash)
            .body(bytes.to_vec())
            .send()
            .await?;
        anyhow::ensure!(
            response.status() == expected_status,
            "unexpected upload status"
        );
        let descriptor: serde_json::Value = response.json().await?;
        anyhow::ensure!(descriptor["sha256"] == hash, "descriptor hash mismatch");
        anyhow::ensure!(
            descriptor["size"] == bytes.len(),
            "descriptor size mismatch"
        );
        anyhow::ensure!(
            descriptor["type"] == "image/png",
            "descriptor MIME mismatch"
        );
    }
    Ok(())
}

async fn preflight(State(state): State<FixtureState>, headers: HeaderMap) -> Response<Body> {
    if let Some(response) = immediate_response(state.mode) {
        return response;
    }
    match validate_headers(&headers, true) {
        Ok(_) => empty(StatusCode::OK),
        Err(response) => response,
    }
}

async fn upload(
    State(state): State<FixtureState>,
    headers: HeaderMap,
    body: Body,
) -> Response<Body> {
    if let Some(response) = immediate_response(state.mode) {
        return response;
    }
    let expected = match validate_headers(&headers, false) {
        Ok(expected) => expected,
        Err(response) => return response,
    };
    let partial = state.root.path().join(format!("{}.part", Uuid::new_v4()));
    let mut file = match tokio::fs::File::create(&partial).await {
        Ok(file) => file,
        Err(_) => return empty(StatusCode::INTERNAL_SERVER_ERROR),
    };
    let mut received = 0_u64;
    let mut hasher = Sha256::new();
    let mut stream = body.into_data_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = match chunk {
            Ok(chunk) => chunk,
            Err(_) => return partial_error(&partial, StatusCode::BAD_REQUEST).await,
        };
        received = received.saturating_add(chunk.len() as u64);
        if received > expected.size || received > MAX_BYTES {
            return partial_error(&partial, StatusCode::PAYLOAD_TOO_LARGE).await;
        }
        hasher.update(&chunk);
        if file.write_all(&chunk).await.is_err() {
            return partial_error(&partial, StatusCode::INTERNAL_SERVER_ERROR).await;
        }
        if state.mode == Mode::Delayed {
            tokio::time::sleep(Duration::from_millis(150)).await;
        }
    }
    if file.flush().await.is_err() || received != expected.size {
        return partial_error(&partial, StatusCode::BAD_REQUEST).await;
    }
    let actual_hash = format!("{:x}", hasher.finalize());
    if actual_hash != expected.hash {
        return partial_error(&partial, StatusCode::BAD_REQUEST).await;
    }
    if state.mode == Mode::Drop {
        remove(&partial).await;
        let failed = stream::once(async {
            Err::<bytes::Bytes, std::io::Error>(std::io::Error::new(
                std::io::ErrorKind::ConnectionAborted,
                "fixture dropped response body",
            ))
        });
        return Response::new(Body::from_stream(failed));
    }

    let final_path = state.root.path().join(&actual_hash);
    let existed = final_path.exists();
    if existed {
        remove(&partial).await;
    } else if tokio::fs::rename(&partial, &final_path).await.is_err() {
        return partial_error(&partial, StatusCode::INTERNAL_SERVER_ERROR).await;
    }
    descriptor(&state, &expected, actual_hash, existed)
}

fn immediate_response(mode: Mode) -> Option<Response<Body>> {
    match mode {
        Mode::Payment => Some(empty(StatusCode::PAYMENT_REQUIRED)),
        Mode::Redirect => Some(
            Response::builder()
                .status(StatusCode::TEMPORARY_REDIRECT)
                .header(LOCATION, "http://localhost/redirected")
                .body(Body::empty())
                .unwrap_or_else(|_| Response::new(Body::empty())),
        ),
        _ => None,
    }
}

fn validate_headers(headers: &HeaderMap, preflight: bool) -> Result<UploadHeaders, Response<Body>> {
    let hash = header(headers, "x-sha-256")?;
    if hash.len() != 64
        || !hash
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(empty(StatusCode::BAD_REQUEST));
    }
    let mime_type = header(
        headers,
        if preflight {
            "x-content-type"
        } else {
            CONTENT_TYPE.as_str()
        },
    )?;
    if !matches!(
        mime_type.as_str(),
        "image/jpeg" | "image/png" | "image/webp" | "video/mp4" | "video/webm"
    ) {
        return Err(empty(StatusCode::UNSUPPORTED_MEDIA_TYPE));
    }
    let size = header(
        headers,
        if preflight {
            "x-content-length"
        } else {
            CONTENT_LENGTH.as_str()
        },
    )?
    .parse::<u64>()
    .map_err(|_| empty(StatusCode::BAD_REQUEST))?;
    if size == 0 || size > MAX_BYTES {
        return Err(empty(StatusCode::PAYLOAD_TOO_LARGE));
    }
    validate_authorization(headers, &hash)?;
    Ok(UploadHeaders {
        hash,
        mime_type,
        size,
    })
}

fn validate_authorization(headers: &HeaderMap, expected_hash: &str) -> Result<(), Response<Body>> {
    let value = header(headers, AUTHORIZATION.as_str())?;
    let encoded = value
        .strip_prefix("Nostr ")
        .ok_or_else(|| empty(StatusCode::UNAUTHORIZED))?;
    let decoded = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(encoded)
        .map_err(|_| empty(StatusCode::UNAUTHORIZED))?;
    let event: Event =
        serde_json::from_slice(&decoded).map_err(|_| empty(StatusCode::UNAUTHORIZED))?;
    if event.verify().is_err()
        || event.kind.as_u16() != UPLOAD_AUTHORIZATION_KIND
        || exact_tag_count(&event, "t", "upload") != 1
        || exact_tag_count(&event, "x", expected_hash) != 1
    {
        return Err(empty(StatusCode::UNAUTHORIZED));
    }
    let expiration = single_tag_value(&event, "expiration")
        .and_then(|value| value.parse::<u64>().ok())
        .ok_or_else(|| empty(StatusCode::UNAUTHORIZED))?;
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| empty(StatusCode::INTERNAL_SERVER_ERROR))?
        .as_secs();
    if event.created_at.as_secs() > now || expiration <= now {
        return Err(empty(StatusCode::UNAUTHORIZED));
    }
    let host = header(headers, HOST.as_str())?;
    let server_tag_count = event
        .tags
        .iter()
        .filter(|tag| tag.as_slice().first().map(String::as_str) == Some("server"))
        .count();
    if host.starts_with("localhost") {
        if exact_tag_count(&event, "server", "localhost") != 1 || server_tag_count != 1 {
            return Err(empty(StatusCode::UNAUTHORIZED));
        }
    } else if server_tag_count != 0 {
        return Err(empty(StatusCode::UNAUTHORIZED));
    }
    Ok(())
}

fn descriptor(
    state: &FixtureState,
    expected: &UploadHeaders,
    actual_hash: String,
    existed: bool,
) -> Response<Body> {
    if state.mode == Mode::Malformed {
        return json_response(StatusCode::CREATED, "{not-json");
    }
    if state.mode == Mode::Oversized {
        return json_response(StatusCode::CREATED, &"x".repeat(65 * 1024));
    }
    let hash = if state.mode == Mode::WrongHash {
        "a".repeat(64)
    } else {
        actual_hash
    };
    let size = if state.mode == Mode::WrongSize {
        expected.size.saturating_add(1)
    } else {
        expected.size
    };
    let mime_type = if state.mode == Mode::WrongMime {
        "image/webp"
    } else {
        &expected.mime_type
    };
    let extension = match expected.mime_type.as_str() {
        "image/jpeg" => ".jpg",
        "image/png" => ".png",
        "image/webp" => ".webp",
        "video/mp4" => ".mp4",
        "video/webm" => ".webm",
        _ => "",
    };
    let url = if state.mode == Mode::UnsafeUrl {
        format!("https://user@localhost/{hash}{extension}")
    } else {
        format!(
            "https://localhost:{}/{hash}{extension}",
            state.descriptor_port
        )
    };
    let uploaded = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(1, |duration| duration.as_secs());
    let body = json!({
        "url": url,
        "sha256": hash,
        "size": size,
        "type": mime_type,
        "uploaded": uploaded,
    })
    .to_string();
    json_response(
        if existed {
            StatusCode::OK
        } else {
            StatusCode::CREATED
        },
        &body,
    )
}

fn header(headers: &HeaderMap, name: &str) -> Result<String, Response<Body>> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned)
        .ok_or_else(|| empty(StatusCode::BAD_REQUEST))
}

fn exact_tag_count(event: &Event, name: &str, value: &str) -> usize {
    event
        .tags
        .iter()
        .filter(|tag| matches!(tag.as_slice(), [tag_name, tag_value] if tag_name == name && tag_value == value))
        .count()
}

fn single_tag_value<'a>(event: &'a Event, name: &str) -> Option<&'a str> {
    let mut values = event.tags.iter().filter_map(|tag| match tag.as_slice() {
        [tag_name, value] if tag_name == name => Some(value.as_str()),
        _ => None,
    });
    let value = values.next()?;
    values.next().is_none().then_some(value)
}

fn empty(status: StatusCode) -> Response<Body> {
    Response::builder()
        .status(status)
        .body(Body::empty())
        .unwrap_or_else(|_| Response::new(Body::empty()))
}

fn json_response(status: StatusCode, body: &str) -> Response<Body> {
    Response::builder()
        .status(status)
        .header(CONTENT_TYPE, "application/json")
        .body(Body::from(body.to_owned()))
        .unwrap_or_else(|_| Response::new(Body::empty()))
}

async fn partial_error(path: &Path, status: StatusCode) -> Response<Body> {
    remove(path).await;
    empty(status)
}

async fn remove(path: &Path) {
    let _ = tokio::fs::remove_file(path).await;
}
