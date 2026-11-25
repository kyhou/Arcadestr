# Regtest harness (adp-server-embedding drivers)

An independent Cargo workspace for the real-Lightning regtest drivers that
embed the adp-server reference implementation. It is not a member of Arcadestr's
workspace and depends on the sibling `../adp-server` checkout, pinned in
[`tests/interop/adp-server.rev`](../interop/adp-server.rev). See
[`tests/interop/README.md`](../interop/README.md).

| Example | Purpose |
|---|---|
| `regtest_refunds` | Isolated partial-settlement and refund cases with an injected settlement clock |
| `regtest_gate1` | Settlement verification Gate 1, including server refund-state inspection |
| `regtest_gate2`, `regtest_gate2_preflight` | Gate 2 multi-operator settlement and its read-only signer preflight |
| `regtest_gate3_*` (`denial_server`, `gui`, `nip46_server`, `nwc_purchase_server`, `restore`) | Gate 3 drivers built on `regtest_refunds` |

The shared `regtest_split_purchase.rs` module stays in `core/examples` (it is
also an Arcadestr-only example there) and is included by path.

Drivers are built and run by [`scripts/regtest`](../../scripts/regtest.md). That
script builds into Arcadestr's `target/`, so binaries keep their
`target/debug/examples/<name>` paths. To build or test them manually:

```sh
cargo build --locked --manifest-path tests/regtest_harness/Cargo.toml \
  --target-dir target --example regtest_gate1
scripts/interop-test regtest-harness   # compile all drivers and run their unit tests
```

The regtest `adp_repo` setting should name the same sibling checkout. That way
the running server binary and the embedded server library come from one source
tree.
