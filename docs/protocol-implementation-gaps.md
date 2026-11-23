# ADP-01 implementation gaps (Arcadestr client)

**Status:** open. Consolidating and synchronizing `ADP-01.md` changed
documentation only and does **not** resolve any item here. Close an item only
when code and tests change, and link the change.

This record compares the Arcadestr client with the consolidated ADP-01 (`0.3.0`).
Each item was checked against the source cited when ADP-01 was consolidated.
Server-side gaps are tracked in `../adp-server/docs/protocol-implementation-gaps.md`.

## Verified discrepancies

| # | ADP-01 requirement | Current client behavior | Evidence |
|---|---|---|---|
| C1 | No clock-skew allowance may activate a `kind:30406` authorization or `kind:30404` attestation before `valid_from` (§No early activation). | `ATTESTATION_OBSERVER_SKEW_SECS = 300` lets `valid_from` sit up to 300 s in the reader's future. | `core/src/authorization.rs:75`, `:91`; described in `CODEBASE.md` §8.1 |
| C2 | `disputed` receipt tips retain ownership (§Receipt status under ADP). | `disputed` is grouped with `refunded` in the non-owning branch. | `core/src/ownership.rs:186` |
| C3 | Developer-signed grants may omit `source_event` (§Entitlement Grants). | The parsed grant type has a required `source_event: EventId`, so a grant without one cannot be represented. | `core/src/entitlements.rs:35`, `:102` |
| C4 | Public and active timed-access downloads are anonymous (§`GET /game/:game_coordinate`). | Native install resolves a signer before checking public/timed policy. The reference server also requires NIP-98 (server gap S1). | `docs/design-handoff/IMPLEMENTATION_PLAN.md` (code-audit note) |
| C5 | Implementations SHOULD prioritize the publisher's NIP-65 write relays (§Relay selection). | Not verified as implemented; no NIP-65 prioritization of listing reads was identified. | Needs confirmation |

## Not yet verified

- Terminal handling of `kind:30406` cancellation, campaign cancellation, and grant revocation (any successor invalid). `core/src/campaign.rs` and `core/src/entitlements.rs` need targeted tests.
- Typed handling of `503` from `/entitlement/claim`, `/purchase/confirm`, and `/game` as indeterminate rather than as a definitive failure.
- Canonical-hex enforcement on every ADP-defined hex input (see `.tmp/issues/file-hash-uppercase-interop.md`), with UUID and opaque identifiers left unconstrained.

## Documentation statements needing re-verification

- `CODEBASE.md` §NIP-103 Entitlements (Publisher authority) says delegated grants need both historical listing delegation and the `kind:30406` lifecycle. ADP-01 anchors historical validity only on `kind:30406`, and no listing-delegation check for grants was found in `core/src/entitlements.rs`. Confirm the code path, then correct the statement.
