# Payment durability and crash recovery

## Scope and commands

```zsh
./scripts/regtest up
./scripts/regtest test --crash-recovery  # isolated real 1,000-sat crash/restart purchase
./scripts/regtest test --verify-durability # recheck that recovered order, no payment
./scripts/regtest test --source          # regression suites; no payments
./scripts/regtest test --verify-paid     # recheck the original successful baseline; no payments
```

`--crash-recovery` is separate from the original `--payments` scenario and from
`--wallet-smoke`. It uses a different listing coordinate and separate manifests,
reservation and evidence files. The original baseline order/receipt is retained.
Regtest node/destination checks, 10,000-sat purchase cap and shared 50,000-sat run
cap remain in force. No wallet, channel, primary identity or NWC connection is
created. No refund, failed-settlement disposition or second operator is involved.

## Live result

The real crash/restart purchase completed:

- Order: `regtest-e2e-6f6f63eb-29c3-4d12-9471-934998d7dd3a`.
- Listing d-tag: `arcadestr-regtest-e2e-crash-durability-v1`.
- The driver process exited with code **86 after the real NWC wallet settled
  Carol's invoice, before returning the response to the durability module**.
- The parent observed LND's settled invoice, a committed local payment intent
  **without** an encrypted payment result, and **no** ADP proof/settled timestamp.
- A fresh process loaded the original invoice and quote, used read-only NWC
  `lookup_invoice`, checked the preimage/hash/amount, checkpointed the encrypted
  result, and submitted the original payment proof. It did not request a
  replacement invoice or send `pay_invoice` again for that leg.
- Bob's remaining leg then completed normally.
- Wallet adapter audit: **one pay dispatch per invoice**, one lookup for Carol's
  original invoice. The audit records hashes/counters only, never preimages.
- Exact balance deltas: Alice **−1,000 sats**, Bob **+900**, Carol **+100**, Dave
  unchanged; routing fees **zero**.
- Valid aggregate kind-1020 receipt:
  `a1d3c5174fc7e45063389dfafd6467f84d84b856d8dc548845f641dc13920e47`.
- Receipt retrieval, client ownership, server/client settled state, two consumed
  payment proofs, buyer-bound token and the hash-verified 30-byte download passed.

Sanitized result: `~/.local/state/arcadestr-regtest/last-payment-durability.json`.
The original result remains in `last-split-purchase.json`. Detailed per-run
records and dispatch audits are in `split-runs/`; manifests/reservations use the
`durability-` prefix. Keep these directories private.

Before the live crash, the existing connection also recovered and hash-verified
both original baseline proofs through read-only NWC lookup. Those proofs were
never printed, and those lookups sent no payment.

## Production module and invariants

`core/src/durable_payments.rs` supplies `DurableNwcPayments` with a small interface:

- `pay_or_resume`: may dispatch only after atomically inserting a new intent.
- `reconcile`: cannot insert an intent or dispatch a payment.

The wallet seam has a real NWC adapter and test/fault-injection adapters. The real
transport and protocol implementation remain `NwcClient`; the regtest harness
does not reimplement settlement or payment reconciliation.

1. **Before dispatch:** commit a unique `(account_pubkey, payment_hash)` intent,
   bound to the wallet and NWC application public keys, exact invoice commitment
   and amount. SQLite connections use `synchronous=FULL`. No transaction/lock is
   held over a wallet request.
2. **After payment:** verify the returned preimage hashes to the invoice hash;
   encrypt the result to the initiating account with NIP-44; commit it before
   returning success to the caller.
3. **After an interrupted response:** an existing intent can only return a cached
   validated proof or perform lookup. `NOT_FOUND`, pending/failed status,
   unsupported lookup, disconnects and timeouts never authorize another pay.
4. **Wallet/account changes:** do not reconcile against a different wallet/app or
   expose an initiating account's result to an account that became active later.
5. **Expired invoices:** an expired invoice cannot start a new payment. An already
   recorded payment intent can still recover a paid proof after expiry.
6. **Before ADP proof submission:** checkpoint the validated, account-encrypted
   preimage on the exact locally stored quote/attempt. A lost HTTP acknowledgement
   can retry that same proof; it does not require another payment.
7. **Before advancing:** `next_split_invoice` refuses a replacement/next invoice
   while an earlier payment intent or proof checkpoint remains unacknowledged.

Migration `018_payment_durability.sql` is additive: it creates the payment journal
and adds a nullable encrypted proof column. It does not rewrite historical
migrations, reset databases, replace accounts or change protocol semantics.

The production `pay_nwc_invoice` command uses this module. The authenticated
`resume_split_payment` command takes `{server_url, quote_id, attempt_id}` inside
its normal `request` argument, retrieves the original account-scoped invoice or
proof, reconciles only if necessary, and submits the original proof through the
existing ADP command. It does **not** call a payment method. Its server must match
the stored quote. GUI/startup wiring for this resume command remains deferred.

This is an **at-most-one client dispatch** guarantee for a recorded invoice, not
a new global exactly-once protocol guarantee. It relies on preserving the client
journal and the existing wallet/app credentials. A deliberate new quote is a
distinct purchase; callers must resume a pending order rather than silently
creating a new purchase identity after interruption.

## Regression coverage

`core/tests/payment_durability.rs` exercises:

- Lost successful wallet response, database reopen, lookup recovery, no second pay.
- Concurrent callers competing for the single durable dispatch claim.
- Cached proof after restart while the wallet is offline.
- Unknown outcomes, wallet changes, account scoping and reconciliation without an intent.
- Invalid proofs never checkpointed as paid and never causing a redispatch.
- Expired new invoice rejection and recovery of an expired existing paid intent.
- Strict outgoing lookup type/hash/amount/response-state validation and redacted
  payment-result Debug output.

`core/tests/settlement_repository.rs` additionally checks encrypted proof
checkpoint persistence, account/quote isolation and invoice-advancement gating.
Existing NWC signature/response-correlation and migration identity tests are
retained. The live adapter uses the same production module as the desktop.

Verification passed: seven durability tests; three settlement-repository tests;
14 migration/storage tests (including a non-destructive version-17 upgrade);
12 NWC tests; 20 supervisor safety tests; the broader source suites; and the
desktop compilation check. Verification-only mode has a regression ensuring it
cannot resume a partially paid order by dispatching the remaining payment.

## Intentional limits

If a process exits after committing the intent but before the wallet receives a
request, lookup may return NOT_FOUND. The outcome remains conservatively blocked;
there is no automatic resubmission, expiring lease or fallback payment. A wallet
that cannot return a settled preimage also remains blocked. These cases require
explicit investigation, not a guessed retry.

The real process-exit test covers the hardest settlement-before-checkpoint window.
The after-checkpoint/offline-wallet window is covered by database-reopen tests;
it has not yet received a separate live process-kill run. Automatic GUI startup
reconciliation, failed/partial-settlement policy, refunds and multi-operator
scenarios remain future work.
Rebuild/restart the desktop from current local sources to activate the new
backend behavior and resume command; GUI controls are not added in this phase.

Recovery of a paid proof after invoice expiry does not extend ADP's payment or
confirmation deadlines. A quote that has actually failed still follows the
existing failure/refund-obligation rules; completing those workflows is the next
phase, not something this test bypasses.
