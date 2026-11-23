# ADP-01 implementation gaps (Arcadestr client)

**Status:** tracked. Close an item only when code changes **and** a targeted test
passes. Consolidating or synchronizing `ADP-01.md` resolves nothing here.

This record compares the Arcadestr client with the consolidated ADP-01 (`0.3.0`).
Server-side gaps are tracked in `../adp-server/docs/protocol-implementation-gaps.md`.

## Open

| # | ADP-01 requirement | Current client behavior | Evidence |
|---|---|---|---|
| C5 | Implementations SHOULD prioritize the publisher's NIP-65 write relays (§Relay selection). This is an optimization, never an authorization requirement. | No NIP-65 prioritization of listing reads was identified. | Needs confirmation |
| C6 | Consumers MUST reject non-lowercase hex (§Canonical Hex Encoding). | Shared `is_sha256_hex` accepts uppercase; `file_hash` from reused listing metadata can be emitted unchanged. Stricter helpers exist (`blossom::is_lowercase_sha256_hex`, `nostr::is_lower_hex_with_len`) but are not used everywhere. | `core/src/hash_validation.rs`; `.tmp/issues/file-hash-uppercase-interop.md` |
| C7 | Duplicate `acquisition` tags make the policy invalid, granting no access (§Malformed acquisition policy). | The listing parser keeps the **first** `acquisition` tag and ignores the rest (`Some("acquisition") if !acquisition_seen`), so `public` followed by a duplicate still parses as `Public`. The server rejects this, so it is a client display/eligibility mismatch, not a server bypass. | `core/src/marketplace.rs` (listing parser) |
| C8 | A gated listing has **no** `acquisition` tag (§Acquisition policy). | The publisher emits `["acquisition", "gated"]`. ADP-01 classifies an unknown mode as invalid, which still denies access, so behavior is safe, but the emitted form is non-canonical. | `core/src/adp_publish.rs` (`AcquisitionPolicy::Gated` arm) |
| C10 | Developer-signed grants without a campaign can be revoked by the developer (§Grant revocation authority). | The publisher revocation flow verifies against campaign evidence, so it refuses campaignless grants with an explicit "not supported yet" error (fails closed). | `desktop/src/publisher_management.rs` |
| C11 | A buyer's durable credentials are discoverable by `#p` + `#a` (§Portable grant verification). | The client learns grants only from its own claims and known-grant reconciliation; it cannot yet discover a developer-issued grant it did not claim. Storage and validation for campaignless grants exist (`ingest_campaignless_event`). | `core/src/entitlement_reconciliation.rs` |

## Closed

| # | Resolution | Tests |
|---|---|---|
| C1 | Removed `ATTESTATION_OBSERVER_SKEW_SECS` and `allows_new_operations_observed_at`. `kind:30404` activation and revocation are exact. A reader whose clock trails the operator gets a distinct, retryable "not active yet" error instead of a revocation message. | `core/tests/authorization.rs` exact-boundary tests (`--features native`); `desktop/src/adp_commands.rs`: `a_reader_trailing_the_operator_clock_gets_a_retryable_not_yet_active_error` |
| C2 | `disputed` receipt tips retain ownership and stay in Library; `refunded` does not. | `core/src/ownership.rs`: `a_disputed_purchase_retains_ownership_and_stays_in_library`, `a_refunded_purchase_does_not_retain_ownership` |
| C3 | `ParsedEntitlementGrant::source_event` is now `Option<EventId>`. `validate_campaignless_entitlement` and `EntitlementsRepository::ingest_campaignless_event` accept publisher-signed grants without a campaign and reject delegated or campaign-anchored ones on that path. Claim responses must still carry the exact claimed campaign. The `NOT NULL` `campaign_root_id` column stores `""` for campaignless grants, and history reports `campaign_id: None`. | `core/src/ownership.rs`: `a_publisher_grant_without_a_campaign_is_owned_and_listed`, `a_campaignless_grant_not_signed_by_the_publisher_is_rejected`, `a_campaign_anchored_grant_is_rejected_by_the_campaignless_path`, `a_publisher_revocation_ends_a_campaignless_grant` |
| C4 | `install_game` no longer requires a signer. Signed out, it proceeds only if the freshly fetched, signature-verified listing grants public or active timed access, and downloads with the new `DownloadAuth::Anonymous` (no credential). Otherwise it fails with "sign in" **before** any download. Tokens and ownership remain account-scoped. **UI note:** `game_detail.rs` still asks the user to sign in before installing a public game, because that flow adds it to Library. That is a product decision and is unchanged. | `desktop/src/main.rs` install tests: `signed_out_install_of_a_public_listing_downloads_without_credentials` (asserts no `Authorization` header), `signed_out_install_during_active_timed_access_is_allowed`, `signed_out_install_after_timed_access_ends_is_refused_before_download`, `signed_out_install_of_a_gated_listing_is_refused_before_download` |
| C9 | Receipt status updates not signed by the developer are dropped from the chain, and are not persisted as evidence by receipt portability, so they cannot change the status or invalidate the chain. Found during this work; it was not on the original list. | `core/src/ownership.rs`: `a_status_update_not_signed_by_the_developer_is_ignored` |

The C2 and C9 tests fail against the previous code.

## Verified conformant (no change needed)

- **Terminal states:** campaign chains skip successors once cancelled (`core/src/campaign.rs`). Grant chains admit only `granted → revoked` (`core/src/entitlements.rs`).
- **`503` handling:** `/game`, `/entitlement/claim`, and `/purchase/confirm` map `503` to the typed `AuthorizationUnavailable` error (`core/src/adp_client.rs`), not to a definitive failure.

## Unrelated pre-existing failure

`core/tests/publisher_projection.rs::additive_projection_migration_preserves_existing_rows_and_allows_verified_refresh` fails on the `HEAD` commit before any of this work (`core/tests/publisher_projection.rs:224`). It concerns migration 25 and the marketplace cache, not ADP conformance.
