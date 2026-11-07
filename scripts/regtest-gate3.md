# Gate 3: startup recovery and refund GUI

Current ownership/execution authority is defined in
[Arcadestr-owned Gate 3](regtest-gate3-arcadestr-scope.md).
This file records historical results, not authority to configure a wallet or pay.

## Status

Implemented and exercised in the actual Linux Tauri/WebKit desktop. **Not a claim
of complete reboot/fault-matrix acceptance**: no machine reboot, locked remote
signer, or new GUI-level NWC relay outage was tested.

Private evidence for this run:
`~/.local/state/arcadestr-regtest/gate3-runs/1793333272393157146/`.
It contains SQLite backups, screenshots, GUI observations, payment counts and
test logs. Account copies and evidence are owner-only. Do not publish them.

## Persistence corrections

- Migration 019 repairs only the derived settlement-leg projection. The original
  Gate 2 client changed from three pending legs to three settled legs; signed
  quote/receipt bytes, encrypted proof bytes and authoritative states were
  preserved. The migration check constructs no network or payment client.
- Receipt/attempt updates are monotonic and transactional where they affect the
  quote projection. Replaying partial progress cannot reopen a settled purchase.
- Actual GUI testing found that acquisition history still applied the legacy
  singular-payment proof check to aggregate receipts. History now uses existing
  aggregate parsing and persisted-chain validation, including fork rejection.
  Protocol semantics and historical signed events are unchanged.

## Recovery behavior

Startup and manual recovery share account/session-epoch claims. They reconcile
only stored attempts with an original wallet intent or encrypted proof. They do
not request replacement purchase invoices, create payment intents or dispatch
payments. Unknown/not-found outcomes remain blocked. A session round trip does
not restore the validity of an old claim.

Recipient actions use authenticated operational obligation reads. Preparation
persists the original account-encrypted invoice; sending requires explicit GUI
confirmation. The existing durable NWC journal binds the account, wallet/app,
invoice hash and amount. An unresolved refund has a recovery action, not a repay
action. An already verified server obligation cannot be paid from stale local UI.

## Actual GUI coverage

| Scenario | Observation |
| --- | --- |
| Completed Gate 2 purchase | Settlement completed and acquisition access active; 1,000-sat amount displayed correctly |
| Buyer proof checkpoint recovery | Actual app restart restored receipt, ownership and token without dispatch |
| Buyer wallet reconciliation | In a private fault-injected copy, the original intent remained but cached proof/result were absent; startup made one read-only NWC lookup and recovered receipt, ownership and token |
| Recovered download | Restored token downloaded the expected 30-byte payload with matching SHA-256 |
| Recipient refund | Load, prepare, explicit confirmation and send through real GUI controls; one 100-sat NWC refund, authenticated verification |
| Interrupted confirmation | A controlled HTTP confirmation outage left the real second refund's encrypted proof checkpointed; GUI showed recovery and no pay button |
| Interrupted refund restart | Actual app restart after ending that outage verified the original proof; refund dispatch count remained one |
| Account isolation | Switching Alice → Bob hid Alice's settlement and recipient obligations; wrong-recipient preparation was rejected; switched back using existing saved identity |
| Concurrent scans | Two overlapping scans through the actual app produced one shared claim and one safe rejection |
| Signed out | Recovery rejected without an active account; existing saved Alice identity restored without importing a key |

Buyer interruption states were deliberately injected **only into private SQLite
copies**, with backups taken first. This is not evidence of a spontaneous crash
or a machine reboot. Refund payments and wallet reconciliation were real.

The new recipient fixtures use the established **controlled event lookup and
deadline clock**, not live relay propagation of commercial terms. They do not
replace the existing operators' live terms. Existing Gate 1 live-relay evidence
remains separate.

An initial exploratory GUI launch used an unsupported `--relays` spelling and
therefore displayed the default relay set. No payments were made in that launch.
It was stopped, corrected to repeated `--relay` flags, and replaced with the
private, localhost-only environment used for all acceptance scenarios above.

## Payment and environment evidence

- Existing Bob paid existing Alice 100 sats for each of two isolated partial
  purchases. Neither partial purchase received a receipt or download token.
- Existing Alice refunded each original principal through her unchanged NWC
  application, once each. No recipient wallet/application was created.
- All four channel-balance totals returned exactly to their starting values;
  no pending HTLCs remained. The new conservative 600-sat reservation was retained,
  together with all historical reservations.
- One regtest block was mined with explicit authorization after checking that
  there were no pending HTLCs/channels or mempool transactions. Height 174 → 175
  restored LND readiness; the existing coinbase destination was reused.
- Only supervisor-owned ADP processes were restarted for the new read endpoints.
  The isolated GUI fixture servers were stopped gracefully after verification.

## Verification

All five earlier non-paying baseline modes passed after readiness was restored:

```sh
./scripts/regtest test --verify-paid
./scripts/regtest test --verify-durability
./scripts/regtest test --verify-refunds
./scripts/regtest test --verify-gate1
./scripts/regtest test --verify-gate2
```

Focused persistence/recovery tests:

```sh
cargo test --locked -p arcadestr-core --features native --lib \
  --test settlement_repository --test payment_durability \
  --test startup_settlement_recovery --test aggregate_ownership
cargo test --locked -p arcadestr-app --lib
cargo test --locked -p arcadestr-desktop
```

Full source runs passed: core 557 unit tests plus 18 focused integration tests
(one unit test ignored), app 501 tests, desktop 304 tests (six ignored), and ADP
server 121 tests (one ignored). Workspace formatting and both repositories'
diff-whitespace checks passed. Final suite output and baseline rechecks are
retained in the private evidence directory. Preserve any failed-test evidence
rather than treating compilation as GUI acceptance.

`regtest_gate3_projection` requires an explicit backed-up-client argument and
runs the normal database migrations. `regtest_gate3_gui` requires explicit payment
opt-in and a private budget reservation; its create-new intent marker prevents
blind redispatch on rerun. It creates real principal obligations, **never sends
the refunds automatically**, and must not be rerun against this completed run.

Remaining coverage: coordinated real machine reboot, locked/unavailable remote
signer GUI behavior, and new GUI-level NWC relay/wallet outage cases. Native tests
cover missing wallets, unresolved/not-found outcomes, wallet/application mismatch,
malformed checkpoints, stale sessions and at-most-once dispatch; that is not a
substitute for the remaining GUI/runtime cases.

## Outstanding-scenario follow-up

The original run above is retained unchanged. The subsequent NWC outage GUI
verification, scoped NIP-46 disconnection result, and verified manual-reboot outcome
are recorded in [regtest-gate3-outstanding.md](regtest-gate3-outstanding.md).
NWC outage safety and changed-boot startup recovery are now verified.
The user's existing remote signer subsequently passed paid-leg recovery across
manual bunker stop/restart, without any recovery payment or access issuance.
Explicit `sign_event` and `nip44_decrypt` refusal/regrant subsequently passed with
an isolated persistent `ohstr/ncli` signer and actual GUI/local-relay evidence;
see [regtest-gate3-denial.md](regtest-gate3-denial.md).
**Gate 3 is still open:** these scoped results do not verify NIP-46/NWC-bound
durability or complete NIP-46 purchase recovery.

The subsequent three-phase follow-up is tracked in
[regtest-gate3-integration.md](regtest-gate3-integration.md). Restored-session
signing/decryption, signer restart, account isolation and unavailable-signer
safety now pass without payment. Dedicated NWC read-only binding/persistence and
isolation also pass. Paid durability and complete purchase recovery remain open;
a new 250-sat total budget is approved but no payment has been dispatched.
