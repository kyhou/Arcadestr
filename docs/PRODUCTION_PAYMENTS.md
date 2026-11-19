# Production payment release policy

The default desktop build has buyer and refund payments **disabled**. Compile
the release policy into the native desktop, not the WebView. No environment
variable, profile setting, or IPC command can activate dispatch.

| Desktop Cargo feature | Effect |
|---|---|
| `production-buyer-payments` | Mainnet buyer dispatch; strict wallet-enforced fee ceiling only. |
| `production-refund-payments` | Mainnet recipient dispatch, independent of the buyer feature. |
| `production-reported-fees` | Enables buyer dispatch and permits the standard-NWC **warning-only** policy (unknown routing fees, explicitly acknowledged by the buyer). The name is historical; it no longer creates reserve-based approvals. |

Production features are refused in debug builds. Regtest features are refused
in release builds; mixing any production and regtest payment features is a
compile error. The regtest-only pin is not a production setting. Regtest
test ceilings count principal plus any fee reserve and reported fees; they are
test-build limits, not a production fee cap.

## Release review and build

1. Record source revisions, lockfile, Rust/Trunk/Tauri versions, exact feature
   list, approver, and artifact hash. Record buyer and refund activation
   independently. Record the fee-policy risk acceptance below in the release
   record. The release and deployment decision remains **pending**; it is not
   implied by the regtest validations, the risk acceptance, or this document.

### Residual routing-fee risk acceptance (warning-only standard NWC)

The release owner has **accepted** the residual routing-fee risk of the
warning-only standard-NWC policy (`unknown_fees_acknowledged`, enabled by
`production-reported-fees`) for v0.1. The acceptance covers:

- Lightning routing fees are chosen by the buyer's wallet, are unknown before
  payment, and are not capped or estimated by Arcadestr; a buyer's final cost
  may exceed the game price.
- Routing fees may be unreported by the wallet and then remain unknown.
- Routing fees are not refunded: partial-settlement refunds return principal
  only, and recipients pay their own refund routing fees.

This acceptance does not cover any other residual risk, does not relax strict
mode, mainnet-only enforcement, managed IPC, or uncertain-outcome recovery,
and **does not authorize production deployment** or publication of an
artifact.
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

- **Strict ceiling** (`provider_ceiling`): dispatch requires a demonstrable,
  enforceable wallet guarantee for the exact invoice, within the approval's
  numerical fee reserve and total. Standard NIP-47 does not offer this;
  Arcadestr's standard NWC adapter does not advertise a ceiling. A wallet's
  balance, connection budget, or method list is not evidence of enforcement.
  Strict mode therefore refuses standard NWC approval and dispatch; Arcadestr
  does not modify Alby Hub or invent a wallet capability. The executor keeps
  strict support for a future capable wallet adapter.
- **Warning-only** (`unknown_fees_acknowledged`) is the supported option for
  standard NWC. Checkout shows the exact game price, the recipient split, and
  this warning: "Your Lightning wallet may charge additional network fees.
  Their amount is not known before payment, so your final cost may exceed the
  game price. Network fees may not be recoverable if a refund is necessary."
  The buyer must tick "I understand that additional Lightning fees may apply."
  It is never preselected. Approval is refused without it. The immutable
  approval stores the policy and the acknowledged notice version
  (`arcadestr/unknown-lightning-routing-fees/v1`). It has no fee estimate,
  fee reserve, or maximum total. Approval does not send a payment.
- Under the warning-only policy, the wallet chooses routing fees. Arcadestr
  has no hidden fee cap and never blocks a later leg because of fees. When the
  wallet reports a fee in `pay_invoice` (or a later read-only
  `lookup_invoice`), Arcadestr records it. A missing report stays **unknown**,
  never zero. A wallet proof whose preimage matches the original invoice is a
  paid leg whether or not a fee is reported. Missing fee data never makes a
  payment uncertain or justifies a second send. Proof is validated against the
  invoice hash. Purchase details show "Routing fees paid: …" or "Routing fees
  not reported". Routing fees are separate from the game price, publisher
  revenue, and ADP operator legs.
- **Legacy reserve accounting** (`reported_fees`) is retired for new
  approvals; the desktop refuses to create one. Approvals persisted earlier
  keep their original semantics: reserve checks, halting on a reported
  overrun, and treating a missing fee report as unresolved for recovery.
  Persisted approval snapshots are not rewritten or reinterpreted.
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
  proof for a leg the app did not dispatch and verify halts further app
  dispatch for that plan under every policy. Under reserve policies, a proof
  without verified local fee accounting also halts dispatch. It is not a way
  to replenish a fee reserve.

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
durable wallet-side payment record. Do not send from both paths. Refunds return
principal only: the buyer's original routing fees are not reimbursed. Refund
routing fees are paid by the sender and are not capped by standard NWC.

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
