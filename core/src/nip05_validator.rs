//! Background NIP-05 validation worker
//!
//! Validates NIP-05 identifiers asynchronously without blocking the UI.
//! Uses a queue-based system where profiles are queued for validation
//! and processed in the background.

use std::collections::VecDeque;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use nostr::PublicKey;
use serde::Deserialize;
use serde_json::Value;
use tokio::sync::mpsc;
use tracing::{debug, info, warn};

use crate::http_client::{HttpClient, ReqwestHttpClient};
use crate::nostr::NostrClient;
use crate::user_cache::UserCache;

/// Command sent to the validator worker
#[derive(Debug, Clone)]
pub enum ValidationCommand {
    /// Queue a profile for NIP-05 validation
    Validate { npub: String, nip05: String },
    /// Shutdown the worker
    Shutdown,
}

/// Result of NIP-05 validation
#[derive(Debug, Clone)]
pub struct ValidationResult {
    pub npub: String,
    pub nip05: String,
    pub verified: bool,
}

/// Result state for standalone NIP-05 identity checks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IdentityValidationState {
    Valid,
    Invalid,
    Error,
}

/// Detailed result for standalone NIP-05 identity checks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdentityValidationResult {
    pub state: IdentityValidationState,
    pub relays: Vec<String>,
}

#[derive(Debug, Clone)]
struct CacheEntry {
    result: IdentityValidationResult,
    cached_at: Instant,
}

/// NIP-05 response document.
#[derive(Debug, Clone, Deserialize)]
struct Nip05Response {
    names: HashMap<String, String>,
    #[serde(default)]
    relays: Option<HashMap<String, Vec<String>>>,
}

/// Standalone NIP-05 identity validator with cache and injectable HTTP client.
pub struct Nip05IdentityValidator {
    http_client: Arc<dyn HttpClient>,
    cache: Mutex<HashMap<String, CacheEntry>>,
    cache_ttl: Duration,
}

impl Nip05IdentityValidator {
    const DEFAULT_CACHE_TTL: Duration = Duration::from_secs(300);

    /// Create validator with production HTTP client and default cache TTL.
    pub fn new() -> Self {
        let http_client = ReqwestHttpClient::new(Duration::from_secs(8))
            .expect("reqwest_http_client initialization failed");
        Self::with_http_client(Arc::new(http_client))
    }

    /// Create validator with custom HTTP client.
    pub fn with_http_client(http_client: Arc<dyn HttpClient>) -> Self {
        Self {
            http_client,
            cache: Mutex::new(HashMap::new()),
            cache_ttl: Self::DEFAULT_CACHE_TTL,
        }
    }

    /// Override cache TTL.
    pub fn with_ttl(mut self, cache_ttl: Duration) -> Self {
        self.cache_ttl = cache_ttl;
        self
    }

    /// Validate NIP-05 identifier against npub and extract relay hints.
    pub async fn validate(&self, npub: &str, identifier: &str) -> IdentityValidationResult {
        let cache_key = format!("{}|{}", npub, identifier);

        if let Some(cached) = self.get_cached(&cache_key) {
            return cached;
        }

        let (name, domain) = match parse_identifier(identifier) {
            Some(parts) => parts,
            None => {
                return IdentityValidationResult {
                    state: IdentityValidationState::Invalid,
                    relays: vec![],
                }
            }
        };

        let expected_hex = match PublicKey::parse(npub) {
            Ok(pubkey) => pubkey.to_hex().to_lowercase(),
            Err(_) => {
                return IdentityValidationResult {
                    state: IdentityValidationState::Invalid,
                    relays: vec![],
                }
            }
        };

        let url = format!("https://{}/.well-known/nostr.json?name={}", domain, name);

        let result = match self.http_client.get_json(&url).await {
            Ok(body) => self.validate_json_response(&body, name, &expected_hex),
            Err(_) => IdentityValidationResult {
                state: IdentityValidationState::Error,
                relays: vec![],
            },
        };

        self.insert_cache(cache_key, result.clone());
        result
    }

    fn get_cached(&self, key: &str) -> Option<IdentityValidationResult> {
        let cache = self
            .cache
            .lock()
            .expect("nip05_identity_validator cache mutex poisoned");
        let entry = cache.get(key)?;

        if entry.cached_at.elapsed() <= self.cache_ttl {
            return Some(entry.result.clone());
        }

        None
    }

    fn insert_cache(&self, key: String, result: IdentityValidationResult) {
        let mut cache = self
            .cache
            .lock()
            .expect("nip05_identity_validator cache mutex poisoned");
        cache.insert(
            key,
            CacheEntry {
                result,
                cached_at: Instant::now(),
            },
        );
    }

    fn validate_json_response(
        &self,
        body: &Value,
        name: &str,
        expected_hex: &str,
    ) -> IdentityValidationResult {
        let parsed: Nip05Response = match serde_json::from_value(body.clone()) {
            Ok(parsed) => parsed,
            Err(_) => {
                return IdentityValidationResult {
                    state: IdentityValidationState::Error,
                    relays: vec![],
                }
            }
        };

        let Some(stored_pubkey) = parsed.names.get(name) else {
            return IdentityValidationResult {
                state: IdentityValidationState::Invalid,
                relays: vec![],
            };
        };

        if !is_hex_pubkey(stored_pubkey) {
            return IdentityValidationResult {
                state: IdentityValidationState::Invalid,
                relays: vec![],
            };
        }

        if stored_pubkey.to_lowercase() != expected_hex {
            return IdentityValidationResult {
                state: IdentityValidationState::Invalid,
                relays: vec![],
            };
        }

        let relays = parsed
            .relays
            .as_ref()
            .and_then(|map| map.get(&stored_pubkey.to_lowercase()).cloned())
            .or_else(|| {
                parsed
                    .relays
                    .as_ref()
                    .and_then(|map| map.get(stored_pubkey).cloned())
            })
            .map(dedup_relays)
            .unwrap_or_default();

        IdentityValidationResult {
            state: IdentityValidationState::Valid,
            relays,
        }
    }
}

impl Default for Nip05IdentityValidator {
    fn default() -> Self {
        Self::new()
    }
}

fn parse_identifier(identifier: &str) -> Option<(&str, &str)> {
    let mut parts = identifier.split('@');
    let name = parts.next()?;
    let domain = parts.next()?;

    if parts.next().is_some() {
        return None;
    }

    if name.is_empty() || domain.is_empty() {
        return None;
    }

    Some((name, domain))
}

fn is_hex_pubkey(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|b| b.is_ascii_hexdigit())
}

fn dedup_relays(relays: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    relays
        .into_iter()
        .filter(|relay| seen.insert(relay.clone()))
        .collect()
}

/// Background NIP-05 validation worker
pub struct Nip05Validator {
    command_tx: mpsc::UnboundedSender<ValidationCommand>,
    result_rx: mpsc::UnboundedReceiver<ValidationResult>,
}

impl Nip05Validator {
    /// Spawn a new validator worker
    pub fn spawn(client: Arc<NostrClient>, user_cache: Arc<UserCache>) -> Self {
        let (command_tx, mut command_rx) = mpsc::unbounded_channel();
        let (result_tx, result_rx) = mpsc::unbounded_channel();

        tokio::spawn(async move {
            let mut queue: VecDeque<(String, String)> = VecDeque::new();
            let mut shutdown = false;

            loop {
                // Process commands or wait
                tokio::select! {
                    Some(cmd) = command_rx.recv() => {
                        match cmd {
                            ValidationCommand::Validate { npub, nip05 } => {
                                debug!("Queued NIP-05 validation for {}", npub);
                                queue.push_back((npub, nip05));
                            }
                            ValidationCommand::Shutdown => {
                                info!("NIP-05 validator shutting down");
                                shutdown = true;
                            }
                        }
                    }
                    _ = tokio::time::sleep(tokio::time::Duration::from_secs(1)), if !queue.is_empty() => {
                        // Process next item in queue
                        if let Some((npub, nip05)) = queue.pop_front() {
                            debug!("Validating NIP-05 for {}: {}", npub, nip05);

                            // Perform validation
                            let verified = client.verify_nip05(&npub, &nip05).await;

                            if verified {
                                info!("NIP-05 verified for {}: {}", npub, nip05);

                                // Update cache with verified status
                                if let Some(mut profile) = user_cache.get(&npub).await {
                                    profile.nip05_verified = true;
                                    if let Err(e) = user_cache.put(&npub, &profile).await {
                                        warn!("Failed to update verified status in cache: {}", e);
                                    }
                                }
                            } else {
                                warn!("NIP-05 verification failed for {}: {}", npub, nip05);
                            }

                            // Send result
                            let _ = result_tx.send(ValidationResult {
                                npub: npub.clone(),
                                nip05: nip05.clone(),
                                verified,
                            });
                        }
                    }
                    else => {
                        if shutdown && queue.is_empty() {
                            break;
                        }
                        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
                    }
                }
            }
        });

        Self {
            command_tx,
            result_rx,
        }
    }

    /// Queue a profile for NIP-05 validation
    pub fn queue_validation(&self, npub: String, nip05: String) {
        let _ = self
            .command_tx
            .send(ValidationCommand::Validate { npub, nip05 });
    }

    /// Try to receive a validation result (non-blocking)
    pub fn try_recv_result(&mut self) -> Option<ValidationResult> {
        self.result_rx.try_recv().ok()
    }

    /// Shutdown the validator
    pub fn shutdown(&self) {
        let _ = self.command_tx.send(ValidationCommand::Shutdown);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    use serde_json::json;

    use crate::test_helpers::http_mocks::MockHttpClient;

    const TEST_HEX_PUBKEY: &str = "d94a3f0b5b907fda6c1d2716af34e4d533ddf8f6f6f0f8f1f4a3f605f6c9a3b4";

    fn test_npub() -> String {
        TEST_HEX_PUBKEY.to_string()
    }

    #[test]
    fn test_validation_command_clone() {
        let cmd = ValidationCommand::Validate {
            npub: "test".to_string(),
            nip05: "test@example.com".to_string(),
        };
        let cloned = cmd.clone();

        match (cmd, cloned) {
            (
                ValidationCommand::Validate {
                    npub: n1,
                    nip05: nip1,
                },
                ValidationCommand::Validate {
                    npub: n2,
                    nip05: nip2,
                },
            ) => {
                assert_eq!(n1, n2);
                assert_eq!(nip1, nip2);
            }
            _ => panic!("Clone mismatch"),
        }
    }

    #[tokio::test]
    async fn valid_identifier_format() {
        let npub = test_npub();
        let expected_hex = PublicKey::parse(&npub)
            .expect("invalid test npub")
            .to_hex();

        let alice_url = "https://example.com/.well-known/nostr.json?name=alice";
        let root_url = "https://example.com/.well-known/nostr.json?name=_";
        let mock = MockHttpClient::new()
            .with_json_response(
                alice_url,
                json!({
                    "names": {
                        "alice": expected_hex,
                    }
                }),
            )
            .with_json_response(
                root_url,
            json!({
                "names": {
                        "_": expected_hex,
                }
            }),
            );

        let validator = Nip05IdentityValidator::with_http_client(Arc::new(mock.clone()));

        let standard = validator.validate(&npub, "alice@example.com").await;
        assert_eq!(standard.state, IdentityValidationState::Valid);

        let root = validator.validate(&npub, "_@example.com").await;
        assert_eq!(root.state, IdentityValidationState::Valid);

        let bare = validator.validate(&npub, "alice").await;
        assert_eq!(bare.state, IdentityValidationState::Invalid);
    }

    #[tokio::test]
    async fn hex_pubkey_match_passes_verification() {
        let npub = test_npub();
        let name = "alice";
        let domain = "example.com";
        let expected_hex = PublicKey::parse(&npub)
            .expect("invalid test npub")
            .to_hex();
        let url = format!("https://{}/.well-known/nostr.json?name={}", domain, name);

        let mock = MockHttpClient::new().with_json_response(
            &url,
            json!({
                "names": {
                    name: expected_hex,
                }
            }),
        );

        let validator = Nip05IdentityValidator::with_http_client(Arc::new(mock));
        let result = validator.validate(&npub, "alice@example.com").await;

        assert_eq!(result.state, IdentityValidationState::Valid);
    }

    #[tokio::test]
    async fn hex_pubkey_mismatch_fails_verification() {
        let npub = test_npub();
        let url = "https://example.com/.well-known/nostr.json?name=alice";

        let mock = MockHttpClient::new().with_json_response(
            url,
            json!({
                "names": {
                    "alice": "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
                }
            }),
        );

        let validator = Nip05IdentityValidator::with_http_client(Arc::new(mock));
        let result = validator.validate(&npub, "alice@example.com").await;

        assert_eq!(result.state, IdentityValidationState::Invalid);
    }

    #[tokio::test]
    async fn http_redirect_is_not_followed() {
        let npub = test_npub();
        let url = "https://example.com/.well-known/nostr.json?name=alice";

        let mock = MockHttpClient::new().with_redirect_response(url, "https://other.example.com");

        let validator = Nip05IdentityValidator::with_http_client(Arc::new(mock));
        let result = validator.validate(&npub, "alice@example.com").await;

        assert_eq!(result.state, IdentityValidationState::Error);
    }

    #[tokio::test]
    async fn missing_names_key_fails() {
        let npub = test_npub();
        let url = "https://example.com/.well-known/nostr.json?name=alice";

        let mock = MockHttpClient::new().with_json_response(
            url,
            json!({
                "wrong": {},
            }),
        );

        let validator = Nip05IdentityValidator::with_http_client(Arc::new(mock));
        let result = validator.validate(&npub, "alice@example.com").await;

        assert_eq!(result.state, IdentityValidationState::Error);
    }

    #[tokio::test]
    async fn npub_in_json_response_is_rejected() {
        let npub = test_npub();
        let url = "https://example.com/.well-known/nostr.json?name=alice";

        let mock = MockHttpClient::new().with_json_response(
            url,
            json!({
                "names": {
                    "alice": "npub1invalidforhexresponse0000000000000000000000000000000000",
                }
            }),
        );

        let validator = Nip05IdentityValidator::with_http_client(Arc::new(mock));
        let verify_npub = test_npub();
        let result = validator.validate(&verify_npub, "alice@example.com").await;

        assert_eq!(result.state, IdentityValidationState::Invalid);
    }

    #[tokio::test]
    async fn cache_returns_cached_result() {
        let npub = test_npub();
        let expected_hex = PublicKey::parse(&npub)
            .expect("invalid test npub")
            .to_hex();
        let url = "https://example.com/.well-known/nostr.json?name=alice";

        let mock = MockHttpClient::new().with_json_response(
            url,
            json!({
                "names": {
                    "alice": expected_hex,
                }
            }),
        );

        let validator = Nip05IdentityValidator::with_http_client(Arc::new(mock.clone()))
            .with_ttl(Duration::from_secs(300));

        let first = validator.validate(&npub, "alice@example.com").await;
        let second = validator.validate(&npub, "alice@example.com").await;

        assert_eq!(first.state, IdentityValidationState::Valid);
        assert_eq!(second.state, IdentityValidationState::Valid);
        assert_eq!(mock.call_count(url), 1);
    }

    #[tokio::test]
    async fn cache_expiry_with_zero_ttl_calls_http_again() {
        let npub = test_npub();
        let expected_hex = PublicKey::parse(&npub)
            .expect("invalid test npub")
            .to_hex();
        let url = "https://example.com/.well-known/nostr.json?name=alice";

        let mock = MockHttpClient::new().with_json_response(
            url,
            json!({
                "names": {
                    "alice": expected_hex,
                }
            }),
        );

        let validator =
            Nip05IdentityValidator::with_http_client(Arc::new(mock.clone())).with_ttl(Duration::ZERO);

        let _ = validator.validate(&npub, "alice@example.com").await;
        let _ = validator.validate(&npub, "alice@example.com").await;

        assert_eq!(mock.call_count(url), 2);
    }

    #[tokio::test]
    async fn relays_from_nostr_json_are_extracted() {
        let npub = test_npub();
        let expected_hex = PublicKey::parse(&npub)
            .expect("invalid test npub")
            .to_hex();
        let url = "https://example.com/.well-known/nostr.json?name=alice";

        let mock = MockHttpClient::new().with_json_response(
            url,
            json!({
                "names": {
                    "alice": expected_hex,
                },
                "relays": {
                    expected_hex: [
                        "wss://relay.damus.io",
                        "wss://relay.primal.net",
                        "wss://relay.primal.net"
                    ]
                }
            }),
        );

        let validator = Nip05IdentityValidator::with_http_client(Arc::new(mock));
        let result = validator.validate(&npub, "alice@example.com").await;

        assert_eq!(result.state, IdentityValidationState::Valid);
        assert_eq!(
            result.relays,
            vec![
                "wss://relay.damus.io".to_string(),
                "wss://relay.primal.net".to_string()
            ]
        );
    }
}
