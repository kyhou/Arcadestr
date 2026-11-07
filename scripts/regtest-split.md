# One genuine Alice → Bob/Carol ADP purchase

## Reproduce

```zsh
./scripts/regtest up
./scripts/regtest test --payments      # one new 1,000-sat ADP purchase via existing NWC
./scripts/regtest test --verify-paid   # assertions on the paid order, no new payment
./scripts/regtest test --source        # isolated regression suites, no payments
```

Basic wallet transfers are separate: `test --wallet-smoke`. They are not ADP
purchases and are not run by `--payments`. If both modes are explicitly selected,
they share the 50,000-sat invocation budget. Maximum purchase remains 10,000 sats;
the implemented scenario is fixed at 1,000 sats, with a 100-sat operator fee.

Use `up`, not `up --no-build`, after changing application sources. Only current
local Arcadestr and sibling ADP sources are built. No protocol specifications,
production credential files, existing wallets/channels, primary identities, or
other listings are changed.

## Actual execution evidence

The first paid order is `regtest-e2e-ea743129-7700-473d-909b-7ac5ce166e29`.

| Verification | Evidence |
|---|---|
| Four fixture identities | Existing Alice/Bob accounts, existing Carol operator and user-supplied Dave public key; other private fixture settings preserved |
| Carol's commercial terms | Valid signed kind 1032, `353ac15e1061079807a4cc6ef47cfe2f87a0e6acd1851850256fbde7b09b2c47`; published to both strfry relays; fixed 100,000 msat fee to Carol's local LNURL |
| Bob's isolated listing | Signed kind 30402, `6816f575ae469373cbf3770845924c835db8fbbf4d4727385086fdf394861408`; d-tag `arcadestr-regtest-e2e-single-operator-v1`; Bob's LNURL; exact Carol terms root |
| Quote | Signature verified, NIP-44 decrypted with existing buyer signer; quote event `1e50fffac1bf9794e46fa5c89a14e2793e7098bf697f41b2fe266b73b5594215`; buyer/listing/processor/fee bindings checked |
| Real NWC payments | Alice's existing account-scoped NWC to the existing Hub app; Hub backend verified as local Alice; Carol received 100 sats and Bob 900 sats; both invoices settled and both Alice LND payment records succeeded |
| Proofs | SHA-256 of each NWC preimage matched the exact invoice hash; ADP accepted both proofs; aggregate hashes match the paid invoice evidence |
| Aggregate receipt | Signed kind 1020, `9dc523a51bf641bedc7e231c19904bcc4be9f41b4b84679d0af73b001cc6c92d`; validated exact quote, listing, processor, roles, amounts, proofs, accepted terms and delegated authorization; retrieved from local relays |
| Ownership | Verified receipt persisted in the existing Arcadestr app DB; production `PurchasesRepository::is_owned` returns true |
| Token/download | Unexpired buyer-bound token persisted; actual `AdpClient::download` succeeded, 30 bytes; SHA-256 `e2d81fcd5583d575f9113dd1615fe1554fa4822aa8227c37f849905fc8d3f8c4` |
| ADP DB | Quote settled, two legs settled, one matching purchase receipt, two consumed proof keys; no database reset |
| Channel balances | Alice 588,590 → 587,590 sats; Bob 197,530 → 198,430; Carol 196,530 → 196,630; Dave unchanged at 196,530. Routing fees zero |

The downloaded bytes are the repository's existing inert `adp-server/testgame.bin`
fixture, uploaded through the production API—not a manually seeded server DB or
a claim that a playable game was installed.

Sanitized machine-readable evidence is under
`~/.local/state/arcadestr-regtest/last-split-purchase.json` and `split-runs/`.
No token, raw preimage, private key, macaroon or NWC URI is included in reports.
Optional private proof checkpoints are encrypted to the existing buyer account.

## What the harness reuses

`core/examples/regtest_split_purchase.rs` uses production `AccountManager`,
`ActiveSigner`, `NwcClient`, `AdpClient`, terms/listing/authorization builders,
quote/receipt/authorization validators, `SettlementRepository`,
`PurchasesRepository`, and `DownloadTokensRepository`. Test assertions, not a
second settlement implementation, check LND invoices/payments and read-only DB
state. All four LND nodes and Bitcoin Core are verified as regtest before paying;
every invoice is signature-decoded and bound to Bob's or Carol's local LND key.

The normal ADP provisioning API creates/reuses the listing-scoped fulfillment
key. Bob authorizes that key through the normal signed authorization event; no
primary identity is replaced and no secret is inserted directly into ADP's DB.

The fixed isolated listing is updated on a new run, and commercial terms are
published before use. The supervisor writes a **public, private-directory**
settlement configuration overlay under its state directory, then gracefully
restarts only its owned ADP process. Existing `.env`, master key, DB paths and
other configuration remain unchanged. NAK relay histories remain intentionally
incomplete; the driver does not copy complete histories into them.

## Confirmed blocker fixed

The first purchase paid successfully and obtained a valid aggregate receipt, but
Arcadestr reported no durable ownership. Both receipt-chain root selectors
required the old singular `payment_hash`/`amount_msat`/`proof` fields, which a
valid aggregate receipt intentionally does not carry. They now share a predicate
that also accepts a parsed/validated aggregate paid root. Account scoping,
authorization, signature/commitment validation and fork rejection remain in force.

`core/tests/aggregate_ownership.rs` reproduces the original failure and covers
persisted ownership, chain selection, idempotent persistence, buyer scoping,
invalid total rejection and ambiguous-root rejection.

After that fix, only the existing paid order's assertions were retried. There was
no second payment: `verify-paid` requires an already-settled order and can only
resubmit an existing valid proof to retrieve its receipt/token. It cannot request
a settlement invoice or call NWC payment. The surrounding test command still
runs non-paying LNURL invoice probes. This is not a protocol recovery/refund workflow.

The helper never automatically retries an ambiguous payment. Inspect a retained
reservation/checkpoint before retrying a failed command. Recovery, refunds,
second-operator purchases and GUI flow testing remain intentionally out of scope.

## Stale regtest tip

A later verification found all four LND nodes at Core's height 171 but reporting
unsynced because the newest block was about 6.5 hours old. The safety gate refused
to proceed. With explicit user approval, exactly one regtest block was mined to
an existing backend wallet-owned mining address (171 → 172); all four nodes then
reported synced. No new Lightning payment, wallet or channel was created. The
supervisor still does not mine automatically. After long idle periods, refresh
the existing regtest chain through Polar before retrying a blocked readiness check.
