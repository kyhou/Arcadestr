# Gate 4 — NWC / buyer-journey gap audit

**Development status: COMPLETE. All eight acceptance criteria PASS.**
This is not live-settlement verification or authorization to spend.

## Authority and baseline

Gate 3 development remains **COMPLETE**. This audit extends its integrations; it
does not replace its executor, approvals, migrations 021/022 or acceptance report.
No provider, ADP Server or protocol specification is modified. No credential or
permission changes, live quote/invoice/payment, mining or commit is authorized.
The independent live-action gates remain in place and app #4 remains read-only.

Baseline inspected: `regtest-gate3-arcadestr-scope.md`,
`regtest-gate3-external-dependencies.md`, `ADP-01.md` (confirmation/download and
historical authorization), `NIP-102.md` (aggregate profile, status chains and
ownership), `NIP-103.md` (non-monetary grants and unallocated-kind warning), and
the current code/tests below. Earlier completion counts were not substituted for
new regression runs. The working tree already contained extensive uncommitted
work; only scoped patches were applied.

## Buyer-journey coverage

| Stage | Actual implementation traced | Verification / disposition |
|---|---|---|
| Discovery and eligibility | `core/src/marketplace.rs`, `app/src/ui_v2/views/game_detail.rs::select_primary_action`; signed listing, price and platform validation; owned access precedes campaigns/payment; public/timed access does not synthesize ownership | Existing marketplace/action tests cover zero price vs public, timing boundaries, existing ownership, campaign precedence, unavailable platforms and sign-in. No new payment path needed |
| Purchase preparation | `desktop/src/adp_commands.rs::start_split_purchase`, `core/src/adp_settlement.rs::verify_quote_listing_terms`; processor/listing/refund binding and exact developer-accepted split; immutable evidence in migration 022 | Adversarial quote tests and immutable execution tests retained. Quotes preflight live invoices, so actual creation remains blocked |
| Wallet connection | Account-scoped OS-keyring NWC storage, safe metadata, replacement/disconnect/reconnect; `get_info` capability parsing; signed/encrypted NIP-47 transport; unknown lookup never authorizes repayment | Existing NWC storage/transport/capability and UI generation tests. Known missing pay_invoice now disables checkout independently of connection verification; unknown get_info is not fabricated permission or a mandatory protocol extension |
| Buyer approval | `PurchasePlanPanel`, `approve_purchase_plan`, `PurchaseExecutor::approve_verified_with_fee_policy`; full recipients/destinations/amounts/terms, limits, network, original keys when disclosed, fee-risk policy and immutable wallet-bound consent | New checkout gating consumes current review; wallet changes invalidate it. Material quote/terms/wallet changes do not inherit old consent or reset admissions. New purchase/consent is required only after old obligations are safely resolved; no in-place rewrite |
| Payment execution | Desktop original-invoice dispatch exclusively delegates to Gate 3 executor under account/session lease; admission precedes wallet; exact hash/amount/network/payee; concurrent and duplicate protection | Existing 22 execution tests cover interruption, cancellation, stale/substituted invoices, cross-account/wallet changes, partial settlement, uncertainty, fee reporting/overrun and read-only recovery. Live-action gate unchanged |
| Confirmation | Original proof is encrypted/checkpointed before ADP submission; ADP owns independent verification/issuance; client validates receipt signature, buyer, issuer authorization, aggregate quote/legs and persists receipt/token | Correct missing/inconsistent receipt response handling; a settled status or replay conflict no longer grants ownership. Original proof remains recoverable and no new invoice/payment is required for a receipt retry |
| Ownership and Library | `OwnershipService`, purchase/grant repositories and history; native `get_listing_ownership`; Library projects saved rows plus validated active acquisitions | New acquisition projection makes paid/claimed but uninstalled games discoverable without manual Save. Dedupe, account isolation, restart, refund/revocation and saved-without-ownership tests |
| Download/install | Existing native install/download path refetches authoritative listing; independent distribution authorization, token/account scope or NIP-98, content hash verification, installation registry | Existing desktop tests for unowned rejection before download, token Path A, NIP-98 Path B, other-account token rejection, fresh listing and installation safety. Not reimplemented; next milestone should audit remaining download gaps |
| History/restart/reconnection | Existing proof journal, admission/recovery queue, startup/manual read-only reconciliation and history card; shared review panel | Recovery now notifies parent history/checkout to refresh state; wallet configuration changes automatically invalidate stale reviews. No uncertainty recovery calls pay_invoice |

## Confirmed findings and scoped fixes

All findings below are Arcadestr-owned; none was reclassified as a wallet limit.

| ID | Classification / severity | Evidence and impact | Correction |
|---|---|---|---|
| G4-01 | DEFECT / high | `game_detail.rs::confirm_with_preimage` treated HTTP 409 as purchased. ADP-01 defines 409 as proof replay, not ownership; a conflict could incorrectly expose Install | Query authoritative local ownership for the initiating account/listing; if not established, show receipt recovery instead of claiming ownership |
| G4-02 | INTEGRATION / high | Split checkout accepted `state=settled` with no receipt; `submit_split_payment_inner` could return that response after acknowledging a leg | Validate settlement response consistency after retaining the acknowledged proof; missing receipt is a recoverable error, never acquisition. UI also requires receipt material from the validated desktop path |
| G4-03 | DEFECT / high | `on_buy` wrote quote/attempt/leg state inside awaited mappings before checking account identity; an old response could contaminate a new checkout | Fence each response before state writes using both initiating account and authentication epoch, including A → B → A; stale responses do not reset another operation |
| G4-04 | INTEGRATION / medium | Plan approval/capabilities/recovery were locally rendered but not connected to the parent checkout/card; old review could survive wallet replacement and parent status could remain stale | Optional review/recovery callbacks; wallet-driven invalidation; current approval/capability/expiry/leg-readiness gating; authoritative ownership refresh and history reload after recovery |
| G4-05 | INTEGRATION / medium | Native Library listed only manually saved rows. Valid paid/claimed, uninstalled acquisitions were absent unless separately saved | Read-only Library projection unions saved rows with validated active acquisitions via existing OwnershipService; no new entitlement or ownership inference |
| G4-06 | MISSING / medium | NIP-102 aggregate profile requires disclosure of public split/payment correlation; approval panel lacked it | Explain public buyer/listing/recipient/amount/hash correlation and encrypted raw evidence before consent |
| G4-07 | INTEGRATION / high | Legacy and split receipt-persistence paths validated authority using the receipt coordinate without first binding it to the requested signed listing; a valid receipt for another game could be mis-associated with confirmation/token state | Shared core `validate_receipt_listing_binding` checks signed listing and exact coordinate before either persistence path; adversarial cross-publisher, cross-game and tampered-listing tests |

## Deferred dependencies and optional improvements

- **EXTERNAL LIMITATION, high live impact:** no independently established standard
  NWC fee ceiling. Strict provider-ceiling mode still refuses unsafe execution;
  reported-fee mode remains separately acknowledged application accounting, not a
  wallet guarantee. No hypothetical wallet update is required to complete Gate 4.
- **EXTERNAL LIMITATION:** outgoing lookup/fee reports may be unavailable; retain
  original uncertainty and stop subsequent dispatch. No automatic repayment.
- **PROTOCOL DEPENDENCY:** NIP-103 entitlement kind is unallocated. Existing local
  experimental profile is not a claim of production interoperability; agree an
  allocation before interoperable release. Do not use NIP-15 kind 1021.
- **PROTOCOL DEPENDENCY / existing trust model:** ADP v1 attests recipient invoice
  provenance; a preimage alone is not proof of economic recipient identity. Stronger
  signed preflight commitments need cross-project agreement, not client-invented fields.
- **EXTERNAL / protocol constraint:** sequential Lightning payments are not atomic;
  partial payment remains possible. Aggregate receipts publicly correlate the split.
- **OPTIONAL IMPROVEMENT:** multi-wallet picker, QR-based URI import and batch NWC
  payments are not prerequisites. Existing per-account replace/reconnect is supported;
  multi_pay_invoice is not assumed atomic and no duplicate orchestration is added.
- **OPTIONAL IMPROVEMENT / verified remaining Arcadestr gap:** autonomous
  cross-device relay receipt discovery is not connected to the buyer UI.
  `desktop/src/main.rs::ingest_receipt` and its bridge exist, but no UI caller or
  automatic receipt sync was found. Current NWC checkout acquires a validated
  receipt from the ADP response and persists it for restart/history/Library; that
  device-local journey does not require autonomous discovery. Do not claim that
  external purchases or a fresh device are automatically synchronized. A separate
  portable-sync feature must reuse authorization/chain verification (including
  delegated roots and status updates), not simply import arbitrary relay events.
- **Release restriction, not development blocker:** real-wallet settlement, paid
  restart durability and full real-server fulfillment remain unverified. No live
  operation was attempted for this audit. See the retained Gate 3 external backlog.

## Validation and acceptance

Affected core runs passed: **569 unit tests (1 ignored)** and **50 integration
tests**, including 23 purchase-execution tests and the new adversarial receipt/
listing binding test. Final app suite passed **511 tests**. Final desktop run passed
**275 application tests (6 ignored)** plus **31 supporting-target tests**; 35 Gate 3
Python guard tests passed. Real-WASM Trunk build and isolated browser regression
passed, including parent callbacks, capability gating and wallet invalidation.
`git diff --check` passed. Gate 4 is **COMPLETE**; Gate 3 remains COMPLETE.

| Acceptance criterion | Current result |
|---|---|
| 1. Buyer journey audited end to end | PASS — coverage matrix, direct ADP receipt acquisition and optional cross-device discovery boundary explicitly distinguished |
| 2. Existing implementation distinguished from gaps | PASS — seven concrete findings; no duplicated executor, wallet storage or download implementation |
| 3. Confirmed required Arcadestr fixes implemented | PASS — seven scoped corrections above and targeted regressions |
| 4. Relevant automated tests pass | PASS — affected core/app/desktop/Python suites, real-WASM build and isolated mock-IPC browser regression |
| 5. Gate 3 safeguards remain intact | PASS — all existing execution tests plus missing-permission regression and 35 Python guards; migrations/checkpoints preserved; live gates unchanged |
| 6. External/protocol dependencies documented | PASS — retained backlog plus Gate 4 additions; no provider or protocol edits |
| 7. No known critical Arcadestr purchase-flow defect remains | PASS within audited device-local NWC journey — confirmed receipt-binding, false-completion and stale-account defects corrected; optional cross-device discovery gap explicitly disclosed |
| 8. Unverified live guarantees accurately restricted | PASS — no new live quote, invoice, payment, permission or block action; paid durability not claimed |

Regression commands (all wallets/HTTP effects are test fixtures):

```sh
cargo test -j 1 -p arcadestr-core --features native --lib --test purchase_execution --test quote_verification --test nwc_capabilities --test receipt_listing_binding --test aggregate_ownership --test multi_operator_gate2 --test payment_durability --test startup_settlement_recovery --test settlement_repository
cargo test -j 1 -p arcadestr-app --features native --lib
cargo test -j 1 -p arcadestr-desktop
python3 -B -m unittest discover -s tests -p 'test_gate3*.py'
# In web/:
trunk build --dist /tmp/opencode/gate4-frontend
# In repository root, after that debug build:
python3 -B scripts/regtest-gate4-ui.py /tmp/opencode/gate4-frontend
git diff --check
```

The browser harness runs real WASM with allowlisted mock IPC, a fresh Chromium
profile, loopback static hosting and blocked external HTTPS. It verifies explicit
consent, privacy disclosure, current parent approval, missing pay capability,
wallet-change invalidation, partial/uncertain recovery and parent notification at
desktop/mobile widths. It cannot provide live Lightning or paid durability evidence.

Evidence logs: `/tmp/opencode/gate4-core-final.log`, `gate4-app-verified.log`,
`gate4-desktop-final.log`, `gate4-gate3-guards.log`, `gate4-frontend-build.log`,
`gate4-browser.log`; browser result `/tmp/opencode/gate4-ui-result.json` and
screenshots `gate4-plan-desktop.png`, `gate4-plan-mobile.png`. Existing broad
working-tree formatting differences were not reformatted away as unrelated work;
this report does not claim a clean workspace-wide formatting check.

Protected reservation, historical approval and pause marker were rechecked against
their preserved SHA-256 values: all three byte-identical. Migrations 021 and 022
were not edited. No abandoned wallet-fork or ADP Server code was modified.

Changed production files: `core/src/{purchase_contract,adp_client,purchases,ownership}.rs`,
`desktop/src/{adp_commands,main}.rs`, `app/src/tauri_bridge.rs`,
`app/src/ui_v2/components/purchase_plan.rs`, and
`app/src/ui_v2/views/{game_detail,purchases,library}.rs`. New/extended tests live in
those modules, `core/tests/{purchase_execution,receipt_listing_binding}.rs` and
`scripts/regtest-gate4-ui.py`. Scope report and external backlog are the only
documentation changes in this audit; existing Gate 3 completion remains intact.

Next development priority based on a concrete remaining gap: **portable relay
receipt synchronization/import**, with delegated authority and full status-chain
validation. This extends cross-device acquisition discovery; it is not a second
payment orchestrator or a blocker for the implemented device-local NWC journey.
The next named milestone is **Gate 5 download/install gap audit**, starting with
the existing Path A/Path B and distribution/hash/registry tests rather than
rewriting implemented download logic. No follow-on work or live execution is
authorized by identifying these priorities.
