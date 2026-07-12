# Arcadestr Client — ADP-01 / NIP-102 Integration Spec v1

Status: draft, for review before roadmap/completion-prompt generation
Depends on: `ADP-01.md`, `NIP-102.md`, `adp-server` (implemented, reference impl available)
Supersedes: all prior Arcadestr publish/buy/install notes in `ARCADESTR-CODEBASE.md` — that
document predates ADP-01 and NIP-102 and should be treated as historical only for this feature.

## 0. Scope & Locked Decisions

This spec covers the Arcadestr desktop client changes needed to take a listing from "published
with no distribution" to "buyer owns it and has it installed," against a real `adp-server`.

Locked for v1 (per Joel):
- **Fulfillment keys are operator-delegated.** Every publish that wants fulfillment support goes
  through `POST /provision` + a locally-constructed `kind:30406` acceptance. Direct developer-key
  signing is not the v1 path (the server and protocol both still support it, so it's a cheap
  fallback later, but the client UI doesn't need to special-case it now).
- **Server selection is manual per publish.** The developer types an ADP server URL into the
  publish form. The client does a live `GET <url>/.well-known/adp` reachability check before
  allowing submit, per ADP-01 §"Mandatory fields for client-published listings" step 3. No
  hardcoded default server, no server directory/discovery UI in this pass.
- **Buy flow goes all the way to bytes on disk.** Not just receipt + token — the client also
  performs the authenticated `GET /game/:coordinate` download and streams it to disk. Archive
  extraction and "launch" are explicitly out of scope for this pass (tracked separately, as
  before).

Explicitly **not** decided yet, flagged as open assumptions in §8 — proceeding with a stated
default for each so this doesn't block drafting, but call these out for correction:
- How the buyer obtains a **payment preimage** after paying the bolt11 invoice (WebLN vs manual).
- Whether `core::lightning`'s existing NIP-57 zap machinery is kept for anything else (badges,
  social zaps) alongside the new LNURL-pay-direct path ADP-01 requires for purchases.

## 1. New Core Modules

All new, native-only (`#[cfg(feature = "native")]`), added under `core/src/`:

### 1.1 `core::nip98_client`
Client-side counterpart to `adp-core`'s `nip98.rs` verifier. Builds and signs a `kind:27235`
event through whatever signer is active (NIP-46 or NIP-07 — goes through the existing
`NostrSigner` trait, see §3), then base64-encodes it into the `Authorization: Nostr <b64>` header
value ADP-01 expects on every authenticated endpoint.

```rust
pub async fn build_nip98_auth_header(
    signer: &dyn NostrSigner,
    url: &str,
    method: &str,
) -> Result<String, Nip98ClientError>;
```

Tags to set: `u` (exact URL, no trailing-slash mismatch with the server's expectation — match
whatever `adp-core::nip98::verify_nip98` normalizes to), `method` (uppercase). Timestamp is
`now()`; the 60s clock-skew tolerance is server-enforced, client has no work to do there beyond
not caching/reusing a stale token.

### 1.2 `core::adp_client`
Typed HTTP client wrapping every ADP-01 endpoint. One struct per server the client talks to
(constructed fresh per call with a `base_url`, not a long-lived connection):

```rust
pub struct AdpClient { base_url: String, http: Arc<dyn HttpClient> }

impl AdpClient {
    pub async fn well_known(&self) -> Result<AdpServerInfo, AdpClientError>;
    pub async fn provision(&self, signer: &dyn NostrSigner, scope: Option<&str>)
        -> Result<ProvisionResponse, AdpClientError>;
    pub async fn provision_revoke(&self, signer: &dyn NostrSigner, fulfillment_pubkey: &str)
        -> Result<RevokeResponse, AdpClientError>;
    pub async fn upload(&self, signer: &dyn NostrSigner, listing_event: &Event, file_path: &Path)
        -> Result<UploadResponse, AdpClientError>;
    pub async fn purchase_confirm(&self, signer: &dyn NostrSigner, req: PurchaseConfirmRequest)
        -> Result<PurchaseConfirmResponse, AdpClientError>;
    pub async fn download(&self, game_coordinate: &str, auth: DownloadAuth, dest: &Path,
        on_progress: impl FnMut(u64, Option<u64>))
        -> Result<DownloadOutcome, AdpClientError>;
}
```

`AdpServerInfo`, `ProvisionResponse`, etc. are 1:1 with the JSON shapes already documented in
`ADP-01.md` §"HTTP Endpoints" — no new schema design needed there, just typed deserialization.

`DownloadAuth` is an enum `Token(String) | Nip98` so the same `download()` covers both Path A
(fast, token from a same-session purchase) and Path B (portable, receipt-based reinstall on a
listing whose original purchasing server may be gone — client falls back to Path B whenever no
locally-stored token exists for that coordinate, e.g. after reinstall on a new machine).

Uses `reqwest` (already a dependency) with streaming bodies both directions: multipart streaming
upload for `/upload`, and `bytes_stream()` consumption for `/game/:coordinate` so large archives
don't buffer fully in memory.

### 1.3 `core::lnurlp`
LNURL-pay resolution and invoice request, resolved **only** from the listing's `lud16` tag per
ADP-01's LSP Verification section — never the developer's `kind:0` profile. This is new,
independent of whatever `core::lightning` currently does for zaps.

```rust
pub struct LnurlPayEndpoint {
    pub callback: String,
    pub min_sendable_msat: u64,
    pub max_sendable_msat: u64,
    pub nostr_pubkey: Option<String>, // for LSP verification cross-check, informational client-side
}

pub async fn resolve_lud16(http: &dyn HttpClient, lud16: &str) -> Result<LnurlPayEndpoint, LnurlError>;
pub async fn request_invoice(http: &dyn HttpClient, endpoint: &LnurlPayEndpoint, amount_msat: u64)
    -> Result<String /* bolt11 */, LnurlError>;
```

Mirrors `adp-server`'s own `lnurl.rs` resolution logic (same `.well-known/lnurlp/<name>` path
construction) so client and server agree on what "the LSP" means for a given listing.

### 1.4 `core::file_hash`
Streaming SHA-256 of a local file, used at publish time to compute `file_hash` client-side (per
ADP-01's publishing-client guidance: never accept a hand-typed hash).

```rust
pub async fn sha256_file(path: &Path) -> Result<String /* hex */, std::io::Error>;
```

Chunked read (e.g. 1MB buffer) so this doesn't load full game archives into memory.

### 1.5 `core::install`
Owns the on-disk layout for installed games and the `installed_games` table (§2.3).

```rust
pub struct InstalledGamesRepository { pool: SqlitePool }

impl InstalledGamesRepository {
    pub async fn record(&self, entry: &InstalledGame) -> Result<(), sqlx::Error>;
    pub async fn get(&self, coordinate: &str) -> Result<Option<InstalledGame>, sqlx::Error>;
    pub async fn list(&self) -> Result<Vec<InstalledGame>, sqlx::Error>;
}
```

Install directory: `{data_local_dir}/arcadestr/games/{sha256_short}/`, filename preserved from
the `Content-Disposition` header if present, else derived from the coordinate's `d`-tag.

## 2. Data Model Changes

Three new SQLite migrations in `core/migrations/`:

### 2.1 `004_adp_provisioning.sql`
Tracks the developer's own provisioning relationships so re-publishing to the same
`(server, scope)` doesn't re-provision every time — mirrors the idempotency `POST /provision`
already guarantees server-side, but the client still needs to remember which fulfillment key it
already has so it can populate the listing's `fulfillment_pubkey` tag without a network round
trip on every publish.

```sql
CREATE TABLE adp_provisioning (
    id TEXT PRIMARY KEY,               -- ULID
    developer_npub TEXT NOT NULL,
    server_url TEXT NOT NULL,
    operator_pubkey TEXT NOT NULL,     -- from /.well-known/adp
    scope TEXT,                        -- NULL = unscoped
    fulfillment_pubkey TEXT NOT NULL,
    attestation_event_id TEXT NOT NULL,
    acceptance_event_id TEXT NOT NULL, -- kind:30406 published by this client
    valid_from INTEGER NOT NULL,
    revoked_at INTEGER,
    created_at INTEGER NOT NULL
);
CREATE UNIQUE INDEX idx_adp_provisioning_active
    ON adp_provisioning(developer_npub, server_url, scope)
    WHERE revoked_at IS NULL;
```

### 2.2 `005_download_tokens.sql`
Client-side cache of tokens returned by `/purchase/confirm`, so a same-session install can use
fast Path A without an extra relay round trip. Purely a performance cache — losing this table
just means falling back to Path B (still correct, just slower).

```sql
CREATE TABLE download_tokens (
    game_coordinate TEXT NOT NULL,
    server_url TEXT NOT NULL,
    token TEXT NOT NULL,
    expires_at INTEGER NOT NULL,
    PRIMARY KEY (game_coordinate, server_url)
);
```

### 2.3 `006_installed_games.sql`
```sql
CREATE TABLE installed_games (
    game_coordinate TEXT PRIMARY KEY,
    file_path TEXT NOT NULL,
    file_hash TEXT NOT NULL,
    version TEXT,
    server_url TEXT NOT NULL,          -- server actually served the bytes
    installed_at INTEGER NOT NULL
);
```

## 3. Signer Trait — Confirmed, No Changes Needed

`core::signers::NostrSigner` (native, defined in `nip46.rs`, re-exported via `mod.rs`) is:

```rust
#[async_trait::async_trait]
pub trait NostrSigner: Send + Sync {
    async fn get_public_key(&self) -> Result<PublicKey, SignerError>;
    async fn sign_event(&self, unsigned: UnsignedEvent) -> Result<Event, SignerError>;
}
```

Confirmed against the actual current code: `sign_event` already takes an arbitrary
`UnsignedEvent` with no kind restriction. `core::nip98_client` (§1.1) needs no trait change —
it just builds a `kind:27235` `UnsignedEvent` (tags: `u`, `method`) and calls the existing
`sign_event` through whichever `NostrSigner` impl (`LocalSigner`, `Nip46Signer`,
`LazyNip46Signer`) is active. Same applies to `core::nwc_client` (§8) for signing/encrypting the
`kind:23194` NWC request event — no new trait surface required there either.

## 4. Publish Flow

Replaces the current (obsolete) publish flow entirely. Sequenced because `/upload` re-fetches the
listing from relays and compares — the listing MUST be published and visible on relays before
upload is attempted, not just constructed locally.

```
1.  Developer fills publish form: title, description, price, platform tags, lud16,
    server URL, build file (local path), version string.
2.  [client] GET {server}/.well-known/adp
       → fail: block submit, show "server unreachable"
       → success: capture operator_pubkey, adp_version
3.  [client] sha256_file(build_file) → file_hash              (background task, show spinner)
4.  [client] check adp_provisioning table for
       (developer_npub, server_url, scope=listing d-tag) with revoked_at IS NULL
       → hit: reuse fulfillment_pubkey, skip to step 7
       → miss: continue to step 5
5.  [client] POST {server}/provision  { scope: <d-tag> }, NIP-98 auth (developer signer)
       → returns { fulfillment_pubkey, attestation_event_id, scope }
6.  [client] construct kind:30406 acceptance:
       d = "{operator_pubkey}:{fulfillment_pubkey}"
       p = operator_pubkey
       fulfillment_pubkey = fulfillment_pubkey
       sign (developer signer) + publish to relays
       persist row in adp_provisioning
7.  [client] construct kind:30402 listing event with tags:
       existing NIP-99 tags (d, title, price, platform, etc.)
       + ["server", server_url]
       + ["file_hash", file_hash]
       + ["version", version]
       + ["fulfillment_pubkey", fulfillment_pubkey, valid_from, ""]
       + ["lud16", lud16]
     sign (developer signer) + publish to relays
8.  [client] confirm relay propagation: re-fetch the just-published listing from ≥2 relays
     before proceeding (reuse the existing multi-relay fetch path from marketplace fetch code) —
     do not just sleep-and-hope. Small bounded retry loop, surfaced to UI as "confirming
     publish...".
9.  [client] POST {server}/upload
       multipart: listing_event (the exact signed JSON from step 7), file (streamed)
       NIP-98 auth (developer signer)
       → returns { game_coordinate, file_hash, download_url }
10. [UI] success state: show game_coordinate, download_url, link to view listing
```

Failure handling: steps 5–9 are each independently retryable from where they left off (the
provisioning check in step 4 and the idempotent `/provision` endpoint mean re-running steps 5–6
is safe; re-publishing the same listing content in step 7 is also safe since it's just a newer
replaceable event). The UI should expose a per-step status list (à la a checklist/stepper) rather
than a single spinner, so a failure at step 9 doesn't force redoing 5–8.

## 5. Buy Flow

```
1.  Buyer views listing (existing marketplace/detail view), clicks Buy.
2.  [client] resolve_lud16(listing.lud16) → LnurlPayEndpoint
3.  [client] request_invoice(endpoint, listing.price_sats * 1000) → bolt11
4.  [UI] show bolt11 as QR + copy button + "Open in wallet" (existing pattern)
5.  If buyer has NWC connected: [client] send `pay_invoice` over the buyer's NWC connection
       (`core::nwc_client`), await `kind:23195` response.
       If not connected: buyer pays externally via any wallet.
6.  [client] obtain preimage — from the NWC response directly, or via manual paste field for
       buyers without NWC connected (see §8).
7.  [client] POST {server}/purchase/confirm
       { game_coordinate, listing_event (the raw signed listing JSON, fetched fresh),
         bolt11, preimage }
       NIP-98 auth (buyer signer)
       → { receipt (kind:1020 event), download_token, token_expires_at }
8.  [client] ingest_receipt(receipt) — reuse existing NIP-102 parse+persist path
       (PurchasesRepository::upsert_receipt), which already exists and is unaffected by this spec.
9.  [client] persist download_token into download_tokens table
10. [UI] listing now shows is_owned = true (existing enrichment path fires on next
       marketplace refresh, or set optimistically immediately)
11. [UI] "Install" button now available → triggers §6 download flow
```

Error responses per ADP-01 (`402` bad payment, `409` replay, `404` not hosted, `500` fulfillment
key not authorized) should map to distinct, specific UI messages rather than a generic "purchase
failed" — particularly `409`, which means the buyer already owns this and should be routed
straight to Install instead of being told the purchase failed.

## 6. Download Flow (new — extends `install_game`)

```
1.  [client] check download_tokens for (coordinate, server_url) with expires_at > now
       → hit: DownloadAuth::Token(token)  (Path A)
       → miss: DownloadAuth::Nip98        (Path B — covers reinstall / different machine)
2.  [client] AdpClient::download(coordinate, auth, dest_path, on_progress)
       - Path A: GET /game/:coordinate?token=...
       - Path B: GET /game/:coordinate with NIP-98 auth; server does its own relay-side
         ownership check, client does no receipt-matching itself here — it just presents
         the signed NIP-98 token and lets the server decide
       - streamed to dest_path, emits "download-progress" Tauri events (bytes, total)
     Error handling: 403 (ownership failed — shouldn't normally happen if step 1 was honest,
     but the server is the authority) / 451 (server not authorized to distribute — distinct
     message: "this server no longer distributes this listing", not a generic error) / 500.
3.  [client] on completion: sha256_file(dest_path) and compare to listing.file_hash
       (belt-and-suspenders client-side check in addition to the server's own integrity check —
       cheap, and catches a corrupted transfer the server-side ETag logic wouldn't).
4.  [client] InstalledGamesRepository::record(...)
5.  [UI] emit "download-complete", switch button to "Installed" (extraction/launch: future work)
```

## 7. Tauri Commands & Events (new inventory for this pass)

| Command | Params | Returns | Notes |
|---|---|---|---|
| `check_adp_server` | `server_url` | `AdpServerInfo` | wraps `.well-known/adp`, used at publish-form validation time |
| `publish_adp_listing` | full listing form + `server_url`, `file_path`, `version` | `PublishResult` | orchestrates §4 steps 3–9, emits `publish-progress` per step |
| `request_lnurl_invoice` | `lud16`, `amount_sats` | `{ bolt11 }` | §5 step 2–3 |
| `confirm_purchase` | `game_coordinate`, `server_url`, `bolt11`, `preimage` | `{ receipt, download_token, token_expires_at }` | §5 step 7; internally re-fetches `listing_event` fresh rather than trusting a client-cached copy |
| `install_game` (replaces existing stub) | `game_coordinate` | `()` | now performs §6 in full, replacing the current ownership-check-only version |
| `get_installed_games` | — | `Vec<InstalledGame>` | for library view |

| Event | Payload | Emitted by |
|---|---|---|
| `publish-progress` | `{ step: String, status: "pending"\|"ok"\|"error", message: Option<String> }` | `publish_adp_listing` |
| `download-progress` | `{ game_coordinate, bytes, total: Option<u64> }` | `install_game` |
| `download-complete` | `{ game_coordinate, file_path }` | `install_game` |

## 8. Payment Path Decision & Remaining Open Item

**Decided: NIP-47 (Nostr Wallet Connect) is the primary payment path, manual preimage paste is
the fallback.** WebLN was considered and rejected for the desktop target — WebLN depends on a
`window.webln` object injected by a browser extension running in the page context, and Tauri's
WebView has no extension ecosystem to inject it, so it isn't reliably available the way it is in
a real browser (it may still make sense later for the separate web/browser-extension target, but
is out of scope here). NWC fits the existing architecture instead: it reuses the same
relay-pool/signer shape Arcadestr already has for NIP-46, and its `pay_invoice` response
(`kind:23195`) includes the preimage as a structured field, rather than something the UI has to
scrape out of a wallet's own display. rust-nostr (the `nostr-sdk` family already pinned at 0.44)
has an NWC client implementation, so this is additive, not a new protocol stack.

This adds one new one-time setup flow, conceptually identical to the existing bunker/nostrconnect
login UI: buyer pastes/scans a `nostr+walletconnect://` connection string once, stored the same
way saved signer profiles are today. Once connected, §5's "buyer pays externally" step becomes
"client sends `pay_invoice` over the buyer's NWC connection, awaits the `kind:23195` response,
extracts `preimage`" — no manual step at all for a buyer who has NWC connected. Buyers without a
connected wallet still see the bolt11 as QR + copy, pay any way they like, and paste the preimage
manually; the UI should treat this as parallel to, not gated behind, having NWC set up.

New core module implied by this: `core::nwc_client` — thin wrapper around signing/publishing
`kind:23194` and awaiting/decrypting the matching `kind:23195` on the buyer's configured relay,
with a timeout and a clear "payment not received back / wallet didn't respond" error distinct
from "payment failed."

**Zaps confirmed kept as-is.** `core::lightning`'s existing NIP-57 zap code is untouched by this
spec — it's used elsewhere (social zaps) down the line, and purchases go exclusively through
`core::lnurlp` (invoice request) + `core::nwc_client`/manual (payment + preimage), never through
a zap receipt for the ADP path. No merge or deletion implied.

Everything in this spec is now decided. Proceeding to the gated completion prompt.

## 9. Suggested Gate Order for Implementation

Matches existing working pattern — stop at each gate for review before proceeding:

1. **Gate 1 — `core::nip98_client` + `core::adp_client` (read-only calls only: `well_known`,
   `provision`, `purchase_confirm`, `download`).** No UI yet. Unit/integration tests against a
   locally-running `adp-server` instance (or `MockRelay`-style test doubles mirroring the server's
   own test patterns).
2. **Gate 2 — `core::lnurlp` + `core::file_hash` + migrations 004–006.** Independent of gate 1,
   can run in parallel.
3. **Gate 3 — Publish flow: Tauri command + UI stepper.** Depends on gates 1–2.
4. **Gate 4 — `core::nwc_client` (NWC connect UI + pay_invoice/response flow), then buy flow
   Tauri command + UI wired to both the NWC path and the manual-paste fallback.**
5. **Gate 5 — Download flow: `install_game` rewrite + `installed_games` + library view update.**

Everything above is now confirmed. Gated completion prompt for opencode follows in a separate
document (`ARCADESTR-ADP-CLIENT-COMPLETION-PROMPT-v1.md`).
