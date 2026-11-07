# Failed/partial settlements and refunds

```sh
./scripts/regtest test --refunds
./scripts/regtest test --verify-refunds # recheck the existing six cases, no payments
```

Explicit opt-in real-Lightning tests using the existing Alice buyer/NWC, Bob
developer and Carol operator. No identities, wallets or channels are recreated.
The successful 1,000-sat purchase and durability orders remain unchanged.

Each case owns a new listing, loopback HTTP server, ADP database and client
payment journal under `~/.local/state/arcadestr-regtest/refund-runs/`. The main
9099 database is not reset or used for these purchases. Each listing costs
200 sats: Bob 100, Carol 100. The existing ascending `(amount_msat, role,
recipient_pubkey)` order puts Bob first for these equal amounts.

## Verified cases

| Case | Injection and assertions |
| --- | --- |
| Bob paid, Carol fails | Cancel Carol's invoice before payment; Bob has a verified 100-sat proof, Carol receives nothing; fail at the confirmation deadline, then refund Bob's principal. |
| Bob fails first | Cancel Bob's invoice; Carol's preflight invoice remains undisclosed and unpaid; fail at the payment deadline with no refund obligation. |
| Refund succeeds | Bob pays the processor-prepared Alice invoice for exactly 100 sats; authenticated confirmation persists the verified refund proof. |
| Refund fails and is retried | Inject wallet unavailability **before intent/dispatch**; restore it, reuse the same persisted invoice, then send one real refund. This is not an uncertain/failed HTLC retry. |
| Process crashes during refund | Bob's LND payment settles, then the child exits 86 before local proof persistence. Check Bob's exact hash/account intent and pending ADP attempt; a fresh child reconciles the original outgoing payment and confirms it without paying again. |
| Repeated recovery | Ten unchanged recovery/preparation requests, cached payment retries and repeated valid confirmations retain one recovery event, one refund invoice and one refund dispatch. |

Every case checks initial/partial/final balances, failed quote state and original
deadlines, proof persistence, absence of kind-1020 purchase receipts, tokens and
ownership, and rejected downloads. All final channel balances equal that case's
initial snapshot. Five partial purchases send 100 sats to Bob and return 100 sats
to Alice; the zero-paid case sends nothing. Observed routing fees were zero.
The crash case's audit recorded one dispatch and one reconciliation lookup;
the second resume used the checkpointed proof.

## Safety and evidence

- Reserve **1,980 sats** before dispatch: conservative allowance for all six
  cases' purchase attempts, refunds and fees. The existing shared 50,000-sat
  automated-run cap and 10,000-sat purchase cap remain enforced.
- Require regtest, synced existing LND nodes, liquidity, exact invoice amount/payee,
  and the established buyer NWC app on local relay 7778 backed by Alice LND 10001.
- `refund-reservation.json` blocks a new suite after an incomplete/ambiguous run.
  Inspect its private databases and Lightning history; never clear it just to
  bypass the guard or blindly rerun an unresolved payment.
- `last-refunds.json` contains the latest sanitized report; each run retains its
  `report.json`, per-case `result.json`, dispatch audits and private databases.
  Private refund jobs contain invoices, not signing credentials; keep the whole
  run directory private. Do not publish databases or driver logs.

## Production changes and limits

Refund preparation returns the original persisted invoice, including an expired
or ambiguous attempt, rather than creating a second payable invoice. Concurrent
creation is atomic per obligation. Verified obligations cannot prepare another
invoice. Repeated valid confirmation is idempotent; authentication, preimage/hash
checks and proof-replay protections still apply.

Recovery claims are canonical per quote/role/evidence commitment. Unchanged
requests return the same signed event; changed evidence after late proof may
produce a new immutable event. Migrations retain historical event IDs and signed
JSON; exact legacy replays adopt the evidence commitment without rewriting them.

The runner uses production authenticated ADP HTTP, LNURL, validators, storage and
durable reconciliation, real NWC for buyer payments and real Bob LND payments
for refunds. A **controlled Relay adapter and test-owned injected clock** make
deadlines deterministic. This does **not** verify live-relay timeout/freshness
behavior. The normal router still uses wall time; no HTTP/environment clock
override was added. Nostr fallback claims and disputed refunds are not covered.

Unknown wallet outcomes remain blocked; there is no automatic repayment or
replacement invoice. Automatic startup recovery and GUI refund integration
remain deferred. Nothing is committed automatically.

The separate [Gate 1 runbook](regtest-gate1.md) extends this coverage to live
strfry/NAK relays, late proofs and ambiguous real refunds without replacing these
six baseline scenarios.
