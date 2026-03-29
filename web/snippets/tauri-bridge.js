// Tauri v2 API bridge
// This provides compatibility between tauri-wasm and Tauri v2

export function is_tauri() {
    return typeof window !== 'undefined' && 
           typeof window.__TAURI__ !== 'undefined' && 
           typeof window.__TAURI__.core !== 'undefined';
}

const ek = ['', 'Any', 'AnyLabel', 'App', 'Window', 'Webview', 'WebviewWindow'];

export function eargs(event, payload, k, l) {
    let o = { event, payload };
    if (k) {
        o.target = { kind: ek[k] };
        if (l) o.target.label = l;
    }
    return o;
}

// Export invoke function that uses Tauri v2 API
export async function invoke(cmd, args, opts) {
    if (!is_tauri()) {
        throw new Error('Tauri API not available');
    }
    return await window.__TAURI__.core.invoke(cmd, args, opts);
}
