# Gate 3: restored NIP-46 session, dedicated NWC, full purchase recovery

> **Scope corrected; execution instructions below are historical only.**
> Active plan: [Arcadestr-owned Gate 3](regtest-gate3-arcadestr-scope.md).
> The custom Hub patch is abandoned, not deployed, and is not a dependency.
> The provider-specific budget wizard and live driver are retired and fail closed.
> App #4 remains read-only; the original reservation and all evidence are retained.

## Current status

| Phase | Result |
| --- | --- |
| 1 — Restored NIP-46 session | **Passed in the actual desktop.** Same persistent signer identity, app key and saved grants; desktop and signer restarts without re-pairing; signing and NIP-44 decryption; existing Bob account isolation; safe unavailable-signer timeouts and later recovery. **0 payment dispatches / 0 wallet recovery lookups.** |
| 2 — Dedicated NWC application | **Read-only GUI checks passed.** Dedicated app #4 is bound to the persistent NIP-46 buyer and existing Alice LND, not Alice's old NWC app. Desktop/keyring restart, account isolation and separate signer/NWC outages passed. Actual payment-intent and paid-reconciliation coverage remain pending Phase 3. |
| 3 — Complete purchase recovery | **Paused by user before payment.** The approved 250-sat cap and reservation are retained, but app #4 remains read-only. No quote, invoice, payment, receipt, ownership or token has been created. Explicit resumption and fresh safety checks are required. Previous CLI-funded principal is not NWC evidence. |

**Gate 3 remains open.** Do not mark phases 2 or 3 passed based on phase 1.

Private run:
`~/.local/state/arcadestr-regtest/gate3-integration-runs/1793419977795571323/`.
It contains protected account/keyring copies and must not be published.
The preceding [denial run](regtest-gate3-denial.md) and all older evidence remain
unchanged. This run reuses copies of that persistent test identity's actual
vault, saved app key, grants and desktop keyring; it does not create or re-pair
the primary identity. Existing Bob was reused only as the secondary identity.

## Confirmed restoration defect

The initial actual GUI authenticated-read attempt failed with an explicit
authorization-denied message and no pending ncli request. The signed local relay
trace showed `connect` and its correlated response:
`denied: no matching pairing in progress`.

The SDK's bootstrap always replays `connect`, including the saved, consumed
single-use pairing secret. The narrow regression reproduced the exact failure
before the fix. Restoring an already authorized app now uses its original key
and the existing NIP-46 request/response types without replaying pairing. It:

- Authenticates the remote signer and checks its returned user key against the
  saved identity before making the restored client available.
- Verifies response signatures, author, recipient and request ID; rejects wrong
  signer/recipient, stale IDs, invalid signatures and invalid event bindings.
- Preserves explicit refusal versus timeout classification. The signer still
  authorizes each operation under its existing saved grants.
- Coalesces connection initialization and does not retain state locks across
  network operations. Background status checks use actual remote verification,
  not merely the cached user public key.

No protocol specification, identity, pairing secret, or remembered permission
was changed to make restoration pass. Regression coverage includes the actual
consumed-secret pattern, adversarial response binding and refusal preservation.
Final validation passed: **16 signer tests**, **505 native app tests**, **12
durability/startup-recovery tests**, **275 desktop tests** (6 ignored), and **32
Gate 3 Python tests**. The normal desktop build and web WASM check also passed.

## Phase 1 actual GUI observations

- After desktop restart, **LOAD OBLIGATIONS — NO PAYMENT** succeeded using saved
  authorization: `No obligations are owed by this account at this processor.`
- The existing debug-only decryption check returned
  `NIP-44 decryption verified. No payment or storage changes.`
- With the owned signer stopped, another desktop restart retained the identity
  without re-pairing. Signing and decryption timed out safely, rather than
  replaying a payment or treating the failure as a successful operation.
- Restarting the same signer with its existing encrypted vault restored both
  operations without new approval requests or pairing.
- Switching through the actual GUI to existing Bob rejected the remote-only
  probe (`Remote signer required for the regtest probe`) and emitted no ncli
  request. Switching back via the saved **Reconnect** control restored the
  original remote app key and both operations. Saved primary remote-profile
  metadata remained byte-identical to the original denial run.
- All isolated purchase/payment table counts remained zero.

`phase1-verification.json` and the `phase1-*.json/png` artifacts retain the
observations. Readiness waits that timed out before the GUI completed are
retained as incomplete harness attempts, not counted as failures of authorization
or as successful operation evidence.

## Readiness and manual NWC handoff

A read-only preflight found all nodes on regtest with active routing liquidity
but stale-tip synchronization at height 177. All pending HTLC/channel counts and
the mempool were zero. The user explicitly authorized **one** readiness block.
After fresh safety checks, height advanced **177→178**, reusing the existing
coinbase destination; all four nodes became synced. No payment was made or
authorized. The initial block-hash parser failure happened before admission or
mining; it did not cause an extra block. Evidence:
`preflight-readiness.json` and `readiness-one-block-result.json`.

The existing Alby Hub is desktop-only: port 21420 is an instance lock, not an
administration API. Accessibility automation was unavailable. Do not bypass its
UI by inserting app/permission rows into its database. The user approved a short
manual handoff to create a separate **read-only** application:

```sh
bash scripts/regtest-gate3-nwc-wizard.sh \
  "$HOME/.local/state/arcadestr-regtest/gate3-integration-runs/1793419977795571323"
```

The wizard requests only `get_info`, `get_balance`, `lookup_invoice` and
`list_transactions`, with no pay, invoice-creation or superuser scope. It hides
the URI, validates its derived app public key against the Hub's new application
and wallet binding, rejects use of Alice's existing app, and saves it to a
private mode-600 file. Existing Hub identity/permission/transaction-row digests
are checked before accepting it. Do not paste the URI into chat.

The user completed that wizard. The connection was saved and verified through
the actual remote buyer's NWC panel. Read-only signed NWC traffic verified
`network=regtest` and the existing Alice Lightning node's identity. The saved URI,
app key, wallet identity and buyer binding remained identical across an actual
desktop plus private Secret Service/keyring restart. Existing Bob's GUI showed
no wallet; a native read addressed to the primary buyer while Bob was active was
rejected, with no legacy fallback. Switching back restored and verified the same
dedicated connection.

A process-local debug transport proxy at loopback port 43208 tested NWC outage
without changing the stored URI or shared Hub/relay. Offline verification failed
with the binding retained; reconnecting passed. Stopping the owned NIP-46 signer
blocked decryption while NWC read-only verification remained available. Restarting
that same signer recovered decryption. Empty-queue reconciliation performed no
payment or wallet recovery lookup. These are **not yet paid-intent recovery**
results; see `phase2-readonly-verification.json` and the binding/fault GUI artifacts.

## Phase 3 preparation and explicit approval

The new live-relay fixture is under `phase3-new-purchase/`. It uses distinct
listing/operator fixture keys and the normal wall clock. Its declared platform
is `linux-x86_64`, its price is 200 sats and the committed recipients are existing
Bob and Carol, 100 sats each, funded through the dedicated buyer app on existing
Alice LND. Neither recipient is the payer. Direct zero-fee routes were verified.
No earlier partial purchase, invoice, journal or proof was reused.

Actual GUI preparation exposed two confirmed integration defects before any
spending: the native app listing mapper dropped accepted commercial-terms
references, and the split checkout incorrectly required a legacy profile payment
address. Focused regressions failed before the fixes. The mapper now preserves
the same references/diagnostic flags as the existing core mapper; split checkout
uses its signed quote path rather than the legacy address gate. Actual GUI
retesting now offers **Buy with Lightning**, without clicking it. No destination
validation was bypassed and no signed listing bytes were changed.

After roughly 10.7 hours, the regtest tip became stale again. A fresh preflight
stopped before quote/payment creation. With separate explicit user authorization
and fresh zero-pending/mempool checks, exactly one further readiness block was
mined, **178→179**; all four nodes synchronized. No additional block was inferred
from the earlier approval. The signer, dedicated NWC, routes and old Hub state
were reverified before the payment-budget question.

The user explicitly answered **Approve 250-sat cap**: 200 sats principal, up to
50 sats fee reserve, at most two 100-sat payment dispatches for this one new
fixture. `budget-approval.json` and a separate retained 250,000-msat reservation
are recorded in the private fixture directory. Spending remains blocked until
the Hub permission is validated:

**Subsequent user decision: Pause before payment.** The commands below are
historical preparation, not authorization to resume. A private
`paused-by-user.json` marker now blocks both the permission validator/wizard and
purchase controller. App #4 was confirmed still read-only with zero transactions;
the private desktop database has zero quotes, invoice attempts, payment intents,
purchases or tokens. The 250-sat reservation and approval remain retained.
Owned private test services were stopped; original Hub and signer services were
left untouched. Protected original-file fingerprints remained unchanged.
The exact GUI-tested executable was preserved privately, and the normal desktop
build was restored. Resume only after explicit user instruction and fresh checks.

```sh
bash scripts/regtest-gate3-budget-wizard.sh \
  "$HOME/.local/state/arcadestr-regtest/gate3-integration-runs/1793419977795571323"
```

This edits only dedicated app #4, retaining its URI and read permissions, adding
sending with an exact **250-sat non-renewing cap**. It does not send a payment.
The validator checks the existing protected URI/app key, explicit approval,
permission expiry, exact cap, no extra write/superuser scopes, and unchanged
original Hub records. Do not modify Alice's original app #3.

The payment controller requires that validated permission, fresh regtest/signer/
NWC checks, exact invoice/buyer/app binding, complete wallet history, immutable
one-shot dispatch admissions and a retained reservation. It stops on inconclusive
verification and contains no redispatch authorization. The planned interruption
is reply loss after the genuine second payment, followed by actual desktop
restart and read-only recovery. This plan has not yet been reported as passed.

No changes are committed. Never clear an unresolved reservation, replace an
unresolved invoice, or automatically repay an uncertain result.
