// Tauri v2 API wrapper for Rust/WASM frontend
// This module provides a bridge to Tauri's JavaScript API

use wasm_bindgen::prelude::*;

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = ["window", "__TAURI__", "core"], js_name = invoke)]
    async fn tauri_invoke(cmd: &str, args: JsValue) -> JsValue;
}

/// Check if running in Tauri environment
pub fn is_tauri() -> bool {
    js_sys::Reflect::get(&js_sys::global(), &JsValue::from_str("__TAURI__"))
        .map(|v| !v.is_undefined())
        .unwrap_or(false)
}

/// Invoke a Tauri command
pub async fn invoke(cmd: &str, args: JsValue) -> Result<JsValue, JsValue> {
    if !is_tauri() {
        return Err(JsValue::from_str("Not running in Tauri environment"));
    }
    
    let promise = tauri_invoke(cmd, args);
    let result = wasm_bindgen_futures::JsFuture::from(js_sys::Promise::from(promise))
        .await
        .map_err(|e| e)?;
    
    Ok(result)
}
