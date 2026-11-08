# Gate 6 Phase 3 — Commercial configuration

Phase 3 development: **implemented and validated**. Scope: G6-02 / G6-03 only.
Gate 6 remains **IN PROGRESS**. No recovery journal,
schema change, payment, live publication, or external-service modification belongs
to this phase.

## Implemented contract

- `PublisherCommercialIntent::Preserve` is the default, including older IPC
  payloads. Publication reads exact `adp_terms` references from the verified
  authoritative listing and carries their original tags forward. A new operator
  advertisement is not consent. Unreviewed legacy `accepted_terms` input is
  rejected rather than being silently applied.
- `ReplaceAfterReview` carries the entire reviewed context, exact root IDs, parsed
  signed offers, recipients and deterministic payment legs. The desktop obtains
  fresh endpoint capabilities and exact signed roots, checks operator identity,
  version 1 and experimental kinds, checks observed withdrawals, and compares the
  reconstructed review before accepting the change. Validation precedes
  publication-side signing/provisioning and is repeated after signing before
  relay publication. Account/session and operation fences remain in force.
- The editor requires an explicit choice to replace acceptance and a separate
  approval after displaying fees, destinations, root IDs, limitations and exact
  msat allocations. Price, destination, selected-server or publisher changes
  invalidate the local review. No quote, invoice, wallet action or LNURL invoice
  request is needed for the calculation.
- Fee arithmetic, coverage, canonical leg ordering and zero-leg omission reuse
  the existing settlement primitives. Multiple URLs for one operator produce at
  most one operator fee. Operator-authored limitations are disclosed verbatim;
  this client does not invent executable predicates for that free-text field.
- Preserving existing references on a public/non-payment listing creates no
  payment legs. Grants and public/timed access do not require fee settlement.
- Current metadata edits do not rewrite stored purchase plans, receipts,
  historical signed listings or commercial roots. A later withdrawal blocks new
  acceptance/publication under that offer; it does not reinterpret an already
  completed sale or invalidate historical ownership.

## Retirement boundary

ADP-01 defines an operator-signed offer withdrawal that stops new quotes without
altering issued quotes, receipts or ownership (`ADP-01.md`, experimental split
profile). It also requires exact operator coverage for split-enabled listings.
It does not specify a complete developer-side retirement workflow spanning
server removal, outstanding quotes and transition to legacy sales.

`Retire` is therefore a distinct, rejected IPC intention and an unavailable UI
option, not an alias for clearing `adp_terms`, changing distribution authority,
revoking grants or refunding sales. Those prospective semantics require separate
agreement. Preserve mode will reject coverage mismatches instead of silently
retiring an operator when the server set changes. This limitation does not block
preserve or explicitly reviewed replacement with complete valid coverage.
Replacement also refuses to drop an already accepted operator, so changing the
server set cannot bypass the unavailable retirement operation.

## Evidence availability

Use the existing EOSE-aware relay transport. Missing exact roots, failed or
incomplete connected-relay responses, and saturated bounded terms-history queries
block the review/publication. The terms format puts withdrawals in JSON rather
than an indexed root tag, so the client queries bounded operator kind-1032
history. More than the bounded history requires a future discovery enhancement;
the client does not claim that truncation establishes absence.

Successful queries establish only the evidence returned by connected relays,
not global relay completeness or proof that an unseen withdrawal does not exist.
The editor discloses this limit. ADP Server still independently validates new
sales. Mock fixtures are not evidence of live relay retention or settlement.

## Focused validation

- `core/tests/publisher_commercial.rs`: forged/wrong-operator/unavailable roots,
  withdrawal, changed advertisements, preserve/replace/retire, multi-operator
  allocation, fee floors/bounds/overflow, capability mismatch, non-payment
  preservation and historical sale snapshots.
- `core/tests/quote_verification.rs`, `multi_operator_gate2.rs`,
  `purchase_execution.rs`, `payment_durability.rs`,
  `startup_settlement_recovery.rs`, `settlement_repository.rs`,
  `aggregate_ownership.rs`, `authorization.rs`, `receipt_listing_binding.rs`,
  `receipt_portability.rs`: existing purchase and historical-credential guards.
- Core ADP publication/settlement unit tests; desktop `adp_commands::tests`
  excluding `live_`; app `publish` tests; native checks; desktop-bridge WASM check.

No live signing, assets, purchases, invoices, payments or credentials are used.

### Validation results

- `cargo test -p arcadestr-core --features native --test publisher_commercial`:
  **14 passed**.
- `cargo test -p arcadestr-core --features native --lib adp_ -- --skip live_`:
  **97 passed**.
- `cargo test -p arcadestr-desktop adp_commands::tests -- --skip live_`:
  **84 passed**.
- `cargo test -p arcadestr-app --lib publish`: **171 passed**.
- Quote verification, multi-operator Gate 2, receipt/listing binding and receipt
  portability: **26 passed** across their four integration targets.
- Purchase execution, payment durability, startup settlement recovery, settlement
  repository, aggregate ownership and historical authorization: **53 passed**
  across their six integration targets.
- Native core/desktop/app checks and
  `cargo check -p arcadestr-app --target wasm32-unknown-unknown` passed.
- Targeted changed-file Rust formatting and `git diff --check` passed.

Logs: `/tmp/opencode/gate6-phase3-*.log`. No interactive UI or live-service
validation is claimed. The WASM check uses the existing desktop-bridge build,
not the independently limited standalone-web feature. Existing unrelated warnings
and working-tree changes remain outside this phase.
