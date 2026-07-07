# Debug Relay Selection Design

## Purpose

Add a developer-focused relay selection option so developers can run Arcadestr against a specific relay set. This allows reproducible testing without automatically connecting to the full default, discovery, and indexer relay lists.

## Current Behavior

Relay configuration is hardcoded in `core/src/nostr.rs` through `DEFAULT_RELAYS`, `DISCOVERY_RELAYS`, and `INDEXER_RELAYS`. Startup connects to all of these through `RelayManager`. Runtime discovery can add more relays through NIP-65 relay lists, extended network discovery, relay hints, and fallback behavior.

There is no current CLI flag, environment variable, or persisted setting for restricting relay connections.

## Desired Behavior

When debug relays are configured, they replace all hardcoded default, discovery, and indexer relays for startup. The app connects only to the configured debug relay list initially.

Discovery behavior is controlled separately by a `block_discovery` option:

- `block_discovery = true`: the app must not add relay connections beyond the debug relay list.
- `block_discovery = false`: the app starts with only the debug relay list, but normal runtime discovery may add more relays.

If no debug relay list is configured, existing behavior remains unchanged.

## Configuration Sources

The desktop app resolves debug relay settings from these sources, highest priority first:

1. CLI flags
2. Environment variables
3. `settings.json`
4. Existing hardcoded defaults

CLI example:

```bash
cargo tauri dev -- --relay wss://relay.example --relay wss://other.example --block-discovery
```

CLI example that starts from debug relays but allows normal discovery:

```bash
cargo tauri dev -- --relay wss://relay.example --allow-discovery
```

Environment example:

```bash
ARCADESTR_RELAYS=wss://relay.example,wss://other.example \
ARCADESTR_BLOCK_DISCOVERY=true \
cargo tauri dev
```

`settings.json` example:

```json
{
  "network_discovery": {
    "allow_insecure_public_ws": false
  },
  "debug_relays": ["wss://relay.example"],
  "block_discovery": true
}
```

When `debug_relays` is present and discovery behavior is not specified, `block_discovery` defaults to `true`. This favors deterministic isolated testing. CLI parsing should reject simultaneous `--block-discovery` and `--allow-discovery` flags.

## Core Configuration

Extend `RelayManagerConfig` in `core/src/relay_manager.rs`:

```rust
pub struct RelayManagerConfig {
    pub max_relays: usize,
    pub query_timeout_secs: u64,
    pub connection_poll_timeout_ms: u64,
    pub connection_poll_interval_ms: u64,
    pub debug_relays: Option<Vec<String>>,
    pub block_discovery: bool,
}
```

`RelayManager::new()` uses `debug_relays` to decide startup relays:

- `Some(relays)`: add only those relays to the pool and connect to them.
- `None`: preserve current default, indexer, and discovery relay initialization.

## Discovery Blocking

When `block_discovery` is enabled, code paths that can add or select relays outside the debug list must return the debug relay list or skip work rather than adding new relays. This includes:

- NIP-65 relay list network discovery
- Extended network relay discovery
- Relay hints from events, NIP-19, and NIP-05 when they would add new relay connections
- Global fallback relay selection when it would use hardcoded defaults instead of debug relays

Existing query paths that fetch from an explicitly supplied subset may continue to use that subset only when it is inside the allowed debug relay list.

## Validation And Logging

Relay inputs must be validated before use:

- Reject empty strings.
- Reject malformed URLs.
- Accept only `ws://` and `wss://` schemes.
- Deduplicate repeated URLs while preserving first-seen order.

When debug relays are active, startup logs should include:

- Which source supplied the relay list: CLI, environment, or settings file.
- The number of debug relays.
- Whether discovery is blocked.

## Testing

Add tests for configuration resolution and relay behavior:

- CLI overrides environment and settings.
- Environment overrides settings.
- Settings are used when CLI and environment are absent.
- Invalid relay URLs are rejected.
- Duplicate relay URLs are removed.
- Debug relays replace default, indexer, and discovery relays at startup.
- `block_discovery = true` prevents additional relay connections.
- `block_discovery = false` preserves existing discovery behavior after startup.

## Non-Goals

- Do not add a production relay management UI.
- Do not persist CLI or environment debug relay choices back into `settings.json`.
- Do not change the default relay list when debug relays are absent.
- Do not remove relay cache or relay hint persistence; only prevent their use for adding connections when discovery is blocked.
