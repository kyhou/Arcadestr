#![cfg(feature = "native")]

use arcadestr_core::http_client::HttpClientError;
use arcadestr_core::nostr::{
    build_nip05_url, parse_nip05_identifier, verify_nip05_identity, Nip05ParseError, NostrError,
};
use serde_json::json;

mod http_client {
    pub use arcadestr_core::http_client::*;
}

#[path = "../src/test_helpers/http_mocks.rs"]
mod http_mocks;

use http_mocks::MockHttpClient;

const PUBKEY_ALICE: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const PUBKEY_BOB: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

#[test]
fn nip05_build_url_uses_name_and_domain() {
    let url = build_nip05_url("example.com", "alice");
    assert_eq!(url, "https://example.com/.well-known/nostr.json?name=alice");
}

#[test]
fn nip05_build_url_defaults_to_root_identifier() {
    let url = build_nip05_url("example.com", "");
    assert_eq!(url, "https://example.com/.well-known/nostr.json?name=_");
}

#[test]
fn nip05_parse_identifier_valid_and_invalid_cases() {
    let parsed =
        parse_nip05_identifier("alice@example.com").expect("valid identifier should parse");
    assert_eq!(parsed.local_part, "alice");
    assert_eq!(parsed.domain, "example.com");

    let invalid = parse_nip05_identifier("alice.example.com");
    assert_eq!(invalid, Err(Nip05ParseError::MissingAtSymbol));
}

#[tokio::test]
async fn nip05_verify_success_returns_relay_hints() {
    let url = "https://example.com/.well-known/nostr.json?name=alice";
    let mock = MockHttpClient::new().with_json_response(
        url,
        json!({
            "names": {
                "alice": PUBKEY_ALICE,
            },
            "relays": {
                PUBKEY_ALICE: [
                    "wss://relay.damus.io",
                    "wss://relay.primal.net",
                    "wss://relay.primal.net"
                ]
            }
        }),
    );

    let result = verify_nip05_identity(&mock, "alice@example.com", PUBKEY_ALICE)
        .await
        .expect("verification should succeed");

    assert_eq!(result.nip05, "alice@example.com");
    assert_eq!(result.pubkey, PUBKEY_ALICE);
    assert!(result.verified);
    assert_eq!(
        result.relays,
        vec![
            "wss://relay.damus.io".to_string(),
            "wss://relay.primal.net".to_string()
        ]
    );
}

#[tokio::test]
async fn nip05_verify_domain_only_identifier_defaults_to_root_name() {
    let url = "https://example.com/.well-known/nostr.json?name=_";
    let mock = MockHttpClient::new().with_nip05_response("example.com", "_", PUBKEY_ALICE);

    let result = verify_nip05_identity(&mock, "example.com", PUBKEY_ALICE)
        .await
        .expect("domain-only identifier should verify against root record");

    assert_eq!(result.nip05, "_@example.com");
    assert_eq!(mock.call_count(url), 1);
}

#[tokio::test]
async fn nip05_verify_reports_pubkey_mismatch() {
    let url = "https://example.com/.well-known/nostr.json?name=alice";
    let mock = MockHttpClient::new().with_json_response(
        url,
        json!({
            "names": {
                "alice": PUBKEY_BOB,
            }
        }),
    );

    let err = verify_nip05_identity(&mock, "alice@example.com", PUBKEY_ALICE)
        .await
        .expect_err("verification should fail on pubkey mismatch");

    assert!(
        matches!(err, NostrError::MalformedEvent(message) if message.contains("pubkey mismatch"))
    );
}

#[tokio::test]
async fn nip05_verify_uses_no_redirect_http_path() {
    let url = "https://example.com/.well-known/nostr.json?name=alice";
    let mock = MockHttpClient::new()
        .with_json_response(
            url,
            json!({
                "names": {
                    "alice": PUBKEY_ALICE,
                }
            }),
        )
        .with_no_redirect_error_response(
            url,
            HttpClientError::RedirectBlocked("https://evil.example/nostr.json".to_string()),
        );

    let err = verify_nip05_identity(&mock, "alice@example.com", PUBKEY_ALICE)
        .await
        .expect_err("verification should use no-redirect HTTP path");

    assert!(
        matches!(err, NostrError::RelayError(message) if message.to_ascii_lowercase().contains("redirect"))
    );
    assert_eq!(mock.no_redirect_call_count(url), 1);
}

#[tokio::test]
async fn nip05_verify_reports_http_errors() {
    let url = "https://example.com/.well-known/nostr.json?name=alice";
    let mock = MockHttpClient::new().with_error_response(url, HttpClientError::Status(500));

    let err = verify_nip05_identity(&mock, "alice@example.com", PUBKEY_ALICE)
        .await
        .expect_err("verification should fail when HTTP request fails");

    assert!(matches!(err, NostrError::RelayError(message) if message.contains("500")));
}

#[tokio::test]
async fn nip05_verify_reports_redirect_blocked() {
    let url = "https://example.com/.well-known/nostr.json?name=alice";
    let mock = MockHttpClient::new().with_redirect_response(url, "https://evil.example/nostr.json");

    let err = verify_nip05_identity(&mock, "alice@example.com", PUBKEY_ALICE)
        .await
        .expect_err("verification should fail on redirects");

    assert!(
        matches!(err, NostrError::RelayError(message) if message.to_ascii_lowercase().contains("redirect"))
    );
}
