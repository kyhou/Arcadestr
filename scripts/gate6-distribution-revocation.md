# Gate 6 Phase 5 — Distribution and revocation management

Scope: G6-05, G6-10 and G6-11. Gate 6 remains **IN PROGRESS**. Phase 6 is not
included. All work uses existing ADP/NIP-103 protocol semantics; ADP Server is
unchanged. No live provisioning, publications, uploads or payments are used for
validation, and no changes are committed.

## Multi-operator setup

The existing listing publication pipeline resolves every selected server's public
operator identity before provisioning. Explicit per-URL receipt/grant roles map to
the existing `issue-receipt`, `issue-grant` and `upload-build` capabilities. A
distribution-only operator receives upload capability, not issuance authority.
The primary provider's existing receipt/grant defaults remain visible in the
editor; other selected servers default to distribution-only.

Identical URLs are deduplicated for provisioning. Several URLs advertising the
same operator are grouped into one held-key/scope provisioning action; their roles
must agree. Conflicting aliases are rejected instead of silently expanding an
operator's capabilities. Each distinct operator is provisioned independently,
with its scope, response, key and exact signed authorization retained in Phase 4's
existing journal. Fulfillment private keys remain exclusively on the operator.
Each URL still has independent upload/registration evidence.

New operators may be authorized on an existing listing only through the explicit
authority-setup checkbox. Existing signed references are preserved. Current
authority with the requested capabilities is reused without another provisioning
POST; cancelled, stale or insufficient authority is not renewed or expanded.
Commercial terms still use Phase 3 review, not implicit acceptance during setup.
Role selections are saved in local drafts, but draft restore does not restore
fresh authority-setup consent.

Partial setup stops safely without rolling back already-published roots or
discarding provisioning responses. Retry uses the original operation. Missing
provisioning/upload responses remain unknown; Phase 4's separate explicit
same-scope/pinned-artifact retry acknowledgement is still required. No new remote
inventory or rollback API is assumed. A retained public response is historical;
current attestation and authority checks remain necessary before further work.

## Cancellation and individual revocation

The managed-game panel offers separate actions for fulfillment-authority
cancellation and individual entitlement-grant revocation. Listed authority roots
can be selected directly; a specific grant is targeted by its root event ID.
Review resolves current relay evidence and displays the target, recipient/operator
and exact chain tip. Mutation requires explicit consent and the active developer
signer, with account/session fencing and the existing per-coordinate lock.

Authority cancellation publishes the existing kind:30406 cancellation transition:
the original terms are unchanged, the original root is the predecessor, and its
timestamp is prospective. It does **not** call `/provision/revoke`, delete a key,
remove a listing reference or retire an operator. Receipts and grants issued
before cancellation retain their historical authorization checks. Issuance at or
after cancellation is not authorized.

Individual revocation publishes the existing NIP-103 entitlement transition,
preserving grant ID, recipient, coordinate, campaign source and authorization
reference. Historical campaign and authorization evidence is validated with the
existing buyer-side resolvers. Only that entitlement becomes revoked; independent
grants, paid receipts and ownership from other proofs are not revoked. The
existing entitlement repository retains the original and revocation events.

Campaign cancellation stops future claims. Listing removal changes listing
discovery. Operator offboarding/key retirement is a distinct unresolved workflow.
None of these is inferred from, or silently performed by, credential revocation.

## Recovery and evidence boundaries

Cancellation/revocation uses the existing session-fenced signer, relay publication
helper and publisher journal. Unsigned intent and exact signed bytes survive a
restart. Multiple operators no longer share one authorization-signature slot:
operator identity and transition status distinguish those checkpoints. The prior
single-authority checkpoint remains readable for its matching identity.

Current chain evidence is reconciled before signing and again after a remote
signer returns. Stale tips, terminal chains and competing valid transitions stop
the action. If the exact retained transition is already visible, recovery records
that result rather than publishing a replacement. Unknown relay outcomes require
explicit original-operation recovery; changing the operation ID cannot bypass a
pending transition for the same credential. Startup and read-only review do not
sign or execute recovery. Observations are selected-relay evidence, not global
consensus or guaranteed complete network history.

Distribution visibility is read from account-scoped Phase 4 operation/checkpoint
records, separately for each URL and operation. It distinguishes declared,
provisioned, upload-pending, registration-acknowledged, failed and unknown states.
Unacknowledged external writes remain unknown even when a transport error was
observed. A historical upload acknowledgement never becomes a current download
availability claim. Neither a health check nor a receipt-issuing role proves that
an operator currently serves the archive. Different operations/artifacts are not
merged into one synthetic success state. Imported listings without local
checkpoints are declared with unknown registration/availability.

## Validation

Completed regression runs: 120 core integration tests covering publisher recovery,
commercial configuration, authorization, entitlement boundaries/repositories,
quotes, receipts, purchase execution and settlement recovery; 97 core ADP unit
tests; 5 native Store Page tests; 18 archive-selection tests; and 5 Gate 5 desktop
tests. All passed with no failures. Live tests were not run.

Focused suites passed: 6 management integration tests, 85 native ADP command
tests, 6 native journal/replay tests and 172 app publisher tests. They cover
multiple operators, conflicting alias roles/identities, least-privilege roots,
partial provisioning and explicit uncertain retries, stale/cancelled authority,
revocation boundaries, immutable pending transitions, restart recovery, and
account isolation. These are isolated databases, synthetic public events and
mocked HTTP/signers, not live service acceptance or process-kill testing.

Native checks, `cargo check -p arcadestr-app --target wasm32-unknown-unknown`,
and an isolated Trunk debug build passed. The real WASM management panel passed
an allowlisted mock-IPC browser regression: historical state disclosures,
separate authority/grant actions, exact reviewed consent, no blind retry,
account-switch isolation of delayed review/mutation responses and narrow-view
layout. Existing Phase 4 recovery, receipt-portability and Gate 4 browser
regressions passed against that same frontend. No live signing or services were
used by those browser tests.

Relevant Rust formatting and `git diff --check` passed. Compiler warnings remain;
successful checks are not a warning-free claim. The final management, native ADP,
journal, app publisher and native-check reruns all passed; their logs use the
`gate6-phase5-final-*` prefix.
Logs and mocked browser evidence: `/tmp/opencode/gate6-phase5-*`.

Phase 5 is implemented and locally validated within these limits. Gate 6 remains
**IN PROGRESS**. Phase 6 has not started. Unrelated working-tree changes and
previous milestone evidence are preserved; no commit was created.
