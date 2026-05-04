use arcadestr_core::auth::AuthState;
use arcadestr_core::http_client::{HttpClient, HttpClientError, ReqwestHttpClient};
use arcadestr_core::storage::{encrypt_private_key_nip49, serialize_ncryptsec, ScryptParams};
use async_trait::async_trait;
use nostr::Keys;
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;

pub struct AppState {
    pub auth: Arc<Mutex<AuthState>>,
    pub http_client: Arc<dyn HttpClient>,
}

#[path = "../src/command_contracts.rs"]
mod command_contracts;

struct StaticNoRedirectHttpClient {
    expected_url: String,
    response: Result<Value, HttpClientError>,
}

impl StaticNoRedirectHttpClient {
    fn success_nip05(domain: &str, local: &str, pubkey_hex: &str) -> Self {
        Self {
            expected_url: format!("https://{domain}/.well-known/nostr.json?name={local}"),
            response: Ok(json!({
                "names": {
                    local: pubkey_hex,
                }
            })),
        }
    }

    fn failing(url: &str, error: HttpClientError) -> Self {
        Self {
            expected_url: url.to_string(),
            response: Err(error),
        }
    }
}

#[async_trait]
impl HttpClient for StaticNoRedirectHttpClient {
    async fn get_json(&self, _url: &str) -> Result<Value, HttpClientError> {
        Err(HttpClientError::Request(
            "test should use get_json_no_redirects".to_string(),
        ))
    }

    async fn get_json_no_redirects(&self, url: &str) -> Result<Value, HttpClientError> {
        if url != self.expected_url {
            return Err(HttpClientError::Request(format!(
                "unexpected URL in test mock: {url}"
            )));
        }

        self.response.clone()
    }
}

fn create_test_state(active_private_key: Option<&str>) -> AppState {
    let default_http = ReqwestHttpClient::new(Duration::from_secs(1))
        .expect("test reqwest http client should build");
    create_test_state_with_http(active_private_key, Arc::new(default_http))
}

fn create_test_state_with_http(
    active_private_key: Option<&str>,
    http_client: Arc<dyn HttpClient>,
) -> AppState {
    let mut auth = AuthState::new();
    if let Some(private_key) = active_private_key {
        auth.connect_with_key(private_key)
            .expect("test auth should accept provided private key");
    }

    AppState {
        auth: Arc::new(Mutex::new(auth)),
        http_client,
    }
}

#[tokio::test]
async fn test_import_encrypted_key_success() {
    let state = create_test_state(None);
    let keys = Keys::generate();
    let private_key_hex = keys.secret_key().to_secret_hex();
    let password = "correct-horse-battery-staple";

    // Generate a real NIP-49 fixture (encrypt -> serialize) instead of a placeholder literal.
    let encrypted = encrypt_private_key_nip49(
        &private_key_hex,
        password,
        Some(ScryptParams::for_testing()),
    )
    .expect("NIP-49 encryption should succeed");
    let ncryptsec = serialize_ncryptsec(&encrypted).expect("NIP-49 serialization should succeed");

    let result = command_contracts::import_encrypted_key(
        &state,
        command_contracts::ImportKeyRequest {
            ncryptsec,
            password: password.to_string(),
        },
    )
    .await
    .expect("import_encrypted_key should decrypt and parse private key");

    assert!(result.success);
    assert_eq!(result.pubkey, keys.public_key().to_hex());
}

#[tokio::test]
async fn test_import_encrypted_key_wrong_password_returns_decryption_error() {
    let state = create_test_state(None);
    let keys = Keys::generate();
    let private_key_hex = keys.secret_key().to_secret_hex();

    let encrypted = encrypt_private_key_nip49(
        &private_key_hex,
        "correct-password",
        Some(ScryptParams::for_testing()),
    )
    .expect("NIP-49 encryption should succeed");
    let ncryptsec = serialize_ncryptsec(&encrypted).expect("NIP-49 serialization should succeed");

    let error = command_contracts::import_encrypted_key(
        &state,
        command_contracts::ImportKeyRequest {
            ncryptsec,
            password: "wrong-password".to_string(),
        },
    )
    .await
    .expect_err("import should fail with wrong password");

    assert!(matches!(
        error,
        command_contracts::CommandError::Decryption(_)
    ));
}

#[tokio::test]
async fn test_export_encrypted_key_without_authenticated_key_returns_no_active_key() {
    let state = create_test_state(None);

    let error = command_contracts::export_encrypted_key(
        &state,
        command_contracts::ExportKeyRequest {
            password: "strong-passphrase".to_string(),
            scrypt_n: None,
        },
    )
    .await
    .expect_err("export should fail when there is no active direct key signer");

    assert_eq!(error, command_contracts::CommandError::NoActiveKey);
}

#[tokio::test]
async fn test_verify_nip05_identity_success() {
    let expected_pubkey = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    let mock_http =
        StaticNoRedirectHttpClient::success_nip05("example.com", "alice", expected_pubkey);
    let state = create_test_state_with_http(None, Arc::new(mock_http));

    let result = command_contracts::verify_nip05_identity(
        &state,
        command_contracts::VerifyNip05Request {
            nip05: "alice@example.com".to_string(),
            expected_pubkey: expected_pubkey.to_string(),
        },
    )
    .await
    .expect("NIP-05 identity should verify with matching mocked response");

    assert!(result.verified);
    assert_eq!(result.error, None);
    assert_eq!(result.nip05, "alice@example.com");
}

#[tokio::test]
async fn test_verify_nip05_identity_http_failure_maps_to_http_error() {
    let expected_pubkey = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    let url = "https://example.com/.well-known/nostr.json?name=alice";
    let mock_http = StaticNoRedirectHttpClient::failing(
        url,
        HttpClientError::Request("network down".to_string()),
    );
    let state = create_test_state_with_http(None, Arc::new(mock_http));

    let error = command_contracts::verify_nip05_identity(
        &state,
        command_contracts::VerifyNip05Request {
            nip05: "alice@example.com".to_string(),
            expected_pubkey: expected_pubkey.to_string(),
        },
    )
    .await
    .expect_err("NIP-05 verification should map HTTP failures to command HTTP error");

    assert!(matches!(error, command_contracts::CommandError::Http(_)));
}
