// WASM-compatible stubs for core functionality.
// These are placeholder implementations for the web target.

/// Placeholder for NOSTR client functionality in WASM.
pub struct NostrClient;

impl NostrClient {
    /// Creates a new NOSTR client instance.
    pub fn new() -> Self {
        Self
    }
}

impl Default for NostrClient {
    fn default() -> Self {
        Self::new()
    }
}

/// Placeholder for Lightning payment functionality in WASM.
pub struct LightningClient;

impl LightningClient {
    /// Creates a new Lightning client instance.
    pub fn new() -> Self {
        Self
    }
}

impl Default for LightningClient {
    fn default() -> Self {
        Self::new()
    }
}

/// Placeholder for signer functionality in WASM.
pub struct Signer;

impl Signer {
    /// Creates a new signer instance.
    pub fn new() -> Self {
        Self
    }
}

impl Default for Signer {
    fn default() -> Self {
        Self::new()
    }
}

/// Placeholder for authentication functionality in WASM.
pub struct AuthManager;

impl AuthManager {
    /// Creates a new auth manager instance.
    pub fn new() -> Self {
        Self
    }
}

impl Default for AuthManager {
    fn default() -> Self {
        Self::new()
    }
}

/// WASM no-op stub for desktop NIP-49 import surface.
pub fn nip49_import(_ncryptsec: &str, _password: &str) -> Result<String, String> {
    Err("NIP-49 import unsupported on wasm target (deferred backend)".to_string())
}

/// WASM no-op stub for desktop NIP-49 export surface.
pub fn nip49_export(_npub: &str, _password: &str) -> Result<String, String> {
    Err("NIP-49 export unsupported on wasm target (deferred backend)".to_string())
}

/// WASM no-op stub for desktop NIP-05 verification surface.
pub fn verify_nip05(_identifier: &str) -> Result<bool, String> {
    Err("NIP-05 verification unsupported on wasm target (deferred backend)".to_string())
}
