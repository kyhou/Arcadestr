# Gate 7 cross-repository acceptance fixtures

Run from the Arcadestr root, with the `adp-server` repository beside Arcadestr:

```sh
cargo test -j 1 --offline --locked \
  --manifest-path tests/gate7_crossflow/Cargo.toml \
  --target-dir ../adp-server/target --test crossflow -- \
  --nocapture --test-threads=1
cargo test -j 1 -p arcadestr-core --features native --locked --test quote_initiation
cargo test -j 1 -p arcadestr-core --features native --locked --lib storage::db::tests
```

This separate test workspace depends on the **actual** core and ADP Server
crates. It uses actual HTTP routers, persistent server storage, the server's
RelayClient, Arcadestr's HTTP/Nostr clients, signed event builders/verifiers,
ownership/portability repositories and installation primitives. Two loopback
Nostr fixtures exchange real EVENT/REQ/EOSE messages and retain immutable events.
Publisher and buyer databases are independent. No public relay is configured.
The included lock file avoids fresh resolution of a yanked transitive dependency;
neither production repository's lock file is changed by these tests.

The four tests cover:

- Multi-server provisioning and artifact registration; public/active timed access;
  expired/upcoming denial; authenticated rather than anonymous transport; explicit
  publisher restart recovery with the original signed event and artifact hash.
- Store Page association and actual marketplace discovery; delegated claims,
  idempotent issuance, campaign cancellation, individual developer revocation,
  independent buyer reconciliation and actual-server ownership denial.
- Multi-operator signed quotes and three-leg synthetic proof verification;
  aggregate and legacy receipts, strict proof replay protection, independent
  receipt import/discovery, server-scoped tokens, token-free NIP-98 downloads,
  authorized server failover, pinned ZIP integrity, real extraction, transactional
  installation/update, restart intent, failed-commit rollback and receipt status
  enforcement. Legacy one-time proof replay fails closed; aggregate proof replay
  recovers the original receipt.
- Lost quote response after the actual server commits, followed by buyer restart:
  the existing desktop/core initiation seam must reuse its durable purchase ID,
  leaving one remote quote. This path does not disable the desktop live guard.

LNURL responses contain **offline-generated synthetic Regtest BOLT-11 bytes**.
Preimages are fixture constants; no Lightning node, invoice issuer, wallet,
payment RPC, regtest miner, public publication or external upload is contacted.
Synthetic signed `refunded` status is a receipt-policy test, not a real refund.
The actual wallet executor's consent, fee, uncertainty and restart behavior is
covered separately by the existing purchase-execution and Gate 3–6 suites.

The tests do not turn local fixture verification into live settlement, relay
retention or production-release acceptance. The desktop live quote/invoice/payment
entry points remain locked. Earlier migrations are immutable; migration 26 adds
only a local request-ID checkpoint, storing public references and a digest rather
than refund addresses, invoice strings, credentials or preimages.
