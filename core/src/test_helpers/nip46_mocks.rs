use std::collections::HashMap;

use futures::executor::block_on;
use nostr::nips::nip44;
use nostr::{Event, EventBuilder, Keys, Kind, PublicKey, Tag, UnsignedEvent};
use serde_json::{json, Value};

#[derive(Clone, Debug)]
enum MethodBehavior {
    Default,
    Result(Value),
    Error(String),
    Disconnect,
}

/// In-process NIP-46 relay simulator for tests.
///
/// Uses direct event processing (no real WebSocket). Designed for reuse in
/// unit tests and integration scenarios that need deterministic protocol flow.
#[derive(Clone, Debug)]
pub struct MockNip46Relay {
    signer_keys: Keys,
    user_signing_keys: Keys,
    expected_secret: Option<String>,
    connected: bool,
    connect_completed: bool,
    method_behaviors: HashMap<String, MethodBehavior>,
}

impl MockNip46Relay {
    pub fn new(signer_keys: Keys, user_signing_keys: Keys) -> Self {
        Self {
            signer_keys,
            user_signing_keys,
            expected_secret: None,
            connected: true,
            connect_completed: false,
            method_behaviors: HashMap::new(),
        }
    }

    pub fn set_expected_secret(&mut self, secret: &str) {
        self.expected_secret = Some(secret.to_string());
    }

    pub fn set_method_result(&mut self, method: &str, result: Value) {
        self.method_behaviors
            .insert(method.to_string(), MethodBehavior::Result(result));
    }

    pub fn set_method_error(&mut self, method: &str, message: &str) {
        self.method_behaviors.insert(
            method.to_string(),
            MethodBehavior::Error(message.to_string()),
        );
    }

    pub fn set_method_disconnect(&mut self, method: &str) {
        self.method_behaviors
            .insert(method.to_string(), MethodBehavior::Disconnect);
    }

    pub fn clear_method_behavior(&mut self, method: &str) {
        self.method_behaviors.remove(method);
    }

    pub fn disconnect(&mut self) {
        self.connected = false;
    }

    pub fn reconnect(&mut self) {
        self.connected = true;
    }

    pub fn process_client_event(&mut self, event: &Event) -> Result<Event, String> {
        if !self.connected {
            return Err("mock relay disconnected".to_string());
        }

        if event.kind != Kind::NostrConnect {
            return Err("expected kind 24133 (NostrConnect) event".to_string());
        }

        let request = Self::decrypt_client_request(&self.signer_keys, event)?;
        let id = request
            .get("id")
            .and_then(Value::as_str)
            .ok_or_else(|| "missing JSON-RPC id".to_string())?
            .to_string();
        let method = request
            .get("method")
            .and_then(Value::as_str)
            .ok_or_else(|| "missing JSON-RPC method".to_string())?
            .to_string();

        match self
            .method_behaviors
            .get(&method)
            .cloned()
            .unwrap_or(MethodBehavior::Default)
        {
            MethodBehavior::Disconnect => {
                self.connected = false;
                return Err(format!(
                    "mock relay disconnected during method '{}'",
                    method
                ));
            }
            MethodBehavior::Error(message) => {
                return self.build_response_event(
                    event.pubkey,
                    &id,
                    Value::Null,
                    Value::String(message),
                );
            }
            MethodBehavior::Result(value) => {
                return self.build_response_event(event.pubkey, &id, value, Value::Null);
            }
            MethodBehavior::Default => {}
        }

        let params = request
            .get("params")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();

        match method.as_str() {
            "connect" => {
                let received_secret = params
                    .first()
                    .and_then(Value::as_str)
                    .ok_or_else(|| "connect requires secret in params[0]".to_string())?;

                if let Some(expected) = &self.expected_secret {
                    if received_secret != expected {
                        return self.build_response_event(
                            event.pubkey,
                            &id,
                            Value::Null,
                            Value::String("invalid secret".to_string()),
                        );
                    }
                }

                self.connect_completed = true;
                self.build_response_event(
                    event.pubkey,
                    &id,
                    Value::String(received_secret.to_string()),
                    Value::Null,
                )
            }
            "get_public_key" => {
                if !self.connect_completed {
                    return self.build_response_event(
                        event.pubkey,
                        &id,
                        Value::Null,
                        Value::String("not connected".to_string()),
                    );
                }
                self.build_response_event(
                    event.pubkey,
                    &id,
                    Value::String(self.user_signing_keys.public_key().to_hex()),
                    Value::Null,
                )
            }
            "sign_event" => {
                if !self.connect_completed {
                    return self.build_response_event(
                        event.pubkey,
                        &id,
                        Value::Null,
                        Value::String("not connected".to_string()),
                    );
                }
                let unsigned_value = params
                    .first()
                    .cloned()
                    .ok_or_else(|| "sign_event requires params[0] unsigned event".to_string())?;
                let unsigned: UnsignedEvent = serde_json::from_value(unsigned_value)
                    .map_err(|e| format!("invalid unsigned event: {}", e))?;
                let signed = block_on(unsigned.sign(&self.user_signing_keys))
                    .map_err(|e| format!("failed to sign event: {}", e))?;
                self.build_response_event(
                    event.pubkey,
                    &id,
                    serde_json::to_value(signed)
                        .map_err(|e| format!("failed to serialize signed event: {}", e))?,
                    Value::Null,
                )
            }
            "ping" => self.build_response_event(
                event.pubkey,
                &id,
                Value::String("pong".to_string()),
                Value::Null,
            ),
            other => self.build_response_event(
                event.pubkey,
                &id,
                Value::Null,
                Value::String(format!("unsupported method: {}", other)),
            ),
        }
    }

    pub fn build_client_request_event(
        app_keys: &Keys,
        signer_pubkey: PublicKey,
        method: &str,
        params: Value,
        id: &str,
    ) -> Event {
        let request = json!({
            "id": id,
            "method": method,
            "params": params,
        });

        let encrypted = nip44::encrypt(
            app_keys.secret_key(),
            &signer_pubkey,
            &request.to_string(),
            nip44::Version::V2,
        )
        .expect("request payload encryption must succeed");

        EventBuilder::new(Kind::NostrConnect, encrypted)
            .tags(vec![Tag::public_key(signer_pubkey)])
            .sign_with_keys(app_keys)
            .expect("request event signing must succeed")
    }

    pub fn build_signer_connect_event(
        signer_keys: &Keys,
        app_pubkey: PublicKey,
        secret: &str,
        id: &str,
    ) -> Event {
        let message = json!({
            "id": id,
            "method": "connect",
            "params": [secret],
        });

        let encrypted = nip44::encrypt(
            signer_keys.secret_key(),
            &app_pubkey,
            &message.to_string(),
            nip44::Version::V2,
        )
        .expect("connect payload encryption must succeed");

        EventBuilder::new(Kind::NostrConnect, encrypted)
            .tags(vec![Tag::public_key(app_pubkey)])
            .sign_with_keys(signer_keys)
            .expect("connect event signing must succeed")
    }

    pub fn decrypt_client_request(signer_keys: &Keys, event: &Event) -> Result<Value, String> {
        let plaintext = nip44::decrypt(signer_keys.secret_key(), &event.pubkey, &event.content)
            .map_err(|e| format!("failed to decrypt request: {}", e))?;
        serde_json::from_str(&plaintext).map_err(|e| format!("invalid JSON-RPC request: {}", e))
    }

    pub fn decrypt_relay_response(app_keys: &Keys, event: &Event) -> Result<Value, String> {
        let plaintext = nip44::decrypt(app_keys.secret_key(), &event.pubkey, &event.content)
            .map_err(|e| format!("failed to decrypt response: {}", e))?;
        serde_json::from_str(&plaintext).map_err(|e| format!("invalid JSON-RPC response: {}", e))
    }

    fn build_response_event(
        &self,
        app_pubkey: PublicKey,
        id: &str,
        result: Value,
        error: Value,
    ) -> Result<Event, String> {
        let response = json!({
            "id": id,
            "result": result,
            "error": error,
        });

        let encrypted = nip44::encrypt(
            self.signer_keys.secret_key(),
            &app_pubkey,
            &response.to_string(),
            nip44::Version::V2,
        )
        .map_err(|e| format!("failed to encrypt response: {}", e))?;

        EventBuilder::new(Kind::NostrConnect, encrypted)
            .tags(vec![Tag::public_key(app_pubkey)])
            .sign_with_keys(&self.signer_keys)
            .map_err(|e| format!("failed to sign response event: {}", e))
    }
}
