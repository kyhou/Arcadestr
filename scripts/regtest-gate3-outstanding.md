# Gate 3 outstanding scenarios

## Outcome

| Scenario | Result |
| --- | --- |
| Real NIP-46 signer disconnection | **Scoped verification passed.** The user's existing `nip46` / `nak bunker` signer was manually stopped and restarted. An existing paid leg's encrypted proof was blocked while disconnected and acknowledged after reconnection, with zero recovery payments. This is not explicit authorization denial, NWC-bound durability, or complete NIP-46 purchase recovery. |
| Explicit NIP-46 authorization denial | **Passed for `sign_event` and `nip44_decrypt`.** An isolated persistent `ohstr/ncli` signer was paired through the actual GUI, permissions revoked, pending requests explicitly rejected and permissions granted again. Signed local-relay traffic confirms `denied: rejected`, not expiry/disconnection. No Lightning payments or existing journal/reservation changes. See [regtest-gate3-denial.md](regtest-gate3-denial.md). |
| NWC outage GUI matrix | **Safety verified** with the actual desktop, original NWC wallet/application and real wallet history. Controlled transport disconnection and reply loss are explicitly fault-injected. |
| Real machine reboot | **Verified after the user's manual reboot.** The current boot differs from the armed checkpoint and matches the post-reboot verification marker. Automatic application-start reconciliation recovered the original receipt, ownership and token without new dispatch admissions. |

**Gate 3 is not closed by this scoped result.** Real signer-disconnection
paid-leg recovery and explicit signing/decryption authorization denial are now
verified separately. NIP-46/NWC durability and complete NIP-46 purchase recovery
remain unverified, not passed. The machine reboot is
separately verified; application restarts are not machine reboot evidence.

Private evidence:
`~/.local/state/arcadestr-regtest/gate3-runs/1793337474483978484/`.
Do not publish that directory: it contains private database/account copies and
authenticated operational evidence. Earlier Gate 1/2/3 evidence is retained.

## 1. Remote signer

### Subsequent explicit-denial verification

See [regtest-gate3-denial.md](regtest-gate3-denial.md) for the complete automated
`ohstr/ncli` run, exact actual GUI messages, signed wire evidence and regression
results. That run uses a separate signer, keyring and application database and
does not alter the original `nak bunker` / `nip46` account or any historical
payment journal or reservation. Both refusal/regrant methods passed without
Lightning payments. The diagnostic decryption check is not purchase recovery.

### Subsequent existing-signer verification

The user subsequently connected an existing remote profile named `nip46`, with
public identity
`npub1xmt6ra5afgf8eeh65g5v42y0qkarq3qv73kfn6ffjdq73j2g6d8s0ggn5h`.
The running private desktop and saved profile matched this identity. Its existing
`nak bunker` automatically authorizes its existing clients; it does not provide
manual per-request approval. The user explicitly authorized initial requests
under that existing policy. No signer credentials or authorized-client list were
replaced, re-paired, or modified.

The initial non-paying GUI quote operation exposed a confirmed defect:
`LazyNip46Signer` rejected NIP-44 operations as unsupported. A focused regression
failed with that exact error before the fix and passed after NIP-44 operations
were forwarded to the existing connected backend. The original post-reboot
executable was retained before rebuilding; historical reboot evidence was not
rerun or relabeled.

The user authorized at most two 100-sat setup transfers from existing Bob, with
regtest destination and self-payment guards. Setup produced:

- One **100-sat successful transfer**, zero routing fee, to existing Alice LND.
- One **failed 100-sat attempt** to existing Carol, conclusively recorded as
  `FAILURE_REASON_NO_ROUTE`; no second payment occurred and it was not retried.
- An earlier CLI option-parser rejection sent no payment. The user explicitly
  authorized correcting that command against the original invoice after a
  complete counted wallet-history check proved no matching payment. The failed
  admission and original invoice were retained separately.
- The CLI's interactive success output also caused a setup parser stop. The
  original successful payment's preimage was recovered from complete wallet
  history, cryptographically checked, and submitted without redispatch.
- Both original invoice attempts and the conservative **200,000-msat setup
  reservation remain retained**. No replacement invoice was created. The new
  partially funded purchase contains 100 sats paid and 100 sats unpaid; it is
  not a completed purchase. The earlier unresolved Alice-to-Bob 100-sat refund
  obligation is separate and unchanged.

The purchase belongs to the NIP-46 buyer, not Alice. These explicit setup
transfers used Bob's LND CLI, not NWC. The user explicitly accepted preserving
the original absence of an NWC binding; no NWC account/application association
was invented. Bob's existing LND identity, original buyer, invoice, encrypted
proof and payment hash were verified instead.

Because setup was partial, the user approved a **separate private checkpoint**:
the existing database and paid proof were copied, and only the copy's paid-leg
local acknowledgement was interrupted. Original source database/invoice digests
remained unchanged. This controlled checkpoint is distinct from the genuine
human-operated signer stop/restart:

1. The user manually stopped the existing bunker. The actual private desktop
   started against the checkpoint; manual GUI recovery reported
   `Invoice checkpoint decryption unavailable`. The original encrypted proof
   remained pending and byte-identical.
2. The user restarted the same bunker with unchanged credentials/authorization.
   Actual GUI recovery reported `Original payment/proof checked. No payment was
   dispatched.` The copied paid-leg acknowledgement returned to `settled`.
3. Complete counted Bob payment history was unchanged throughout both phases;
   recovery added **zero LND payments**, **zero NWC dispatch admissions** and
   **zero replacement invoices**. Saved remote-profile metadata and the original
   source database/invoice remained unchanged.
4. The quote remained `active` and partially funded. There were **zero receipts,
   zero ownership records and zero download tokens** for this NIP-46 buyer.

Evidence is under the private run's
`nip46-recovery-setup3/signer-checkpoint/`, including `checkpoint.json`,
`disconnected-verification.json`, `reconnected-verification.json` and actual
GUI DOM/screenshots. Failed server-only setup attempts are retained separately;
they created no purchases or payments. Do not rerun paid setup commands.

Validation: the focused native signer suite passed **10 tests**; Gate 3 Python
harness suites passed **17 tests**, including partial/missing wallet-history
rejection, unknown status, self-payment, destination, amount and duplicate guards.
The desktop and no-payment fixture server built successfully. The isolated
desktop/driver were closed and only the new fixture server was stopped after
verification; the user's bunker and pre-existing shared services were untouched.

**Scope:** actual NIP-46 signer-disconnection recovery of an existing paid leg.
Not explicit authorization denial, NWC payment durability under NIP-46, or
complete NIP-46 purchase/receipt/access recovery. That disconnection run did not
mark those scenarios passed or close Gate 3. Explicit denial was subsequently
verified by the separate ncli run above; the other boundaries remain unverified.

### Earlier unavailable-signer inspection (historical)

Read-only inspection found two `Local` account records and no existing NIP-46
profile or running signer process. The user answered **Not available**. No remote
GUI verification, unlock/reconnect claim, or simulated remote-signer success is
reported. Dispatches and lookups attributable to this unrun scenario: **0 / 0**.

Testing requires an existing remote signer supplied/unlocked through its normal
GUI, without replacing the current local identities or disclosing connection
secrets in chat. Current local-key results do not substitute for that requirement.

## 2. NWC outage verification

Two new isolated partial orders were funded by existing Bob, paying existing
Alice 100 sats each. Neither partial order obtained ownership or a download token.
The original successful purchases and already-verified refund scenarios were not
paid again or reset.

The laboratory proxy disconnects only the test desktop's NWC transport or drops
encrypted response events. Shared relay/Hub services remain running. It neither
decrypts payloads nor changes wallet credentials. A debug-only client override
requires explicit opt-in and loopback original/proxy relays, rejects unsafe URLs,
and leaves `payment_identity()` unchanged. Release builds ignore the override.
This is **controlled network fault injection with real GUI/wallet traffic**, not
evidence that the shared Alby Hub service itself went down.

An attempted network-namespace harness failed in the host's GTK image loader.
Those failed-launch logs were retained; its temporary processes were stopped.
No global sandbox/credential configuration was changed. The final tests used the
normal desktop process with the restricted transport override.

### Observations

| Fault | Actual observation |
| --- | --- |
| Before forwarding a payment | Explicit GUI confirmation created a durable dispatch admission, but the offline proxy forwarded **zero** NWC commands. Wallet history has no matching payment. The invoice/attempt is retained and the UI offers read-only recovery, not repayment. |
| After wallet dispatch | One real 100-sat refund succeeded in the wallet. Its encrypted reply was dropped. Local proof was initially absent and the GUI correctly showed uncertainty and no pay button. |
| During refund reconciliation | Disconnecting transport and repeating recovery did not dispatch another payment. |
| Offline application restart | Both never-forwarded and paid-but-uncertain states remained durable. Startup performed recovery only; it never repaid either invoice. |
| Reconnection | Read-only wallet lookup recovered the paid refund's original proof and authenticated confirmation. The never-forwarded payment remains unresolved/NOT_FOUND and is not retried. |

Final **journal admissions** at the pre-reboot checkpoint:

| New outage fixture | Dispatch admissions | Lookup admissions | Matching wallet payments | Final state |
| --- | ---: | ---: | ---: | --- |
| Before forwarding | 1 | 700 | 0 | uncertain; original obligation retained |
| Wallet paid, response lost | 1 | 1 | 1 | verified; encrypted proof persisted |

`dispatch_count` is written before transport; it must not be confused with a
forwarded wallet command. Across the proxy there were 465 forwarded encrypted
request events, two dropped responses and 955 refused connections. The first
outage forwarded zero payment commands. The large lookup count includes the
existing periodic read-only scan while GUI/build work remained open; failed
connections consume lookup admissions without forwarding a request. Wallet
history independently confirms no duplicate payment.

**100 sats remain owed by Alice to Bob** in the deliberately unresolved fixture.
This is not a completed refund. Its invoice, journal and conservative 600-sat
run reservation remain intact; no replacement invoice or automatic repayment
was attempted. Channel balances match that one remaining principal exactly;
there are no pending HTLCs. Historical reservations were not cleared.

Key evidence includes:

- `outage-before-dispatch-screen.{json,png}` and `before-dispatch-outage-audit.json`
- `after-dispatch-response-lost-screen.{json,png}` and its audit
- `paid-uncertain-restart-offline-screen.{json,png}`
- `nwc-reconnected-recovered-gui.json`
- `outstanding-verification-report.json`, `network-audit-final-pre-reboot.json`

Some Wayland captures timed out; corresponding JSON explicitly records that a
screenshot is unavailable while retaining actual GUI DOM evidence.

### Confirmed defect fixed

Background recovery verified the paid refund in SQLite/server state, but the
open recipient card remained `UNCERTAIN`. `recipient_refunds.rs` now subscribes
to account-scoped successful recovery updates and rereads authenticated metadata.
It does not prepare/pay from an event; ignores foreign accounts, busy/unsent
processor contexts and empty recoveries; clears the loaded processor on account
change; and removes its listener on teardown.

Focused tests cover those guards. The actual desktop event regression replays
one stale **read result** and sends synthetic recovery notifications: a foreign
account is ignored, and the matching account reloads the real authenticated
backend into `VERIFIED`. That regression is explicitly simulated notification/
read replay, not an additional wallet payment or another real outage. See
`ui-refresh-regression-classification.json`, `ui-refresh-wrong-account-event.json`
and `ui-refresh-event-regression.json`.

## 3. Manual reboot handoff and verified outcome

The actual private desktop was closed before arming. The checkpoint retains the
original paid buyer proof but deliberately omits receipt/ownership/token progress
**only in the private client copy**. The original signed evidence and pre-fault
backup remain intact. The genuine unresolved refund intent also remains durable.

Before arming, checks recorded existing container IDs, four LND identities,
balances, no pending HTLCs, protected signer/config fingerprints, wallet/app
binding, every reservation's digest, the executable digest and SQLite durability.
Private signer copies are separately sealed; missing storage is rejected, not
regenerated. Existing-only fixture-server restoration was tested without creating
quotes, invoices or payments.

The stale regtest tip was advanced **175 → 176**, exactly one block with explicit
user authorization after checking HTLCs, pending channels and mempool were empty.
The existing coinbase destination was reused. All LND nodes returned to synced
status. This did not reboot the machine or pay an invoice.

### Human steps

1. Save unrelated work and review the retained 100-sat obligation/reservation.
2. Only after your explicit authorization, use your desktop's normal Restart
   control. **No tool or script here invokes reboot/shutdown.**
3. After login, unlock the existing OS keyring and existing Alby Hub regtest
   profile if requested. If Docker/Polar needs attention, start the existing
   network/containers; never create a replacement network, wallet or identity.
4. From the Arcadestr repository, run:

```sh
python3 -B scripts/regtest-gate3-remaining.py after-reboot \
  --root "$HOME/.local/state/arcadestr-regtest/gate3-runs/1793337474483978484"
```

Alternatively, the interactive handoff is:

```sh
bash scripts/regtest-gate3-reboot-wizard.sh \
  "$HOME/.local/state/arcadestr-regtest/gate3-runs/1793337474483978484"
```

The wizard stops on the same boot and never requests a reboot. The verification
command refuses the original boot ID before starting anything. After a real boot
change it validates durable/protected state, starts only existing infrastructure
and fixture databases, launches the private desktop, and waits for its automatic
read-only startup recovery. It checks original receipt identity, ownership/token,
NWC wallet/app identity, unchanged dispatch admissions, wallet histories, refund
GUI states and the recovered download hash. No OS autostart registration is
installed: the command launches the existing app, whose reconciliation is automatic.

Do not rebuild the recorded executable or open the private test copy before
verification. If readiness, keyring/signer, ports, fingerprints or balances fail,
stop and retain evidence. Never repay, clear a reservation, or mine blindly.
An interrupted post-reboot verification can resume against its recorded boot/
checkpoint; it does not create another paid run.

The initial same-boot attempt was rejected. Subsequently, the user manually
rebooted and ran the command above successfully. Independent read-only artifact
inspection confirmed:

- The current kernel boot ID differs from the checkpoint and matches the
  post-reboot verification marker.
- Protected identities/configuration, existing infrastructure and reservations
  were preserved; the original wallet/application reconnected.
- Automatic application-start recovery restored the identical original receipt,
  ownership and download token. The recovered download matched the expected
  30-byte payload hash.
- New payment admissions: **0**. Refund wallet histories remained **0 / 1**.
  Both original refund dispatch-admission counters remained **1 / 1**.
- At the verification snapshot, refund lookup admissions were **701 / 1**
  (pre-reboot: **700 / 1**). Further periodic read-only lookups of the unresolved
  case may increase that counter; they are not new payment dispatches.
- Actual post-reboot GUI evidence is retained in
  `post-reboot-refund-0.{json,png}` and `post-reboot-refund-1.{json,png}`. The
  unresolved refund has no repay button; the paid refund is verified. The latter
  capture also shows the purchase completed with active access.

`reboot-verification.json` now records `machine_reboot_verified=true` and
`gate3_closed=false`. The intentionally unresolved 100-sat obligation and its
reservation remain intact. Do not repeat the successful reboot scenario; the
then-remaining acceptance blocker was the unavailable real NIP-46 remote signer.
The subsequent scoped signer-disconnection result and its untested boundaries
are recorded in section 1 above; the original reboot artifact is historical.

## Focused validation

- NWC client tests: 13 passed, including opt-in/loopback-only transport and stable
  wallet/application identity.
- Account-scoped recovery refresh test passed; current WASM checks/build passed.
- Harness tests cover same-boot rejection without side effects, response-only
  filtering, private-copy boundaries, encrypted BLOB fingerprints and refusal to
  regenerate missing signer storage.
- Bash syntax, Python syntax, workspace formatting and diff checks passed.

No protocol specification was modified and no changes were committed. The
subsequent real signer-disconnection result is reported separately in section 1;
explicit authorization denial is now verified separately in
[regtest-gate3-denial.md](regtest-gate3-denial.md). Complete NIP-46 recovery
remains unverified.
