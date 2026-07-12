# ADP Gate 5 Download Flow Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement ADP download/install flow through verified artifact persistence and library display.

**Architecture:** `core::adp_client` owns protocol download mechanics; desktop Tauri commands own app orchestration, token lookup, fresh listing lookup, ownership preflight, destination path, hash verification, DB recording, and events. Leptos consumes installed-game state only; archive extraction and launch are explicitly out of scope for this gate.

**Tech Stack:** Rust, Tauri, Leptos, SQLite/sqlx, reqwest/http abstraction, NIP-98/Nostr signing.

---

## Scope Locks

- Gate 5 installs a **verified downloaded artifact plus metadata only**.
- No archive extraction.
- No launcher/runtime integration.
- Hash mismatch behavior is **quarantine**, not delete: keep the bad artifact for debugging by moving/renaming it to a deterministic corrupt-artifact location such as `<dest>.corrupt` or an app-data `corrupt/` subdirectory, then return a clear hash mismatch error and do not record an installed game.
- `install_game` must **always re-fetch the listing event fresh** before resolving ADP download metadata or delegation-sensitive ownership state. It must not trust stale UI-cached listing JSON for Gate 5 control decisions.
- `install_game` must preflight local ownership before any ADP download request. If the buyer has no valid local receipt for the coordinate, fail clearly with a client-side `purchase not found` / `you do not own this game` error and do not fall through to Path B/NIP-98.
- Gate 5 live purchase setup uses the Gate 4 fixture/manual-preimage Path B purchase flow (`gen_test_invoice`/manual preimage style), not live NWC wallet chasing.

## Files and Responsibilities

- Modify `core/src/adp_client.rs`
  - Implement `DownloadAuth::{Token,Nip98}` in `AdpClient::download()`.
  - Stream `GET /game/:coordinate` to disk.
  - Surface 403/451 distinctly.
  - Invoke progress callback with downloaded byte count and optional content length.

- Modify `core/src/http_client.rs`
  - Add the smallest download-capable HTTP seam needed by `AdpClient::download()` while preserving existing JSON helpers.

- Modify `core/src/test_helpers/http_mocks.rs`
  - Add byte-download mocking for success, corrupt/truncated payloads, 403, and 451.

- Modify `desktop/src/main.rs`
  - Replace current `install_game` ownership-check stub.
  - Add `get_installed_games` Tauri command.
  - Register new command.

- Modify `desktop/src/adp_commands.rs` if shared ADP command helpers belong there instead of `main.rs`
  - Reuse Gate 4 listing fetch/token/coordinate patterns.
  - Do not duplicate receipt/delegation validation logic.

- Modify `app/src/tauri_bridge.rs`
  - Add `InstalledGame` response type.
  - Add `invoke_get_installed_games`.
  - Add download event listener wrappers if needed by UI.

- Modify `app/src/ui_v2/views/library.rs`
  - Load installed games from `get_installed_games`.
  - Show installed artifact metadata.
  - Keep action state as “Installed”; no launch behavior.

- Modify `app/src/ui_v2/views/game_detail.rs`
  - Ensure the install button calls the rewritten `install_game`.
  - Reflect installed state after `download-complete`.

---

## Task 1: Core ADP Download Client

**Files:**
- Modify: `core/src/adp_client.rs`
- Modify: `core/src/http_client.rs`
- Modify: `core/src/test_helpers/http_mocks.rs`

- [ ] **Step 1: Write token-download test**

Add a core test named `download_with_token_streams_file_and_reports_progress` that arranges a mocked `GET /game/<coordinate>?token=<token>` byte response, calls `AdpClient::download(coordinate, DownloadAuth::Token(token), dest, progress_cb)`, and asserts:

```rust
assert_eq!(std::fs::read(&dest).expect("downloaded file should exist"), expected_bytes);
assert_eq!(outcome.bytes_written, expected_bytes.len() as u64);
assert!(progress_events.iter().any(|event| event.0 == expected_bytes.len() as u64));
```

- [ ] **Step 2: Run token-download test and verify it fails**

```bash
cargo test -p arcadestr-core --features native download_with_token_streams_file_and_reports_progress
```

Expected: fails because `AdpClient::download()` still returns `NotImplemented` or lacks byte download plumbing.

- [ ] **Step 3: Write NIP-98 download test**

Add a core test named `download_with_nip98_sets_authorization_header` that calls `AdpClient::download(coordinate, DownloadAuth::Nip98 { signer }, dest, progress_cb)` and asserts the mock saw an `Authorization: Nostr <token>` header for `GET /game/<coordinate>` without a token query parameter.

- [ ] **Step 4: Run NIP-98 test and verify it fails**

```bash
cargo test -p arcadestr-core --features native download_with_nip98_sets_authorization_header
```

Expected: fails until Path B auth header generation is implemented.

- [ ] **Step 5: Write status-mapping tests**

Add tests:

```rust
#[tokio::test]
async fn download_403_returns_ownership_error() { /* mocked 403 */ }

#[tokio::test]
async fn download_451_returns_distribution_error() { /* mocked 451 */ }
```

Assert their stringified errors contain distinct user-facing meanings:

```rust
assert!(err.to_string().contains("ownership") || err.to_string().contains("own"));
assert!(err.to_string().contains("no longer distributes") || err.to_string().contains("authorized"));
```

- [ ] **Step 6: Implement minimal download plumbing**

Implement the smallest reusable byte-download seam and `AdpClient::download()` behavior:

```rust
pub async fn download(
    &self,
    game_coordinate: &str,
    auth: DownloadAuth,
    dest: &Path,
    mut on_progress: impl FnMut(u64, Option<u64>),
) -> Result<DownloadOutcome, AdpClientError> {
    let url = match &auth {
        DownloadAuth::Token(token) => self.url(&format!("/game/{game_coordinate}?token={token}")),
        DownloadAuth::Nip98 { .. } => self.url(&format!("/game/{game_coordinate}")),
    };
    let headers = /* token path empty; NIP-98 path has Authorization */;
    let outcome = self.http.download_to_path(&url, headers, dest, |bytes, total| {
        on_progress(bytes, total);
    }).await?;
    Ok(DownloadOutcome { bytes_written: outcome.bytes_written })
}
```

Use actual project trait/error names when editing. Preserve existing JSON methods.

- [ ] **Step 7: Run core download tests**

```bash
cargo test -p arcadestr-core --features native adp_client::tests::download
cargo check -p arcadestr-core --features native
```

Expected: all new download tests pass; core check exits 0.

---

## Task 2: Client-Side Hash Verification and Quarantine

**Files:**
- Modify: `desktop/src/main.rs` or a focused helper module if introduced during implementation
- Test: `desktop/src/main.rs` tests or existing desktop command test module

- [ ] **Step 1: Write hash match install test**

Add a desktop test named `install_records_game_when_hash_matches` that arranges:

```rust
let expected_bytes = b"verified artifact bytes";
let expected_hash = sha256_hex(expected_bytes);
```

Then calls the install orchestration against a mocked download response and asserts `InstalledGamesRepository::get(coordinate)` returns a row with:

```rust
assert_eq!(installed.file_hash, expected_hash);
assert_eq!(installed.server_url, server_url);
assert!(installed.file_path.exists());
```

- [ ] **Step 2: Write hash mismatch quarantine test**

Add a desktop test named `install_quarantines_hash_mismatch_without_recording_installed_game` that arranges listing `file_hash` for `expected_bytes` but mocked download bytes as `b"corrupt bytes"`. Assert:

```rust
assert!(err.to_string().contains("hash"));
assert!(repo.get(coordinate).await.expect("lookup should work").is_none());
assert!(quarantine_path.exists());
assert!(!final_install_path.exists());
```

- [ ] **Step 3: Run tests and verify they fail**

```bash
cargo test -p arcadestr-desktop install_records_game_when_hash_matches
cargo test -p arcadestr-desktop install_quarantines_hash_mismatch_without_recording_installed_game
```

Expected: fail until install orchestration and quarantine behavior exist.

- [ ] **Step 4: Implement hash verification and quarantine**

After `AdpClient::download()` completes:

```rust
let actual_hash = sha256_file(&dest_path).await?;
if actual_hash != expected_hash {
    let quarantine_path = corrupt_artifact_path(&dest_path);
    tokio::fs::rename(&dest_path, &quarantine_path).await?;
    return Err(format!(
        "downloaded file hash mismatch: expected {expected_hash}, got {actual_hash}; quarantined at {}",
        quarantine_path.display()
    ));
}
```

Do not record `installed_games` when this branch is taken.

- [ ] **Step 5: Run hash tests**

```bash
cargo test -p arcadestr-desktop install_records_game_when_hash_matches
cargo test -p arcadestr-desktop install_quarantines_hash_mismatch_without_recording_installed_game
```

Expected: both pass.

---

## Task 3: Rewrite `install_game`

**Files:**
- Modify: `desktop/src/main.rs`
- Possibly modify: `desktop/src/adp_commands.rs`

- [ ] **Step 1: Write unpurchased preflight test**

Add `install_game_unpurchased_listing_fails_before_download_request`. Arrange no local receipt for the buyer/coordinate and a mock ADP client that would panic or count calls if download is attempted. Assert:

```rust
assert!(err.to_string().contains("purchase") || err.to_string().contains("own"));
assert_eq!(mock_download_call_count(), 0);
```

- [ ] **Step 2: Write Path A token-cache test**

Add `install_game_uses_cached_token_path_a`. Arrange:

```rust
DownloadTokensRepository::upsert(&DownloadToken {
    game_coordinate: coordinate.to_string(),
    server_url: server_url.to_string(),
    token: "token-path-a".to_string(),
    expires_at: now + 3600,
}).await?;
```

Assert the mock download URL includes `?token=token-path-a`.

- [ ] **Step 3: Write Path B no-token test**

Add `install_game_without_token_uses_nip98_path_b`. Arrange valid local receipt/ownership but no `download_tokens` row. Assert the mock download call has NIP-98 authorization and no token query string.

- [ ] **Step 4: Write fresh listing re-fetch test**

Add `install_game_refetches_listing_event_before_download`. Provide stale UI listing metadata that conflicts with relay/mock fresh listing metadata. Assert `install_game` uses fresh listing `server_url` and `file_hash`, not stale UI data.

- [ ] **Step 5: Run tests and verify they fail**

```bash
cargo test -p arcadestr-desktop install_game_unpurchased_listing_fails_before_download_request
cargo test -p arcadestr-desktop install_game_uses_cached_token_path_a
cargo test -p arcadestr-desktop install_game_without_token_uses_nip98_path_b
cargo test -p arcadestr-desktop install_game_refetches_listing_event_before_download
```

Expected: fail against current ownership-check-only stub.

- [ ] **Step 6: Implement `install_game` flow**

Implementation order:

1. Authenticate buyer and derive buyer pubkey.
2. Build coordinate from `AppGameListing`.
3. Preflight `state.purchases.is_owned(&buyer_pubkey_hex, &coordinate)`.
4. If not owned, return clear client-side purchase/ownership error before ADP network call.
5. Always re-fetch fresh listing event for the coordinate.
6. Extract ADP tags: `server_url`, `file_hash`, `version`.
7. Look up valid `download_tokens` for `(coordinate, server_url)`.
8. Select `DownloadAuth::Token` on hit; `DownloadAuth::Nip98` on miss.
9. Download to deterministic app-data artifact path.
10. Emit `download-progress` from the progress callback.
11. Hash verify and quarantine on mismatch.
12. Record `InstalledGame`.
13. Emit `download-complete`.

- [ ] **Step 7: Run install tests and desktop check**

```bash
cargo test -p arcadestr-desktop install_game_unpurchased_listing_fails_before_download_request
cargo test -p arcadestr-desktop install_game_uses_cached_token_path_a
cargo test -p arcadestr-desktop install_game_without_token_uses_nip98_path_b
cargo test -p arcadestr-desktop install_game_refetches_listing_event_before_download
cargo check -p arcadestr-desktop
```

Expected: all pass; desktop check exits 0.

---

## Task 4: Add `get_installed_games`

**Files:**
- Modify: `desktop/src/main.rs`
- Modify: `app/src/tauri_bridge.rs`

- [ ] **Step 1: Write desktop command test**

Add `get_installed_games_returns_recorded_installs`. Insert two `InstalledGame` rows with different `installed_at` values. Assert the command returns both in newest-first order.

- [ ] **Step 2: Run test and verify it fails**

```bash
cargo test -p arcadestr-desktop get_installed_games_returns_recorded_installs
```

Expected: fails because command is missing.

- [ ] **Step 3: Implement command**

Add Tauri command:

```rust
#[tauri::command]
async fn get_installed_games(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<arcadestr_core::adp_storage::InstalledGame>, String> {
    let repo = arcadestr_core::adp_storage::InstalledGamesRepository::new(
        state.database.pool().clone(),
    );
    repo.list().await.map_err(|error| error.to_string())
}
```

Register it in the Tauri invoke handler.

- [ ] **Step 4: Add frontend bridge type/wrapper**

Add a serializable bridge struct matching the command response and:

```rust
#[cfg(not(feature = "web"))]
pub async fn invoke_get_installed_games() -> Result<Vec<InstalledGame>, String> {
    crate::tauri_invoke::invoke("get_installed_games", serde_json::json!({})).await
}

#[cfg(feature = "web")]
pub async fn invoke_get_installed_games() -> Result<Vec<InstalledGame>, String> {
    Ok(Vec::new())
}
```

- [ ] **Step 5: Validate command and bridge**

```bash
cargo test -p arcadestr-desktop get_installed_games_returns_recorded_installs
cargo check -p arcadestr-app --target wasm32-unknown-unknown --features web
```

Expected: both pass.

---

## Task 5: Library View Integration

**Files:**
- Modify: `app/src/ui_v2/views/library.rs`
- Modify: `app/src/tauri_bridge.rs` if event wrappers are needed
- Modify: `app/src/ui_v2/views/game_detail.rs`

- [ ] **Step 1: Add installed-games load state**

In `LibraryView`, create a resource/action that calls `invoke_get_installed_games()` on mount.

- [ ] **Step 2: Render empty state**

When no installed games are returned, render a clear empty state such as:

```rust
<p class="v2-social-meta">"No installed games yet. Buy and install a game to see it here."</p>
```

- [ ] **Step 3: Render installed rows/cards**

For each installed game, render coordinate, version, server URL, file hash, and local file path. The action button text must be `Installed`, not `Launch`.

- [ ] **Step 4: Reflect install completion in detail view**

Ensure `game_detail.rs` updates the install button/status after `install_game` succeeds or after `download-complete` is observed.

- [ ] **Step 5: Validate frontend compile**

```bash
cargo check -p arcadestr-app --target wasm32-unknown-unknown --features web
cargo check -p arcadestr-desktop
```

Expected: both pass.

- [ ] **Step 6: Record UI automation limitation**

If `webkit2gtk-driver` is still unavailable, record UI automation as disclosed debt and rely on compile checks plus manual desktop observation if runtime is available.

---

## Task 6: Live Gate 5 Proof

**Files:**
- Modify/add ignored desktop integration test near existing ADP live harnesses.

- [ ] **Step 1: Use Gate 4 fixture/manual-preimage purchase setup**

Use the same fixture/manual preimage purchase flow proven in Gate 4. Do not attempt live NWC wallet Path A; that remains external-wallet/backend debt.

- [ ] **Step 2: Live Path A install**

With a fresh purchase token present, call real `install_game` and assert:

```text
ADP_GATE5_PATH_A_INSTALL_OK=true
artifact exists=true
sha256 == listing file_hash
installed_games row present=true
get_installed_games includes coordinate=true
```

- [ ] **Step 3: Live Path B install**

Delete the local `download_tokens` row for the coordinate, keep the valid local receipt, call `install_game` again, and assert:

```text
ADP_GATE5_PATH_B_NIP98_INSTALL_OK=true
artifact exists=true
sha256 == listing file_hash
installed_games row present=true
```

This explicitly confirms Gate 5's portable/cross-machine route rather than re-proving only raw token download mechanics.

- [ ] **Step 4: Run ignored live test**

```bash
ADP_TEST_SERVER_URL=http://localhost:9099 \
ARCADESTR_RELAYS=ws://localhost:10547,ws://localhost:10548 \
ARCADESTR_BLOCK_DISCOVERY=1 \
cargo test -p arcadestr-desktop --ignored live_install_game_path_a_and_path_b -- --nocapture
```

Expected: Path A and Path B install both succeed. If live infra fails, stop and report whether the failure is client logic or environment/backend debt.

---

## Task 7: Final Verification and STOP Report

- [ ] **Step 1: Run unit/command checks**

```bash
cargo test -p arcadestr-core --features native adp_client::tests
cargo test -p arcadestr-core --features native adp_storage::tests
cargo test -p arcadestr-desktop install_game
cargo test -p arcadestr-desktop get_installed_games
cargo check -p arcadestr-core --features native
cargo check -p arcadestr-desktop
cargo check -p arcadestr-app --target wasm32-unknown-unknown --features web
```

- [ ] **Step 2: Run narrowed clippy and report changed-file diagnostics**

```bash
cargo clippy -p arcadestr-core --features native --message-format=json -- -D warnings
cargo clippy -p arcadestr-desktop --message-format=json
```

Report target diagnostics for:

- `core/src/adp_client.rs`
- `core/src/http_client.rs`
- `core/src/test_helpers/http_mocks.rs`
- `desktop/src/main.rs`
- `desktop/src/adp_commands.rs`
- `app/src/tauri_bridge.rs`
- `app/src/ui_v2/views/library.rs`
- `app/src/ui_v2/views/game_detail.rs`

- [ ] **Step 3: STOP after Gate 5**

Report:

- Path A install result.
- Path B install result.
- hash verification evidence.
- quarantine behavior test result.
- unpurchased preflight test result.
- fresh listing re-fetch test result.
- `installed_games` row evidence.
- `get_installed_games` / library view status.
- UI automation limitation, if still applicable.

Do not proceed to extraction or launch work.
