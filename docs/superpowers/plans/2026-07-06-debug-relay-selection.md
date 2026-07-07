# Debug Relay Selection Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add developer debug relay selection so Arcadestr can connect only to specified relays for reproducible testing.

**Architecture:** Parse debug relay settings in the desktop crate from CLI, environment, or `settings.json`, then pass the resolved config into `RelayManagerConfig`. The core relay manager owns startup relay replacement and strict discovery blocking so all `NostrClient` instances behave consistently.

**Tech Stack:** Rust, Tauri, Tokio, `url`, `nostr-sdk`, existing `RelayManager`, existing `settings.json`.

---

## File Structure

Modify `core/src/relay_manager.rs`.

Responsibility: Add debug relay config fields, relay URL validation, debug relay startup initialization, and strict `add_discovered_relay` blocking.

Modify `core/src/nostr.rs`.

Responsibility: Ensure `NostrClient` constructors do not re-add default or extra startup relays when debug relays are active. Short-circuit discovery methods when discovery is blocked.

Modify `desktop/src/main.rs`.

Responsibility: Extend persisted settings, parse CLI/env debug relay options, resolve priority order, pass resolved config to every startup `NostrClient`.

No new dependency is needed.

Reason: `desktop` and `core` already depend on `url = "2"`.

---

## Task 1: Core Config And Relay Validation

**Files:**

Modify: `core/src/relay_manager.rs`

- [ ] **Step 1: Add failing tests for config defaults and URL validation**

Add tests inside existing `#[cfg(test)] mod tests` in `core/src/relay_manager.rs`:

```rust
#[test]
fn test_relay_manager_config_default_debug_fields() {
    let config = RelayManagerConfig::default();

    assert_eq!(config.debug_relays, None);
    assert!(!config.block_discovery);
}

#[test]
fn test_normalize_relay_urls_rejects_invalid_values() {
    let err = normalize_relay_urls(vec![
        "".to_string(),
        "https://relay.example.com".to_string(),
        "not a url".to_string(),
    ])
    .expect_err("invalid relays should fail");

    assert!(err.to_string().contains("Invalid relay URL"));
}

#[test]
fn test_normalize_relay_urls_accepts_ws_and_wss_and_deduplicates() {
    let relays = normalize_relay_urls(vec![
        "wss://relay.example.com".to_string(),
        "ws://localhost:8080".to_string(),
        "wss://relay.example.com".to_string(),
    ])
    .expect("valid relays should normalize");

    assert_eq!(
        relays,
        vec![
            "wss://relay.example.com".to_string(),
            "ws://localhost:8080".to_string(),
        ]
    );
}
```

- [ ] **Step 2: Run tests and verify they fail**

Run:

```bash
cargo test -p arcadestr-core relay_manager_config_default_debug_fields
cargo test -p arcadestr-core normalize_relay_urls
```

Expected: fail because `debug_relays`, `block_discovery`, and `normalize_relay_urls` do not exist.

- [ ] **Step 3: Implement config fields, error variant, and validation helper**

In `core/src/relay_manager.rs`, extend `RelayManagerConfig`:

```rust
#[derive(Debug, Clone)]
pub struct RelayManagerConfig {
    pub max_relays: usize,
    pub query_timeout_secs: u64,
    pub connection_poll_timeout_ms: u64,
    pub connection_poll_interval_ms: u64,
    pub debug_relays: Option<Vec<String>>,
    pub block_discovery: bool,
}
```

Extend `Default`:

```rust
impl Default for RelayManagerConfig {
    fn default() -> Self {
        Self {
            max_relays: 100,
            query_timeout_secs: 10,
            connection_poll_timeout_ms: 3000,
            connection_poll_interval_ms: 50,
            debug_relays: None,
            block_discovery: false,
        }
    }
}
```

Extend `RelayManagerError`:

```rust
#[error("Invalid relay URL `{0}`: {1}")]
InvalidRelayUrl(String, String),
```

Add helper near the config/error definitions:

```rust
pub fn normalize_relay_urls(relays: Vec<String>) -> Result<Vec<String>, RelayManagerError> {
    let mut normalized = Vec::new();

    for relay in relays {
        let trimmed = relay.trim();

        if trimmed.is_empty() {
            return Err(RelayManagerError::InvalidRelayUrl(
                relay,
                "empty relay URL".to_string(),
            ));
        }

        let parsed = url::Url::parse(trimmed).map_err(|e| {
            RelayManagerError::InvalidRelayUrl(trimmed.to_string(), e.to_string())
        })?;

        match parsed.scheme() {
            "ws" | "wss" => {}
            scheme => {
                return Err(RelayManagerError::InvalidRelayUrl(
                    trimmed.to_string(),
                    format!("unsupported scheme `{}`", scheme),
                ));
            }
        }

        let relay = parsed.to_string();
        if !normalized.contains(&relay) {
            normalized.push(relay);
        }
    }

    Ok(normalized)
}
```

- [ ] **Step 4: Run tests and verify they pass**

Run:

```bash
cargo test -p arcadestr-core relay_manager_config_default_debug_fields
cargo test -p arcadestr-core normalize_relay_urls
```

Expected: pass.

- [ ] **Step 5: Commit**

```bash
rtk git add core/src/relay_manager.rs
rtk git commit -m "feat: add relay debug config validation"
```

---

## Task 2: RelayManager Debug Startup And Discovery Blocking

**Files:**

Modify: `core/src/relay_manager.rs`

- [ ] **Step 1: Add failing tests for debug startup and blocked discovery**

Add tests in `core/src/relay_manager.rs`:

```rust
#[tokio::test]
async fn test_relay_manager_debug_relays_replace_defaults() {
    let config = RelayManagerConfig {
        debug_relays: Some(vec!["wss://debug.example.com".to_string()]),
        block_discovery: true,
        ..RelayManagerConfig::default()
    };

    let manager = RelayManager::new("debug".to_string(), config, None)
        .await
        .expect("manager should initialize");

    let pool = manager.get_relay_pool().await;
    let mut relays = pool.get_relays().await;
    relays.sort();

    assert_eq!(relays, vec!["wss://debug.example.com".to_string()]);
}

#[tokio::test]
async fn test_relay_manager_block_discovery_ignores_new_relays() {
    let config = RelayManagerConfig {
        debug_relays: Some(vec!["wss://debug.example.com".to_string()]),
        block_discovery: true,
        ..RelayManagerConfig::default()
    };

    let manager = RelayManager::new("blocked".to_string(), config, None)
        .await
        .expect("manager should initialize");

    manager
        .add_discovered_relay("wss://other.example.com".to_string())
        .await
        .expect("blocked discovery should be a no-op");

    let pool = manager.get_relay_pool().await;
    let mut relays = pool.get_relays().await;
    relays.sort();

    assert_eq!(relays, vec!["wss://debug.example.com".to_string()]);
}

#[tokio::test]
async fn test_relay_manager_allow_discovery_adds_new_relays() {
    let config = RelayManagerConfig {
        debug_relays: Some(vec!["wss://debug.example.com".to_string()]),
        block_discovery: false,
        ..RelayManagerConfig::default()
    };

    let manager = RelayManager::new("allowed".to_string(), config, None)
        .await
        .expect("manager should initialize");

    manager
        .add_discovered_relay("wss://other.example.com".to_string())
        .await
        .expect("discovery should be allowed");

    let pool = manager.get_relay_pool().await;
    let mut relays = pool.get_relays().await;
    relays.sort();

    assert_eq!(
        relays,
        vec![
            "wss://debug.example.com".to_string(),
            "wss://other.example.com".to_string(),
        ]
    );
}
```

- [ ] **Step 2: Run tests and verify they fail**

Run:

```bash
cargo test -p arcadestr-core relay_manager_debug_relays_replace_defaults
cargo test -p arcadestr-core relay_manager_block_discovery_ignores_new_relays
cargo test -p arcadestr-core relay_manager_allow_discovery_adds_new_relays
```

Expected: fail because startup still uses defaults and discovery is not blocked.

- [ ] **Step 3: Implement debug relay startup helpers**

Add methods to `impl RelayManager`:

```rust
pub fn blocks_discovery(&self) -> bool {
    self.config.debug_relays.is_some() && self.config.block_discovery
}

pub fn debug_relays(&self) -> Option<Vec<String>> {
    self.config.debug_relays.clone()
}

pub fn has_debug_relays(&self) -> bool {
    self.config.debug_relays.is_some()
}
```

Add private initializer:

```rust
async fn initialize_debug_relays(&self, relays: &[String]) -> Result<(), RelayManagerError> {
    for relay in relays {
        if self
            .pool
            .add_relay(relay.clone(), RelaySource::Default)
            .await
        {
            debug!("Added debug relay: {}", relay);
        }
    }

    self.connect_all_relays().await?;

    Ok(())
}
```

Update `RelayManager::new()`:

```rust
let manager = Self {
    client,
    pool,
    config,
    shutdown: Arc::new(RwLock::new(false)),
    event_sender,
    last_known_states: Arc::new(RwLock::new(std::collections::HashMap::new())),
};

if let Some(debug_relays) = &manager.config.debug_relays {
    let debug_relays = normalize_relay_urls(debug_relays.clone())?;
    manager.initialize_debug_relays(&debug_relays).await?;
} else {
    manager.initialize_default_relays().await?;
}

Ok(manager)
```

- [ ] **Step 4: Guard `add_discovered_relay`**

At the start of `add_discovered_relay`, after the doc comment and before capacity checks, add:

```rust
if self.blocks_discovery() {
    if let Some(debug_relays) = &self.config.debug_relays {
        if !debug_relays.contains(&url) {
            debug!(
                "Discovery is blocked; ignoring relay outside debug set: {}",
                url
            );
            return Ok(());
        }
    }
}
```

- [ ] **Step 5: Run tests and verify they pass**

Run:

```bash
cargo test -p arcadestr-core relay_manager_debug_relays_replace_defaults
cargo test -p arcadestr-core relay_manager_block_discovery_ignores_new_relays
cargo test -p arcadestr-core relay_manager_allow_discovery_adds_new_relays
```

Expected: pass.

- [ ] **Step 6: Commit**

```bash
rtk git add core/src/relay_manager.rs
rtk git commit -m "feat: restrict relay manager debug relays"
```

---

## Task 3: NostrClient Discovery Short-Circuiting

**Files:**

Modify: `core/src/nostr.rs`

- [ ] **Step 1: Add failing tests for constructor and discovery behavior**

Add tests in an appropriate existing native test module in `core/src/nostr.rs`:

```rust
#[tokio::test]
async fn test_nostr_client_debug_relays_skip_constructor_relays() {
    let config = RelayManagerConfig {
        debug_relays: Some(vec!["wss://debug.example.com".to_string()]),
        block_discovery: false,
        ..RelayManagerConfig::default()
    };

    let client = NostrClient::new(
        "debug-client".to_string(),
        vec!["wss://extra.example.com".to_string()],
        Some(config),
    )
    .await
    .expect("client should initialize");

    let manager = client.relay_manager();
    let manager = manager.lock().await;
    let pool = manager.get_relay_pool().await;
    let mut relays = pool.get_relays().await;
    relays.sort();

    assert_eq!(relays, vec!["wss://debug.example.com".to_string()]);
}

#[tokio::test]
async fn test_get_relays_for_pubkey_returns_debug_relays_when_blocked() {
    let db_path = temp_db_path("debug_blocked_relays");
    let cache = RelayCache::new(&db_path).expect("cache should initialize");

    let config = RelayManagerConfig {
        debug_relays: Some(vec!["wss://debug.example.com".to_string()]),
        block_discovery: true,
        ..RelayManagerConfig::default()
    };

    let client = NostrClient::new("debug-discovery".to_string(), vec![], Some(config))
        .await
        .expect("client should initialize");

    let result = client
        .get_relays_for_pubkey(
            "npub180cvv07t6ndx4weylg3jtvsvgp87u4qa3gtlgcy7m0c3sn5zxa9s8my3zg",
            &cache,
            None,
        )
        .await
        .expect("blocked discovery should return debug relays");

    assert_eq!(result.read_relays, vec!["wss://debug.example.com".to_string()]);
    assert_eq!(result.write_relays, vec!["wss://debug.example.com".to_string()]);

    let _ = std::fs::remove_file(db_path);
}
```

Use an existing test helper for `temp_db_path` if available in that test module. If not available in scope, add:

```rust
fn temp_db_path(name: &str) -> std::path::PathBuf {
    let unique = format!(
        "{}_{}_{}.db",
        name,
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system time should be after unix epoch")
            .as_nanos()
    );

    std::env::temp_dir().join(unique)
}
```

- [ ] **Step 2: Run tests and verify they fail**

Run:

```bash
cargo test -p arcadestr-core test_nostr_client_debug_relays_skip_constructor_relays
cargo test -p arcadestr-core test_get_relays_for_pubkey_returns_debug_relays_when_blocked
```

Expected: fail because constructor relays are still added and discovery does not short-circuit.

- [ ] **Step 3: Skip constructor relays when debug relays are configured**

In `NostrClient::new()` and `NostrClient::new_with_cache()`, keep a pre-move flag:

```rust
let config = config.unwrap_or_default();
let has_debug_relays = config.debug_relays.is_some();
```

After creating `relay_manager`, change additional relay injection to:

```rust
if !has_debug_relays {
    for relay in &relays {
        if !DEFAULT_RELAYS.contains(&relay.as_str())
            && !INDEXER_RELAYS.contains(&relay.as_str())
        {
            let _ = relay_manager.add_discovered_relay(relay.clone()).await;
        }
    }
}
```

- [ ] **Step 4: Add blocked-discovery fallback helper**

Add a private helper in `impl NostrClient`:

```rust
async fn blocked_discovery_relays(&self) -> Option<Vec<String>> {
    let manager = self.relay_manager.lock().await;

    if manager.blocks_discovery() {
        return manager.debug_relays();
    }

    None
}
```

- [ ] **Step 5: Short-circuit discovery methods**

At the top of `fetch_profile_with_relay_discovery()`:

```rust
if self.blocked_discovery_relays().await.is_some() {
    tracing::debug!("Relay discovery is blocked; fetching profile without NIP-65 discovery");
    return self.fetch_profile(npub, None).await;
}
```

At the top of `fetch_relay_list()`:

```rust
if self.blocked_discovery_relays().await.is_some() {
    return Err(NostrError::RelayError(
        "relay discovery is blocked by debug relay configuration".to_string(),
    ));
}
```

At the top of `get_relays_for_pubkey()`:

```rust
if let Some(relays) = self.blocked_discovery_relays().await {
    return Ok(RelayDiscoveryResult {
        write_relays: relays.clone(),
        read_relays: relays,
        source: RelayDiscoverySource::GlobalFallback,
    });
}
```

At the top of `get_relays_for_pubkey_with_hints()`:

```rust
if let Some(relays) = self.blocked_discovery_relays().await {
    return RelayDiscoveryResult {
        write_relays: relays.clone(),
        read_relays: relays,
        source: RelayDiscoverySource::GlobalFallback,
    };
}
```

- [ ] **Step 6: Run tests and verify they pass**

Run:

```bash
cargo test -p arcadestr-core test_nostr_client_debug_relays_skip_constructor_relays
cargo test -p arcadestr-core test_get_relays_for_pubkey_returns_debug_relays_when_blocked
```

Expected: pass.

- [ ] **Step 7: Commit**

```bash
rtk git add core/src/nostr.rs
rtk git commit -m "feat: block relay discovery in debug mode"
```

---

## Task 4: Desktop CLI, Environment, And Settings Resolution

**Files:**

Modify: `desktop/src/main.rs`

- [ ] **Step 1: Add failing tests for resolution priority and flags**

Add a test module near existing tests in `desktop/src/main.rs`:

```rust
#[cfg(test)]
mod debug_relay_config_tests {
    use super::*;

    #[test]
    fn test_cli_debug_relays_override_env_and_settings() {
        let settings = NetworkDiscoverySettings {
            allow_insecure_public_ws: false,
            debug_relays: Some(vec!["wss://settings.example.com".to_string()]),
            block_discovery: Some(false),
        };

        let cli = parse_debug_relay_cli_args(vec![
            "--relay".to_string(),
            "wss://cli.example.com".to_string(),
            "--block-discovery".to_string(),
        ])
        .expect("cli should parse");

        let env = parse_debug_relay_env(
            Some("wss://env.example.com".to_string()),
            Some("false".to_string()),
        )
        .expect("env should parse");

        let resolved = resolve_debug_relay_options(cli, env, &settings)
            .expect("options should resolve");

        assert_eq!(resolved.relays, Some(vec!["wss://cli.example.com".to_string()]));
        assert!(resolved.block_discovery);
        assert_eq!(resolved.source, Some(DebugRelayConfigSource::Cli));
    }

    #[test]
    fn test_env_debug_relays_override_settings() {
        let settings = NetworkDiscoverySettings {
            allow_insecure_public_ws: false,
            debug_relays: Some(vec!["wss://settings.example.com".to_string()]),
            block_discovery: Some(false),
        };

        let cli = DebugRelayCliOptions::default();
        let env = parse_debug_relay_env(
            Some("wss://env.example.com".to_string()),
            Some("true".to_string()),
        )
        .expect("env should parse");

        let resolved = resolve_debug_relay_options(cli, env, &settings)
            .expect("options should resolve");

        assert_eq!(resolved.relays, Some(vec!["wss://env.example.com".to_string()]));
        assert!(resolved.block_discovery);
        assert_eq!(resolved.source, Some(DebugRelayConfigSource::Environment));
    }

    #[test]
    fn test_settings_debug_relays_default_to_block_discovery() {
        let settings = NetworkDiscoverySettings {
            allow_insecure_public_ws: false,
            debug_relays: Some(vec!["wss://settings.example.com".to_string()]),
            block_discovery: None,
        };

        let resolved = resolve_debug_relay_options(
            DebugRelayCliOptions::default(),
            DebugRelayEnvOptions::default(),
            &settings,
        )
        .expect("options should resolve");

        assert_eq!(
            resolved.relays,
            Some(vec!["wss://settings.example.com".to_string()])
        );
        assert!(resolved.block_discovery);
        assert_eq!(resolved.source, Some(DebugRelayConfigSource::Settings));
    }

    #[test]
    fn test_allow_discovery_sets_block_discovery_false() {
        let cli = parse_debug_relay_cli_args(vec![
            "--relay".to_string(),
            "wss://cli.example.com".to_string(),
            "--allow-discovery".to_string(),
        ])
        .expect("cli should parse");

        let resolved = resolve_debug_relay_options(
            cli,
            DebugRelayEnvOptions::default(),
            &NetworkDiscoverySettings::default(),
        )
        .expect("options should resolve");

        assert!(!resolved.block_discovery);
    }

    #[test]
    fn test_conflicting_discovery_flags_are_rejected() {
        let err = parse_debug_relay_cli_args(vec![
            "--relay".to_string(),
            "wss://cli.example.com".to_string(),
            "--block-discovery".to_string(),
            "--allow-discovery".to_string(),
        ])
        .expect_err("conflicting flags should fail");

        assert!(err.contains("cannot be used together"));
    }
}
```

- [ ] **Step 2: Run tests and verify they fail**

Run:

```bash
cargo test -p arcadestr-desktop debug_relay_config_tests
```

Expected: fail because the structs and functions do not exist.

- [ ] **Step 3: Extend settings struct**

Change `NetworkDiscoverySettings`:

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
struct NetworkDiscoverySettings {
    #[serde(default)]
    allow_insecure_public_ws: bool,
    #[serde(default)]
    debug_relays: Option<Vec<String>>,
    #[serde(default)]
    block_discovery: Option<bool>,
}
```

Update `Default`:

```rust
impl Default for NetworkDiscoverySettings {
    fn default() -> Self {
        Self {
            allow_insecure_public_ws: false,
            debug_relays: None,
            block_discovery: None,
        }
    }
}
```

- [ ] **Step 4: Add resolution structs and parsers**

Add near `NetworkDiscoverySettings`:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DebugRelayConfigSource {
    Cli,
    Environment,
    Settings,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct DebugRelayCliOptions {
    relays: Vec<String>,
    block_discovery: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct DebugRelayEnvOptions {
    relays: Vec<String>,
    block_discovery: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ResolvedDebugRelayOptions {
    relays: Option<Vec<String>>,
    block_discovery: bool,
    source: Option<DebugRelayConfigSource>,
}
```

Add CLI parser:

```rust
fn parse_debug_relay_cli_args(args: Vec<String>) -> Result<DebugRelayCliOptions, String> {
    let mut relays = Vec::new();
    let mut block_discovery = None;
    let mut iter = args.into_iter();

    while let Some(arg) = iter.next() {
        if arg == "--relay" {
            let relay = iter
                .next()
                .ok_or_else(|| "--relay requires a URL".to_string())?;
            relays.push(relay);
        } else if let Some(relay) = arg.strip_prefix("--relay=") {
            relays.push(relay.to_string());
        } else if arg == "--block-discovery" {
            if block_discovery == Some(false) {
                return Err("--block-discovery and --allow-discovery cannot be used together".to_string());
            }
            block_discovery = Some(true);
        } else if arg == "--allow-discovery" {
            if block_discovery == Some(true) {
                return Err("--block-discovery and --allow-discovery cannot be used together".to_string());
            }
            block_discovery = Some(false);
        }
    }

    Ok(DebugRelayCliOptions {
        relays,
        block_discovery,
    })
}
```

Add env parser:

```rust
fn parse_debug_relay_env(
    relays: Option<String>,
    block_discovery: Option<String>,
) -> Result<DebugRelayEnvOptions, String> {
    let relays = relays
        .map(|value| {
            value
                .split(',')
                .map(str::trim)
                .filter(|relay| !relay.is_empty())
                .map(ToString::to_string)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    let block_discovery = match block_discovery {
        Some(value) => Some(parse_bool_env(&value)?),
        None => None,
    };

    Ok(DebugRelayEnvOptions {
        relays,
        block_discovery,
    })
}

fn parse_bool_env(value: &str) -> Result<bool, String> {
    match value.trim().to_ascii_lowercase().as_str() {
        "true" | "1" | "yes" | "on" => Ok(true),
        "false" | "0" | "no" | "off" => Ok(false),
        _ => Err(format!("invalid boolean value `{}`", value)),
    }
}
```

Add resolver:

```rust
fn resolve_debug_relay_options(
    cli: DebugRelayCliOptions,
    env: DebugRelayEnvOptions,
    settings: &NetworkDiscoverySettings,
) -> Result<ResolvedDebugRelayOptions, String> {
    let (relays, block_discovery, source) = if !cli.relays.is_empty() {
        (
            Some(cli.relays),
            cli.block_discovery.unwrap_or(true),
            Some(DebugRelayConfigSource::Cli),
        )
    } else if !env.relays.is_empty() {
        (
            Some(env.relays),
            env.block_discovery.unwrap_or(true),
            Some(DebugRelayConfigSource::Environment),
        )
    } else if let Some(relays) = &settings.debug_relays {
        (
            Some(relays.clone()),
            settings.block_discovery.unwrap_or(true),
            Some(DebugRelayConfigSource::Settings),
        )
    } else {
        (None, false, None)
    };

    let relays = match relays {
        Some(relays) => Some(
            arcadestr_core::relay_manager::normalize_relay_urls(relays)
                .map_err(|e| e.to_string())?,
        ),
        None => None,
    };

    Ok(ResolvedDebugRelayOptions {
        relays,
        block_discovery,
        source,
    })
}
```

- [ ] **Step 5: Run tests and verify they pass**

Run:

```bash
cargo test -p arcadestr-desktop debug_relay_config_tests
```

Expected: pass.

- [ ] **Step 6: Commit**

```bash
rtk git add desktop/src/main.rs
rtk git commit -m "feat: resolve debug relay settings"
```

---

## Task 5: Wire Debug Relay Config Into Desktop Startup

**Files:**

Modify: `desktop/src/main.rs`

- [ ] **Step 1: Add startup resolution before runtime initialization**

Near the existing startup setup before `runtime.block_on(async { ... })`, add:

```rust
let network_settings = load_network_discovery_settings();

let debug_relay_options = {
    let cli = parse_debug_relay_cli_args(std::env::args().skip(1).collect())
        .unwrap_or_else(|e| {
            eprintln!("Invalid debug relay CLI options: {}", e);
            std::process::exit(2);
        });

    let env = parse_debug_relay_env(
        std::env::var("ARCADESTR_RELAYS").ok(),
        std::env::var("ARCADESTR_BLOCK_DISCOVERY").ok(),
    )
    .unwrap_or_else(|e| {
        eprintln!("Invalid debug relay environment options: {}", e);
        std::process::exit(2);
    });

    resolve_debug_relay_options(cli, env, &network_settings).unwrap_or_else(|e| {
        eprintln!("Invalid debug relay configuration: {}", e);
        std::process::exit(2);
    })
};

if let Some(relays) = &debug_relay_options.relays {
    info!(
        "Debug relay mode active from {:?}: {} relay(s), block_discovery={}",
        debug_relay_options.source,
        relays.len(),
        debug_relay_options.block_discovery
    );
}
```

- [ ] **Step 2: Pass resolved config to main client**

Replace the existing `relay_config` literal with:

```rust
let relay_config = arcadestr_core::relay_manager::RelayManagerConfig {
    max_relays: 100,
    query_timeout_secs: 10,
    connection_poll_timeout_ms: 3000,
    connection_poll_interval_ms: 50,
    debug_relays: debug_relay_options.relays.clone(),
    block_discovery: debug_relay_options.block_discovery,
};
```

Use startup relays only when debug mode is inactive:

```rust
let startup_relays = if debug_relay_options.relays.is_some() {
    vec![]
} else {
    DEFAULT_RELAYS.iter().map(|s| s.to_string()).collect()
};
```

Pass `startup_relays` to the primary `NostrClient::new_with_cache()`.

- [ ] **Step 3: Pass same config to fallback and validator clients**

Use `Some(relay_config.clone())` for every startup `NostrClient::new_with_cache()` in this block.

Primary client:

```rust
Some(relay_config.clone())
```

Failure fallback:

```rust
NostrClient::new_with_cache(
    "default".to_string(),
    vec![],
    cache.clone(),
    Some(relay_config.clone()),
)
.await
.expect("Failed to create empty client")
```

NIP-05 validator client:

```rust
NostrClient::new_with_cache(
    "default".to_string(),
    vec![],
    cache.clone(),
    Some(relay_config.clone()),
)
.await
```

`Arc::try_unwrap` fallback:

```rust
NostrClient::new_with_cache(
    "default".to_string(),
    vec![],
    cache.clone(),
    Some(relay_config.clone()),
)
.await
.expect("Failed to create fallback client")
```

- [ ] **Step 4: Update startup log wording**

Change:

```rust
info!("Connecting to default relays before starting Tauri...");
```

To:

```rust
info!("Connecting to configured relays before starting Tauri...");
```

- [ ] **Step 5: Skip extended network startup when discovery is blocked**

At the beginning of nested `initialize_extended_network(...)`, add:

```rust
{
    let nostr = state.nostr.lock().await;
    let manager = nostr.relay_manager();
    let manager = manager.lock().await;

    if manager.blocks_discovery() {
        info!("Skipping extended network discovery because debug relay discovery is blocked");
        return;
    }
}
```

- [ ] **Step 6: Run focused checks**

Run:

```bash
cargo check -p arcadestr-desktop
cargo test -p arcadestr-desktop debug_relay_config_tests
```

Expected: pass.

- [ ] **Step 7: Commit**

```bash
rtk git add desktop/src/main.rs
rtk git commit -m "feat: wire debug relay startup config"
```

---

## Task 6: Final Verification

**Files:**

Read-only verification across modified files.

- [ ] **Step 1: Format**

Run:

```bash
cargo fmt
```

Expected: completes without errors.

- [ ] **Step 2: Run core relay tests**

Run:

```bash
cargo test -p arcadestr-core relay_manager
cargo test -p arcadestr-core debug_relays
cargo test -p arcadestr-core blocked_discovery
```

Expected: pass.

- [ ] **Step 3: Run desktop config tests**

Run:

```bash
cargo test -p arcadestr-desktop debug_relay_config_tests
```

Expected: pass.

- [ ] **Step 4: Run package checks**

Run:

```bash
cargo check -p arcadestr-core
cargo check -p arcadestr-desktop
```

Expected: pass.

- [ ] **Step 5: Manual smoke test with blocked discovery**

Run:

```bash
cd desktop && timeout 60 cargo tauri dev -- --relay wss://relay.damus.io --block-discovery
```

Expected: logs show debug relay mode active, `block_discovery=true`, and one configured relay.

- [ ] **Step 6: Manual smoke test with discovery allowed**

Run:

```bash
cd desktop && timeout 60 cargo tauri dev -- --relay wss://relay.damus.io --allow-discovery
```

Expected: logs show debug relay mode active, `block_discovery=false`, and startup begins from one configured relay.

- [ ] **Step 7: Commit final formatting if needed**

```bash
rtk git add core/src/relay_manager.rs core/src/nostr.rs desktop/src/main.rs
rtk git commit -m "chore: format debug relay selection"
```

Only commit if `cargo fmt` changed files.

---

## Self-Review

Spec coverage: covered CLI, env, `settings.json`, priority resolution, relay replacement, discovery blocking toggle, URL validation, deduplication, logging, tests, and default behavior preservation.

Placeholder scan: no `TBD`, `TODO`, “fill in”, or unspecified “add tests” steps.

Type consistency: plan consistently uses `debug_relays: Option<Vec<String>>`, `block_discovery: bool` in core config, `block_discovery: Option<bool>` in persisted settings, and `ResolvedDebugRelayOptions` for desktop resolution.
