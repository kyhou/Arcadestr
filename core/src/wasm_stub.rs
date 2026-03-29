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
