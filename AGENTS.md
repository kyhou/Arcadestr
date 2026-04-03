# Arcadestr Agent Guidelines

## Build & Development Commands

### Running the Application
```bash
# Development server (with 60s timeout to prevent hanging)
cd /home/joel/Sync/Projetos/Arcadestr/desktop && timeout 60 cargo tauri dev 2>&1

# Build desktop app
cargo build -p arcadestr-desktop

# Check specific crate
cargo check -p arcadestr-core
cargo check -p arcadestr-desktop
cargo check -p arcadestr-app
```

### Testing
```bash
# Run all tests for a crate
cargo test -p arcadestr-core
cargo test -p arcadestr-desktop

# Run a single test (example pattern)
cargo test -p arcadestr-core --lib test_insert_and_query

# Run tests with single thread (for SQLite tests)
cargo test -p arcadestr-core --lib -- --test-threads=1
```

### Linting & Formatting
```bash
# Format code
cargo fmt

# Check with clippy
cargo clippy -p arcadestr-core -- -D warnings

# Fix auto-fixable issues
cargo fix --lib -p arcadestr-core
```

## Code Style Guidelines

### Project Structure
- `/core` - Core business logic (Nostr, NIP-46, storage) - **Library crate**
- `/desktop` - Tauri desktop application - **Binary crate**
- `/app` - Leptos web frontend - **WASM crate**
- `/web` - Web-specific utilities

### Naming Conventions
- **Files**: `snake_case.rs` (e.g., `social_graph.rs`)
- **Structs/Enums**: `PascalCase` (e.g., `SocialGraphDb`, `RelayDiscoveryResult`)
- **Functions/Variables**: `snake_case` (e.g., `insert_batch`, `relay_cache`)
- **Constants**: `SCREAMING_SNAKE_CASE` (e.g., `MAX_HINTS_PER_PUBKEY`)
- **Error types**: Suffix with `Error` (e.g., `SocialGraphError`)
- **Avoid weasel words**: Use `RelayHints` not `RelayHintStore`

### Imports & Organization
```rust
// 1. Standard library
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

// 2. External crates
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tracing::{debug, info, warn};

// 3. Internal crate imports
use crate::relay_cache::RelayCache;
use crate::nostr::{NostrClient, KIND_RELAY_LIST};
```

### Error Handling
- **Libraries (core crate)**: Use `thiserror` with canonical error structs
- **Applications (desktop crate)**: May use `anyhow` for simplicity
- **Never use `.unwrap()` in production code** - use `.expect()` with context or proper error propagation
- **Mutex locks**: Use `.expect("mutex_name mutex poisoned")` instead of `.unwrap()`

### Type Safety
- Use strong types over primitives (avoid primitive obsession)
- Wrap shared state in `Arc<Mutex<T>>` or `Arc<RwLock<T>>`
- Assert Send/Sync for public types: `const _: () = assert_send::<T>();`

### Documentation
```rust
/// Summary sentence < 15 words.
///
/// Extended documentation explaining purpose and behavior.
///
/// # Examples
/// ```rust
/// let result = function_name(arg)?;
/// assert_eq!(result, expected);
/// ```
///
/// # Errors
/// Returns `ErrorType` when condition occurs.
pub fn function_name(arg: Type) -> Result<Type, ErrorType> {
```

### Testing Patterns
```rust
#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_descriptive_name() {
        // Arrange
        let input = setup();
        
        // Act
        let result = function_under_test(input);
        
        // Assert
        assert_eq!(result, expected);
    }
}
```

### Database Testing
- Use `std::env::temp_dir()` for test databases
- Clean up database files in tests
- Use unique paths per test to avoid conflicts

### Async Patterns
- Use `tokio::time::timeout` for network operations
- Spawn background tasks with `tauri::async_runtime::spawn` in Tauri apps
- Don't hold mutex locks across await points

### Feature Gates
- Use `#[cfg(feature = "native")]` for native-only code
- Use `#[cfg(target_arch = "wasm32")]` for WASM-specific code

## External References

- **Microsoft Rust Guidelines**: https://microsoft.github.io/rust-guidelines/
- **Detailed Guidelines**: See `RUST_GUIDELINES.md` in repository root
- **Development Guide**: See `CLAUDE.md` for project-specific workflows

## Git Workflow

```bash
# Stage and commit
rtk git add <files>
rtk git commit -m "type: description"

# Common commit types:
# - feat: new feature
# - fix: bug fix
# - refactor: code restructuring
# - test: adding tests
# - docs: documentation
```

## Rust Guidelines Enforcement

Follow the Microsoft Rust Guidelines:
- https://microsoft.github.io/rust-guidelines/guidelines/index.html

Core rules (must comply):
- Naming conventions
- Error handling patterns (Result, no unwrap in production)
- Ownership and borrowing correctness
- Unsafe usage restrictions
- API design consistency

If a change violates these, reject or rewrite it.

### Protocol Layer ('nostr/')

Each NIP is a standalone 'object':
- Condensed NIP reference docs at '.claude/nips/*.md' with index at '.claude/nips/README.md'

### Relay Layer ('relay/')

- 'Relay' — single WebSocket connection via OkHttp
- 'RelayPool' — connection pooling with persistent/ephemeral split
- 'OutboxRouter' — outbox/inbox routing per NIP-65
- 'RelayScoreBoard' — tracks relay reliability and author coverage
- 'SubscriptionManager' — REQ subscription lifecycle

### Repository Layer ('repo/')

- 'EventRepository' — LRU cache (5,000 events), profile parsing, reaction/repost/zap tracking
- 'ContactRepository' — follow list with SharedPreferences persistence
- 'KeyRepository' — EncryptedSharedPreferences for private keys
- 'DmRepository' — conversation caching with ECDH key cache

## Code Conventions

- Default relays: 'wss://relay.damus.io', 'wss://relay.primal.net'

## Crypto Stack

- **Signing**: secp256k1 (Schnorr) using native bindings (secp256k1 crate or libsecp256k1)
- **NIP-44 encryption**: ECDH + HKDF + XChaCha20-Poly1305 (xchacha20poly1305, hkdf, hmac, sha2 or libsodium)
- **Key storage**: libsecret (Secret Service API) or AES-256-GCM with Argon2-derived key