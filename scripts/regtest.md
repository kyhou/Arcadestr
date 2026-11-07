# Arcadestr / ADP local regtest runbook

Latest extensions: [refund scenarios](regtest-refunds.md),
[Gate 1 live-relay/ambiguous refund verification](regtest-gate1.md), and
[Gate 2 independent multi-operator settlement](regtest-gate2.md). The historical
single-operator scope below remains the original baseline, not a current claim
that these later gates are unimplemented. Dave's explicitly authorized Nostr
identity rotation and protected reboot-persistent signer locations are recorded
in the Gate 2 runbook; his Polar Lightning identity remains unchanged.

## Status and scope

The supervisor, regression/smoke harness, and **one genuine single-operator
ADP/NWC split purchase** are implemented and passing. See
[the focused split-purchase runbook](regtest-split.md) for evidence and commands.
The original full 16-scenario suite is not complete; recovery, refunds and a
second operator are deliberately outside the current scenario. Wallet transfers
are not labeled end-to-end purchases.

All builds use these current local working trees: `Arcadestr` and sibling
`adp-server`. No GitHub checkout, database reset, seed, channel creation, wallet
creation, new Nostr identity, or new NWC connection is used. Existing unrelated
working-tree edits are retained.

## Commands (Zsh / Arch Linux)

From `Arcadestr`:

```zsh
./scripts/regtest up                 # build current sources; restore existing services
./scripts/regtest status             # inspect readiness; never starts services
./scripts/regtest test               # live read-only/invoice probes; reports coverage gaps
./scripts/regtest test --source      # additionally run protocol/storage/route regressions
./scripts/regtest test --payments    # real 1,000-sat Bob/Carol ADP purchase through existing NWC
./scripts/regtest test --verify-paid # recheck the settled test order, no new payment
./scripts/regtest test --crash-recovery # isolated crash/restart purchase, never redispatch a recorded invoice
./scripts/regtest test --verify-durability # recheck the crash-recovered order without paying
./scripts/regtest test --wallet-smoke # separate wallet-only Lightning smoke transfers
./scripts/regtest up --gui           # also build/start frontend and desktop
./scripts/regtest status --gui
./scripts/regtest down               # gracefully stop only recorded, owned processes
```

Optional Zsh convenience, without an installation or OS startup hook:

```zsh
alias regtest='/home/joel/Sync/Projetos/Arcadestr/scripts/regtest'
```

`up --no-build` reuses previously built local binaries; do not use it after source
changes. An already-running service is health-checked, not silently replaced with
a newly built binary. To run a changed server, use `down`, then `up`.

Requirements: Python 3, Cargo/Rust, Docker access, existing Polar containers,
`nak`, the existing strfry binary/configs, and a graphical session for GUI launch.
The script never invokes sudo or changes Docker/OS startup policy. If Docker is
not running, start Docker using the machine's existing administrative setup.

## Preserved services and configuration

| Service | Existing resource |
|---|---|
| Bitcoin / LND | `polar-n1-backend1`, `polar-n1-{alice,bob,carol,dave}` |
| Polar volumes | `~/.polar/networks/1/volumes` |
| strfry | `~/strfry-relays/relay{1,2}/strfry.conf`, persistent DBs, ports 7777/7778 |
| NAK | in-memory relays on 10547/10548; empty histories after restart are intentional |
| LNURL fixture | existing `adp-server/examples/polar_lnurl_fixture.rs`, port 18080 |
| ADP | existing `.env`, database and files, port 9099 |
| Alby Hub | existing `~/.local/share/arcadestr-albyhub-regtest/nwc.db` |
| Arcadestr | existing accounts, application data and OS-keyring entries |

`up` uses `docker start` on existing containers, never `compose up` or container
recreation. Missing containers require operator investigation in Polar. No blocks
are mined and no liquidity is moved automatically to repair channels. Polar's GUI
is reported separately; its running containers do not require the GUI to be open.

`down` deliberately leaves Polar containers and pre-existing Alby/GUI processes
alone. It preserves all persistent data. Only processes started by this supervisor
are stopped, after matching PID, kernel start time and boot ID. It never kills a
port owner, uses `pkill`, force-kills a process, or runs Docker volume removal.

ADP's existing environment file is explicitly loaded as **data**, not sourced as
shell code. The supervised process binds `127.0.0.1:9099`; the file is not edited.
The configured public URL remains unchanged for exact NIP-98 URL matching.
The supervisor rejects external ADP/relay URLs and a missing existing ADP database.

Optional machine-specific overrides belong in
`~/.config/arcadestr-regtest/settings.json`, outside both repositories:

```json
{
  "adp_repo": "/absolute/path/to/adp-server",
  "adp_env": "/absolute/private/path/to/existing-adp.env",
  "polar_dir": "/home/joel/.polar/networks/1",
  "strfry": "/home/joel/Downloads/strfry/strfry",
  "relay_dir": "/home/joel/strfry-relays"
}
```

The pre-existing repository `.env` is read for compatibility, never copied or
rewritten. Use `adp_env` to select an existing private external credential file.
Never put new credentials in either checkout or in `settings.json`.

## Existing identities and startup requirements

1. `~/.config/arcadestr-regtest/lnurl-fixture.env` now contains all four established
   public identities. Alice/Bob came from the existing account store; Carol from
   the existing ADP operator; Dave was supplied explicitly by the user. Existing
   settings, invoice-only macaroons and TLS certificates were preserved. If moving
   this environment, retain those **existing public Nostr identities**, never
   replace them:

   ```text
   POLAR_ALICE_NOSTR_PUBKEY=<existing buyer 64-hex public key>
   POLAR_BOB_NOSTR_PUBKEY=<existing developer 64-hex public key>
   POLAR_CAROL_NOSTR_PUBKEY=<existing operator-A 64-hex public key>
   POLAR_DAVE_NOSTR_PUBKEY=<existing operator-B 64-hex public key>
   ```

   Keep this file mode `600`. Literal assignments and data-only `$HOME`/`POLAR_*`
   references are supported; `source`, command substitution and executable shell
   are never used. Certificate/macaroon paths default
   to the existing Polar volume files; public LND node keys come from `getinfo`.
   Explicit literal `POLAR_*_TLS_CERT` / `POLAR_*_INVOICE_MACAROON` overrides work.
   The fixture config generated outside the repositories caps invoices at 10,000
   sats. Neither the existing example config nor production settings are changed.

2. Alby Hub's existing buyer app was found under the local label
   `Alice Arcadestr Regtest Buyer`. Its payment permission exists; no connection
   was recreated. Live read-only NWC verification now passes, separately from
   database/process checks. The standalone helper initially lacked the Linux
   Secret Service backend enabled by the desktop's Tauri plugin; its Linux
   development dependency now enables that same backend.
   The read-only `regtest_nwc` helper uses Arcadestr's production OS-keyring loader
   and `NwcClient::verify`. Unlock the existing Hub and keyring/account if it
   reports blocked. Do not repair this by making a new connection.

3. The single-operator driver uses Arcadestr's existing account manager/signers,
   NWC client, ADP client, event builders, validators and repositories. No private
   key export or GUI login is needed for the existing local accounts. It does not
   implement recovery, refunds or a second operator.

GUI launching requires the graphical login session. `up --gui` compiles the
current desktop and starts Trunk on 5173, putting generated assets under private
state instead of overwriting the already-modified `web/dist`. Opening/unlocking
accounts and refund-recipient workflows still require GUI verification. The GUI
launch path has not been exercised in this run.

## Safety and diagnostics

- Every wallet smoke payment rechecks Bitcoin's regtest chain and **all four**
  LND nodes' exclusive Bitcoin/regtest chain and synchronization.
- Each node needs at least 10,000 sats of both local and remote active-channel
  balance against verified local peers. This is a readiness floor, not a route
  guarantee. No automatic rebalancing or channel changes occur.
- LND decodes every invoice; the harness checks regtest currency, exact amount,
  unexpired timestamp and exact local recipient node public key before payment.
- `--payments` executes one 1,000-sat ADP purchase (Bob 900, Carol 100) via Alice's
  existing NWC. `--wallet-smoke` separately sends 100 sats Alice→Bob/Carol/Dave,
  then returns 100 sats from
  each successfully paid recipient. These are wallet-level return transfers,
  **not protocol refunds**. Wallet-smoke routing fees are limited to 10 sats per
  transfer; the NWC scenario reserves fee headroom and verifies actual Alice fees.
- Maximum principal per logical purchase: 10,000 sats. Maximum reserved payments
  per invocation: 50,000 sats; reservations conservatively include maximum fees.
  Reservation is fsynced before payment. Failed/ambiguous payments retain their
  reservation and are never automatically retried.
- Payment journals contain hashes/statuses, never preimages or credentials.
  No payment API accepting arbitrary destinations is exposed by this script.
- State/logs are owner-only under `~/.local/state/arcadestr-regtest` (or
  `$XDG_STATE_HOME/arcadestr-regtest`). `last-test.json` contains sanitized results;
  `payment-run.json` records the most recent wallet run. Logs remain private and
  are not dumped into command output. Do not upload raw application logs.
- Repeated `up` reuses healthy listeners. A different service on a required port
  is an error, never something to kill. Commands take an exclusive supervisor lock.

## Coverage and results

| Requested scenario | Evidence / remaining work |
|---|---|
| 1. Connectivity/authentication | Live relay REQ/EOSE, ADP health/discovery, missing-auth rejection and signed NIP-98 accepted (authenticated missing-resource 404) |
| 2. Four LNURL invoices | All four live invoice probes pass; existing fixture's isolated regressions pass |
| 3–4. One/two-operator purchases | Genuine single-operator ADP/NWC purchase passes; second operator excluded |
| 5. Fee distribution | Real 1,000-sat purchase: Bob 900, Carol 100, verified by invoices and channel deltas |
| 6. Aggregate receipt | Signed kind-1020 receipt validated, persisted and retrieved from local relays |
| 7. Ownership/download | Persisted buyer ownership, valid token, real 30-byte fixture download with matching SHA-256 |
| 8. Partial failure | Deterministic source storage deadline/obligation tests; live interruption driver missing |
| 9. Application interruption | Real settlement-before-checkpoint process exit and restart passes through NWC lookup; one dispatch per invoice; GUI startup wiring deferred |
| 10. Server restart | Actual supervisor stop/start exercised; isolated persisted-receipt DB reopen/retry test passes; no live mid-purchase restart claim |
| 11. Refunds | Source obligation/deadline tests plus six real wallet transfers pass; no ADP refund execution claim |
| 12–13. Missing/incomplete relay histories | Source suites pass; single-operator purchase succeeds while NAK histories intentionally remain incomplete; dedicated fault scenarios excluded |
| 14. Duplicate proofs | Existing protocol/route/storage suite passes; no live split replay scenario claim |
| 15. Idempotent retries | Repeated supervisor startup/shutdown and persisted-receipt retry regression pass |
| 16. Invalid/expired invoices | Fixture/source/driver safety regressions and live invalid-amount probes pass |

Executed: 20 supervisor safety tests; ADP core, server library/integration, fixture,
Arcadestr split-domain, persistence and LNURL policy suites. All these source
suites passed; one pre-existing >1-GiB upload test is ignored. Desktop `cargo check`
passed with existing warnings. Six actual wallet transfers settled, totaling 600
regtest sats of principal (including the three returns), in the earlier wallet-only
run. The focused genuine ADP purchase separately settled 1,000 sats via NWC.

## Confirmed fixes and unresolved design issues

- **Fixed:** valid aggregate receipts now grant durable ownership in both receipt
  chain selectors; singular proof fields are not incorrectly required. A shared
  root predicate and account/fork/malformed-total regressions preserve validation.

- **Fixed:** refund LUD-16 resolver supports explicit loopback HTTP in debug
  desktop builds with `ARCADESTR_REGTEST_ALLOW_LOOPBACK_HTTP=true`. Release builds
  cannot enable it. Default resolution is HTTPS-only; local HTTP callbacks must
  retain the exact origin; redirects and URL credential/path injection are rejected.
  `up --gui` sets the opt-in for its desktop process, not globally.
- **Fixed:** republishing a durable aggregate receipt now returns a fresh usable
  download token through the same issuance code as initial settlement. Regression
  covers publication failure, DB reopen, unauthorized buyer, identical receipt,
  repeated retries, expiry and token-to-buyer binding.
- **Fixed (test tooling):** the standalone NWC helper uses the same Linux keyring
  backend as the desktop. It successfully verifies the existing buyer connection
  without exporting, rewriting, or recreating the NWC credential.
- **Implemented:** durable payment intent/proof checkpoints, read-only NWC
  reconciliation and an authenticated resume command. The real
  settlement-before-checkpoint crash test passes without duplicate dispatch;
  see [payment durability](regtest-durability.md). Unknown/unsupported lookup
  remains blocked safely; GUI/startup resume wiring is still deferred.
- **Unresolved:** purchases UI prepares recovery/replaces refund addresses, but
  does not complete recipient-side refund payment/confirmation. Buyer NWC must
  not be used to pay a recipient's refund obligation.
- **Unresolved:** `SettlementConfig` is consulted for advertisements/discovery,
  but quote/purchase handlers do not enforce its enabled/configured fee/legacy
  sales policy. Terms acceptance and runtime configuration need one explicit,
  typed authority boundary. No protocol behavior was silently changed here.

Timeout tests call existing production deadline functions with explicit integer
times in isolated test databases. No wall-clock jumps, shortened production
deadlines, or 24-hour sleeps are used.
