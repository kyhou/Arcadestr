# Settlement verification — Gate 1

## Commands

```sh
./scripts/regtest test --gate1          # opt-in isolated real payments
./scripts/regtest test --verify-gate1   # existing signatures/chains/payment proofs, no payments
./scripts/regtest test --verify-paid
./scripts/regtest test --verify-durability
./scripts/regtest test --verify-refunds # all six existing refund baselines, read-only
```

The driver uses the production `RelayClient`, authenticated ADP HTTP routes,
LNURL validators, storage and durable payment module. Local strfry ports
**7777/7778** and NAK ports **10547/10548** serve actual WebSocket requests.
There is no controlled Relay adapter in the Gate 1 server path.

## Deadline methodology

**Deadline advancement is simulated, not elapsed wall-clock time.** Test-owned
routers inject their settlement clock; production `routes::router` still uses
wall time. Backdated, developer-signed authorization roots allow isolated
historical quote timestamps. Real BOLT-11 creation/expiry, HTLC execution,
NIP-98 authentication and relay admission continue to use real time.

The confirmation fixture leaves 750 seconds after wall time for its simulated
24-hour deadline. Its processor event remains within existing relay future-time
admission limits. No relay configuration, host clock, production deadline or
HTTP/environment time override is changed.

## Mandatory coverage

| Area | Verified behavior |
| --- | --- |
| Publication/retrieval | Signed processor and buyer kind-1034 events return byte-identically from all four live relays. |
| Missing events | Production lookup confirms absence only after completed responses from at least two relays; per-relay probes require the matching EOSE. |
| Delayed/split propagation | Publish a fallback only on strfry 7777; it is absent on other relays. After 250 ms publish on NAK 10547; after another 250 ms publish on the remaining relays. Union lookup finds the event without waiting for every relay. This is explicitly staged forwarding, not automatic relay replication or a propagation SLA. |
| 30-minute disclosure | Active at deadline−1; at the exact deadline disclosure is blocked, failure is fixed to that deadline, and no unproven payment creates an obligation. |
| Late purchase proof | Bob's real payment proof arrives after disclosure failure. The failed quote stays failed, a principal obligation appears at the original failure+604800, and changed evidence creates a new signed event without deleting the old one. Repeating the new evidence retains its ID. |
| 24-hour confirmation | Both invoices are disclosed; only Bob pays. Active at confirmation−1, failed exactly at confirmation; Carol remains unpaid. |
| One-hour claim grace | Processor claim remains available at failure+3600; early buyer fallback is rejected. At +3601 fallback is offered and authenticated buyer evidence is admitted idempotently. |
| Seven-day obligations | Owed at due time, overdue at due+1; original due time survives late proof, preparation, restart and reconciliation. |
| Adversarial recovery | Duplicate valid claims reuse the canonical ID. Signed malformed/conflicting payloads may exist on relays but cannot enter authoritative ADP storage. Invalid signatures and ciphertext/plaintext mismatch are rejected. |
| Signed chains/persistence | Read-only recheck verifies listing/quote IDs, fulfillment authorization root/role/capability resolution, encrypted recovery evidence commitments and quote anchors, retained historical claims on every relay, and refund proofs against exact Alice invoice hashes. |

All failed orders retain zero ownership, purchase receipts and download tokens.
Both paid cases restore their starting channel balances; routing fees were zero.

## Ambiguous real refunds

Each child first commits the production payment intent. Its wallet adapter
spawns Bob's real LND payment and exits **87 immediately**, without waiting for
or reading the wallet result. The wallet process writes only to a private file.

1. **Lost settled reply:** Alice's normal refund invoice settles, but the client
   has no proof checkpoint. Six fresh-process attempts make real read-only wallet
   queries whose replies are masked as NOT_FOUND, timeout or unsupported (twice
   each). Every attempt blocks, preserves the intent and reuses the exact refund
   invoice. These fault replies are injected; they are not claimed as native LND
   statuses. One later real lookup recovers and checkpoints the proof; the next
   restart reads the cached proof without lookup/dispatch.
2. **Genuinely unresolved HTLC:** a test-owned LNURL endpoint returns an exact
   100-sat Alice LND hold invoice. Observe Alice `ACCEPTED` and Bob `IN_FLIGHT`.
   Two fresh-process reconciliations return unresolved and never repay. Reveal
   the original preimage to settle that same invoice; a third lookup recovers
   the proof. Another restart uses the cache. This is a real pending HTLC, not a
   mocked wallet status.

### Dispatch audit

| Case | Buyer purchase dispatches | Refund dispatches | Refund wallet-adapter lookups |
| --- | ---: | ---: | ---: |
| Late proof / lost settled reply | 1 | 1 | 7 |
| Confirmation / real pending HTLC | 1 | 1 | 3 |
| **Total** | **2** | **2** | **10** |

The ten counts cover recovery-adapter invocations, each of which executes a real
read-only LND query. Additional LND balance/invoice/history inspection and NWC
readiness checks are not included in those recovery counters. Cached retries
cause zero additional payment or recovery-lookup calls. Each refund has exactly
one matching successful outgoing LND record and a settled incoming Alice invoice.

## Evidence and reservations

Latest report: `~/.local/state/arcadestr-regtest/last-gate1.json`.
Verified run: `gate1-runs/1793321292326579655/` under that state directory.
Keep it private. It retains signed quotes, per-case progress/results, ADP/client
databases, dispatch/lookup counters, child checkpoints and the hold-invoice
preimage needed for safe recovery. **Never publish private wallet output or
hold-secret/job files.** `verification.json` records the no-payment recheck.

Reserve **990 sats** conservatively before execution; maintain the existing
10,000-sat purchase and 50,000-sat cumulative automated-run limits. Each listing
is 200 sats (Bob/Carol 100 each); only Bob's 100-sat leg is dispatched in each
paid case. Existing identities, NWC app, wallets, channels and baseline databases
are reused without clearing or recreation.

The initial run paused after completing the first two stages because the new
hold LNURL test fixture omitted required `nostrPubkey` metadata. Bob's second
purchase had settled, but **no refund attempt/dispatch existed**. The fixture was
fixed and the inspected original reservation/order continued with
`test --resume-gate1`; no purchase was recreated or replayed. The second case's
initial balance reference was reconstructed from the preceding completed case's
final snapshot and checked against the exact outstanding 100-sat principal delta.
The original log, reservation and all evidence remain intact.

`--resume-gate1` is deliberately narrow: it restores that inspected, failed,
overdue order only before any refund dispatch job exists. It refuses to replace
an existing refund attempt/job. It is not a general startup recovery feature.
Never delete an unresolved reservation to force another run.

## Confirmed production defects fixed

- Buyer fallback admission previously verified the separately supplied evidence
  but did not decrypt/bind it to the signed event content. It now requires valid
  operator-addressed NIP-44 ciphertext matching that evidence before persistence.
- Direct fallback submission previously depended on a preceding status/prepare
  request to materialize expired deadlines. It now processes deterministic
  deadlines itself, without extending them.

Both defects have red-to-green authenticated route regressions in
`adp-server/adp-server/tests/recovery_admission_tests.rs`. No protocol semantics
were changed. Automatic startup recovery, GUI handling and Dave's server remain
out of scope. This gate does not claim public-relay behavior, natural multi-day
wall-clock waits, recipient cooperation guarantees or global exactly-once payment.
