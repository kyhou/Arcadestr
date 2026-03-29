// Tauri v2 invoke bridge for WASM target
// Uses direct JavaScript interop to call Tauri v2's window.__TAURI__.core

use std::future::Future;
use std::pin::Pin;
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

    // Get the response as a string
    let response_str = js_value
        .as_string()
        .ok_or_else(|| "Expected string response".to_string())?;

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
            serde_json::from_str(&format!("\"{}\"", response_str))
                .map_err(|e| format!("Failed to parse response from command '{}': {}. Response was: {}", command, e, &response_str[..response_str.len().min(200)]))
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
