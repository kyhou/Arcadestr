# Combined Gate 3 / Gate 4 source baseline

Gate 3 and Gate 4 are committed together rather than reconstructing an
unverifiable intermediate source state. Git contains no pre-Gate-4 snapshot;
the shared checkout and review UI includes overwritten Gate 3 implementation.
Splitting those changes would require speculative reconstruction. This baseline
does not represent a separately validated historical Gate 3 tree.

The combined scope includes durable purchase execution and recovery, explicit
wallet-bound approval, NWC integrations, receipt/listing validation, checkout
completion guards and acquisition-backed Library projection. The existing
[Gate 3 scope](regtest-gate3-arcadestr-scope.md) and
[Gate 4 audit](regtest-gate4-buy-flow-audit.md) describe the individual concerns.
Historical runbooks retain their original narrower verification boundaries;
development completion does not establish unverified live-wallet guarantees.

## Required schema and test dependencies

- 018 adds the payment journal and encrypted proof checkpoint used by durable
  payments and settlement recovery.
- 019 repairs derived leg progress from existing settled evidence without
  creating ownership or promoting unpaid quotes.
- 020 adds refund attempts, reconciliation progress and journal audit counters
  read and written by recovery and payment execution.
- 021 persists immutable purchase approval and irreversible dispatch admissions.
- 022 persists the original signed listing and terms used by quote verification
  and receipt confirmation. All five migrations are registered in storage.

Earlier regtest examples and fixtures are included because the retained Gate 3
harness builds on them. Their Axum and sibling ADP Server development dependencies
support fixture servers and adversarial tests, not a new production dependency.
Bitcoin/invoice helpers generate deterministic test invoices; the standalone
keyring backend matches desktop account-scoped wallet storage. Merely committing
these helpers does not authorize running spending or infrastructure commands.

## Preserved boundaries

ADP Server prerequisites are recorded independently in its repository:

- `83bbf78`: bounded uploads and upload-capacity configuration.
- `99b8a24`: deterministic settlement clocks and multi-operator tests.
- `504cea8`: recovery/refund retries, history routes and migrations 0007/0008.
- `c4423a0`: exact raw-target download authentication.
- `df6d02f`: the separate Polar LNURL fixture and synthetic regressions.

Use sibling ADP Server revision `df6d02f` for this source backup. The Cargo path
dependency does not pin that revision automatically. Independent relay-policy,
protocol-wide and documentation changes remain outside those server commits.
Server recovery limitations are recorded in `docs/settlement-recovery-backup.md`.

This is a development backup, not a new acceptance certification. Basic build
checks do not establish live-payment safety or complete test-suite success. The
earlier milestone reports retain their historical scope and verification claims.

Unrelated repository instructions, broad architecture-document refreshes,
machine-specific documentation links, frontend-cache changes and generated web
output are excluded. Credentials, private operational evidence and checkpoints
remain outside the commit. Existing history is not rewritten. Receipt portability
is follow-on work and is not implemented or claimed by this baseline.
