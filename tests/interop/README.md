# ADP cross-implementation tests

Arcadestr and [adp-server](https://github.com/kyhou/adp-server) are independent
applications. They interoperate only through ADP-01 over HTTP and Nostr. Neither
Cargo workspace references the other repository:

- `cargo build`, `cargo test` and `--locked` checks in Arcadestr's workspace
  resolve and build with no adp-server checkout present.
- adp-server's own workspace never references Arcadestr. Its `adp-spec-sync` CI
  check only compares documentation (the ADP-01 mirror), which is not build
  coupling.

Tests that need the reference server live in two **independent** Cargo
workspaces under `tests/`. Each declares an empty `[workspace]` and has its own
`Cargo.lock`, so Arcadestr's ordinary workspace never resolves them:

| Workspace | Suite | What it exercises |
|---|---|---|
| `tests/gate7_crossflow` | `black_box` | Spawns the real `adp-server` **binary** as a separate process. All assertions go through ADP HTTP endpoints and Nostr events on two loopback relays: discovery announcement, `/.well-known/adp`, NIP-98-only provisioning, idempotency, attestation publication and verification with Arcadestr's own parser, and per-developer key isolation. |
| `tests/gate7_crossflow` | `crossflow` | Gate 7 acceptance flows against the server's actual router and storage, hosted in-process (see [its README](../gate7_crossflow/README.md)). |
| `tests/regtest_harness` | examples | Real-Lightning regtest drivers that embed the server router, inject its settlement clock and decrypt its refund state (see [its README](../regtest_harness/README.md)). The interop runner executes their unit tests: the payment guard that accepts only exact, local, unexpired regtest invoices, and the Gate 2 signer-file policy. The Lightning scenarios themselves run against Polar via `scripts/regtest`, which builds them from this workspace. |

`crossflow` and the regtest drivers import adp-server internals because they
inject deadline time and inspect server-side refund state, which no ADP endpoint
exposes. Making them black-box would lose that coverage, so they stay in isolated
workspaces instead. New interop coverage should be black-box (`black_box.rs`)
where the protocol surface allows it.

## Running

The interop workspaces resolve adp-server as the sibling checkout
`<parent>/adp-server`, beside Arcadestr:

```sh
scripts/interop-test                 # all suites at the pinned revision
scripts/interop-test black-box       # one suite: black-box | crossflow | regtest-harness
scripts/interop-test --fetch         # clone adp-server beside Arcadestr if absent
scripts/interop-test --revision SHA  # test another adp-server commit
scripts/interop-test --any-revision  # test whatever the sibling has checked out
```

The runner refuses to run when the sibling is not at the selected revision. It
never checks out or modifies an existing adp-server checkout, and it warns when
that checkout has uncommitted changes. Builds use the separate
`target/interop` directory.

Isolation:

- The black-box test gets a fresh temporary directory (server database and
  files), a dynamically allocated HTTP port and two in-process loopback relays
  on ephemeral ports. It uses a generated operator key and never contacts a
  public relay.
- The server process is killed when the test ends, including on assertion
  failure. The runner also kills any leftover process started from
  `target/interop/debug/adp-server` on exit.
- Every suite keeps its databases, server files and downloads in
  `tempfile`-managed directories. They are created under the system temporary
  directory (`TMPDIR`) and removed when the test ends.

Requirements: a Rust toolchain, `git`, and on Linux `libssl-dev` and
`pkg-config`. The Lightning regtest scenarios additionally need the Polar
infrastructure described in [`scripts/regtest.md`](../../scripts/regtest.md).

## Pinned adp-server revision

`tests/interop/adp-server.rev` holds the full commit SHA that
`.github/workflows/adp-interop.yml` checks out and `scripts/interop-test`
verifies. CI never follows adp-server's moving `main`. You can run a manual
workflow with a different `adp_server_revision`.

To move the pin when protocol compatibility changes:

1. Push the adp-server commit, so CI can fetch it from GitHub.
2. Check it out in the sibling repository and run `scripts/interop-test`.
3. If adp-server's dependencies changed, refresh the interop lock files without
   upgrading unrelated crates. Then rerun with `--locked`:
   `(cd tests/gate7_crossflow && cargo metadata --format-version 1 >/dev/null)`,
   and the same in `tests/regtest_harness`.
4. Write the new SHA to `tests/interop/adp-server.rev` and commit it together
   with any interop test or lock-file changes in the same commit.

Protocol changes that alter cross-implementation behavior should update ADP-01
first (canonical here, mirrored in adp-server) and then move this pin.
