# Production payment release policy

The default desktop build has buyer and refund payments **disabled**. Compile
the release policy into the native desktop, not the WebView. No environment
variable, profile setting, or IPC command can activate dispatch.

| Desktop Cargo feature | Effect |
|---|---|
| `production-buyer-payments` | Mainnet buyer dispatch; strict fee ceiling only by default. |
| `production-refund-payments` | Mainnet recipient dispatch, independent of the buyer feature. |
| `production-reported-fees` | Enables buyer dispatch and permits reported-fee approvals. Add only after separate risk acceptance. |

Production features are refused in debug builds. Regtest features are refused
in release builds; mixing any production and regtest payment features is a
compile error. The regtest-only pin is not a production setting.

## Release review and build

1. Record source revisions, lockfile, Rust/Trunk/Tauri versions, exact feature
   list, approver, and artifact hash. Record buyer and refund activation
   independently. Do not enable reported fees without a signed-off release
   decision that explicitly accepts the residual risk below. That decision is
   **pending**, not implied by the regtest validations or this document.
2. From the workspace root with a reviewed checkout and Linux dependencies,
   verify the default policy and compile without producing an enabled desktop:

   ```sh
   cargo run --locked -p arcadestr-xtask -- frontend-build --release
   cargo build --locked --release -p arcadestr-desktop --features custom-protocol
   cargo check --locked --release -p arcadestr-desktop \
     --features custom-protocol,production-buyer-payments,production-refund-payments,production-reported-fees
   ```

3. Only after separate release authorization, package in `desktop/` using
   `cargo tauri build --bundles deb --features <reviewed-comma-separated-features> -- --locked`.
   The Tauri build embeds the freshly stamped frontend. Verify `get_payment_activation`
   against the reviewed settings in a disposable profile. Do not test by spending.
   The paid release must name a usable buyer wallet policy and an authorized
   refund execution path; a strict-only artifact without a capable wallet is
   not a functioning paid release.

## Fee and execution guarantees

- **Strict ceiling:** dispatch requires a demonstrable, enforceable wallet
  guarantee for the exact invoice, within the approval's remaining reserve.
  Standard NIP-47 does not offer this; Arcadestr's standard NWC adapter does not
  advertise a ceiling. A wallet's balance, connection budget, or method list
  is not evidence of enforcement. Strict mode therefore refuses standard NWC
  dispatch; Arcadestr does not modify Alby Hub or invent a wallet capability.
- **Reported-fee accounting** is the supported option for standard NWC.
  If compiled in, the buyer must explicitly choose and acknowledge the mode
  before approving the immutable plan. This is not a wallet-enforced fee
  ceiling. The reserve is an
  accounting budget, not a maximum wallet charge. Arcadestr checks remaining
  spend before sending and stops further dispatch on a reported overrun. It
  cannot cap an in-flight fee, undo it, or independently attest to a wallet's
  fee report. Missing fee evidence leaves the leg unresolved for safe recovery.
- Arcadestr requires a matching, recognized wallet network from `get_info`
  and standard payment and lookup methods. Before dispatch it checks account
  and wallet generation, approval, exact persisted invoice and commercial
  terms, and current listing revision. It admits and journals the original
  payment before sending. Unknown outcomes are lookup-only, never a second
  send on the same attempt.
- Managed buyer and refund IPC return display metadata, not payment material.
  Legacy invoice creation, raw-invoice dispatch, and legacy invoice proof
  confirmation are not registered desktop commands. ADP Server requires
  quote-bound proof for split listings and rejects standalone BOLT11/preimage
  confirmations in release builds. Paid operators must use the hardened
  server; configure `ADP_LEGACY_PAID_SALES=decline` if legacy zaps are unwanted.
  An invoice obtained independently from a processor or another client may
  still be paid externally. Manual proof must match the original attempt and
  a verified receipt is required for ownership. A valid externally supplied
  proof without verified local fee accounting halts further app dispatch for
  that plan; it is not a way to replenish the fee reserve.

## Partial settlement

A partially paid purchase without a validated receipt does not grant
ownership. The processor exposes principal refund obligations for the
settled legs; unpaid legs are not refunded. Recipient software must use the
original obligation and invoice. With the independent desktop feature enabled,
a developer or operator can sign in with the obligated recipient identity,
prepare the original refund, and explicitly authorize a journaled payment from
that account's wallet. On uncertainty, check the original payment only.
The ADP `adp-refunds` operator CLI can review, prepare, and confirm externally
executed operator refunds; it does not send payments or substitute for a
durable wallet-side payment record. Do not send from both paths. Refund routing
 fees are paid by the sender and are not guaranteed capped by standard NWC.

## Production ADP credentials

Before deployment, independently generate the production operator signing key
and master encryption key. Give the production service a designated, access-
controlled secret source outside all checkouts and synchronized directories.
Require that source on startup; do not fall back to checkout `.env` files,
regtest profiles, or ambient development variables. Verify provisioning
provenance and fail-closed loading with a disposable configuration; do not
embed a list of test keys or log credential values.

The historically synchronized ADP credentials in Gate 8 were test-only.
Keep their containment controls, identities, receipts and databases intact.
Rotation, remote deletion and historical-key migration for those test
identities are not prerequisites for production deployment.
