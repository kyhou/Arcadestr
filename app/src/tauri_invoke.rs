// Tauri v2 invoke bridge for WASM target
// Uses direct JavaScript interop to call Tauri v2's window.__TAURI__.core

use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::JsFuture;

/// Check if Tauri is available
fn is_tauri_available() -> bool {
    js_sys::eval(
        "typeof window.__TAURI__ !== 'undefined' && typeof window.__TAURI__.core !== 'undefined'",
    )
    .map(|v| v.as_bool().unwrap_or(false))
    .unwrap_or(false)
}

/// Check if Tauri event API is available
fn is_tauri_event_available() -> bool {
    js_sys::eval(
        "typeof window.__TAURI__ !== 'undefined' && typeof window.__TAURI__.event !== 'undefined'",
    )
    .map(|v| v.as_bool().unwrap_or(false))
    .unwrap_or(false)
}

/// Listen for a Tauri event
/// Returns a cleanup function that can be called to unlisten
pub async fn listen<F>(event: &str, mut callback: F) -> Result<impl FnOnce(), String>
where
    F: FnMut(serde_json::Value) + 'static,
{
    if !is_tauri_event_available() {
        return Err("Tauri event API not available".to_string());
    }

    // Create a JavaScript callback wrapper
    let closure = Closure::wrap(Box::new(move |event_data: JsValue| {
        // Parse the event data
        let data_str = if let Some(s) = event_data.as_string() {
            s
        } else {
            match js_sys::JSON::stringify(&event_data) {
                Ok(s) => s.as_string().unwrap_or_else(|| "{}".to_string()),
                Err(_) => "{}".to_string(),
            }
        };

        if let Ok(json) = serde_json::from_str::<serde_json::Value>(&data_str) {
            if let Some(payload) = json.get("payload") {
                callback(payload.clone());
            } else {
                callback(json);
            }
        }
    }) as Box<dyn FnMut(JsValue)>);

    let window = web_sys::window().ok_or_else(|| "Window not available".to_string())?;
    let tauri_obj = js_sys::Reflect::get(&window, &JsValue::from_str("__TAURI__"))
        .map_err(|e| format!("Failed to access __TAURI__: {:?}", e))?;
    let event_api = js_sys::Reflect::get(&tauri_obj, &JsValue::from_str("event"))
        .map_err(|e| format!("Failed to access __TAURI__.event: {:?}", e))?;
    let listen_fn = js_sys::Reflect::get(&event_api, &JsValue::from_str("listen"))
        .map_err(|e| format!("Failed to access event.listen: {:?}", e))?
        .dyn_into::<js_sys::Function>()
        .map_err(|_| "event.listen is not a function".to_string())?;

    let promise_val = listen_fn
        .call2(
            &event_api,
            &JsValue::from_str(event),
            closure.as_ref().unchecked_ref(),
        )
        .map_err(|e| format!("Failed to call listen for '{}': {:?}", event, e))?;

    let promise = promise_val
        .dyn_into::<js_sys::Promise>()
        .map_err(|_| format!("listen('{}') did not return a Promise", event))?;

    let unlisten_val = JsFuture::from(promise)
        .await
        .map_err(|e| format!("listen('{}') promise rejected: {:?}", event, e))?;

    let unlisten_fn = unlisten_val
        .dyn_into::<js_sys::Function>()
        .map_err(|_| format!("listen('{}') resolved value is not a function", event))?;

    let cleanup = move || {
        let _ = unlisten_fn.call0(&JsValue::NULL);
        drop(closure);
    };

    Ok(cleanup)
}

/// Listen for bunker-auth-challenge events (opens browser for approval)
pub async fn listen_bunker_auth_challenge<F>(mut callback: F) -> Result<impl FnOnce(), String>
where
    F: FnMut(String) + 'static,
{
    listen("bunker-auth-challenge", move |data| {
        if let Some(url) = data.as_str() {
            callback(url.to_string());
        } else if let Some(url) = data.get("payload").and_then(|p| p.as_str()) {
            callback(url.to_string());
        }
    })
    .await
}

/// Listen for auth_success events
pub async fn listen_auth_success<F>(mut callback: F) -> Result<impl FnOnce(), String>
where
    F: FnMut(String) + 'static,
{
    listen("auth_success", move |data| {
        if let Some(npub) = data.as_str() {
            callback(npub.to_string());
        } else if let Some(npub) = data.get("payload").and_then(|p| p.as_str()) {
            callback(npub.to_string());
        }
    })
    .await
}

/// Listen for bunker-heartbeat events
pub async fn listen_bunker_heartbeat<F>(callback: F) -> Result<impl FnOnce(), String>
where
    F: FnMut(serde_json::Value) + 'static,
{
    listen("bunker-heartbeat", callback).await
}

/// Listen for qr-login-complete events (Flow B successful connection)
pub async fn listen_qr_login_complete<F>(mut callback: F) -> Result<impl FnOnce(), String>
where
    F: FnMut(String) + 'static,
{
    listen("qr-login-complete", move |data| {
        if let Some(npub) = data.as_str() {
            callback(npub.to_string());
        } else if let Some(npub) = data.get("payload").and_then(|p| p.as_str()) {
            callback(npub.to_string());
        }
    })
    .await
}

/// Invoke a Tauri command via direct JavaScript interop
fn receipt_private_command(command:&str)->bool {
    matches!(command,"sync_purchase_receipts"|"import_purchase_receipts"|"ingest_receipt")
}

/// Invoke a Tauri command via direct JavaScript interop
/// Uses window.__TAURI__.core.invoke which is the Tauri v2 API
#[allow(dead_code)]
pub async fn invoke<T: serde::de::DeserializeOwned + 'static>(
    command: &str,
    args: serde_json::Value,
) -> Result<T, String> {
    // Check if Tauri is available
    if !is_tauri_available() {
        return Err(
            "Tauri API not available. Make sure you're running inside a Tauri window.".to_string(),
        );
    }

    // Call JavaScript function that wraps Tauri v2 invoke
    let promise = tauri_invoke(command, args)?;

    // Convert the Promise to a Future that returns Result<JsValue, JsValue>
    let js_value = JsFuture::from(promise).await.map_err(|e| {
        if matches!(
            command,
            "connect_nwc_wallet"
                | "get_nwc_wallet"
                | "disconnect_nwc_wallet"
                | "verify_nwc_wallet"
                | "pay_nwc_invoice"
                | "sync_purchase_receipts"
                | "import_purchase_receipts"
                | "ingest_receipt"
        ) {
            // These commands return sanitized, user-facing backend errors.
            e.as_string().unwrap_or_else(|| {
                if receipt_private_command(command) {return "Receipt verification operation failed. Retry synchronization or import.".into();}
                "NWC operation failed. Retry or refresh the saved wallet.".into()
            })
        } else {
            format!("JS error: {:?}", e)
        }
    })?;

    // Convert the response to a string - it could be a string or an object
    let response_str = if let Some(s) = js_value.as_string() {
        s
    } else {
        // It's an object, need to stringify it using JSON.stringify
        let stringify_call = js_sys::JSON::stringify(&js_value);
        match stringify_call {
            Ok(s) => s.as_string().unwrap_or_else(|| "{}".to_string()),
            Err(_) => "{}".to_string(),
        }
    };

    // Debug: print what we received
    if !receipt_private_command(command) {
        // Other commands can also return invoices/preimages. Log the command,
        // never the response body, even outside receipt portability.
        web_sys::console::log_1(&format!("Tauri response received for '{}'", command).into());
    }

    // Try to parse as JSON first, then as plain string
    // Some commands return JSON, others return plain strings (like npub)
    let result: Result<T, _> = serde_json::from_str(&response_str);

    match result {
        Ok(value) => Ok(value),
        Err(_) => {
            if receipt_private_command(command) {return Err("Invalid receipt verification response".into());}
            // If JSON parsing fails, try to deserialize as a plain string
            // This handles commands that return plain strings like "npub1..."
            // We need to properly escape the string for JSON
            let escaped = serde_json::to_string(&response_str)
                .map_err(|e| format!("Failed to escape string: {}", e))?;
            serde_json::from_str(&escaped).map_err(|e| {
                format!(
                    "Failed to parse response from command '{}': {}. Response was: {}",
                    command,
                    e,
                    &response_str[..response_str.len().min(200)]
                )
            })
        }
    }
}

/// Invoke a Tauri command while preserving structured rejection payloads.
pub async fn invoke_typed<T, E>(command: &str, args: serde_json::Value) -> Result<T, E>
where
    T: serde::de::DeserializeOwned + 'static,
    E: serde::de::DeserializeOwned + From<String>,
{
    if !is_tauri_available() {
        return Err(E::from("Tauri API not available.".to_string()));
    }
    let promise = tauri_invoke(command, args).map_err(E::from)?;
    let value = match JsFuture::from(promise).await {
        Ok(value) => value,
        Err(rejection) => {
            if let Some(raw) = rejection.as_string() {
                return match serde_json::from_str::<E>(&raw) {
                    Ok(error) => Err(error),
                    Err(_) => Err(E::from(raw)),
                };
            }
            let serialized = js_value_json(&rejection);
            return match serde_json::from_str::<E>(&serialized) {
                Ok(error) => Err(error),
                Err(_) => Err(E::from(serialized)),
            };
        }
    };
    let serialized = js_value_json(&value);
    serde_json::from_str(&serialized).map_err(|error| {
        E::from(format!(
            "Failed to parse response from command '{command}': {error}"
        ))
    })
}

/// Invokes an archive command without exposing unstructured rejection text.
pub async fn invoke_archive_typed<T>(
    command: &str,
    args: serde_json::Value,
) -> Result<T, arcadestr_core::archive_inspection_contract::ArchiveSelectionError>
where
    T: serde::de::DeserializeOwned + 'static,
{
    use arcadestr_core::archive_inspection_contract::ArchiveSelectionError;

    if !is_tauri_available() {
        return Err(ArchiveSelectionError::Transport);
    }
    let promise = tauri_invoke(command, args).map_err(|diagnostic| {
        web_sys::console::error_1(
            &format!("Archive IPC transport failed for '{command}': {diagnostic}").into(),
        );
        ArchiveSelectionError::Transport
    })?;
    let value = match JsFuture::from(promise).await {
        Ok(value) => value,
        Err(rejection) => {
            let serialized = rejection
                .as_string()
                .unwrap_or_else(|| js_value_json(&rejection));
            return match serde_json::from_str::<ArchiveSelectionError>(&serialized) {
                Ok(error) => Err(error),
                Err(error) => {
                    web_sys::console::error_1(
                        &format!(
                            "Archive IPC error contract mismatch for '{command}': {error}; raw rejection: {serialized}"
                        )
                        .into(),
                    );
                    Err(ArchiveSelectionError::ContractMismatch)
                }
            };
        }
    };
    let serialized = js_value_json(&value);
    serde_json::from_str(&serialized).map_err(|error| {
        web_sys::console::error_1(
            &format!("Archive IPC response contract mismatch for '{command}': {error}").into(),
        );
        ArchiveSelectionError::ContractMismatch
    })
}

fn js_value_json(value: &JsValue) -> String {
    if let Some(value) = value.as_string() {
        serde_json::to_string(&value).unwrap_or_else(|_| "\"IPC failure\"".to_string())
    } else {
        js_sys::JSON::stringify(value)
            .ok()
            .and_then(|value| value.as_string())
            .unwrap_or_else(|| "{}".to_string())
    }
}

/// Invoke a Tauri command that returns no value
#[allow(dead_code)]
pub async fn invoke_void(command: &str, args: serde_json::Value) -> Result<(), String> {
    // Check if Tauri is available
    if !is_tauri_available() {
        return Err(
            "Tauri API not available. Make sure you're running inside a Tauri window.".to_string(),
        );
    }

    let promise = tauri_invoke(command, args)?;

    let _ = JsFuture::from(promise)
        .await
        .map_err(|e| format!("JS error: {:?}", e))?;

    Ok(())
}

// Helper to get window and call __TAURI__.core.invoke
fn tauri_invoke(command: &str, args: serde_json::Value) -> Result<js_sys::Promise, String> {
    // This uses JavaScript to access window.__TAURI__.core.invoke
    let js_code = format!(
        "window.__TAURI__.core.invoke('{}', {})",
        command,
        args.to_string()
    );

    js_sys::eval(&js_code)
        .map(|v| v.unchecked_into::<js_sys::Promise>())
        .map_err(|e| {
            if command == "connect_nwc_wallet" {
                // An eval exception can include source text containing the submitted URI.
                "Failed to submit NWC credentials to the desktop backend.".to_string()
            } else if receipt_private_command(command) {
                "Failed to submit receipt evidence to the desktop backend.".to_string()
            } else {
                format!("Failed to invoke Tauri command: {:?}", e)
            }
        })
}

#[cfg(test)]
mod receipt_privacy_tests {
    #[test]
    fn receipt_commands_do_not_echo_request_or_response_payloads(){
        for command in ["sync_purchase_receipts","import_purchase_receipts","ingest_receipt"] {assert!(super::receipt_private_command(command));}
        assert!(!super::receipt_private_command("get_platform_info"));
    }
}
