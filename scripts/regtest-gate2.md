# Settlement verification — Gate 2

## Commands

```sh
./scripts/regtest up
./scripts/regtest test --gate2        # explicit new purchase + isolated third-leg failure
./scripts/regtest test --verify-gate2 # existing orders/proofs/download; no new payment
```

Use all prior non-paying regression modes as well:
`--verify-paid`, `--verify-durability`, `--verify-refunds`, `--verify-gate1`.
Do not use `--payments`, `--crash-recovery`, `--refunds` or `--gate1` to reverify
old evidence: those modes can deliberately create new paid scenarios.

Gate 2 passed using current local Arcadestr/ADP sources, production authenticated
HTTP, actual local strfry/NAK relays, existing LNURL/Polar infrastructure and
Alice's established NWC. No GUI changes or protocol semantics were added.

## Independent operators and protected signers

| Role | Persistent storage / runtime |
| --- | --- |
| Alice buyer | Existing encrypted account in `~/.local/share/arcadestr/accounts.db`; protected existing `.master_key`; unchanged NWC app/Secret Service credential. |
| Bob developer | Existing encrypted account in the same account store; no plaintext export or replacement identity. |
| Carol operator A | `~/.config/arcadestr-regtest/carol-adp.env`, mode 0600; selected by `settings.json`; existing database/files and identity, HTTP 9099. |
| Dave operator B | `~/.config/arcadestr-regtest/dave-operator.key` and `dave-adp.env`, both 0600; independent encryption master key, database/files under `~/.local/state/arcadestr-regtest/operator-dave/`, HTTP 19100. |

`signer-locations.json` in that config directory records these persistent
references without containing signer secrets. The config directory is 0700;
Alice/Bob's account database and existing master-key file are owner-only. Carol's
original repository `.env` was retained, restricted to 0600 and not rewritten;
the active supervisor now loads the external protected copy. Back up the protected
stores through an appropriate secure mechanism; do not publish them as evidence.

With explicit authorization, Dave's **Nostr** identity changed from
`3c124122e49c39cc031f2267b88250cf78f9ce2955cc1c3440ffe4b758a24f72` to
`e19b33b9a31fcf73be0e915d17172c726510271a2fbe115040e1bf637895b0b0`.
The old public identity and original LNURL fixture config are retained under
`~/.config/arcadestr-regtest/identity-history/1793323450347612272/`.
No historical signed events, listings or paid orders were rewritten. Dave's
existing Polar LND identity, wallet and channels were not replaced or reconfigured.

`regtest up` loads Dave's independent config and starts his own ADP process. It
reuses the existing protected identity and immutable terms, never generates a
replacement key at startup. The persisted signer reference takes precedence over
obsolete transient `LSP_SEC` values. Missing, mismatched or insecure credentials
fail closed. Startup and a graceful Dave process restart were tested; an actual
machine reboot was not performed. Existing account/keyring unlocking requirements
remain unchanged.

## Successful real purchase

Order: `regtest-gate2-f4b3a25e-a984-4b09-8442-e66e06441bff`.

| Canonical leg | Amount | Receiving node |
| --- | ---: | --- |
| Carol operator A | 100 sats | Existing Carol LND |
| Dave operator B | 150 sats | Existing Dave LND |
| Bob developer remainder | 750 sats | Existing Bob LND |
| **Alice total** | **1,000 sats** | Existing Alice NWC/Hub/LND |

Both operator fees are fixed. Leg order remains ascending
`(amount_msat, role, recipient_pubkey)`. Operator URLs do **not** create additional
fees: multiple URLs belonging to one public identity deduplicate to one accepted
root/fee; duplicate or conflicting accepted roots are rejected.

| Node | Before (sats) | After (sats) | Delta |
| --- | ---: | ---: | ---: |
| Alice | 586,590 | 585,590 | −1,000 |
| Bob | 199,330 | 200,080 | +750 |
| Carol | 196,730 | 196,830 | +100 |
| Dave | 196,530 | 196,680 | +150 |

Routing fees were zero. Exactly **three successful NWC dispatches**, one per
distinct invoice/hash, matched Alice's outgoing and each recipient's settled LND
records. No payment lookup was needed for the planned restart: after Carol's real
payment the driver exited 86 with an encrypted proof checkpoint; a fresh process
used that cache, submitted the original proof and paid only the remaining legs.
Repeated proof submission reused the same aggregate receipt.

Verified:

- Independent signed kind-30403 announcements and matching live HTTP identities,
  distinct ADP credentials/encryption keys/databases, and both signed kind-1032
  terms accepted by Bob's isolated listing.
- Signed/encrypted quote, exactly three legs, exact fee/remainder sum, intended
  local LND payees and valid BOLT-11/preimage proofs.
- Aggregate kind-1020 receipt
  `1c41301e7fdc826bb59f77fd8793150b28c61430846afe6c83076cd493f5c9b9`,
  with quote/listing/processor, both accepted terms, all proofs and the delegated
  fulfillment authorization validated; receipt retrieved from live relays.
- Buyer ownership and settled receipt persisted in the **isolated Gate 2 client
  database** through production repositories, plus one matching server purchase,
  three consumed proofs and a valid buyer-bound download token.
- Actual 30-byte download, SHA-256
  `e2d81fcd5583d575f9113dd1615fe1554fa4822aa8227c37f849905fc8d3f8c4`.
- Dave's independent server had no duplicate sale/quote for Carol's processor
  order. Dave distributed the listing and received his own fee; he did not run a
  second settlement for the same purchase.

Carol's current immutable terms root is
`77879b4f21cc2dd4c42b05f22893ad8655f6fd3d2772afc224619f5e4fc60eaf`;
Dave's new root is
`0ac7455b68167adecb08b801258a13e35061f8451dca3c95b0d20643902bd3a7`.
The original single-operator listing retains its older accepted root unchanged.

## Required negatives

Focused source suites use synthetic test identities/mock LNURL invoices, **not**
replacement live identities or claims of real-payment coverage:

- `adp-server/adp-server/tests/multi_operator_tests.rs`: six authenticated-route /
  domain tests cover missing/forged second terms, duplicate identities and
  conflicting acceptance/fee roots, fees equal to or above gross, URL aliases,
  exact three-leg construction, persisted quote/proof retries after reopening
  SQLite and rejected third proof/deadline disposition without ownership.
- `core/tests/multi_operator_gate2.rs`: three client domain/discovery tests cover
  absent/invalid second announcements, complete consent coverage, canonical
  allocation, duplicate legs and conflicting acceptance. Missing-announcement
  rejection here is discovery/coverage testing, not relay-history deletion or a
  change to production discovery semantics.

The **real third-payment failure** uses a separate listing, HTTP router and
ADP/client databases. Carol and Dave receive 100/150 sats; Bob's disclosed
750-sat invoice is canceled and its one NWC payment attempt fails. No receipt,
ownership or token is issued, and an authenticated download fails. A test-owned
clock advances the confirmation deadline (not a 24-hour wall-clock wait).
Production recovery records two principal obligations. Carol and Dave each pay
one real refund invoice to Alice through the existing durable wallet seam and
authenticated refund endpoints. Repeated payment/confirmation requests use the
original cached proofs. Both obligations become verified and all four balances
return to this negative case's starting snapshot.

Negative-case counters: **2 successful NWC payments, 1 failed NWC attempt,
2 successful LND refund dispatches**, no redispatch. The canceled third invoice's
blocked original payment intent remains retained, not deleted or automatically
retried. The additional operator transfers belong only to this explicit separate
negative order; they are not duplicate fees on the successful purchase.

## Evidence and safety

Latest summary: `~/.local/state/arcadestr-regtest/last-gate2.json`.
Verified run: `gate2-runs/1793324188547768849/` beneath that state directory.
It retains the successful/failure manifests, announcements, signed quotes and
receipt, encrypted client proof journals, per-invoice dispatch audits,
before/after balances, downloads and isolated negative ADP state.
`operator-independence.json` records non-secret configuration isolation checks.
Keep the entire directory private; never publish credentials, tokens or decrypted
proofs. Operator identities/terms persist separately in `operator-dave/`.

The shared reservation is **2,400 sats** conservatively including the failed
third attempt, negative refunds and fee headroom. Existing 10,000-sat purchase
and 50,000-sat automated-invocation limits remain enforced. Regtest, node identity,
liquidity, exact invoice payee/amount and Alice's established local NWC binding
are checked before irreversible payment.

Initial attempts stopped **before listing/order/payment creation** because
Carol's previously unowned process advertised legacy-only settings, and then
because the first harness draft selected the original listing's old terms rather
than Carol's currently advertised root. With explicit authorization, only that
verified Carol process was stopped gracefully and restarted against its unchanged
database using current local sources/protected config. The harness now selects
the actual advertised immutable root. All logs and the original reservation were
retained; `--continue-gate2` continued only the inspected empty pre-payment stage.
That option refuses any stage containing an order or payment evidence. Never
clear an incomplete reservation or create a replacement order to bypass it.

No production settlement/protocol defect required a change in this gate.
Harness fixes cover current-terms selection, protected independent startup and
waiting for a stopped service's port to be released before restart. All earlier
paid, durability, refund and Gate 1 baselines passed non-paying verification.
No automatic startup payment recovery, GUI work or automatic commits were added.
