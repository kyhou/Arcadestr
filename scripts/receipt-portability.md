# Portable NIP-102 receipt synchronization and import

Development acceptance: **COMPLETE — all ten criteria PASS**.
Gate 3 and Gate 4 remain COMPLETE. No live cross-device or Lightning verification
is claimed. No wallet, ADP Server, protocol, permission or payment action belongs
to this milestone, and no Gate 5 implementation is included.

## Existing components reused

- `purchases.rs`: signed-event parsing, payment declarations/bindings, root
  authorization, receipt-chain resolution, purchase persistence and ownership.
- `authorization.rs`: immutable fulfillment roots, capability and coordinate
  binding, cancellation and historical issuance-time verification.
- `ownership.rs`: account acquisition records, NIP-103 separation and Library
  projection. Install/download eligibility continues through existing ownership.
- Existing `RelayManager` and configured relay pool; no parallel Nostr client.
- Existing desktop authentication/session recovery fences; public receipt reads
  need no private key or NIP-44 decryption.

## Six implementation phases

1. **Shared verification:** the existing receipt-chain resolver now accounts for
   all supplied members rather than silently ignoring malformed/disconnected
   descendants. Persisted ownership uses the same structural traversal, and
   History no longer selects status by timestamp. Portable evidence adds explicit
   VERIFIED / INDETERMINATE / INVALID assessments; missing predecessors or authority
   do not grant access. Conflicting roots, forks, changed invariants, bad authority
   and invalid public evidence fail closed. A valid refunded/disputed chain can
   be VERIFIED cryptographically without granting ownership.
   Aggregate proof types, canonical hashes/references and ordered positive legs
   are checked by the existing receipt parser. Public Zap proofs reuse its
   evidence verifier; aggregate legs resolve their LSP from the exact signed
   sale listing and developer-accepted signed operator terms, not recipient keys.
2. **Durability:** additive migration `023_receipt_portability.sql` stores
   buyer-scoped signed evidence, chain assessments, observed public LSP keys and
   per-relay cursors. Migrations 021/022 are unchanged. Verified chain rows and
   assessments commit atomically; unresolved evidence is retained for retries.
   Identical signed events are deduplicated and original ciphertext retained.
3. **Discovery:** configured relays receive bounded `kind:1020` / `#p` queries.
   Initial/backfill pages overlap timestamp boundaries rather than skipping equal
   timestamps; saturation is reported incomplete. Incremental scans overlap their
   checkpoint, unfinished backfills resume, and periodic backfills plus explicit
   rescan cover late evidence. Missing predecessor IDs, authorization chains and
   public Zap proofs are fetched with bounded dependency closure. Startup/sign-in,
   account changes, relay reconnects and a 60-second retry coordinator trigger work.
4. **Manual import:** Purchase History accepts one signed Nostr event or a JSON
   array, up to 4 MiB / 2048 events. The array may include signed receipt ancestors,
   authorization events and public Zap evidence. It is not a custom export format.
   Import is offline: no relay is required for a self-contained supported chain.
   Missing evidence is retained as INDETERMINATE; Sync can retrieve dependencies
   and public LSP metadata later. The old single-event IPC also routes through
   this same pipeline; there is no import bypass.
5. **Application integration:** account-scoped state exposes progress, errors,
   rejected inputs, incomplete discovery and all three assessment outcomes.
   Ownership revisions refresh History, Library and game-detail eligibility.
   Library includes verified, active, uninstalled purchases without a separate
   Save action. NIP-103 grants remain separate credentials. Receipt refreshes do
   not trigger settlement proof submission, invoice creation or payment execution.
6. **Validation:** disposable databases, signed encrypted fixtures, mocked relay
   responses and isolated WASM/mock-IPC browser tests; results recorded below.

## Semantics and safety

- VERIFIED means the available signed chain and required verification references
  passed existing protocol checks. It does **not** prove relays retained every
  historical event or that an unseen future status update does not exist.
- INDETERMINATE means dependencies are missing/unavailable; it never creates an
  ownership grant. A known unresolved extension blocks that purchase's current
  ownership projection without deleting its previously stored signed receipts.
- INVALID means supplied evidence proves a contradiction, invalid chain or
  unauthorized issuance. Bad signatures/foreign-buyer inputs are rejected before
  staging and cannot overwrite existing valid events using a claimed event ID.
- An outage with no new contradictory evidence does not delete or downgrade
  previously verified local receipts. Observed authorization cancellations or
  complete status transitions reevaluate eligibility using their relevant times.
- Pre-portability confirmations already verified issuer authority before storing
  receipts but did not archive all signed evidence. Their exact, unchanged local
  chains retain that prior verification during outages, with an explicit cached
  trust reason and incomplete-discovery warning. This exception cannot promote a
  new import or new status extension. New ADP confirmations now archive receipt
  and signed authorization through the shared pipeline for offline revalidation.
- Ciphertext is stored unchanged; no universal decryption key, private-key
  export, content decryption, raw-proof IPC response or decrypted-data logging is
  introduced. Public payment fields remain public under NIP-102's existing model.
- New signed events, assessment updates and cursor progress are idempotent.
  Interrupted evidence commits roll back; interruption before cursor update
  repeats discovery safely. Account/session epochs fence stale asynchronous work.
- The client does not mint a download token or publish imported receipts. A
  serving ADP server still independently verifies ownership and distribution
  authorization under existing token/NIP-98 paths.

## External limitations / future improvements

| Limitation | Current behavior / mitigation | Improvement condition |
|---|---|---|
| Relay retention, availability, pagination caps and backdated events | Best-effort multi-relay discovery, incremental overlap, resumable backfill, periodic rescan, explicit incomplete status and independent manual import. Neither successful query nor EOSE proves full history | Better relay retention/coverage and optional transport capabilities; no guaranteed historical recovery claim |
| Multi-character `#order` indexing | Installed nostr 0.44 Filter represents only single-letter tags. The source interface includes an optional indexed-order hook; the current adapter uses required `#p` plus local filtering and exact-ID dependency retrieval. Silent/failed optional order indexing falls back | An existing SDK/transport gains multi-character filtering; no custom Nostr client or incompatible wire format |
| NIP-59 wrapped/private receipts | Not discoverable through public `#p`; wrapped-event input is unsupported rather than mislabeled verified | Separately scoped private transport/inbox integration using the active signer, without universal keys |
| Missing public Zap/LSP evidence | INDETERMINATE until signed proof and authoritative listing LSP metadata are available. Metadata lookup never requests an invoice. Successfully verified public LSP evidence is cached per purchase | Evidence availability; no fabricated signer/LSP authority or decryption requirement |
| ADP server cannot see an imported receipt or is unavailable | Local ownership can be restored, but successful serving is not guaranteed. Existing server verification and distribution checks remain; no client publication or invented receipt-upload API | Available evidence on the serving side or an agreed protocol capability, outside this milestone |
| Relay omission of receipt/authorization updates | Available-chain verification and coverage status remain distinct; no claim that absence from a relay proves absence of cancellation/refund | Stronger interoperable completeness evidence if ever standardized |

These external/optional limits do not alone keep development open. Manual import
is an independent recovery mechanism for supported self-contained signed chains,
not a guarantee that missing signatures, authorization or public proof can be
manufactured from an incomplete export.

## Validation

Targeted and regression commands:

```sh
cargo test -j 1 -p arcadestr-core --features native --lib --test receipt_portability --test purchase_execution --test quote_verification --test receipt_listing_binding --test aggregate_ownership --test multi_operator_gate2 --test payment_durability --test startup_settlement_recovery --test settlement_repository --test nwc_capabilities
cargo test -j 1 -p arcadestr-app --features native --lib
cargo test -j 1 -p arcadestr-desktop
cargo check -j 1 -p arcadestr-desktop
python3 -B -m unittest discover -s tests -p 'test_gate3*.py'
# From web/:
trunk build --dist /tmp/opencode/receipt-frontend
# From repository root:
python3 -B scripts/regtest-receipt-portability-ui.py /tmp/opencode/receipt-frontend
git diff --check
```

Final passing runs: **569 core unit tests** (1 ignored), **69 integration tests**
(50 existing targets plus 19 portability tests), **514 app tests**, **306 desktop
package tests** (6 ignored), **35 Gate 3 Python guards**, normal native desktop
`cargo check`, real-WASM Trunk build, isolated browser regression and
`git diff --check`. Receipt-IPC privacy and NWC capability regressions passed.
After the final logging hardening, the targeted IPC privacy test and
`cargo check -p arcadestr-web --target wasm32-unknown-unknown` also passed.
The dependency traversal uses an indexed successor graph rather than repeatedly
revalidating the entire event set for every purchase.

Earlier focused runs identified an invalid
refund test fixture (a second root rather than a predecessor-linked status
update); the refund-awareness assertion is preserved using a valid signed chain.
No live cross-device recovery was performed. Mock tests are not live evidence.

Evidence: `/tmp/opencode/receipt-core-final-checked.log`,
`receipt-app-final-privacy.log`, `receipt-desktop-verified.log`,
`receipt-desktop-final-check.log`, `receipt-gate3-guards.log`,
`receipt-frontend-final-privacy.log`, `receipt-portability-browser-final.log`.
Browser artifacts: `/tmp/opencode/receipt-portability-ui.json`,
`receipt-portability-desktop.png`, `receipt-portability-mobile.png`.
Final logging checks: `receipt-ipc-final-privacy.log` and
`receipt-wasm-final-check.log` in the same artifact directory.
Workspace-wide formatting differences predate this work; unrelated files were
not rewritten to make a global formatting check appear clean. New modules were
rustfmt-formatted. No whole-workspace or live-service test pass is claimed.

The 19 portability tests cover fresh-device discovery, multi-relay duplicates,
offline single/chain imports, delegated authority and its historical cancellation,
public/aggregate proof bindings, bad signatures and foreign buyers, forked or
mutated orders, missing predecessors/authority, partial/outage recovery, old local
verification retention, explicit rescan, persisted incremental overlap, concurrent
imports and rollback immediately before commit. They also check History/Library
assessment consistency, ciphertext preservation and zero payment side effects.

Changed files:

- `core/src/receipt_portability.rs`, `receipt_sync_contract.rs`, `purchases.rs`,
  `ownership.rs`, `lib.rs`, `storage/db.rs`; additive migration 023 and
  `core/tests/receipt_portability.rs`.
- `desktop/src/receipt_commands.rs`, `main.rs`, `adp_commands.rs`.
- `app/src/models.rs`, `tauri_bridge.rs`, `tauri_invoke.rs`,
  `ui_v2/receipt_sync.rs`, module/root registration and purchase/library/detail
  ownership refresh hooks; `web/src/main.rs` debug-only isolated preview.
- `scripts/regtest-receipt-portability-ui.py`, this report and the retained
  external dependency backlog. Unrelated pre-existing working-tree changes are
  preserved, not formatted or reverted away.

Migrations 021/022 and protected reservation, original approval and pause-marker
SHA-256 values were checked byte-identical. No credential/permission operation,
live Lightning quote/invoice/payment, receipt publication, mining or commit was
performed. New receipt IPC suppresses payload logging and sanitizes transport /
parse failures rather than echoing imported material.
The generic IPC debug logger also logs command names only, not response bodies
that might contain invoices or preimages from other acquisition commands.

## Completion matrix

| # | Criterion | Result |
|---|---|---|
| 1 | Standard automatic discovery | PASS in isolated/mock tests; no guaranteed relay history claim |
| 2 | Manual receipt import | PASS in offline signed-event tests and isolated UI |
| 3 | Existing shared cryptographic pipeline | PASS; no independent verifier or protocol format |
| 4 | Correct chain-based ownership | PASS in chain, fork, authority and status-transition tests |
| 5 | History, Library and download eligibility | PASS in projection/regression tests; server access remains independent |
| 6 | Account isolation and privacy | PASS in core/UI epoch, ciphertext and receipt-IPC privacy tests |
| 7 | Idempotency and restart durability | PASS in duplicate, restart, concurrent import and commit-rollback tests |
| 8 | Relevant automated tests | PASS; targeted, regression, native/WASM and isolated UI results above |
| 9 | External limitations documented | PASS |
| 10 | No known critical Arcadestr defect remains | PASS within the implemented receipt portability scope; external completeness/serving limits remain disclosed |

## Next milestone

**Gate 5 — Download/Install Gap Audit:** audit eligibility, ADP server selection
and fallback, package integrity, installation, updates and recovery, starting from
existing implementations and tests. It has not been implemented as part of this
milestone. No live spending, protocol/server changes or commits are authorized.
