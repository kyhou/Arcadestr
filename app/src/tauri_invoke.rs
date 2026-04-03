// Tauri v2 invoke bridge for WASM target
// Uses direct JavaScript interop to call Tauri v2's window.__TAURI__.core

use wasm_bindgen::prelude::*;
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
            callback(json);
        }
    }) as Box<dyn FnMut(JsValue)>);

    let js_callback = closure.as_ref().unchecked_ref::<js_sys::Function>().clone();
    
    // Call Tauri's listen function and get the unlisten handle
    let js_code = format!(
        "window.__TAURI__.event.listen('{}', {})",
        event,
        js_callback.as_string().unwrap_or_default()
    );

    let unlisten_fn = js_sys::eval(&js_code)
        .map_err(|e| format!("Failed to listen for event '{}': {:?}", event, e))?;
    
    // Create cleanup function
    let cleanup = move || {
        // Call unlisten if available
        if let Ok(unlisten) = unlisten_fn.dyn_into::<js_sys::Function>() {
            let _ = unlisten.call0(&JsValue::NULL);
        }
        // Note: closure is still leaked here, but at least we unlisten
    };
    
    closure.forget(); // Still needed for now, but we have unlisten
    
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
    let js_value = JsFuture::from(promise)
        .await
        .map_err(|e| format!("JS error: {:?}", e))?;

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
    web_sys::console::log_1(&format!("Tauri response for '{}': {}", command, response_str).into());

    // Try to parse as JSON first, then as plain string
    // Some commands return JSON, others return plain strings (like npub)
    let result: Result<T, _> = serde_json::from_str(&response_str);

    match result {
        Ok(value) => Ok(value),
        Err(_) => {
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
        .map_err(|e| format!("Failed to invoke Tauri command: {:?}", e))
}
