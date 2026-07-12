# Arcadestr — ADP-01 / NIP-102 Client Integration: Gated Implementation Prompt v1

For: opencode
Reviewer/gatekeeper: Claude (Joel runs this — do not skip a gate's stop point)
Design source of truth: `ARCADESTR-ADP-CLIENT-INTEGRATION-v1.md` (read it in full before
starting; this prompt sequences the work, that document specifies it)

## Ground Rules (apply to every gate below)

1. **Obsolescence**: all prior Arcadestr publish/buy/install code (anything touching NIP-15
   product publishing, the old zap-based buy flow tied to purchase completion, and the current
   stub `install_game`) predates ADP-01/NIP-102 and is being replaced by this work. Do not try to
   preserve or extend it where it conflicts with the spec — replace it. Existing NIP-102 receipt
   *parsing/storage* (`core::purchases`, `PurchasesRepository`) is NOT obsolete and should be
   reused as-is wherever this spec calls for "ingest_receipt".
2. **Compile-gate discipline**: after every meaningful chunk of a gate, run `cargo check` (and
   `cargo check --target wasm32-unknown-unknown --features web` for anything touching `app`).
   Stop on the *first* compile error. Do not batch up multiple errors and fix them together — fix
   one, recheck, continue. Do not proceed to the next gate with any outstanding warning from
   `cargo clippy -- -D warnings`.
3. **No silent scope changes**: if something in the spec turns out to be wrong once you're
   actually looking at the code (e.g. a type doesn't exist, a module is structured differently
   than assumed), stop and report back rather than improvising a workaround. This is a gate, not
   a suggestion.
4. **Approval required before any code changes** at each gate boundary marked STOP. Implement up
   to the boundary, report what you did and what you tested, then wait.
5. **Tests**: every new `core` module gets unit tests for its pure logic (parsing, header
   construction, hash computation) without a network dependency, plus at least one integration
   test that exercises it against a locally-running `adp-server` where the spec calls for HTTP
   interaction (Gates 1 and 4 especially). Use the `MockHttpClient` pattern already established in
   `core/src/test_helpers/http_mocks.rs` for anything that shouldn't require a live server in CI.

---

## Gate 1 — `core::nip98_client` + `core::adp_client` (read-only + provisioning calls)

Scope: §1.1 and §1.2 of the spec, plus the `AdpServerInfo`/`ProvisionResponse`/etc. response
types. Do NOT implement `upload`, `purchase_confirm`, or `download` bodies yet beyond compiling
type-correct stubs — those depend on modules from Gate 2/4 (file hashing, LNURL, NWC) and belong
in later gates once those exist. This gate's deliverable is: server reachability check, and
provisioning (`provision` + `provision_revoke`), fully working and tested.

**Files to add:**
- `core/src/nip98_client.rs`
- `core/src/adp_client.rs`

**Acceptance for this gate:**
- `build_nip98_auth_header(signer, url, method)` produces a header that a locally-running
  `adp-server` accepts (verify against `POST /provision` with a `LocalSigner` test signer —
  reuse whatever test signer fixture already exists for other command-contract tests).
- `AdpClient::well_known()` round-trips against a local `adp-server`.
- `AdpClient::provision()` round-trips: call it, confirm you get back a `fulfillment_pubkey` +
  `attestation_event_id`, confirm calling it again with the same scope returns the same key
  (idempotency, per ADP-01).
- Unit tests for header construction and response deserialization that don't require a live
  server (use `MockHttpClient`).

**STOP after Gate 1.** Report: what compiled, what tests pass, and whether the `Relay`-trait-style
mockability pattern from `adp-server`'s own test suite was worth mirroring here (your call to
propose, not obligatory to match).

---

## Gate 2 — `core::lnurlp` + `core::file_hash` + migrations 004–006

Scope: §1.3, §1.4, §2 of the spec. Independent of Gate 1 — can be done in either order, but
still gated separately for review.

**Files to add:**
- `core/src/lnurlp.rs`
- `core/src/file_hash.rs`
- `core/migrations/004_adp_provisioning.sql`
- `core/migrations/005_download_tokens.sql`
- `core/migrations/006_installed_games.sql`
- Corresponding repository structs (`AdpProvisioningRepository` or fold into an existing
  `core::adp_client`-adjacent module — your call, propose a location) for the three new tables.

**Acceptance for this gate:**
- `resolve_lud16` + `request_invoice` tested against `adp-server`'s own LNURL test fixtures if
  reusable, else a `wiremock`-based test mirroring the pattern in `adp-server/tests/lnurl_tests.rs`.
- `sha256_file` tested against a known-hash fixture file, including a >1MB file to confirm
  chunked reading actually engages (don't just test a tiny file that fits in one read).
- Migrations apply cleanly on top of the existing `003_purchases.sql` baseline; run the full
  existing migration test suite to confirm no regressions.
- Repository CRUD methods have unit tests (insert, idempotent-lookup-by-scope for provisioning,
  expiry check for tokens).

**STOP after Gate 2.**

---

## Gate 3 — Publish Flow (Tauri command + UI)

Scope: §4 of the spec. Depends on Gates 1 and 2 both being merged.

**Backend:**
- Fill in `AdpClient::upload()` body now that `file_hash` (Gate 2) exists.
- New Tauri command `publish_adp_listing` in `desktop/src/main.rs` (or a new
  `desktop/src/adp_commands.rs` if `main.rs` is getting unwieldy — your call) implementing spec
  §4 steps 2–9 in full, including the `kind:30406` construction/publish and the relay-propagation
  confirmation step (step 8) — reuse the existing multi-relay fetch path from marketplace code
  rather than writing a new one.
- Emits `publish-progress` events per step as specified in §7's event table.

**Frontend:**
- New publish form fields: server URL, build file picker, version string. Wire the
  `check_adp_server` reachability call to block submit on failure (per ADP-01's own "mandatory
  fields" guidance — this is a UX gate the client owns, not something the server enforces).
- A stepper/checklist UI component reflecting `publish-progress`, not a single spinner — per the
  spec's failure-handling note, a failure at a late step shouldn't force the developer to redo
  everything from scratch, so the UI should make it legible which step failed.

**Acceptance for this gate:**
- End-to-end manual test: publish a real small test game through a locally-running `adp-server`,
  confirm the listing shows up correctly tagged on relays, confirm the file is retrievable via
  `GET /game/:coordinate` with a manually-obtained token afterward (download flow itself isn't
  built yet — curl is fine for this check).
- Existing publish-form tests (if any survive obsolescence) updated or replaced; new tests for
  the reachability-check gating logic and for `adp_provisioning` idempotent reuse (publish the
  same listing/scope twice, confirm no second `/provision` call happens).

**STOP after Gate 3.**

---

## Gate 4 — `core::nwc_client` + Buy Flow

Scope: §8 (NWC) and §5 of the spec. Independent of Gate 3, but do it after Gates 1–2.

**Backend:**
- `core/src/nwc_client.rs`: NWC connection storage (reuse the saved-profile/keyring pattern from
  `core::nip46::storage` rather than inventing a new one — an NWC connection string is
  conceptually the same class of secret as a bunker URI), `pay_invoice` request/response
  round-trip per NIP-47 (`kind:23194` request, `kind:23195` response, timeout with a distinct
  "wallet didn't respond" error).
- Fill in `AdpClient::purchase_confirm()` body now that both `lnurlp` and `nwc_client` exist.
- New Tauri commands: `connect_nwc_wallet`, `request_lnurl_invoice`, `confirm_purchase` (per §7's
  table). `confirm_purchase` internally re-fetches the listing fresh rather than trusting a
  client-cached copy, per the spec note in §5 step 7.
- `ingest_receipt` call reuses the existing `PurchasesRepository::upsert_receipt` — do not write
  a new receipt-parsing path.

**Frontend:**
- NWC connect UI: one-time "paste or scan your wallet connect string" flow, styled consistently
  with the existing bunker/nostrconnect login UI (§8 of the spec explicitly calls this out as
  conceptually the same flow).
- Buy panel: bolt11 QR + copy (existing pattern, keep it), route to NWC auto-pay when a wallet is
  connected, manual preimage-paste field as the visible alternative otherwise — not hidden behind
  a "no wallet connected" dead end.
- Distinct UI messages for `402`/`409`/`404`/`500` per the spec's error-handling note — `409`
  specifically should route the buyer to Install, not show a generic failure.

**Acceptance for this gate:**
- Unit tests for NWC event construction/encryption and response parsing without a live wallet
  (mock the relay response).
- If a test NWC wallet service is reasonably available for integration testing, exercise a real
  `pay_invoice` round trip; if not, document what was mocked instead and flag it for a manual
  test pass with a real wallet (e.g. Alby) before this gate is considered fully closed.
- End-to-end manual test: full buy flow against the Gate 3 test listing, confirm receipt is
  persisted, `is_owned` flips, download_token is stored.

**STOP after Gate 4.**

---

## Gate 5 — Download Flow

Scope: §6 of the spec. Depends on Gates 1–4.

**Backend:**
- Fill in `AdpClient::download()` body (streaming, both Path A token and Path B NIP-98 auth
  variants).
- Rewrite `install_game` Tauri command per §6 in full: token-cache check → download → client-side
  hash verification → `InstalledGamesRepository::record`.
- `get_installed_games` command.
- `download-progress` / `download-complete` events per §7.

**Frontend:**
- Wire `install_game`'s existing button/UI to the new progress events (progress bar, not just a
  spinner, given files can be large).
- Library view reads from `get_installed_games` rather than whatever it currently does.
- Distinct message for `451` (server not authorized to distribute) vs `403` (ownership failed) —
  per the spec, these mean structurally different things and users troubleshooting one shouldn't
  be shown the other's message.

**Acceptance for this gate:**
- End-to-end manual test covering both paths: Path A (fresh purchase → immediate install) and
  Path B (simulate a "different machine" by clearing the local `download_tokens` row for a
  coordinate the test account has a valid receipt for, confirm install still works via NIP-98).
- Client-side hash verification test: deliberately truncate/corrupt a downloaded file in a test
  and confirm the mismatch is caught and surfaced, not silently accepted.

**STOP after Gate 5. This closes the spec.**

---

## After Gate 5

Explicitly out of scope for this prompt (per the spec's stated scope boundary): archive
extraction, "launch installed game," NIP-38 currently-playing status, trending games. These stay
on the existing roadmap as separate items — do not fold them into this work opportunistically
just because the download plumbing is now in place.
