# Gate 3 — Arcadestr ownership correction and non-spending plan

## Authority and current state

Arcadestr is the development target. **The custom Alby Hub patch is abandoned.**
Do not repair, audit further, build, deploy, migrate or configure that fork.
Leave `/tmp/opencode/albyhub-gate3-wallet` and all preserved source/audit/test
artifacts untouched as historical reference. The installed Hub is unmodified.

Gate 3 development is **COMPLETE** against the revised seven-criterion matrix
below. This is not live-payment verification or release authorization.
Phase 1 passed; Phase 2 read-only persistence/isolation
passed; live paid durability and Phase 3 remain unverified. App #4 stays read-only.
The existing 250-sat reservation, approval record, pause marker, original Alice
application, identities, unresolved invoices and previous evidence are retained.
They are not recreated, released or copied into a new executable approval.

Current authority is inspection, Arcadestr corrections, mocked-wallet tests,
disposable databases and standard read-only capability inspection. No live quote/invoice, payment, block, wallet permission
change or commit is authorized. A future controlled payment needs separate consent.

## Removed dependencies / superseded plans

- No Rust production module imports the custom wallet fork, its policy tables,
  migrations, private permission handlers or fee-limit interface.
- The mistaken dependency was in the **test strategy**, not a deployed adapter:
  `regtest-gate3-purchase.py` used the Hub-specific validator as a spending gate;
  the budget wizard treated private permission-row values as cap verification.
  Both live execution paths are retired and fail closed, even without a pause
  marker or with an old approval/validation artifact.
- `regtest-gate3-nwc-validate.py` retains its old **read-only forensic fixture
  audit** for earlier evidence. Its spending-validation mode is retired before
  credential/database access. It is not an Arcadestr runtime dependency and must
  not be reused as the active provider-compatibility or purchase-approval gate.
- Earlier integration/network/readiness scripts and fixture-specific Hub/LND
  diagnostics are historical lab tools, not portable payment-adapter contracts.
  In particular their original-app table fingerprints are evidence-preservation
  checks, not a production authorization model.
- `regtest-gate3-integration.md` and `regtest-gate3.md` retain historical results,
  but their execution instructions are superseded by this document.
- No custom wallet patch, schema, exact wallet DB representation, recipient
  restriction or lifetime-budget implementation is required by the active plan.

## Actual implementation assessment

Status legend: **verified** means scoped automated evidence or the specifically
recorded read-only GUI evidence, not a new real-money recovery acceptance claim.

| Capability | Actual implementation and assessment |
|---|---|
| NWC connection, credential isolation, signing | `core/src/nwc_client.rs`, desktop NWC commands/keyring: implemented and verified by existing contracts and Phase 2 read-only GUI checks. No Hub API/schema in the runtime. |
| Standard `pay_invoice` | Implemented, signed/encrypted responses authenticated and correlated, proof validated. Existing durable intent tests pass. Real dedicated-wallet paid recovery is unverified. |
| `multi_pay_invoice` | Missing as an Arcadestr transport operation; not required for sequential leg orchestration. Do not add it as a shortcut or assume batch atomicity. |
| Capability discovery | Standard authenticated `get_info` parses advertised methods/network. Fresh read-only inspection of the preserved connection confirms regtest and lookup_invoice, but no pay_invoice. No fee ceiling inferred. Unsupported/ambiguous lookup blocks instead of repaying. |
| Invoice generation | Arcadestr LNURL-P and ADP client calls exist. ADP Server's current `/settlement/quote` preflights exact invoices; `/next` discloses attempts sequentially. Quote creation therefore already creates live invoice obligations and is prohibited in this phase. |
| Accepted terms / deterministic split | `verify_quote_listing_terms` verifies the signed listing and developer-accepted term roots and reconstructs the complete deterministic split. Desktop quote creation invokes it before persistence; approval re-verifies retained original evidence in migration 022. Processor URL and refund commitment also bind the request. |
| Immutable settlement plan | Signed quote/event snapshots and account-scoped legs existed. The new app approval snapshot additionally pins the entire verified quote, event ID, wallet identity, network, reviewed Lightning payees and buyer limits. Projection rewrites now reject recipient/amount/order/count drift. |
| Buyer presentation / approval | Shared `PurchasePlanPanel` on checkout/history shows recipients, destinations, amounts, operator fees, desired limits, invoice signing keys when disclosed, fee policy, wallet methods, restrictions and per-leg recovery. Explicit `approve_purchase_plan` binds the reviewed quote, wallet, terms, network and limits under the account/session lease. A stored quote is not consent. |
| Application spending guard | `purchase_execution.rs` now persists approval and irreversible per-leg admissions, checks limits/bindings, serializes across database connections, halts on uncertainty and retains admissions across restart. Verified with mocks only. |
| Desktop dispatch | `pay_nwc_invoice` now requires the exact original stored invoice and calls the purchase executor under the existing account/session lease. Raw-invoice/legacy NWC dispatch without approved purchase context is intentionally disabled. Manual proof recovery and read-only wallet configuration remain available. |
| Payment persistence / idempotency | Existing encrypted `durable_payments.rs` intent/proof journal is preserved. New purchase-wide admissions precede it; both survive process cancellation. No automatic redispatch or replacement on uncertain outcomes. |
| Partial settlement / recovery | Startup query includes unresolved purchase admissions even without low-level intents. Manual/startup recovery reconciles executor admission before legacy proof submission, under the account lease. Shared UI distinguishes pending, uncertain, paid and accepted proof; completion requires a bound authorized receipt in purchases storage. No recovery gains dispatch authority. |
| Purchase confirmation | Legacy `/purchase/confirm` and split proof submission already exist. Proof is checkpointed before ADP submission; receipt/publication retry uses original proof rather than another payment. |
| NIP-102 validation / ownership | `purchases.rs`, `ownership.rs` and desktop `persist_split_receipt` verify signatures, buyer, aggregate quote/legs and issuer authorization, then retain receipt/token. Existing aggregate ownership tests are retained. No new live receipt/token is claimed. |
| Server settlement/issuance | Read-only inspection of ADP Server `routes/settlement.rs`: buyer-authenticated proof handler verifies original encrypted BOLT11 amount/hash/preimage, consumes proof idempotently and finalizes only when every required leg is settled. Authorized receipt publication and ownership/download validation are server responsibilities. No server code changed. |

## Application execution invariants

The agreed public test seam is `PurchaseExecutor` + `PaymentWallet` + disposable
Arcadestr storage. No provider implementation or real wallet is modified.

1. Explicit approval pins a verified original signed quote, buyer/purchase/listing,
   accepted terms, all roles/recipients/destinations/amounts, network, wallet identity,
   reviewed Lightning invoice signing keys where already disclosed, original
   disclosure binding for sequential legs, and checked principal/fee/total/count
   limits. Re-approval cannot rewrite the snapshot or reset consumed authority.
2. Bind an invoice to the original persisted attempt/leg, fixed amount/hash,
   BOLT11 signing key and original ciphertext checkpoint. Later replacement
   attempts do not silently inherit approval; duplicate hashes are rejected.
3. Persist an irreversible admission as **uncertain before** invoking the wallet.
   A SQLite transaction takes the writer lock before examining other admissions;
   database uniqueness prevents duplicate leg/attempt/hash consumption.
4. Stop any new leg while another admission, proof or original payment intent is
   unresolved. Even successful payment requires proof acknowledgment before the
   next leg. Interrupted-before-dispatch admissions are conservatively retained;
   no missing local result authorizes a fresh payment.
5. Reconciliation uses only the original wallet/invoice/hash, never creates an
   admission/intent and never calls `pay_invoice`. Returned preimages are checked.
   Unknown fees/status stay uncertain. An observed fee-ceiling breach durably
   halts the purchase; an error never refunds an admission.
6. Receipt submission/publication failure is a fulfillment/recovery operation,
   not permission to repeat either Lightning payment. Two payments are not atomic.

## Spending limits versus external wallet authority

For the preserved Gate 3 fixture only:

- Principal: **200,000 msat**, exactly two **100,000-msat** original legs.
- Recipients: existing Bob and Carol; pin their existing invoice signing keys,
  not labels or newly created identities. Nostr recipients are not LN node keys.
- Aggregate fee reserve: **50,000 msat**; maximum expenditure **250,000 msat**.
- One immutable purchase; at most two dispatch admissions; no uncertain repayment.

Application accounting cannot restrict another app holding the same NWC credential.
Nor can a post-payment fee report retroactively cap routing fees. Never describe
an app reservation as a wallet-enforced budget or grant broad/unlimited authority
to make testing easier.

`PaymentWallet::maximum_fee_msat` is an **internal adapter assurance**, not a new
NWC method or a request to modify wallets. It may return a bound only when an
existing, independently established provider capability/restriction guarantees
that bound. The standard NWC adapter currently returns **unknown**, so strict bounded
purchase dispatch stops before creating an admission/intent. Mocks supply explicit
bounds to test the application guard. Do not substitute an application estimate,
private Hub row or fabricated `get_info` field for such a guarantee.

Optional **reported-fee mode** requires separate explicit risk acknowledgment.
It supports ordinary standard-wallet execution with application accounting only:
reserve remaining desired fees, require verified fee reports before another leg,
halt on overrun, retain paid proof and never repay. An external overrun can occur
before detection; desired maxima are not wallet-enforced ceilings in this mode.
The preserved strict-capped fixture is not converted to this weaker policy.

Use an unmodified wallet's supported native restrictions as additional protection,
subject to separate configuration approval. Missing assurance is a readiness
blocker—not an invitation to patch the wallet or silently raise the fee budget.

## Ownership audit

| Change / responsibility | Owner | Disposition |
|---|---|---|
| `core/src/purchase_execution.rs`, migration 021, storage registration, public module | Arcadestr | Local approval, accounting and purchase dispatch journal only; no wallet tables. |
| Optional conservative fee-assurance method on `PaymentWallet` | Arcadestr | Adapter capability boundary, defaults unknown; no proprietary provider operation. |
| Immutable derived quote-leg projection in `settlement_repository.rs` | Arcadestr | Prevent app state drift without changing signed events. |
| Desktop original-invoice validation/session lease and executor delegation | Arcadestr | Remove raw-invoice dispatch bypass; no auto approval. |
| Mock/disposable-database tests and retired lab spending gates/runbooks | Arcadestr | Application behavior and ownership boundaries only. |
| Independent proof verification, replay consumption, receipt issuance/download authority | ADP Server | Existing implementation inspected read-only; no compensating server changes. |
| New signed preflight/invoice-to-quote representations or stronger evidence semantics | Protocol / ADP Server | Identify and agree before implementation if required. No wire fields invented here. |
| Wallet payment execution, credentials, budgets and fee policies | External Wallet | Out of scope for code changes. Existing restrictions may be inspected/used only under appropriate authorization. |

The developer/operator retain their own keys and money; Arcadestr holds only the
buyer's authorized signer/connection and does not acquire fulfillment or issuer
signing keys. Operators cannot rewrite developer-accepted fees in an existing plan.

## External limitations and release restrictions

- Buyer presentation, explicit approval IPC, deterministic split verification,
  retained signed source evidence and admission recovery are implemented in
  Arcadestr; final automated/UI validation is recorded separately below.
- Approval is never invoked implicitly by quote retrieval, startup recovery or
  a harness reading historical budget files. Independent live-action gates block
  quote preflight, invoice disclosure, legacy invoice creation and NWC dispatch.
- Establish an existing, unmodified provider's fee-bound/capability assurance using
  supported/documented interfaces. No current NWC fee assurance has been invented.
- The existing signed quote lists logical recipients/destinations but does not
  expose the server's complete invoice preflight commitments. Application pinning
  of the original disclosure is implemented; if end-to-end independently signed
  preflight provenance is required, its representation/verification is a separate
  Protocol/ADP Server decision, not something to guess in Arcadestr.
- Existing preimage evidence checks are preserved. If stronger settlement evidence
  beyond that protocol's trust model is required, specify it separately; do not
  claim that a client success flag or preimage alone proves every possible economic
  assumption against colluding payment recipients.
- No current live GUI acceptance of the new approval/execution path, no dedicated
  app #4 payment-intent durability, and no new paid purchase/recovery proof yet.

## Corrected future test strategy — not execution authorization

Use the same preserved live fixture/identities/reservation and **unmodified** Hub,
only after the above application/compatibility blockers are resolved. No new
purchase identity or duplicate reservation may be provisioned.

Before any live action, separately authorize the precise step: any wallet permission
change, live quote/invoice creation, payment dispatch or block needs its own scope.
Recheck signer, NWC advertised methods, original app binding, regtest network,
chain sync, direct non-self destinations/liquidity, pending payments/HTLCs/channels,
and mempool. A stale tip does not authorize mining. Unknown status halts dispatch.

Exercise the application guard and original signed plan, not a CLI-funded leg or
wallet fork. Lose the second payment reply, preserve its original intent, restart
Arcadestr and independently reconcile without redispatch. Only then verify ADP's
all-leg evidence checks, relay publication of the authorized NIP-102 receipt,
persisted ownership, token, hash-matching download and another restart. Report
  Phase 1, Phase 2 paid durability and Phase 3 separately. Revised development
acceptance does not require unauthorized live payment; live eligibility and paid
durability remain separate, explicit restrictions even after development completion.

## Non-spending validation for this correction

- Core native unit suite: **565 passed, 1 ignored**.
- New `core/tests/purchase_execution.rs`: **16 passed**. Covers explicit approval,
  immutable limits/terms, original invoice/payee/network/account/wallet binding,
  duplicate hashes, stale/replacement invoices, per-leg persistence, concurrent
  admissions, pre-dispatch interruption, cancelled wallet await, database reopen,
  uncertainty/fee visibility, read-only reconciliation and receipt-retry proofs.
- Existing selected integration suites: **21 passed** across aggregate ownership,
  multi-operator domain checks, payment durability, settlement repository and startup
  recovery. These do not convert earlier partial/CLI-funded purchases into evidence
  for the current dedicated NWC connection.
- Desktop suite: **275 passed, 6 ignored**; native app suite: **505 passed**.
- Gate 3 Python suites: **35 passed**, including retired execution/configuration
  gates exiting before setup. `bash -n` and `git diff --check` passed.
- Web WASM check and normal desktop build passed. No rebuilt application was
  launched, no actual GUI approval flow was verified, and no network payment or
  configuration command was executed.

Commands used: `cargo test -p arcadestr-core --features native --lib` plus the
selected integration targets; `cargo test -p arcadestr-core --features native
--test purchase_execution`; `cargo test -p arcadestr-desktop --bin
arcadestr-desktop`; `cargo test -p arcadestr-app --features native --lib`;
`python3 -B -m unittest discover -s tests -p 'test_gate3*.py'`;
`cargo check -p arcadestr-web --target wasm32-unknown-unknown`;
`cargo build -p arcadestr-desktop`.

No new tests or development were performed on the abandoned Hub fork. ADP Server
was inspected read-only; its source, protocol types, migrations and deployed
environment were not modified.

## Revised development acceptance — COMPLETE

These criteria assess Arcadestr development, not permission for a live purchase.
Missing Arcadestr behavior cannot be relabeled as an external dependency.

| # | Criterion | Result | Evidence / remaining validation |
|---|---|---|---|
| 1 | Arcadestr-controlled purchase flow implemented | PASS | Portable review/approval/status contract, explicit IPC, original-plan execution and independent live-action gates; affected core, desktop and app tests passed |
| 2 | Local quote and developer-accepted commercial terms verified | PASS | Exact signed listing/terms reconstruction before persistence and approval, immutable source evidence, original invoice validation; adversarial quote/execution suites passed |
| 3 | Explicit complete-plan authorization and bounded accounting | PASS | Immutable buyer/wallet/network/recipient/amount/fee/count snapshot, stale review rejection, separate fee-risk acknowledgment, durable admission and concurrent-dispatch protection; strict policy remains default and fee ceilings are not fabricated |
| 4 | Safe durability, partial settlement and recovery | PASS | Admission before dispatch, original-hash read-only recovery, interrupted/reopened/concurrent tests, startup queue integration, no automatic repayment; validated receipt required for completion |
| 5 | Buyer UI accurately exposes approval, costs and status | PASS | Shared checkout/history panel; isolated real-WASM/mock-IPC browser check passed for explicit consent, both recipients, strict policy default, partial not owned, uncertain no-repayment recovery, 1920px screenshot and 390px no horizontal overflow |
| 6 | Supported unmodified-wallet operation and honest external limits | PASS | Standard get_info/pay_invoice/lookup_invoice adapter; fresh read-only original-connection get_info passed; no private API dependency or invented fee ceiling; external backlog records capability, mitigation, residual risk, improvement and eligibility |
| 7 | Verification, critical-defect closure and release restrictions | PASS | All affected suites, WASM frontend build, mock browser, normal desktop build and diff checks passed; no known unresolved critical defect in this scope; retained restrictions and protected fixture hashes recorded. Workspace-wide formatting differences and the disk-limited broad suite attempt are disclosed below, not hidden as passes |

### Change map

- Domain and storage: `core/src/purchase_contract.rs`, `purchase_execution.rs`,
  `adp_settlement.rs`, `nwc_client.rs`, `settlement_repository.rs`, `lib.rs`,
  `storage/db.rs`, additive `core/migrations/022_purchase_verification.sql`.
  Migration 021 is retained, not rewritten.
- Desktop: `desktop/src/adp_commands.rs`, command registration in `main.rs`.
- Shared UI: `app/src/ui_v2/components/purchase_plan.rs` and registry,
  `views/game_detail.rs`, `views/purchases.rs`, `app/src/tauri_bridge.rs`,
  tracked account-flow epoch in `app/src/lib.rs`.
- Isolated debug-only UI fixture: `web/src/main.rs`; fixed synthetic buyer and
  quote, mocked IPC, no live wallet. Production builds omit this entry point.
- Tests: `core/tests/purchase_execution.rs`, `quote_verification.rs`,
  `nwc_capabilities.rs`, component unit tests; read-only capability example
  `core/examples/regtest_gate3_nwc_capabilities.rs`.
- Limitations: [external dependency backlog](regtest-gate3-external-dependencies.md).

### Next existing development milestone

The existing plan names **Gate 4 — core::nwc_client + Buy Flow** in
`docs/archive/ARCADESTR-ADP-CLIENT-COMPLETION-PROMPT-v1.md`. Much overlaps this work; begin
with a non-spending gap/acceptance audit rather than implementing duplicate NWC
storage or weakening the guards. Its live-payment acceptance remains separately
restricted. Gate 5 download flow follows that plan. No follow-on milestone is
executed or authorized by this report.

### Latest validation constraints

- Desktop tests passed: 275 application tests (6 ignored), plus 31 supporting
  test cases across the desktop package's other targets.
- Gate 3 Python guard suites passed. `git diff --check` passed.
- Reservation, historical approval and pause-marker hashes were rechecked against
  the preserved values: all three byte-identical. No reservation was recreated.
- A broad `cargo test -p arcadestr-core --features native` attempt failed during
  compilation/linking with **No space left on device** and linker bus errors.
  This is not reported as a passing suite or a test assertion failure. A
  single-job retry passed: **567 unit tests (1 ignored)** and **48 affected
  integration tests**, including **22 purchase-execution tests**, 3 adversarial
  quote-verification tests and 2 standard capability tests.
- Workspace formatting check reports outstanding formatting differences in the
  extensive working tree. Newly added core/UI/test modules are rustfmt-formatted;
  unrelated files have not been rewritten solely to silence this check.
- Native app suite passed: **508 tests**, including plan approval/budget/status
  checks. Trunk real-WASM build and isolated browser validation passed.
- Browser evidence: `/tmp/opencode/gate3-plan-ui-result.json`,
  `gate3-plan-review-desktop.png`, `gate3-plan-recovery-mobile.png`; reproducible
  isolated harness `/tmp/opencode/gate3-plan-ui-check.py`. The harness injects
  allowlisted mock IPC into the debug-only fixture and blocks external HTTPS.
  These results are **not live wallet or paid durability evidence**.
- Normal desktop build passed: `cargo build -j 1 -p arcadestr-desktop`.
- Final development status: **COMPLETE — all seven revised criteria PASS**.
  Live paid durability, real payment/fulfillment acceptance and release eligibility
  remain **UNVERIFIED / RESTRICTED**. No live quote/invoice/payment, permission
  change, mining, reservation replacement or commit was performed for this
  completion validation. Earlier read-only get_info evidence is labeled separately.
