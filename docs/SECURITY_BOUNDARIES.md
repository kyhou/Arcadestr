# Desktop security boundaries

Current behavior of credential storage, remote-signer secrets, debug automation,
payment authorization and external connections. Source is authoritative; update
this document with any change to these boundaries.

## OS credential store

The OS keyring (Secret Service, Keychain, Credential Manager) holds NIP-46
signer keys and connection URIs, the profile index, the last-active pointer,
NIP-49 backups and per-account NWC connections. Local-key accounts use the
file-based master key and do not depend on the keyring.

All keyring calls go through `core/src/credential_store.rs`:

- One worker thread serializes every platform call. Each caller waits at most
  5 s (automatic) or 120 s (explicit user Retry).
- A deadline does not cancel a call that already started. A queued request whose
  caller gave up is skipped; a running one finishes in the background and only
  updates the store status. Its value is discarded, so a late answer cannot
  change a newer session.
- After the store reports locked, a dismissed prompt or a missed deadline,
  automatic requests fail immediately (no prompt storm) until the user chooses
  Retry on the sign-in screen (`retry_credential_store`).
- An unavailable store is never treated as "profile removed": nothing stored is
  cleared or rewritten. Secrets are never stored anywhere else as a fallback.
- Session restore re-runs after a successful Retry and refuses to overwrite a
  login or account switch that happened meanwhile.

Non-secret profile metadata is cached in `profile_metadata_cache.json`, written
with mode 0600.

## NIP-46 connection secrets

`bunker://` and `nostrconnect://` URIs carry a `secret` parameter. It is stored
only in the keyring entry `arcadestr-auth/<signer>_uri`.

- Account labels default to `Remote signer npub1…`. A connection URI is never
  accepted as a label, including user-entered names.
- Legacy labels containing a URI or `secret=` are replaced when the profile
  index or cache is loaded. Credentials are not touched.
- Logs redact `secret` values; remote-signer approval URLs are logged without
  their query. `SavedProfile` debug output redacts the URI and the app key.
- The app's own freshly generated pairing URI is shown as a QR code by design.

## Debug automation bridge

Debug builds register `tauri-plugin-mcp-bridge` (webview script execution and
IPC). It listens on `127.0.0.1:9223`. Remote access requires
`ARCADESTR_MCP_BRIDGE_REMOTE=allow-all-interfaces` and should be used only in
a separately authorized setup. Release builds do not register the plugin
(`#[cfg(debug_assertions)]`).

## Payment authorization model

Two independently compiled Lightning dispatch controls are disabled by
default. See [Production payment release policy](PRODUCTION_PAYMENTS.md).

| | Buyer purchase | Recipient refund |
|---|---|---|
| Who pays | Buyer, for a game | Developer/operator who received principal of a failed purchase |
| Managed entry points | `start_split_purchase`, `next_split_invoice`, `pay_purchase_leg` | `prepare_split_refund`, `pay_split_refund` |
| Recovery (no dispatch) | `submit_split_payment`, `resume_split_payment`, startup recovery | `resume_split_refund`, `confirm_split_refund` |
| Authorization | Explicit approval of an immutable verified plan (quote, terms, recipients, amounts, network, wallet, limits) | Authenticated obligation from the processor addressed to the active identity; explicit per-payment confirmation |
| Binding | Original processor invoice per leg (hash, amount, signing key, attempt) | Original processor-prepared refund invoice (hash, amount, attempt, leg); principal must equal the obligation |
| Durability | Admission persisted as uncertain before dispatch; one leg at a time; halt on uncertainty | Durable NWC intent before dispatch; uncertain outcome is never repaid |
| Fees | `ProviderCeiling` needs a wallet-guaranteed bound (standard NWC has none); `UnknownFeesAcknowledged` (warning-only, no reserve or cap; reported fees recorded, missing ones unknown) needs separate build approval and explicit buyer acknowledgement stored in the approval; legacy `ReportedFees` approvals keep their reserve semantics but cannot be newly created | Disclosed, not bounded by the application; buyer routing fees are not part of refunded principal |
| Network | Invoice and wallet network must match the approval and compiled policy | Invoice and wallet network must match the independently compiled policy |

Managed IPC keeps BOLT11 material in the backend. Recovery may submit valid
proof from elsewhere, but cannot start a payment or grant ownership without a
validated receipt.

## External connections

With `ARCADESTR_RELAYS` and `ARCADESTR_BLOCK_DISCOVERY=1`, normal use connects
only to the configured relays and services. Exceptions by design:

- **Nostr Connect QR sign-in** uses the standard NIP-46 relays
  `wss://relay.nip46.com`, `wss://nrs.primal.net` and
  `wss://bucket.coracle.social` (shared with Amethyst/Amber), so a phone signer
  can reach the app. Relay discovery settings do not apply to this pairing.
- **`bunker://` connections** use the relays listed in the URI.
- **NWC** uses the relays listed in the wallet connection URI.
- **ADP Servers, Blossom servers and LNURL endpoints** are contacted as
  configured or as named by signed listings.
