# Gate 7 Phase 5 — final cross-flow acceptance

**Gate 7 development: COMPLETE. Production release: NOT READY / NOT AUTHORIZED.**

All 13 development criteria pass at the evidence levels and exclusions below.
This closes only the approved development milestone, not live-execution safety
gates or production qualification. No critical in-scope defect remains known
after these checks; this is not a proof that every possible defect is absent.

## Scope and evidence

Phase 5 preserves Gates 3–6 and all Phase 1–4 corrections. Evidence directory:
`/tmp/opencode/gate7-phase5-83h9l6js/`. Original source/status manifests, dirty-file
snapshots and binary diffs were captured before edits in both repositories.
Arcadestr baseline: `8d99fbeec0e19903da98fb831ee4d531da4e18c9`;
ADP Server baseline: `df6d02fd0a11a6d0d3074fb2faec9c355d7b7a82`.

Evidence levels:

- **Local integration:** actual Arcadestr clients/core and ADP Server routers,
  server SQLite, HTTP, server RelayClient, buyer RelayManager, two loopback Nostr
  fixtures, synthetic signed events and independent disposable databases.
- **Synthetic settlement:** offline-generated Regtest BOLT-11 bytes and fixture
  preimages submitted to actual verification routes. No Lightning settlement,
  live invoice creation, wallet payment, regtest mining or real refund.
- **Automated:** focused domain/native tests, mock-wallet interruption tests,
  real WASM browser flows with mocked IPC and blocked external transports.
- **Platform smoke:** unsigned Linux debug Debian package, extraction rather than
  global installation, Xvfb/WebKitGTK startup in a new network namespace with
  read-only host root and disposable HOME/XDG/keyring. Not release certification.

## Confirmed defect and correction

**Arcadestr-owned lost quote response:** the actual server committed a quote, the
HTTP adapter discarded only its response, and buyer restart allocated a new ID.
The pre-fix fixture created **two** remote purchase quotes. ADP Server already
replays a request with its original purchase ID correctly; no server fix was needed.

`SettlementRepository::reserve_quote_initiation` now persists the ID **before**
transport. The existing desktop entry point uses that seam, binding retries to
account, processor, coordinate, listing revision, terms and refund commitment.
Migration 26 is additive; migrations 1–25 are unchanged. Only public references
and a digest are stored, not invoice bytes, credentials, refund addresses or
preimages. Unresolved initiations cannot silently change terms or allocate a fresh
obligation; concurrent reservations cannot produce two successful identities.
Only a completed local quote with a receipt permits a new initiation.

Red evidence: `dropped-quote-confirmed-red.log`. Green evidence:
`dropped-quote-green.log` and the final cross-flow log. Recovery leaves one remote
quote after restart. The public desktop live quote/invoice/payment lock remains
closed; no new permission, executor, wallet implementation or protocol was added.

Other exploratory failures were fixture/environment issues, not product fixes:
the test initially used a non-decrypting signer instead of desktop ActiveSigner;
an installation fixture lacked the required signed launch tier/default; a static
invoice responder reused a payment hash; the claim assertion expected a specific
cancellation error whereas the server deliberately returns generic 403
`campaign is not claimable`. Final assertions follow actual protocol behavior.

## Persistence and packaging

- Fresh initialization and restart create schema 26. A disposable version-25
  database **copy** upgrades additively while preserving receipt, publisher,
  installation and salt-byte sentinels; the original remains version 25.
- Signed grant/revocation, receipt, quote-initiation and installation/update
  records are exercised separately through actual repositories and restart.
- Update intent survives reopening without promoting its candidate; rejected
  commit leaves the original ready snapshot intact. Valid promotion is atomic.
- Production native source in the packaging copy is hash-identical to the final
  production changes (`packaging-source-consistency.json`). Frontend assets were
  built/stamped in that copy, leaving preserved `web/dist` untouched.
- Initial bundling exhausted host disk space. Newly generated staging/binary
  artifacts were preserved in tmpfs; Tauri's bundler then produced a Debian
  package successfully. No source checkpoints or preserved databases were removed.
- Package: `package-target/debug/bundle/deb/Arcadestr_0.1.0_amd64.deb` in the
  evidence directory; SHA-256
  `b182634d10e0869281ca6ba99a2dfe4173800679a377cd239f7f709c3af27a8a`.
- `dpkg-deb --info` and extraction passed. Clean startup **and restart** reached
  the stable sign-in screen, not merely account-loading text. Both application
  databases were schema 26 with zero accounts/purchases/quotes/payment intents.
- The bundler warned that its bundle-type marker was unavailable and that the
  `.app` identifier suffix is discouraged on macOS. These are documented tooling
  observations, not evidence of updater or macOS release compatibility.

No signed release, release-mode native package, AppImage/RPM, global package
installation, Windows/macOS startup, real signer/extension, wallet interoperability
or production upgrade is certified here. The standalone full-shell mobile overflow
reported in Phase 4 remains separate from required desktop acceptance; no layout
substitution or unrelated UI change was introduced.

## Cross-flow coverage matrix

| Requested flow | Actual evidence | Excluded scenarios |
|---|---|---|
| Publisher authentication/listing | NIP-98 client → actual provision/upload routers; developer-signed listing and operator metadata | Real remote signer, public publication |
| Store Page publication/association | Signed Store Page and reciprocal listing pointer over local Nostr; association verifier | Production relay propagation/retention |
| Multi-operator provisioning/artifacts | Two actual operators, separate held keys, same signed artifact; three quote payment legs | External operator deployment |
| Discovery/listing integrity | Actual buyer NostrClient/RelayManager finds exact signed listing/hash | Live marketplace inventory |
| Public/timed acquisition | Public and active timed NIP-98 downloads; expired/upcoming and anonymous denial | Anonymous transport, live external serving |
| Claim/cancellation/revocation | Actual issuance/idempotence; cancellation rejects a new buyer; old grant survives cancellation; developer revocation reconciles independent buyer and server | General grant portability |
| Paid settlement/receipt | Actual quote decrypt/verification and three synthetic proof legs; signed aggregate receipt; invalid/duplicate/cross-buyer proof checks; synthetic refund status | Real settlement, payment dispatch, actual refund |
| Receipt portability | Separate buyer import and another buyer database's actual local-relay discovery; verified ownership/Library projections in regression suite | Live cross-device availability |
| Aggregate/token-free downloads | Both receipt profiles accepted by actual second server with independent NIP-98 verification | Trusting issuer assertions without authorization |
| Authorized fallback/pinning | Actual first router stopped; second authorized router serves pinned ZIP; cross-server token rejected; corrupted artifact rejected | Arbitrary unsigned mirrors |
| Installation/update/rollback | Verified extraction, repository ready state, durable update intent, rejected-commit rollback and valid promotion; Gate 5 lifecycle guard tests | Production game compatibility, every OS filesystem |
| Publisher/buyer recovery | Persisted immutable publisher intent, lost upload acknowledgment and explicit original-artifact recovery; lost quote response/restart; signed grant and installation journals | Live external crash/power-loss guarantees |
| Account/async isolation | Native claim/reconciliation adversarial tests and real WASM stale callback/account/ABA regressions; account-scoped repositories | Real extension/wallet interoperability |

This is cross-component integration plus complementary UI/desktop regressions,
not a claim that one automated native UI session executed every flow. Local relay
fixtures retain all historical snapshots intentionally; production retention is
an explicit external limitation.

## Restrictions and follow-up

Live quote/invoice/payment creation and automatic repayment remain locked.
Synthetic proof/status acceptance does not prove real Lightning settlement/refund
or paid restart durability with a real wallet. Multi-device evidence uses separate
local databases, not live devices or production relays. Public/timed access still
requires token or NIP-98 transport authentication. No general grant portability,
NIP-59 receipt transport or browser payment/publishing/install/recovery executor
was introduced. Experimental event kinds, strict fee-assurance limits, relay
retention limits and existing external capability restrictions remain effective.

Uncertain quote initiation remains conservatively pinned; changing its terms or
abandoning it requires a separately designed explicit reconciliation policy, not
automatic replacement. Legacy one-time payment-proof replay returns conflict;
the original receipt remains recoverable through relay discovery. Aggregate proof
replay returns its original receipt. No replay behavior authorizes repayment.

Recommended next milestone: separately scoped production-hardening/release
qualification, including signed release packaging, supported host matrix and
explicitly authorized real signer/wallet interoperability. Any live settlement
exercise needs separate authorization and must retain the existing safety gates.
No next milestone, commit or push is performed by this acceptance work.

## Commands and regression results

All commands ran locally. Exact argument arrays and exit codes are retained in
`regression-status.json`, `frontend-status.json`, `browser-status.json`,
`packaging-final-status.json`, `startup-status.json` and server test status.

| Command/group | Result and evidence level |
|---|---|
| `cargo test -j 1 --offline --locked --manifest-path tests/gate7_crossflow/Cargo.toml --target-dir ../adp-server/target --test crossflow -- --nocapture --test-threads=1` | **4 passed**, actual-client/actual-router cross-repository integration; `crossflow-verified.log` |
| `cargo test -j 1 -p arcadestr-core --features native --locked --test …` | 18 selected integration suites, including purchase execution, recovery, receipt portability, publisher management, authorization and quote initiation; automated/disposable SQLite |
| Core `--lib` filters: `adp_client::tests`, `http_client::tests`, `adp_download::tests`, `storage::db::tests`, `adp_storage::tests`, `package_extraction::tests` | Together with the preceding row: **253 passed**, one pre-existing live test ignored |
| `cargo test -j 1 -p arcadestr-desktop --locked` with `adp_commands::tests`, `gate5`, `publisher_commands::tests` filters | **103 passed**, five pre-existing live tests ignored; native/adversarial/local fixtures |
| `cargo test -j 1 -p arcadestr-app --locked` | **518 passed** |
| `cargo test -j 1 -p arcadestr-xtask --locked` | **3 passed**, frontend fingerprint/stamp tests |
| `python3 -B -m unittest discover -s tests -p 'test*.py'` | **63 passed**, supervisor/live-lock/front-end safety tests |
| ADP Server: `cargo test -j 1 --workspace --locked --offline` | **245 passed**, one pre-existing large-upload test ignored |
| `bash scripts/check-frontend-targets.sh` | PASS: standalone app/web WASM, default desktop-bridge WASM, native desktop |
| `cargo check -j 1 -p arcadestr-web --target wasm32-unknown-unknown --features web --release --locked` | PASS; standalone release-mode compilation, **not** native release packaging |
| `CARGO_BUILD_JOBS=1 CARGO_NET_OFFLINE=true trunk build [--features web] --dist <isolated-output>` | Both desktop-bridge and standalone PASS; preserved `web/dist` unchanged |
| Existing standalone boundary, desktop boundary, receipt-portability, claim, revocation and Gate 4 purchase browser scripts | **6 suites passed**, real compiled WASM/mock IPC; external transports blocked |
| Packaging-copy `cargo run -p arcadestr-xtask -- frontend-build`; `cargo tauri build --debug --bundles deb --no-sign …` | Native build passed; initial bundling failed from disk exhaustion, not compilation |
| `CARGO_TARGET_DIR=<tmpfs-target> cargo tauri bundle --debug --bundles deb --no-sign` | Debian package PASS after preserving generated output in tmpfs |
| `dpkg-deb --info`, `dpkg-deb --extract`; sandboxed `dbus-run-session`/Xvfb startup probe | PASS package metadata/extraction, clean-profile stable sign-in and restart |
| `cargo fmt --all -- --check` | **Pre-existing failure**: eight byte-identical baseline files, proved by hashes in `workspace-format-baseline-proof.json`; unrelated files were not reformatted |
| Isolated harness format check; changed production-file formatting; `git diff --check` | PASS; no new formatting defect in the changed production files |

No ignored live test was enabled, and no entire-repository formatting success is
claimed. Evidence from initial failing fixture attempts is retained rather than
overwritten by the final results.

## All 13 acceptance criteria

PASS means the **development** requirement is met within the stated supported
scope. External/live scenarios explicitly excluded here remain unverified, not
implicitly passing production checks.

| # | Criterion | Verdict | Evidence and exclusions |
|---|---|---|---|
| 1 | Publisher-to-buyer lifecycle consistency | PASS | Four actual-client/server cross-flows plus complementary native/UI regressions; not one fully automated native UI session or live deployment |
| 2 | Paid settlement, receipt and ownership correctness | PASS | Actual quote/three-leg synthetic preimage verification, aggregate/legacy receipt import and ownership/status enforcement; mock-wallet uncertainty tests; no real Lightning settlement/refund |
| 3 | Public, timed and claim acquisition | PASS | Actual authenticated public/timed acquisition, inactive-window denial, delegated claim/idempotence, cancellation and individual revocation; anonymous transport excluded |
| 4 | Cross-device receipt import/discovery and actual-server downloads | PASS | Separate buyer databases, offline import and actual local-relay discovery, token-free second-server NIP-98 downloads; no live devices/production retention guarantee |
| 5 | Multi-server artifact pinning and installation integrity | PASS | Two actual servers, scoped-token rejection, stopped-router fallback, pinned ZIP extraction, interrupted update and rejected-commit rollback, corrupted file rejection; host filesystem/launch matrix excluded |
| 6 | Publisher management and recovery | PASS | Immutable publisher operation/checkpoint, lost upload acknowledgment, explicit original-artifact recovery, management/revocation tests; no public publication/external upload |
| 7 | Identity, authorization, account isolation and privacy | PASS | Exact signed-event/NIP-98 routes, malformed/historical-authority tests, native session/ABA guards, WASM stale callbacks, private durable payload checks; actual extension/wallet interoperability excluded |
| 8 | Cross-repository protocol compatibility | PASS | Current implementations compiled together, four actual-router cross-flows and 245 server tests; no Phase 5 server/protocol edit required |
| 9 | Fresh initialization, supported upgrades and persistence | PASS | Fresh schema 26; supported version-25 copy upgrade preserves existing records; durable quote identity, signed revocation/receipt and install/update restart tests; arbitrary historical/production schemas excluded |
| 10 | Supported-platform compilation, packaging and startup | PASS | Native Linux, default desktop WASM, reduced standalone WASM, both Trunk builds, unsigned debug Debian creation/extraction and network-isolated clean/restart sign-in; signed release/native release package and other OSes excluded |
| 11 | Relevant automated and local end-to-end validation | PASS | 253 core + 103 desktop + 518 app + 3 xtask + 63 Python + 245 server + 4 cross-flow tests; six browser suites; seven combined pre-existing ignored tests remain ignored; workspace formatting has only proven pre-existing failures |
| 12 | External limitations and operational restrictions documented | PASS | This report, harness README and existing standalone capability documentation distinguish local/synthetic evidence, release exclusions, mobile overflow and live locks |
| 13 | No known critical Arcadestr-owned correctness/security defect remains | PASS | Reproduced lost-response defect corrected and red/green tested; all relevant final checks passed except unchanged baseline formatting; no broader correctness or production-security guarantee claimed |

## Preservation and stop point

Only three pre-existing source files changed during Phase 5:
`core/src/settlement_repository.rs`, `core/src/storage/db.rs` and
`desktop/src/adp_commands.rs`. The desktop diff is confined to durable quote-ID
reservation, preserving all earlier Gate 7 edits. New files are migration 26,
`core/tests/quote_initiation.rs` and this isolated test/report workspace.

Baseline manifests confirm no other pre-existing Arcadestr file was altered or
removed; ADP Server source, untracked checkpoints, status and dirty diff remain
unchanged. Both HEADs are unchanged. Original database locations and user profiles
were not reset or deleted. Packaging writes used a disposable source copy and
profiles; generated artifacts were preserved when relocated for disk space.

Final machine-readable evidence: `final-validation-summary.json` in the evidence
directory. Changes remain **uncommitted and unpushed**. Phase 5 stops here; no
subsequent milestone or live activity is authorized by this result.
