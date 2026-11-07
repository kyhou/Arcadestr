# Gate 5 — Download flow and game updates

Status: **COMPLETE for the accepted isolated-fixture acceptance criteria.**
Live infrastructure is not required for this gate. Gates 3, 4 and receipt
portability remain complete; no external software or protocol was modified.

## Behavior

- Resolve and verify one fresh signed listing and host-specific install plan.
  Retain its coordinate, digest, package format, platform and launch profiles
  throughout all attempts. Never substitute another artifact during failover.
- Preserve existing server preference: valid buyer/coordinate/server tokens in
  signed server order, then other signed servers in that order. Every source
  must also occur in the resolved artifact's ADP source list.
- Tokens remain bound to their issuing server. Expired local tokens are excluded.
  NIP-98 attempts require independently established ownership or explicit current
  public/timed access; possession of another server's token is not sufficient.
  Sign each request for its actual target. Existing redirect blocking remains.
- Retry transport failures, missing/unavailable artifacts (404), transient HTTP
  408/429/500/502/503/504, and digest mismatches against the same pinned artifact.
  Truncate and rewind the private candidate before every attempt. Each eligible
  server is attempted at most once.
- Stop on authentication (401), ownership (403), or distribution-authorization
  (451) rejection. Do not silently downgrade credentials or bypass a denial by
  trying another server. Ownership and distribution failures remain distinct.
  Storage, local filesystem and archive-policy failures do not trigger failover.
  Response bodies and credential-bearing request URLs are not surfaced by the
  failover service.

## Updates and recovery

The detail page exposes **CHECK & UPDATE** through the existing installation
callback, modal and byte-progress events. An unchanged installed revision is a
no-op. Different revisions use the existing installer, package verifier,
capability-owned file placement and installation repository—not a second installer.

1. Hold the existing per-coordinate lifecycle lock. Settle any pending update
   recovery before another installation, retained-package operation or uninstall.
2. Persist update intent while leaving the old ready installation, manifest and
   retained package untouched. Use the centralized replaceable-event ordering to
   reject stale revisions, including its event-ID tie breaker.
3. Download to a distinct private retained-package candidate. Verify its digest,
   inspect the archive, check storage, and extract/place a distinct installation
   root. Available disk space already accounts for the still-present old version.
4. Atomically switch the existing installation's snapshot, profiles, manifest and
   retained-package reference, and mark the journal committed in the same SQLite
   transaction. Reuse normal `commit_extraction` validation inside the transaction.
   Reject stale snapshots, mismatched packages, incomplete manifests, and replacing
   a launchable build with a known unlaunchable replacement.
5. Before commit, any failure or restart discards only candidate files; the working
   version never needs to be reconstructed. After commit, recovery removes only
   retired files. Check active references before deletion, use managed-directory
   capabilities, and retain intent if cleanup fails. Repeated recovery is safe.
   One game's deferred cleanup does not stop recovery of other games.

The same coordinate lock covers install/update/uninstall/retained-package commands
and recovery. Existing root locks additionally protect file placement and cleanup.
The unique journal coordinate and snapshot comparison are storage-level defenses.

Following `AGENTS.md`, the journal table extends the existing pre-production
`009_installations.sql` schema. **No compatibility migration was added.** Existing
developer databases are not upgraded automatically; use a fresh development database
when exercising this schema. No existing database or protected fixture was reset.

Retained-only older revisions without a ready installation still require explicit
removal before a different build is installed. A retained archive is never silently
reinterpreted as another revision. Updating an existing ready installation needs no
such removal.

## Acceptance evidence

All networking in Gate 5 tests is mocked. Signed listing and receipt fixtures,
private SQLite databases, real archive verification, extraction and filesystem
placement exercise the production command path.

| Criterion | Result |
| --- | --- |
| Token Path A and receipt/entitlement-backed NIP-98 Path B | PASS — existing command tests plus failover fixtures |
| Signed server preference, authorized-source filtering and credential isolation | PASS — candidate-order tests and multi-server command fixtures |
| Local token expiry and another account's token | PASS — expired tokens are excluded; other-account credentials cannot grant access |
| Distinct auth/ownership/distribution denials, no eligibility downgrade | PASS — 401/403/451 fixtures stop before the second server |
| Unavailable servers and corruption | PASS — transient failures retry; longer corrupt bytes cannot contaminate the next attempt |
| Atomic replacement and rollback | PASS — successful update changes active root/package; corruption, 403/451/503 and storage failure preserve old snapshot/files |
| Restart recovery | PASS — partial download, partial extraction, placed candidate, and post-commit/pre-cleanup interruption; repeated recovery |
| Conflicting operations and stale state | PASS — coordinate/root lock fixtures, duplicate intent rejection, stale-snapshot rejection |
| Transactional integrity and usable replacement | PASS — mismatched package, incomplete manifest and unavailable replacement profiles leave old rows unchanged |
| Progress, installed registry and frontend integration | PASS — existing app regressions; native/WASM checks and isolated Trunk build |

Validation:

- `cargo test -p arcadestr-core --features native`: 571 unit tests passed,
  1 ignored; all integration suites passed, including 19 receipt-portability tests.
- `cargo test -p arcadestr-desktop`: 311 tests passed across targets, 6 ignored.
  Includes five Gate 5 test groups with multiple failure/interrupt scenarios.
- `cargo test -p arcadestr-app`: 512 passed.
- Gate 3 Python guards: 35 passed.
- Desktop/default-WASM check and isolated Trunk build passed.
- Existing receipt-portability mock-IPC browser regression passed against that
  build. This is a receipt UI regression, not a live update or server test.
- Touched Rust files pass formatting checks; `git diff --check` passes.
- Payment migrations `021` and `022` retain their recorded hashes.

Logs: `/tmp/opencode/gate5-{core-final,desktop-final,app-tests,gate3-guards,wasm-check,trunk,receipt-browser}.log`.
One earlier desktop run hit an existing one-second provisioning timestamp
assertion; subsequent full runs passed without modifying that test or subsystem.

Additional check, outside the desktop download/install target:
`cargo check -p arcadestr-web --target wasm32-unknown-unknown --features web`
fails on pre-existing references to the excluded `tauri_invoke` module in
`app/src/ui_v2/receipt_sync.rs:145`, `app/src/ui_v2/views/recipient_refunds.rs:219`
and `app/src/lib.rs:5411`. These files were not changed by Gate 5. This is an
Arcadestr browser-only build issue, not an external limitation or a failure of
the default desktop WASM target. Log: `/tmp/opencode/gate5-browser-wasm-check.log`.

## External limitations (not gate blockers)

- A serving ADP server remains authoritative for ownership and its independent
  distribution authorization. Portable local ownership does not guarantee that
  server can retrieve historical receipt evidence.
- Relay outages, pruned history, absent artifacts and differing external server
  versions can prevent live downloads. Failover cannot manufacture evidence or
  distribution permission.
- Retry restarts from byte zero; resumable HTTP ranges are not implemented.
- No live quotes, invoices, payments, wallet permission changes, receipt publication,
  relay recovery, external-server modification or mining was performed. No claim
  is made about live Lightning settlement, live cross-device retrieval, power-loss
  fault injection or a visually inspected game-update browser session.

Development backup scope: Gate 5 implementation, tests and this report only.
Full release validation is not required for this backup; unrelated working-tree
changes remain outside the commit.
