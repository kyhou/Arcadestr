# Gate 6 — Publisher and fulfillment final acceptance

Status: **COMPLETE — all 12 Arcadestr-controlled acceptance criteria PASS.**
Scope: accepted six-phase Arcadestr work, including G6-13. This is development
acceptance using isolated fixtures, not a production release or live-service
certification. No external service/protocol specification was modified, no live
operation or payment was performed, and no commit was created.

## Phase 6 changes

- Add `025_marketplace_campaign_pointers.sql`; prior migration identities and SQL
  remain unchanged. `marketplace_listings.campaigns_json` stores advisory root
  references separately from authorization evidence. Existing rows default to an
  empty projection; no missing historical association is invented.
- Cache reads preserve valid pointers, reject malformed IDs/hints and deduplicate
  entries without discarding the listing. All four time-filter query shapes and
  SQLite restarts retain associations. A verified refresh of the same source
  event can repair an incomplete projection without relaxing revision ordering.
- Ordinary metadata publication merges pointers from the verified current
  listing before provisioning/signing. Missing/stale cached pointers cannot
  remove verified associations or replace their relay hints. Malformed requested
  pointers stop before authorization/provisioning. Intentional pointer removal
  continues through its separate current-revision-checked command.
- Cached associations remain discovery hints. Existing campaign, authorization,
  grant, receipt and commercial verifiers remain authoritative. No cache field
  grants issuance, ownership or payment authority.
- Add a command-level native lifecycle fixture and a real-WASM regression of the
  existing publisher console. Debug fixture routes mount existing components;
  there is no new production authoring/execution system or UI redesign.
- Fix commercial-review consent's live checkbox property. The new console test
  first demonstrated that refreshing exact terms cleared logical consent but
  left the old checkmark visible. A fresh review now visibly requires new consent;
  the existing native exact-review/approval checks remain unchanged.
- Replace timestamp-only ADP storage test database names with UUIDs following a
  parallel fixture initialization collision. Production migration behavior is
  unchanged.

## Lifecycle coverage and disposition

| Stage | Arcadestr disposition and evidence |
| --- | --- |
| Listing draft | Explicit account-scoped save/restore and optimistic revisions; native handles and previous launch trust are not restored. |
| Preparation/commercial review | Exact signed terms, operator identities, capability choices, deterministic allocation and current-account consent; edits preserve existing acceptance unless a new exact review is approved. |
| Operator provisioning | One independent scope/held-key action per issuing/distribution operator; aliases cannot silently expand roles; fulfillment private keys remain with the operator. |
| Fulfillment authority | Exact signed roots are retained; metadata preserves historical references. Explicit developer cancellation is prospective, not operator/key retirement. |
| Listing publication | App-minted stable coordinate, signer/session fences, current-revision checks and exact-event confirmation on selected relays. |
| Store Page | Existing content validation/sanitization, reciprocal association checks, independent pointer outcomes and exact-event recovery. |
| Archive registration | Native selection, inspected/hashed leased bytes, signed digest/coordinate and validated same-origin upload acknowledgement; partial/unknown outcomes remain visible. |
| Browse/management projection | Live and cached projections preserve campaign associations, source revisions, commercial references and authorization references; projections are not credentials. |
| Metadata update | Verified current source overrides omissions in a cached form; unmanaged authorization, commercial and Store Page references survive. Explicit pointer-removal workflows stay separate. |
| Campaigns | Developer-authored creation, supported pre-start modification, scoped listing pointers, claims within the valid interval and prospective cancellation. |
| Grants | Existing authorized issuance and historical evidence validation; only the developer may revoke one entitlement. Other grants/purchases remain independent. |
| Purchase/receipt/download | Existing approval, quote verification, durable payment proof, receipt portability, ownership and authenticated download paths are reused and regression-tested. |
| Interrupted operations | Read-only restart, durable immutable checkpoints/signatures, explicit reconciliation, unknown-POST acknowledgement and freshly verified archive reselection. No automatic side-effect replay. |
| Cancellation/account change | Session epochs fence later signing/publication/upload; stale frontend callbacks cannot apply. Archive cancellation discards a selection, not remote history or financial proofs. |
| Listing removal/operator offboarding | Distinct, explicitly unavailable actions; neither is implied by campaign cancellation, grant revocation or authority cancellation. No unsupported retirement protocol is invented. |

## Isolated end-to-end evidence

`desktop/src/publisher_lifecycle_tests.rs` drives the real command layer, existing
signer, repositories, relay manager, archive service and HTTP client against two
loopback WebSocket relays and two loopback operator fixtures. Its HTTP wrapper
rejects non-fixture destinations; Lightning-address metadata is an in-memory
fixture and no invoice/payment/wallet call is made.

The fixture reviews two exact commercial roots, mints/publishes one stable
listing, independently provisions both operators and sends real leased multipart
archive bytes. One operator registers bytes but loses its acknowledgement. The
test closes/reopens the SQLite database with a new app/archive service, verifies
startup is read-only and rejects the old handle, reselects the pinned archive,
requires separate unknown-outcome retry consent, and reuses the exact signed
listing. The already-acknowledged operator is not uploaded/provisioned again.

It then publishes/links a Store Page, creates and modifies a campaign, reconciles
a recoverable clock-guarded pointer update without replacing its campaign root,
checks Browse/cache/management consistency, and edits metadata from a form with
missing campaign pointers without losing verified associations, commercial
acceptance or authorization. It verifies a held-key grant through the native claim
command, downloads the registered archive through existing NIP-98 transport,
cancels a campaign, revokes one grant, cancels one fulfillment authority and checks
historical issuance boundaries and A–B–A session fencing.

The clock guard is deliberately preserved: an immediate replacement in the same
second stops rather than manufacturing a future timestamp. The fixture waits for
the required boundary and explicitly reconciles the original operation.

The native lifecycle is complemented by isolated payment/receipt-portability
fixtures and real-WASM mock-IPC browser tests; it does not claim a live Lightning
settlement or a production relay/operator interoperability run.

## UI integration evidence

`regtest-publisher-console-ui.py` mounts the actual publisher routing/components,
not a replacement UI. It exercises managed-game distribution/revocation controls,
listing draft save/restore, the staged commercial/role controls, normal Store Page
navigation, Store Page draft saving and account switching. It permits only mock
IPC and blocks external browser transport. Separate existing recovery,
management/revocation, Gate 4 purchase and receipt-portability browser regressions
cover explicit consent, operation identity, stale asynchronous responses and
bounded/partial state disclosures.

The console test retrieves an exact commercial review, checks consent, retrieves
a fresh review and asserts the visible checkbox is reset. The pre-fix failure is
retained in `gate6-phase6-commercial-consent-red.log`; it is not hidden by the
later passing run.

## External/protocol scenarios and non-blocking limits

| Scenario | Disposition |
| --- | --- |
| Global relay consensus/completeness | **BLOCKED externally**; exact-current and revocation observations are bounded selected-relay evidence, never global consensus. Missing/ambiguous evidence stops the relevant action. |
| Read-only remote provisioning/upload inventory | **BLOCKED by absent ADP capability**; unknown responses stay unknown and require explicit same-scope/pinned-artifact retry acknowledgement. No client-side rollback or global exactly-once guarantee is invented. |
| Current download availability from historical registration or health | **Not inferred**; historical per-operation evidence is displayed truthfully. Successful registration/health is not a current availability proof. |
| Operator/key retirement and coordinated offboarding | **Not implemented / unresolved protocol workflow**; explicit prospective authorization cancellation is supported separately and does not destroy old credentials or silently renegotiate terms. |
| Optional media hosting/live service acceptance | Existing hosting/upload paths are reused; interrupted selections require reselection. Mocked/native validation does not certify external retention, live service behavior or hosting availability. |
| Standalone browser publishing | Existing explicit unsupported boundary remains; WASM acceptance uses the desktop IPC bridge. No new browser-side publisher executor is introduced. |
| Experimental protocol kinds | Existing repository allocations and chain rules are preserved; this work does not claim a new interoperable allocation or modify external protocol specifications. |
| Whole-workspace formatting baseline | Pre-existing formatting differences in untouched receipt/library/shell files remain outside this change. Changed Gate 6 Rust files are checked separately; unrelated files are not reformatted to hide that baseline. |

Compiler warnings and manifest/future-compatibility warnings remain. They are not
treated as a warning-free build claim. No live acceptance is inferred from a
mocked or isolated success.

## Final assessment

| # | Original criterion | Decision | Supporting evidence |
| --- | --- | --- | --- |
| 1 | Lifecycle coverage | **PASS** | Every stage is traced and dispositioned above, including drafts, provisioning, authorization, listing/Store Page publication, registration, Browse, metadata, campaigns, grants, purchase/download, recovery and explicitly unsupported removal/offboarding. The native fixture exercises the supported command sequence. |
| 2 | Existing capability preservation | **PASS** | Existing publication pipelines, session-fenced signer, ADP client, archive service, authorization/entitlement resolvers and repositories, commercial allocation, purchase and publisher journal are reused. New operator/relay implementations exist only in isolated test fixtures. No parallel production publication, authorization, upload or payment executor was added. |
| 3 | Identity safety | **PASS** | Native signer/session/replay adversarial tests, A–B–A fencing in the command lifecycle, full app state tests, and browser delayed-review/delayed-mutation/recovery account-switch tests pass. Old handles, consent and callbacks cannot authorize another account's work. |
| 4 | Listing integrity | **PASS** | Stable backend-minted coordinate, exact-current selected-relay confirmation, NIP-01 ordering and clock guards, verified source/CAS-style checks, immutable signed retries and metadata preservation pass. Five projection tests cover cache round trips, all query shapes, corrupt pointers, stale revisions, upgrade/restart and authoritative merging. The native fixture compares Browse/management associations, source IDs and commercial/authority references. |
| 5 | Store Page integrity | **PASS** | 62 core Store Page-related tests and 5 native Store Page tests pass, including content validation/render sanitization, association and source-revision protections and partial-pointer behavior. The native lifecycle publishes/links a Store Page; the real console reaches its existing editor and saves a durable draft. |
| 6 | Artifact integrity | **PASS** | The native command fixture inspects/hashes a real archive, uploads leased multipart bytes to both fixtures, checks signed digests and validated registration responses, rejects a restart-expired handle and reselects the original digest. Its downloaded bytes agree. Archive, transport, acknowledgement-binding and Gate 5 adversarial regressions pass. |
| 7 | Distribution coordination | **PASS** | Independent operator provisioning/authority, duplicate URL/alias identity and role checks, least-privilege capabilities and partial/unknown retry tests pass. Native distribution projection reports each operation's validated registration evidence without treating it or health as current download availability. |
| 8 | Commercial correctness | **PASS** | Exact signed-root review and deterministic allocations pass in isolated commercial/quote tests and in the native two-operator lifecycle. Metadata retains original acceptance. Missing/withdrawn/incompatible evidence stops publication rather than renegotiating silently. The real console discloses both reviewed roots and visibly clears consent on review refresh. |
| 9 | Campaign/grant correctness | **PASS** | Existing campaign, claim, authorization and entitlement protocol/repository tests pass. The command fixture creates/modifies/cancels a campaign, verifies a held-key claim, rejects management from the buyer account, revokes one grant as developer and explicitly cancels one authority. Historical issuance remains valid before its cutoff; independent grants/paid ownership proofs retain existing rules. |
| 10 | Recovery | **PASS** | Durable draft/CAS and immutable checkpoint tests, original signed-event replay tests and the database-restart command fixture pass. Startup performs no provisioning/upload replay; unknown remote outcomes require separate explicit consent. Archive cancellation/reselection and session fencing preserve safe stops without deleting remote history, receipts or other financial proofs. |
| 11 | Regression protection | **PASS** | The final 23-suite core integration run, affected core unit groups, all 517 app tests, affected native command/archive/Gate 5 tests and native/WASM checks pass. All five real-WASM mock-IPC browser regressions pass, including Gate 4 and receipt portability. No live/ignored acceptance test was enabled. |
| 12 | Boundaries | **PASS** | External/protocol scenarios are separately dispositioned above. Unsupported capabilities remain explicit, bounded observations are not advertised as global truth, and neither unknown outcomes nor historical acknowledgements become availability claims. After the projection and consent-display fixes, no known critical Arcadestr-owned security/correctness defect remains in the accepted scope. |

There are **no FAIL or BLOCKED Arcadestr-controlled acceptance criteria** in this
assessment. The separately listed external scenarios remain blocked/unsupported
only for those scenarios; they do not negate unrelated supported functionality.
Operator retirement alone is not a reason to leave Gate 6 in progress.

## Final validation results

| Run | Passed | Failed |
| --- | ---: | ---: |
| 23 core integration suites | 174 | 0 |
| Core ADP unit filter | 97 | 0 |
| Core marketplace/cache filter | 58 | 0 |
| Core Store Page filter | 62 | 0 |
| Core purchases filter | 1 | 0 |
| Core schema/migration filter | 15 | 0 |
| Core HTTP transport filter | 12 | 0 |
| Core settlement recovery filter | 3 | 0 |
| Core ownership filter | 7 | 0 |
| Full app library suite | 517 | 0 |
| Native ADP command suite, including isolated lifecycle | 86 | 0 |
| Native publisher journal/replay suite | 6 | 0 |
| Native Store Page suite | 5 | 0 |
| Native archive selection suite | 18 | 0 |
| Native Gate 5 filter | 5 | 0 |

Filters can overlap; these counts are per command, not a claim of unique tests
or a full workspace test run. The core integration targets were:
`publisher_projection`, `publisher_recovery`, `publisher_management`,
`publisher_commercial`, `authorization`, `adp_entitlement_protocol`,
`entitlements_repository`, `campaign_discovery`, `multi_operator_gate2`,
`adp_settlement`, `quote_verification`, `receipt_protocol`,
`receipt_listing_binding`, `receipt_portability`, `purchase_execution`,
`payment_durability`, `startup_settlement_recovery`, `settlement_repository`,
`aggregate_ownership`, `adp_publish_protocol`, `relay_fetch_outcomes`,
`archive_inspection_zip` and `archive_inspection_tar`, using
`cargo test -p arcadestr-core --features native --test <target>`.

Additional results:

- **PASS:** `cargo check -p arcadestr-core --features native -p arcadestr-desktop -p arcadestr-app`.
- **PASS:** `cargo check -p arcadestr-app --target wasm32-unknown-unknown`.
- **PASS:** `trunk build --dist /tmp/opencode/gate6-phase6-frontend` from `web/`.
- **PASS:** publisher-console, management/revocation, recovery, receipt-portability
  and Gate 4 browser regression scripts against that isolated frontend.
- **PASS:** relevant changed Gate 6 Rust formatting and `git diff --check`.
- **NON-BLOCKING BASELINE FAILURE:** `cargo fmt --all -- --check` reports only
  unchanged files: `app/src/tauri_invoke.rs`, `app/src/ui_v2/shell.rs`,
  `app/src/ui_v2/views/library.rs`, `app/src/ui_v2/views/purchases.rs`,
  `core/src/ownership.rs` and `core/src/purchases.rs`. They were not reformatted.

Final logs use `/tmp/opencode/gate6-phase6-final-*.log`; formatting and diff logs
use `gate6-phase6-format-*.log` and `gate6-phase6-diff-check.log`. Earlier red runs
are development feedback, not final acceptance failures: incomplete test-server
metadata, fixture requests bypassing the intended review extension, deliberate
clock-safe partial results, a timestamp-named database fixture collision and the
commercial consent display regression were addressed without weakening protocol
or session checks.

Prior phase evidence: `gate6-commercial-configuration.md`,
`gate6-publisher-recovery.md` and `gate6-distribution-revocation.md` in this folder.
Their in-progress status text is historical; this final assessment supersedes it
for Gate 6's current status.

**Gate 6 is COMPLETE for the accepted isolated-fixture development criteria.**
Work stops after Phase 6. All changes remain uncommitted, unrelated working-tree
changes are preserved, and no external service or live operation was used.
