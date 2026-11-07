# Gate 3: explicit NIP-46 authorization denial

## Result and scope

**Passed:** actual desktop GUI refusal/regrant for `sign_event` and
`nip44_decrypt`, using actual signed, encrypted NIP-46 requests and responses
over **only `ws://127.0.0.1:7778`**, the existing local strfry relay.
All pairing, initial approvals, permission revocations, rejections and regrants
were automated. The signer remained running during the refusal matrix.

**Not verified:** NIP-46/NWC payment durability or complete purchase recovery.
This result does not close Gate 3 or change earlier payment reservations.

Private evidence is retained at:
`~/.local/state/arcadestr-regtest/gate3-denial-runs/1793410830858993406/`.
Do not publish this directory: it contains encrypted signer/keyring storage,
protected unlock passwords, pairing material and private operational logs.

## Actual CLI capabilities

The checksum-verified upstream **v0.6.0 release lacks `bunker pending`** and
rejects the requested scripted workflow with a usage error. It was not used to
pretend that approval/rejection succeeded.

The upstream source revision
`2718f2841bd58ad9c11c5e176d460cb48fe5ba30` was built with Go 1.26.10 into the
private run's `bin/ncli`. Its actual help and commands support:

```text
ncli bunker pending list --json
ncli bunker pending approve <id> --always --for 24h --json
ncli bunker sessions grants <app-pubkey> --json
ncli bunker sessions revoke-grant <app-pubkey> --method sign_event --kind 27235 --json
ncli bunker sessions revoke-grant <app-pubkey> --method nip44_decrypt --json
ncli bunker pending reject <id> --json
```

The harness always invokes that pinned binary, not the older release on PATH.
Its revision and executable digest are recorded in `ncli-revision.json`.

## Isolation and safety

- A new persistent signer identity was generated into ncli's password-encrypted
  vault. Its vault, password, configuration and daemon state remain in the
  owner-only run directory. A short private symlink was required because the
  original absolute Unix-socket path exceeded Linux's socket-path limit.
- The desktop used empty isolated application storage, a separate D-Bus session
  and a separately unlocked persistent Secret Service/keyring. The existing
  `nak bunker`, `nip46` profile, credentials and global active-profile pointer
  were not used or modified.
- Both ncli and the desktop used only the existing strfry relay at port 7778.
  Discovery was disabled. GUI evidence shows **1 RELAY**.
- Signing was exercised through the real Purchases → **LOAD OBLIGATIONS — NO
  PAYMENT** control against a new isolated server's authenticated read route.
  That route verifies the returned NIP-98 signature. No existing operator or
  wallet credentials were used by the fixture.
- Decryption used the real debug GUI's opt-in **REGTEST NIP-44 CHECK — NO
  PAYMENT** control. The backend encrypts a fresh local challenge to the active
  remote identity, requests `nip44_decrypt` through its actual signer, validates
  the returned plaintext and checks that the active identity is unchanged.
  It accepts no user payload, publishes no event, and writes no business state.
  Release builds reject the probe even if its environment flag is set.
- The fixture blocks `/settlement/.../next` before invoice preparation. No Buy
  operation was performed; no quote or invoice was needed for the refusal matrix.
- Final isolated desktop counts: **0** NWC journal entries, **0** invoice
  attempts, **0** quotes, **0** refund attempts, **0** purchases, **0** tokens.
  **Zero Lightning payments, zero new reservations.** Digests of **55 protected
  original database/profile/configuration/reservation files** matched the
  pre-test baseline.

## Exact observed GUI and protocol behavior

Each initial operation was approved with a remembered grant. The relevant grant
was then removed using `sessions revoke-grant`, and the empty relevant-grant
state was checked before triggering the next GUI action. Only the isolated
client's permissions were changed.

| Method | Rejected request ID | Actual NIP-46 error | GUI observation after CLI rejection | Recovery after approval |
| --- | --- | --- | --- | --- |
| `sign_event` (kind 27235) | `3687584086` | `denied: rejected`, no result | `JS error: JsValue("Remote signer denied authorization. Grant permission in the signer and retry; no payment was sent.")` | `No obligations are owed by this account at this processor.` |
| `nip44_decrypt` | `2369438050` | `denied: rejected`, no result | The debug result contains the same explicit authorization-denied message, with the existing JS error wrapper. | `NIP-44 decryption verified. No payment or storage changes.` |

The GUI refusal was observed about **0.139 seconds** and **0.137 seconds** after
the respective CLI rejection calls. ncli history records both as explicitly
rejected, **not expired**. The bunker was connected, not stopped; these are not
timeouts or disconnection simulations.

`actual-nip46-wire-verification.json` independently verifies **five actual
request/response pairs**: signing reject/regrant and decryption initial
approve/reject/regrant. The read-only audit fetched kind-24133 events from port
7778, verified their signatures, decrypted transport locally using only the
isolated identity, correlated request IDs and checked the exact refusal errors
and successful responses. It wrote neither private keys nor successful plaintext
results. This is actual wire evidence, not synthetic notification replay.

## Confirmed defect and regression coverage

Before the fix, the same real signing rejection produced only:
`JS error: JsValue("Authenticated recipient obligations unavailable")`.

The SDK erases backend errors into strings; Arcadestr flattened those into a
generic signing failure, and the desktop obligations-read command discarded the
cause again. The fix:

1. Classifies ncli's explicit `denied: ...` refusal and exact known refusal
   strings as `AuthorizationDenied`; classifies the SDK's exact `timeout`
   separately. Arbitrary error substrings are not treated as refusal.
2. Preserves this classification through SDK signer adapters.
3. Shows a safe authorization-denied message for the authenticated GUI read,
   without exposing an arbitrary remote error payload. Timeout and generic
   failures remain distinct.

The regression failed before the fix with `Signing failed: denied: rejected`.
The actual GUI matrix then passed with the rebuilt desktop and private frontend.
Validation passed: **11 native signer tests**, **2 focused desktop tests**,
**502 app tests**, **22 Gate 3 Python harness tests**, and the private WASM
frontend build. Probe tests cover release rejection and explicit opt-in;
harness tests cover ambiguous requests, foreign-client isolation, exact grant
revocation and protected-state mismatch stopping before signer access.

## Retained failed attempts and limitations

- The first no-payment server attempt lacked its isolated server master-key
  environment variable. Its failed state was retained; a separate fixture
  directory used a newly generated protected key. It made no payment.
- An earlier restored isolated desktop session failed its authenticated read
  without a matching new pending request/history entry. It was **not** counted
  as explicit-denial evidence. The successful matrix used a fresh real GUI
  pairing to the same persistent isolated signer. Restored-session behavior was
  not verified by this run.
- The ordinary purchase view was compatibility-gated and therefore was not
  used to claim decryption or purchase success. The scoped decryption probe is
  explicitly diagnostic, not paid-proof, invoice or receipt recovery.
- Early incomplete harness attempts and pre-fix refusals remain retained. No
  failed attempt is relabeled as a pass.

`scripts/regtest-gate3-denial.py matrix PRIVATE_ROOT` automates the real GUI
pairing and approval/revoke/reject/regrant decisions against prepared isolated
services; it refuses a previously verified run. `verify` rechecks the saved
actual wire evidence, live isolated signer state and protected fingerprints
without triggering another operation. The completed evidence must not be
overwritten or the matrix rerun in this directory.

The isolated desktop, daemon and fixture services are stopped after final
verification; protected credentials and all evidence are retained. The original
shared infrastructure and existing bunker remain untouched. No protocol
specifications were changed and no changes were committed.
The exact tested executable is retained privately as `desktop-denial-verified`;
the normal desktop binary was subsequently rebuilt with its ordinary dev URL,
so it does not depend on the stopped private frontend at port 5184.

The later restoration/NWC/purchase follow-up is separate:
[regtest-gate3-integration.md](regtest-gate3-integration.md). It fixes and verifies
restored-session behavior without re-pairing; the denial evidence above remains
unchanged and is not relabeled as purchase or NWC durability evidence.
