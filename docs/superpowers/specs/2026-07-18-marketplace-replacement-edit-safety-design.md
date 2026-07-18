# Marketplace Replacement and Edit Safety Design

## Goal

Keep NIP-99 replaceable listings, persisted ADP fulfillment metadata, and publisher campaign state consistent across relay ingestion, SQLite, application stores, and edit flows.

## Replaceable Event Ordering

One core comparator defines replacement order everywhere:

- A candidate with greater `created_at` wins.
- When timestamps are equal, the lexicographically lower event ID wins.
- A candidate with a known event ID wins a timestamp tie against an entry without an event ID so legacy cache rows can be enriched.
- An entry with a known event ID rejects a timestamp-tied candidate without one.
- Two entries without event IDs do not replace each other on a timestamp tie.

Relay ingestion, marketplace stores, publisher listing selection, and SQLite upserts must reject stale candidates immediately. SQLite must enforce the timestamp and event-ID tie-break in its conflict-update guard rather than accepting stale data for later repair.

## Fulfillment Edit Safety

Editing an existing listing preserves `fulfillment_valid_from` and `fulfillment_revoked_at`. A revoked key remains revoked; ordinary metadata or artifact edits cannot silently reactivate it. Reactivation requires an explicit future key-rotation or reprovisioning flow.

Delegated operator restoration uses the fulfillment key and listing scope to query local provisioning records. Exactly one match prefills the operator. Zero or multiple matches leave the operator unresolved and require explicit publisher selection. Distribution-server order is never used as an operator heuristic.

An unchanged artifact may reuse a validated existing SHA-256 hash without requiring file selection or upload. Malformed legacy hashes are not reused and are rendered as a safe placeholder rather than sliced at assumed byte boundaries.

## Campaign State

After a successful campaign-pointer mutation, the current local listing is updated immediately with the returned listing event ID and pointer change. Campaign creation and cancellation similarly update local campaign/pointer state where the command result is authoritative. Relay discovery later reconciles the optimistic local state.

If campaign publication succeeds but pointer publication fails, the UI preserves the campaign result, reports the pointer error, and does not claim that local pointer state changed.

## Verification

Focused regression tests cover:

- Newer and stale replaceable events in both arrival orders.
- Equal timestamps with lexicographically lower and higher event IDs.
- SQLite rejecting stale and tie-break-losing upserts.
- Store, loader, and publisher-management ordering consistency.
- Fulfillment validity and revocation preservation during edits.
- Unique, missing, and ambiguous local provisioning matches.
- Malformed artifact hashes.
- Local campaign pointer updates after successful mutations.

The final verification includes core, desktop, frontend, and WASM checks plus desktop UI validation.
