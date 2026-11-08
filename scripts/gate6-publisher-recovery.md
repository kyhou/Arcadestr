# Gate 6 Phase 4 — Durable publisher recovery

Scope: G6-09. Gate 6 remains **IN PROGRESS**; Phase 5 distribution/revocation
management is not included. No production release or live infrastructure claim.

## Persistence and ownership

Additive migration `024_publisher_recovery.sql` stores account-scoped drafts,
operations and immutable stage intent. Existing migration files are unchanged.
The repository uses the existing SQLite pool and transaction/error patterns.
Draft saves compare revisions; stale or simultaneous writers cannot overwrite a
newer draft. Operation IDs and checkpoints cannot be substituted on retry.

Listing and Store Page editors expose explicit save and restore actions. Drafts
retain metadata/content, operator choices, exact commercial and authorization
references, launch text/defaults and artifact hashes. Restore is local, replaces
the displayed form only on request, and is not publication or renewed consent.
Store Page draft status distinguishes a saved local snapshot from later edits
and from published events. Commercial changes still require Phase 3 review.

Native archive/media selections and launch-path trust flags are not restored.
Opaque handles, private native paths and credential fields are excluded from
durable request/draft payloads. No fulfillment private key, NIP-98 header, wallet
credential or download token is required. Signed public events are retained
unchanged, including IDs, authors, signatures, tags and timestamps.

## Existing execution paths, not a second executor

The existing listing, Store Page, campaign and pointer commands run inside a
publisher-journal context. The existing session-fenced signer records unsigned
intent before waiting for a signer and retains the signed result before relay
publication. Retry returns that exact signed event instead of signing a new
timestamp/revision. Retained events must still have valid signatures and match
the publisher, kind and publication coordinate.

Provisioning and upload use write-ahead checkpoints before the existing ADP
client is invoked. Their validated public responses are retained separately.
Successful upload acknowledgements still require the exact pinned coordinate,
digest and safe URL policy. Known acknowledgements are reused, not uploaded
again. Failed attempts retain a safe diagnostic without persisting untrusted
response bodies or credentials. Missing acknowledgements mean unknown outcome,
not proof that the remote action did not happen.

Store Page and campaign pointer mutations retain their source revision and signed
replacement. Publication/pointer acknowledgements are recorded separately from
selected-relay propagation confirmation. Partial results remain incomplete.
Database failure before a checkpoint prevents the corresponding network action;
failure after a remote action leaves its earlier checkpoint for reconciliation.

## Restart and explicit recovery

The Publisher recovery panel reads local account-scoped operations after restart.
Neither startup, opening the panel nor refreshing it signs, publishes, provisions
or uploads anything. A resume button requires explicit authorization. Native
commands also enforce it, the active publisher/session epoch and existing
operation/coordinate coordination. Stale frontend responses cannot populate a
different account's recovery state.

Resume reloads the original intent and invokes the same publication pipeline.
It checks current listing/campaign/Store Page revisions before retrying, skips
already-observed exact events and refuses concurrent/stale replacements. Current
authorization evidence is reconciled before an authorization event is retried;
cancelled authority is not renewed. Commercial acceptance is never replaced with
a newer advertisement. Completed operation results are read as historical local
evidence, not a claim of current server availability.

For an archive still needing upload, a fresh native selection must be inspected,
hashed and leased by the existing archive service. Its verified digest must
match the original pin **before** authorization replay or other external work.
Expired handles are never revived. If all upload acknowledgements were already
retained, recovery can reconcile without reading/uploading the archive again.

## External limits and safe stops

- ADP currently has no read-only provisioning/upload inventory API. An uncertain
  POST therefore remains unknown. A separate explicit acknowledgement is required
  for a same-scope/pinned-artifact retry; it is not dispatched automatically.
  Existing endpoint idempotency is reused, not invented client-side rollback.
- Relay observations are limited to available selected/connected-relay evidence,
  not global consensus. Incomplete evidence or an intervening revision can leave
  the operation blocked for review without blocking unrelated games.
- A changed/cancelled authority, changed commercial offer or stale source revision
  does not authorize editing an old operation in place. Save desired edits as a
  draft and review the current game state. Historical credentials remain intact.
- Pending media file selections must be reselected through the existing Blossom
  UI. Saving a draft does not assert that an interrupted remote media upload was
  rolled back or that the hosting service retained it.
- Journals are retained, not deleted on completion. No financial proof, receipt,
  previous milestone evidence or unrelated working-tree change is discarded.

## Focused validation

Tests use temporary databases, synthetic signed events, injected checkpoint
failures and mocked clients. They cover restart before/after logical side-effect
boundaries, unknown outcomes, revision CAS, account isolation, duplicate results,
partial multi-server state, exact signature reuse, stale session fences,
cross-coordinate substitution, provisioning retry admission and archive digest
reselection. These are not live process-kill or live service acceptance results.

Relevant checks: publisher recovery/commercial/authorization/quote tests; existing
Gate 3/4 execution, durability and receipt-portability tests; native publisher,
Store Page, archive-selection and Gate 5 tests; migration integrity tests; native
and desktop-bridge WASM checks. Detailed logs: `/tmp/opencode/gate6-phase4-*.log`.

Validated suites (passed / failed):

| Suite | Passed | Failed |
| --- | ---: | ---: |
| Durable repository/restart recovery | 7 | 0 |
| App publisher state | 172 | 0 |
| Native ADP command regressions | 84 | 0 |
| Native publisher journal/replay safeguards | 5 | 0 |
| Store Page commands | 5 | 0 |
| Native archive selection | 18 | 0 |
| Gate 5 desktop regressions | 5 | 0 |
| Commercial, authorization, quote, purchase and receipt integrations | 92 | 0 |
| SQLite schema/migration tests | 15 | 0 |
| Core ADP unit regressions | 97 | 0 |

The schema test initially rejected the three new tables because its expected
table inventory was unchanged. Updating that inventory made all 15 tests pass;
no previous migration was rewritten. Compiler warnings remain; successful
checks are not a warning-free claim.

Existing receipt-portability and Gate 4 UI regressions passed using the isolated
debug WASM frontend and allowlisted mock IPC. No live payments, publications,
uploads or provisioning were performed. Changes remain uncommitted.

Final native checks, `cargo check -p arcadestr-app --target wasm32-unknown-unknown`,
and the isolated Trunk frontend build passed. The recovery UI regression also
passed: read-only startup, unknown-outcome disclosure, explicit retry consent,
archive reselection, original operation identity and stale-account response
isolation. The browser test caught consent checkboxes retaining stale visible
state after an attempt; binding their live `checked` properties fixes that reset.
The harness waits for reactive button enablement before clicking.

Relevant Rust formatting and `git diff --check` passed. Phase 4 is implemented
and locally validated within the mocked/native limits above; Phase 5 has not
started, and Gate 6 remains **IN PROGRESS**.
